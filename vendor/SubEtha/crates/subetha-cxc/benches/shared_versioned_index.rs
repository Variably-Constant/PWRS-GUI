//! Bench: what `SharedVersionedIndex` costs against the structures a
//! caller would otherwise reach for.
//!
//! The index is an ordered map any number of threads and processes write
//! at once, keeping every version a pinned reader can still need. The
//! contenders:
//!
//! - `SkipIndex`, the in-process stand-in PrismLQL built before the index:
//!   a `crossbeam_skiplist::SkipMap` keyed `(key, Reverse(born))`, each
//!   version carrying an atomic `died`, reproduced below from `lql-core`'s
//!   `versioned_index.rs` with its write, read, range and sweep paths
//!   unchanged. It keeps the same history and takes the same writers, in
//!   one process's heap.
//! - `VersionedBTreeMap`: one entry per key, so an update replaces rather
//!   than versioning, and one writer at a time.
//! - `LanedVersionedMap`: that map split into lanes, one writer a lane.
//!
//! Fairness, against the three questions the house rule asks:
//!
//! 1. Every arm holds the same keys and does the same writes. A statement
//!    takes a pin and a ticket from its structure's `SharedEpochs` table,
//!    writes `BATCH` keys at the ticket's epoch and publishes, so the
//!    epoch table costs every arm the same; the maps write at the epoch
//!    with `insert_at` and `remove_at`.
//! 2. No arm carries surplus work. Every structure is built and filled
//!    before its timing starts, and the sweeps and deletions that keep
//!    history bounded between iterations run outside the timed region.
//! 3. Where the arms differ, the difference is the design under test and
//!    stays in. The index and `SkipIndex` add a version per write and the
//!    maps replace in place, which is the history the maps do not keep;
//!    the index records each write on its ticket's write list, which is
//!    how a dead writer's epoch is taken back.
//!
//! The arms, over a store of `KEYS` keys:
//!
//! - `write`: one statement rewriting `BATCH` keys, one writer.
//! - `get_at`: `BATCH` reads of existing keys at one pin.
//! - `range_at`: `ROWS` rows from a key inside the store, at one pin.
//! - `sweep`: taking back the `BATCH` keys a statement deleted once no pin
//!   can see them, which walks the whole store.
//! - `racing_insert/<n>`: `n` threads at once, each running `STATEMENTS`
//!   statements of `BATCH` inserts of keys nobody else writes. Fresh keys,
//!   because a statement's pin sits below every ticket still open, so a
//!   writer rewriting even its own keys meets versions born after its pin
//!   and is refused; a rewrite race measures how often that happens, which
//!   is the epoch table's contract rather than the structure's cost.
//!   `VersionedBTreeMap` takes one writer and has no racing arm.
//!
//! What the numbers do not show: the index lives in files every attached
//! process maps, and `SkipIndex` in one process's heap. A second process
//! cannot reach `SkipIndex` at any price.

use std::cmp::Reverse;
use std::hint::black_box;
use std::ops::Bound;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use criterion::{criterion_group, criterion_main, Criterion};
use crossbeam_skiplist::SkipMap;

use subetha_cxc::laned_versioned_map::LanedError;
use subetha_cxc::shared_epochs::{Epoch, EpochTicket, PinGuard, SharedEpochs};
use subetha_cxc::versioned_btree_map::{VersionedError, DIED_LIVE};
use subetha_cxc::{LanedVersionedMap, SharedVersionedIndex, VersionedBTreeMap};

/// Keys every store is filled with.
const KEYS: u64 = 16_384;
/// Writes in one statement, and reads in one `get_at` iteration.
const BATCH: u64 = 1_024;
/// Rows one `range_at` iteration reads.
const ROWS: usize = 1_024;
/// Pins and tickets each epoch table holds.
const PINS: usize = 64;
/// Statements each racing writer runs per iteration.
const STATEMENTS: u64 = 16;
/// Lanes the laned map's read and sweep arms spread the store over.
const READ_LANES: usize = 4;
/// Entries a map is sized for: the store and every racing insert.
const MAP_CAPACITY: usize = 1 << 18;

type Index = SharedVersionedIndex<u64, u64>;
type Map = VersionedBTreeMap<u64, u64>;
type Laned = LanedVersionedMap<u64, u64>;

/// A version and the epoch it was superseded at, as `lql-core` keeps one.
struct Slot {
    died: AtomicU64,
    value: u64,
}

/// `lql-core`'s `SkipIndex<u64, u64>`: versions keyed `(key, born)`, the
/// newest first, each carrying an atomic `died`.
struct SkipIndex {
    map: SkipMap<(u64, Reverse<Epoch>), Slot>,
}

/// Whether a reader at `pin` sees a version born `born` and superseded at
/// `died`, as `lql-core` decides it.
fn visible_to(born: Epoch, died: Epoch, pin: Epoch, own: Option<Epoch>) -> bool {
    let mine = |e: Epoch| own == Some(e);
    (born <= pin || mine(born)) && pin < died && !mine(died)
}

fn lower(b: Bound<u64>) -> Bound<(u64, Reverse<Epoch>)> {
    match b {
        Bound::Included(k) => Bound::Included((k, Reverse(Epoch::MAX))),
        Bound::Excluded(k) => Bound::Excluded((k, Reverse(0))),
        Bound::Unbounded => Bound::Unbounded,
    }
}

impl SkipIndex {
    fn new() -> Self {
        SkipIndex { map: SkipMap::new() }
    }

    fn versions_of(&self, key: u64) -> impl Iterator<Item = crossbeam_skiplist::map::Entry<'_, (u64, Reverse<Epoch>), Slot>> {
        self.map.range((key, Reverse(Epoch::MAX))..=(key, Reverse(0)))
    }

    fn insert_fresh(&self, key: u64, value: Option<u64>, epoch: Epoch, pin: Epoch) -> Result<(), Epoch> {
        let Some(value) = value else {
            return Ok(());
        };
        self.map.insert((key, Reverse(epoch)), Slot { died: AtomicU64::new(DIED_LIVE), value });
        let rival = self.versions_of(key).map(|e| e.key().1.0).find(|&b| b != epoch && b > pin);
        if let Some(b) = rival {
            self.map.remove(&(key, Reverse(epoch)));
            return Err(b);
        }
        Ok(())
    }

    fn write(&self, key: u64, value: Option<u64>, epoch: Epoch, pin: Epoch) -> Result<(), Epoch> {
        let Some(newest) = self.versions_of(key).next() else {
            return self.insert_fresh(key, value, epoch, pin);
        };
        let born = newest.key().1.0;
        if born == epoch {
            newest.remove();
            if let Some(value) = value {
                self.map.insert((key, Reverse(epoch)), Slot { died: AtomicU64::new(DIED_LIVE), value });
            }
            return Ok(());
        }
        if born > pin {
            return Err(born);
        }
        let died = newest.value().died.load(Ordering::Acquire);
        if died != DIED_LIVE {
            if died > pin && died != epoch {
                return Err(died);
            }
            return self.insert_fresh(key, value, epoch, pin);
        }
        newest.value().died.compare_exchange(DIED_LIVE, epoch, Ordering::AcqRel, Ordering::Acquire)?;
        if let Some(value) = value {
            self.map.insert((key, Reverse(epoch)), Slot { died: AtomicU64::new(DIED_LIVE), value });
        }
        Ok(())
    }

    fn get_at(&self, key: u64, pin: Epoch) -> Option<u64> {
        self.versions_of(key)
            .find(|e| visible_to(e.key().1.0, e.value().died.load(Ordering::Acquire), pin, None))
            .map(|e| e.value().value)
    }

    fn range_at(&self, low: Bound<u64>, limit: usize, pin: Epoch) -> (Vec<(u64, u64)>, Option<u64>) {
        let mut out = Vec::new();
        let mut last: Option<u64> = None;
        for (examined, e) in self.map.range((lower(low), Bound::Unbounded)).enumerate() {
            let (key, Reverse(born)) = *e.key();
            if examined >= limit && last != Some(key) {
                return (out, last);
            }
            last = Some(key);
            if visible_to(born, e.value().died.load(Ordering::Acquire), pin, None) {
                out.push((key, e.value().value));
            }
        }
        (out, None)
    }

    fn sweep(&self, horizon: Epoch) -> usize {
        let mut dropped = 0usize;
        for e in self.map.iter() {
            let died = e.value().died.load(Ordering::Acquire);
            if died != DIED_LIVE && died <= horizon && e.remove() {
                dropped += 1;
            }
        }
        dropped
    }
}

/// `SkipIndex` with the epoch table its statements take pins and tickets
/// from, which the index and the maps carry inside them.
struct Skip {
    index: SkipIndex,
    epochs: SharedEpochs,
}

/// One contender, driven the same way as every other.
trait Arm: Sync {
    fn epochs(&self) -> &SharedEpochs;
    /// Make `value` current at `key`, or delete it for `None`, in the
    /// statement holding `ticket` whose snapshot is `pin`.
    fn put(&self, key: u64, value: Option<u64>, ticket: &EpochTicket<'_>, pin: &PinGuard<'_>);
    fn get(&self, key: u64, pin: &PinGuard<'_>) -> Option<u64>;
    /// Rows read walking `ROWS` keys from `from`.
    fn range(&self, from: u64, pin: &PinGuard<'_>) -> usize;
    fn sweep(&self) -> usize;
}

impl Arm for Index {
    fn epochs(&self) -> &SharedEpochs {
        SharedVersionedIndex::epochs(self)
    }

    fn put(&self, key: u64, value: Option<u64>, ticket: &EpochTicket<'_>, pin: &PinGuard<'_>) {
        if let Err(e) = self.write(key, value, ticket, pin) {
            panic!("the index refused a write nobody raced: {e}");
        }
    }

    fn get(&self, key: u64, pin: &PinGuard<'_>) -> Option<u64> {
        match self.get_at(&key, pin, None) {
            Ok(found) => found,
            Err(e) => panic!("the index refused a read: {e}"),
        }
    }

    fn range(&self, from: u64, pin: &PinGuard<'_>) -> usize {
        match self.range_at(Bound::Included(&from), Bound::Unbounded, ROWS, pin, None) {
            Ok((rows, _)) => rows.len(),
            Err(e) => panic!("the index refused a range: {e}"),
        }
    }

    fn sweep(&self) -> usize {
        match SharedVersionedIndex::sweep(self) {
            Ok(dropped) => dropped,
            Err(e) => panic!("the index refused a sweep: {e}"),
        }
    }
}

impl Arm for Skip {
    fn epochs(&self) -> &SharedEpochs {
        &self.epochs
    }

    fn put(&self, key: u64, value: Option<u64>, ticket: &EpochTicket<'_>, pin: &PinGuard<'_>) {
        if let Err(epoch) = self.index.write(key, value, ticket.epoch(), pin.epoch()) {
            panic!("the skip index refused a write nobody raced: a version from epoch {epoch}");
        }
    }

    fn get(&self, key: u64, pin: &PinGuard<'_>) -> Option<u64> {
        self.index.get_at(key, pin.epoch())
    }

    fn range(&self, from: u64, pin: &PinGuard<'_>) -> usize {
        self.index.range_at(Bound::Included(from), ROWS, pin.epoch()).0.len()
    }

    fn sweep(&self) -> usize {
        self.index.sweep(self.epochs.reclaim_horizon())
    }
}

impl Arm for Map {
    fn epochs(&self) -> &SharedEpochs {
        VersionedBTreeMap::epochs(self)
    }

    fn put(&self, key: u64, value: Option<u64>, ticket: &EpochTicket<'_>, _pin: &PinGuard<'_>) {
        let done = match value {
            Some(v) => self.insert_at(key, v, ticket.epoch()).map(|_| ()),
            None => self.remove_at(&key, ticket.epoch()).map(|_| ()),
        };
        if let Err(e) = done {
            panic!("the map refused a write: {e}");
        }
    }

    fn get(&self, key: u64, pin: &PinGuard<'_>) -> Option<u64> {
        self.get_at(&key, pin)
    }

    fn range(&self, from: u64, pin: &PinGuard<'_>) -> usize {
        self.range_at(Bound::Included(&from), Bound::Unbounded, ROWS, pin).len()
    }

    fn sweep(&self) -> usize {
        match VersionedBTreeMap::sweep(self) {
            Ok(freed) => freed,
            // The map reports finding nothing to free as Full.
            Err(VersionedError::Full) => 0,
            Err(e) => panic!("the map refused a sweep: {e}"),
        }
    }
}

fn laned_sweep(map: &Laned) -> usize {
    match map.sweep() {
        Ok(freed) => freed,
        Err(LanedError::Versioned(VersionedError::Full)) => 0,
        Err(e) => panic!("the laned map refused a sweep: {e}"),
    }
}

fn dir(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("subetha-bench-vindex-{name}-{}", std::process::id()));
    match std::fs::remove_dir_all(&d) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => panic!("stale bench directory {} not removed: {e}", d.display()),
    }
    std::fs::create_dir_all(&d).expect("the arm's directory is made");
    d
}

/// Remove an arm's directory once its structures are dropped; a refusal
/// is a bench bug, a mapping still alive, not something to leave behind.
fn remove(d: &Path) {
    std::fs::remove_dir_all(d).expect("the arm's structures are dropped and its directory removable");
}

/// The keys `0..KEYS` in one fixed shuffled order, the same for every arm.
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

/// One statement: a pin, then a ticket, then `f`, then the publish.
fn statement<A: Arm + ?Sized>(arm: &A, f: impl FnOnce(&EpochTicket<'_>, &PinGuard<'_>)) {
    let epochs = arm.epochs();
    let pin = epochs.pin().expect("a pin");
    let ticket = epochs.begin().expect("a ticket");
    f(&ticket, &pin);
    ticket.publish();
    drop(pin);
}

fn fill<A: Arm + ?Sized>(arm: &A) {
    statement(arm, |t, p| {
        for k in 0..KEYS {
            arm.put(k, Some(k), t, p);
        }
    });
}

/// The contenders that take statements alike, each filled, in a directory
/// of its own: the index, `SkipIndex`, and with `single_writer` the map,
/// which takes one writer at a time and so sits out the racing arms.
fn contenders(op: &str, single_writer: bool) -> Vec<(&'static str, PathBuf, Box<dyn Arm>)> {
    let d = dir(&format!("{op}-index"));
    let index: Box<dyn Arm> = Box::new(Index::create(d.join("index"), d.join("epochs"), PINS).expect("the index is made"));
    let mut arms = vec![("shared_versioned_index", d, index)];
    let d = dir(&format!("{op}-skip"));
    let skip: Box<dyn Arm> = Box::new(Skip {
        index: SkipIndex::new(),
        epochs: SharedEpochs::create(d.join("epochs"), PINS).expect("the epoch table is made"),
    });
    arms.push(("skip_index", d, skip));
    if single_writer {
        let d = dir(&format!("{op}-map"));
        let map: Box<dyn Arm> =
            Box::new(Map::create(d.join("map"), MAP_CAPACITY, d.join("epochs"), PINS).expect("the map is made"));
        arms.push(("versioned_btree_map", d, map));
    }
    for (_, _, arm) in &arms {
        fill(arm.as_ref());
    }
    arms
}

/// A laned map of `lanes` lanes holding the store, keys dealt round-robin,
/// so every lane spans the whole key range.
fn laned(op: &str, lanes: usize) -> (PathBuf, Laned) {
    let d = dir(&format!("{op}-laned{lanes}"));
    let map = Laned::create(&d, lanes, MAP_CAPACITY / lanes, PINS).expect("the laned map is made");
    {
        let claims: Vec<_> = (0..lanes).map(|_| map.claim_lane().expect("a lane")).collect();
        let pin = map.epochs().pin().expect("a pin");
        let ticket = map.epochs().begin().expect("a ticket");
        for k in 0..KEYS {
            if let Err(e) = claims[(k as usize) % lanes].insert_at(k, k, ticket.epoch()) {
                panic!("the laned map refused a fill: {e}");
            }
        }
        ticket.publish();
        drop(pin);
    }
    (d, map)
}

fn write(c: &mut Criterion) {
    let order = shuffled();
    for (name, d, arm) in contenders("write", true) {
        let next = AtomicUsize::new(0);
        c.bench_function(&format!("vindex.write/{name}"), |b| {
            b.iter_custom(|iters| {
                let mut spent = Duration::ZERO;
                for i in 0..iters {
                    let at = next.fetch_add(BATCH as usize, Ordering::Relaxed) % KEYS as usize;
                    let keys = &order[at..at + BATCH as usize];
                    let start = Instant::now();
                    statement(arm.as_ref(), |t, p| {
                        for &k in keys {
                            arm.put(k, Some(black_box(k ^ i)), t, p);
                        }
                    });
                    spent += start.elapsed();
                    if i % 8 == 7 {
                        arm.sweep();
                    }
                }
                spent
            });
        });
        drop(arm);
        remove(&d);
    }
}

fn get_at(c: &mut Criterion) {
    let order = shuffled();
    for (name, d, arm) in contenders("get", true) {
        let pin = arm.epochs().pin().expect("a pin");
        let next = AtomicUsize::new(0);
        c.bench_function(&format!("vindex.get_at/{name}"), |b| {
            b.iter(|| {
                let at = next.fetch_add(BATCH as usize, Ordering::Relaxed) % KEYS as usize;
                let mut found = 0u64;
                for &k in &order[at..at + BATCH as usize] {
                    found += arm.get(k, &pin).is_some() as u64;
                }
                assert_eq!(found, BATCH, "{name} lost a key");
                black_box(found)
            });
        });
        drop(pin);
        drop(arm);
        remove(&d);
    }
    let (d, map) = laned("get", READ_LANES);
    let pin = map.pin().expect("a pin");
    let next = AtomicUsize::new(0);
    c.bench_function(&format!("vindex.get_at/laned_versioned_map_{READ_LANES}"), |b| {
        b.iter(|| {
            let at = next.fetch_add(BATCH as usize, Ordering::Relaxed) % KEYS as usize;
            let mut found = 0u64;
            for &k in &order[at..at + BATCH as usize] {
                found += map.get_at(&k, &pin).is_some() as u64;
            }
            assert_eq!(found, BATCH, "the laned map lost a key");
            black_box(found)
        });
    });
    drop(pin);
    drop(map);
    remove(&d);
}

fn range_at(c: &mut Criterion) {
    let starts = (KEYS as usize - ROWS) as u64;
    for (name, d, arm) in contenders("range", true) {
        let pin = arm.epochs().pin().expect("a pin");
        assert_eq!(arm.range(0, &pin), ROWS, "{name} reads {ROWS} rows a range");
        let next = AtomicU64::new(0);
        c.bench_function(&format!("vindex.range_at/{name}"), |b| {
            b.iter(|| black_box(arm.range(next.fetch_add(997, Ordering::Relaxed) % starts, &pin)));
        });
        drop(pin);
        drop(arm);
        remove(&d);
    }
    // Each lane walks a quarter of the rows, keys being dealt round-robin,
    // so the merged page is the same ROWS rows the other arms read.
    let (d, map) = laned("range", READ_LANES);
    let pin = map.pin().expect("a pin");
    let page = |from: u64| map.range_at_with_cursor(Bound::Included(&from), Bound::Unbounded, ROWS / READ_LANES, &pin).0.len();
    let first = page(0);
    assert!(first + READ_LANES >= ROWS && first <= ROWS, "the laned map reads {first} rows a range, not about {ROWS}");
    let next = AtomicU64::new(0);
    c.bench_function(&format!("vindex.range_at/laned_versioned_map_{READ_LANES}"), |b| {
        b.iter(|| black_box(page(next.fetch_add(997, Ordering::Relaxed) % starts)));
    });
    drop(pin);
    drop(map);
    remove(&d);
}

fn sweep(c: &mut Criterion) {
    let order = shuffled();
    for (name, d, arm) in contenders("sweep", true) {
        let next = AtomicUsize::new(0);
        c.bench_function(&format!("vindex.sweep/{name}"), |b| {
            b.iter_custom(|iters| {
                let mut spent = Duration::ZERO;
                for _ in 0..iters {
                    let at = next.fetch_add(BATCH as usize, Ordering::Relaxed) % KEYS as usize;
                    let keys = &order[at..at + BATCH as usize];
                    statement(arm.as_ref(), |t, p| {
                        for &k in keys {
                            arm.put(k, None, t, p);
                        }
                    });
                    let start = Instant::now();
                    let dropped = arm.sweep();
                    spent += start.elapsed();
                    assert!(dropped > 0, "{name} took nothing back");
                    statement(arm.as_ref(), |t, p| {
                        for &k in keys {
                            arm.put(k, Some(k), t, p);
                        }
                    });
                }
                spent
            });
        });
        drop(arm);
        remove(&d);
    }
    let (d, map) = laned("sweep", READ_LANES);
    let next = AtomicUsize::new(0);
    c.bench_function(&format!("vindex.sweep/laned_versioned_map_{READ_LANES}"), |b| {
        b.iter_custom(|iters| {
            let mut spent = Duration::ZERO;
            for _ in 0..iters {
                let at = next.fetch_add(BATCH as usize, Ordering::Relaxed) % KEYS as usize;
                let keys = &order[at..at + BATCH as usize];
                let pin = map.epochs().pin().expect("a pin");
                let ticket = map.epochs().begin().expect("a ticket");
                for &k in keys {
                    let lane = map.claim_lane_for(&k).expect("the key's lane");
                    if let Err(e) = lane.remove_at(&k, ticket.epoch()) {
                        panic!("the laned map refused a delete: {e}");
                    }
                }
                ticket.publish();
                drop(pin);
                let start = Instant::now();
                let dropped = laned_sweep(&map);
                spent += start.elapsed();
                assert!(dropped > 0, "the laned map took nothing back");
                let pin = map.epochs().pin().expect("a pin");
                let ticket = map.epochs().begin().expect("a ticket");
                let claims: Vec<_> = (0..READ_LANES).map(|_| map.claim_lane().expect("a lane")).collect();
                for &k in keys {
                    if let Err(e) = claims[(k as usize) % READ_LANES].insert_at(k, k, ticket.epoch()) {
                        panic!("the laned map refused a fill: {e}");
                    }
                }
                drop(claims);
                ticket.publish();
                drop(pin);
            }
            spent
        });
    });
    drop(map);
    remove(&d);
}

/// The first fresh key writer `w` inserts, above the store and above every
/// other writer's keys.
fn fresh_base(w: u64) -> u64 {
    KEYS + w * STATEMENTS * BATCH
}

fn racing_insert(c: &mut Criterion) {
    let mut group = c.benchmark_group("vindex.racing_insert");
    group.sample_size(10);
    for writers in [4u64, 8] {
        for (name, d, arm) in contenders(&format!("race{writers}"), false) {
            group.bench_function(format!("{name}/{writers}"), |b| {
                b.iter_custom(|iters| {
                    let mut spent = Duration::ZERO;
                    for _ in 0..iters {
                        let start = Instant::now();
                        std::thread::scope(|s| {
                            for w in 0..writers {
                                let arm = arm.as_ref();
                                s.spawn(move || {
                                    let base = fresh_base(w);
                                    for st in 0..STATEMENTS {
                                        statement(arm, |t, p| {
                                            for j in 0..BATCH {
                                                let k = base + st * BATCH + j;
                                                arm.put(black_box(k), Some(k), t, p);
                                            }
                                        });
                                    }
                                });
                            }
                        });
                        spent += start.elapsed();
                        statement(arm.as_ref(), |t, p| {
                            for k in fresh_base(0)..fresh_base(writers) {
                                arm.put(k, None, t, p);
                            }
                        });
                        arm.sweep();
                    }
                    spent
                });
            });
            drop(arm);
            remove(&d);
        }
        let (d, map) = laned(&format!("race{writers}"), writers as usize);
        group.bench_function(format!("laned_versioned_map/{writers}"), |b| {
            b.iter_custom(|iters| {
                let mut spent = Duration::ZERO;
                for _ in 0..iters {
                    let start = Instant::now();
                    std::thread::scope(|s| {
                        for w in 0..writers {
                            let map = &map;
                            s.spawn(move || {
                                let lane = map.claim_lane().expect("a lane per writer");
                                let base = fresh_base(w);
                                for st in 0..STATEMENTS {
                                    let pin = map.epochs().pin().expect("a pin");
                                    let ticket = map.epochs().begin().expect("a ticket");
                                    for j in 0..BATCH {
                                        let k = base + st * BATCH + j;
                                        if let Err(e) = lane.insert_at(black_box(k), k, ticket.epoch()) {
                                            panic!("the laned map refused an insert: {e}");
                                        }
                                    }
                                    ticket.publish();
                                    drop(pin);
                                }
                            });
                        }
                    });
                    spent += start.elapsed();
                    let pin = map.epochs().pin().expect("a pin");
                    let ticket = map.epochs().begin().expect("a ticket");
                    for w in 0..writers {
                        let lane = map.claim_lane_for(&fresh_base(w)).expect("the writer's lane");
                        for k in fresh_base(w)..fresh_base(w + 1) {
                            if let Err(e) = lane.remove_at(&k, ticket.epoch()) {
                                panic!("the laned map refused a delete: {e}");
                            }
                        }
                    }
                    ticket.publish();
                    drop(pin);
                    laned_sweep(&map);
                }
                spent
            });
        });
        drop(map);
        remove(&d);
    }
    group.finish();
}

criterion_group!(benches, write, get_at, range_at, sweep, racing_insert);
criterion_main!(benches);
