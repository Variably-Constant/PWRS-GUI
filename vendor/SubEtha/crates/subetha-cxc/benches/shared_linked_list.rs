//! Bench: SharedLinkedList vs Mutex<VecDeque<T>> (FIFO baseline)
//! and Mutex<LinkedList<T>> (std DLL baseline).
//!
//! Architectural claim: handle-based O(1) removal is the unique
//! value. End-only push/pop ties with VecDeque; iteration ties
//! with std LinkedList; but `remove(handle)` is O(1) for us vs
//! O(N) scan for std::collections::LinkedList::remove (which
//! requires walking from head/tail to find the target).
//!
//! Workloads:
//! - push_back hot
//! - pop_front hot
//! - push_back + pop_front on a list held at 64 values
//! - push_back into a full list, and into lists nobody has popped from
//! - iter_forward 100-element list
//! - remove from middle by handle (vs scan-and-remove)

// clippy::disallowed_types is allowed here: this bench measures against lock-based baselines.
#![allow(clippy::disallowed_types)]

use std::collections::{LinkedList, VecDeque};
use std::hint::black_box;
use std::sync::Mutex;

use criterion::{criterion_group, criterion_main, Criterion};

use subetha_cxc::{LinkedListError, NodeHandle, RegionError, SharedLinkedList};

fn tmp(name: &str) -> std::path::PathBuf {
    let mut p = std::env::temp_dir();
    let pid = std::process::id();
    p.push(format!("subetha-bench-linkedlist-{name}-{pid}.bin"));
    p
}

// =========================================================
// push_back hot
// =========================================================

/// Nodes the push_back list can hold, its sentinel's slot among them.
const PUSH_CAP: usize = 1 << 20;

fn push_back_hot(c: &mut Criterion) {
    let p = tmp("push-back");
    let l: SharedLinkedList<u32> = SharedLinkedList::create(&p, PUSH_CAP).unwrap();
    c.bench_function("linkedlist.push_back/mmf", |b| {
        b.iter(|| {
            match l.push_back(black_box(42)) {
                Ok(h) => black_box(h),
                // Once full, take from the front until a push fits: one pop for
                // a list that frees a node at a time, more for one that frees
                // a chunk at a time.
                Err(_) => loop {
                    l.pop_front();
                    if let Ok(h) = l.push_back(42) {
                        break h;
                    }
                },
            }
        });
    });
    drop(l);
    std::fs::remove_file(&p).ok();

    // The baselines hold as many values as the list, and like it take one
    // from the front before each push once full.
    let v: Mutex<VecDeque<u32>> = Mutex::new(VecDeque::with_capacity(PUSH_CAP));
    c.bench_function("linkedlist.push_back/mutex_vecdeque", |b| {
        b.iter(|| {
            let mut g = v.lock().unwrap();
            if g.len() >= PUSH_CAP - 1 {
                black_box(g.pop_front());
            }
            g.push_back(black_box(42))
        });
    });

    let s: Mutex<LinkedList<u32>> = Mutex::new(LinkedList::new());
    c.bench_function("linkedlist.push_back/mutex_std_linkedlist", |b| {
        b.iter(|| {
            let mut g = s.lock().unwrap();
            if g.len() >= PUSH_CAP - 1 {
                black_box(g.pop_front());
            }
            g.push_back(black_box(42))
        });
    });
}

// =========================================================
// pop_front hot (pre-filled)
// =========================================================

fn pop_front_hot(c: &mut Criterion) {
    c.bench_function("linkedlist.pop_front/mmf", |b| {
        let p = tmp("pop-front");
        let l: SharedLinkedList<u32> = SharedLinkedList::create(&p, 1 << 16).unwrap();
        b.iter(|| {
            if l.is_empty() {
                for i in 0..1000u32 { l.push_back(i).unwrap(); }
            }
            black_box(l.pop_front())
        });
        drop(l);
        std::fs::remove_file(&p).ok();
    });

    c.bench_function("linkedlist.pop_front/mutex_vecdeque", |b| {
        let v: Mutex<VecDeque<u32>> = Mutex::new(VecDeque::with_capacity(1 << 16));
        b.iter(|| {
            if v.lock().unwrap().is_empty() {
                let mut g = v.lock().unwrap();
                for i in 0..1000u32 { g.push_back(i); }
            }
            black_box(v.lock().unwrap().pop_front())
        });
    });
}

// =========================================================
// push_back + pop_front on a list that stays at 64 values
// =========================================================

fn push_pop_64(c: &mut Criterion) {
    let p = tmp("push-pop");
    let l: SharedLinkedList<u32> = SharedLinkedList::create(&p, 256).unwrap();
    for i in 0..64u32 { l.push_back(i).unwrap(); }
    c.bench_function("linkedlist.push_pop_64/mmf", |b| {
        b.iter(|| {
            black_box(l.push_back(black_box(42)).unwrap());
            black_box(l.pop_front())
        });
    });
    // The same pairs in a plain timed loop on the same list, the shape of
    // the ops_profile harness's list_push_pop.
    c.bench_function("linkedlist.push_pop_64/mmf_loop", |b| {
        b.iter_custom(|iters| {
            let start = std::time::Instant::now();
            for i in 0..iters {
                black_box(l.push_back(black_box(i as u32)).unwrap());
                black_box(l.pop_front());
            }
            start.elapsed()
        });
    });
    // The same pairs under criterion's loop, the popped value handed to
    // criterion's own black_box alone.
    c.bench_function("linkedlist.push_pop_64/mmf_once", |b| {
        b.iter(|| {
            black_box(l.push_back(black_box(42)).unwrap());
            l.pop_front()
        });
    });
    drop(l);
    if let Err(e) = std::fs::remove_file(&p) {
        eprintln!("bench cleanup: {} not removed: {e}", p.display());
    }

    let v: Mutex<VecDeque<u32>> = Mutex::new((0..64u32).collect());
    c.bench_function("linkedlist.push_pop_64/mutex_vecdeque", |b| {
        b.iter(|| {
            let mut g = v.lock().unwrap();
            g.push_back(black_box(42));
            black_box(g.pop_front())
        });
    });
}

// =========================================================
// push_back into a full list, and into fresh lists
// =========================================================

/// A push into a full list: the refusal at the free stack and the node
/// counter. The control refuses at a length check under its lock.
fn push_refused(c: &mut Criterion) {
    let p = tmp("refused");
    let l: SharedLinkedList<u32> = SharedLinkedList::create(&p, 64).unwrap();
    loop {
        match l.push_back(0) {
            Ok(_) => {}
            Err(LinkedListError::Region(RegionError::Full)) => break,
            Err(e) => panic!("filling the list: {e}"),
        }
    }
    c.bench_function("linkedlist.push_refused/mmf", |b| {
        b.iter(|| black_box(l.push_back(black_box(42))));
    });
    drop(l);
    if let Err(e) = std::fs::remove_file(&p) {
        eprintln!("bench cleanup: {} not removed: {e}", p.display());
    }

    // As many values as the list holds, its sentinel's slot aside.
    let v: Mutex<VecDeque<u32>> = Mutex::new((0..63u32).collect());
    c.bench_function("linkedlist.push_refused/mutex_vecdeque", |b| {
        b.iter(|| {
            let mut g = v.lock().unwrap();
            let full = g.len() >= 63;
            if !full {
                g.push_back(black_box(42));
            }
            black_box(full)
        });
    });
}

/// Nodes in each list the fresh cell makes, its sentinel's slot among them.
const FRESH_NODES: u64 = 1024;

/// A push into a list nobody has popped from, its node a fresh one from the
/// node counter: lists made afresh, the making, the unmapping and the
/// file's removal outside the timing.
fn push_fresh(c: &mut Criterion) {
    let p = tmp("fresh");
    c.bench_function("linkedlist.push_fresh/mmf", |b| {
        b.iter_custom(|iters| {
            let mut timed = std::time::Duration::ZERO;
            let mut left = iters;
            while left > 0 {
                let n = left.min(FRESH_NODES - 1);
                let l: SharedLinkedList<u32> = SharedLinkedList::create(&p, FRESH_NODES as usize).unwrap();
                let start = std::time::Instant::now();
                for i in 0..n {
                    black_box(l.push_back(black_box(i as u32)).unwrap());
                }
                timed += start.elapsed();
                drop(l);
                std::fs::remove_file(&p).expect("the fresh list's file is removed before the next is made");
                left -= n;
            }
            timed
        });
    });

    c.bench_function("linkedlist.push_fresh/mutex_vecdeque", |b| {
        b.iter_custom(|iters| {
            let mut timed = std::time::Duration::ZERO;
            let mut left = iters;
            while left > 0 {
                let n = left.min(FRESH_NODES - 1);
                let v: Mutex<VecDeque<u32>> = Mutex::new(VecDeque::with_capacity(FRESH_NODES as usize - 1));
                let start = std::time::Instant::now();
                for i in 0..n {
                    v.lock().unwrap().push_back(black_box(i as u32));
                }
                timed += start.elapsed();
                drop(v);
                left -= n;
            }
            timed
        });
    });
}

// =========================================================
// iter_forward over 100 elements
// =========================================================

fn iter_100(c: &mut Criterion) {
    let p = tmp("iter");
    let l: SharedLinkedList<u32> = SharedLinkedList::create(&p, 128).unwrap();
    for i in 0..100u32 { l.push_back(i).unwrap(); }
    c.bench_function("linkedlist.iter_100/mmf", |b| {
        b.iter(|| black_box(l.iter_forward()));
    });
    drop(l);
    std::fs::remove_file(&p).ok();

    let v: Mutex<VecDeque<u32>> = Mutex::new((0..100u32).collect());
    c.bench_function("linkedlist.iter_100/mutex_vecdeque", |b| {
        b.iter(|| {
            let g = v.lock().unwrap();
            black_box(g.iter().copied().collect::<Vec<u32>>())
        });
    });

    let s: Mutex<LinkedList<u32>> = Mutex::new((0..100u32).collect());
    c.bench_function("linkedlist.iter_100/mutex_std_linkedlist", |b| {
        b.iter(|| {
            let g = s.lock().unwrap();
            black_box(g.iter().copied().collect::<Vec<u32>>())
        });
    });
}

// =========================================================
// The headline: remove from middle of 100-element list.
//
// SharedLinkedList: pass the handle directly to remove() = O(1).
// Mutex<LinkedList>: must scan to find the target = O(N/2 avg).
// Mutex<VecDeque>: pop_front + push_back doesn't apply (we need
//   to remove a specific item, not just one end), so this would
//   be O(N) shift. Skipped because the comparison is unfair.
// =========================================================

fn remove_middle(c: &mut Criterion) {
    const N: usize = 100;

    let p = tmp("remove-middle");
    c.bench_function("linkedlist.remove_middle_100/mmf_handle", |b| {
        let l: SharedLinkedList<u32> = SharedLinkedList::create(&p, 256).unwrap();
        let mut handles: Vec<NodeHandle<u32>> = (0..N as u32)
            .map(|i| l.push_back(i).unwrap()).collect();
        let mut next_to_remove = N / 2;
        b.iter(|| {
            if l.is_empty() {
                // Refill.
                handles.clear();
                for i in 0..N as u32 { handles.push(l.push_back(i).unwrap()); }
                next_to_remove = N / 2;
            }
            // Remove the middle-ish handle.
            let idx = next_to_remove % handles.len();
            let h = handles.remove(idx);
            black_box(l.remove(h));
            next_to_remove = next_to_remove.saturating_sub(1);
        });
        drop(l);
        std::fs::remove_file(&p).ok();
    });

    c.bench_function("linkedlist.remove_middle_100/std_linkedlist_scan", |b| {
        // Std LinkedList has no O(1) remove-by-iterator (cursor is
        // unstable); use the legacy "split_off + collect" pattern
        // which is O(N).
        let mut l: LinkedList<u32> = (0..N as u32).collect();
        let mut next_idx = N / 2;
        b.iter(|| {
            if l.is_empty() {
                l = (0..N as u32).collect();
                next_idx = N / 2;
            }
            // O(N): split at the target index, take the head, pop
            // the target, splice the rest back. This is what callers
            // do when they need middle removal on std::LinkedList.
            let idx = next_idx % l.len();
            let mut after = l.split_off(idx);
            let removed = after.pop_front();
            l.append(&mut after);
            black_box(removed);
            next_idx = next_idx.saturating_sub(1);
        });
    });
}

// =========================================================
// Storage witness
// =========================================================

fn storage(c: &mut Criterion) {
    const NODES: usize = 1 << 12;
    let p = tmp("storage");
    let l: SharedLinkedList<u32> = SharedLinkedList::create(&p, NODES).unwrap();
    let file_bytes = std::fs::metadata(&p).unwrap().len() as usize;
    drop(l);
    std::fs::remove_file(&p).ok();
    let per_node = file_bytes / NODES;
    eprintln!("[storage] SharedLinkedList<u32> file = {per_node} bytes per node of capacity");
    c.bench_function("linkedlist.storage_witness", |b| {
        b.iter(|| black_box(per_node));
    });
}

criterion_group!(benches,
    push_back_hot,
    pop_front_hot,
    push_pop_64,
    push_refused,
    push_fresh,
    iter_100,
    remove_middle,
    storage,
);
criterion_main!(benches);
