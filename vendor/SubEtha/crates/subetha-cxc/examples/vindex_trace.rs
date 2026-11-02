//! Where a `SharedVersionedIndex::get_at` spends its time, counted.
//!
//! Builds the store the `shared_versioned_index` bench's `get_at` arm reads:
//! `KEYS` keys written in ascending order in one statement, then read back at
//! one pin in the bench's shuffled order. With the `trace-index` feature the
//! index counts, per read, the levels its search walked, the keys it visited
//! and compared, the versions it examined, and the timestamp ticks spent
//! searching against those spent picking the version. This prints each per
//! read, and the ticks in nanoseconds against the wall clock.
//!
//! ```text
//! cargo run --release -p subetha-cxc --features trace-index --example vindex_trace
//! ```

use std::path::PathBuf;
use std::time::Instant;

use subetha_cxc::shared_versioned_index::trace;
use subetha_cxc::SharedVersionedIndex;

const KEYS: u64 = 16_384;
const PINS: usize = 64;
/// Passes over every key after the warm pass; each is `KEYS` reads.
const PASSES: u64 = 64;

/// The bench's fixed shuffled order of `0..KEYS`.
fn shuffled() -> Vec<u64> {
    let mut keys: Vec<u64> = (0..KEYS).collect();
    let mut state = 0x9E37_79B9_7F4A_7C15u64;
    for i in (1..keys.len()).rev() {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        let j = (state % (i as u64 + 1)) as usize;
        keys.swap(i, j);
    }
    keys
}

fn per_read(total: u64, reads: u64) -> f64 {
    total as f64 / reads as f64
}

fn main() {
    let dir = std::env::temp_dir().join(format!("subetha-vindex-trace-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("the trace directory is made");
    let index: SharedVersionedIndex<u64, u64> =
        SharedVersionedIndex::create(dir.join("index"), dir.join("epochs"), PINS).expect("the index is made");
    {
        let epochs = index.epochs();
        let pin = epochs.pin().expect("a pin");
        let ticket = epochs.begin().expect("a ticket");
        for k in 0..KEYS {
            index.write(k, Some(k), &ticket, &pin).expect("a fill nobody races");
        }
        ticket.publish();
    }

    let order = shuffled();
    let pin = index.epochs().pin().expect("a pin");
    let read_all = || {
        let mut found = 0u64;
        for &k in &order {
            found += index.get_at(&k, &pin, None).expect("a read").is_some() as u64;
        }
        assert_eq!(found, KEYS, "every key is found");
    };

    read_all();
    let fill_and_warm = trace::take();

    let wall = Instant::now();
    let tsc = trace_ticks();
    for _ in 0..PASSES {
        read_all();
    }
    let tsc = trace_ticks().wrapping_sub(tsc);
    let wall = wall.elapsed();
    let c = trace::take();

    let reads = KEYS * PASSES;
    let ns_per_tick = wall.as_nanos() as f64 / tsc.max(1) as f64;
    println!("keys {KEYS}, passes {PASSES}, reads {reads}; fill and warm pass: {} searches", fill_and_warm.finds);
    println!("wall {:.1} ns per read; {:.4} ns per timestamp tick", wall.as_nanos() as f64 / reads as f64, ns_per_tick);
    println!(
        "searches {} (reads {}), retries {}, unlinks {}",
        c.finds, c.reads, c.retries, c.unlinks
    );
    println!(
        "per read: levels {:.2}, hops {:.2} (tall {:.2}), compares {:.2}, versions {:.3}",
        per_read(c.levels, reads),
        per_read(c.hops, reads),
        per_read(c.tall_hops, reads),
        per_read(c.compares, reads),
        per_read(c.versions, reads),
    );
    println!(
        "per read: {:.2} levels ended at the key that ended the level above, so {:.2} hops on this store without that",
        per_read(c.stops, reads),
        per_read(c.hops + c.stops, reads),
    );
    println!("hops per level walked: {:.2}", c.hops as f64 / c.levels.max(1) as f64);
    println!(
        "per read: search {:.1} ns, version {:.1} ns ({:.0} and {:.0} ticks)",
        per_read(c.search_ticks, reads) * ns_per_tick,
        per_read(c.version_ticks, reads) * ns_per_tick,
        per_read(c.search_ticks, reads),
        per_read(c.version_ticks, reads),
    );

    drop(pin);
    drop(index);
    remove(dir);
}

/// The timestamp counter the index's trace reads, or zero off x86_64.
fn trace_ticks() -> u64 {
    #[cfg(target_arch = "x86_64")]
    // SAFETY: RDTSC exists on every x86_64 processor and only reads a
    // counter.
    #[allow(unused_unsafe)]
    let t = unsafe { std::arch::x86_64::_rdtsc() };
    #[cfg(not(target_arch = "x86_64"))]
    let t = 0;
    t
}

fn remove(dir: PathBuf) {
    std::fs::remove_dir_all(&dir).expect("the trace directory is removed once the index is dropped");
}
