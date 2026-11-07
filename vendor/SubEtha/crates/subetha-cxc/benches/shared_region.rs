//! Bench: SharedRegion<T> vs `Mutex<Vec<T>> + Mutex<Vec<usize>>`
//! (the textbook in-process typed-arena + free-list pattern).
//!
//! Architectural claim: SharedRegion provides typed cross-process
//! arena allocation at lock-free fetch_add + Treiber-CAS cost.
//! The Mutex-based baseline pays Mutex lock+unlock per allocate
//! and per free. Plus only the MMF version provides position-
//! independent OffsetPtr<T> resolvable across processes.
//!
//! Workloads:
//! - allocate hot (bump path, uncontended)
//! - allocate + free cycle (free-list reuse)
//! - get hot (resolve a known OffsetPtr)
//! - 4-thread concurrent allocate
//! - 4-thread concurrent allocate + free mix
//!
//! The allocate cells also time RawRegion, the same region at an element
//! size chosen at run time, and FrameRegion, the block region behind
//! oversized ring frames.

// clippy::disallowed_types is allowed here: this bench measures against lock-based baselines.
#![allow(clippy::disallowed_types)]

use std::hint::black_box;
use std::sync::Mutex;

use criterion::{criterion_group, criterion_main, Criterion};

use subetha_cxc::frame_region::FrameRegion;
use subetha_cxc::raw_region::RawRegion;
use subetha_cxc::raw_treiber_stack::ElementLayout;
use subetha_cxc::SharedRegion;

/// The raw region's element: a u64's bytes.
const ELEMENT: ElementLayout = ElementLayout { slot_size: 8, alignment: 8, tag: 1 };
const PAYLOAD: [u8; 8] = 42u64.to_le_bytes();
/// The frame region's block size, in bytes.
const BLOCK: usize = 64;

fn tmp(name: &str) -> std::path::PathBuf {
    let mut p = std::env::temp_dir();
    let pid = std::process::id();
    p.push(format!("subetha-bench-region-{name}-{pid}.bin"));
    p
}

// =========================================================
// In-process arena baseline: Mutex<Vec<T>> + Mutex<Vec<usize>>
// =========================================================

struct MutexArena<T: Copy + Default> {
    slots: Mutex<Vec<T>>,
    free_list: Mutex<Vec<usize>>,
}

impl<T: Copy + Default> MutexArena<T> {
    fn new(capacity: usize) -> Self {
        Self {
            slots: Mutex::new(Vec::with_capacity(capacity)),
            free_list: Mutex::new(Vec::with_capacity(capacity)),
        }
    }
    fn allocate(&self, v: T) -> usize {
        // Try free list first.
        if let Some(idx) = self.free_list.lock().unwrap().pop() {
            self.slots.lock().unwrap()[idx] = v;
            return idx;
        }
        // Bump alloc.
        let mut s = self.slots.lock().unwrap();
        s.push(v);
        s.len() - 1
    }
    fn free(&self, idx: usize) -> T {
        let v = self.slots.lock().unwrap()[idx];
        self.free_list.lock().unwrap().push(idx);
        v
    }
    fn get(&self, idx: usize) -> T {
        self.slots.lock().unwrap()[idx]
    }
    fn clear(&self) {
        self.slots.lock().unwrap().clear();
        self.free_list.lock().unwrap().clear();
    }
}

// =========================================================
// allocate hot (bump path)
// =========================================================

fn allocate_hot(c: &mut Criterion) {
    let p = tmp("alloc");
    // Cap of 4096 is enough for one b.iter call (1 allocate per iter)
    // with clear() reset between batches via iter_batched. .expect()
    // panics on overflow rather than silently returning Err.
    let r: SharedRegion<u64> = SharedRegion::create(&p, 4096).unwrap();
    c.bench_function("region.allocate/mmf", |b| {
        b.iter_batched(
            || r.clear(),
            |_| {
                black_box(r.allocate(black_box(42)).expect("region overflow"))
            },
            criterion::BatchSize::PerIteration,
        );
    });
    drop(r);
    remove(&p);

    let p = tmp("alloc-raw");
    let r = RawRegion::create(&p, 4096, ELEMENT).unwrap();
    c.bench_function("region.allocate/raw", |b| {
        b.iter_batched(
            || r.clear(),
            |_| black_box(r.allocate(black_box(&PAYLOAD)).expect("region overflow")),
            criterion::BatchSize::PerIteration,
        );
    });
    drop(r);
    remove(&p);

    let a: MutexArena<u64> = MutexArena::new(4096);
    c.bench_function("region.allocate/mutex_arena", |b| {
        b.iter_batched(
            || a.clear(),
            |_| black_box(a.allocate(black_box(42))),
            criterion::BatchSize::PerIteration,
        );
    });
}

// =========================================================
// alloc + free cycle (free-list path)
// =========================================================

fn alloc_free_cycle(c: &mut Criterion) {
    let p = tmp("cycle");
    let r: SharedRegion<u64> = SharedRegion::create(&p, 1024).unwrap();
    c.bench_function("region.alloc_free_cycle/mmf", |b| {
        b.iter(|| {
            let ptr = r.allocate(black_box(7)).unwrap();
            black_box(r.free(ptr).unwrap())
        });
    });
    drop(r);
    remove(&p);

    let a: MutexArena<u64> = MutexArena::new(1024);
    c.bench_function("region.alloc_free_cycle/mutex_arena", |b| {
        b.iter(|| {
            let idx = a.allocate(black_box(7));
            black_box(a.free(idx))
        });
    });
}

// =========================================================
// get hot (resolve known ptr)
// =========================================================

fn get_hot(c: &mut Criterion) {
    let p = tmp("get");
    let r: SharedRegion<u64> = SharedRegion::create(&p, 16).unwrap();
    let ptr = r.allocate(0xDEAD_BEEF).unwrap();
    c.bench_function("region.get/mmf", |b| {
        b.iter(|| black_box(r.get(black_box(ptr)).unwrap()));
    });
    drop(r);
    remove(&p);

    let a: MutexArena<u64> = MutexArena::new(16);
    let idx = a.allocate(0xDEAD_BEEF);
    c.bench_function("region.get/mutex_arena", |b| {
        b.iter(|| black_box(a.get(black_box(idx))));
    });
}

// =========================================================
// allocate on a full region (the refusal at the bump counter)
// =========================================================

fn allocate_refused(c: &mut Criterion) {
    let p = tmp("refused");
    let r: SharedRegion<u64> = SharedRegion::create(&p, 16).unwrap();
    while r.allocate(0).is_ok() {}
    c.bench_function("region.allocate_refused/mmf", |b| {
        b.iter(|| black_box(r.allocate(black_box(42)).is_err()));
    });
    drop(r);
    remove(&p);

    let p = tmp("refused-raw");
    let r = RawRegion::create(&p, 16, ELEMENT).unwrap();
    while r.allocate(&PAYLOAD).is_ok() {}
    c.bench_function("region.allocate_refused/raw", |b| {
        b.iter(|| black_box(r.allocate(black_box(&PAYLOAD)).is_err()));
    });
    drop(r);
    remove(&p);

    let p = tmp("refused-frame");
    let f = FrameRegion::create(&p, BLOCK, 16).unwrap();
    while f.alloc().is_some() {}
    c.bench_function("region.allocate_refused/frame", |b| {
        b.iter(|| black_box(f.alloc().is_none()));
    });
    drop(f);
    remove(&p);

    let slots: Mutex<Vec<u64>> = Mutex::new(vec![0; 16]);
    c.bench_function("region.allocate_refused/mutex_arena", |b| {
        b.iter(|| {
            let mut s = slots.lock().unwrap();
            let full = s.len() >= 16;
            if !full {
                s.push(black_box(42));
            }
            black_box(full)
        });
    });
}

/// An allocate nobody has freed into, its slot a fresh one from the bump
/// counter: frame regions of 1024 blocks laid out afresh, and shared and raw
/// regions of 1024 slots cleared, the layout, the clear and the unmapping
/// outside the timing.
fn allocate_fresh(c: &mut Criterion) {
    let p = tmp("fresh-frame");
    c.bench_function("region.allocate_fresh/frame", |b| {
        b.iter_custom(|iters| {
            let mut timed = std::time::Duration::ZERO;
            let mut left = iters;
            while left > 0 {
                let n = left.min(1024);
                let f = FrameRegion::reset(&p, BLOCK, 1024).unwrap();
                let start = std::time::Instant::now();
                for _ in 0..n {
                    f.alloc().expect("a block in a fresh region");
                }
                timed += start.elapsed();
                drop(f);
                left -= n;
            }
            timed
        });
    });
    remove(&p);

    // The shared and raw regions cleared outside the timing, so every
    // timed allocate is a bump.
    let p = tmp("fresh-mmf");
    let r: SharedRegion<u64> = SharedRegion::create(&p, 1024).unwrap();
    c.bench_function("region.allocate_fresh/mmf", |b| {
        b.iter_custom(|iters| {
            let mut timed = std::time::Duration::ZERO;
            let mut left = iters;
            while left > 0 {
                let n = left.min(1024);
                r.clear();
                let start = std::time::Instant::now();
                for _ in 0..n {
                    black_box(r.allocate(black_box(42)).expect("a slot in a cleared region"));
                }
                timed += start.elapsed();
                left -= n;
            }
            timed
        });
    });
    drop(r);
    remove(&p);

    let p = tmp("fresh-raw");
    let r = RawRegion::create(&p, 1024, ELEMENT).unwrap();
    c.bench_function("region.allocate_fresh/raw", |b| {
        b.iter_custom(|iters| {
            let mut timed = std::time::Duration::ZERO;
            let mut left = iters;
            while left > 0 {
                let n = left.min(1024);
                r.clear();
                let start = std::time::Instant::now();
                for _ in 0..n {
                    black_box(r.allocate(black_box(&PAYLOAD)).expect("a slot in a cleared region"));
                }
                timed += start.elapsed();
                left -= n;
            }
            timed
        });
    });
    drop(r);
    remove(&p);

    c.bench_function("region.allocate_fresh/mutex_arena", |b| {
        b.iter_custom(|iters| {
            let mut timed = std::time::Duration::ZERO;
            let mut left = iters;
            while left > 0 {
                let n = left.min(1024);
                let a: MutexArena<u64> = MutexArena::new(1024);
                let start = std::time::Instant::now();
                for _ in 0..n {
                    a.allocate(black_box(42));
                }
                timed += start.elapsed();
                drop(a);
                left -= n;
            }
            timed
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

// Multi-thread allocate correctness is covered by source-level
// unit tests. A per-iter thread::spawn microbench (even inside
// iter_batched) is dominated by Windows thread-creation cost.

criterion_group!(benches,
    allocate_hot,
    alloc_free_cycle,
    get_hot,
    allocate_refused,
    allocate_fresh,
);
criterion_main!(benches);
