//! Bench: SharedTreiberStack vs Mutex<Vec<T>> (textbook in-process
//! stack baseline). The refusal and fresh-push cells also time
//! RawTreiberStack, the same stack at a slot size chosen at run time.

// clippy::disallowed_types is allowed here: this bench measures against lock-based baselines.
#![allow(clippy::disallowed_types)]

use std::hint::black_box;
use std::sync::Mutex;

use criterion::{criterion_group, criterion_main, Criterion};

use subetha_cxc::raw_treiber_stack::{ElementLayout, RawTreiberStack};
use subetha_cxc::SharedTreiberStack;

/// The raw stack's element: a u64's bytes.
const ELEMENT: ElementLayout = ElementLayout { slot_size: 8, alignment: 8, tag: 1 };
const PAYLOAD: [u8; 8] = 42u64.to_le_bytes();

fn tmp(name: &str) -> std::path::PathBuf {
    let mut p = std::env::temp_dir();
    let pid = std::process::id();
    p.push(format!("subetha-bench-stack-{name}-{pid}.bin"));
    p
}

fn push_hot(c: &mut Criterion) {
    let p = tmp("push");
    let s: SharedTreiberStack<u64> = SharedTreiberStack::create(&p, 1 << 20).unwrap();
    c.bench_function("stack.push/mmf", |b| {
        b.iter(|| {
            if s.push(black_box(42)).is_err() {
                while s.pop().is_some() {}
            }
        });
    });
    drop(s);
    remove(&p);

    // Cleared at the stack's capacity, so the vec never reallocates.
    let m: Mutex<Vec<u64>> = Mutex::new(Vec::with_capacity(1 << 20));
    c.bench_function("stack.push/mutex_vec", |b| {
        b.iter(|| {
            let mut v = m.lock().unwrap();
            if v.len() == 1 << 20 {
                v.clear();
            }
            v.push(black_box(42));
        });
    });
}

fn push_pop_cycle(c: &mut Criterion) {
    let p = tmp("cycle");
    let s: SharedTreiberStack<u64> = SharedTreiberStack::create(&p, 16).unwrap();
    c.bench_function("stack.push_pop_cycle/mmf", |b| {
        b.iter(|| {
            s.push(black_box(42)).unwrap();
            black_box(s.pop().unwrap())
        });
    });
    drop(s);
    remove(&p);

    let m: Mutex<Vec<u64>> = Mutex::new(Vec::with_capacity(16));
    c.bench_function("stack.push_pop_cycle/mutex_vec", |b| {
        b.iter(|| {
            m.lock().unwrap().push(black_box(42));
            black_box(m.lock().unwrap().pop().unwrap())
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

/// A push into a full stack: the refusal at the node counter. The control
/// refuses at a length check under its lock.
fn push_refused(c: &mut Criterion) {
    let p = tmp("refused");
    let s: SharedTreiberStack<u64> = SharedTreiberStack::create(&p, 16).unwrap();
    while s.push(0).is_ok() {}
    c.bench_function("stack.push_refused/mmf", |b| {
        b.iter(|| black_box(s.push(black_box(42)).is_err()));
    });
    drop(s);
    remove(&p);

    let p = tmp("refused-raw");
    let s = RawTreiberStack::create(&p, 16, ELEMENT).unwrap();
    while s.push(&PAYLOAD).is_ok() {}
    c.bench_function("stack.push_refused/raw", |b| {
        b.iter(|| black_box(s.push(black_box(&PAYLOAD)).is_err()));
    });
    drop(s);
    remove(&p);

    let m: Mutex<Vec<u64>> = Mutex::new(vec![0; 16]);
    c.bench_function("stack.push_refused/mutex_vec", |b| {
        b.iter(|| {
            let mut v = m.lock().unwrap();
            let full = v.len() >= 16;
            if !full {
                v.push(black_box(42));
            }
            black_box(full)
        });
    });
}

/// A push into a stack nobody has popped from, its node a fresh one from
/// the node counter: stacks of 1024 laid out afresh, the layout and the
/// unmapping outside the timing.
fn push_fresh(c: &mut Criterion) {
    let p = tmp("fresh");
    c.bench_function("stack.push_fresh/mmf", |b| {
        b.iter_custom(|iters| {
            let mut timed = std::time::Duration::ZERO;
            let mut left = iters;
            while left > 0 {
                let n = left.min(1024);
                let s = SharedTreiberStack::<u64>::reset(&p, 1024).unwrap();
                let start = std::time::Instant::now();
                for i in 0..n {
                    s.push(black_box(i)).unwrap();
                }
                timed += start.elapsed();
                drop(s);
                left -= n;
            }
            timed
        });
    });
    remove(&p);

    let p = tmp("fresh-raw");
    c.bench_function("stack.push_fresh/raw", |b| {
        b.iter_custom(|iters| {
            let mut timed = std::time::Duration::ZERO;
            let mut left = iters;
            while left > 0 {
                let n = left.min(1024);
                let s = RawTreiberStack::reset(&p, 1024, ELEMENT).unwrap();
                let start = std::time::Instant::now();
                for i in 0..n {
                    s.push(black_box(&i.to_le_bytes())).unwrap();
                }
                timed += start.elapsed();
                drop(s);
                left -= n;
            }
            timed
        });
    });
    remove(&p);

    c.bench_function("stack.push_fresh/mutex_vec", |b| {
        b.iter_custom(|iters| {
            let mut timed = std::time::Duration::ZERO;
            let mut left = iters;
            while left > 0 {
                let n = left.min(1024);
                let m = Mutex::new(Vec::<u64>::with_capacity(1024));
                let start = std::time::Instant::now();
                for i in 0..n {
                    m.lock().unwrap().push(black_box(i));
                }
                timed += start.elapsed();
                drop(m);
                left -= n;
            }
            timed
        });
    });
}

// Multi-thread push/pop correctness is covered by source-level
// unit tests. A per-iter thread::spawn microbench is dominated
// by Windows thread-creation cost.

criterion_group!(benches, push_hot, push_pop_cycle, push_refused, push_fresh);
criterion_main!(benches);
