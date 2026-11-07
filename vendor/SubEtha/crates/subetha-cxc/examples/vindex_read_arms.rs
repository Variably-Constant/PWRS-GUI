//! One `get_at` arm of the `shared_versioned_index` bench as a plain
//! program, so hardware counters can be read per read.
//!
//! Builds the store the bench's `get_at` arm reads - `KEYS` keys written in
//! ascending order in one statement - for one arm, then reads every key
//! `passes` times at one pin in the bench's shuffled order and prints the
//! read count and the wall time per read. Run it under `perf stat` once with
//! the passes wanted and once with zero passes: the difference is the reads
//! alone, and dividing by the read count gives each event per read.
//!
//! ```text
//! cargo run --release -p subetha-cxc --example vindex_read_arms -- index 64
//! cargo run --release -p subetha-cxc --example vindex_read_arms -- skip 64
//! cargo run --release -p subetha-cxc --example vindex_read_arms -- map 64
//! ```
//!
//! `skip` is the bench's `SkipIndex`, `lql-core`'s versions keyed
//! `(key, born)` newest first in a crossbeam skip list, with the bench's
//! read path.

use std::cmp::Reverse;
use std::hint::black_box;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;

use crossbeam_skiplist::SkipMap;

use subetha_cxc::shared_epochs::{Epoch, PinGuard, SharedEpochs};
use subetha_cxc::versioned_btree_map::DIED_LIVE;
use subetha_cxc::{SharedVersionedIndex, VersionedBTreeMap};

const KEYS: u64 = 16_384;
const PINS: usize = 64;
const MAP_CAPACITY: usize = 1 << 18;

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

/// A version and the epoch it was superseded at, as the bench's
/// `SkipIndex` keeps one.
struct Slot {
    died: AtomicU64,
    value: u64,
}

/// Whether a reader at `pin` sees a version born `born` and superseded at
/// `died`, as the bench decides it.
fn visible_to(born: Epoch, died: Epoch, pin: Epoch) -> bool {
    born <= pin && pin < died
}

/// The bench's `SkipIndex` read path over a store filled with one version
/// per key.
struct SkipIndex {
    map: SkipMap<(u64, Reverse<Epoch>), Slot>,
}

impl SkipIndex {
    fn get_at(&self, key: u64, pin: Epoch) -> Option<u64> {
        self.map
            .range((key, Reverse(Epoch::MAX))..=(key, Reverse(0)))
            .find(|e| visible_to(e.key().1.0, e.value().died.load(Ordering::Acquire), pin))
            .map(|e| e.value().value)
    }
}

/// Read every key in `order` `passes` times through `get`, and answer the
/// reads made and the nanoseconds each took.
fn read_passes(order: &[u64], passes: u64, mut get: impl FnMut(u64) -> bool) -> (u64, f64) {
    let start = Instant::now();
    let mut found = 0u64;
    for _ in 0..passes {
        for &k in order {
            found += get(k) as u64;
        }
    }
    let spent = start.elapsed();
    let reads = passes * order.len() as u64;
    assert_eq!(black_box(found), reads, "every key is found");
    (reads, spent.as_nanos() as f64 / reads.max(1) as f64)
}

fn fresh_dir(arm: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("subetha-vindex-arms-{arm}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("the arm's directory is made");
    dir
}

fn remove(dir: &Path) {
    std::fs::remove_dir_all(dir).expect("the arm's directory is removed once its store is dropped");
}

fn report(arm: &str, passes: u64, reads: u64, ns: f64) {
    println!("arm {arm} passes {passes} reads {reads} ns_per_read {ns:.1}");
}

fn main() {
    let mut args = std::env::args().skip(1);
    let arm = args.next().expect("an arm: index, skip or map");
    let passes: u64 = args.next().map_or(64, |p| p.parse().expect("passes is a whole number"));
    let order = shuffled();
    let dir = fresh_dir(&arm);
    match arm.as_str() {
        "index" => {
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
            let pin = index.epochs().pin().expect("a pin");
            let (reads, ns) = read_passes(&order, passes, |k| index.get_at(&k, &pin, None).expect("a read").is_some());
            report(&arm, passes, reads, ns);
            drop(pin);
            drop(index);
        }
        "skip" => {
            let epochs = SharedEpochs::create(dir.join("epochs"), PINS).expect("the epoch table is made");
            let skip = SkipIndex { map: SkipMap::new() };
            {
                let pin = epochs.pin().expect("a pin");
                let ticket = epochs.begin().expect("a ticket");
                for k in 0..KEYS {
                    skip.map.insert((k, Reverse(ticket.epoch())), Slot { died: AtomicU64::new(DIED_LIVE), value: k });
                }
                ticket.publish();
                drop(pin);
            }
            let pin: PinGuard<'_> = epochs.pin().expect("a pin");
            let at = pin.epoch();
            let (reads, ns) = read_passes(&order, passes, |k| skip.get_at(k, at).is_some());
            report(&arm, passes, reads, ns);
            drop(pin);
            drop(epochs);
        }
        "map" => {
            let map: VersionedBTreeMap<u64, u64> =
                VersionedBTreeMap::create(dir.join("map"), MAP_CAPACITY, dir.join("epochs"), PINS).expect("the map is made");
            {
                let epochs = map.epochs();
                let pin = epochs.pin().expect("a pin");
                let ticket = epochs.begin().expect("a ticket");
                for k in 0..KEYS {
                    map.insert_at(k, k, ticket.epoch()).expect("a fill nobody races");
                }
                ticket.publish();
                drop(pin);
            }
            let pin = map.epochs().pin().expect("a pin");
            let (reads, ns) = read_passes(&order, passes, |k| map.get_at(&k, &pin).is_some());
            report(&arm, passes, reads, ns);
            drop(pin);
            drop(map);
        }
        other => panic!("unknown arm {other:?}: index, skip or map"),
    }
    remove(&dir);
}
