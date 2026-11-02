//! Bench: the strategy-switching set, typed (`SharedUniversal<u64>`) and
//! raw (`RawUniversal` at 8-byte elements, the C ABI's face): membership
//! on each strategy, inserts into an empty set, a migration, a clear,
//! four threads at once, and lookups spread across sets too many to stay
//! in cache.
//!
//! Every cell builds against the set before and after the lock-free table,
//! using only calls both have, so one file times both in an A/B. Built with
//! `SUBETHA_AB_BASELINE` set, against the set before, the typed insert into
//! the list strategy checks membership first: that set's list appended
//! without the check its insert now makes.
//!
//! Each membership and insert group ends with a `std_hashset` cell doing the
//! same work on a std `HashSet`. No build of SubEtha changes its code, so in
//! an A/B its ratio measures the host and the cell's position, not the
//! change, and a filter on `universal` or `raw` keeps it.

use std::collections::HashSet;
use std::hint::black_box;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use criterion::{criterion_group, criterion_main, BatchSize, Criterion};

use subetha_cxc::raw_universal::RawUniversal;
use subetha_cxc::{SharedUniversal, UniversalStrategy as Strategy};

const N: u64 = 1024;
const PER_ITER: u64 = 256;
/// Whether this build times the set before the lock-free table.
const BASELINE: bool = option_env!("SUBETHA_AB_BASELINE").is_some();
const THREADS: usize = 4;
const COLD_SETS: usize = 128;

fn tmp_base(name: &str) -> std::path::PathBuf {
    std::env::temp_dir().join(format!("subetha-bench-universal-{name}-{}", std::process::id()))
}

/// Remove every file named after `base`, saying which one could not be.
fn cleanup(base: &std::path::Path) {
    let stem = base.file_name().expect("a base has a file name").to_string_lossy().to_string();
    let parent = base.parent().expect("a base has a directory");
    let entries = match std::fs::read_dir(parent) {
        Ok(entries) => entries,
        Err(e) => {
            eprintln!("bench cleanup: {} not listed: {e}", parent.display());
            return;
        }
    };
    for entry in entries {
        let entry = match entry {
            Ok(entry) => entry,
            Err(e) => {
                eprintln!("bench cleanup: an entry of {} not read: {e}", parent.display());
                continue;
            }
        };
        if entry.file_name().to_string_lossy().starts_with(&stem) {
            match std::fs::remove_file(entry.path()) {
                Ok(()) => {}
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => eprintln!("bench cleanup: {} not removed: {e}", entry.path().display()),
            }
        }
    }
}

fn typed(name: &str, capacity: u64, strategy: Strategy) -> (SharedUniversal<u64>, std::path::PathBuf) {
    let base = tmp_base(name);
    cleanup(&base);
    let u: SharedUniversal<u64> = SharedUniversal::create(&base, capacity as usize).expect("create");
    u.migrate_to(strategy).expect("strategy");
    (u, base)
}

fn raw(name: &str, capacity: u64, strategy: Strategy) -> (RawUniversal, std::path::PathBuf) {
    let base = tmp_base(name);
    cleanup(&base);
    let u = RawUniversal::create(&base, capacity as usize, 8, Strategy::Vec).expect("create");
    u.migrate_to(strategy).expect("strategy");
    (u, base)
}

/// Threads that each run `work` with their index once per round the
/// caller starts, waiting by spinning on a round counter so a round costs
/// no thread start.
struct Crew {
    round: Arc<AtomicU64>,
    done: Arc<AtomicU64>,
    stop: Arc<AtomicBool>,
    threads: Vec<std::thread::JoinHandle<()>>,
}

impl Crew {
    fn new(work: impl Fn(usize) + Send + Sync + 'static) -> Self {
        let work = Arc::new(work);
        let round = Arc::new(AtomicU64::new(0));
        let done = Arc::new(AtomicU64::new(0));
        let stop = Arc::new(AtomicBool::new(false));
        let threads = (0..THREADS)
            .map(|t| {
                let (work, round, done, stop) = (Arc::clone(&work), Arc::clone(&round), Arc::clone(&done), Arc::clone(&stop));
                std::thread::spawn(move || {
                    let mut seen = 0;
                    loop {
                        let now = loop {
                            let r = round.load(Ordering::Acquire);
                            if r != seen || stop.load(Ordering::Acquire) {
                                break r;
                            }
                            std::hint::spin_loop();
                        };
                        if stop.load(Ordering::Acquire) {
                            return;
                        }
                        seen = now;
                        work(t);
                        done.fetch_add(1, Ordering::AcqRel);
                    }
                })
            })
            .collect();
        Self { round, done, stop, threads }
    }

    /// One round, timed from its start to the last thread's finish.
    fn run(&self) -> Duration {
        let target = self.done.load(Ordering::Acquire) + THREADS as u64;
        let start = Instant::now();
        self.round.fetch_add(1, Ordering::AcqRel);
        while self.done.load(Ordering::Acquire) < target {
            std::hint::spin_loop();
        }
        start.elapsed()
    }
}

impl Drop for Crew {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        for t in self.threads.drain(..) {
            if t.join().is_err() {
                eprintln!("a bench thread panicked");
            }
        }
    }
}

fn typed_contains(c: &mut Criterion) {
    for (cell, strategy) in [("universal.contains/vec_n1k", Strategy::Vec), ("universal.contains/map_n1k", Strategy::Map)] {
        let (u, base) = typed(cell.rsplit('/').next().expect("a cell name"), N, strategy);
        for k in 0..N {
            u.insert(k).expect("insert");
        }
        c.bench_function(cell, |b| b.iter(|| black_box(u.contains(black_box(&999u64)).expect("contains"))));
        drop(u);
        cleanup(&base);
    }
    let s: HashSet<u64> = (0..N).collect();
    c.bench_function("universal.contains/std_hashset_n1k", |b| b.iter(|| black_box(s.contains(black_box(&999u64)))));
}

fn raw_contains(c: &mut Criterion) {
    for (cell, strategy) in [("raw.contains/vec_n1k", Strategy::Vec), ("raw.contains/map_n1k", Strategy::Map)] {
        let (u, base) = raw(&format!("raw-{}", cell.rsplit('/').next().expect("a cell name")), N, strategy);
        for k in 0..N {
            u.insert(&k.to_le_bytes()).expect("insert");
        }
        let probe = 999u64.to_le_bytes();
        c.bench_function(cell, |b| b.iter(|| black_box(u.contains(black_box(&probe)).expect("contains"))));
        drop(u);
        cleanup(&base);
    }
    let s: HashSet<[u8; 8]> = (0..N).map(u64::to_le_bytes).collect();
    let probe = 999u64.to_le_bytes();
    c.bench_function("raw.contains/std_hashset_n1k", |b| b.iter(|| black_box(s.contains(black_box(&probe)))));
}

fn typed_insert(c: &mut Criterion) {
    for (cell, strategy) in [("universal.insert/vec_n1k", Strategy::Vec), ("universal.insert/map_n1k", Strategy::Map)] {
        let (u, base) = typed(&format!("ins-{}", cell.rsplit('/').next().expect("a cell name")), N, strategy);
        let check_first = BASELINE && cell.ends_with("vec_n1k");
        c.bench_function(cell, |b| {
            b.iter_batched(
                || u.clear().expect("clear"),
                |_| {
                    for k in 0..PER_ITER {
                        if check_first && u.contains(&black_box(k)).expect("contains") {
                            continue;
                        }
                        u.insert(black_box(k)).expect("insert");
                    }
                },
                BatchSize::PerIteration,
            );
        });
        drop(u);
        cleanup(&base);
    }
    c.bench_function("universal.insert/std_hashset_n1k", |b| {
        b.iter_batched(
            || HashSet::<u64>::with_capacity(N as usize),
            |mut s| {
                for k in 0..PER_ITER {
                    s.insert(black_box(k));
                }
                s
            },
            BatchSize::PerIteration,
        );
    });
}

fn raw_insert(c: &mut Criterion) {
    for (cell, strategy) in [("raw.insert/vec_n1k", Strategy::Vec), ("raw.insert/map_n1k", Strategy::Map)] {
        let (u, base) = raw(&format!("raw-ins-{}", cell.rsplit('/').next().expect("a cell name")), N, strategy);
        c.bench_function(cell, |b| {
            b.iter_batched(
                || u.clear().expect("clear"),
                |_| {
                    for k in 0..PER_ITER {
                        u.insert(black_box(&k.to_le_bytes())).expect("insert");
                    }
                },
                BatchSize::PerIteration,
            );
        });
        drop(u);
        cleanup(&base);
    }
    c.bench_function("raw.insert/std_hashset_n1k", |b| {
        b.iter_batched(
            || HashSet::<[u8; 8]>::with_capacity(N as usize),
            |mut s| {
                for k in 0..PER_ITER {
                    s.insert(black_box(k.to_le_bytes()));
                }
                s
            },
            BatchSize::PerIteration,
        );
    });
}

fn migrations(c: &mut Criterion) {
    let (u, base) = typed("migrate", N, Strategy::Vec);
    for k in 0..N {
        u.insert(k).expect("insert");
    }
    let mut toggle = false;
    c.bench_function("universal.migrate/vec_map_n1k", |b| {
        b.iter(|| {
            let target = if toggle { Strategy::Vec } else { Strategy::Map };
            u.migrate_to(target).expect("migrate");
            toggle = !toggle;
        });
    });
    drop(u);
    cleanup(&base);

    let (u, base) = raw("raw-migrate", N, Strategy::Vec);
    for k in 0..N {
        u.insert(&k.to_le_bytes()).expect("insert");
    }
    let mut toggle = false;
    c.bench_function("raw.migrate/vec_map_n1k", |b| {
        b.iter(|| {
            let target = if toggle { Strategy::Vec } else { Strategy::Map };
            u.migrate_to(target).expect("migrate");
            toggle = !toggle;
        });
    });
    drop(u);
    cleanup(&base);
}

fn clears(c: &mut Criterion) {
    let (u, base) = typed("clear", N, Strategy::Map);
    c.bench_function("universal.clear/map_n256", |b| {
        b.iter_batched(
            || {
                for k in 0..PER_ITER {
                    u.insert(k).expect("insert");
                }
            },
            |_| u.clear().expect("clear"),
            BatchSize::PerIteration,
        );
    });
    drop(u);
    cleanup(&base);

    let (u, base) = raw("raw-clear", N, Strategy::Map);
    c.bench_function("raw.clear/map_n256", |b| {
        b.iter_batched(
            || {
                for k in 0..PER_ITER {
                    u.insert(&k.to_le_bytes()).expect("insert");
                }
            },
            |_| u.clear().expect("clear"),
            BatchSize::PerIteration,
        );
    });
    drop(u);
    cleanup(&base);
}

fn contended(c: &mut Criterion) {
    let (u, base) = typed("contended-insert", N * 2, Strategy::Map);
    let u = Arc::new(u);
    let crew = {
        let u = Arc::clone(&u);
        Crew::new(move |t| {
            let first = t as u64 * PER_ITER;
            for k in first..first + PER_ITER {
                u.insert(black_box(k)).expect("insert");
            }
        })
    };
    c.bench_function("universal.contended/insert_map_4t", |b| {
        b.iter_custom(|iters| {
            (0..iters)
                .map(|_| {
                    u.clear().expect("clear");
                    crew.run()
                })
                .sum()
        });
    });
    drop(crew);
    drop(u);
    cleanup(&base);

    let (u, base) = typed("contended-contains", N, Strategy::Map);
    for k in 0..N {
        u.insert(k).expect("insert");
    }
    let u = Arc::new(u);
    let crew = {
        let u = Arc::clone(&u);
        Crew::new(move |t| {
            for i in 0..N {
                black_box(u.contains(&((i * 7 + t as u64) % N)).expect("contains"));
            }
        })
    };
    c.bench_function("universal.contended/contains_map_4t", |b| {
        b.iter_custom(|iters| (0..iters).map(|_| crew.run()).sum());
    });
    drop(crew);
    drop(u);
    cleanup(&base);

    let (u, base) = raw("raw-contended-insert", N * 2, Strategy::Map);
    let u = Arc::new(u);
    let crew = {
        let u = Arc::clone(&u);
        Crew::new(move |t| {
            let first = t as u64 * PER_ITER;
            for k in first..first + PER_ITER {
                u.insert(black_box(&k.to_le_bytes())).expect("insert");
            }
        })
    };
    c.bench_function("raw.contended/insert_map_4t", |b| {
        b.iter_custom(|iters| {
            (0..iters)
                .map(|_| {
                    u.clear().expect("clear");
                    crew.run()
                })
                .sum()
        });
    });
    drop(crew);
    drop(u);
    cleanup(&base);
}

fn cold(c: &mut Criterion) {
    let sets: Vec<(SharedUniversal<u64>, std::path::PathBuf)> = (0..COLD_SETS)
        .map(|s| {
            let (u, base) = typed(&format!("cold{s}"), N, Strategy::Map);
            for k in 0..N {
                u.insert(k).expect("insert");
            }
            (u, base)
        })
        .collect();
    let mut i = 0u64;
    c.bench_function("universal.cold/contains_map_128_sets", |b| {
        b.iter(|| {
            i = i.wrapping_add(1);
            let (u, _) = &sets[(i as usize) % COLD_SETS];
            black_box(u.contains(&(i.wrapping_mul(2_654_435_761) % N)).expect("contains"))
        });
    });
    for (u, base) in sets {
        drop(u);
        cleanup(&base);
    }
}

criterion_group!(benches, typed_contains, raw_contains, typed_insert, raw_insert, migrations, clears, contended, cold);
criterion_main!(benches);
