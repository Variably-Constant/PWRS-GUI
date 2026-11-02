//! Bench: SharedLRUCache vs Mutex<(HashMap, VecDeque)> textbook
//! in-process LRU baseline.
//!
//! Workloads:
//! - get hot (a read that leaves the entry's mark)
//! - put new (insert that doesn't evict)
//! - get_and_touch (a read that marks the entry)
//! - put with eviction (full cache + new key)

// clippy::disallowed_types is allowed here: this bench measures against lock-based baselines.
#![allow(clippy::disallowed_types)]

use std::collections::{HashMap, VecDeque};
use std::hint::black_box;
use std::sync::Mutex;

use criterion::{criterion_group, criterion_main, Criterion};

use subetha_cxc::SharedLRUCache;

fn tmp_base(name: &str) -> std::path::PathBuf {
    let mut p = std::env::temp_dir();
    let pid = std::process::id();
    p.push(format!("subetha-bench-lru-{name}-{pid}"));
    p
}

/// Remove every file a cache at `base` can leave: the single cache file,
/// and the map and list files of the two-file layout an A/B baseline uses.
/// An absent file is expected; any other failure stops the bench, since a
/// cache left behind is attached to by the next window with its entries.
fn cleanup(base: &std::path::Path) {
    let stem = base.file_name().expect("a bench base has a file name").to_string_lossy().to_string();
    for ext in ["cache", "map", "list"] {
        let mut p = base.to_path_buf();
        p.set_file_name(format!("{stem}.{ext}.bin"));
        match std::fs::remove_file(&p) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => panic!("could not clear {}: {e}", p.display()),
        }
    }
}

// Textbook in-process LRU: HashMap + VecDeque (recency order) inside a Mutex.
struct MutexLRU {
    inner: Mutex<MutexLRUInner>,
    capacity: usize,
}
struct MutexLRUInner {
    map: HashMap<u32, u32>,
    order: VecDeque<u32>,  // front = MRU, back = LRU
}

impl MutexLRU {
    fn new(capacity: usize) -> Self {
        Self {
            inner: Mutex::new(MutexLRUInner {
                map: HashMap::with_capacity(capacity),
                order: VecDeque::with_capacity(capacity),
            }),
            capacity,
        }
    }
    fn get(&self, k: u32) -> Option<u32> {
        self.inner.lock().unwrap().map.get(&k).copied()
    }
    fn get_and_touch(&self, k: u32) -> Option<u32> {
        let mut g = self.inner.lock().unwrap();
        let v = g.map.get(&k).copied()?;
        // Move k to front of order.
        if let Some(pos) = g.order.iter().position(|&x| x == k) {
            g.order.remove(pos);
            g.order.push_front(k);
        }
        Some(v)
    }
    fn put(&self, k: u32, v: u32) {
        let mut g = self.inner.lock().unwrap();
        if let std::collections::hash_map::Entry::Occupied(mut e) = g.map.entry(k) {
            e.insert(v);
            if let Some(pos) = g.order.iter().position(|&x| x == k) {
                g.order.remove(pos);
            }
            g.order.push_front(k);
            return;
        }
        if g.map.len() >= self.capacity
            && let Some(evict) = g.order.pop_back() {
                g.map.remove(&evict);
            }
        g.map.insert(k, v);
        g.order.push_front(k);
    }
}

// =========================================================
// get hot (leaves the mark)
// =========================================================

fn get_hot(c: &mut Criterion) {
    let base = tmp_base("get");
    cleanup(&base);
    let cache: SharedLRUCache<u32, u32>
        = SharedLRUCache::create(&base, 1000).unwrap();
    for k in 0..1000u32 { cache.put(k, k * 10).unwrap(); }
    c.bench_function("lru.get/mmf", |b| {
        b.iter(|| black_box(cache.get(&black_box(500u32))));
    });
    drop(cache);
    cleanup(&base);

    let m = MutexLRU::new(1000);
    for k in 0..1000u32 { m.put(k, k * 10); }
    c.bench_function("lru.get/mutex_hashmap_vecdeque", |b| {
        b.iter(|| black_box(m.get(black_box(500u32))));
    });
}

// =========================================================
// put new (no eviction)
// =========================================================

fn put_new(c: &mut Criterion) {
    // Criterion at publication-grade defaults on fast silicon measures
    // hundreds of millions of iterations, more new keys than the cache
    // holds, so each window of at most the capacity starts from a fresh
    // cache and every put lands a new key without evicting.
    c.bench_function("lru.put/mmf", |b| {
        let base = tmp_base("put-new");
        const CAP_PER_WINDOW: u32 = 100_000;
        b.iter_custom(|iters| {
            use std::time::{Duration, Instant};
            let mut total = Duration::ZERO;
            let mut remaining = iters as u32;
            let mut k: u32 = 0;
            while remaining > 0 {
                cleanup(&base);
                let cache: SharedLRUCache<u32, u32>
                    = SharedLRUCache::create(&base, 100_000).unwrap();
                let batch = remaining.min(CAP_PER_WINDOW);
                let start = Instant::now();
                for _ in 0..batch {
                    k = k.wrapping_add(1);
                    black_box(cache.put(black_box(k), black_box(42)).expect("a put into the bench cache"));
                }
                total += start.elapsed();
                drop(cache);
                cleanup(&base);
                remaining = remaining.saturating_sub(batch);
            }
            total
        });
    });

    let m = MutexLRU::new(100_000);
    let mut k = 0u32;
    c.bench_function("lru.put/mutex_hashmap_vecdeque", |b| {
        b.iter(|| {
            k = k.wrapping_add(1);
            m.put(black_box(k), black_box(42));
        });
    });
}

// =========================================================
// get_and_touch (read + mark)
// =========================================================

fn get_and_touch(c: &mut Criterion) {
    let base = tmp_base("touch");
    cleanup(&base);
    let cache: SharedLRUCache<u32, u32>
        = SharedLRUCache::create(&base, 100).unwrap();
    for k in 0..100u32 { cache.put(k, k * 10).unwrap(); }
    c.bench_function("lru.get_and_touch/mmf", |b| {
        b.iter(|| black_box(cache.get_and_touch(&black_box(50u32))));
    });
    drop(cache);
    cleanup(&base);

    let m = MutexLRU::new(100);
    for k in 0..100u32 { m.put(k, k * 10); }
    c.bench_function("lru.get_and_touch/mutex_hashmap_vecdeque", |b| {
        b.iter(|| black_box(m.get_and_touch(black_box(50u32))));
    });
}

// =========================================================
// put with eviction (full cache, new key)
//
// Each window fills a cache of ten, then puts fifty new keys into it,
// each evicting one.
// =========================================================

fn put_with_eviction(c: &mut Criterion) {
    c.bench_function("lru.put_evict/mmf", |b| {
        let base = tmp_base("put-evict");
        const CAP_PER_WINDOW: u64 = 50;
        b.iter_custom(|iters| {
            use std::time::{Duration, Instant};
            let mut remaining = iters.max(1);
            let mut total = Duration::ZERO;
            let mut k = 100u32;
            while remaining > 0 {
                cleanup(&base);
                let cache: SharedLRUCache<u32, u32>
                    = SharedLRUCache::create(&base, 10).unwrap();
                for k0 in 0..10u32 { cache.put(k0, k0).unwrap(); }
                let batch = remaining.min(CAP_PER_WINDOW);
                let start = Instant::now();
                for _ in 0..batch {
                    k = k.wrapping_add(1);
                    black_box(cache.put(black_box(k), black_box(k)).expect("a put into the bench cache"));
                }
                total += start.elapsed();
                drop(cache);
                cleanup(&base);
                remaining = remaining.saturating_sub(batch);
            }
            total
        });
    });

    c.bench_function("lru.put_evict/mutex_hashmap_vecdeque", |b| {
        let m = MutexLRU::new(10);
        for k in 0..10u32 { m.put(k, k); }
        let mut k = 100u32;
        b.iter(|| {
            k = k.wrapping_add(1);
            m.put(black_box(k), black_box(k));
        });
    });
}

criterion_group!(benches,
    get_hot,
    put_new,
    get_and_touch,
    put_with_eviction,
);
criterion_main!(benches);
