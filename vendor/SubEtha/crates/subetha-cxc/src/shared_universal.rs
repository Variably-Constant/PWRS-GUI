//! `SharedUniversal<T>` - a cross-process set that moves between a list
//! and a map as the workload shape changes.
//!
//! # The architectural claim
//!
//! A single cross-process set that changes how it stores itself when the
//! observed operation mix favors the other shape. At creation the set
//! starts as a list (a cheap insert into a short scan); when `contains`
//! calls dominate, the common case for membership and dedup workloads, it
//! moves to a map (a lookup that stays short however large the set grows).
//! Every handle follows a move on its next operation.
//!
//! # Scope
//!
//! - **A set on both strategies**: an insert reports whether the value was
//!   new, and the set holds each value once, as a list and as a map.
//! - **Every process writes**: any number of processes insert and migrate
//!   at once, and none waits on another, live or dead.
//! - **Local policy**: a handle's view of the shared operation counts
//!   drives the migration decisions it makes.
//!
//! This is the typed face of the core
//! [`RawUniversal`](crate::raw_universal::RawUniversal) is the byte face
//! of; its module describes the tables, the seal a migration publishes
//! and the files on disk. `T` is stored and hashed as its bytes, as
//! [`SharedHashMap`](crate::shared_hash_map::SharedHashMap) stores its
//! keys, so it must be plain data with no padding and no pointer, and
//! every process attached to one set must agree on it; two values are the
//! same element when their bytes are.

use std::marker::PhantomData;
use std::mem::size_of;
use std::path::Path;
use std::sync::atomic::Ordering;

use crate::adaptive_ring::UnlinkReport;
use crate::raw_universal::{unpack, CoreError, UniversalCore};

pub use crate::raw_universal::{UniversalHeader, UNIVERSAL_MAGIC};

/// Strategy tag: how the live table finds a value.
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Strategy {
    /// A list scanned from the front.
    Vec = 0,
    /// A map probed from the value's hash.
    Map = 1,
}

#[derive(Debug)]
pub enum UniversalError {
    /// The state word names no strategy.
    InvalidStrategy,
    /// The operating system refused to open, size or map a file; the
    /// error carries its code and message.
    IoError(std::io::Error),
    /// The state file or a table on disk was laid out for another shape.
    LayoutMismatch(crate::layout::Mismatch),
    /// The set holds its capacity.
    Full,
    /// The (generation, version) pair is at (`u16::MAX`, `u32::MAX`), and
    /// the set refuses a migration or a clear rather than wrap back to a
    /// pair already used.
    VersionExhausted,
    /// Another process migrated the set while this migration ran. That
    /// migration was run to its end and the set is on the table it chose;
    /// `published` is the state word that stands.
    MigrationRaced { published: u64 },
}

impl std::fmt::Display for UniversalError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidStrategy => write!(f, "the state word names no strategy"),
            Self::IoError(e) => write!(f, "io: {e}"),
            Self::LayoutMismatch(m) => write!(f, "a file of another shape: {m}"),
            Self::Full => write!(f, "the set holds its capacity"),
            Self::VersionExhausted => {
                write!(f, "the version is at its maximum, so no further migration is allowed")
            }
            Self::MigrationRaced { published } => {
                write!(f, "another process migrated the set first; word {published:#x} stands")
            }
        }
    }
}

impl std::error::Error for UniversalError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::IoError(e) => Some(e),
            _ => None,
        }
    }
}

impl From<std::io::Error> for UniversalError {
    fn from(e: std::io::Error) -> Self {
        Self::IoError(e)
    }
}

impl From<CoreError> for UniversalError {
    fn from(e: CoreError) -> Self {
        match e {
            CoreError::Mismatch(m) => Self::LayoutMismatch(m),
            CoreError::InvalidStrategy => Self::InvalidStrategy,
            CoreError::Full => Self::Full,
            CoreError::VersionExhausted => Self::VersionExhausted,
            CoreError::MigrationRaced { published } => Self::MigrationRaced { published },
            CoreError::Io(e) => Self::IoError(e),
        }
    }
}

/// The bytes of `value`, which the type's contract says are all data.
fn bytes_of<T: Copy>(value: &T) -> &[u8] {
    // SAFETY: a `T` is `size_of::<T>()` readable bytes with no padding, by
    // the contract on `SharedUniversal`.
    unsafe { std::slice::from_raw_parts(value as *const T as *const u8, size_of::<T>()) }
}

pub struct SharedUniversal<T: Copy + Eq + 'static> {
    core: UniversalCore,
    _phantom: PhantomData<T>,
    header_sidecar: subetha_core::HandshakeHeader,
    ring_sidecar: Box<subetha_core::ObservationRing>,
}

// SAFETY: the core reaches shared state only through atomics and the
// table's claim protocol; `T` crosses as bytes.
unsafe impl<T: Copy + Eq + Send + 'static> Send for SharedUniversal<T> {}
// SAFETY: as for Send.
unsafe impl<T: Copy + Eq + Sync + 'static> Sync for SharedUniversal<T> {}

impl<T: Copy + Eq + Send + Sync + 'static> subetha_sidecar::AdaptiveInstance for SharedUniversal<T> {
    fn header(&self) -> &subetha_core::HandshakeHeader { &self.header_sidecar }
    fn ring(&self) -> &subetha_core::ObservationRing { &self.ring_sidecar }
    fn make_policy(&self) -> Box<dyn subetha_sidecar::Policy> {
        Box::new(subetha_sidecar::NoMigrationPolicy)
    }
}

impl<T: Copy + Eq + 'static> SharedUniversal<T> {
    const ELEMENT: usize = {
        assert!(size_of::<T>() > 0, "a set element carries at least one byte");
        size_of::<T>()
    };

    fn with_core(core: UniversalCore) -> Self {
        Self {
            core,
            _phantom: PhantomData,
            header_sidecar: subetha_core::HandshakeHeader::new(),
            ring_sidecar: Box::new(subetha_core::ObservationRing::new()),
        }
    }

    /// Obtain the set at `base`: a new one starts empty on the list
    /// strategy at (generation 0, version 0) with no tables but its own,
    /// and one that exists is attached to in the round it is in. A set
    /// built with a different capacity is a `LayoutMismatch`.
    /// [`reset`](Self::reset) reinitializes.
    pub fn create(base: impl AsRef<Path>, capacity: usize) -> Result<Self, UniversalError> {
        assert!(capacity >= 1);
        let core = UniversalCore::create(base.as_ref(), capacity, Self::ELEMENT, Strategy::Vec)?;
        Ok(Self::with_core(core))
    }

    /// Truncate the state at `base`, remove every table, and start an
    /// empty set on the list strategy at (generation 0, version 0). For a
    /// caller that knows nobody else maps the set.
    pub fn reset(base: impl AsRef<Path>, capacity: usize) -> Result<Self, UniversalError> {
        assert!(capacity >= 1);
        let core = UniversalCore::reset(base.as_ref(), capacity, Self::ELEMENT, Strategy::Vec)?;
        Ok(Self::with_core(core))
    }

    /// Open an existing set: reads the live (generation, version, strategy)
    /// from the state file and opens the table it names.
    pub fn open(base: impl AsRef<Path>, capacity: usize) -> Result<Self, UniversalError> {
        let core = UniversalCore::open(base.as_ref(), capacity, Self::ELEMENT)?;
        Ok(Self::with_core(core))
    }

    /// Remove every file of the set at `base`: its state file and each
    /// table. A missing file is counted, not an error; a refusal is counted
    /// and the first one is named. A handle still open keeps its mappings
    /// until it drops.
    pub fn unlink(base: impl AsRef<Path>) -> UnlinkReport {
        UniversalCore::unlink(base.as_ref())
    }

    /// The strategy the shared state names. A handle follows a move on its
    /// next operation.
    pub fn strategy(&self) -> Strategy {
        if unpack(self.core.word()).2 == Strategy::Map as u8 { Strategy::Map } else { Strategy::Vec }
    }

    /// The shared table version. Bumps on every migration and every clear;
    /// wraps to 0 at u32::MAX with the generation counter incrementing.
    pub fn strategy_version(&self) -> u32 {
        unpack(self.core.word()).0
    }

    /// The shared generation counter. Bumps each time `version` wraps from
    /// u32::MAX back to 0. Together with `version` it forms the true
    /// monotonic table counter.
    pub fn strategy_generation(&self) -> u16 {
        unpack(self.core.word()).1
    }

    /// Add `value`; `true` when it was new, `false` when the set already
    /// held it. Of several handles adding one value at once, in any
    /// processes, exactly one is told it was new.
    pub fn insert(&self, value: T) -> Result<bool, UniversalError> {
        let r = self.core.insert(bytes_of(&value)).map_err(UniversalError::from);
        self.core.header().insert_count.fetch_add(1, Ordering::Relaxed);
        self.ring_sidecar.push_op(
            crate::sidecar_ops::universal::OP_INSERT,
            if r.is_err() { 1 } else { 0 },
        );
        r
    }

    /// Membership check. Bumps the contains counter so the local policy
    /// can observe contains-heavy workloads.
    pub fn contains(&self, value: &T) -> Result<bool, UniversalError> {
        let hit = self.core.contains(bytes_of(value))?;
        self.core.header().contains_count.fetch_add(1, Ordering::Relaxed);
        self.ring_sidecar.push_op(
            crate::sidecar_ops::universal::OP_CONTAINS,
            if hit { 0 } else { 2 },
        );
        Ok(hit)
    }

    /// Values held. A migration found part way through is run to its end
    /// first, so the count is the set's and not one table's.
    pub fn len(&self) -> Result<usize, UniversalError> {
        Ok(self.core.len()?)
    }

    pub fn is_empty(&self) -> Result<bool, UniversalError> {
        Ok(self.len()? == 0)
    }

    /// Empty the set: it moves to the next round of its own table, which
    /// starts empty, so the version bumps as for a migration. A value added
    /// while the clear runs is kept or dropped as a whole.
    pub fn clear(&self) -> Result<(), UniversalError> {
        Ok(self.core.clear()?)
    }

    /// Every value, in no promised order.
    pub fn snapshot(&self) -> Result<Vec<T>, UniversalError> {
        let mut out = Vec::new();
        // SAFETY: every stored element is the bytes of a `T`.
        self.core.for_each(|bytes| out.push(unsafe { std::ptr::read_unaligned(bytes.as_ptr() as *const T) }))?;
        Ok(out)
    }

    /// Operation counts since creation, summed across every handle. The
    /// policy code reads these to decide when to migrate.
    pub fn op_histogram(&self) -> (u64, u64) {
        let hdr = self.core.header();
        (hdr.insert_count.load(Ordering::Acquire), hdr.contains_count.load(Ordering::Acquire))
    }

    /// Move the set to `target`, carrying every value across: the live
    /// round is sealed with one compare-and-swap on the state word, every
    /// value published before the seal is copied into the next round of
    /// the other table, and the state word moves to that round. A writer
    /// that finds the seal runs the move to its end and adds to the round
    /// after, so no insert is lost while the move runs. Migrating to the
    /// strategy in force changes nothing.
    ///
    /// # Concurrency
    ///
    /// Any process may migrate. When another move seals the set first, this
    /// one runs that move to its end and returns
    /// [`UniversalError::MigrationRaced`] with the word that stands.
    pub fn migrate_to(&self, target: Strategy) -> Result<(), UniversalError> {
        let r = self.core.migrate_to(target).map_err(UniversalError::from);
        self.ring_sidecar.push_op(
            crate::sidecar_ops::universal::OP_MIGRATE,
            if r.is_err() { 1 } else { 0 },
        );
        r
    }

    /// Local-policy migration trigger. If the observed `contains` ops
    /// outnumber `insert` ops by at least `contains_to_insert_ratio`,
    /// and total ops exceed `min_total_ops`, migrate Vec → Map. If
    /// the inverse holds, migrate Map → Vec.
    ///
    /// Returns `Ok(Some(new_strategy))` if a migration happened,
    /// `Ok(None)` if no policy threshold was crossed.
    pub fn maybe_migrate_by_policy(
        &self,
        contains_to_insert_ratio: f64,
        min_total_ops: u64,
    ) -> Result<Option<Strategy>, UniversalError> {
        let (ins, cnt) = self.op_histogram();
        if ins + cnt < min_total_ops { return Ok(None); }
        let current = self.strategy();
        let ratio = cnt as f64 / (ins.max(1)) as f64;
        let want = if ratio >= contains_to_insert_ratio { Strategy::Map } else { Strategy::Vec };
        if want == current { return Ok(None); }
        self.migrate_to(want)?;
        Ok(Some(want))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::raw_universal::pack;
    use std::path::PathBuf;
    use std::sync::Arc;

    fn tmp_base(name: &str) -> PathBuf {
        let mut p = std::env::temp_dir();
        p.push(format!("subetha-universal-{name}-{}", std::process::id()));
        p
    }

    fn cleanup(base: &Path) {
        let report = SharedUniversal::<u64>::unlink(base);
        assert_eq!(report.failed, 0, "the set's files were not all removed: {report:?}");
    }

    fn sorted(u: &SharedUniversal<u64>) -> Vec<u64> {
        let mut snap = u.snapshot().unwrap();
        snap.sort_unstable();
        snap
    }

    #[test]
    fn create_starts_in_vec_strategy() {
        let base = tmp_base("starts-vec");
        cleanup(&base);
        let u: SharedUniversal<u64> = SharedUniversal::create(&base, 64).unwrap();
        assert_eq!(u.strategy(), Strategy::Vec);
        assert_eq!(u.strategy_version(), 0);
        assert_eq!(u.len().unwrap(), 0);
        drop(u);
        cleanup(&base);
    }

    #[test]
    fn an_insert_reports_whether_the_value_was_new_on_either_strategy() {
        let base = tmp_base("set-both");
        cleanup(&base);
        let u: SharedUniversal<u64> = SharedUniversal::create(&base, 64).unwrap();
        for strategy in [Strategy::Vec, Strategy::Map] {
            u.migrate_to(strategy).unwrap();
            assert!(u.insert(7 + strategy as u64).unwrap(), "{strategy:?}: new");
            assert!(!u.insert(7 + strategy as u64).unwrap(), "{strategy:?}: already held");
        }
        assert_eq!(sorted(&u), vec![7, 8], "the list kept each value once");
        drop(u);
        cleanup(&base);
    }

    #[test]
    fn insert_and_contains_round_trip_in_vec_mode() {
        let base = tmp_base("vec-roundtrip");
        cleanup(&base);
        let u: SharedUniversal<u64> = SharedUniversal::create(&base, 64).unwrap();
        for k in 0..10u64 { u.insert(k).unwrap(); }
        for k in 0..10u64 { assert!(u.contains(&k).unwrap()); }
        assert!(!u.contains(&999).unwrap());
        assert_eq!(u.len().unwrap(), 10);
        drop(u);
        cleanup(&base);
    }

    #[test]
    fn explicit_migrate_to_map_preserves_contents() {
        let base = tmp_base("explicit-map");
        cleanup(&base);
        let u: SharedUniversal<u64> = SharedUniversal::create(&base, 64).unwrap();
        for k in 0..10u64 { u.insert(k).unwrap(); }
        u.migrate_to(Strategy::Map).unwrap();
        assert_eq!(u.strategy(), Strategy::Map);
        assert_eq!(u.strategy_version(), 1);
        for k in 0..10u64 { assert!(u.contains(&k).unwrap()); }
        assert_eq!(u.len().unwrap(), 10);
        drop(u);
        cleanup(&base);
    }

    #[test]
    fn migrate_back_and_forth_preserves_contents() {
        let base = tmp_base("round-trip");
        cleanup(&base);
        let u: SharedUniversal<u64> = SharedUniversal::create(&base, 64).unwrap();
        for k in 0..5u64 { u.insert(k).unwrap(); }
        u.migrate_to(Strategy::Map).unwrap();
        u.migrate_to(Strategy::Vec).unwrap();
        u.migrate_to(Strategy::Map).unwrap();
        assert_eq!(u.strategy(), Strategy::Map);
        assert_eq!(u.strategy_version(), 3);
        assert_eq!(sorted(&u), vec![0, 1, 2, 3, 4]);
        drop(u);
        cleanup(&base);
    }

    #[test]
    fn an_insert_landing_while_a_migration_runs_is_kept() {
        let base = tmp_base("late-insert");
        cleanup(&base);
        let u = Arc::new(SharedUniversal::<u64>::create(&base, 64).unwrap());
        for k in 0..5u64 { u.insert(k).unwrap(); }

        // The migration stops once it has read the state, and an insert
        // lands in that window.
        let migrating = Arc::clone(&u);
        let (pause, migration) = crate::test_races::stopped(move || migrating.migrate_to(Strategy::Map));
        assert!(u.insert(99).unwrap());
        pause.release();
        migration.join().expect("the migration thread").expect("the migration");

        assert_eq!(u.strategy(), Strategy::Map);
        assert!(u.contains(&99).unwrap(), "the insert made during the migration is in the migrated set");
        assert_eq!(u.len().unwrap(), 6);
        drop(u);
        cleanup(&base);
    }

    /// Two migrations from one state: the one stopped before its seal loses
    /// it, runs the winner to its end and reports the word that stands.
    #[test]
    fn a_migration_that_loses_the_race_leaves_the_winners_table_alone() {
        let base = tmp_base("race");
        cleanup(&base);
        let loser = Arc::new(SharedUniversal::<u64>::create(&base, 64).unwrap());
        for k in 0..5u64 { loser.insert(k).unwrap(); }
        loser.migrate_to(Strategy::Map).unwrap();
        let winner: SharedUniversal<u64> = SharedUniversal::open(&base, 64).unwrap();

        let migrating = Arc::clone(&loser);
        let (pause, raced) = crate::test_races::stopped(move || migrating.migrate_to(Strategy::Vec));
        winner.migrate_to(Strategy::Vec).unwrap();
        let published = winner.core.word();
        pause.release();

        match raced.join().expect("the losing migration's thread") {
            Err(UniversalError::MigrationRaced { published: stands }) => {
                assert_eq!(stands, published, "the loser reports the word that stands");
            }
            other => panic!("the stopped migration was not refused as raced: {other:?}"),
        }
        assert_eq!(unpack(published), (2, 0, Strategy::Vec as u8));
        assert_eq!(sorted(&winner), vec![0, 1, 2, 3, 4], "the published round holds the set");
        let reader: SharedUniversal<u64> = SharedUniversal::open(&base, 64).unwrap();
        assert_eq!(reader.len().unwrap(), 5, "a fresh handle reads the published table");
        drop(reader);
        drop(winner);
        drop(loser);
        cleanup(&base);
    }

    /// A migration back into a table the set left holds that table's new
    /// round only: what it held before the set left it is not in the set.
    #[test]
    fn a_table_a_migration_returns_to_holds_only_the_round_it_serves() {
        let base = tmp_base("returned");
        cleanup(&base);
        let u: SharedUniversal<u64> = SharedUniversal::create(&base, 16).unwrap();
        for k in 0..5u64 { u.insert(k).unwrap(); }
        u.migrate_to(Strategy::Map).unwrap();
        u.clear().unwrap();
        for k in 10..13u64 { u.insert(k).unwrap(); }
        u.migrate_to(Strategy::Vec).unwrap();
        assert_eq!(u.strategy_version(), 3);
        assert_eq!(sorted(&u), vec![10, 11, 12], "the list table's earlier keys stayed behind");
        assert!(!u.contains(&3).unwrap());
        assert!(u.insert(3).unwrap(), "an earlier key is new to the round");
        assert_eq!(u.len().unwrap(), 4);
        drop(u);
        cleanup(&base);
    }

    #[test]
    fn migrate_to_same_strategy_is_noop() {
        let base = tmp_base("same");
        cleanup(&base);
        let u: SharedUniversal<u64> = SharedUniversal::create(&base, 16).unwrap();
        u.migrate_to(Strategy::Vec).unwrap();
        assert_eq!(u.strategy_version(), 0);
        u.migrate_to(Strategy::Map).unwrap();
        u.migrate_to(Strategy::Map).unwrap();
        assert_eq!(u.strategy_version(), 1);
        drop(u);
        cleanup(&base);
    }

    #[test]
    fn local_policy_migrates_to_map_under_contains_load() {
        let base = tmp_base("policy-map");
        cleanup(&base);
        let u: SharedUniversal<u64> = SharedUniversal::create(&base, 64).unwrap();
        for k in 0..5u64 { u.insert(k).unwrap(); }
        // 100 contains, 5 inserts → ratio = 20, well above 0.5
        for _ in 0..100 { u.contains(&3).unwrap(); }
        let migrated = u.maybe_migrate_by_policy(0.5, 100).unwrap();
        assert_eq!(migrated, Some(Strategy::Map));
        assert_eq!(u.strategy(), Strategy::Map);
        drop(u);
        cleanup(&base);
    }

    /// A second create attaches to the live state - including a
    /// post-migration map table - with contents in place; reset is what
    /// strips them.
    #[test]
    fn second_create_attaches_and_keeps_contents() {
        let base = tmp_base("attach");
        cleanup(&base);
        let u: SharedUniversal<u64> = SharedUniversal::create(&base, 64).unwrap();
        for k in 0..10u64 { u.insert(k).unwrap(); }
        u.migrate_to(Strategy::Map).unwrap();

        let u2: SharedUniversal<u64> = SharedUniversal::create(&base, 64).unwrap();
        assert_eq!(u2.strategy(), Strategy::Map, "attach ignored the live strategy");
        assert_eq!(u2.len().unwrap(), 10, "attach lost contents");
        for k in 0..10u64 { assert!(u2.contains(&k).unwrap()); }
        assert!(matches!(
            SharedUniversal::<u64>::create(&base, 32),
            Err(UniversalError::LayoutMismatch(crate::layout::Mismatch {
                field: "capacity",
                found: 64,
                wanted: 32,
            })),
        ));

        // Windows refuses to truncate a mapped file, so every handle
        // goes before the reset.
        drop(u);
        drop(u2);
        let fresh: SharedUniversal<u64> = SharedUniversal::reset(&base, 64).unwrap();
        assert_eq!(fresh.strategy(), Strategy::Vec, "reset kept the migrated strategy");
        assert_eq!(fresh.len().unwrap(), 0, "reset kept contents");
        drop(fresh);
        cleanup(&base);
    }

    #[test]
    fn local_policy_keeps_vec_under_insert_load() {
        let base = tmp_base("policy-vec");
        cleanup(&base);
        let u: SharedUniversal<u64> = SharedUniversal::create(&base, 256).unwrap();
        for k in 0..200u64 { u.insert(k).unwrap(); }
        for _ in 0..10 { u.contains(&3).unwrap(); }
        let migrated = u.maybe_migrate_by_policy(0.5, 100).unwrap();
        assert_eq!(migrated, None);
        assert_eq!(u.strategy(), Strategy::Vec);
        drop(u);
        cleanup(&base);
    }

    #[test]
    fn reader_handle_observes_migration_via_version_bump() {
        // After the writer migrates, the reader's next op reads the table
        // the state word names.
        let base = tmp_base("cross-handle");
        cleanup(&base);
        let writer: SharedUniversal<u64> = SharedUniversal::create(&base, 32).unwrap();
        let reader: SharedUniversal<u64> = SharedUniversal::open(&base, 32).unwrap();
        for k in 0..5u64 { writer.insert(k).unwrap(); }
        assert_eq!(reader.strategy(), Strategy::Vec);
        assert_eq!(reader.len().unwrap(), 5);
        writer.migrate_to(Strategy::Map).unwrap();
        assert_eq!(reader.strategy(), Strategy::Map);
        assert_eq!(reader.strategy_version(), 1);
        assert_eq!(reader.len().unwrap(), 5);
        for k in 0..5u64 { assert!(reader.contains(&k).unwrap()); }
        drop(writer);
        drop(reader);
        cleanup(&base);
    }

    #[test]
    fn snapshot_preserves_through_migration() {
        let base = tmp_base("snap");
        cleanup(&base);
        let u: SharedUniversal<u32> = SharedUniversal::create(&base, 32).unwrap();
        for k in [10u32, 5, 7, 1, 99] { u.insert(k).unwrap(); }
        let mut pre = u.snapshot().unwrap();
        pre.sort();
        u.migrate_to(Strategy::Map).unwrap();
        let mut post = u.snapshot().unwrap();
        post.sort();
        assert_eq!(pre, post);
        drop(u);
        cleanup(&base);
    }

    #[test]
    fn op_histogram_tracks_real_ops() {
        let base = tmp_base("hist");
        cleanup(&base);
        let u: SharedUniversal<u64> = SharedUniversal::create(&base, 32).unwrap();
        u.insert(1).unwrap();
        u.insert(2).unwrap();
        u.insert(3).unwrap();
        u.contains(&2).unwrap();
        u.contains(&2).unwrap();
        let (ins, cnt) = u.op_histogram();
        assert_eq!(ins, 3);
        assert_eq!(cnt, 2);
        drop(u);
        cleanup(&base);
    }

    #[test]
    fn pack_unpack_round_trips_all_three_fields() {
        let packed = pack(0xDEAD_BEEF, 0xCAFE, Strategy::Map);
        assert_eq!(unpack(packed), (0xDEAD_BEEF, 0xCAFE, Strategy::Map as u8));
    }

    #[test]
    fn starts_at_generation_zero() {
        let base = tmp_base("gen-zero");
        cleanup(&base);
        let u: SharedUniversal<u64> = SharedUniversal::create(&base, 16).unwrap();
        assert_eq!(u.strategy_version(), 0);
        assert_eq!(u.strategy_generation(), 0);
        drop(u);
        cleanup(&base);
    }

    #[test]
    fn migration_within_generation_keeps_generation_zero() {
        let base = tmp_base("same-gen");
        cleanup(&base);
        let u: SharedUniversal<u64> = SharedUniversal::create(&base, 16).unwrap();
        u.migrate_to(Strategy::Map).unwrap();
        u.migrate_to(Strategy::Vec).unwrap();
        u.migrate_to(Strategy::Map).unwrap();
        assert_eq!(u.strategy_version(), 3);
        assert_eq!(u.strategy_generation(), 0);
        drop(u);
        cleanup(&base);
    }

    /// A state at the last version of a generation: the next migration
    /// wraps the version to 0 and bumps the generation.
    #[test]
    fn version_wrap_bumps_generation_and_resets_version() {
        let base = tmp_base("wrap-gen");
        cleanup(&base);
        let u: SharedUniversal<u64> = SharedUniversal::create(&base, 16).unwrap();
        u.core.jump_to(u32::MAX);
        u.insert(3).unwrap();
        u.insert(4).unwrap();
        u.migrate_to(Strategy::Map).unwrap();
        assert_eq!(u.strategy_version(), 0);
        assert_eq!(u.strategy_generation(), 1);
        assert_eq!(u.strategy(), Strategy::Map);
        assert_eq!(sorted(&u), vec![3, 4]);
        drop(u);
        cleanup(&base);
    }

    #[test]
    fn true_exhaustion_returns_version_exhausted() {
        let base = tmp_base("exhausted");
        cleanup(&base);
        let u: SharedUniversal<u64> = SharedUniversal::create(&base, 16).unwrap();
        // A move refused for its version touches no table, so the word
        // alone sets the state up.
        u.core.header().state.store(pack(u32::MAX, u16::MAX, Strategy::Vec), Ordering::Release);
        assert!(matches!(u.migrate_to(Strategy::Map), Err(UniversalError::VersionExhausted)));
        assert!(matches!(u.clear(), Err(UniversalError::VersionExhausted)));
        drop(u);
        cleanup(&base);
    }

    /// The load-bearing safety property: when the version wraps to 0 and
    /// the generation bumps, a reader that opened generation 0 must take
    /// generation 1's tables on its next operation, not its own.
    #[test]
    fn reader_re_opens_on_generation_change_even_at_same_version() {
        let base = tmp_base("reader-gen");
        cleanup(&base);
        let writer: SharedUniversal<u64> = SharedUniversal::create(&base, 16).unwrap();
        let reader: SharedUniversal<u64> = SharedUniversal::open(&base, 16).unwrap();
        writer.insert(11).unwrap();
        writer.insert(22).unwrap();
        writer.core.jump_to(u32::MAX);
        writer.migrate_to(Strategy::Map).unwrap();
        writer.insert(99).unwrap();
        assert!(reader.contains(&99u64).unwrap());
        assert!(!reader.contains(&11u64).unwrap(), "generation 1 holds 99 alone");
        assert_eq!(reader.strategy(), Strategy::Map);
        assert_eq!(reader.strategy_version(), 0);
        assert_eq!(reader.strategy_generation(), 1);
        drop(writer);
        drop(reader);
        cleanup(&base);
    }

    /// One writer process: adds every key in an order of its own, counting
    /// each add it is told was new in the tally file, and migrates the set
    /// partway through when it is the first writer. Runs only when the
    /// cross-process test starts it.
    #[test]
    fn universal_writer_role() {
        let Some(spec) = std::env::var_os("SUBETHA_UNIVERSAL_WRITER") else {
            return;
        };
        let spec = spec.to_string_lossy().to_string();
        let parts: Vec<&str> = spec.split('|').collect();
        let [base, tally, writer, keys] = parts[..] else {
            eprintln!("writer: a spec of {} parts: {spec}", parts.len());
            std::process::exit(2);
        };
        let writer: u64 = writer.parse().expect("the writer index parses");
        let keys: u64 = keys.parse().expect("the key count parses");
        let u: SharedUniversal<u64> = SharedUniversal::open(base, 2 * keys as usize).expect("the writer opens the set");
        let tally = Tally::open(Path::new(tally), keys as usize);
        for step in 0..keys {
            let k = (step + writer * 131) % keys;
            if u.insert(k).expect("an insert") {
                tally.word(k as usize).fetch_add(1, Ordering::AcqRel);
            }
            if writer == 0 && step % (keys / 4) == keys / 8 {
                let target = if u.strategy() == Strategy::Vec { Strategy::Map } else { Strategy::Vec };
                match u.migrate_to(target) {
                    Ok(()) | Err(UniversalError::MigrationRaced { .. }) => {}
                    Err(other) => {
                        eprintln!("writer {writer}: migration: {other}");
                        std::process::exit(1);
                    }
                }
            }
        }
        std::process::exit(0);
    }

    /// One counter per key in a file every writer process maps.
    struct Tally(memmap2::MmapMut);

    impl Tally {
        fn create(path: &Path, keys: usize) -> Self {
            let file = std::fs::OpenOptions::new().read(true).write(true).create(true).truncate(true).open(path).expect("the tally file");
            file.set_len((keys * 4) as u64).expect("the tally file sized");
            Self(unsafe { memmap2::MmapMut::map_mut(&file) }.expect("the tally mapped"))
        }

        fn open(path: &Path, keys: usize) -> Self {
            let file = std::fs::OpenOptions::new().read(true).write(true).open(path).expect("the tally file");
            assert_eq!(file.metadata().expect("the tally's size").len(), (keys * 4) as u64);
            Self(unsafe { memmap2::MmapMut::map_mut(&file) }.expect("the tally mapped"))
        }

        fn word(&self, k: usize) -> &std::sync::atomic::AtomicU32 {
            unsafe { &*(self.0.as_ptr().add(k * 4) as *const std::sync::atomic::AtomicU32) }
        }
    }

    struct Writer(Option<std::process::Child>);

    impl Writer {
        fn reap(mut self) -> std::process::ExitStatus {
            let mut child = self.0.take().expect("the writer is reaped once");
            child.wait().expect("the writer is waited on")
        }
    }

    impl Drop for Writer {
        fn drop(&mut self) {
            if let Some(mut child) = self.0.take() {
                if let Err(e) = child.kill() {
                    eprintln!("the writer was not killed: {e}");
                }
                if let Err(e) = child.wait() {
                    eprintln!("the writer was not reaped: {e}");
                }
            }
        }
    }

    /// Writer processes adding the same keys in different orders while the
    /// first of them migrates the set back and forth: each key is reported
    /// new to exactly one process, and the set holds every key once.
    #[test]
    fn writer_processes_racing_migrations_add_each_key_exactly_once() {
        const WRITERS: u64 = 4;
        const KEYS: u64 = 400;
        let base = tmp_base("processes");
        cleanup(&base);
        let u: SharedUniversal<u64> = SharedUniversal::create(&base, 2 * KEYS as usize).unwrap();
        let mut tally_path = base.clone().into_os_string();
        tally_path.push(".tally");
        let tally_path = PathBuf::from(tally_path);
        let tally = Tally::create(&tally_path, KEYS as usize);
        let exe = std::env::current_exe().expect("the test binary's own path");
        let writers: Vec<Writer> = (0..WRITERS)
            .map(|w| {
                Writer(Some(
                    std::process::Command::new(&exe)
                        .arg("shared_universal::tests::universal_writer_role")
                        .arg("--exact")
                        .arg("--nocapture")
                        .env(
                            "SUBETHA_UNIVERSAL_WRITER",
                            format!("{}|{}|{w}|{KEYS}", base.display(), tally_path.display()),
                        )
                        .spawn()
                        .expect("a writer process spawns"),
                ))
            })
            .collect();
        for writer in writers {
            let status = writer.reap();
            assert!(status.success(), "a writer failed: {status}");
        }
        let counts: Vec<u32> = (0..KEYS as usize).map(|k| tally.word(k).load(Ordering::Acquire)).collect();
        assert!(counts.iter().all(|&n| n == 1), "every key reported new to exactly one process: {counts:?}");
        assert!(u.strategy_version() > 0, "the set migrated while the writers ran");
        assert_eq!(u.len().unwrap(), KEYS as usize);
        assert_eq!(sorted(&u), (0..KEYS).collect::<Vec<_>>());
        drop(tally);
        drop(u);
        match std::fs::remove_file(&tally_path) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => panic!("{} not removed: {e}", tally_path.display()),
        }
        cleanup(&base);
    }
}
