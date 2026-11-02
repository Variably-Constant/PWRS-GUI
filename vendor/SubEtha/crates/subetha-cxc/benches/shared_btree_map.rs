//! Bench: SharedBTreeMap vs Mutex<BTreeMap>.
//!
//! Architectural claim: SharedBTreeMap is the substrate's ordered-map
//! primitive - a cross-process, lock-free-read sorted key/value map that an
//! in-process `BTreeMap` cannot be. Each node packs up to B=15 sorted keys
//! in a contiguous array (fanout 16), so a lookup touches ~log_16(N) nodes
//! and each node's binary search reads one prefetcher-friendly key run
//! rather than chasing scattered single-cache-line nodes. The textbook
//! `Mutex<BTreeMap>` contender measures the same operations; the mmf adds
//! cross-process visibility, lock-free reads, and disk persistence.
//!
//! Workloads:
//! - get_hit / get_miss (100 keys, warm: matches small-map use)
//! - iter_ascending (100 elements)
//! - get_hit_100k (large map, pseudo-random keys: exposes cache locality)
//! - insert_refused (an insert past a spent node pool)
//! - insert_fresh (ascending inserts into fresh maps)
//!
//! The insert cells also time RawBTreeMap, the same map at key and value
//! sizes chosen at run time.

// clippy::disallowed_types is allowed here: this bench measures against lock-based baselines.
#![allow(clippy::disallowed_types)]

use std::collections::BTreeMap;
use std::hint::black_box;
use std::sync::Mutex;

use criterion::{criterion_group, criterion_main, Criterion};

use subetha_cxc::raw_btree_map::RawBTreeMap;
use subetha_cxc::SharedBTreeMap;

/// The raw map's layout tag; its keys are big-endian u32 bytes, so byte
/// order is numeric order, and its values are four bytes.
const TAG: u64 = 1;
const VALUE: [u8; 4] = [0; 4];

fn tmp_path(name: &str) -> std::path::PathBuf {
    let mut p = std::env::temp_dir();
    let pid = std::process::id();
    p.push(format!("subetha-bench-btree-{name}-{pid}.bin"));
    p
}

// Deterministic key spread (LCG) so large-map lookups don't all hit the
// same hot nodes; keeps the bench reproducible (no RNG).
#[inline]
fn next_key(state: &mut u32, modulo: u32) -> u32 {
    *state = state.wrapping_mul(1_103_515_245).wrapping_add(12_345);
    (*state >> 8) % modulo
}

// =========================================================
// get_hit / get_miss at 100 keys (warm)
// =========================================================

fn get_small(c: &mut Criterion) {
    let path = tmp_path("get-small");
    let bt: SharedBTreeMap<u32, u32> = SharedBTreeMap::create(&path, 256).unwrap();
    for k in 0..100u32 { bt.insert(k, k.wrapping_mul(10)).unwrap(); }
    c.bench_function("btree.get_hit/mmf", |b| {
        b.iter(|| black_box(bt.get(black_box(&50))));
    });
    c.bench_function("btree.get_miss/mmf", |b| {
        b.iter(|| black_box(bt.get(black_box(&9999))));
    });
    drop(bt);
    remove(&path);

    let m: Mutex<BTreeMap<u32, u32>> =
        Mutex::new((0..100u32).map(|k| (k, k.wrapping_mul(10))).collect());
    c.bench_function("btree.get_hit/mutex_btreemap", |b| {
        b.iter(|| {
            let g = m.lock().unwrap();
            black_box(g.get(black_box(&50)).copied())
        });
    });
    c.bench_function("btree.get_miss/mutex_btreemap", |b| {
        b.iter(|| {
            let g = m.lock().unwrap();
            black_box(g.get(black_box(&9999)).copied())
        });
    });
}

// =========================================================
// iter_ascending (100 elements)
// =========================================================

fn iter_small(c: &mut Criterion) {
    let path = tmp_path("iter");
    let bt: SharedBTreeMap<u32, u32> = SharedBTreeMap::create(&path, 256).unwrap();
    for k in 0..100u32 { bt.insert(k, k).unwrap(); }
    c.bench_function("btree.iter_ascending/mmf", |b| {
        b.iter(|| black_box(bt.iter_ascending()));
    });
    drop(bt);
    remove(&path);

    let m: Mutex<BTreeMap<u32, u32>> =
        Mutex::new((0..100u32).map(|k| (k, k)).collect());
    c.bench_function("btree.iter_ascending/mutex_btreemap", |b| {
        b.iter(|| {
            let g = m.lock().unwrap();
            black_box(g.iter().map(|(k, v)| (*k, *v)).collect::<Vec<_>>())
        });
    });
}

// =========================================================
// get_hit at 100k keys (large; pseudo-random lookups)
// =========================================================

fn get_large(c: &mut Criterion) {
    const N: u32 = 100_000;
    let path = tmp_path("get-large");
    // ~N/4 nodes is ample (each node holds up to 15 keys); round up.
    let bt: SharedBTreeMap<u32, u32> =
        SharedBTreeMap::create(&path, (N / 3) as usize).unwrap();
    for k in 0..N { bt.insert(k, k).unwrap(); }
    let mut st = 0x1234_5678u32;
    c.bench_function("btree.get_hit_100k/mmf", |b| {
        b.iter(|| {
            let key = next_key(&mut st, N);
            black_box(bt.get(black_box(&key)))
        });
    });
    drop(bt);
    remove(&path);

    let m: Mutex<BTreeMap<u32, u32>> =
        Mutex::new((0..N).map(|k| (k, k)).collect());
    let mut st = 0x1234_5678u32;
    c.bench_function("btree.get_hit_100k/mutex_btreemap", |b| {
        b.iter(|| {
            let key = next_key(&mut st, N);
            let g = m.lock().unwrap();
            black_box(g.get(black_box(&key)).copied())
        });
    });
}

/// An insert into a map whose node pool is spent: keys go in ascending
/// until the first refusal, which leaves the rightmost leaf full, so the
/// next key up needs a node the counter refuses every time. The control
/// refuses at a length check under its lock.
fn insert_refused(c: &mut Criterion) {
    let path = tmp_path("refused");
    let bt: SharedBTreeMap<u32, u32> = SharedBTreeMap::create(&path, 16).unwrap();
    let mut key = 0u32;
    while bt.insert(key, key).is_ok() {
        key += 1;
    }
    c.bench_function("btree.insert_refused/mmf", |b| {
        b.iter(|| black_box(bt.insert(black_box(key), 0).is_err()));
    });
    drop(bt);
    remove(&path);

    let path = tmp_path("refused-raw");
    let m = RawBTreeMap::create(&path, 16, 4, 4, TAG).unwrap();
    let mut raw_key = 0u32;
    while m.insert(&raw_key.to_be_bytes(), &VALUE, None).is_ok() {
        raw_key += 1;
    }
    let refused = raw_key.to_be_bytes();
    c.bench_function("btree.insert_refused/raw", |b| {
        b.iter(|| black_box(m.insert(black_box(&refused), &VALUE, None).is_err()));
    });
    drop(m);
    remove(&path);

    let m: Mutex<BTreeMap<u32, u32>> = Mutex::new((0..key).map(|k| (k, k)).collect());
    c.bench_function("btree.insert_refused/mutex_btreemap", |b| {
        b.iter(|| {
            let mut g = m.lock().unwrap();
            let full = g.len() >= key as usize;
            if !full {
                g.insert(black_box(key), 0);
            }
            black_box(full)
        });
    });
}

/// Ascending inserts into maps nobody has removed from, so each split takes
/// a fresh node from the node counter: maps laid out afresh for every 1024
/// keys, the layout and the unmapping outside the timing.
fn insert_fresh(c: &mut Criterion) {
    let path = tmp_path("fresh");
    c.bench_function("btree.insert_fresh/mmf", |b| {
        b.iter_custom(|iters| {
            let mut timed = std::time::Duration::ZERO;
            let mut left = iters;
            while left > 0 {
                let n = left.min(1024);
                let bt: SharedBTreeMap<u32, u32> = SharedBTreeMap::reset(&path, 256).unwrap();
                let start = std::time::Instant::now();
                for k in 0..n as u32 {
                    bt.insert(black_box(k), k).unwrap();
                }
                timed += start.elapsed();
                drop(bt);
                left -= n;
            }
            timed
        });
    });
    remove(&path);

    let path = tmp_path("fresh-raw");
    c.bench_function("btree.insert_fresh/raw", |b| {
        b.iter_custom(|iters| {
            let mut timed = std::time::Duration::ZERO;
            let mut left = iters;
            while left > 0 {
                let n = left.min(1024);
                let m = RawBTreeMap::reset(&path, 256, 4, 4, TAG).unwrap();
                let start = std::time::Instant::now();
                for k in 0..n as u32 {
                    m.insert(black_box(&k.to_be_bytes()), &VALUE, None).unwrap();
                }
                timed += start.elapsed();
                drop(m);
                left -= n;
            }
            timed
        });
    });
    remove(&path);

    c.bench_function("btree.insert_fresh/mutex_btreemap", |b| {
        b.iter_custom(|iters| {
            let mut timed = std::time::Duration::ZERO;
            let mut left = iters;
            while left > 0 {
                let n = left.min(1024);
                let m: Mutex<BTreeMap<u32, u32>> = Mutex::new(BTreeMap::new());
                let start = std::time::Instant::now();
                for k in 0..n as u32 {
                    m.lock().unwrap().insert(black_box(k), k);
                }
                timed += start.elapsed();
                drop(m);
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

criterion_group!(benches,
    get_small,
    iter_small,
    get_large,
    insert_refused,
    insert_fresh,
);
criterion_main!(benches);
