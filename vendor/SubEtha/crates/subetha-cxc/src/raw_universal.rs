//! `RawUniversal` - the strategy-switching set at an element size chosen
//! at run time, and the core
//! [`SharedUniversal`](crate::shared_universal::SharedUniversal) is built
//! on.
//!
//! # Two strategies, one table format
//!
//! A small set is faster to scan than to hash, and a large one is not. The
//! set keeps one table per strategy, each its own file, and lives in one at
//! a time: the list scans from the front, the map probes from the key's
//! hash. A migration moves the set into the other table while it holds
//! data, so a caller that guessed wrong at creation is not stuck with the
//! guess.
//!
//! Nothing here migrates on its own. [`migrate_to`](RawUniversal::migrate_to)
//! is called or it is not, and the operation counts are published so the
//! caller can decide. A policy that reads those counts belongs above this
//! layer, where the knowledge of the access pattern lives.
//!
//! # Rounds
//!
//! The live table serves a round, which is the set's version. A clear moves
//! the set to the next round of the same table, which starts empty where it
//! lies; a migration moves it to the next round of the other table. Every
//! slot carries the round it was taken in, so an earlier round's keys are
//! stale where they lie, and a writer stopped across a move cannot publish
//! into the round after.
//!
//! # Every process writes
//!
//! Any number of processes insert at once, on either strategy, and none
//! waits on another, live or dead: a table's slots are claimed and published
//! by compare-and-swap. A clear or a migration seals the state word first,
//! and a writer that finds the word moved under its claim frees the claim,
//! runs the move to its end and adds its key in the round after. The move's
//! pass gives up every claim still open and, for a migration, copies every
//! key published before the seal; only then does the word move on. A mover
//! that stops part way leaves a sealed word, and the next operation that
//! meets it runs the move to its end.
//!
//! # How a reader knows the ground moved
//!
//! One 64-bit word, in a state file every handle maps, holds the strategy,
//! a generation and a version, and names the live table and round. A
//! migration or a clear moves it as a unit, so a reader that samples it
//! before and after an operation and sees the same value knows the set
//! stayed in one round. The version is bumped per move and rolls into the
//! generation, so the pair is unique across 2^48 moves on one base path;
//! exhausting both is reported rather than wrapped, because a wrapped pair
//! makes two different states compare equal.
//!
//! # Files
//!
//! ```text
//! <base>.state.bin          the state word, the shape, the counts
//! <base>-g{G}-list.bin      generation G's table on the list strategy
//! <base>-g{G}-map.bin       generation G's table on the map strategy
//! ```
//!
//! A generation's tables are laid out when the set reaches it: at creation,
//! and when the version rolls into the next generation, whose tables start
//! with no rounds behind them. The process that moves the state word past a
//! generation removes its tables. A new set starts with no tables but its
//! own, and [`unlink`](RawUniversal::unlink) removes every file of the set.

use std::fs::File;
use std::io;
use std::mem::size_of;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use memmap2::{MmapMut, MmapOptions};
use subetha_core::SwapCell;

use crate::adaptive_ring::UnlinkReport;
use crate::layout::Mismatch;
use crate::set_table::{self, Kind, Outcome, SetTable};
use crate::shared_universal::Strategy;

/// The state file's magic.
pub const UNIVERSAL_MAGIC: u32 = 0x5345_5455;
const UNIVERSAL_FORMAT: u32 = 2;

/// Set in a sealed state word: a clear or a migration is under way.
const SEALED: u64 = 1 << 8;
/// Set beside [`SEALED`] for a clear: the round's keys are dropped, not
/// carried.
const DISCARD: u64 = 1 << 9;
/// Set beside [`SEALED`] when the move goes to the map strategy.
const TO_MAP: u64 = 1 << 10;
/// The bits a move sets; the rest name the round.
const FLAGS: u64 = 0xFF00;

/// The state file every handle of a set maps: the word naming the live
/// table and round, the shape every table is laid out in, and the
/// operation counts handles share.
#[repr(C, align(64))]
pub struct UniversalHeader {
    pub magic: u32,
    pub format: u32,
    pub capacity: u64,
    pub element_size: u64,
    /// The live round's version in bits 63 to 32, its generation in bits 31
    /// to 16, the move under way in bits 15 to 8, and the live strategy in
    /// bits 7 to 0.
    pub state: AtomicU64,
    /// Inserts made through the handles that count them here.
    pub insert_count: AtomicU64,
    /// Membership tests made through the handles that count them here.
    pub contains_count: AtomicU64,
    _pad: [u8; 16],
}

const _: () = assert!(size_of::<UniversalHeader>() == 64);

/// A state word for the round at `version` and `generation` on `strategy`.
pub(crate) fn pack(version: u32, generation: u16, strategy: Strategy) -> u64 {
    (u64::from(version) << 32) | (u64::from(generation) << 16) | strategy as u64
}

/// The version, generation and strategy byte of a state word.
pub(crate) fn unpack(word: u64) -> (u32, u16, u8) {
    ((word >> 32) as u32, ((word >> 16) & 0xFFFF) as u16, (word & 0xFF) as u8)
}

/// The (version, generation) pair after this one: the next version, or
/// version 0 of the next generation once the version is at its maximum;
/// `None` once both are.
fn next_pair(version: u32, generation: u16) -> Option<(u32, u16)> {
    match version.checked_add(1) {
        Some(v) => Some((v, generation)),
        None => generation.checked_add(1).map(|g| (0, g)),
    }
}

/// Whether two state words name the same round of the same table, whatever
/// move either has under way.
fn same_round(a: u64, b: u64) -> bool {
    a & !FLAGS == b & !FLAGS
}

fn kind_of(strategy: Strategy) -> Kind {
    match strategy {
        Strategy::Vec => Kind::List,
        Strategy::Map => Kind::Map,
    }
}

/// Why an operation on the core could not be carried out.
#[derive(Debug)]
pub(crate) enum CoreError {
    /// A file or the state word was laid out for another shape.
    Mismatch(Mismatch),
    /// The state word names no strategy.
    InvalidStrategy,
    /// The set holds its capacity.
    Full,
    /// Both the version and the generation are at their maximum.
    VersionExhausted,
    /// Another move sealed the set first; the word is the state word once
    /// that move finished.
    MigrationRaced { published: u64 },
    Io(io::Error),
}

impl From<io::Error> for CoreError {
    fn from(e: io::Error) -> Self {
        match Mismatch::carried_by(&e) {
            Some(m) => Self::Mismatch(m),
            None => Self::Io(e),
        }
    }
}

fn strategy_of(word: u64) -> Result<Strategy, CoreError> {
    match word & 0xFF {
        0 => Ok(Strategy::Vec),
        1 => Ok(Strategy::Map),
        _ => Err(CoreError::InvalidStrategy),
    }
}

/// `base` with `suffix` added to its file name.
fn with_suffix(base: &Path, suffix: &str) -> PathBuf {
    let mut p = base.as_os_str().to_owned();
    p.push(suffix);
    PathBuf::from(p)
}

pub(crate) fn state_path(base: &Path) -> PathBuf {
    with_suffix(base, ".state.bin")
}

/// The file of `generation`'s table on `strategy`.
pub(crate) fn table_path(base: &Path, generation: u16, strategy: Strategy) -> PathBuf {
    let kind = match strategy {
        Strategy::Vec => "list",
        Strategy::Map => "map",
    };
    with_suffix(base, &format!("-g{generation}-{kind}.bin"))
}

/// Whether `name` is the file name of one of the tables of a set whose
/// base file name is `stem`.
fn is_table_name(name: &str, stem: &str) -> bool {
    let Some(rest) = name.strip_prefix(stem).and_then(|r| r.strip_prefix("-g")) else {
        return false;
    };
    let Some((generation, kind)) = rest.split_once('-') else {
        return false;
    };
    !generation.is_empty() && generation.bytes().all(|b| b.is_ascii_digit()) && (kind == "list.bin" || kind == "map.bin")
}

/// Remove every table file named after `base`, into a report.
fn remove_every_table(base: &Path) -> UnlinkReport {
    let mut report = UnlinkReport::default();
    let Some(stem) = base.file_name().and_then(|s| s.to_str()) else {
        return report;
    };
    let dir = match base.parent() {
        Some(p) if !p.as_os_str().is_empty() => p.to_path_buf(),
        _ => PathBuf::from("."),
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
                if entry.file_name().to_str().is_some_and(|name| is_table_name(name, stem)) {
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

/// The first failure in `report`, as the error it was.
fn first_failure(report: UnlinkReport) -> Option<io::Error> {
    report.first_failure.map(|(path, kind, message)| io::Error::new(kind, format!("{}: {message}", path.display())))
}

/// Remove `generation`'s two tables, which the set has moved past: every key
/// they held in their last round is in the live one. A handle still mapping
/// one keeps its mapping. The move is already made, so a removal refused
/// here is reported rather than failing it.
fn remove_generation(base: &Path, generation: u16) {
    for strategy in [Strategy::Vec, Strategy::Map] {
        let p = table_path(base, generation, strategy);
        match crate::region_file::remove(&p) {
            Ok(()) => {}
            Err(e) if e.kind() == io::ErrorKind::NotFound => {}
            Err(e) => eprintln!("subetha: the set table {} was not removed: {e}", p.display()),
        }
    }
}

/// Lay a fresh state file out: the shape and the word first, the magic
/// last, because an attacher spins on the magic.
///
/// # Safety
/// `ptr` addresses at least `size_of::<UniversalHeader>()` writable zeroed
/// bytes.
unsafe fn init_state(ptr: *mut u8, capacity: usize, element_size: usize, strategy: Strategy) {
    let hdr = ptr as *mut UniversalHeader;
    unsafe {
        (*hdr).format = UNIVERSAL_FORMAT;
        (*hdr).capacity = capacity as u64;
        (*hdr).element_size = element_size as u64;
        (*hdr).state.store(pack(0, 0, strategy), Ordering::Release);
        std::ptr::write_volatile(&raw mut (*hdr).magic, UNIVERSAL_MAGIC);
    }
}

/// The first of a mapped state file's magic, format, capacity and element
/// size that differs from this shape.
fn state_matches(mmap: &MmapMut, capacity: usize, element_size: usize) -> Result<(), CoreError> {
    // SAFETY: the mapping is a header long by construction.
    let hdr = unsafe { &*(mmap.as_ptr() as *const UniversalHeader) };
    Mismatch::first(&[
        ("magic", u64::from(hdr.magic), u64::from(UNIVERSAL_MAGIC)),
        ("format", u64::from(hdr.format), u64::from(UNIVERSAL_FORMAT)),
        ("capacity", hdr.capacity, capacity as u64),
        ("element size", hdr.element_size, element_size as u64),
    ])
    .map_err(CoreError::Mismatch)
}

/// One generation's two tables, the list's first.
struct Generation {
    generation: u16,
    tables: [SetTable; 2],
}

impl Generation {
    /// Obtain `generation`'s tables, laying out whichever is missing.
    fn create(base: &Path, generation: u16, capacity: usize, element_size: usize) -> io::Result<Self> {
        let table = |s: Strategy| SetTable::create_or_attach(&table_path(base, generation, s), kind_of(s), capacity, element_size);
        Ok(Self { generation, tables: [table(Strategy::Vec)?, table(Strategy::Map)?] })
    }

    /// Attach to `generation`'s tables, which must exist.
    fn open(base: &Path, generation: u16, capacity: usize, element_size: usize) -> io::Result<Self> {
        let table = |s: Strategy| SetTable::open(&table_path(base, generation, s), kind_of(s), capacity, element_size);
        Ok(Self { generation, tables: [table(Strategy::Vec)?, table(Strategy::Map)?] })
    }

    fn table(&self, strategy: Strategy) -> &SetTable {
        &self.tables[strategy as usize]
    }
}

/// Generations the set reached after a handle opened it.
struct Later(Vec<Arc<Generation>>);

/// A generation's tables: the handle's first, or a later one held for the
/// call.
enum Tables<'a> {
    First(&'a Generation),
    Later(Arc<Generation>),
}

impl std::ops::Deref for Tables<'_> {
    type Target = Generation;

    fn deref(&self) -> &Generation {
        match self {
            Self::First(generation) => generation,
            Self::Later(generation) => generation,
        }
    }
}

/// The set itself, at a run-time element size, its elements compared byte
/// for byte. [`RawUniversal`] and
/// [`SharedUniversal`](crate::shared_universal::SharedUniversal) are each a
/// face on one.
pub(crate) struct UniversalCore {
    base: PathBuf,
    capacity: usize,
    element_size: usize,
    _state_file: File,
    state: MmapMut,
    /// The tables of the generation the set was at when this handle opened it.
    first: Generation,
    /// The tables of generations it reached after.
    later: SwapCell<Later>,
}

impl UniversalCore {
    fn assemble(base: &Path, capacity: usize, element_size: usize, state_file: File, state: MmapMut, first: Generation) -> Self {
        Self {
            base: base.to_path_buf(),
            capacity,
            element_size,
            _state_file: state_file,
            state,
            first,
            later: SwapCell::new(Later(Vec::new())),
        }
    }

    /// Obtain the set at `base`: a new one starts on `strategy` with no
    /// tables but its own, and one that exists keeps the strategy in force.
    pub(crate) fn create(base: &Path, capacity: usize, element_size: usize, strategy: Strategy) -> Result<Self, CoreError> {
        let mut cleared = None;
        let (state_file, state) = crate::mmf_attach::create_or_attach(
            &state_path(base),
            size_of::<UniversalHeader>(),
            |p| {
                // Only the builder of a new state file runs this, before anyone
                // can find the set, so the tables an earlier set left go first.
                cleared = Some(remove_every_table(base));
                // SAFETY: the builder hands `init` a zeroed mapping of a header's length.
                unsafe { init_state(p, capacity, element_size, strategy) }
            },
            // SAFETY: an attacher hands the reader a mapping of a header's length.
            |p| unsafe { u64::from((*(p as *const UniversalHeader)).magic) },
            u64::from(UNIVERSAL_MAGIC),
        )?;
        if let Some(e) = cleared.and_then(first_failure) {
            return Err(e.into());
        }
        state_matches(&state, capacity, element_size)?;
        // SAFETY: the mapping is a header long.
        let word = unsafe { &*(state.as_ptr() as *const UniversalHeader) }.state.load(Ordering::Acquire);
        strategy_of(word)?;
        let first = Generation::create(base, unpack(word).1, capacity, element_size)?;
        Ok(Self::assemble(base, capacity, element_size, state_file, state, first))
    }

    /// Attach to the set another process created at `base`.
    pub(crate) fn open(base: &Path, capacity: usize, element_size: usize) -> Result<Self, CoreError> {
        let state_file = crate::region_file::open_existing(&state_path(base))?;
        let len = state_file.metadata()?.len();
        let wanted = size_of::<UniversalHeader>() as u64;
        if len < wanted {
            return Err(CoreError::Mismatch(Mismatch::new("length", len, wanted)));
        }
        // SAFETY: the file is at least a header long, checked above.
        let state = unsafe { MmapOptions::new().len(size_of::<UniversalHeader>()).map_mut(&state_file)? };
        state_matches(&state, capacity, element_size)?;
        // SAFETY: the mapping is a header long.
        let word = unsafe { &*(state.as_ptr() as *const UniversalHeader) }.state.load(Ordering::Acquire);
        strategy_of(word)?;
        let first = Generation::open(base, unpack(word).1, capacity, element_size)?;
        Ok(Self::assemble(base, capacity, element_size, state_file, state, first))
    }

    /// Truncate the state at `base`, remove every table, and start an empty
    /// set on `strategy`. For a caller that knows nobody else maps the set.
    pub(crate) fn reset(base: &Path, capacity: usize, element_size: usize, strategy: Strategy) -> Result<Self, CoreError> {
        if let Some(e) = first_failure(remove_every_table(base)) {
            return Err(e.into());
        }
        let (state_file, state) = crate::mmf_attach::reset(
            &state_path(base),
            size_of::<UniversalHeader>(),
            // SAFETY: the reset hands `init` a zeroed mapping of a header's length.
            |p| unsafe { init_state(p, capacity, element_size, strategy) },
        )?;
        let first = Generation::create(base, 0, capacity, element_size)?;
        Ok(Self::assemble(base, capacity, element_size, state_file, state, first))
    }

    /// Remove the files of the set at `base`: its state file and every
    /// table named after it.
    pub(crate) fn unlink(base: &Path) -> UnlinkReport {
        let mut report = remove_every_table(base);
        report.remove(state_path(base));
        report
    }

    pub(crate) fn base(&self) -> &Path {
        &self.base
    }

    pub(crate) fn capacity(&self) -> usize {
        self.capacity
    }

    pub(crate) fn element_size(&self) -> usize {
        self.element_size
    }

    pub(crate) fn header(&self) -> &UniversalHeader {
        // SAFETY: the mapping is a header long by construction.
        unsafe { &*(self.state.as_ptr() as *const UniversalHeader) }
    }

    /// The state word: which table and round are live, and any move under way.
    pub(crate) fn word(&self) -> u64 {
        self.header().state.load(Ordering::Acquire)
    }

    /// `generation`'s tables: this handle's first, a later generation's it
    /// holds, or ones opened now. `None` when the set has moved past
    /// `generation` and its tables are gone; the caller starts again from
    /// the state word.
    #[inline]
    fn tables(&self, generation: u16) -> Result<Option<Tables<'_>>, CoreError> {
        if generation == self.first.generation {
            return Ok(Some(Tables::First(&self.first)));
        }
        self.later_tables(generation)
    }

    /// [`tables`](Self::tables) for a generation past this handle's first.
    #[inline(never)]
    fn later_tables(&self, generation: u16) -> Result<Option<Tables<'_>>, CoreError> {
        if let Some(held) = self.later.load().0.iter().find(|g| g.generation == generation) {
            return Ok(Some(Tables::Later(Arc::clone(held))));
        }
        match Generation::open(&self.base, generation, self.capacity, self.element_size) {
            Ok(opened) => Ok(Some(Tables::Later(self.keep(opened)))),
            Err(e) => {
                let gone = matches!(e.kind(), io::ErrorKind::NotFound | io::ErrorKind::PermissionDenied);
                if gone && unpack(self.word()).1 != generation { Ok(None) } else { Err(e.into()) }
            }
        }
    }

    /// Hold `opened` among the later generations, dropping any the set has
    /// moved past, and return the one held: another thread may have opened
    /// the same generation first.
    fn keep(&self, opened: Generation) -> Arc<Generation> {
        let opened = Arc::new(opened);
        loop {
            let held = self.later.load_full();
            if let Some(existing) = held.0.iter().find(|g| g.generation == opened.generation) {
                return Arc::clone(existing);
            }
            let live = unpack(self.word()).1;
            let mut generations: Vec<Arc<Generation>> = held.0.iter().filter(|g| g.generation >= live).cloned().collect();
            generations.push(Arc::clone(&opened));
            if self.later.compare_and_set(&held, Arc::new(Later(generations))).is_ok() {
                return opened;
            }
        }
    }

    /// Add `value`, exactly `element_size` bytes; whether it was new. A value
    /// of eight bytes or fewer goes as one word.
    pub(crate) fn insert(&self, value: &[u8]) -> Result<bool, CoreError> {
        if (1..=8).contains(&value.len()) {
            self.insert_key(set_table::Word::new(value))
        } else {
            self.insert_key(set_table::Bytes(value))
        }
    }

    /// [`insert`](Self::insert) of the value as `key`.
    fn insert_key<K: set_table::Key>(&self, key: K) -> Result<bool, CoreError> {
        let h = key.hash();
        loop {
            let word = self.word();
            if word & SEALED != 0 {
                self.complete(word)?;
                continue;
            }
            let (version, generation, _) = unpack(word);
            let strategy = strategy_of(word)?;
            let Some(tables) = self.tables(generation)? else { continue };
            let current = || self.header().state.load(Ordering::SeqCst) == word;
            match tables.table(strategy).insert_key(version, h, key, false, &current) {
                Outcome::Added => return Ok(true),
                Outcome::Present => return Ok(false),
                Outcome::Full if self.word() == word => return Err(CoreError::Full),
                Outcome::Full | Outcome::Moved => {}
            }
        }
    }

    /// Whether the set holds `value`, exactly `element_size` bytes. A value
    /// of eight bytes or fewer goes as one word.
    pub(crate) fn contains(&self, value: &[u8]) -> Result<bool, CoreError> {
        if (1..=8).contains(&value.len()) {
            self.contains_key(set_table::Word::new(value))
        } else {
            self.contains_key(set_table::Bytes(value))
        }
    }

    /// [`contains`](Self::contains) of the value as `key`.
    fn contains_key<K: set_table::Key>(&self, key: K) -> Result<bool, CoreError> {
        let h = key.hash();
        loop {
            let word = self.word();
            let (version, generation, _) = unpack(word);
            let strategy = strategy_of(word)?;
            let Some(tables) = self.tables(generation)? else { continue };
            let found = tables.table(strategy).contains_key(version, h, key);
            if same_round(self.word(), word) {
                return Ok(found);
            }
        }
    }

    /// The state word once no move is under way, a move found under way run
    /// to its end first.
    fn settled(&self) -> Result<u64, CoreError> {
        loop {
            let word = self.word();
            if word & SEALED == 0 {
                return Ok(word);
            }
            self.complete(word)?;
        }
    }

    pub(crate) fn len(&self) -> Result<usize, CoreError> {
        loop {
            let word = self.settled()?;
            let (version, generation, _) = unpack(word);
            let strategy = strategy_of(word)?;
            let Some(tables) = self.tables(generation)? else { continue };
            if let Some(n) = tables.table(strategy).len(version) {
                return Ok(n);
            }
        }
    }

    /// Calls `f` with every element, in no promised order.
    pub(crate) fn for_each(&self, mut f: impl FnMut(&[u8])) -> Result<(), CoreError> {
        loop {
            let word = self.settled()?;
            let (version, generation, _) = unpack(word);
            let strategy = strategy_of(word)?;
            let Some(tables) = self.tables(generation)? else { continue };
            tables.table(strategy).for_each(version, &mut f);
            return Ok(());
        }
    }

    /// Move the set to the next round on `target`, or empty it into the next
    /// round of its own table when `discard` is set. `Err` when another move
    /// sealed the live round first; that move is run to its end before this
    /// returns.
    fn replace(&self, target: Option<Strategy>, discard: bool) -> Result<(), CoreError> {
        loop {
            let word = self.word();
            if word & SEALED != 0 {
                self.complete(word)?;
                continue;
            }
            let (version, generation, _) = unpack(word);
            let from = strategy_of(word)?;
            let to = match target {
                Some(t) if t == from => return Ok(()),
                Some(t) => t,
                None => from,
            };
            let (_, next_generation) = next_pair(version, generation).ok_or(CoreError::VersionExhausted)?;
            if next_generation != generation {
                self.keep(Generation::create(&self.base, next_generation, self.capacity, self.element_size)?);
            }
            #[cfg(test)]
            crate::test_races::pause_point();
            let sealed = word | SEALED | if discard { DISCARD } else { 0 } | if to == Strategy::Map { TO_MAP } else { 0 };
            match self.header().state.compare_exchange(word, sealed, Ordering::SeqCst, Ordering::SeqCst) {
                Ok(_) => return self.complete(sealed),
                Err(now) => {
                    if now & SEALED != 0 {
                        self.complete(now)?;
                    }
                    return Err(CoreError::MigrationRaced { published: self.word() });
                }
            }
        }
    }

    /// Move the set into the next round on `target`, carrying every
    /// element. A migration to the strategy in force changes nothing.
    pub(crate) fn migrate_to(&self, target: Strategy) -> Result<(), CoreError> {
        self.replace(Some(target), false)
    }

    /// Empty the set into the next round of its own table. A clear racing
    /// another move runs that one to its end and clears what it left.
    pub(crate) fn clear(&self) -> Result<(), CoreError> {
        loop {
            match self.replace(None, true) {
                Err(CoreError::MigrationRaced { .. }) => continue,
                other => return other,
            }
        }
    }

    /// Run the move `sealed` began to its end: its pass, then the state word
    /// moved to the round it names. Any number of processes run one at once;
    /// the one that moves the word past a generation removes its tables.
    fn complete(&self, sealed: u64) -> Result<(), CoreError> {
        if self.word() != sealed {
            // The word moves on only after a whole pass.
            return Ok(());
        }
        let (version, generation, _) = unpack(sealed);
        let from = strategy_of(sealed)?;
        let (next_version, next_generation) = next_pair(version, generation).ok_or(CoreError::VersionExhausted)?;
        let to = if sealed & TO_MAP != 0 { Strategy::Map } else { Strategy::Vec };
        let (Some(within), Some(into)) = (self.tables(generation)?, self.tables(next_generation)?) else {
            return Ok(());
        };
        let (source, target) = (within.table(from), into.table(to));
        if sealed & DISCARD == 0 {
            target.begin(next_version);
            let mut cursor = Some(0);
            let mut copied = 0usize;
            source.close(version, |key| {
                copied += 1;
                self.carry(target, next_version, key, sealed, &mut cursor)
            })?;
            // The pass copied each key once unless writers of the sealed round
            // were still settling two copies of one key, which is exactly when
            // the round's own count, which only a kept copy adds to, differs.
            let keys = if source.len(version) == Some(copied) { copied } else { source.distinct(version) };
            target.set_count(next_version, keys);
        } else {
            source.discard(version);
            target.begin(next_version);
        }
        let after = pack(next_version, next_generation, to);
        if self.header().state.compare_exchange(sealed, after, Ordering::AcqRel, Ordering::Acquire).is_ok() && next_generation != generation {
            remove_generation(&self.base, generation);
        }
        Ok(())
    }

    /// Add a key the sealed round held to the round the move goes to. A key
    /// already carried, or a move another process finished meanwhile, needs
    /// nothing more. A list takes the key past this carrier's `cursor` while
    /// no other carrier is placing keys, as
    /// [`SetTable::carry_after`] says.
    fn carry(&self, target: &SetTable, round: u32, key: &[u8], sealed: u64, cursor: &mut Option<usize>) -> Result<(), CoreError> {
        let current = || self.header().state.load(Ordering::SeqCst) == sealed;
        let h = set_table::table_hash(key);
        let outcome = if target.is_list() {
            target.carry_after(round, h, set_table::Bytes(key), cursor, &current)
        } else {
            target.insert(round, h, key, true, &current)
        };
        match outcome {
            Outcome::Added | Outcome::Present | Outcome::Moved => Ok(()),
            Outcome::Full => Err(CoreError::Full),
        }
    }

    /// Push the live table and the state file to disk, returning when they
    /// are durable.
    pub(crate) fn flush(&self) -> Result<(), CoreError> {
        loop {
            let word = self.word();
            let strategy = strategy_of(word)?;
            let Some(tables) = self.tables(unpack(word).1)? else { continue };
            tables.table(strategy).flush()?;
            self.state.flush()?;
            return Ok(());
        }
    }

    /// Seal the live round toward `target` without running the pass, as a
    /// mover that stopped after its seal leaves the set; the word the move
    /// publishes.
    #[cfg(test)]
    pub(crate) fn seal_and_stop(&self, target: Strategy) -> u64 {
        let word = self.word();
        let sealed = word | SEALED | if target == Strategy::Map { TO_MAP } else { 0 };
        self.header().state.compare_exchange(word, sealed, Ordering::SeqCst, Ordering::SeqCst).expect("the live round was not sealed");
        let (version, generation, _) = unpack(word);
        let (next_version, next_generation) = next_pair(version, generation).expect("a round after");
        pack(next_version, next_generation, target)
    }

    /// Claim a slot of the live round for `value` and leave it claimed, as a
    /// writer that died between its claim and its publish leaves it.
    #[cfg(test)]
    pub(crate) fn abandon_claim(&self, value: &[u8]) -> usize {
        let word = self.word();
        let strategy = strategy_of(word).expect("a strategy");
        let tables = self.tables(unpack(word).1).expect("the tables").expect("the live tables");
        tables.table(strategy).abandon_claim(unpack(word).0, value)
    }

    /// Put the set at `version` of its generation on the strategy in force,
    /// its table serving that round from empty: a set that has moved that
    /// many times, for a test of what lies past it.
    #[cfg(test)]
    pub(crate) fn jump_to(&self, version: u32) {
        let word = self.word();
        let (_, generation, _) = unpack(word);
        let strategy = strategy_of(word).expect("a strategy");
        let tables = self.tables(generation).expect("the tables").expect("the live tables");
        tables.table(strategy).begin(version);
        self.header().state.store(pack(version, generation, strategy), Ordering::Release);
    }
}

/// Why an operation on the container could not be carried out.
#[derive(Debug)]
pub enum RawUniversalError {
    /// A value slice was not the size this container was built for.
    WrongSize { expected: usize, found: usize },
    /// The element size asked for is zero.
    ZeroElement,
    /// The set holds its capacity.
    Full,
    /// Both the version and the generation are at their maximum, so no
    /// further round could be told apart from an earlier one.
    VersionExhausted,
    /// The state file or a table on disk was laid out for another capacity
    /// or element size, or is not one of this container's files.
    StateMismatch(Mismatch),
    /// The state word names no strategy.
    InvalidStrategy,
    /// Another process moved the container while this migration ran. That
    /// move was run to its end, and the container is on the table it
    /// chose; `published` is the state word that stands.
    MigrationRaced { published: u64 },
    /// The operating system refused to open, size or map a file; the error
    /// carries its code and message.
    Io(io::Error),
}

impl std::fmt::Display for RawUniversalError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::WrongSize { expected, found } => {
                write!(f, "the value is {found} bytes where the container holds {expected}")
            }
            Self::ZeroElement => write!(f, "the element size is zero"),
            Self::Full => write!(f, "the set holds its capacity"),
            Self::VersionExhausted => {
                write!(f, "the version and the generation are both at their maximum")
            }
            Self::StateMismatch(m) => write!(f, "a file of another shape: {m}"),
            Self::InvalidStrategy => write!(f, "the state word names no strategy"),
            Self::MigrationRaced { published } => {
                write!(f, "another process migrated the container first; word {published:#x} stands")
            }
            Self::Io(e) => write!(f, "io: {e}"),
        }
    }
}

impl std::error::Error for RawUniversalError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(e) => Some(e),
            _ => None,
        }
    }
}

impl From<io::Error> for RawUniversalError {
    fn from(e: io::Error) -> Self {
        Self::Io(e)
    }
}

impl From<CoreError> for RawUniversalError {
    fn from(e: CoreError) -> Self {
        match e {
            CoreError::Mismatch(m) => Self::StateMismatch(m),
            CoreError::InvalidStrategy => Self::InvalidStrategy,
            CoreError::Full => Self::Full,
            CoreError::VersionExhausted => Self::VersionExhausted,
            CoreError::MigrationRaced { published } => Self::MigrationRaced { published },
            CoreError::Io(e) => Self::Io(e),
        }
    }
}

/// The strategy word, and how many operations of each kind have run.
#[derive(Debug, Clone, Copy)]
pub struct RawUniversalState {
    pub strategy: Strategy,
    pub version: u32,
    pub generation: u16,
    pub inserts: u64,
    pub contains: u64,
}

impl RawUniversalState {
    /// The word a reader compares across an operation. Equal before and
    /// after means the set stayed in one round.
    pub fn stamp(&self) -> u64 {
        pack(self.version, self.generation, self.strategy)
    }
}

/// The strategy-switching set at a run-time element size, its elements
/// compared byte for byte.
pub struct RawUniversal {
    core: UniversalCore,
    inserts: AtomicU64,
    contains: AtomicU64,
}

impl RawUniversal {
    fn with_core(core: UniversalCore) -> Self {
        Self { core, inserts: AtomicU64::new(0), contains: AtomicU64::new(0) }
    }

    /// Obtain the container under `base_path`: a new one starts on
    /// `strategy`, and one that already exists keeps the strategy in
    /// force, whatever this caller would have started it on.
    pub fn create(
        base_path: impl AsRef<Path>,
        capacity: usize,
        element_size: usize,
        strategy: Strategy,
    ) -> Result<Self, RawUniversalError> {
        if element_size == 0 {
            return Err(RawUniversalError::ZeroElement);
        }
        Ok(Self::with_core(UniversalCore::create(base_path.as_ref(), capacity, element_size, strategy)?))
    }

    /// Attach to a container another process created. The strategy in
    /// force is read from the state file rather than declared.
    pub fn open(base_path: impl AsRef<Path>, capacity: usize, element_size: usize) -> Result<Self, RawUniversalError> {
        if element_size == 0 {
            return Err(RawUniversalError::ZeroElement);
        }
        Ok(Self::with_core(UniversalCore::open(base_path.as_ref(), capacity, element_size)?))
    }

    /// Remove every file of the container under `base_path`: its state
    /// file and each table. A missing file is counted, not an error; a
    /// refusal is counted and the first one is named. A handle still open
    /// keeps its mappings until it drops.
    pub fn unlink(base_path: impl AsRef<Path>) -> UnlinkReport {
        UniversalCore::unlink(base_path.as_ref())
    }

    pub fn base(&self) -> &Path {
        self.core.base()
    }

    pub fn capacity(&self) -> usize {
        self.core.capacity()
    }

    pub fn element_size(&self) -> usize {
        self.core.element_size()
    }

    /// The strategy word and the operation counts, read together. The
    /// word is the shared one; the counts are this handle's own.
    pub fn state(&self) -> RawUniversalState {
        let (version, generation, strategy) = unpack(self.core.word());
        RawUniversalState {
            strategy: if strategy == Strategy::Map as u8 { Strategy::Map } else { Strategy::Vec },
            version,
            generation,
            inserts: self.inserts.load(Ordering::Relaxed),
            contains: self.contains.load(Ordering::Relaxed),
        }
    }

    pub fn strategy(&self) -> Strategy {
        self.state().strategy
    }

    fn sized(&self, value: &[u8]) -> Result<(), RawUniversalError> {
        if value.len() == self.core.element_size() {
            Ok(())
        } else {
            Err(RawUniversalError::WrongSize { expected: self.core.element_size(), found: value.len() })
        }
    }

    /// Add `value` if it is not already present.
    ///
    /// `true` when it was added, `false` when the set already held it. Of
    /// several handles adding one value at once, in any processes, exactly
    /// one is told it was added.
    pub fn insert(&self, value: &[u8]) -> Result<bool, RawUniversalError> {
        self.sized(value)?;
        self.inserts.fetch_add(1, Ordering::Relaxed);
        Ok(self.core.insert(value)?)
    }

    /// Whether the set holds `value`.
    pub fn contains(&self, value: &[u8]) -> Result<bool, RawUniversalError> {
        self.sized(value)?;
        self.contains.fetch_add(1, Ordering::Relaxed);
        Ok(self.core.contains(value)?)
    }

    /// Elements held. A move found part way through is run to its end
    /// first, so the count is the set's and not a sealed round's.
    pub fn len(&self) -> Result<usize, RawUniversalError> {
        Ok(self.core.len()?)
    }

    pub fn is_empty(&self) -> Result<bool, RawUniversalError> {
        Ok(self.len()? == 0)
    }

    /// Every element, in no promised order.
    pub fn snapshot(&self) -> Result<Vec<Vec<u8>>, RawUniversalError> {
        let mut out = Vec::new();
        self.core.for_each(|element| out.push(element.to_vec()))?;
        Ok(out)
    }

    /// Move to `target`, carrying every element across.
    ///
    /// The live round is sealed first; writers that find the seal run the
    /// move to its end and add to the round after, and the state word moves
    /// only once every element published before the seal is copied.
    /// Migrating to the strategy already in force changes nothing. Another
    /// process's move sealing the set first is run to its end and reported
    /// as `MigrationRaced`.
    pub fn migrate_to(&self, target: Strategy) -> Result<(), RawUniversalError> {
        Ok(self.core.migrate_to(target)?)
    }

    /// Empty the set: it moves to the next round of its own table, which
    /// starts empty, so the state word moves as for a migration. An element
    /// added while the clear runs is kept or dropped as a whole.
    pub fn clear(&self) -> Result<(), RawUniversalError> {
        Ok(self.core.clear()?)
    }

    /// Push the live table and the state file to disk and wait for them.
    pub fn flush(&self) -> Result<(), RawUniversalError> {
        Ok(self.core.flush()?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let mut p = std::env::temp_dir();
        p.push(format!("subetha_raw_universal_{name}_{}", std::process::id()));
        p
    }

    fn cleanup(base: &Path) {
        let report = RawUniversal::unlink(base);
        assert_eq!(report.failed, 0, "the set's files were not all removed: {report:?}");
    }

    fn le(k: u64) -> [u8; 8] {
        k.to_le_bytes()
    }

    fn elements(u: &RawUniversal) -> Vec<u64> {
        let mut out: Vec<u64> = u
            .snapshot()
            .expect("snapshot")
            .into_iter()
            .map(|e| u64::from_le_bytes(e.try_into().expect("8 bytes")))
            .collect();
        out.sort_unstable();
        out
    }

    /// The set's files on disk: the state file and each table, by name.
    fn files_of(base: &Path) -> Vec<String> {
        let stem = base.file_name().and_then(|s| s.to_str()).expect("a base name").to_string();
        let mut names: Vec<String> = std::fs::read_dir(base.parent().expect("a directory"))
            .expect("the directory lists")
            .map(|entry| entry.expect("the entry reads").file_name().to_string_lossy().to_string())
            .filter(|name| name.starts_with(&stem))
            .map(|name| name[stem.len()..].to_string())
            .collect();
        names.sort_unstable();
        names
    }

    #[test]
    fn a_set_holds_each_value_once_on_either_strategy() {
        for strategy in [Strategy::Vec, Strategy::Map] {
            let base = scratch(&format!("once_{}", strategy as u8));
            cleanup(&base);
            let u = RawUniversal::create(&base, 16, 8, strategy).expect("created");

            assert!(u.insert(&le(1)).expect("first"), "newly added");
            assert!(!u.insert(&le(1)).expect("again"), "already present");
            assert_eq!(u.len().expect("len"), 1, "{strategy:?} kept one copy");
            assert!(u.contains(&le(1)).expect("contains"));
            assert!(!u.contains(&le(2)).expect("contains"));

            drop(u);
            cleanup(&base);
        }
    }

    #[test]
    fn a_set_at_its_capacity_refuses_a_new_value_and_answers_a_held_one() {
        for strategy in [Strategy::Vec, Strategy::Map] {
            let base = scratch(&format!("capacity_{}", strategy as u8));
            cleanup(&base);
            let u = RawUniversal::create(&base, 4, 8, strategy).expect("created");
            for i in 0u64..4 {
                assert!(u.insert(&le(i)).expect("insert"));
            }
            assert!(matches!(u.insert(&le(9)), Err(RawUniversalError::Full)), "{strategy:?}");
            assert!(!u.insert(&le(2)).expect("a held value at capacity"));
            drop(u);
            cleanup(&base);
        }
    }

    #[test]
    fn a_migration_carries_every_element_across() {
        let base = scratch("migrate");
        cleanup(&base);
        let u = RawUniversal::create(&base, 32, 8, Strategy::Vec).expect("created");
        for i in 0u64..10 {
            u.insert(&le(i)).expect("insert");
        }
        assert_eq!(u.strategy(), Strategy::Vec);
        assert_eq!(u.len().expect("len"), 10);

        u.migrate_to(Strategy::Map).expect("to map");
        assert_eq!(u.strategy(), Strategy::Map);
        assert_eq!(u.len().expect("len"), 10, "nothing lost crossing over");
        for i in 0u64..10 {
            assert!(u.contains(&le(i)).expect("contains"), "{i} survived");
        }

        u.migrate_to(Strategy::Vec).expect("to vec");
        assert_eq!(u.len().expect("len"), 10);
        assert_eq!(elements(&u), (0..10).collect::<Vec<_>>(), "every element survived the return");

        drop(u);
        cleanup(&base);
    }

    #[test]
    fn the_state_word_moves_on_every_migration_and_clear_and_not_otherwise() {
        let base = scratch("stamp");
        cleanup(&base);
        let u = RawUniversal::create(&base, 16, 8, Strategy::Vec).expect("created");

        let before = u.state().stamp();
        u.insert(&le(5)).expect("insert");
        u.contains(&le(5)).expect("contains");
        assert_eq!(u.state().stamp(), before, "ordinary work leaves the word alone");

        u.migrate_to(Strategy::Map).expect("migrate");
        let after = u.state().stamp();
        assert_ne!(after, before, "a migration is visible in the word");
        assert_eq!(u.state().version, 1);

        // Migrating to the strategy already in force changes nothing, so
        // it must not bump the version either.
        u.migrate_to(Strategy::Map).expect("same strategy");
        assert_eq!(u.state().stamp(), after, "no change, no version bump");

        u.clear().expect("clear");
        assert_eq!(u.state().version, 2, "a clear moves the set to the next round");
        assert_eq!(u.strategy(), Strategy::Map, "and keeps the strategy");
        assert!(u.is_empty().expect("len"));
        assert!(!u.contains(&le(5)).expect("contains"), "a clear drops what the set held");

        drop(u);
        cleanup(&base);
    }

    #[test]
    fn the_operation_counts_are_published_for_a_policy_above_this_layer() {
        let base = scratch("counts");
        cleanup(&base);
        let u = RawUniversal::create(&base, 16, 8, Strategy::Vec).expect("created");

        for i in 0u64..3 {
            u.insert(&le(i)).expect("insert");
        }
        for _ in 0..7 {
            u.contains(&le(1)).expect("contains");
        }
        let s = u.state();
        assert_eq!(s.inserts, 3);
        assert_eq!(s.contains, 7);
        // Nothing migrated on its own; the caller decides.
        assert_eq!(s.strategy, Strategy::Vec);
        assert_eq!(s.version, 0);

        drop(u);
        cleanup(&base);
    }

    #[test]
    fn a_value_of_the_wrong_size_is_refused_rather_than_padded() {
        let base = scratch("sizes");
        cleanup(&base);
        let u = RawUniversal::create(&base, 16, 8, Strategy::Vec).expect("created");
        assert!(matches!(u.insert(&[0u8; 4]), Err(RawUniversalError::WrongSize { expected: 8, found: 4 })));
        assert!(matches!(u.contains(&[0u8; 16]), Err(RawUniversalError::WrongSize { expected: 8, found: 16 })));
        assert_eq!(u.len().expect("len"), 0);
        drop(u);
        cleanup(&base);
    }

    #[test]
    fn a_zero_element_size_is_refused() {
        let base = scratch("zeroelem");
        cleanup(&base);
        assert!(matches!(RawUniversal::create(&base, 16, 0, Strategy::Vec), Err(RawUniversalError::ZeroElement)));
        cleanup(&base);
    }

    #[test]
    fn a_snapshot_holds_the_same_set_on_either_strategy() {
        let base = scratch("snapshot");
        cleanup(&base);
        let u = RawUniversal::create(&base, 32, 8, Strategy::Vec).expect("created");
        for i in 0u64..5 {
            u.insert(&le(i)).expect("insert");
        }
        let from_vec = elements(&u);
        u.migrate_to(Strategy::Map).expect("migrate");
        // Order is not promised, so both are sorted before comparing; what
        // is promised is that the same elements are present.
        assert_eq!(from_vec, elements(&u));
        drop(u);
        cleanup(&base);
    }

    #[test]
    fn a_second_handle_reads_what_the_first_stored() {
        let base = scratch("shared");
        cleanup(&base);
        let a = RawUniversal::create(&base, 16, 8, Strategy::Vec).expect("created");
        a.insert(&le(42)).expect("insert");

        let b = RawUniversal::open(&base, 16, 8).expect("opened");
        assert!(b.contains(&le(42)).expect("contains"));
        assert_eq!(b.len().expect("len"), 1);

        drop((a, b));
        cleanup(&base);
    }

    /// A reader opened before a migration reads the table and round the
    /// state word names on its next operation, so what is added after the
    /// move is visible to it.
    #[test]
    fn a_migration_through_one_handle_is_what_every_other_handle_reads() {
        let base = scratch("migrate_shared");
        cleanup(&base);
        let a = RawUniversal::create(&base, 32, 8, Strategy::Vec).expect("created");
        let b = RawUniversal::open(&base, 32, 8).expect("opened");
        for i in 0u64..5 {
            a.insert(&le(i)).expect("insert");
        }
        let seen_before = b.state().stamp();

        a.migrate_to(Strategy::Map).expect("migrate");
        assert_eq!(b.strategy(), Strategy::Map, "the other handle reads the live strategy");
        assert_ne!(b.state().stamp(), seen_before, "and its stamp moved with it");
        for i in 0u64..5 {
            assert!(b.contains(&le(i)).expect("contains"), "{i} is in the new round for the other handle");
        }

        assert!(a.insert(&le(100)).expect("insert after"));
        assert!(b.contains(&le(100)).expect("the other handle sees it"));
        assert!(b.insert(&le(200)).expect("insert through the other"));
        assert!(a.contains(&le(200)).expect("and the first sees that"));
        assert_eq!(a.len().expect("len"), 7);
        assert_eq!(b.len().expect("len"), 7);

        drop((a, b));
        cleanup(&base);
    }

    /// Obtaining an existing container keeps the strategy in force rather
    /// than the one the late caller would have started it on, and a shape
    /// that disagrees is refused by the field that differs.
    #[test]
    fn a_late_create_attaches_to_the_strategy_in_force() {
        let base = scratch("late_create");
        cleanup(&base);
        let a = RawUniversal::create(&base, 16, 8, Strategy::Vec).expect("created");
        a.insert(&le(7)).expect("insert");
        a.migrate_to(Strategy::Map).expect("migrate");

        let late = RawUniversal::create(&base, 16, 8, Strategy::Vec).expect("obtained");
        assert_eq!(late.strategy(), Strategy::Map, "the live strategy, not the argument");
        assert_eq!(late.state().version, 1);
        assert!(late.contains(&le(7)).expect("contains"));

        assert!(matches!(
            RawUniversal::create(&base, 16, 4, Strategy::Vec),
            Err(RawUniversalError::StateMismatch(Mismatch { field: "element size", found: 8, wanted: 4 }))
        ));
        assert!(matches!(
            RawUniversal::open(&base, 64, 8),
            Err(RawUniversalError::StateMismatch(Mismatch { field: "capacity", found: 16, wanted: 64 }))
        ));
        drop((a, late));
        cleanup(&base);
    }

    /// Two migrations racing are not merged: the one that seals second runs
    /// the first to its end and reports the word that stands.
    #[test]
    fn a_migration_that_loses_the_seal_runs_the_winner_and_reports_it() {
        let base = scratch("raced");
        cleanup(&base);
        let a = Arc::new(RawUniversal::create(&base, 16, 8, Strategy::Vec).expect("created"));
        let b = RawUniversal::open(&base, 16, 8).expect("opened");
        for i in 0u64..6 {
            a.insert(&le(i)).expect("insert");
        }
        let loser = Arc::clone(&a);
        let (pause, raced) = crate::test_races::stopped(move || loser.migrate_to(Strategy::Map));
        b.migrate_to(Strategy::Map).expect("the winner migrates");
        let published = b.state().stamp();
        pause.release();
        match raced.join().expect("the losing migration's thread") {
            Err(RawUniversalError::MigrationRaced { published: stands }) => assert_eq!(stands, published),
            other => panic!("the stopped migration was not refused as raced: {other:?}"),
        }
        assert_eq!(unpack(published), (1, 0, Strategy::Map as u8));
        assert_eq!(files_of(&base), vec!["-g0-list.bin", "-g0-map.bin", ".state.bin"], "no table was laid out for the loser");
        assert_eq!(elements(&a), (0..6).collect::<Vec<_>>());
        drop((a, b));
        cleanup(&base);
    }

    /// A mover that stops after its seal: the next call meets the sealed
    /// word, runs the move to its end, and lands in the round after.
    #[test]
    fn a_migration_stopped_after_its_seal_is_finished_by_the_next_call() {
        for (from, to) in [(Strategy::Vec, Strategy::Map), (Strategy::Map, Strategy::Vec)] {
            let base = scratch(&format!("stopped_{}", from as u8));
            cleanup(&base);
            let u = RawUniversal::create(&base, 64, 8, from).expect("created");
            for i in 0u64..20 {
                u.insert(&le(i)).expect("insert");
            }
            let next = u.core.seal_and_stop(to);
            assert_eq!(u.state().version, 0, "the word has not moved");
            assert!(u.contains(&le(5)).expect("contains"), "{from:?}: a reader reads the sealed round");
            assert!(u.insert(&le(100)).expect("a new key"), "{from:?}: a writer finishes the move and adds");
            assert_eq!(u.core.word(), next, "{from:?}: the move is finished");
            assert!(!u.insert(&le(5)).expect("a held key"), "{from:?}: a key the move carried is held");
            assert_eq!(u.len().expect("len"), 21);
            assert_eq!(u.strategy(), to);
            let mut want: Vec<u64> = (0..20).collect();
            want.push(100);
            assert_eq!(elements(&u), want);
            drop(u);
            cleanup(&base);
        }
    }

    /// A writer that died between its claim and its publish is passed, by
    /// later writers and by a migration's pass alike.
    #[test]
    fn a_claim_left_open_stops_neither_writers_nor_a_migration() {
        let base = scratch("open_claim");
        cleanup(&base);
        let u = RawUniversal::create(&base, 32, 8, Strategy::Vec).expect("created");
        u.insert(&le(1)).expect("insert");
        u.core.abandon_claim(&le(2));
        u.core.abandon_claim(&le(3));
        assert!(u.insert(&le(2)).expect("insert"), "a key whose claim was left open still lands");
        u.migrate_to(Strategy::Map).expect("the pass gives the open claim up");
        assert!(u.insert(&le(3)).expect("insert"), "the other open claim did not move with the set");
        assert_eq!(elements(&u), vec![1, 2, 3]);
        drop(u);
        cleanup(&base);
    }

    /// A thread that runs `step` over and over until told to stop, counting
    /// the steps it finished where writers can wait on the count.
    fn mover(step: impl Fn(u32) + Send + 'static) -> (Arc<AtomicU64>, Arc<std::sync::atomic::AtomicBool>, std::thread::JoinHandle<()>) {
        let moves = Arc::new(AtomicU64::new(0));
        let stop = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let thread = {
            let (moves, stop) = (Arc::clone(&moves), Arc::clone(&stop));
            std::thread::spawn(move || {
                while !stop.load(Ordering::Acquire) {
                    step(moves.load(Ordering::Relaxed) as u32);
                    moves.fetch_add(1, Ordering::AcqRel);
                }
            })
        };
        (moves, stop, thread)
    }

    /// Wait until `moves` has passed `since`: a move landed while the
    /// caller was writing.
    fn wait_for_a_move(moves: &AtomicU64, since: u64) {
        while moves.load(Ordering::Acquire) <= since {
            std::thread::yield_now();
        }
    }

    /// Writers in threads adding overlapping values while another thread
    /// migrates the set back and forth, every writer seeing a move land
    /// partway through: of each value's adds, exactly one reports it new,
    /// and the set holds every value once.
    #[test]
    fn writers_racing_migrations_add_each_value_exactly_once() {
        let base = scratch("migrating_writers");
        cleanup(&base);
        let u = Arc::new(RawUniversal::create(&base, 1024, 8, Strategy::Vec).expect("created"));
        let writers = std::thread::available_parallelism().expect("the host reports its parallelism").get().clamp(2, 7);
        let (moves, stop, migrator) = {
            let u = Arc::clone(&u);
            mover(move |n| {
                let target = if n.is_multiple_of(2) { Strategy::Map } else { Strategy::Vec };
                u.migrate_to(target).expect("a migration");
            })
        };
        let handles: Vec<_> = (0..writers)
            .map(|w| {
                let (u, moves) = (Arc::clone(&u), Arc::clone(&moves));
                std::thread::spawn(move || {
                    let since = moves.load(Ordering::Acquire);
                    let mut added = vec![0u32; 600];
                    for step in 0..600u64 {
                        if step == 300 {
                            wait_for_a_move(&moves, since);
                        }
                        let k = (step + 97 * w as u64) % 600;
                        if u.insert(&le(k)).expect("an insert") {
                            added[k as usize] += 1;
                        }
                    }
                    added
                })
            })
            .collect();
        let mut total = vec![0u32; 600];
        for h in handles {
            for (k, n) in h.join().expect("a writer").into_iter().enumerate() {
                total[k] += n;
            }
        }
        stop.store(true, Ordering::Release);
        migrator.join().expect("the migrator");
        assert!(total.iter().all(|&n| n == 1), "every value added exactly once: {total:?}");
        assert_eq!(u.len().expect("len"), 600);
        assert_eq!(elements(&u), (0..600).collect::<Vec<_>>());
        drop(u);
        cleanup(&base);
    }

    /// Threads finishing one sealed move into the list at the same moment
    /// each carry the keys: one places them past its own cursor, and the
    /// others, finding keys they did not place, probe for each. Every key
    /// lands once.
    #[test]
    fn carriers_finishing_one_move_into_a_list_place_every_key_once() {
        let base = scratch("carriers");
        cleanup(&base);
        let u = Arc::new(RawUniversal::create(&base, 1024, 8, Strategy::Map).expect("created"));
        let tables: Vec<usize> = u.core.first.tables.iter().map(SetTable::trace_name).collect();
        for round in 0..20u64 {
            u.clear().expect("clear");
            for k in 0..500u64 {
                u.insert(&le(k * 3 + round)).expect("insert");
            }
            crate::set_trace::name(&tables);
            u.core.seal_and_stop(Strategy::Vec);
            let ready = Arc::new(AtomicU64::new(0));
            let carriers: Vec<_> = (0..4)
                .map(|_| {
                    let (u, ready) = (Arc::clone(&u), Arc::clone(&ready));
                    std::thread::spawn(move || {
                        ready.fetch_add(1, Ordering::AcqRel);
                        while ready.load(Ordering::Acquire) < 4 {
                            std::hint::spin_loop();
                        }
                        u.len().expect("len, which finishes the move first")
                    })
                })
                .collect();
            let counts: Vec<usize> = carriers.into_iter().map(|c| c.join().expect("a carrier")).collect();
            assert_eq!(u.strategy(), Strategy::Vec);
            let held = elements(&u);
            let wanted: Vec<u64> = (0..500).map(|k| k * 3 + round).collect();
            if held != wanted {
                let mut report = format!("round {round}: {} keys held, {} wanted, counts {counts:?}\n", held.len(), wanted.len());
                for (i, k) in held.iter().enumerate() {
                    if i > 0 && held[i - 1] == *k {
                        report.push_str(&format!("key {k} is held twice:\n{}", crate::set_trace::story(*k)));
                    }
                    if wanted.binary_search(k).is_err() {
                        report.push_str(&format!("key {k} was never added:\n{}", crate::set_trace::story(*k)));
                    }
                }
                for k in &wanted {
                    if held.binary_search(k).is_err() {
                        report.push_str(&format!("key {k} is missing:\n{}", crate::set_trace::story(*k)));
                    }
                }
                panic!("{report}");
            }
            assert_eq!(counts, vec![500; 4], "round {round}: the count each carrier read");
            u.migrate_to(Strategy::Map).expect("back to the map");
        }
        drop(u);
        cleanup(&base);
    }

    /// Writers adding values while another thread clears the set over and
    /// over, every writer seeing a clear land partway through: the count
    /// agrees with the keys left, and adding every value again holds each
    /// once.
    #[test]
    fn writers_racing_clears_leave_a_count_that_agrees_with_the_keys() {
        let base = scratch("clearing_writers");
        cleanup(&base);
        let u = Arc::new(RawUniversal::create(&base, 1024, 8, Strategy::Map).expect("created"));
        let (clears, stop, clearer) = {
            let u = Arc::clone(&u);
            mover(move |_| u.clear().expect("a clear"))
        };
        let handles: Vec<_> = (0..4u64)
            .map(|w| {
                let (u, clears) = (Arc::clone(&u), Arc::clone(&clears));
                std::thread::spawn(move || {
                    let since = clears.load(Ordering::Acquire);
                    for step in 0..2000u64 {
                        if step == 1000 {
                            wait_for_a_move(&clears, since);
                        }
                        u.insert(&le((step * 7 + w) % 500)).expect("an insert");
                    }
                })
            })
            .collect();
        for h in handles {
            h.join().expect("a writer");
        }
        stop.store(true, Ordering::Release);
        clearer.join().expect("the clearer");
        let keys = elements(&u);
        assert_eq!(u.len().expect("len"), keys.len(), "the count agrees with the keys");
        for k in 0u64..500 {
            u.insert(&le(k)).expect("insert");
        }
        assert_eq!(u.len().expect("len"), 500);
        assert_eq!(elements(&u), (0..500).collect::<Vec<_>>());
        drop(u);
        cleanup(&base);
    }

    /// A clear empties the set where it lies: the files stay as they are,
    /// and values added again land in the round after.
    #[test]
    fn a_clear_starts_the_round_after_empty_in_the_same_files() {
        for strategy in [Strategy::Vec, Strategy::Map] {
            let base = scratch(&format!("clear_in_place_{}", strategy as u8));
            cleanup(&base);
            let u = RawUniversal::create(&base, 64, 8, strategy).expect("created");
            let files = files_of(&base);
            for round in 0u32..5 {
                for k in 0u64..40 {
                    assert!(u.insert(&le(k + u64::from(round % 2) * 7)).expect("insert"), "{strategy:?} round {round}");
                }
                assert_eq!(u.len().expect("len"), 40);
                u.clear().expect("clear");
                assert!(u.is_empty().expect("len"), "{strategy:?} round {round}");
                assert!((0u64..50).all(|k| !u.contains(&le(k)).expect("contains")));
            }
            assert_eq!(files_of(&base), files, "{strategy:?}: no file was laid out or removed");
            drop(u);
            cleanup(&base);
        }
    }

    /// Past the last version of a generation the set moves to the next
    /// generation's tables, carrying or dropping its elements as the move
    /// says, and the tables it left are removed.
    #[test]
    fn a_version_rollover_moves_the_set_to_the_next_generations_tables() {
        let base = scratch("rollover");
        cleanup(&base);
        let u = RawUniversal::create(&base, 32, 8, Strategy::Vec).expect("created");
        u.core.jump_to(u32::MAX);
        for i in 0u64..8 {
            assert!(u.insert(&le(i)).expect("insert"));
        }
        u.migrate_to(Strategy::Map).expect("a migration across the rollover");
        assert_eq!((u.state().version, u.state().generation), (0, 1));
        assert_eq!(elements(&u), (0..8).collect::<Vec<_>>(), "every element crossed");
        assert_eq!(files_of(&base), vec!["-g1-list.bin", "-g1-map.bin", ".state.bin"], "generation 0's tables are removed");
        let other = RawUniversal::open(&base, 32, 8).expect("a handle opened after");
        assert!(other.contains(&le(3)).expect("contains"));
        u.core.jump_to(u32::MAX);
        assert!(u.insert(&le(50)).expect("insert"));
        u.clear().expect("a clear across the rollover");
        assert_eq!((u.state().version, u.state().generation), (0, 2));
        assert!(u.is_empty().expect("len"));
        assert!(!other.contains(&le(50)).expect("contains"), "a handle of an earlier generation reads the live one");
        assert_eq!(files_of(&base), vec!["-g2-list.bin", "-g2-map.bin", ".state.bin"]);
        drop((u, other));
        cleanup(&base);
    }

    #[test]
    fn unlink_removes_the_state_file_and_every_table_and_nothing_else() {
        let base = scratch("unlink");
        cleanup(&base);
        let neighbour = with_suffix(&base, "-g1-vx-map.bin");
        std::fs::write(&neighbour, b"not a table").expect("a neighbouring file");
        let u = RawUniversal::create(&base, 16, 8, Strategy::Vec).expect("created");
        u.migrate_to(Strategy::Map).expect("migrate");
        u.migrate_to(Strategy::Vec).expect("migrate back");
        let leftover = table_path(&base, 9, Strategy::Map);
        std::fs::write(&leftover, b"a stopped rollover's table").expect("a leftover table");
        drop(u);
        let report = RawUniversal::unlink(&base);
        assert_eq!((report.removed, report.failed), (4, 0), "the state file, both tables and the leftover: {report:?}");
        assert!(neighbour.exists(), "a file that only shares the prefix is left");
        std::fs::remove_file(&neighbour).expect("the neighbouring file is removed");
        assert_eq!(RawUniversal::unlink(&base).removed, 0);
    }

    /// A new set starts with no tables but its own: tables an earlier set
    /// left under the base are removed before anyone can attach them.
    #[test]
    fn a_new_set_removes_the_tables_an_earlier_one_left() {
        let base = scratch("leftovers");
        cleanup(&base);
        std::fs::write(table_path(&base, 0, Strategy::Map), b"an earlier set's table").expect("a leftover");
        std::fs::write(table_path(&base, 3, Strategy::Vec), b"an earlier set's table").expect("a leftover");
        let u = RawUniversal::create(&base, 16, 8, Strategy::Map).expect("created over the leftovers");
        assert!(u.insert(&le(1)).expect("insert"));
        assert_eq!(files_of(&base), vec!["-g0-list.bin", "-g0-map.bin", ".state.bin"]);
        drop(u);
        cleanup(&base);
    }

    /// A set cleared and migrated many times keeps its state file and its
    /// two tables, and nothing more.
    #[test]
    fn a_set_that_moves_many_times_keeps_its_two_tables() {
        let base = scratch("moves");
        cleanup(&base);
        let u = RawUniversal::create(&base, 16, 8, Strategy::Vec).expect("created");
        for round in 0u64..40 {
            u.insert(&le(round)).expect("insert");
            if round % 2 == 0 {
                u.clear().expect("clear");
            } else {
                let to = if u.strategy() == Strategy::Vec { Strategy::Map } else { Strategy::Vec };
                u.migrate_to(to).expect("migrate");
            }
        }
        drop(u);
        let report = RawUniversal::unlink(&base);
        assert_eq!((report.removed, report.failed), (3, 0), "the state file and the two tables: {report:?}");
    }

    /// A writer stopped inside its claim while the set moves twice: the
    /// first move's pass gives its claim up, and it frees the claim, starts
    /// again from the state word and lands its value once.
    #[test]
    fn a_writer_stopped_across_two_moves_lands_its_value_once() {
        let base = scratch("stopped_writer");
        cleanup(&base);
        let a = RawUniversal::create(&base, 32, 8, Strategy::Vec).expect("created");
        let b = Arc::new(RawUniversal::open(&base, 32, 8).expect("opened"));
        a.insert(&le(1)).expect("insert");
        let writer = Arc::clone(&b);
        let (pause, stalled) = crate::test_races::stopped(move || writer.insert(&le(9)));
        a.migrate_to(Strategy::Map).expect("the first move");
        a.migrate_to(Strategy::Vec).expect("the second move");
        pause.release();
        assert!(stalled.join().expect("the writer").expect("the insert"), "the value is new");
        assert_eq!(elements(&a), vec![1, 9]);
        drop(b);
        drop(a);
        cleanup(&base);
    }
}
