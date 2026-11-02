//! Bench: RawArena's write cycle (allocate, write, publish, retire) and its
//! read, and SharedNamedValues' set and get over it, beside a std Vec push
//! and pop no arena change touches, which reads the box.

use std::hint::black_box;
use std::path::{Path, PathBuf};

use criterion::{criterion_group, criterion_main, Criterion};

use subetha_cxc::raw_arena::RawArena;
use subetha_cxc::shared_epochs::SharedEpochs;
use subetha_cxc::shared_named_values::{NamedValuesLayout, SharedNamedValues};

/// Bytes of block space, and the classes: 64 to 256-byte blocks.
const CAPACITY: usize = 1 << 20;
const MIN_CLASS: u32 = 6;
const MAX_CLASS: u32 = 8;
/// Pin slots of the arena's epoch table.
const PINS: usize = 4;

fn tmp(name: &str) -> PathBuf {
    std::env::temp_dir().join(format!("subetha-bench-arena-{name}-{}", std::process::id()))
}

fn remove(path: &Path) {
    match std::fs::remove_file(path) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => panic!("{} was not removed: {e}", path.display()),
    }
}

fn arena_workload(c: &mut Criterion) {
    let mut g = c.benchmark_group("arena");

    let path = tmp("cycle");
    remove(&path);
    let arena = RawArena::create(&path, CAPACITY, MIN_CLASS, MAX_CLASS).unwrap();
    let epochs = SharedEpochs::create_anon(PINS).unwrap();
    let payload = [7u8; 48];
    g.bench_function("cycle/raw", |b| {
        b.iter(|| {
            let h = arena.allocate(payload.len(), &epochs).unwrap();
            arena.write_payload(h, &payload).unwrap();
            arena.publish(h).unwrap();
            black_box(arena.retire(h, &epochs).unwrap())
        });
    });

    let held = arena.allocate(payload.len(), &epochs).unwrap();
    arena.write_payload(held, &payload).unwrap();
    arena.publish(held).unwrap();
    let mut out = Vec::with_capacity(payload.len());
    g.bench_function("read/raw", |b| {
        b.iter(|| black_box(arena.read_into(black_box(held), &mut out).unwrap()));
    });
    drop(arena);
    remove(&path);

    let mut v: Vec<u64> = Vec::with_capacity(16);
    g.bench_function("control/std_vec_push_pop", |b| {
        b.iter(|| {
            v.push(black_box(1));
            black_box(v.pop())
        });
    });

    g.finish();
}

fn named_workload(c: &mut Criterion) {
    let mut g = c.benchmark_group("named");

    let dir = tmp("named");
    match std::fs::create_dir(&dir) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
        Err(e) => panic!("{} was not made: {e}", dir.display()),
    }
    let layout = NamedValuesLayout { names: 64, arena_bytes: CAPACITY, min_class: MIN_CLASS, max_class: MAX_CLASS, pins: PINS };
    let store = SharedNamedValues::create(&dir, "bench", layout).unwrap();
    let value = [9u8; 32];
    store.set("bench", &value).unwrap();
    g.bench_function("set/raw", |b| {
        b.iter(|| store.set(black_box("bench"), black_box(&value)).unwrap());
    });
    g.bench_function("get/raw", |b| {
        b.iter(|| black_box(store.get(black_box("bench")).unwrap()));
    });
    drop(store);
    match std::fs::remove_dir_all(&dir) {
        Ok(()) => {}
        Err(e) => panic!("{} was not removed: {e}", dir.display()),
    }

    g.finish();
}

criterion_group!(benches, arena_workload, named_workload);
criterion_main!(benches);
