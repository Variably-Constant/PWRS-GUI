//! `SharedVersionedIndex<K, V>` - an ordered map in shared memory that any
//! number of threads and processes write at once, keeping every version
//! of a key that a pinned reader can still need.
//!
//! # What it is
//!
//! A lock-free skip list of keys in files every attached process maps.
//! Each key holds its versions newest first behind one atomic word, each
//! version stamped with the epoch that made it current; a deletion is a
//! version with no value, so a version stops being current at the birth
//! of the one after it. A reader shows its [`PinGuard`] and sees, for each
//! key, the newest version born at or before the pin, or one its own
//! write made.
//!
//! # Writes
//!
//! A write is stamped with its statement's [`EpochTicket`] and judged
//! against the statement's pin. It builds only on the version the pin
//! sees: a key whose newest version was born after the pin by another
//! epoch, published or not, refuses it with [`IndexError::Conflict`]. One
//! compare-and-swap on the key's version word decides each write, so of
//! writers racing on one key exactly one wins and the rest are refused,
//! and nobody waits. A key not yet in the map is inserted with its first
//! version in place, so of writers racing to create a key exactly one
//! lands. A second write by the same epoch replaces its own version.
//!
//! A version an epoch wrote is the newest on its key until the epoch
//! publishes: every pin sits below an open ticket, so any other writer
//! meets it as born after its pin. [`void`](SharedVersionedIndex::void)
//! takes back such a version, and the one it superseded is current again.
//!
//! # A writer that dies
//!
//! Before a write installs a version it records the key on a list kept
//! for its ticket's slot. [`void_epoch`](SharedVersionedIndex::void_epoch)
//! walks that list, so the epoch of a ticket whose process died, which
//! [`SharedEpochs::dead_tickets`] names, is taken back key by key, at the
//! cost of what that epoch wrote, before
//! [`SharedEpochs::free_dead_ticket`] releases its slot. A writer that dies
//! anywhere leaves the skip list readable: a key's first level is linked
//! in one compare-and-swap and the levels above it are only shortcuts.
//!
//! # Reclamation
//!
//! [`sweep`](SharedVersionedIndex::sweep) drops every version superseded
//! at or below the epoch table's reclaim horizon, and removes a key whose
//! newest version is a deletion at or below it. What it unlinks is
//! retired, not freed: a slot is reused only once the horizon has passed
//! the epoch it was retired at, when no pin that could still be reading it
//! is left. Every read and write takes a pin from this index's epoch
//! table for that reason; a pin from a table in another file is not
//! counted in the horizon and does not keep what it reads in place.
//!
//! # Storage
//!
//! The main file at the path holds the header, the shortcuts out of the
//! head of the list and the per-ticket write lists. Keys, versions and
//! write-list entries live in chunk files beside it, named after it, made
//! as the index grows. The epoch table is a [`SharedEpochs`] at its own
//! path, so every structure of a store shares one by naming one file.
//!
//! `K` and `V` are stored as their bytes, so they must be plain data that
//! holds no pointer or reference, and every process attached to one index
//! must agree on their types and on `K`'s ordering.
//! [`RawSharedVersionedIndex`](crate::raw_shared_versioned_index::RawSharedVersionedIndex)
//! is the same index at key and value sizes given at run time.

use std::cell::Cell;
use std::cmp::Ordering as KeyOrder;
use std::fs::File;
use std::io;
use std::marker::PhantomData;
use std::mem::{align_of, size_of};
use std::ops::Bound;
use std::path::Path;
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::sync::Arc;

use memmap2::MmapMut;

use crate::chunk_pool::{body_offset, slot_size_for, ChunkPool, PoolHeader, Slots};
use crate::shared_epochs::{Epoch, EpochError, EpochTicket, PinGuard, SharedEpochs};

/// "VINDEX_1".
pub const INDEX_MAGIC: u64 = 0x5649_4E44_4558_5F31;

/// The most levels a key's shortcuts span, enough for 4^24 keys.
pub const MAX_HEIGHT: usize = 24;

/// Keys this tall or shorter live in the short pool.
const SHORT_HEIGHT: usize = 2;

/// Log2 of the slots a chunk file holds by default.
pub const DEFAULT_CHUNK_BITS: u32 = 16;

/// A key link's mark: the key is being removed at that level.
const MARK: u32 = 1 << 31;
/// A key link's pool: set for the tall pool.
const TALL: u32 = 1 << 30;
const SLOT_MASK: u32 = TALL - 1;
/// The key link to no key.
const KEY_NIL: u32 = SLOT_MASK;
/// The key link to the head of the list, whose shortcuts are in the main
/// file.
const HEAD: u32 = SLOT_MASK - 1;
/// Slots a key pool may hold, below the two reserved links.
const KEY_SLOTS: usize = HEAD as usize;

/// A version link to no version.
const VNIL: u32 = u32::MAX;
/// A key's version word once a sweep is removing the key.
const VRETIRED: u32 = u32::MAX - 1;
/// A version link a reclaimer has claimed, read as the end of the chain.
const VSEALED: u32 = u32::MAX - 2;
/// Slots the version pool may hold, below the reserved links.
const VERSION_SLOTS: usize = VSEALED as usize;

/// A version with no value: the key was deleted at its epoch.
const TOMBSTONE: u32 = 1;

/// A key being linked level by level.
const BUILDING: u32 = 1;
/// A key whose levels are all linked.
const BUILT: u32 = 2;
/// A key removed while it was being linked; its builder retires it.
const REMOVED_WHILE_BUILDING: u32 = 3;
/// A key its remover retires.
const RETIRING: u32 = 4;

/// Where the header's parts sit in the main file.
const SHORT_POOL: usize = 64;
const TALL_POOL: usize = 128;
const VERSION_POOL: usize = 192;
const WRITE_POOL: usize = 256;

/// The pools, in header order; each one's chunk files lie beside the main
/// file, named `<main>.<pool><chunk>`.
const POOLS: [&str; 4] = ["keys", "tall", "versions", "writes"];

/// Remove an index's main file at `path` and every chunk file named after
/// it, into a report of what was removed, missing or refused.
pub(crate) fn unlink_files(path: &Path) -> crate::adaptive_ring::UnlinkReport {
    let mut report = crate::adaptive_ring::UnlinkReport::default();
    report.remove(path.to_path_buf());
    let Some(stem) = path.file_name().and_then(|s| s.to_str()) else {
        return report;
    };
    let dir = match path.parent() {
        Some(p) if !p.as_os_str().is_empty() => p.to_path_buf(),
        _ => std::path::PathBuf::from("."),
    };
    let is_chunk = |name: &str| {
        name.strip_prefix(stem).and_then(|r| r.strip_prefix('.')).is_some_and(|rest| {
            POOLS.iter().any(|pool| {
                rest.strip_prefix(pool).is_some_and(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()))
            })
        })
    };
    let entries = match std::fs::read_dir(&dir) {
        Ok(entries) => entries,
        Err(e) => {
            report.failed += 1;
            report.first_failure.get_or_insert((dir, e.kind(), e.to_string()));
            return report;
        }
    };
    for entry in entries {
        match entry {
            Ok(entry) => {
                if entry.file_name().to_str().is_some_and(is_chunk) {
                    report.remove(entry.path());
                }
            }
            Err(e) => {
                report.failed += 1;
                report.first_failure.get_or_insert((dir.clone(), e.kind(), e.to_string()));
            }
        }
    }
    report
}
const HEAD_TOWER: usize = 320;
const WRITE_HEADS: usize = HEAD_TOWER + 128;

const _: () = assert!(MAX_HEIGHT * size_of::<AtomicU32>() <= 128);

#[repr(C, align(64))]
struct IndexHeader {
    magic: u64,
    key_size: u32,
    key_align: u32,
    value_size: u32,
    value_align: u32,
    ticket_slots: u32,
    chunk_bits: u32,
    max_height: u32,
    _pad0: u32,
    _pad: [u8; 24],
}

const _: () = assert!(size_of::<IndexHeader>() == 64);

/// The start of a key's slot body.
#[repr(C)]
struct KeyHead {
    /// The newest version, [`VNIL`], or [`VRETIRED`].
    versions: AtomicU32,
    height: u32,
    /// Who retires the key: [`BUILDING`] through [`RETIRING`].
    state: AtomicU32,
    _pad: u32,
}

/// The start of a version's slot body.
#[repr(C)]
struct VersionHead {
    born: Epoch,
    /// The version this one superseded, [`VNIL`], or [`VSEALED`].
    next: AtomicU32,
    flags: u32,
}

/// The start of a write-list entry's slot body; the key follows it.
#[repr(C)]
struct EntryHead {
    /// The next entry plus one, or zero.
    next: u32,
    _pad: u32,
}

/// How an index sizes, aligns and orders what it stores.
pub(crate) trait Shape: Send + Sync + 'static {
    /// The magic its main file carries. Shapes that order keys differently
    /// carry different magics, so neither opens the other's file.
    const MAGIC: u64;
    fn key_size(&self) -> usize;
    fn key_align(&self) -> usize;
    fn value_size(&self) -> usize;
    fn value_align(&self) -> usize;
    /// Orders the keys at `a` and `b`.
    ///
    /// # Safety
    /// Each points to `key_size()` readable bytes aligned to `key_align()`.
    unsafe fn cmp(&self, a: *const u8, b: *const u8) -> KeyOrder;

    /// Whether the key at `a` orders before the key at `b`: one test where
    /// a search only needs to know whether to move on.
    ///
    /// # Safety
    /// As for [`Shape::cmp`].
    unsafe fn less(&self, a: *const u8, b: *const u8) -> bool {
        // SAFETY: the caller's promise is `cmp`'s.
        matches!(unsafe { self.cmp(a, b) }, KeyOrder::Less)
    }
}

/// The shape of a [`SharedVersionedIndex<K, V>`]: keys ordered by `K: Ord`.
pub(crate) struct Typed<K, V>(PhantomData<fn() -> (K, V)>);

impl<K: Copy + Ord + 'static, V: Copy + 'static> Shape for Typed<K, V> {
    const MAGIC: u64 = INDEX_MAGIC;

    fn key_size(&self) -> usize {
        size_of::<K>()
    }

    fn key_align(&self) -> usize {
        align_of::<K>()
    }

    fn value_size(&self) -> usize {
        size_of::<V>()
    }

    fn value_align(&self) -> usize {
        align_of::<V>()
    }

    unsafe fn cmp(&self, a: *const u8, b: *const u8) -> KeyOrder {
        // SAFETY: the caller passes two `K`s, each aligned for `K`.
        unsafe { (*(a as *const K)).cmp(&*(b as *const K)) }
    }

    unsafe fn less(&self, a: *const u8, b: *const u8) -> bool {
        // SAFETY: the caller passes two `K`s, each aligned for `K`.
        unsafe { *(a as *const K) < *(b as *const K) }
    }
}

/// Where each kind of slot keeps its parts, fixed by the shape.
#[derive(Clone, Copy, Debug)]
struct Layout {
    key_body: usize,
    key_off: usize,
    tower_off: usize,
    short_slot: usize,
    tall_slot: usize,
    version_body: usize,
    value_off: usize,
    version_slot: usize,
    entry_body: usize,
    entry_key_off: usize,
    entry_slot: usize,
}

impl Layout {
    /// The layout `shape` needs, or `None` when one of its slots would not
    /// fit a pool.
    fn of(shape: &impl Shape) -> Option<Self> {
        let (key_size, key_align) = (shape.key_size(), shape.key_align());
        let (value_size, value_align) = (shape.value_size(), shape.value_align());
        let key_slot_align = key_align.max(8);
        let value_slot_align = value_align.max(8);
        let key_off = size_of::<KeyHead>().checked_next_multiple_of(key_align)?;
        let tower_off = key_off.checked_add(key_size)?.checked_next_multiple_of(4)?;
        let value_off = size_of::<VersionHead>().checked_next_multiple_of(value_align)?;
        let entry_key_off = size_of::<EntryHead>().checked_next_multiple_of(key_align)?;
        Some(Self {
            key_body: body_offset(key_slot_align),
            key_off,
            tower_off,
            short_slot: slot_size_for(tower_off.checked_add(4 * SHORT_HEIGHT)?, key_slot_align)?,
            tall_slot: slot_size_for(tower_off.checked_add(4 * MAX_HEIGHT)?, key_slot_align)?,
            version_body: body_offset(value_slot_align),
            value_off,
            version_slot: slot_size_for(value_off.checked_add(value_size)?, value_slot_align)?,
            entry_body: body_offset(key_slot_align),
            entry_key_off,
            entry_slot: slot_size_for(entry_key_off.checked_add(key_size)?, key_slot_align)?,
        })
    }
}

/// Bytes the main file holds for an epoch table of `ticket_slots` tickets.
const fn main_file_size(ticket_slots: usize) -> usize {
    WRITE_HEADS + ticket_slots * size_of::<AtomicU64>()
}

/// The most chunk files one pool may have, which bounds the table of
/// mappings each process keeps for it at 128 KiB.
const MAX_CHUNKS: usize = 1 << 14;

/// Chunks a pool may hold so its slots stay below `limit`.
const fn chunks_below(limit: usize, chunk_bits: u32) -> usize {
    let fit = limit >> chunk_bits;
    if fit < MAX_CHUNKS { fit } else { MAX_CHUNKS }
}

fn check_chunk_bits(chunk_bits: u32) -> Result<(), IndexError> {
    if (4..=20).contains(&chunk_bits) {
        Ok(())
    } else {
        Err(IndexError::InvalidConfig("a chunk holds 2^4 to 2^20 slots"))
    }
}

/// What [`SharedVersionedIndex::range_at`] answers: the rows, and the last
/// key examined when the walk stopped short of the range's end.
pub type RangeRows<K, V> = (Vec<(K, V)>, Option<K>);

#[derive(Debug)]
pub enum IndexError {
    /// A write met a version of the key born after its pin by another
    /// epoch, published or not: the statement takes back what it wrote and
    /// runs again.
    Conflict { epoch: Epoch },
    /// Every slot a pool can hold is in use or retired.
    Full,
    LayoutMismatch(crate::layout::Mismatch),
    /// An argument the index cannot take.
    InvalidConfig(&'static str),
    Epochs(EpochError),
    /// The operating system refused to open, size or map a file; the error
    /// carries its code and message.
    IoError(io::Error),
}

impl std::fmt::Display for IndexError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            IndexError::Conflict { epoch } => {
                write!(f, "the key has a version from epoch {epoch}, after the pin")
            }
            IndexError::Full => write!(f, "the index has no free slot"),
            IndexError::LayoutMismatch(m) => write!(f, "an index of another shape: {m}"),
            IndexError::InvalidConfig(why) => write!(f, "{why}"),
            IndexError::Epochs(e) => write!(f, "epoch table: {e}"),
            IndexError::IoError(e) => write!(f, "versioned index io: {e}"),
        }
    }
}

impl std::error::Error for IndexError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            IndexError::Epochs(e) => Some(e),
            IndexError::IoError(e) => Some(e),
            IndexError::Conflict { .. }
            | IndexError::Full
            | IndexError::LayoutMismatch(_)
            | IndexError::InvalidConfig(_) => None,
        }
    }
}

impl From<io::Error> for IndexError {
    fn from(e: io::Error) -> Self {
        match crate::layout::Mismatch::carried_by(&e) {
            Some(m) => IndexError::LayoutMismatch(m),
            None => IndexError::IoError(e),
        }
    }
}

impl From<EpochError> for IndexError {
    fn from(e: EpochError) -> Self {
        IndexError::Epochs(e)
    }
}

thread_local! {
    /// Each thread's xorshift state for key heights.
    static HEIGHTS: Cell<u64> = Cell::new(height_seed());
}

fn height_seed() -> u64 {
    let local = 0u8;
    let time = match std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH) {
        Ok(d) => d.as_nanos() as u64,
        Err(e) => e.duration().as_nanos() as u64,
    };
    (time ^ ((&raw const local) as usize as u64).rotate_left(32)) | 1
}

/// A key height, each level above the first a quarter as likely as the
/// one below it.
fn random_height() -> usize {
    HEIGHTS.with(|state| {
        let mut x = state.get();
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        state.set(x);
        let mut height = 1;
        let mut bits = x;
        while height < MAX_HEIGHT && bits & 3 == 0 {
            height += 1;
            bits >>= 2;
        }
        height
    })
}

/// What the searches and reads on one thread did, for finding where a
/// read's time goes. Built only with the `trace-index` feature. Each thread
/// counts into its own cell and a search records once at its end, so
/// counting takes no atomic and makes no thread wait.
#[cfg(feature = "trace-index")]
pub mod trace {
    use std::cell::Cell;

    /// Counts since the last [`take`] on this thread.
    #[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
    pub struct SearchCounts {
        /// Searches finished.
        pub finds: u64,
        /// Searches started over after losing a race to unlink a marked key.
        pub retries: u64,
        /// Levels walked, summed over searches.
        pub levels: u64,
        /// Keys visited, summed over searches: each one a shortcut loaded out
        /// of it and its key compared.
        pub hops: u64,
        /// Visits that landed on a key in the tall pool.
        pub tall_hops: u64,
        /// Key comparisons, the final equality check included.
        pub compares: u64,
        /// Levels a read-only search ended on reaching the key that ended
        /// the level above, without visiting it: each one a hop saved.
        pub stops: u64,
        /// Marked keys a search unlinked on its way.
        pub unlinks: u64,
        /// Reads that found their key and went on to pick a version.
        pub reads: u64,
        /// Versions those reads examined.
        pub versions: u64,
        /// Timestamp-counter ticks those reads spent searching; zero off
        /// x86_64.
        pub search_ticks: u64,
        /// Ticks those reads spent picking the visible version.
        pub version_ticks: u64,
    }

    const ZERO: SearchCounts = SearchCounts {
        finds: 0,
        retries: 0,
        levels: 0,
        hops: 0,
        tall_hops: 0,
        compares: 0,
        stops: 0,
        unlinks: 0,
        reads: 0,
        versions: 0,
        search_ticks: 0,
        version_ticks: 0,
    };

    thread_local! {
        static COUNTS: Cell<SearchCounts> = const { Cell::new(ZERO) };
    }

    /// Add `more` to this thread's counts.
    pub(super) fn record(more: &SearchCounts) {
        COUNTS.with(|counts| {
            let mut c = counts.get();
            c.finds += more.finds;
            c.retries += more.retries;
            c.levels += more.levels;
            c.hops += more.hops;
            c.tall_hops += more.tall_hops;
            c.compares += more.compares;
            c.stops += more.stops;
            c.unlinks += more.unlinks;
            c.reads += more.reads;
            c.versions += more.versions;
            c.search_ticks += more.search_ticks;
            c.version_ticks += more.version_ticks;
            counts.set(c);
        });
    }

    /// This thread's counts since the last call, which starts them again.
    pub fn take() -> SearchCounts {
        COUNTS.with(|counts| counts.replace(ZERO))
    }

    /// The timestamp counter, or zero where there is none to read.
    pub(super) fn ticks() -> u64 {
        #[cfg(target_arch = "x86_64")]
        // SAFETY: RDTSC exists on every x86_64 processor and only reads a
        // counter.
        #[allow(unused_unsafe)]
        let t = unsafe { std::arch::x86_64::_rdtsc() };
        #[cfg(not(target_arch = "x86_64"))]
        let t = 0;
        t
    }
}

/// One search's counts, kept in registers and recorded once when it ends.
/// Without the `trace-index` feature it holds nothing and every method is
/// empty.
#[derive(Default)]
struct Tally {
    #[cfg(feature = "trace-index")]
    counts: trace::SearchCounts,
}

#[cfg(feature = "trace-index")]
impl Tally {
    fn retry(&mut self) {
        self.counts.retries += 1;
    }

    fn levels(&mut self, levels: usize) {
        self.counts.levels += levels as u64;
    }

    fn hop(&mut self, link: u32) {
        self.counts.hops += 1;
        self.counts.tall_hops += u64::from(link & TALL != 0);
    }

    fn compare(&mut self) {
        self.counts.compares += 1;
    }

    fn stop(&mut self) {
        self.counts.stops += 1;
    }

    fn unlink(&mut self) {
        self.counts.unlinks += 1;
    }

    fn done(mut self) {
        self.counts.finds += 1;
        trace::record(&self.counts);
    }
}

#[cfg(not(feature = "trace-index"))]
impl Tally {
    #[inline(always)]
    fn retry(&mut self) {}

    #[inline(always)]
    fn levels(&mut self, _: usize) {}

    #[inline(always)]
    fn hop(&mut self, _: u32) {}

    #[inline(always)]
    fn compare(&mut self) {}

    #[inline(always)]
    fn stop(&mut self) {}

    #[inline(always)]
    fn unlink(&mut self) {}

    #[inline(always)]
    fn done(self) {}
}

/// The timestamp counter when reads are traced.
#[cfg(feature = "trace-index")]
fn read_ticks() -> u64 {
    trace::ticks()
}

/// Zero: reads are not traced.
#[cfg(not(feature = "trace-index"))]
#[inline(always)]
fn read_ticks() -> u64 {
    0
}

/// Record a read that searched from `start` to `found`, then examined
/// `versions` versions until `end`.
#[cfg(feature = "trace-index")]
fn read_done(versions: usize, start: u64, found: u64, end: u64) {
    trace::record(&trace::SearchCounts {
        reads: 1,
        versions: versions as u64,
        search_ticks: found.wrapping_sub(start),
        version_ticks: end.wrapping_sub(found),
        ..trace::SearchCounts::default()
    });
}

/// Nothing: reads are not traced.
#[cfg(not(feature = "trace-index"))]
#[inline(always)]
fn read_done(_: usize, _: u64, _: u64, _: u64) {}

/// The key pools' geometry and the head's shortcuts, read once for a run of
/// hops such as one search. A hop through [`Index::next`] reads the pools'
/// fields again each time, since the atomic load on every hop keeps the
/// compiler from reusing them; through this view a hop needs no load beyond
/// the node it visits while the node lies in its pool's first chunk.
#[derive(Clone, Copy)]
struct KeyRead<'a> {
    short: Slots<'a>,
    tall: Slots<'a>,
    /// Each pool's slot 0 body and slot size, which place every key in
    /// chunk 0 with one multiply.
    short_first: *mut u8,
    tall_first: *mut u8,
    short_size: usize,
    tall_size: usize,
    /// The link bits naming a slot past chunk 0, the same for both pools.
    past_first: u32,
    head: *const AtomicU32,
    key_body: usize,
    key_off: usize,
    tower_off: usize,
}

impl<'a> KeyRead<'a> {
    #[inline]
    fn body(&self, link: u32) -> io::Result<*mut u8> {
        let index = link & SLOT_MASK;
        let tall = link & TALL != 0;
        if link & self.past_first == 0 {
            let (first, size) = if tall { (self.tall_first, self.tall_size) } else { (self.short_first, self.short_size) };
            // SAFETY: `index` names a slot of chunk 0, whose bodies follow
            // slot 0's at the pool's slot size.
            return Ok(unsafe { first.add(index as usize * size) });
        }
        let pool = if tall { &self.tall } else { &self.short };
        pool.body(index, self.key_body)
    }

    /// The shortcuts out of the key whose body is at `body`, the first
    /// level's first.
    ///
    /// # Safety
    /// `body` is what [`KeyRead::body`] answered for a key.
    #[inline]
    unsafe fn tower_in(&self, body: *mut u8) -> *const AtomicU32 {
        // SAFETY: the tower follows the key inside the slot body.
        unsafe { body.add(self.tower_off) as *const AtomicU32 }
    }

    /// [`Index::next`] through this view.
    #[inline]
    fn next(&self, node: u32, level: usize) -> io::Result<&'a AtomicU32> {
        if node == HEAD {
            // SAFETY: `head` addresses `MAX_HEIGHT` shortcuts in the main
            // mapping, which outlives `'a`, and `level` is below that.
            return Ok(unsafe { &*self.head.add(level) });
        }
        let body = self.body(node)?;
        // SAFETY: the tower follows the key, and `level` is below the node's
        // height, which its pool's slots hold.
        Ok(unsafe { &*(body.add(self.tower_off + 4 * level) as *const AtomicU32) })
    }

    /// [`Index::key_ptr`] through this view.
    #[inline]
    fn key_ptr(&self, link: u32) -> io::Result<*const u8> {
        let body = self.body(link)?;
        // SAFETY: the key lies inside the slot body, past its head.
        Ok(unsafe { body.add(self.key_off) })
    }

    /// The shortcut at `level` out of the key whose body is at `body`.
    ///
    /// # Safety
    /// `body` is what [`KeyRead::body`] answered for a key whose height is
    /// above `level`.
    #[inline]
    unsafe fn link_in(&self, body: *mut u8, level: usize) -> &'a AtomicU32 {
        // SAFETY: the tower follows the key and holds the key's height, which
        // the caller promises is above `level`; the mapping outlives `'a`.
        unsafe { &*(body.add(self.tower_off + 4 * level) as *const AtomicU32) }
    }

    /// Where the key whose body is at `body` keeps its key.
    ///
    /// # Safety
    /// `body` is what [`KeyRead::body`] answered for a key.
    #[inline]
    unsafe fn key_in(&self, body: *mut u8) -> *const u8 {
        // SAFETY: the key lies inside the slot body, past its head.
        unsafe { body.add(self.key_off) }
    }
}

/// Where a search for a key ended at each level.
struct Search {
    preds: [u32; MAX_HEIGHT],
    succs: [u32; MAX_HEIGHT],
    found: Option<u32>,
}

/// What [`Index::insert_key`] did.
enum Inserted {
    New,
    Existing,
}

/// The index over keys and values as the shape `S` stores and orders them.
/// A key or value reaches it as a pointer to the shape's bytes: one of the
/// shape's size, aligned for it, which is what every caller of its unsafe
/// methods promises and what every key and value stored in a slot is.
pub(crate) struct Index<S: Shape> {
    // The pools point into `mmap` and are declared first so they drop first.
    short: ChunkPool,
    tall: ChunkPool,
    versions: ChunkPool,
    writes: ChunkPool,
    epochs: Arc<SharedEpochs>,
    mmap: MmapMut,
    _file: File,
    layout: Layout,
    ticket_slots: usize,
    shape: S,
    /// How many of the head's levels the last search in this process found
    /// in use, where the next search starts looking. The levels in use move
    /// only when the index gains or loses a level, and any starting level
    /// gives a correct search, so a stale value costs a few loads.
    top_hint: AtomicU32,
}

impl<S: Shape> std::fmt::Debug for Index<S> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Index")
            .field("ticket_slots", &self.ticket_slots)
            .field("layout", &self.layout)
            .field("epochs", &self.epochs)
            .finish()
    }
}

impl<S: Shape> Index<S> {
    /// Obtain the index at `path` over the epoch table `epochs`, laying it
    /// out with chunks of `1 << chunk_bits` slots when the file does not
    /// exist and attaching to it when it does.
    pub(crate) fn create(path: &Path, epochs: Arc<SharedEpochs>, shape: S, chunk_bits: u32) -> Result<Self, IndexError> {
        check_chunk_bits(chunk_bits)?;
        let layout = Layout::of(&shape).ok_or(IndexError::InvalidConfig("a key or value too large for a slot"))?;
        let ticket_slots = epochs.capacity();
        let (file, mmap) = crate::mmf_attach::create_or_attach(
            path,
            main_file_size(ticket_slots),
            |ptr| unsafe { Self::init_main(ptr, &shape, &layout, ticket_slots, chunk_bits) },
            // SAFETY: the mapping is at least the header long.
            |ptr| unsafe { std::ptr::read_volatile(ptr as *const u64) },
            S::MAGIC,
        )?;
        Self::assemble(path, file, mmap, epochs, shape, layout, chunk_bits)
    }

    /// Attach to an existing index over the epoch table `epochs`.
    pub(crate) fn open(path: &Path, epochs: Arc<SharedEpochs>, shape: S) -> Result<Self, IndexError> {
        let layout = Layout::of(&shape).ok_or(IndexError::InvalidConfig("a key or value too large for a slot"))?;
        let ticket_slots = epochs.capacity();
        let file = crate::region_file::open_existing(path)?;
        let total = main_file_size(ticket_slots);
        let held = file.metadata()?.len();
        if held < total as u64 {
            return Err(IndexError::LayoutMismatch(crate::layout::Mismatch::new("length", held, total as u64)));
        }
        // SAFETY: a shared file mapping of the length just checked.
        let mmap = unsafe { memmap2::MmapOptions::new().len(total).map_mut(&file)? };
        // SAFETY: the mapping is at least the header long.
        let chunk_bits = unsafe { (*(mmap.as_ptr() as *const IndexHeader)).chunk_bits };
        Self::assemble(path, file, mmap, epochs, shape, layout, chunk_bits)
    }

    /// Lay out an empty index: the pools, an empty list, empty write lists,
    /// and the magic last, because attachers spin on it.
    ///
    /// # Safety
    /// `ptr` addresses at least `main_file_size(ticket_slots)` zeroed bytes.
    unsafe fn init_main(ptr: *mut u8, shape: &S, layout: &Layout, ticket_slots: usize, chunk_bits: u32) {
        unsafe {
            let h = ptr as *mut IndexHeader;
            (*h).key_size = shape.key_size() as u32;
            (*h).key_align = shape.key_align() as u32;
            (*h).value_size = shape.value_size() as u32;
            (*h).value_align = shape.value_align() as u32;
            (*h).ticket_slots = ticket_slots as u32;
            (*h).chunk_bits = chunk_bits;
            (*h).max_height = MAX_HEIGHT as u32;
            PoolHeader::init(ptr.add(SHORT_POOL) as *mut PoolHeader, layout.short_slot, chunk_bits);
            PoolHeader::init(ptr.add(TALL_POOL) as *mut PoolHeader, layout.tall_slot, chunk_bits);
            PoolHeader::init(ptr.add(VERSION_POOL) as *mut PoolHeader, layout.version_slot, chunk_bits);
            PoolHeader::init(ptr.add(WRITE_POOL) as *mut PoolHeader, layout.entry_slot, chunk_bits);
            for level in 0..MAX_HEIGHT {
                (*(ptr.add(HEAD_TOWER + 4 * level) as *const AtomicU32)).store(KEY_NIL, Ordering::Relaxed);
            }
            std::ptr::write_volatile(&raw mut (*h).magic, S::MAGIC);
        }
    }

    /// Check an initialized main file against the shape and the epoch
    /// table, then attach the pools.
    fn assemble(
        path: &Path,
        file: File,
        mmap: MmapMut,
        epochs: Arc<SharedEpochs>,
        shape: S,
        layout: Layout,
        chunk_bits: u32,
    ) -> Result<Self, IndexError> {
        let ticket_slots = epochs.capacity();
        // SAFETY: the mapping is at least the header long.
        let h = unsafe { &*(mmap.as_ptr() as *const IndexHeader) };
        crate::layout::Mismatch::first(&[
            ("magic", h.magic, S::MAGIC),
            ("key size", u64::from(h.key_size), shape.key_size() as u64),
            ("key alignment", u64::from(h.key_align), shape.key_align() as u64),
            ("value size", u64::from(h.value_size), shape.value_size() as u64),
            ("value alignment", u64::from(h.value_align), shape.value_align() as u64),
            ("ticket slots", u64::from(h.ticket_slots), ticket_slots as u64),
            ("chunk bits", u64::from(h.chunk_bits), u64::from(chunk_bits)),
            ("max height", u64::from(h.max_height), MAX_HEIGHT as u64),
        ])
        .map_err(IndexError::LayoutMismatch)?;
        check_chunk_bits(chunk_bits)?;
        let base = mmap.as_ptr() as *mut u8;
        // SAFETY: each header was laid out by `init_main` in this mapping,
        // which the index keeps for as long as the pools.
        // One run per ticket slot in each pool, so writers racing on fresh
        // keys reach the shared bump cursor once a run rather than once a slot.
        let (short, tall, versions, writes) = unsafe {
            (
                ChunkPool::attach(
                    base.add(SHORT_POOL) as *mut PoolHeader,
                    path,
                    POOLS[0],
                    chunks_below(KEY_SLOTS, chunk_bits),
                    ticket_slots,
                )?,
                ChunkPool::attach(
                    base.add(TALL_POOL) as *mut PoolHeader,
                    path,
                    POOLS[1],
                    chunks_below(KEY_SLOTS, chunk_bits),
                    ticket_slots,
                )?,
                ChunkPool::attach(
                    base.add(VERSION_POOL) as *mut PoolHeader,
                    path,
                    POOLS[2],
                    chunks_below(VERSION_SLOTS, chunk_bits),
                    ticket_slots,
                )?,
                ChunkPool::attach(
                    base.add(WRITE_POOL) as *mut PoolHeader,
                    path,
                    POOLS[3],
                    chunks_below(u32::MAX as usize, chunk_bits),
                    ticket_slots,
                )?,
            )
        };
        Ok(Self {
            short,
            tall,
            versions,
            writes,
            epochs,
            mmap,
            _file: file,
            layout,
            ticket_slots,
            shape,
            top_hint: AtomicU32::new(MAX_HEIGHT as u32),
        })
    }

    /// The epoch table: pins, tickets, the reclaim horizon and dead tickets.
    #[inline]
    pub(crate) fn epochs(&self) -> &Arc<SharedEpochs> {
        &self.epochs
    }

    #[inline]
    pub(crate) fn shape(&self) -> &S {
        &self.shape
    }

    fn base(&self) -> *mut u8 {
        self.mmap.as_ptr() as *mut u8
    }

    fn head_tower(&self, level: usize) -> &AtomicU32 {
        // SAFETY: the head's shortcuts are laid out at `HEAD_TOWER` for
        // `MAX_HEIGHT` levels, and the mapping lives as long as `self`.
        unsafe { &*(self.base().add(HEAD_TOWER + 4 * level) as *const AtomicU32) }
    }

    fn write_head(&self, slot: usize) -> &AtomicU64 {
        // SAFETY: one word per ticket slot at `WRITE_HEADS`; callers check
        // `slot` against `ticket_slots`.
        unsafe { &*(self.base().add(WRITE_HEADS + 8 * slot) as *const AtomicU64) }
    }

    fn key_pool(&self, link: u32) -> &ChunkPool {
        if link & TALL != 0 { &self.tall } else { &self.short }
    }

    fn key_body(&self, link: u32) -> io::Result<*mut u8> {
        self.key_pool(link).body(link & SLOT_MASK, self.layout.key_body)
    }

    fn key_head(&self, link: u32) -> io::Result<&KeyHead> {
        // SAFETY: a key's body starts with its `KeyHead`, and the chunk lives
        // as long as `self`.
        Ok(unsafe { &*(self.key_body(link)? as *const KeyHead) })
    }

    /// Where `link`'s key is stored. The key was written before the key
    /// was linked, and the slot is not reused while a pin that could reach
    /// it is held.
    fn key_ptr(&self, link: u32) -> io::Result<*const u8> {
        let body = self.key_body(link)?;
        // SAFETY: the key lies inside the slot body, past its head.
        Ok(unsafe { body.add(self.layout.key_off) })
    }

    /// Orders the keys at `a` and `b`, each a key of the shape.
    fn cmp(&self, a: *const u8, b: *const u8) -> KeyOrder {
        // SAFETY: every key pointer the index holds is a caller's key or one
        // stored in a slot, both of the shape's size and aligned for it.
        unsafe { self.shape.cmp(a, b) }
    }

    /// Whether the key at `a` orders before the key at `b`, each a key of
    /// the shape.
    fn less(&self, a: *const u8, b: *const u8) -> bool {
        // SAFETY: as for `cmp`.
        unsafe { self.shape.less(a, b) }
    }

    /// The key pools' geometry and the head's shortcuts, read once for a run
    /// of hops.
    fn key_read(&self) -> KeyRead<'_> {
        let (short, tall) = (self.short.slots(), self.tall.slots());
        let key_body = self.layout.key_body;
        // Both key pools are laid out with the main file's chunk bits; the
        // smaller of the two their headers carry keeps every slot this
        // counts as chunk 0 inside both pools' chunk 0.
        let bits = short.chunk_bits().min(tall.chunk_bits());
        KeyRead {
            short,
            tall,
            short_first: short.first_at(key_body),
            tall_first: tall.first_at(key_body),
            short_size: short.slot_size(),
            tall_size: tall.slot_size(),
            past_first: SLOT_MASK >> bits << bits,
            // SAFETY: the head's shortcuts are laid out at `HEAD_TOWER` for
            // `MAX_HEIGHT` levels, and the mapping lives as long as `self`.
            head: unsafe { self.base().add(HEAD_TOWER) as *const AtomicU32 },
            key_body,
            key_off: self.layout.key_off,
            tower_off: self.layout.tower_off,
        }
    }

    /// The shortcut out of `node` at `level`: the main file's for the head.
    fn next(&self, node: u32, level: usize) -> io::Result<&AtomicU32> {
        if node == HEAD {
            return Ok(self.head_tower(level));
        }
        let body = self.key_body(node)?;
        // SAFETY: the tower follows the key, and `level` is below the node's
        // height, which its pool's slots hold.
        Ok(unsafe { &*(body.add(self.layout.tower_off + 4 * level) as *const AtomicU32) })
    }

    fn version_head(&self, v: u32) -> io::Result<&VersionHead> {
        // SAFETY: a version's body starts with its `VersionHead`.
        Ok(unsafe { &*(self.versions.body(v, self.layout.version_body)? as *const VersionHead) })
    }

    /// A slot from `pool` for the writer at ticket slot `writer`, reclaiming
    /// what the horizon allows when the pool has none.
    fn alloc(&self, pool: &ChunkPool, writer: usize) -> Result<u32, IndexError> {
        if let Some(slot) = pool.alloc(writer)? {
            return Ok(slot);
        }
        pool.reclaim(self.epochs.reclaim_horizon())?;
        pool.alloc(writer)?.ok_or(IndexError::Full)
    }

    /// Find where `key` belongs at every level, unlinking any key marked for
    /// removal on the way.
    fn find(&self, key: *const u8) -> io::Result<Search> {
        let keys = self.key_read();
        let mut tally = Tally::default();
        'retry: loop {
            let mut s = Search { preds: [HEAD; MAX_HEIGHT], succs: [KEY_NIL; MAX_HEIGHT], found: None };
            let mut pred = HEAD;
            // A level the head links to no key ends at the head, which is
            // what `s` already holds for it, so the search starts below the
            // empty levels at the top.
            let mut top = MAX_HEIGHT;
            while top > 1 && keys.next(HEAD, top - 1)?.load(Ordering::Acquire) & !MARK == KEY_NIL {
                top -= 1;
            }
            tally.levels(top);
            for level in (0..top).rev() {
                let mut curr = keys.next(pred, level)?.load(Ordering::Acquire) & !MARK;
                loop {
                    if curr == KEY_NIL {
                        break;
                    }
                    tally.hop(curr);
                    let mut succ = keys.next(curr, level)?.load(Ordering::Acquire);
                    while succ & MARK != 0 {
                        let after = succ & !MARK;
                        if keys.next(pred, level)?.compare_exchange(curr, after, Ordering::AcqRel, Ordering::Acquire).is_err()
                        {
                            tally.retry();
                            continue 'retry;
                        }
                        tally.unlink();
                        curr = after;
                        if curr == KEY_NIL {
                            break;
                        }
                        tally.hop(curr);
                        succ = keys.next(curr, level)?.load(Ordering::Acquire);
                    }
                    if curr == KEY_NIL {
                        break;
                    }
                    tally.compare();
                    if self.cmp(keys.key_ptr(curr)?, key) == KeyOrder::Less {
                        pred = curr;
                        curr = succ;
                    } else {
                        break;
                    }
                }
                s.preds[level] = pred;
                s.succs[level] = curr;
            }
            if s.succs[0] != KEY_NIL {
                tally.compare();
                if self.cmp(keys.key_ptr(s.succs[0])?, key) == KeyOrder::Equal {
                    s.found = Some(s.succs[0]);
                }
            }
            tally.done();
            return Ok(s);
        }
    }

    /// How many of the head's levels are in use: one past the highest whose
    /// shortcut names a key, and at least one. Starts from the count the
    /// last search in this process found.
    fn top_levels(&self, keys: &KeyRead<'_>) -> usize {
        // SAFETY: `head` addresses `MAX_HEIGHT` shortcuts and every level
        // passed is below that.
        let used = |level: usize| unsafe { &*keys.head.add(level) }.load(Ordering::Acquire) & !MARK != KEY_NIL;
        let hint = self.top_hint.load(Ordering::Relaxed) as usize;
        let mut top = hint.clamp(1, MAX_HEIGHT);
        while top < MAX_HEIGHT && used(top) {
            top += 1;
        }
        while top > 1 && !used(top - 1) {
            top -= 1;
        }
        if top != hint {
            self.top_hint.store(top as u32, Ordering::Relaxed);
        }
        top
    }

    /// The body of the live key equal to `key`, by a search that only
    /// reads. It takes the shortcuts from the top, passes a key marked for
    /// removal by following its successor without unlinking it or stepping
    /// down from it, and stops at the first unmarked key equal to `key` at
    /// any level. Levels are marked top first, so a key unmarked at one
    /// level is unmarked at every level below it, its first included, and is
    /// live.
    ///
    /// The key that ends a level by ordering after `key` ends every level
    /// below it where the search reaches it again, without a load or a
    /// compare: the pin keeps its slot, and so its key, from reuse.
    fn find_key(&self, key: *const u8) -> io::Result<Option<*mut u8>> {
        let keys = self.key_read();
        let mut tally = Tally::default();
        let top = self.top_levels(&keys);
        tally.levels(top);
        // The shortcuts of the last key found ordering before `key`, the
        // head's until one is.
        let mut pred = keys.head;
        // The last key found ordering after `key`.
        let mut stop = KEY_NIL;
        for level in (0..top).rev() {
            // SAFETY: `pred` is the head's tower, `MAX_HEIGHT` high, or the
            // tower of a key reached at this level or one above it, whose
            // height is above `level`.
            let mut curr = unsafe { &*pred.add(level) }.load(Ordering::Acquire) & !MARK;
            while curr != KEY_NIL {
                if curr == stop {
                    tally.stop();
                    break;
                }
                tally.hop(curr);
                let body = keys.body(curr)?;
                // SAFETY: `curr` was reached through a shortcut at `level`, so
                // its height is above `level`.
                let succ = unsafe { keys.link_in(body, level) }.load(Ordering::Acquire);
                if succ & MARK != 0 {
                    curr = succ & !MARK;
                    continue;
                }
                tally.compare();
                // SAFETY: `body` is the body of the key `curr`.
                let at = unsafe { keys.key_in(body) };
                if self.less(at, key) {
                    // SAFETY: `body` is the body of the key `curr`.
                    pred = unsafe { keys.tower_in(body) };
                    curr = succ;
                    continue;
                }
                if self.cmp(at, key) == KeyOrder::Equal {
                    tally.done();
                    return Ok(Some(body));
                }
                stop = curr;
                break;
            }
        }
        tally.done();
        Ok(None)
    }

    /// Link a new key carrying `first` as its newest version, unless the key
    /// is already there. The first level's compare-and-swap is the moment
    /// the key and its version appear together.
    fn insert_key(&self, key: *const u8, first: u32, writer: usize) -> Result<Inserted, IndexError> {
        let height = random_height();
        let tall = height > SHORT_HEIGHT;
        let pool = if tall { &self.tall } else { &self.short };
        let link = self.alloc(pool, writer)? | if tall { TALL } else { 0 };
        let body = self.key_body(link)?;
        // SAFETY: a slot no other thread holds yet, sized for the shape's key.
        unsafe {
            let kh = body as *mut KeyHead;
            (*kh).versions.store(first, Ordering::Relaxed);
            (*kh).height = height as u32;
            (*kh).state.store(BUILDING, Ordering::Relaxed);
            std::ptr::copy_nonoverlapping(key, body.add(self.layout.key_off), self.shape.key_size());
        }
        let mut s;
        loop {
            s = self.find(key)?;
            if s.found.is_some() {
                pool.free_unseen(link & SLOT_MASK)?;
                return Ok(Inserted::Existing);
            }
            for (level, succ) in s.succs.iter().enumerate().take(height) {
                self.next(link, level)?.store(*succ, Ordering::Relaxed);
            }
            if self.next(s.preds[0], 0)?.compare_exchange(s.succs[0], link, Ordering::AcqRel, Ordering::Acquire).is_ok() {
                break;
            }
        }
        self.build_levels(link, key, height, s)?;
        Ok(Inserted::New)
    }

    /// Link `link`'s levels above the first, stopping if it is marked for
    /// removal meanwhile, then settle who retires it.
    fn build_levels(&self, link: u32, key: *const u8, height: usize, mut s: Search) -> io::Result<()> {
        'levels: for level in 1..height {
            loop {
                if self.next(link, 0)?.load(Ordering::Acquire) & MARK != 0 {
                    break 'levels;
                }
                let ours = self.next(link, level)?.load(Ordering::Acquire);
                if ours & MARK != 0 {
                    break 'levels;
                }
                if ours != s.succs[level]
                    && self.next(link, level)?.compare_exchange(ours, s.succs[level], Ordering::AcqRel, Ordering::Acquire).is_err()
                {
                    break 'levels;
                }
                if self.next(s.preds[level], level)?
                    .compare_exchange(s.succs[level], link, Ordering::AcqRel, Ordering::Acquire)
                    .is_ok()
                {
                    break;
                }
                s = self.find(key)?;
                if s.found != Some(link) {
                    break 'levels;
                }
            }
        }
        if self.key_head(link)?.state.compare_exchange(BUILDING, BUILT, Ordering::AcqRel, Ordering::Acquire).is_err() {
            // Removed while it was being linked: unlink any level linked after
            // the mark, then retire it, which its remover left to this call.
            self.find(key)?;
            self.retire_key(link)?;
        }
        Ok(())
    }

    /// Mark every level of `link`, top first, so searches unlink it.
    fn mark_levels(&self, link: u32) -> io::Result<()> {
        let height = self.key_head(link)?.height as usize;
        for level in (0..height).rev() {
            let word = self.next(link, level)?;
            let mut cur = word.load(Ordering::Acquire);
            while cur & MARK == 0 {
                match word.compare_exchange(cur, cur | MARK, Ordering::AcqRel, Ordering::Acquire) {
                    Ok(_) => break,
                    Err(seen) => cur = seen,
                }
            }
        }
        Ok(())
    }

    /// Remove a key whose version word this caller set to [`VRETIRED`], and
    /// retire it or hand it to the call still linking it.
    fn remove_key(&self, link: u32, key: *const u8) -> io::Result<()> {
        self.mark_levels(link)?;
        self.find(key)?;
        let state = &self.key_head(link)?.state;
        loop {
            match state.compare_exchange(BUILT, RETIRING, Ordering::AcqRel, Ordering::Acquire) {
                Ok(_) => return self.retire_key(link),
                Err(BUILDING) => {
                    if state
                        .compare_exchange(BUILDING, REMOVED_WHILE_BUILDING, Ordering::AcqRel, Ordering::Acquire)
                        .is_ok()
                    {
                        return Ok(());
                    }
                }
                Err(other) => {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        format!("a key being removed was in state {other}"),
                    ));
                }
            }
        }
    }

    fn retire_key(&self, link: u32) -> io::Result<()> {
        self.key_pool(link).retire(link & SLOT_MASK, self.epochs.now())
    }

    /// A version born at `born` holding the value at `value`, or a deletion
    /// for `None`, superseding `next`, allocated for the writer at ticket slot
    /// `writer`.
    fn new_version(&self, born: Epoch, value: Option<*const u8>, next: u32, writer: usize) -> Result<u32, IndexError> {
        let v = self.alloc(&self.versions, writer)?;
        let body = self.versions.body(v, self.layout.version_body)?;
        // SAFETY: a slot no other thread holds yet, sized for the shape's
        // value.
        unsafe {
            let vh = body as *mut VersionHead;
            (*vh).born = born;
            (*vh).next.store(next, Ordering::Relaxed);
            (*vh).flags = if value.is_some() { 0 } else { TOMBSTONE };
            if let Some(value) = value {
                std::ptr::copy_nonoverlapping(value, body.add(self.layout.value_off), self.shape.value_size());
            }
        }
        Ok(v)
    }

    /// Retire one version no chain reaches any more.
    fn retire_version(&self, v: u32) -> io::Result<()> {
        self.versions.retire(v, self.epochs.now())
    }

    /// Retire a run of versions this caller unlinked, claiming each link as
    /// it goes so a sweep that unlinked a later part of the run retires only
    /// that part. Returns how many held a value.
    fn retire_run(&self, first: u32) -> io::Result<usize> {
        let stamp = self.epochs.now();
        let mut v = first;
        let mut valued = 0usize;
        while v != VNIL && v != VSEALED {
            let vh = self.version_head(v)?;
            let next = vh.next.swap(VSEALED, Ordering::AcqRel);
            if vh.flags & TOMBSTONE == 0 {
                valued += 1;
            }
            self.versions.retire(v, stamp)?;
            v = next;
        }
        Ok(valued)
    }

    /// Record `key` on the write list of ticket slot `slot`, for epoch
    /// `epoch`. The first write of a ticket that reuses a slot starts a new
    /// list and retires the old one.
    fn record_write(&self, slot: usize, epoch: Epoch, key: *const u8) -> Result<(), IndexError> {
        if slot >= self.ticket_slots {
            return Err(IndexError::InvalidConfig("a ticket from another epoch table"));
        }
        let entry = self.alloc(&self.writes, slot)?;
        let body = self.writes.body(entry, self.layout.entry_body)?;
        // SAFETY: a slot no other thread holds yet, sized for the shape's key.
        unsafe { std::ptr::copy_nonoverlapping(key, body.add(self.layout.entry_key_off), self.shape.key_size()) };
        let head = self.write_head(slot);
        let tag = epoch as u32;
        loop {
            let seen = head.load(Ordering::Acquire);
            let fresh = (seen >> 32) as u32 != tag;
            let first = seen as u32;
            // SAFETY: the entry is not yet on any list.
            unsafe { (*(body as *mut EntryHead)).next = if fresh { 0 } else { first } };
            let pushed = (u64::from(tag) << 32) | u64::from(entry + 1);
            if head.compare_exchange(seen, pushed, Ordering::AcqRel, Ordering::Acquire).is_ok() {
                if fresh && first != 0 {
                    self.retire_entries(first - 1)?;
                }
                return Ok(());
            }
        }
    }

    fn retire_entries(&self, first: u32) -> io::Result<()> {
        let stamp = self.epochs.now();
        let mut entries = Vec::new();
        let mut e = first + 1;
        while e != 0 {
            let body = self.writes.body(e - 1, self.layout.entry_body)?;
            // SAFETY: an entry's head was written before it was pushed.
            let next = unsafe { (*(body as *const EntryHead)).next };
            entries.push(e - 1);
            e = next;
        }
        self.writes.retire_all(&entries, stamp)
    }

    /// Make the value at `value` current for the key at `key` in the
    /// statement holding the ticket at `epoch` in slot `slot` whose
    /// snapshot is `pin`; `None` deletes.
    ///
    /// # Safety
    /// `key` and `value` are a key and a value of the shape.
    pub(crate) unsafe fn write_at(
        &self,
        key: *const u8,
        value: Option<*const u8>,
        epoch: Epoch,
        slot: usize,
        pin: Epoch,
    ) -> Result<(), IndexError> {
        loop {
            let s = self.find(key)?;
            let Some(link) = s.found else {
                let Some(value) = value else {
                    return Ok(());
                };
                let v = self.new_version(epoch, Some(value), VNIL, slot)?;
                self.record_write(slot, epoch, key)?;
                match self.insert_key(key, v, slot)? {
                    Inserted::New => return Ok(()),
                    Inserted::Existing => {
                        self.versions.free_unseen(v)?;
                        continue;
                    }
                }
            };
            let kh = self.key_head(link)?;
            let head = kh.versions.load(Ordering::Acquire);
            if head == VRETIRED {
                self.mark_levels(link)?;
                self.find(key)?;
                continue;
            }
            if head == VNIL {
                let Some(value) = value else {
                    return Ok(());
                };
                let v = self.new_version(epoch, Some(value), VNIL, slot)?;
                self.record_write(slot, epoch, key)?;
                if kh.versions.compare_exchange(VNIL, v, Ordering::AcqRel, Ordering::Acquire).is_ok() {
                    return Ok(());
                }
                self.versions.free_unseen(v)?;
                continue;
            }
            let hv = self.version_head(head)?;
            if hv.born == epoch {
                let next = hv.next.load(Ordering::Acquire);
                let replacement = match value {
                    Some(value) => Some(self.new_version(epoch, Some(value), next, slot)?),
                    // Deleting its own version: a deletion when that version
                    // superseded a value, otherwise the key goes back to how
                    // it was before this epoch wrote it.
                    None if next != VNIL && self.version_head(next)?.flags & TOMBSTONE == 0 => {
                        Some(self.new_version(epoch, None, next, slot)?)
                    }
                    None => None,
                };
                let target = replacement.unwrap_or(next);
                if kh.versions.compare_exchange(head, target, Ordering::AcqRel, Ordering::Acquire).is_ok() {
                    self.retire_version(head)?;
                    return Ok(());
                }
                if let Some(r) = replacement {
                    self.versions.free_unseen(r)?;
                }
                continue;
            }
            if hv.born > pin {
                return Err(IndexError::Conflict { epoch: hv.born });
            }
            if value.is_none() && hv.flags & TOMBSTONE != 0 {
                return Ok(());
            }
            let v = self.new_version(epoch, value, head, slot)?;
            self.record_write(slot, epoch, key)?;
            if kh.versions.compare_exchange(head, v, Ordering::AcqRel, Ordering::Acquire).is_ok() {
                return Ok(());
            }
            self.versions.free_unseen(v)?;
        }
    }

    /// Take back what `epoch` wrote to the key at `key`, holding a pin of
    /// its own while it does.
    ///
    /// # Safety
    /// `key` is a key of the shape.
    pub(crate) unsafe fn void(&self, key: *const u8, epoch: Epoch) -> Result<bool, IndexError> {
        let held = self.epochs.pin()?;
        let voided = self.void_held(key, epoch);
        drop(held);
        voided
    }

    fn void_held(&self, key: *const u8, epoch: Epoch) -> Result<bool, IndexError> {
        loop {
            let Some(link) = self.find(key)?.found else {
                return Ok(false);
            };
            let kh = self.key_head(link)?;
            let head = kh.versions.load(Ordering::Acquire);
            if head == VNIL || head == VRETIRED {
                return Ok(false);
            }
            let hv = self.version_head(head)?;
            if hv.born != epoch {
                return Ok(false);
            }
            let next = hv.next.load(Ordering::Acquire);
            if kh.versions.compare_exchange(head, next, Ordering::AcqRel, Ordering::Acquire).is_ok() {
                self.retire_version(head)?;
                return Ok(true);
            }
        }
    }

    /// Take back everything `epoch` wrote, key by key, from the write list
    /// of the ticket holding it. Returns how many keys it took a version
    /// back from.
    pub(crate) fn void_epoch(&self, epoch: Epoch) -> Result<usize, IndexError> {
        let tickets = self.epochs.tickets();
        // A ticket slot holds its epoch biased by one, as `claim_ticket`
        // publishes it.
        let held_as = epoch + 1;
        let Some(slot) = (0..tickets.capacity()).find(|&s| tickets.payload(s) == Some(held_as)) else {
            return Ok(0);
        };
        let pin = self.epochs.pin()?;
        let head = self.write_head(slot).load(Ordering::Acquire);
        if (head >> 32) as u32 != epoch as u32 {
            drop(pin);
            return Ok(0);
        }
        let mut e = head as u32;
        let mut voided = 0usize;
        while e != 0 {
            let body = self.writes.body(e - 1, self.layout.entry_body)?;
            // SAFETY: an entry's head and key were written before it was
            // pushed, and the pin keeps it from reuse.
            let (next, key) = unsafe { ((*(body as *const EntryHead)).next, body.add(self.layout.entry_key_off) as *const u8) };
            if self.void_held(key, epoch)? {
                voided += 1;
            }
            e = next;
        }
        drop(pin);
        Ok(voided)
    }

    /// The version of `link` a reader at `pin` sees, or one its own epoch
    /// `own` wrote: how many versions it examined, and where the value is,
    /// or `None` for a deletion or no version at all.
    fn visible(&self, link: u32, pin: Epoch, own: Option<Epoch>) -> io::Result<(usize, Option<*const u8>)> {
        self.visible_in(self.key_body(link)?, pin, own)
    }

    /// [`Index::visible`] for the key whose body is at `body`.
    fn visible_in(&self, body: *mut u8, pin: Epoch, own: Option<Epoch>) -> io::Result<(usize, Option<*const u8>)> {
        // SAFETY: a key's body starts with its `KeyHead`, and the chunk lives
        // as long as `self`.
        let mut v = unsafe { &*(body as *const KeyHead) }.versions.load(Ordering::Acquire);
        let versions = self.versions.slots();
        let mut examined = 0usize;
        while v != VNIL && v != VRETIRED && v != VSEALED {
            examined += 1;
            let at = versions.body(v, self.layout.version_body)?;
            // SAFETY: a version's body starts with its `VersionHead`.
            let vh = unsafe { &*(at as *const VersionHead) };
            if vh.born <= pin || own == Some(vh.born) {
                if vh.flags & TOMBSTONE != 0 {
                    return Ok((examined, None));
                }
                // SAFETY: the value lies inside the slot body, past its head,
                // and was written before the version was linked.
                return Ok((examined, Some(unsafe { at.add(self.layout.value_off) } as *const u8)));
            }
            v = vh.next.load(Ordering::Acquire);
        }
        Ok((examined, None))
    }

    /// Where the value of the key at `key` that a statement reading at
    /// `pin` sees is stored: the version current at the pin, or one its own
    /// epoch `own` wrote. The value stays in place while a pin at or below
    /// `pin` is held in this index's epoch table.
    ///
    /// # Safety
    /// `key` is a key of the shape.
    pub(crate) unsafe fn get_at(&self, key: *const u8, pin: Epoch, own: Option<Epoch>) -> Result<Option<*const u8>, IndexError> {
        let start = read_ticks();
        let Some(body) = self.find_key(key)? else {
            return Ok(None);
        };
        let found = read_ticks();
        let (examined, value) = self.visible_in(body, pin, own)?;
        read_done(examined, start, found, read_ticks());
        Ok(value)
    }

    /// The first key at or past `low` in key order, unmarked.
    fn first_from(&self, low: Bound<*const u8>) -> io::Result<u32> {
        let start = match low {
            Bound::Unbounded => self.head_tower(0).load(Ordering::Acquire) & !MARK,
            Bound::Included(k) => self.find(k)?.succs[0],
            Bound::Excluded(k) => {
                let s = self.find(k)?;
                match s.found {
                    Some(at) => self.next(at, 0)?.load(Ordering::Acquire) & !MARK,
                    None => s.succs[0],
                }
            }
        };
        Ok(start)
    }

    fn past_high(&self, key: *const u8, high: Bound<*const u8>) -> bool {
        match high {
            Bound::Unbounded => false,
            Bound::Included(h) => self.cmp(key, h) == KeyOrder::Greater,
            Bound::Excluded(h) => self.cmp(key, h) != KeyOrder::Less,
        }
    }

    fn crossed(&self, low: Bound<*const u8>, high: Bound<*const u8>) -> bool {
        match (low, high) {
            (Bound::Included(l), Bound::Included(h)) => self.cmp(l, h) == KeyOrder::Greater,
            (Bound::Included(l), Bound::Excluded(h))
            | (Bound::Excluded(l), Bound::Included(h))
            | (Bound::Excluded(l), Bound::Excluded(h)) => self.cmp(l, h) != KeyOrder::Less,
            _ => false,
        }
    }

    /// Walk the range in key order, handing `visit` each key and value a
    /// statement reading at `pin` sees, its own writes under `own`
    /// included. `limit` counts versions examined; when it stops the walk
    /// the last key examined is returned to resume from with
    /// [`Bound::Excluded`], and `None` means the range is exhausted. Every
    /// key and value handed out, and the key returned, stay in place while
    /// a pin at or below `pin` is held in this index's epoch table.
    ///
    /// # Safety
    /// Each bound is a key of the shape.
    pub(crate) unsafe fn range_walk(
        &self,
        low: Bound<*const u8>,
        high: Bound<*const u8>,
        limit: usize,
        pin: Epoch,
        own: Option<Epoch>,
        mut visit: impl FnMut(*const u8, *const u8),
    ) -> Result<Option<*const u8>, IndexError> {
        if self.crossed(low, high) {
            return Ok(None);
        }
        let mut examined = 0usize;
        let mut last: Option<*const u8> = None;
        let mut curr = self.first_from(low)?;
        while curr != KEY_NIL {
            let succ = self.next(curr, 0)?.load(Ordering::Acquire);
            if succ & MARK == 0 {
                let key = self.key_ptr(curr)?;
                if self.past_high(key, high) {
                    break;
                }
                if examined >= limit && last.is_some() {
                    return Ok(last);
                }
                last = Some(key);
                let (seen, value) = self.visible(curr, pin, own)?;
                examined += seen.max(1);
                if let Some(value) = value {
                    visit(key, value);
                }
            }
            curr = succ & !MARK;
        }
        Ok(None)
    }

    /// Whether any key in the range gained a version after `since` by an
    /// epoch other than `own`, a deletion included. The caller holds a pin
    /// in this index's epoch table for the length of the call.
    ///
    /// # Safety
    /// Each bound is a key of the shape.
    pub(crate) unsafe fn changed_since(
        &self,
        low: Bound<*const u8>,
        high: Bound<*const u8>,
        since: Epoch,
        own: Option<Epoch>,
    ) -> Result<bool, IndexError> {
        if self.crossed(low, high) {
            return Ok(false);
        }
        let mut curr = self.first_from(low)?;
        while curr != KEY_NIL {
            let succ = self.next(curr, 0)?.load(Ordering::Acquire);
            if succ & MARK == 0 {
                if self.past_high(self.key_ptr(curr)?, high) {
                    break;
                }
                let mut v = self.key_head(curr)?.versions.load(Ordering::Acquire);
                while v != VNIL && v != VRETIRED && v != VSEALED {
                    let vh = self.version_head(v)?;
                    if vh.born <= since {
                        break;
                    }
                    if own != Some(vh.born) {
                        return Ok(true);
                    }
                    v = vh.next.load(Ordering::Acquire);
                }
            }
            curr = succ & !MARK;
        }
        Ok(false)
    }

    /// Drop every version superseded at or below the epoch table's reclaim
    /// horizon, remove every key whose newest version is a deletion at or
    /// below it, and reuse what earlier sweeps retired once the horizon has
    /// passed it. Returns how many versions holding a value went.
    pub(crate) fn sweep(&self) -> Result<usize, IndexError> {
        let horizon = self.epochs.reclaim_horizon();
        let pin = self.epochs.pin()?;
        let mut dropped = 0usize;
        let mut curr = self.head_tower(0).load(Ordering::Acquire) & !MARK;
        while curr != KEY_NIL {
            let succ = self.next(curr, 0)?.load(Ordering::Acquire);
            if succ & MARK == 0 {
                dropped += self.sweep_key(curr, horizon)?;
            }
            curr = succ & !MARK;
        }
        drop(pin);
        for pool in [&self.short, &self.tall, &self.versions, &self.writes] {
            pool.reclaim(horizon)?;
        }
        Ok(dropped)
    }

    fn sweep_key(&self, link: u32, horizon: Epoch) -> Result<usize, IndexError> {
        let kh = self.key_head(link)?;
        let head = kh.versions.load(Ordering::Acquire);
        if head == VRETIRED {
            return Ok(0);
        }
        if head == VNIL {
            if kh.versions.compare_exchange(VNIL, VRETIRED, Ordering::AcqRel, Ordering::Acquire).is_ok() {
                self.remove_key(link, self.key_ptr(link)?)?;
            }
            return Ok(0);
        }
        let mut v = head;
        loop {
            let vh = self.version_head(v)?;
            if vh.born <= horizon {
                if v == head && vh.flags & TOMBSTONE != 0 {
                    if kh.versions.compare_exchange(head, VRETIRED, Ordering::AcqRel, Ordering::Acquire).is_ok() {
                        self.remove_key(link, self.key_ptr(link)?)?;
                        return Ok(self.retire_run(head)?);
                    }
                    return Ok(0);
                }
                let tail = vh.next.load(Ordering::Acquire);
                if tail != VNIL
                    && tail != VSEALED
                    && vh.next.compare_exchange(tail, VNIL, Ordering::AcqRel, Ordering::Acquire).is_ok()
                {
                    return Ok(self.retire_run(tail)?);
                }
                return Ok(0);
            }
            let next = vh.next.load(Ordering::Acquire);
            if next == VNIL || next == VSEALED {
                return Ok(0);
            }
            v = next;
        }
    }

    /// Versions held, current and superseded, deletions included.
    pub(crate) fn version_count(&self) -> Result<usize, IndexError> {
        let pin = self.epochs.pin()?;
        let mut count = 0usize;
        let mut curr = self.head_tower(0).load(Ordering::Acquire) & !MARK;
        while curr != KEY_NIL {
            let succ = self.next(curr, 0)?.load(Ordering::Acquire);
            if succ & MARK == 0 {
                let mut v = self.key_head(curr)?.versions.load(Ordering::Acquire);
                while v != VNIL && v != VRETIRED && v != VSEALED {
                    count += 1;
                    v = self.version_head(v)?.next.load(Ordering::Acquire);
                }
            }
            curr = succ & !MARK;
        }
        drop(pin);
        Ok(count)
    }

    /// Keys whose newest version holds a value.
    pub(crate) fn live_count(&self) -> Result<usize, IndexError> {
        let pin = self.epochs.pin()?;
        let mut count = 0usize;
        let mut curr = self.head_tower(0).load(Ordering::Acquire) & !MARK;
        while curr != KEY_NIL {
            let succ = self.next(curr, 0)?.load(Ordering::Acquire);
            if succ & MARK == 0 {
                let v = self.key_head(curr)?.versions.load(Ordering::Acquire);
                if v != VNIL && v != VRETIRED && self.version_head(v)?.flags & TOMBSTONE == 0 {
                    count += 1;
                }
            }
            curr = succ & !MARK;
        }
        drop(pin);
        Ok(count)
    }
}

/// The bytes of `value`, as a pointer the index reads a key or value from.
fn bytes_of<T>(value: &T) -> *const u8 {
    (value as *const T).cast()
}

pub struct SharedVersionedIndex<K, V>
where
    K: Copy + Ord + 'static,
    V: Copy + 'static,
{
    core: Index<Typed<K, V>>,
}

impl<K, V> std::fmt::Debug for SharedVersionedIndex<K, V>
where
    K: Copy + Ord + 'static,
    V: Copy + 'static,
{
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SharedVersionedIndex").field("index", &self.core).finish()
    }
}

impl<K, V> SharedVersionedIndex<K, V>
where
    K: Copy + Ord + 'static,
    V: Copy + 'static,
{
    /// Obtain the index at `path` with its epoch table at `epochs_path`,
    /// making either that does not exist and attaching to either that
    /// does. `max_pins` is how many pins, and how many tickets, the epoch
    /// table holds at once.
    pub fn create(path: impl AsRef<Path>, epochs_path: impl AsRef<Path>, max_pins: usize) -> Result<Self, IndexError> {
        Self::create_with_chunks(path, epochs_path, max_pins, DEFAULT_CHUNK_BITS)
    }

    /// [`create`](Self::create) with chunk files of `1 << chunk_bits`
    /// slots, from 4 to 20 bits. An index is attached at the chunk size it
    /// was made with.
    pub fn create_with_chunks(
        path: impl AsRef<Path>,
        epochs_path: impl AsRef<Path>,
        max_pins: usize,
        chunk_bits: u32,
    ) -> Result<Self, IndexError> {
        check_chunk_bits(chunk_bits)?;
        let epochs = Arc::new(SharedEpochs::create(epochs_path, max_pins)?);
        Ok(Self { core: Index::create(path.as_ref(), epochs, Typed(PhantomData), chunk_bits)? })
    }

    /// Attach to an existing index and its epoch table.
    pub fn open(path: impl AsRef<Path>, epochs_path: impl AsRef<Path>, expected_pins: usize) -> Result<Self, IndexError> {
        let epochs = Arc::new(SharedEpochs::open(epochs_path, expected_pins)?);
        Ok(Self { core: Index::open(path.as_ref(), epochs, Typed(PhantomData))? })
    }

    /// The epoch table: pins, tickets, the reclaim horizon and dead tickets.
    #[inline]
    pub fn epochs(&self) -> &SharedEpochs {
        self.core.epochs()
    }

    /// Make `value` current for `key` in the statement holding `ticket`
    /// whose snapshot is `pin`; `None` deletes. A key whose newest version
    /// was born after the pin by another epoch refuses the write with
    /// [`IndexError::Conflict`], and the statement voids what it wrote and
    /// runs again. A second write by the same ticket replaces its own
    /// version.
    pub fn write(&self, key: K, value: Option<V>, ticket: &EpochTicket<'_>, pin: &PinGuard<'_>) -> Result<(), IndexError> {
        // SAFETY: a `K` and a `V` are the shape's key and value.
        unsafe { self.core.write_at(bytes_of(&key), value.as_ref().map(bytes_of), ticket.epoch(), ticket.slot(), pin.epoch()) }
    }

    /// Take back what `epoch` wrote to `key`: its version goes and the one
    /// it superseded is current again. `false` when the key's newest version
    /// is not `epoch`'s. An epoch's version is the newest on its key until
    /// the epoch publishes; once a later version supersedes it, it is
    /// history a reader may already have seen, and it stays.
    pub fn void(&self, key: &K, epoch: Epoch) -> Result<bool, IndexError> {
        // SAFETY: a `K` is the shape's key.
        unsafe { self.core.void(bytes_of(key), epoch) }
    }

    /// Take back everything `epoch` wrote, key by key, from the write list
    /// of the ticket holding it: the epoch of a dead ticket, before
    /// [`SharedEpochs::free_dead_ticket`]. Returns how many keys it took a
    /// version back from.
    pub fn void_epoch(&self, epoch: Epoch) -> Result<usize, IndexError> {
        self.core.void_epoch(epoch)
    }

    /// The value of `key` a statement reading at `pin` sees: the version
    /// current at the pin, or one its own ticket `own` wrote, and not one
    /// `own` has since superseded.
    pub fn get_at(&self, key: &K, pin: &PinGuard<'_>, own: Option<&EpochTicket<'_>>) -> Result<Option<V>, IndexError> {
        // SAFETY: a `K` is the shape's key, and the value it answers is a
        // `V` aligned for it, kept in place by `pin`.
        unsafe {
            Ok(self.core.get_at(bytes_of(key), pin.epoch(), own.map(EpochTicket::epoch))?.map(|v| std::ptr::read(v as *const V)))
        }
    }

    /// The entries a statement reading at `pin` sees in the range, its own
    /// writes under `own` included, in key order. `limit` counts versions
    /// examined; when it stops the walk the last key examined is returned
    /// to resume from with [`Bound::Excluded`], and `None` means the range
    /// is exhausted. A key's versions are never split across two calls, and
    /// every call examines at least one key. A range whose low bound sorts
    /// after its high bound reads as no rows.
    pub fn range_at(
        &self,
        low: Bound<&K>,
        high: Bound<&K>,
        limit: usize,
        pin: &PinGuard<'_>,
        own: Option<&EpochTicket<'_>>,
    ) -> Result<RangeRows<K, V>, IndexError> {
        let mut rows = Vec::new();
        // SAFETY: the bounds are `K`s, and every key and value the walk
        // visits, and the key it answers, are a `K` and a `V` aligned for
        // them, kept in place by `pin`.
        unsafe {
            let cursor = self.core.range_walk(
                low.map(bytes_of),
                high.map(bytes_of),
                limit,
                pin.epoch(),
                own.map(EpochTicket::epoch),
                |k, v| rows.push((std::ptr::read(k as *const K), std::ptr::read(v as *const V))),
            )?;
            Ok((rows, cursor.map(|k| std::ptr::read(k as *const K))))
        }
    }

    /// Whether any key in the range gained a version after `since` by an
    /// epoch other than `own`'s, a deletion included: what a statement
    /// validates for each range it scanned before it publishes. `_pin` is
    /// held, not read: it keeps every key and version the walk reaches in
    /// place.
    pub fn changed_since(
        &self,
        low: Bound<&K>,
        high: Bound<&K>,
        since: Epoch,
        own: Option<&EpochTicket<'_>>,
        _pin: &PinGuard<'_>,
    ) -> Result<bool, IndexError> {
        // SAFETY: the bounds are `K`s.
        unsafe { self.core.changed_since(low.map(bytes_of), high.map(bytes_of), since, own.map(EpochTicket::epoch)) }
    }

    /// Drop every version superseded at or below the epoch table's reclaim
    /// horizon, remove every key whose newest version is a deletion at or
    /// below it, and reuse what earlier sweeps retired once the horizon has
    /// passed it. Returns how many versions holding a value went.
    pub fn sweep(&self) -> Result<usize, IndexError> {
        self.core.sweep()
    }

    /// Versions held, current and superseded, deletions included.
    pub fn version_count(&self) -> Result<usize, IndexError> {
        self.core.version_count()
    }

    /// Keys whose newest version holds a value.
    pub fn live_count(&self) -> Result<usize, IndexError> {
        self.core.live_count()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use std::sync::atomic::AtomicUsize;

    type Idx = SharedVersionedIndex<u64, u64>;

    fn tmp(name: &str) -> (PathBuf, PathBuf) {
        let mut p = std::env::temp_dir();
        p.push(format!("subetha-vindex-{name}-{}.bin", std::process::id()));
        let mut e = std::env::temp_dir();
        e.push(format!("subetha-vindex-{name}-{}.epochs", std::process::id()));
        remove_family(&p);
        remove_family(&e);
        (p, e)
    }

    fn remove_family(main: &Path) {
        let dir = main.parent().expect("the path has a directory");
        let stem = main.file_name().expect("the path names a file").to_string_lossy().to_string();
        for entry in std::fs::read_dir(dir).expect("the temp directory lists") {
            let entry = entry.expect("the entry reads");
            let name = entry.file_name().to_string_lossy().to_string();
            if name == stem || name.starts_with(&format!("{stem}.")) {
                match std::fs::remove_file(entry.path()) {
                    Ok(()) => {}
                    Err(e) if e.kind() == io::ErrorKind::NotFound => {}
                    Err(e) => panic!("{} not removed: {e}", entry.path().display()),
                }
            }
        }
    }

    fn index(name: &str) -> (Idx, PathBuf, PathBuf) {
        let (p, e) = tmp(name);
        let idx = Idx::create_with_chunks(&p, &e, 32, 6).expect("the index is made");
        (idx, p, e)
    }

    fn done(idx: Idx, p: PathBuf, e: PathBuf) {
        drop(idx);
        remove_family(&p);
        remove_family(&e);
    }

    /// One statement: a ticket and a pin taken before it, then `f`, then
    /// publish.
    fn statement(idx: &Idx, f: impl FnOnce(&EpochTicket<'_>, &PinGuard<'_>)) -> Epoch {
        let pin = idx.epochs().pin().expect("a pin");
        let ticket = idx.epochs().begin().expect("a ticket");
        let epoch = ticket.epoch();
        f(&ticket, &pin);
        ticket.publish();
        drop(pin);
        epoch
    }

    #[test]
    fn a_write_is_seen_by_pins_at_or_after_its_epoch() {
        let (idx, p, e) = index("seen");
        let before = idx.epochs().pin().expect("a pin");
        statement(&idx, |t, pin| idx.write(7, Some(70), t, pin).expect("write"));
        assert_eq!(idx.get_at(&7, &before, None).expect("read"), None);
        let after = idx.epochs().pin().expect("a pin");
        assert_eq!(idx.get_at(&7, &after, None).expect("read"), Some(70));
        drop((before, after));
        done(idx, p, e);
    }

    /// The property a one-entry-per-key map lacks: an earlier pin keeps the
    /// value it saw across an update and a delete-then-reinsert.
    #[test]
    fn earlier_pins_keep_their_version_across_update_delete_and_reinsert() {
        let (idx, p, e) = index("history");
        let mut pins = Vec::new();
        statement(&idx, |t, pin| idx.write(1, Some(10), t, pin).expect("write"));
        pins.push(idx.epochs().pin().expect("a pin"));
        statement(&idx, |t, pin| idx.write(1, Some(20), t, pin).expect("write"));
        pins.push(idx.epochs().pin().expect("a pin"));
        statement(&idx, |t, pin| idx.write(1, None, t, pin).expect("delete"));
        pins.push(idx.epochs().pin().expect("a pin"));
        statement(&idx, |t, pin| idx.write(1, Some(30), t, pin).expect("write"));
        pins.push(idx.epochs().pin().expect("a pin"));
        let seen: Vec<Option<u64>> = pins.iter().map(|pin| idx.get_at(&1, pin, None).expect("read")).collect();
        assert_eq!(seen, vec![Some(10), Some(20), None, Some(30)]);
        drop(pins);
        done(idx, p, e);
    }

    /// A statement sees its own writes to a key and in a range, a deletion
    /// included, while another reader at its pin does not.
    #[test]
    fn a_statement_reads_its_own_index_writes() {
        let (idx, p, e) = index("own");
        statement(&idx, |t, pin| idx.write(1, Some(10), t, pin).expect("write"));
        let pin = idx.epochs().pin().expect("a pin");
        let ticket = idx.epochs().begin().expect("a ticket");
        idx.write(1, Some(20), &ticket, &pin).expect("write");
        idx.write(2, Some(30), &ticket, &pin).expect("write");
        assert_eq!(idx.get_at(&1, &pin, Some(&ticket)).expect("read"), Some(20));
        assert_eq!(idx.get_at(&1, &pin, None).expect("read"), Some(10));
        let (rows, _) = idx.range_at(Bound::Unbounded, Bound::Unbounded, usize::MAX, &pin, Some(&ticket)).expect("range");
        assert_eq!(rows, vec![(1, 20), (2, 30)]);
        idx.write(1, None, &ticket, &pin).expect("delete");
        assert_eq!(idx.get_at(&1, &pin, Some(&ticket)).expect("read"), None, "its own deletion");
        assert_eq!(idx.get_at(&1, &pin, None).expect("read"), Some(10));
        ticket.publish();
        drop(pin);
        done(idx, p, e);
    }

    #[test]
    fn a_version_born_after_the_pin_is_a_conflict() {
        let (idx, p, e) = index("conflict");
        let stale = idx.epochs().pin().expect("a pin");
        let won = statement(&idx, |t, pin| idx.write(1, Some(10), t, pin).expect("write"));
        let late = idx.epochs().begin().expect("a ticket");
        assert!(matches!(idx.write(1, Some(20), &late, &stale), Err(IndexError::Conflict { epoch }) if epoch == won));
        late.publish();
        let now = idx.epochs().pin().expect("a pin");
        assert_eq!(idx.get_at(&1, &now, None).expect("read"), Some(10));
        drop((stale, now));
        done(idx, p, e);
    }

    #[test]
    fn a_deletion_after_the_pin_is_a_conflict() {
        let (idx, p, e) = index("deleted");
        statement(&idx, |t, pin| idx.write(1, Some(10), t, pin).expect("write"));
        let stale = idx.epochs().pin().expect("a pin");
        let deleted = statement(&idx, |t, pin| idx.write(1, None, t, pin).expect("delete"));
        let late = idx.epochs().begin().expect("a ticket");
        assert!(matches!(idx.write(1, Some(20), &late, &stale), Err(IndexError::Conflict { epoch }) if epoch == deleted));
        late.publish();
        drop(stale);
        done(idx, p, e);
    }

    #[test]
    fn a_second_write_by_the_same_epoch_replaces_its_own_version() {
        let (idx, p, e) = index("replace");
        statement(&idx, |t, pin| idx.write(1, Some(10), t, pin).expect("write"));
        let pin = idx.epochs().pin().expect("a pin");
        let ticket = idx.epochs().begin().expect("a ticket");
        idx.write(1, Some(20), &ticket, &pin).expect("write");
        idx.write(1, Some(21), &ticket, &pin).expect("rewrite");
        assert_eq!(idx.get_at(&1, &pin, None).expect("read"), Some(10));
        assert_eq!(idx.get_at(&1, &pin, Some(&ticket)).expect("read"), Some(21));
        idx.write(1, None, &ticket, &pin).expect("delete");
        assert_eq!(idx.get_at(&1, &pin, Some(&ticket)).expect("read"), None, "its own version replaced by a deletion");
        assert_eq!(idx.get_at(&1, &pin, None).expect("read"), Some(10));
        ticket.publish();
        drop(pin);
        done(idx, p, e);
    }

    #[test]
    fn void_takes_back_the_version_and_revives_what_it_superseded() {
        let (idx, p, e) = index("void");
        statement(&idx, |t, pin| {
            idx.write(1, Some(10), t, pin).expect("write");
            idx.write(2, Some(1), t, pin).expect("write");
        });
        let pin = idx.epochs().pin().expect("a pin");
        let ticket = idx.epochs().begin().expect("a ticket");
        idx.write(1, Some(20), &ticket, &pin).expect("write");
        idx.write(2, None, &ticket, &pin).expect("delete");
        assert!(idx.void(&1, ticket.epoch()).expect("void"));
        assert!(!idx.void(&1, ticket.epoch()).expect("void"), "nothing left to take back");
        assert!(idx.void(&2, ticket.epoch()).expect("void"));
        ticket.publish();
        let now = idx.epochs().pin().expect("a pin");
        assert_eq!(idx.get_at(&1, &now, None).expect("read"), Some(10));
        assert_eq!(idx.get_at(&2, &now, None).expect("read"), Some(1), "a voided deletion leaves the key live");
        assert_eq!(idx.version_count().expect("count"), 2);
        drop((pin, now));
        done(idx, p, e);
    }

    #[test]
    fn writers_racing_on_a_live_key_admit_exactly_one() {
        let (idx, p, e) = index("race-live");
        statement(&idx, |t, pin| idx.write(1, Some(0), t, pin).expect("write"));
        let pin = idx.epochs().pin().expect("a pin");
        let won = AtomicUsize::new(0);
        std::thread::scope(|s| {
            for t in 0..8u64 {
                let (idx, won, pin) = (&idx, &won, &pin);
                s.spawn(move || {
                    let ticket = idx.epochs().begin().expect("a ticket");
                    match idx.write(1, Some(t), &ticket, pin) {
                        Ok(()) => {
                            won.fetch_add(1, Ordering::Relaxed);
                        }
                        Err(IndexError::Conflict { .. }) => {}
                        Err(other) => panic!("thread {t}: {other}"),
                    }
                    ticket.publish();
                });
            }
        });
        assert_eq!(won.load(Ordering::Relaxed), 1);
        assert_eq!(idx.live_count().expect("count"), 1);
        drop(pin);
        done(idx, p, e);
    }

    /// The map's key insert carries its first version, so of writers racing
    /// to create a key exactly one lands, where SkipIndex only promises at
    /// most one.
    #[test]
    fn writers_racing_on_an_absent_key_land_exactly_one_version() {
        let (idx, p, e) = index("race-absent");
        for round in 0..200u64 {
            let key = 1_000 + round;
            let pin = idx.epochs().pin().expect("a pin");
            let won = AtomicUsize::new(0);
            std::thread::scope(|s| {
                for t in 0..8u64 {
                    let (idx, won, pin) = (&idx, &won, &pin);
                    s.spawn(move || {
                        let ticket = idx.epochs().begin().expect("a ticket");
                        match idx.write(key, Some(t), &ticket, pin) {
                            Ok(()) => {
                                won.fetch_add(1, Ordering::Relaxed);
                            }
                            Err(IndexError::Conflict { .. }) => {}
                            Err(other) => panic!("round {round} thread {t}: {other}"),
                        }
                        ticket.publish();
                    });
                }
            });
            assert_eq!(won.load(Ordering::Relaxed), 1, "round {round}");
            drop(pin);
        }
        assert_eq!(idx.live_count().expect("count"), 200);
        done(idx, p, e);
    }

    #[test]
    fn a_range_shows_what_the_pin_sees_and_resumes_without_losing_a_key() {
        let (idx, p, e) = index("range");
        statement(&idx, |t, pin| {
            for k in 0..10u64 {
                idx.write(k, Some(k), t, pin).expect("write");
            }
        });
        let pin = idx.epochs().pin().expect("a pin");
        statement(&idx, |t, pin| {
            for k in (0..10u64).step_by(2) {
                idx.write(k, Some(k + 100), t, pin).expect("write");
            }
        });
        let (seen, _) = idx.range_at(Bound::Unbounded, Bound::Unbounded, usize::MAX, &pin, None).expect("range");
        assert_eq!(seen, (0..10u64).map(|k| (k, k)).collect::<Vec<_>>());
        let mut keys = Vec::new();
        let mut low: Option<u64> = None;
        loop {
            let lb = match &low {
                Some(k) => Bound::Excluded(k),
                None => Bound::Unbounded,
            };
            let (chunk, cursor) = idx.range_at(lb, Bound::Unbounded, 3, &pin, None).expect("range");
            keys.extend(chunk.into_iter().map(|(k, _)| k));
            match cursor {
                Some(c) => low = Some(c),
                None => break,
            }
        }
        assert_eq!(keys, (0..10u64).collect::<Vec<_>>());
        drop(pin);
        done(idx, p, e);
    }

    #[test]
    fn a_range_with_its_bounds_crossed_reads_as_no_rows() {
        let (idx, p, e) = index("crossed");
        statement(&idx, |t, pin| {
            for k in 0..10u64 {
                idx.write(k, Some(k), t, pin).expect("write");
            }
        });
        let pin = idx.epochs().pin().expect("a pin");
        let crossed = idx.range_at(Bound::Included(&7), Bound::Included(&3), usize::MAX, &pin, None).expect("range");
        assert_eq!(crossed, (Vec::new(), None));
        let empty = idx.range_at(Bound::Excluded(&5), Bound::Excluded(&5), usize::MAX, &pin, None).expect("range");
        assert_eq!(empty, (Vec::new(), None));
        assert!(!idx.changed_since(Bound::Included(&7), Bound::Included(&3), 0, None, &pin).expect("changed"));
        drop(pin);
        done(idx, p, e);
    }

    #[test]
    fn changed_since_reports_another_epochs_change_in_the_range() {
        let (idx, p, e) = index("changed");
        statement(&idx, |t, pin| idx.write(5, Some(1), t, pin).expect("write"));
        let since = idx.epochs().now();
        let pin = idx.epochs().pin().expect("a pin");
        assert!(!idx.changed_since(Bound::Included(&0), Bound::Included(&9), since, None, &pin).expect("changed"));
        drop(pin);
        statement(&idx, |t, pin| idx.write(7, Some(1), t, pin).expect("write"));
        let pin = idx.epochs().pin().expect("a pin");
        assert!(idx.changed_since(Bound::Included(&0), Bound::Included(&9), since, None, &pin).expect("changed"));
        assert!(!idx.changed_since(Bound::Included(&8), Bound::Included(&9), since, None, &pin).expect("changed"));
        drop(pin);
        let pin = idx.epochs().pin().expect("a pin");
        let ticket = idx.epochs().begin().expect("a ticket");
        idx.write(9, Some(2), &ticket, &pin).expect("write");
        assert!(
            !idx.changed_since(Bound::Included(&8), Bound::Included(&9), since, Some(&ticket), &pin).expect("changed"),
            "its own write"
        );
        ticket.publish();
        drop(pin);
        let mid = idx.epochs().now();
        statement(&idx, |t, pin| idx.write(5, None, t, pin).expect("delete"));
        let pin = idx.epochs().pin().expect("a pin");
        assert!(idx.changed_since(Bound::Included(&5), Bound::Included(&5), mid, None, &pin).expect("changed"), "a deletion");
        drop(pin);
        done(idx, p, e);
    }

    #[test]
    fn sweep_drops_versions_superseded_at_or_below_the_horizon() {
        let (idx, p, e) = index("sweep");
        statement(&idx, |t, pin| idx.write(1, Some(10), t, pin).expect("write"));
        statement(&idx, |t, pin| idx.write(1, Some(20), t, pin).expect("write"));
        let holding = idx.epochs().pin().expect("a pin");
        statement(&idx, |t, pin| idx.write(1, Some(30), t, pin).expect("write"));
        assert_eq!(idx.sweep().expect("sweep"), 1, "the version a held pin cannot see");
        assert_eq!(idx.version_count().expect("count"), 2);
        assert_eq!(idx.get_at(&1, &holding, None).expect("read"), Some(20));
        drop(holding);
        let now = idx.epochs().pin().expect("a pin");
        assert_eq!(idx.get_at(&1, &now, None).expect("read"), Some(30));
        drop(now);
        done(idx, p, e);
    }

    /// A key deleted at or below the horizon is removed from the list and
    /// its slots come back; the key can be written again.
    #[test]
    fn a_deleted_key_is_removed_by_a_sweep_and_can_come_back() {
        let (idx, p, e) = index("removed");
        statement(&idx, |t, pin| {
            for k in 0..50u64 {
                idx.write(k, Some(k), t, pin).expect("write");
            }
        });
        statement(&idx, |t, pin| {
            for k in 0..50u64 {
                idx.write(k, None, t, pin).expect("delete");
            }
        });
        assert_eq!(idx.sweep().expect("sweep"), 50, "every value superseded by a deletion");
        assert_eq!(idx.version_count().expect("count"), 0, "no key is left in the list");
        statement(&idx, |t, pin| idx.write(3, Some(33), t, pin).expect("write"));
        let pin = idx.epochs().pin().expect("a pin");
        assert_eq!(idx.get_at(&3, &pin, None).expect("read"), Some(33));
        let (rows, _) = idx.range_at(Bound::Unbounded, Bound::Unbounded, usize::MAX, &pin, None).expect("range");
        assert_eq!(rows, vec![(3, 33)]);
        drop(pin);
        done(idx, p, e);
    }

    /// The epoch of a ticket whose holder is gone is taken back key by key
    /// from its write list, and a reader never sees it.
    #[test]
    fn void_epoch_takes_back_everything_a_dead_epoch_wrote() {
        let (idx, p, e) = index("dead");
        statement(&idx, |t, pin| idx.write(1, Some(10), t, pin).expect("write"));
        let pin = idx.epochs().pin().expect("a pin");
        let ticket = idx.epochs().begin().expect("a ticket");
        let epoch = ticket.epoch();
        for k in 0..20u64 {
            idx.write(k, Some(k + 1000), &ticket, &pin).expect("write");
        }
        assert_eq!(idx.void_epoch(epoch).expect("void"), 20);
        ticket.publish();
        drop(pin);
        let now = idx.epochs().pin().expect("a pin");
        assert_eq!(idx.get_at(&1, &now, None).expect("read"), Some(10));
        assert_eq!(idx.get_at(&2, &now, None).expect("read"), None);
        assert_eq!(idx.live_count().expect("count"), 1);
        drop(now);
        done(idx, p, e);
    }

    /// A second attach reads what the first wrote, and its writes conflict
    /// with the first's as a second process's would.
    #[test]
    fn a_second_attach_shares_the_index() {
        let (idx, p, e) = index("attach");
        statement(&idx, |t, pin| idx.write(4, Some(40), t, pin).expect("write"));
        let other = Idx::open(&p, &e, 32).expect("the index opens");
        let pin = other.epochs().pin().expect("a pin");
        assert_eq!(other.get_at(&4, &pin, None).expect("read"), Some(40));
        let stale = idx.epochs().pin().expect("a pin");
        let won = statement(&other, |t, pin| other.write(4, Some(41), t, pin).expect("write"));
        let late = idx.epochs().begin().expect("a ticket");
        assert!(matches!(idx.write(4, Some(42), &late, &stale), Err(IndexError::Conflict { epoch }) if epoch == won));
        late.publish();
        drop((pin, stale));
        drop(other);
        done(idx, p, e);
    }

    /// Writers, readers and a sweeper at once. Every value a reader sees is
    /// its own key's and no newer than its pin, and one pin reads the same
    /// rows twice while the sweeper removes keys and reuses slots, so a slot
    /// reused under a reader shows as a key mismatch, a future value or two
    /// views that differ.
    #[test]
    fn sweeps_beside_writers_and_readers_lose_nothing() {
        const KEYS: u64 = 64;
        const WRITERS: usize = 3;
        let (idx, p, e) = index("stress");
        let finished = AtomicUsize::new(0);
        std::thread::scope(|s| {
            for w in 0..WRITERS as u64 {
                let (idx, finished) = (&idx, &finished);
                s.spawn(move || {
                    let mut rng = 0x9E37_79B9_7F4A_7C15u64 ^ (w + 1);
                    let mut step = || {
                        rng ^= rng << 13;
                        rng ^= rng >> 7;
                        rng ^= rng << 17;
                        rng
                    };
                    for _ in 0..1500 {
                        let pin = idx.epochs().pin().expect("a pin");
                        let ticket = idx.epochs().begin().expect("a ticket");
                        let epoch = ticket.epoch();
                        let mut written = Vec::new();
                        for _ in 0..4 {
                            let k = step() % KEYS;
                            let value = if step() % 5 == 0 { None } else { Some((epoch << 16) | k) };
                            match idx.write(k, value, &ticket, &pin) {
                                Ok(()) => written.push(k),
                                Err(IndexError::Conflict { .. }) => {
                                    for k in &written {
                                        idx.void(k, epoch).expect("void");
                                    }
                                    break;
                                }
                                Err(other) => panic!("writer {w}: {other}"),
                            }
                        }
                        ticket.publish();
                        drop(pin);
                    }
                    finished.fetch_add(1, Ordering::AcqRel);
                });
            }
            for r in 0..2 {
                let (idx, finished) = (&idx, &finished);
                s.spawn(move || {
                    while finished.load(Ordering::Acquire) < WRITERS {
                        let pin = idx.epochs().pin().expect("a pin");
                        let (first, _) =
                            idx.range_at(Bound::Unbounded, Bound::Unbounded, usize::MAX, &pin, None).expect("range");
                        for &(k, v) in &first {
                            assert_eq!(v & 0xFFFF, k, "reader {r}: key {k} holds another key's value {v:#x}");
                            assert!(v >> 16 <= pin.epoch(), "reader {r}: key {k} holds epoch {} past pin {}", v >> 16, pin.epoch());
                        }
                        let (second, _) =
                            idx.range_at(Bound::Unbounded, Bound::Unbounded, usize::MAX, &pin, None).expect("range");
                        assert_eq!(first, second, "reader {r}: one pin read two views");
                        drop(pin);
                    }
                });
            }
            let (idx, finished) = (&idx, &finished);
            s.spawn(move || {
                while finished.load(Ordering::Acquire) < WRITERS {
                    idx.sweep().expect("sweep");
                }
            });
        });
        done(idx, p, e);
    }

    /// A version retired while a pin that could be stepping over it is held
    /// is not handed out again until that pin is gone and the horizon has
    /// passed the version's retirement.
    #[test]
    fn a_retired_version_is_not_reused_while_a_pin_that_could_see_it_is_held() {
        let (idx, p, e) = index("grace");
        statement(&idx, |t, pin| idx.write(1, Some(10), t, pin).expect("write"));
        let reader = idx.epochs().pin().expect("a pin");
        let ticket = idx.epochs().begin().expect("a ticket");
        idx.write(1, Some(20), &ticket, &reader).expect("write");
        let link = idx.core.find(bytes_of(&1u64)).expect("find").found.expect("the key is there");
        let stepped_over = idx.core.key_head(link).expect("head").versions.load(Ordering::Acquire);
        assert!(idx.void(&1, ticket.epoch()).expect("void"));
        // A second retirement behind it, so a reclaim that ignored the pin
        // would move it out of the queue's dummy place onto the free list.
        idx.write(3, Some(30), &ticket, &reader).expect("write");
        assert!(idx.void(&3, ticket.epoch()).expect("void"));
        ticket.publish();
        idx.sweep().expect("sweep");
        let mut drawn = Vec::new();
        while let Some(v) = idx.core.versions.alloc(0).expect("alloc") {
            assert_ne!(v, stepped_over, "the retired version was handed out under a pin that could see it");
            drawn.push(v);
            if drawn.len() >= 256 {
                break;
            }
        }
        for v in drawn {
            idx.core.versions.free_unseen(v).expect("free");
        }
        drop(reader);
        // A slot leaves the retirement queue as its dummy, freed when the next
        // retired slot moves past it, so retire one more first.
        statement(&idx, |t, pin| {
            idx.write(3, Some(3), t, pin).expect("write");
            assert!(idx.void(&3, t.epoch()).expect("void"));
        });
        statement(&idx, |t, pin| idx.write(2, Some(2), t, pin).expect("write"));
        idx.sweep().expect("sweep");
        let mut back = false;
        for _ in 0..256 {
            match idx.core.versions.alloc(0).expect("alloc") {
                Some(v) if v == stepped_over => {
                    back = true;
                    break;
                }
                Some(_) => {}
                None => break,
            }
        }
        assert!(back, "once the pin is gone and the horizon is past it, the slot comes back");
        done(idx, p, e);
    }

    /// Atomics racer processes share: per round, how many have pinned, how
    /// many won and how many have published.
    struct Rendezvous {
        mmap: MmapMut,
        _file: File,
    }

    impl Rendezvous {
        fn create(path: &Path, rounds: usize) -> Self {
            let file = std::fs::OpenOptions::new()
                .read(true)
                .write(true)
                .create(true)
                .truncate(true)
                .open(path)
                .expect("the rendezvous file is made");
            file.set_len((rounds * 24) as u64).expect("the rendezvous file is sized");
            Self::map(file, rounds)
        }

        fn open(path: &Path, rounds: usize) -> Self {
            let file = std::fs::OpenOptions::new()
                .read(true)
                .write(true)
                .open(path)
                .expect("the rendezvous file opens");
            Self::map(file, rounds)
        }

        fn map(file: File, rounds: usize) -> Self {
            let mmap = unsafe { memmap2::MmapOptions::new().len(rounds * 24).map_mut(&file) }.expect("the rendezvous maps");
            Self { mmap, _file: file }
        }

        fn word(&self, round: usize, which: usize) -> &AtomicU64 {
            unsafe { &*(self.mmap.as_ptr().add(round * 24 + which * 8) as *const AtomicU64) }
        }

        fn wait_for(&self, round: usize, which: usize, count: u64) {
            while self.word(round, which).load(Ordering::Acquire) < count {
                std::thread::yield_now();
            }
        }
    }

    struct Peer(Option<std::process::Child>);

    impl Peer {
        fn reap(mut self) -> std::process::ExitStatus {
            let mut child = self.0.take().expect("the racer is reaped once");
            child.wait().expect("the racer is waited on")
        }
    }

    impl Drop for Peer {
        fn drop(&mut self) {
            if let Some(mut child) = self.0.take() {
                if let Err(e) = child.kill() {
                    eprintln!("the racer was not killed: {e}");
                }
                if let Err(e) = child.wait() {
                    eprintln!("the racer was not reaped: {e}");
                }
            }
        }
    }

    const ARRIVED: usize = 0;
    const WON: usize = 1;
    const PUBLISHED: usize = 2;

    /// One racer process: each round it pins, waits for every racer to
    /// pin, writes, records a win, publishes and waits for every racer to
    /// publish. Runs only when a race test starts it.
    #[test]
    fn index_racer_role() {
        let Some(spec) = std::env::var_os("SUBETHA_INDEX_RACER") else {
            return;
        };
        let spec = spec.to_string_lossy().to_string();
        let parts: Vec<&str> = spec.split('|').collect();
        let [index_path, epochs_path, rendezvous_path, racers, rounds, live] = parts[..] else {
            eprintln!("racer: a spec of {} parts: {spec}", parts.len());
            std::process::exit(2);
        };
        let racers: u64 = racers.parse().expect("the racer count parses");
        let rounds: usize = rounds.parse().expect("the round count parses");
        let live = live == "live";
        let idx = Idx::open(index_path, epochs_path, 32).expect("the racer opens the index");
        let rv = Rendezvous::open(Path::new(rendezvous_path), rounds);
        for round in 0..rounds {
            let key = if live { 1 } else { 1_000 + round as u64 };
            let pin = idx.epochs().pin().expect("a pin");
            rv.word(round, ARRIVED).fetch_add(1, Ordering::AcqRel);
            rv.wait_for(round, ARRIVED, racers);
            let ticket = idx.epochs().begin().expect("a ticket");
            match idx.write(key, Some(u64::from(std::process::id())), &ticket, &pin) {
                Ok(()) => {
                    rv.word(round, WON).fetch_add(1, Ordering::AcqRel);
                }
                Err(IndexError::Conflict { .. }) => {}
                Err(other) => {
                    eprintln!("racer round {round}: {other}");
                    std::process::exit(1);
                }
            }
            ticket.publish();
            drop(pin);
            rv.word(round, PUBLISHED).fetch_add(1, Ordering::AcqRel);
            rv.wait_for(round, PUBLISHED, racers);
        }
        std::process::exit(0);
    }

    fn race_across_processes(name: &str, live: bool, racers: usize, rounds: usize) {
        let (idx, p, e) = index(name);
        if live {
            statement(&idx, |t, pin| idx.write(1, Some(0), t, pin).expect("write"));
        }
        let mut rendezvous_path = std::env::temp_dir();
        rendezvous_path.push(format!("subetha-vindex-{name}-{}.rendezvous", std::process::id()));
        let rv = Rendezvous::create(&rendezvous_path, rounds);
        let spec = format!(
            "{}|{}|{}|{racers}|{rounds}|{}",
            p.display(),
            e.display(),
            rendezvous_path.display(),
            if live { "live" } else { "absent" }
        );
        let exe = std::env::current_exe().expect("the test binary's own path");
        let peers: Vec<Peer> = (0..racers)
            .map(|_| {
                Peer(Some(
                    std::process::Command::new(&exe)
                        .arg("shared_versioned_index::tests::index_racer_role")
                        .arg("--exact")
                        .arg("--nocapture")
                        .env("SUBETHA_INDEX_RACER", &spec)
                        .spawn()
                        .expect("a racer process spawns"),
                ))
            })
            .collect();
        for peer in peers {
            let status = peer.reap();
            assert!(status.success(), "a racer failed: {status}");
        }
        for round in 0..rounds {
            assert_eq!(rv.word(round, WON).load(Ordering::Acquire), 1, "round {round}");
        }
        assert_eq!(idx.live_count().expect("count"), if live { 1 } else { rounds });
        drop(rv);
        match std::fs::remove_file(&rendezvous_path) {
            Ok(()) => {}
            Err(err) if err.kind() == io::ErrorKind::NotFound => {}
            Err(err) => panic!("{} not removed: {err}", rendezvous_path.display()),
        }
        done(idx, p, e);
    }

    /// A writer process that ends mid-statement: it writes under a ticket
    /// and exits without publishing it. Runs only when its test starts it.
    #[test]
    fn index_dead_writer_role() {
        let Some(spec) = std::env::var_os("SUBETHA_INDEX_DEAD_WRITER") else {
            return;
        };
        let spec = spec.to_string_lossy().to_string();
        let Some((index_path, epochs_path)) = spec.split_once('|') else {
            eprintln!("dead writer: a spec without a separator: {spec}");
            std::process::exit(2);
        };
        let idx = Idx::open(index_path, epochs_path, 32).expect("the writer opens the index");
        let pin = idx.epochs().pin().expect("a pin");
        let ticket = idx.epochs().begin().expect("a ticket");
        for k in 0..20u64 {
            idx.write(k, Some(k + 5_000), &ticket, &pin).expect("write");
        }
        // Exit runs no destructor, so the ticket is never published.
        std::process::exit(0);
    }

    /// The epoch of a writer that died mid-statement is named by
    /// `dead_tickets`, invisible to every reader, taken back by
    /// `void_epoch`, and still invisible once its slot is freed and the
    /// epoch is in the past.
    #[test]
    fn a_dead_writers_epoch_is_never_seen_and_is_taken_back() {
        let (idx, p, e) = index("deadproc");
        statement(&idx, |t, pin| idx.write(1, Some(10), t, pin).expect("write"));
        let spec = format!("{}|{}", p.display(), e.display());
        let exe = std::env::current_exe().expect("the test binary's own path");
        let writer = Peer(Some(
            std::process::Command::new(&exe)
                .arg("shared_versioned_index::tests::index_dead_writer_role")
                .arg("--exact")
                .arg("--nocapture")
                .env("SUBETHA_INDEX_DEAD_WRITER", &spec)
                .spawn()
                .expect("the writer process spawns"),
        ));
        let status = writer.reap();
        assert!(status.success(), "the writer failed: {status}");
        let dead = idx.epochs().dead_tickets();
        assert_eq!(dead.len(), 1, "one dead ticket: {dead:?}");
        let epoch = dead[0];
        let pin = idx.epochs().pin().expect("a pin");
        assert!(pin.epoch() < epoch, "a pin sits below the dead ticket");
        assert_eq!(idx.get_at(&2, &pin, None).expect("read"), None);
        assert_eq!(idx.get_at(&1, &pin, None).expect("read"), Some(10));
        drop(pin);
        assert_eq!(idx.void_epoch(epoch).expect("void"), 20);
        assert!(idx.epochs().free_dead_ticket(epoch), "the slot is freed");
        statement(&idx, |t, pin| idx.write(50, Some(50), t, pin).expect("write"));
        let pin = idx.epochs().pin().expect("a pin");
        assert!(pin.epoch() > epoch, "the dead epoch is in the past");
        assert_eq!(idx.get_at(&2, &pin, None).expect("read"), None);
        assert_eq!(idx.get_at(&1, &pin, None).expect("read"), Some(10));
        assert_eq!(idx.live_count().expect("count"), 2);
        drop(pin);
        done(idx, p, e);
    }

    /// Writers in separate processes racing on one live key: exactly one
    /// wins each round.
    #[test]
    fn racers_in_separate_processes_on_a_live_key_admit_exactly_one_per_round() {
        race_across_processes("xlive", true, 4, 20);
    }

    /// Writers in separate processes racing to create a key: exactly one
    /// version lands each round.
    #[test]
    fn racers_in_separate_processes_on_absent_keys_land_exactly_one_version_each() {
        race_across_processes("xabsent", false, 4, 50);
    }

    /// Many keys across chunk boundaries, written, swept and read back while
    /// the slots of what was swept are reused.
    #[test]
    fn churn_across_chunks_keeps_every_live_key() {
        let (idx, p, e) = index("churn");
        for round in 0..6u64 {
            statement(&idx, |t, pin| {
                for k in 0..300u64 {
                    idx.write(k, Some(k * 10 + round), t, pin).expect("write");
                }
            });
            idx.sweep().expect("sweep");
        }
        let pin = idx.epochs().pin().expect("a pin");
        let (rows, cursor) = idx.range_at(Bound::Unbounded, Bound::Unbounded, usize::MAX, &pin, None).expect("range");
        assert_eq!(cursor, None);
        assert_eq!(rows, (0..300u64).map(|k| (k, k * 10 + 5)).collect::<Vec<_>>());
        drop(pin);
        done(idx, p, e);
    }
}
