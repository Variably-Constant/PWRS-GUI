//! Bench: SharedVec<T> vs Mutex<Vec<T>> (the textbook in-process
//! shared-vec pattern) and RwLock<Vec<T>> (the reader-optimized
//! variant).
//!
//! SharedVec claims and publishes each slot with one compare-and-swap
//! and reads one without waiting on a writer, in a file any process maps.
//! Mutex<Vec> and RwLock<Vec> lock around every push and get, and neither
//! can leave its process.
//!
//! Workloads:
//! - push_back (hot producer)
//! - pop_back alone, timed apart from the pushes that fill it, and a push
//!   and a pop in turn
//! - get(i) for stable i (hot observer; the dominant access pattern)
//! - push_back from four threads at once, in rounds that fill the vec, each
//!   thread waiting at a barrier while the vec is emptied between rounds
//!
//! The push cells also time RawVec, the same vec at an element size
//! chosen at run time.

// clippy::disallowed_types is allowed here: this bench measures against lock-based baselines.
#![allow(clippy::disallowed_types)]

use std::hint::black_box;
use std::sync::{Mutex, RwLock};

use criterion::{criterion_group, criterion_main, Criterion};

use subetha_cxc::raw_treiber_stack::ElementLayout;
use subetha_cxc::raw_vec::RawVec;
use subetha_cxc::SharedVec;

/// The raw vec's element: a u32's bytes.
const ELEMENT: ElementLayout = ElementLayout { slot_size: 4, alignment: 4, tag: 1 };
const PAYLOAD: [u8; 4] = 7u32.to_le_bytes();
/// The writer table each vec is made with.
const WRITERS: usize = 4;

fn tmp(name: &str) -> std::path::PathBuf {
    let mut p = std::env::temp_dir();
    let pid = std::process::id();
    p.push(format!("subetha-bench-vec-{name}-{pid}.bin"));
    p
}

// =========================================================
// push_back
// =========================================================

fn push_back(c: &mut Criterion) {
    let p = tmp("push");
    let v: SharedVec<u32> = SharedVec::create(&p, 1_000_000, WRITERS).unwrap();
    c.bench_function("vec.push/mmf", |b| {
        b.iter(|| {
            if v.push_back(black_box(7)).is_err() {
                v.clear();
            }
        });
    });
    drop(v);
    remove(&p);

    let p = tmp("push-raw");
    let v = RawVec::create(&p, 1_000_000, WRITERS, ELEMENT).unwrap();
    c.bench_function("vec.push/raw", |b| {
        b.iter(|| {
            if v.push_back(black_box(&PAYLOAD)).is_err() {
                v.clear().unwrap();
            }
        });
    });
    drop(v);
    remove(&p);

    // Each control clears at the capacity the shared vec clears at, so it
    // never reallocates.
    let m: Mutex<Vec<u32>> = Mutex::new(Vec::with_capacity(1_000_000));
    c.bench_function("vec.push/mutex_vec", |b| {
        b.iter(|| {
            let mut g = m.lock().unwrap();
            if g.len() == 1_000_000 {
                g.clear();
            }
            g.push(black_box(7));
        });
    });

    let rw: RwLock<Vec<u32>> = RwLock::new(Vec::with_capacity(1_000_000));
    c.bench_function("vec.push/rwlock_vec", |b| {
        b.iter(|| {
            let mut g = rw.write().unwrap();
            if g.len() == 1_000_000 {
                g.clear();
            }
            g.push(black_box(7));
        });
    });
}

// =========================================================
// pop_back, and a push and a pop in turn
// =========================================================

/// Elements a pop cell pushes untimed before it times their pops.
const POP_BATCH: u64 = 4096;

fn pop_back(c: &mut Criterion) {
    let p = tmp("pop");
    let v: SharedVec<u32> = SharedVec::create(&p, POP_BATCH as usize, WRITERS).unwrap();
    c.bench_function("vec.pop/mmf", |b| {
        b.iter_custom(|iters| {
            let mut timed = std::time::Duration::ZERO;
            let mut left = iters;
            while left > 0 {
                let batch = left.min(POP_BATCH);
                for i in 0..batch {
                    v.push_back(i as u32).unwrap();
                }
                let start = std::time::Instant::now();
                for _ in 0..batch {
                    black_box(v.pop_back());
                }
                timed += start.elapsed();
                left -= batch;
            }
            timed
        });
    });
    drop(v);
    remove(&p);

    let p = tmp("pop-raw");
    let v = RawVec::create(&p, POP_BATCH as usize, WRITERS, ELEMENT).unwrap();
    let mut out = [0u8; 4];
    c.bench_function("vec.pop/raw", |b| {
        b.iter_custom(|iters| {
            let mut timed = std::time::Duration::ZERO;
            let mut left = iters;
            while left > 0 {
                let batch = left.min(POP_BATCH);
                for _ in 0..batch {
                    v.push_back(&PAYLOAD).unwrap();
                }
                let start = std::time::Instant::now();
                for _ in 0..batch {
                    black_box(v.pop_back(&mut out).unwrap());
                }
                timed += start.elapsed();
                left -= batch;
            }
            timed
        });
    });
    drop(v);
    remove(&p);

    let p = tmp("push-pop");
    let v: SharedVec<u32> = SharedVec::create(&p, 1024, WRITERS).unwrap();
    c.bench_function("vec.push_pop/mmf", |b| {
        b.iter(|| {
            v.push_back(black_box(7)).unwrap();
            black_box(v.pop_back());
        });
    });
    drop(v);
    remove(&p);

    let p = tmp("push-pop-raw");
    let v = RawVec::create(&p, 1024, WRITERS, ELEMENT).unwrap();
    c.bench_function("vec.push_pop/raw", |b| {
        b.iter(|| {
            v.push_back(black_box(&PAYLOAD)).unwrap();
            black_box(v.pop_back(&mut out).unwrap());
        });
    });
    drop(v);
    remove(&p);
}

// =========================================================
// get (observer hot path)
// =========================================================

fn get_observer(c: &mut Criterion) {
    let p = tmp("get");
    let v: SharedVec<u32> = SharedVec::create(&p, 256, WRITERS).unwrap();
    for i in 0..100u32 { v.push_back(i).unwrap(); }
    c.bench_function("vec.get/mmf", |b| {
        b.iter(|| black_box(v.get(50)));
    });
    drop(v);
    remove(&p);

    let mv: Mutex<Vec<u32>> = Mutex::new((0..100u32).collect());
    c.bench_function("vec.get/mutex_vec", |b| {
        b.iter(|| {
            let g = mv.lock().unwrap();
            black_box(g.get(50).copied())
        });
    });

    let rw: RwLock<Vec<u32>> = RwLock::new((0..100u32).collect());
    c.bench_function("vec.get/rwlock_vec", |b| {
        b.iter(|| {
            let g = rw.read().unwrap();
            black_box(g.get(50).copied())
        });
    });
}

// =========================================================
// len (hot observer; this is what dashboards poll)
// =========================================================

fn len_observer(c: &mut Criterion) {
    let p = tmp("len");
    let v: SharedVec<u32> = SharedVec::create(&p, 256, WRITERS).unwrap();
    for i in 0..42u32 { v.push_back(i).unwrap(); }
    c.bench_function("vec.len/mmf", |b| {
        b.iter(|| black_box(v.len()));
    });
    drop(v);
    remove(&p);

    let mv: Mutex<Vec<u32>> = Mutex::new((0..42u32).collect());
    c.bench_function("vec.len/mutex_vec", |b| {
        b.iter(|| black_box(mv.lock().unwrap().len()));
    });
}

// =========================================================
// push_back into a full vec (the refusal at the index counter)
// =========================================================

fn push_refused(c: &mut Criterion) {
    let p = tmp("refused");
    let v: SharedVec<u32> = SharedVec::create(&p, 16, WRITERS).unwrap();
    while v.push_back(0).is_ok() {}
    c.bench_function("vec.push_refused/mmf", |b| {
        b.iter(|| black_box(v.push_back(black_box(7)).is_err()));
    });
    drop(v);
    remove(&p);

    let p = tmp("refused-raw");
    let v = RawVec::create(&p, 16, WRITERS, ELEMENT).unwrap();
    while v.push_back(&PAYLOAD).is_ok() {}
    c.bench_function("vec.push_refused/raw", |b| {
        b.iter(|| black_box(v.push_back(black_box(&PAYLOAD)).is_err()));
    });
    drop(v);
    remove(&p);

    let m: Mutex<Vec<u32>> = Mutex::new(vec![0; 16]);
    c.bench_function("vec.push_refused/mutex_vec", |b| {
        b.iter(|| {
            let mut g = m.lock().unwrap();
            let full = g.len() >= 16;
            if !full {
                g.push(black_box(7));
            }
            black_box(full)
        });
    });
}

// =========================================================
// push_back from four threads at once
// =========================================================

/// Threads a contended cell pushes from, and the pushes each makes in one
/// round; the vec holds exactly one round.
const THREADS: usize = 4;
const PER_THREAD: u64 = 4096;

/// The time of `iters` pushes made from [`THREADS`] threads at once, in
/// rounds of [`PER_THREAD`] pushes each. Before every round `clear` empties
/// the vec while every thread waits at a barrier; the clock runs from the
/// barrier's release to the last thread's arrival at the next, and the
/// rounds' time is scaled to `iters` pushes.
fn timed_rounds(iters: u64, clear: impl Fn(), push: impl Fn(u64) + Sync) -> std::time::Duration {
    let per_round = THREADS as u64 * PER_THREAD;
    let rounds = iters.div_ceil(per_round).max(1);
    let start = std::sync::Barrier::new(THREADS + 1);
    let end = std::sync::Barrier::new(THREADS + 1);
    let mut timed = std::time::Duration::ZERO;
    std::thread::scope(|s| {
        for _ in 0..THREADS {
            s.spawn(|| {
                for _ in 0..rounds {
                    start.wait();
                    for i in 0..PER_THREAD {
                        push(i);
                    }
                    end.wait();
                }
            });
        }
        for _ in 0..rounds {
            clear();
            start.wait();
            let began = std::time::Instant::now();
            end.wait();
            timed += began.elapsed();
        }
    });
    let nanos = timed.as_nanos() * u128::from(iters) / u128::from(rounds * per_round);
    std::time::Duration::from_nanos(nanos as u64)
}

fn contended_push(c: &mut Criterion) {
    let capacity = THREADS * PER_THREAD as usize;
    let p = tmp("push-4t");
    let v: SharedVec<u32> = SharedVec::create(&p, capacity, WRITERS).unwrap();
    c.bench_function("vec.push_4t/mmf", |b| {
        b.iter_custom(|iters| {
            timed_rounds(iters, || v.clear(), |i| {
                v.push_back(black_box(i as u32)).unwrap();
            })
        });
    });
    drop(v);
    remove(&p);

    let p = tmp("push-4t-raw");
    let v = RawVec::create(&p, capacity, WRITERS, ELEMENT).unwrap();
    c.bench_function("vec.push_4t/raw", |b| {
        b.iter_custom(|iters| {
            timed_rounds(iters, || v.clear().unwrap(), |_| {
                v.push_back(black_box(&PAYLOAD)).unwrap();
            })
        });
    });
    drop(v);
    remove(&p);

    let m: Mutex<Vec<u32>> = Mutex::new(Vec::with_capacity(capacity));
    c.bench_function("vec.push_4t/mutex_vec", |b| {
        b.iter_custom(|iters| {
            timed_rounds(iters, || m.lock().unwrap().clear(), |i| {
                m.lock().unwrap().push(black_box(i as u32));
            })
        });
    });
}

/// The bench file at `p`, removed when a cell made it; a failure to remove
/// it is reported.
fn remove(p: &std::path::Path) {
    match std::fs::remove_file(p) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => eprintln!("bench cleanup: {} not removed: {e}", p.display()),
    }
}

criterion_group!(benches,
    push_back,
    pop_back,
    get_observer,
    len_observer,
    push_refused,
    contended_push,
);
criterion_main!(benches);
