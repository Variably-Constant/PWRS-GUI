//! Bench: SharedCell reads and writes against an `RwLock<T>` baseline,
//! hot and cold, alone and against a live writer or live readers.
//!
//! Workloads:
//! - get and set of a 24-byte value, hot, one thread, against `RwLock`
//! - get and set of a 32-byte value, hot, one thread
//! - get cold: one read from each of 512 cells in turn, more cells' lines
//!   than L1 holds, so a read takes its lines from L2
//! - get under a writer: reads while another thread sets as fast as it can
//! - set under readers: writes while three threads get as fast as they can
//!
//! Only `create`, `set` and `get` are used, so the same file times the cell
//! of any commit.

// clippy::disallowed_types is allowed here: this bench measures against lock-based baselines.
#![allow(clippy::disallowed_types)]

use std::hint::black_box;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::RwLock;

use criterion::{Criterion, criterion_group, criterion_main};

use subetha_cxc::shared_cell::SharedCell;

/// Cells the cold read walks: 512 mappings of a page each, whose two hot
/// lines apiece are 64 KiB, twice a 32 KiB L1, and whose open files stay
/// under a 1024-descriptor limit beside the harness's own.
const COLD_CELLS: usize = 512;
/// Threads reading while the measured thread writes.
const READERS: usize = 3;

fn tmp(name: &str) -> PathBuf {
    let mut p = std::env::temp_dir();
    let pid = std::process::id();
    p.push(format!("subetha-bench-cell-{name}-{pid}.bin"));
    p
}

/// Remove a bench file, saying so when the system refuses.
fn remove(path: &Path) {
    if let Err(e) = std::fs::remove_file(path) {
        eprintln!("could not remove {}: {e}", path.display());
    }
}

#[derive(Clone, Copy, Debug)]
#[repr(C)]
struct State {
    epoch: u64,
    counter: u64,
    flags: u32,
    pad: u32,
}

/// A 32-byte value.
#[derive(Clone, Copy, Debug)]
#[repr(C)]
struct Wide {
    words: [u64; 4],
}

/// A 48-byte value.
#[derive(Clone, Copy, Debug)]
#[repr(C)]
struct Wider {
    words: [u64; 6],
}

/// A 52-byte value, the largest a cell held before values of 56 bytes.
#[derive(Clone, Copy, Debug)]
#[repr(C)]
struct Widest {
    words: [u32; 13],
}

fn get_workload(c: &mut Criterion) {
    let mut g = c.benchmark_group("cell.get");

    let rw = RwLock::new(State { epoch: 1, counter: 100, flags: 0, pad: 0 });
    g.bench_function("native_rwlock_struct", |b| {
        b.iter(|| {
            let s = rw.read().unwrap();
            black_box(*s)
        });
    });

    let path = tmp("get");
    let cell: SharedCell<State> = SharedCell::create(&path).unwrap();
    cell.set(State { epoch: 1, counter: 100, flags: 0, pad: 0 });
    g.bench_function("shared_cell_struct", |b| {
        b.iter(|| black_box(cell.get()));
    });
    drop(cell);
    remove(&path);

    let path = tmp("get-wide");
    let cell: SharedCell<Wide> = SharedCell::create(&path).unwrap();
    cell.set(Wide { words: [1, 2, 3, 4] });
    g.bench_function("shared_cell_32_bytes", |b| {
        b.iter(|| black_box(cell.get()));
    });
    drop(cell);
    remove(&path);

    let path = tmp("get-wider");
    let cell: SharedCell<Wider> = SharedCell::create(&path).unwrap();
    cell.set(Wider { words: [1, 2, 3, 4, 5, 6] });
    g.bench_function("shared_cell_48_bytes", |b| {
        b.iter(|| black_box(cell.get()));
    });
    drop(cell);
    remove(&path);

    let path = tmp("get-widest");
    let cell: SharedCell<Widest> = SharedCell::create(&path).unwrap();
    cell.set(Widest { words: [7; 13] });
    g.bench_function("shared_cell_52_bytes", |b| {
        b.iter(|| black_box(cell.get()));
    });
    drop(cell);
    remove(&path);

    g.finish();
}

fn set_workload(c: &mut Criterion) {
    let mut g = c.benchmark_group("cell.set");

    let rw = RwLock::new(State { epoch: 0, counter: 0, flags: 0, pad: 0 });
    g.bench_function("native_rwlock_struct", |b| {
        let mut i = 0u64;
        b.iter(|| {
            i = i.wrapping_add(1);
            let mut s = rw.write().unwrap();
            *s = State { epoch: i, counter: i * 10, flags: 0, pad: 0 };
        });
    });

    let path = tmp("set");
    let cell: SharedCell<State> = SharedCell::create(&path).unwrap();
    g.bench_function("shared_cell_struct", |b| {
        let mut i = 0u64;
        b.iter(|| {
            i = i.wrapping_add(1);
            cell.set(State { epoch: i, counter: i * 10, flags: 0, pad: 0 });
        });
    });
    drop(cell);
    remove(&path);

    let path = tmp("set-wide");
    let cell: SharedCell<Wide> = SharedCell::create(&path).unwrap();
    g.bench_function("shared_cell_32_bytes", |b| {
        let mut i = 0u64;
        b.iter(|| {
            i = i.wrapping_add(1);
            cell.set(Wide { words: [i; 4] });
        });
    });
    drop(cell);
    remove(&path);

    let path = tmp("set-wider");
    let cell: SharedCell<Wider> = SharedCell::create(&path).unwrap();
    g.bench_function("shared_cell_48_bytes", |b| {
        let mut i = 0u64;
        b.iter(|| {
            i = i.wrapping_add(1);
            cell.set(Wider { words: [i; 6] });
        });
    });
    drop(cell);
    remove(&path);

    let path = tmp("set-widest");
    let cell: SharedCell<Widest> = SharedCell::create(&path).unwrap();
    g.bench_function("shared_cell_52_bytes", |b| {
        let mut i = 0u32;
        b.iter(|| {
            i = i.wrapping_add(1);
            cell.set(Widest { words: [i; 13] });
        });
    });
    drop(cell);
    remove(&path);

    g.finish();
}

fn cold_workload(c: &mut Criterion) {
    let mut g = c.benchmark_group("cell.cold");
    let paths: Vec<_> = (0..COLD_CELLS).map(|i| tmp(&format!("cold-{i}"))).collect();
    let cells: Vec<SharedCell<State>> = paths
        .iter()
        .map(|p| {
            let cell = SharedCell::create(p).unwrap();
            cell.set(State { epoch: 1, counter: 2, flags: 3, pad: 0 });
            cell
        })
        .collect();
    g.bench_function("shared_cell_struct_get", |b| {
        let mut at = 0usize;
        b.iter(|| {
            at = (at + 1) % COLD_CELLS;
            black_box(cells[at].get())
        });
    });
    drop(cells);
    for p in &paths {
        remove(p);
    }
    g.finish();
}

fn contended_workload(c: &mut Criterion) {
    let mut g = c.benchmark_group("cell.contended");

    let path = tmp("under-writer");
    let cell: SharedCell<State> = SharedCell::create(&path).unwrap();
    let stop = AtomicBool::new(false);
    std::thread::scope(|s| {
        s.spawn(|| {
            let mut i = 0u64;
            while !stop.load(Ordering::Relaxed) {
                i = i.wrapping_add(1);
                cell.set(State { epoch: i, counter: i, flags: 0, pad: 0 });
            }
        });
        g.bench_function("get_under_a_writer", |b| {
            b.iter(|| black_box(cell.get()));
        });
        stop.store(true, Ordering::Relaxed);
    });
    drop(cell);
    remove(&path);

    let path = tmp("under-readers");
    let cell: SharedCell<State> = SharedCell::create(&path).unwrap();
    let stop = AtomicBool::new(false);
    std::thread::scope(|s| {
        for _ in 0..READERS {
            s.spawn(|| {
                while !stop.load(Ordering::Relaxed) {
                    black_box(cell.get());
                }
            });
        }
        g.bench_function("set_under_three_readers", |b| {
            let mut i = 0u64;
            b.iter(|| {
                i = i.wrapping_add(1);
                cell.set(State { epoch: i, counter: i, flags: 0, pad: 0 });
            });
        });
        stop.store(true, Ordering::Relaxed);
    });
    drop(cell);
    remove(&path);

    g.finish();
}

criterion_group!(benches, get_workload, set_workload, cold_workload, contended_workload);
criterion_main!(benches);
