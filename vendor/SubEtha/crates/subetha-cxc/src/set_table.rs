//! The table behind the strategy-switching set: fixed-size keys in slots
//! that writers claim and publish by compare-and-swap, so any number of
//! processes add to one table and none of them waits on another.
//!
//! One file holds one table, which the set reuses from one round to the
//! next; a round is the set's version while the table serves it. Every slot
//! word carries the round it was last taken in, its state and a 29-bit
//! fingerprint, and a word of an earlier round is stale: the set was cleared
//! or moved since, so a stale published key is not in the set.
//!
//! The list strategy probes every key from the first slot, reading a packed
//! filter of 16-bit fingerprints four slots at a time; the map strategy
//! probes from the slot the key's hash names. A probe in a round ends at a
//! slot no writer of that round can fill: one never taken, or a stale
//! published or retired key. A writer takes the first slot it may on the
//! probe, a freed one or the one that ends it, so every copy of a key in a
//! round lies on its probe before the slot that ends it. A key re-added in
//! the round after its own takes its stale copy back with one
//! compare-and-swap.
//!
//! Two writers adding one key at once both reach it on its probe, and the
//! copy earliest on the probe is kept: a writer that publishes gives up
//! every claim of its fingerprint it passed before its slot and stands down
//! when it finds an earlier published copy, and one that takes a slot ahead
//! of claims it passed gives those up first. A claim whose writer died is
//! passed the same way. A slot can change hands after a probe passed it, a
//! claim of any key given up and freed and the freed slot taken by another
//! writer of the key, so a writer checks every slot it passed that holds a
//! claim or that a writer may take, not only the claims it saw. A list
//! writer gives back a freed slot it took once the hint has passed it,
//! since a probe reading that hint finds the slot only through its filter.
//!
//! The set seals its state word before it clears or moves, and a writer
//! reads the word after it claims and frees the claim when the word moved.
//! The pass that follows gives up every claim of the sealed round still
//! open. A move's pass also retires every stale published key so none is
//! taken back behind it, and hands each key published in the round to the
//! caller.

use std::fs::File;
use std::io;
use std::mem::{size_of, MaybeUninit};
use std::path::Path;
use std::ptr;
use std::sync::atomic::{fence, AtomicU64, Ordering};

use memmap2::{MmapMut, MmapOptions};

use crate::layout::Mismatch;

/// Record an event about `$table` in the test-build trace, when a test
/// named the table. Outside a test build it is nothing.
macro_rules! note {
    ($table:expr, $what:ident, $slot:expr, $round:expr, $key:expr, $word:expr, $extra:expr) => {
        #[cfg(test)]
        crate::set_trace::record(
            $table.base as usize,
            crate::set_trace::What::$what,
            $slot,
            $round,
            $key,
            $word,
            $extra,
        );
    };
}

const TABLE_MAGIC: u32 = 0x5354_424C;
const TABLE_FORMAT: u32 = 3;

const STATE_MASK: u64 = 0b111;
/// The fingerprint's bits in a slot word, above the state.
const FP_MASK: u64 = 0xFFFF_FFF8;
const EMPTY: u64 = 0;
const CLAIMED: u64 = 1;
const FULL: u64 = 2;
/// A published copy that stood down for an earlier copy of its key, or a
/// stale published key a pass retired.
const TOMB: u64 = 3;
/// A claim another writer or a pass gave up. Its writer may still be
/// filling the key, so nobody takes the slot until that writer frees it.
const SKIPPED: u64 = 4;
/// A slot its last writer is done with: any writer may take it, and a probe
/// goes on past it.
const FREE: u64 = 5;

/// One in each 16-bit lane of a filter word.
const LANES: u64 = 0x0001_0001_0001_0001;
/// The top bit of each 16-bit lane.
const LANE_TOPS: u64 = 0x8000_8000_8000_8000;

/// How a table finds a key's slot.
#[repr(u32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Kind {
    /// Every probe starts at the first slot.
    List = 0,
    /// A probe starts at the slot the key's hash names.
    Map = 1,
}

/// What an insert did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Outcome {
    /// The key was not in the round and now is.
    Added,
    /// The key was already in the round.
    Present,
    /// The round holds its capacity, or its probe has no slot left to take.
    Full,
    /// The set's state word moved under the claim, which was freed: the
    /// round is sealed or over.
    Moved,
}

/// `bytes`, at most eight, as a little-endian word with the bytes past them
/// zero: two overlapping loads of four bytes, or of two, so no copy takes a
/// length known only at run time.
#[inline(always)]
fn small_word(bytes: &[u8]) -> u64 {
    let n = bytes.len();
    debug_assert!(n <= 8);
    if let (Some(lo), Some(hi)) = (bytes.first_chunk::<4>(), bytes.last_chunk::<4>()) {
        // Bytes 0..4 and n-4..n cover every byte, the overlap the same in both.
        u64::from(u32::from_le_bytes(*lo)) | (u64::from(u32::from_le_bytes(*hi)) << (8 * (n - 4)))
    } else if let (Some(lo), Some(hi)) = (bytes.first_chunk::<2>(), bytes.last_chunk::<2>()) {
        u64::from(u16::from_le_bytes(*lo)) | (u64::from(u16::from_le_bytes(*hi)) << (8 * (n - 2)))
    } else {
        bytes.first().map_or(0, |&b| u64::from(b))
    }
}

/// Write the low `out.len()` bytes of the little-endian `word`, at most
/// eight, into `out`: two overlapping stores of four bytes, or of two.
#[inline(always)]
fn put_small(out: &mut [u8], word: u64) {
    let n = out.len();
    debug_assert!(n <= 8);
    if n >= 4 {
        out[..4].copy_from_slice(&(word as u32).to_le_bytes());
        out[n - 4..].copy_from_slice(&((word >> (8 * (n - 4))) as u32).to_le_bytes());
    } else if n >= 2 {
        out[..2].copy_from_slice(&(word as u16).to_le_bytes());
        out[n - 2..].copy_from_slice(&((word >> (8 * (n - 2))) as u16).to_le_bytes());
    } else if let Some(b) = out.first_mut() {
        *b = word as u8;
    }
}

/// The hash every probe and fingerprint comes from: the key read eight bytes
/// at a time, little-endian with the last word zero-filled, each word folded
/// in with a multiply and a rotate, then the length and MurmurHash3's
/// finalizer, so every bit a probe or a fingerprint uses depends on every
/// byte.
#[inline]
pub(crate) fn hash_key(key: &[u8]) -> u64 {
    // An eight-byte key is one word, folded once.
    let h = match key.first_chunk::<8>() {
        Some(word) if key.len() == 8 => fold(0, u64::from_le_bytes(*word)),
        _ => {
            let mut h = 0u64;
            let (words, rest) = key.as_chunks::<8>();
            for word in words {
                h = fold(h, u64::from_le_bytes(*word));
            }
            if !rest.is_empty() {
                h = fold(h, small_word(rest));
            }
            h
        }
    };
    finish(h, key.len())
}

/// One word folded into the hash with a multiply and a rotate.
#[inline(always)]
fn fold(h: u64, word: u64) -> u64 {
    const FOLD: u64 = 0x9E37_79B9_7F4A_7C15;
    (h ^ word).wrapping_mul(FOLD).rotate_left(31)
}

/// The folded words of a key of `len` bytes, the length mixed in, through
/// MurmurHash3's finalizer.
#[inline(always)]
fn finish(mut h: u64, len: usize) -> u64 {
    h ^= len as u64;
    h ^= h >> 33;
    h = h.wrapping_mul(0xff51_afd7_ed55_8ccd);
    h ^= h >> 33;
    h = h.wrapping_mul(0xc4ce_b9fe_1a85_ec53);
    h ^ (h >> 33)
}

/// The hash a set table probes by: [`hash_key`]'s high half, which the
/// fingerprint comes from, and a low half the probe starts from. For a key
/// of one to eight bytes the low half is the top half of its word times the
/// golden ratio, so consecutive keys start at distinct slots; a longer key
/// keeps [`hash_key`]'s low half.
#[inline]
pub(crate) fn table_hash(key: &[u8]) -> u64 {
    if (1..=8).contains(&key.len()) { Word::new(key).hash() } else { hash_key(key) }
}

/// [`table_hash`] of a key of `len` bytes, one to eight, whose word is
/// `bits`. The fold's product is the start's, so it costs no second
/// multiply.
#[inline(always)]
fn table_word_hash(bits: u64, len: usize) -> u64 {
    const GOLDEN: u64 = 0x9E37_79B9_7F4A_7C15;
    let start = bits.wrapping_mul(GOLDEN) >> 32;
    (finish(fold(0, bits), len) & !0xFFFF_FFFF) | start
}

/// Whether the key at `at`, a slot's key, is `key`: compared a word at a
/// time, each word read whole, the last masked to the key's length.
///
/// # Safety
/// `at` is eight-byte aligned and addresses `key.len()` rounded up to a
/// multiple of eight readable bytes.
#[inline]
pub(crate) unsafe fn same_key(at: *const u8, key: &[u8]) -> bool {
    let words = at as *const u64;
    if let Some(given) = key.first_chunk::<8>()
        && key.len() == 8
    {
        // SAFETY: the slot's key is one aligned word, by the caller.
        return unsafe { crate::atomic_copy::load_word(words) } == u64::from_ne_bytes(*given);
    }
    let full = key.len() / 8;
    for i in 0..full {
        // SAFETY: word `i` lies within the slot's key by the caller, and within
        // `key` because `i` is below `full`.
        let (stored, given) = unsafe { (crate::atomic_copy::load_word(words.add(i)), ptr::read_unaligned(key.as_ptr().add(i * 8) as *const u64)) };
        if stored != given {
            return false;
        }
    }
    let tail = key.len() % 8;
    if tail == 0 {
        return true;
    }
    // SAFETY: the slot rounds its key up to a whole word, by the caller.
    let stored = unsafe { crate::atomic_copy::load_word(words.add(full)) }.to_ne_bytes();
    let mask = u64::MAX >> (64 - 8 * tail);
    u64::from_le_bytes(stored) & mask == small_word(&key[full * 8..])
}

/// Write `key` into a slot's key a word at a time, each word written whole,
/// the last zero-filled past the key's length.
///
/// # Safety
/// `to` is eight-byte aligned and addresses `key.len()` rounded up to a
/// multiple of eight writable bytes that nobody else writes meanwhile.
#[inline]
pub(crate) unsafe fn put_key(to: *mut u8, key: &[u8]) {
    let words = to as *mut u64;
    let full = key.len() / 8;
    for i in 0..full {
        // SAFETY: as for `same_key`.
        unsafe { crate::atomic_copy::store_word(words.add(i), ptr::read_unaligned(key.as_ptr().add(i * 8) as *const u64)) };
    }
    if !key.len().is_multiple_of(8) {
        let last = small_word(&key[full * 8..]).to_le_bytes();
        // SAFETY: the slot rounds its key up to a whole word, by the caller.
        unsafe { crate::atomic_copy::store_word(words.add(full), u64::from_ne_bytes(last)) };
    }
}

/// Read a slot's key into `out` a word at a time, each word read whole.
///
/// # Safety
/// `from` is eight-byte aligned and addresses `out.len()` rounded up to a
/// multiple of eight readable bytes.
#[inline]
pub(crate) unsafe fn take_key(from: *const u8, out: &mut [u8]) {
    let words = from as *const u64;
    let full = out.len() / 8;
    for i in 0..full {
        // SAFETY: as for `same_key`, with `out` in place of the key.
        unsafe { ptr::write_unaligned(out.as_mut_ptr().add(i * 8) as *mut u64, crate::atomic_copy::load_word(words.add(i))) };
    }
    if !out.len().is_multiple_of(8) {
        // SAFETY: the slot rounds its key up to a whole word, by the caller.
        let last = unsafe { crate::atomic_copy::load_word(words.add(full)) }.to_ne_bytes();
        put_small(&mut out[full * 8..], u64::from_le_bytes(last));
    }
}

/// A key as a table hashes, compares and stores it. Every form hashes,
/// compares and stores a given key's bytes exactly as the byte functions
/// do, so a table holds the same bytes whichever form wrote them.
pub(crate) trait Key: Copy {
    /// The key's length in bytes.
    fn len(self) -> usize;

    /// [`table_hash`] of the key's bytes.
    fn hash(self) -> u64;

    /// [`hash_key`] of the key's bytes.
    fn key_hash(self) -> u64;

    /// [`same_key`] of the key's bytes.
    ///
    /// # Safety
    /// As for [`same_key`].
    unsafe fn same(self, at: *const u8) -> bool;

    /// [`put_key`] of the key's bytes.
    ///
    /// # Safety
    /// As for [`put_key`].
    unsafe fn put(self, to: *mut u8);

    /// The test-build trace's word for the key.
    #[cfg(test)]
    fn trace(self) -> u64;
}

/// A key of any length, as its bytes.
#[derive(Clone, Copy)]
pub(crate) struct Bytes<'a>(pub(crate) &'a [u8]);

impl Key for Bytes<'_> {
    #[inline(always)]
    fn len(self) -> usize {
        self.0.len()
    }

    #[inline(always)]
    fn hash(self) -> u64 {
        table_hash(self.0)
    }

    #[inline(always)]
    fn key_hash(self) -> u64 {
        hash_key(self.0)
    }

    #[inline(always)]
    unsafe fn same(self, at: *const u8) -> bool {
        // SAFETY: the caller's contract.
        unsafe { same_key(at, self.0) }
    }

    #[inline(always)]
    unsafe fn put(self, to: *mut u8) {
        // SAFETY: the caller's contract.
        unsafe { put_key(to, self.0) }
    }

    #[cfg(test)]
    fn trace(self) -> u64 {
        crate::set_trace::key_word(self.0)
    }
}

/// A key of one to eight bytes as the one word its slot holds: the bytes
/// little-endian, zero past them.
#[derive(Clone, Copy)]
pub(crate) struct Word {
    bits: u64,
    len: usize,
}

impl Word {
    /// The key whose bytes are `key`, one to eight of them.
    #[inline(always)]
    pub(crate) fn new(key: &[u8]) -> Self {
        debug_assert!((1..=8).contains(&key.len()));
        let bits = match key.first_chunk::<8>() {
            Some(word) => u64::from_le_bytes(*word),
            None => small_word(key),
        };
        Self { bits, len: key.len() }
    }
}

impl Key for Word {
    #[inline(always)]
    fn len(self) -> usize {
        self.len
    }

    #[inline(always)]
    fn hash(self) -> u64 {
        table_word_hash(self.bits, self.len)
    }

    #[inline(always)]
    fn key_hash(self) -> u64 {
        finish(fold(0, self.bits), self.len)
    }

    #[inline(always)]
    unsafe fn same(self, at: *const u8) -> bool {
        // SAFETY: the slot's key is one aligned word, by the caller.
        let stored = u64::from_le(unsafe { crate::atomic_copy::load_word(at as *const u64) });
        stored & (u64::MAX >> (64 - 8 * self.len)) == self.bits
    }

    #[inline(always)]
    unsafe fn put(self, to: *mut u8) {
        // SAFETY: the slot's key is one aligned word nobody else writes
        // meanwhile, by the caller.
        unsafe { crate::atomic_copy::store_word(to as *mut u64, self.bits.to_le()) }
    }

    #[cfg(test)]
    fn trace(self) -> u64 {
        self.bits
    }
}

/// The fingerprint a slot word carries, in the bits above its state.
fn fingerprint(h: u64) -> u64 {
    (h >> 32) & FP_MASK
}

/// A slot word: the round in the high half, the fingerprint and the state in
/// the low.
fn slot_word(round: u32, fp: u64, state: u64) -> u64 {
    (u64::from(round) << 32) | fp | state
}

/// The round in the high half of a slot word or a header word.
fn round_of(w: u64) -> u32 {
    (w >> 32) as u32
}

/// A key's filter lane: never zero, which marks a lane not yet written.
fn lane_of(h: u64) -> u16 {
    match (h >> 16) as u16 {
        0 => 1,
        v => v,
    }
}

/// The top bit of each lane of `x` that is zero, and possibly of a lane
/// just above one that is: a candidate the caller checks, never a miss.
fn zero_lanes(x: u64) -> u64 {
    x.wrapping_sub(LANES) & !x & LANE_TOPS
}

/// Which of its group of four a slot is, for the lane of a filter word
/// counted from the low end; the same mapping both ways.
fn lane_slot(lane: usize) -> usize {
    if cfg!(target_endian = "little") { lane } else { 3 - lane }
}

/// What a slot is to a probe of one round.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Seen {
    /// Never taken, or a stale published or retired key: the probe ends
    /// here, and a writer may take it.
    End,
    /// Freed: a writer may take it, and the probe goes on.
    Free,
    /// A claim of the round, whose writer may still publish it.
    Claim,
    /// A key published in the round.
    Key,
    /// Anything else, which the probe passes.
    Pass,
}

fn seen(w: u64, round: u32) -> Seen {
    let state = w & STATE_MASK;
    if state == EMPTY {
        return Seen::End;
    }
    // A later round's word belongs to a round this probe's round ended
    // before; nothing in it is this probe's to take or to read.
    if round_of(w) > round {
        return Seen::Pass;
    }
    let current = round_of(w) == round;
    match state {
        FREE => Seen::Free,
        FULL if current => Seen::Key,
        CLAIMED if current => Seen::Claim,
        FULL | TOMB if !current => Seen::End,
        _ => Seen::Pass,
    }
}

/// Slots a writer's probe passed that hold a claim, or could still be taken
/// by another writer of its key: claims of the round, freed slots, and
/// claims given up or of an earlier round, which their writers free. The
/// first few inline, written only as they are pushed, the rest on the heap.
struct Watch {
    first: [MaybeUninit<usize>; 4],
    len: usize,
    more: Vec<usize>,
}

impl Watch {
    fn new() -> Self {
        Self { first: [MaybeUninit::uninit(); 4], len: 0, more: Vec::new() }
    }

    fn push(&mut self, slot: usize) {
        if self.len < self.first.len() {
            self.first[self.len].write(slot);
            self.len += 1;
        } else {
            self.more.push(slot);
        }
    }

    fn clear(&mut self) {
        self.len = 0;
        self.more.clear();
    }

    /// No slot pushed since the last clear.
    fn is_empty(&self) -> bool {
        self.len == 0
    }

    fn slots(&self) -> impl Iterator<Item = usize> + '_ {
        // SAFETY: the first `len` entries were written by `push`.
        self.first[..self.len].iter().map(|slot| unsafe { slot.assume_init() }).chain(self.more.iter().copied())
    }
}

/// The first two cache lines of a table file.
#[repr(C, align(64))]
struct TableHeader {
    magic: u32,
    format: u32,
    kind: u32,
    key_size: u32,
    slot_count: u64,
    capacity: u64,
    _line0: [u8; 32],
    /// The round the table serves in the high half, and the keys added in
    /// that round in the low half.
    count: AtomicU64,
    /// The round in the high half and, in the low half, one past the last
    /// slot a list writer claimed in that round.
    hint: AtomicU64,
    _line1: [u8; 48],
}

const _: () = assert!(size_of::<TableHeader>() == 128);

/// Where everything in a table file lies, for one kind, capacity and key
/// size.
#[derive(Debug, Clone, Copy)]
struct Shape {
    kind: Kind,
    key_size: usize,
    key_stride: usize,
    capacity: usize,
    slot_count: usize,
    filter_at: usize,
    words_at: usize,
    keys_at: usize,
    total: usize,
}

impl Shape {
    /// Twice the capacity in slots, at least 16 and a power of two, so a
    /// map probe stays short and a list has room for given-up slots. A
    /// round's count is 32 bits, which bounds the capacity.
    fn new(kind: Kind, capacity: usize, key_size: usize) -> io::Result<Self> {
        let invalid = |what: &str| io::Error::new(io::ErrorKind::InvalidInput, what.to_string());
        if capacity == 0 {
            return Err(invalid("a set holds at least one key"));
        }
        if u32::try_from(capacity).is_err() {
            return Err(invalid("a set holds at most 4294967295 keys"));
        }
        if key_size == 0 || u32::try_from(key_size).is_err() {
            return Err(invalid("a key is between one byte and four gigabytes"));
        }
        let too_large = || invalid("the table for this capacity and key size does not fit in memory");
        let slot_count = capacity
            .checked_mul(2)
            .and_then(|n| n.max(16).checked_next_power_of_two())
            .ok_or_else(too_large)?;
        let key_stride = key_size.checked_next_multiple_of(8).ok_or_else(too_large)?;
        let filter_at = size_of::<TableHeader>();
        let filter_bytes = slot_count.checked_mul(2).and_then(|n| n.checked_next_multiple_of(64)).ok_or_else(too_large)?;
        let words_at = filter_at.checked_add(filter_bytes).ok_or_else(too_large)?;
        let word_bytes = slot_count.checked_mul(8).and_then(|n| n.checked_next_multiple_of(64)).ok_or_else(too_large)?;
        let keys_at = words_at.checked_add(word_bytes).ok_or_else(too_large)?;
        let total = slot_count
            .checked_mul(key_stride)
            .and_then(|n| n.checked_add(keys_at))
            .ok_or_else(too_large)?;
        Ok(Self { kind, key_size, key_stride, capacity, slot_count, filter_at, words_at, keys_at, total })
    }

    /// Lay a fresh header out: every field first, the magic last, because
    /// an attacher spins on it. The zeroed counts are round 0's.
    ///
    /// # Safety
    /// `ptr` addresses at least `self.total` writable zeroed bytes.
    unsafe fn init(&self, ptr: *mut u8) {
        let hdr = ptr as *mut TableHeader;
        unsafe {
            (*hdr).format = TABLE_FORMAT;
            (*hdr).kind = self.kind as u32;
            (*hdr).key_size = self.key_size as u32;
            (*hdr).slot_count = self.slot_count as u64;
            (*hdr).capacity = self.capacity as u64;
            ptr::write_volatile(&raw mut (*hdr).magic, TABLE_MAGIC);
        }
    }

    /// The first header field of the table at `base` that differs from this
    /// shape.
    fn check(&self, base: *const u8) -> io::Result<()> {
        // SAFETY: the caller mapped at least a header's length at `base`.
        let hdr = unsafe { &*(base as *const TableHeader) };
        Mismatch::first(&[
            ("magic", u64::from(hdr.magic), u64::from(TABLE_MAGIC)),
            ("format", u64::from(hdr.format), u64::from(TABLE_FORMAT)),
            ("strategy", u64::from(hdr.kind), self.kind as u64),
            ("element size", u64::from(hdr.key_size), self.key_size as u64),
            ("capacity", hdr.capacity, self.capacity as u64),
            ("slots", hdr.slot_count, self.slot_count as u64),
        ])
        .map_err(io::Error::from)
    }
}

/// What a writer's probe found.
struct Probe {
    /// A copy of the key published in the round.
    present: bool,
    /// The slot a writer takes and the word seen in it: the first freed
    /// slot, else the one that ended the probe.
    take: Option<(usize, u64)>,
    /// The slot that ended the probe, when it is a stale copy of the key,
    /// which the writer takes back instead.
    revive: Option<(usize, u64)>,
    /// The first claim of the round a list's probe passed past the hint.
    /// Its writer may not have written its filter lane yet, so a hint this
    /// writer raises stops there.
    claim: Option<usize>,
}

impl Probe {
    const PRESENT: Self = Self { present: true, take: None, revive: None, claim: None };
}

/// One set table, mapped.
pub(crate) struct SetTable {
    _file: File,
    mmap: MmapMut,
    base: *mut u8,
    shape: Shape,
    mask: usize,
}

// SAFETY: every shared access goes through the atomic slot words, filter
// words and header counts, and a key's bytes are written only by the writer
// holding its slot's claim, before the release that publishes them.
unsafe impl Send for SetTable {}
// SAFETY: as for Send.
unsafe impl Sync for SetTable {}

impl SetTable {
    fn from_parts(file: File, mut mmap: MmapMut, shape: Shape) -> Self {
        let base = mmap.as_mut_ptr();
        Self { _file: file, mmap, base, shape, mask: shape.slot_count - 1 }
    }

    /// Obtain the table at `path`: an empty one is laid out when the file
    /// does not exist, and an existing one is attached with its keys in
    /// place.
    pub(crate) fn create_or_attach(path: &Path, kind: Kind, capacity: usize, key_size: usize) -> io::Result<Self> {
        let shape = Shape::new(kind, capacity, key_size)?;
        let (file, mmap) = crate::mmf_attach::create_or_attach(
            path,
            shape.total,
            // SAFETY: the builder hands `init` a zeroed mapping of `shape.total` bytes.
            |p| unsafe { shape.init(p) },
            // SAFETY: an attacher hands the reader a mapping of at least a header.
            |p| unsafe { u64::from((*(p as *const TableHeader)).magic) },
            u64::from(TABLE_MAGIC),
        )?;
        shape.check(mmap.as_ptr())?;
        Ok(Self::from_parts(file, mmap, shape))
    }

    /// Attach to the table at `path`, which must exist.
    pub(crate) fn open(path: &Path, kind: Kind, capacity: usize, key_size: usize) -> io::Result<Self> {
        let shape = Shape::new(kind, capacity, key_size)?;
        let file = crate::region_file::open_existing(path)?;
        let len = file.metadata()?.len();
        if len < shape.total as u64 {
            return Err(Mismatch::new("length", len, shape.total as u64).into());
        }
        // SAFETY: the file is at least `shape.total` bytes, checked above.
        let mmap = unsafe { MmapOptions::new().len(shape.total).map_mut(&file)? };
        shape.check(mmap.as_ptr())?;
        Ok(Self::from_parts(file, mmap, shape))
    }

    fn header(&self) -> &TableHeader {
        // SAFETY: the mapping starts with a header.
        unsafe { &*(self.base as *const TableHeader) }
    }

    fn word(&self, slot: usize) -> &AtomicU64 {
        debug_assert!(slot < self.shape.slot_count);
        // SAFETY: the words region holds `slot_count` aligned words.
        unsafe { &*(self.base.add(self.shape.words_at + slot * 8) as *const AtomicU64) }
    }

    /// The filter word holding the lanes of slots `4 * quad` to `4 * quad + 3`.
    fn quad(&self, quad: usize) -> &AtomicU64 {
        debug_assert!(quad * 4 < self.shape.slot_count);
        // SAFETY: the filter region holds `slot_count / 4` aligned words.
        unsafe { &*(self.base.add(self.shape.filter_at + quad * 8) as *const AtomicU64) }
    }

    fn key(&self, slot: usize) -> *mut u8 {
        debug_assert!(slot < self.shape.slot_count);
        // SAFETY: the keys region holds `slot_count` keys of `key_stride` bytes.
        unsafe { self.base.add(self.shape.keys_at + slot * self.shape.key_stride) }
    }

    /// Whether the slot whose word was `w` holds `key`: `None` when the word
    /// moved during the comparison, which a writer replacing the key does
    /// before it writes a byte, so a `Some` never answers from a torn key.
    fn holds<K: Key>(&self, slot: usize, w: u64, key: K) -> Option<bool> {
        // SAFETY: the keys region holds `key_stride` bytes, eight-byte aligned,
        // at every slot, and a key a writer is replacing reads as some bytes,
        // which the word checked next discards.
        let same = unsafe { key.same(self.key(slot)) };
        fence(Ordering::Acquire);
        (self.word(slot).load(Ordering::Relaxed) == w).then_some(same)
    }

    /// Keys added in `round`, or `None` when the table serves another round.
    pub(crate) fn len(&self, round: u32) -> Option<usize> {
        let count = self.header().count.load(Ordering::Acquire);
        (round_of(count) == round).then_some((count & 0xFFFF_FFFF) as usize)
    }

    /// Make the table serve `round` from empty, unless it already serves
    /// that round or a later one. Any number of processes may call it at once.
    pub(crate) fn begin(&self, round: u32) {
        for word in [&self.header().count, &self.header().hint] {
            let mut now = word.load(Ordering::Acquire);
            while round_of(now) < round {
                match word.compare_exchange_weak(now, u64::from(round) << 32, Ordering::AcqRel, Ordering::Acquire) {
                    Ok(_) => break,
                    Err(seen) => now = seen,
                }
            }
        }
    }

    /// Count a key added in `round`, unless the table serves a later one,
    /// where the key was carried or cleared already.
    fn count_added(&self, round: u32) {
        let count = &self.header().count;
        let mut now = count.load(Ordering::Relaxed);
        while round_of(now) == round {
            match count.compare_exchange_weak(now, now + 1, Ordering::AcqRel, Ordering::Relaxed) {
                Ok(_) => return,
                Err(seen) => now = seen,
            }
        }
    }

    /// Set the count of `round`, which a move's carriers fill, to the `n`
    /// keys the move carried, if nothing has counted in the round yet;
    /// whether this call set it. Every carrier's pass carries the same keys,
    /// so whichever sets it first sets what the others would have; a writer
    /// counts in the round only after the move made it live, so a carrier
    /// that comes to it late changes nothing.
    pub(crate) fn set_count(&self, round: u32, n: usize) -> bool {
        let empty = u64::from(round) << 32;
        // The count's half holds below 2^32 keys, as `Shape::new` holds the
        // capacity.
        let carried = empty | (n as u64 & 0xFFFF_FFFF);
        note!(self, Counted, usize::MAX, round, 0, carried, 2);
        self.header().count.compare_exchange(empty, carried, Ordering::AcqRel, Ordering::Acquire).is_ok()
    }

    /// Push the table's dirty pages to disk, returning when they are durable.
    pub(crate) fn flush(&self) -> io::Result<()> {
        self.mmap.flush()
    }

    /// Where the probe of a key of hash `h` starts: a list's first slot, or
    /// in a map the slot the top bits of the hash's low half name. `LIST` is
    /// the table's kind, which the hot paths are compiled for once each.
    #[inline(always)]
    fn start_in<const LIST: bool>(&self, h: u64) -> usize {
        if LIST { 0 } else { (((h & 0xFFFF_FFFF) * (self.mask as u64 + 1)) >> 32) as usize }
    }

    /// The slot a probe visits after `slot`, for a table of kind `LIST`.
    #[inline(always)]
    fn after_in<const LIST: bool>(&self, slot: usize) -> usize {
        if LIST { slot + 1 } else { (slot + 1) & self.mask }
    }

    /// How far along the probe of a key of hash `h` `slot` lies, for a table
    /// of kind `LIST`.
    #[inline(always)]
    fn position_in<const LIST: bool>(&self, h: u64, slot: usize) -> usize {
        slot.wrapping_sub(self.start_in::<LIST>(h)) & self.mask
    }

    fn start(&self, h: u64) -> usize {
        if self.is_list() { self.start_in::<true>(h) } else { self.start_in::<false>(h) }
    }

    fn after(&self, slot: usize) -> usize {
        if self.is_list() { self.after_in::<true>(slot) } else { self.after_in::<false>(slot) }
    }

    /// Where a list's probe of `round` leaves the filter: every slot below it
    /// was taken in the round or is passed by every probe.
    fn hint(&self, round: u32) -> usize {
        let hint = self.header().hint.load(Ordering::Acquire);
        if round_of(hint) == round { ((hint & 0xFFFF_FFFF) as usize).min(self.shape.slot_count) } else { 0 }
    }

    /// Where a writer that took `slot` raises the hint to: past the slot, or
    /// only up to `claim`, a claim of the round its probe passed first,
    /// whose writer may not have written its filter lane yet, so every slot
    /// below the hint keeps a lane that finds what it holds.
    fn hint_past(slot: usize, claim: Option<usize>) -> usize {
        match claim {
            Some(c) if c < slot => c,
            _ => slot + 1,
        }
    }

    /// Move the hint of `round` past `slot`. A hint past the last slot a
    /// 32-bit half holds stops there, and the probe's walk covers the rest.
    /// In one order with every claim, as [`hint_seq`](Self::hint_seq) is.
    fn raise_hint(&self, round: u32, past: usize) {
        note!(self, HintRaised, past, round, 0, past as u64, 0);
        let past = past.min(u32::MAX as usize) as u64;
        self.header().hint.fetch_max((u64::from(round) << 32) | past, Ordering::SeqCst);
    }

    /// [`hint`](Self::hint), read in one order with every claim and raise, so
    /// a writer that claimed a freed slot sees any raise past it that a
    /// writer probing beside it read.
    fn hint_seq(&self, round: u32) -> usize {
        let hint = self.header().hint.load(Ordering::SeqCst);
        if round_of(hint) == round { ((hint & 0xFFFF_FFFF) as usize).min(self.shape.slot_count) } else { 0 }
    }

    /// Write `slot`'s filter lane, which may hold an earlier key's.
    fn write_lane(&self, slot: usize, lane: u16) {
        let shift = (lane_slot(slot % 4) * 16) as u32;
        let mask = 0xFFFF_u64 << shift;
        let want = u64::from(lane) << shift;
        let quad = self.quad(slot / 4);
        let mut now = quad.load(Ordering::Relaxed);
        while now & mask != want {
            match quad.compare_exchange_weak(now, (now & !mask) | want, Ordering::Release, Ordering::Relaxed) {
                Ok(_) => return,
                Err(seen) => now = seen,
            }
        }
    }

    /// Calls `f` with each slot in `from..to` whose filter lane is `lane` or
    /// not yet written, and stops at the first that answers true; whether
    /// one did.
    fn visit_lanes(&self, from: usize, to: usize, lane: u16, mut f: impl FnMut(usize) -> bool) -> bool {
        let pattern = u64::from(lane).wrapping_mul(LANES);
        let mut quad = from / 4;
        while quad * 4 < to {
            let lanes = self.quad(quad).load(Ordering::Acquire);
            let mut hits = zero_lanes(lanes ^ pattern) | zero_lanes(lanes);
            while hits != 0 {
                let slot = quad * 4 + lane_slot((hits.trailing_zeros() / 16) as usize);
                hits &= hits - 1;
                if slot >= from && slot < to && f(slot) {
                    return true;
                }
            }
            quad += 1;
        }
        false
    }

    /// One past the last slot a key or claim of `round` can occupy: every
    /// slot of a map, and for a list the slot that ends its walk past the
    /// hint.
    fn reach(&self, round: u32) -> usize {
        if self.shape.kind == Kind::Map {
            return self.shape.slot_count;
        }
        let mut slot = self.hint(round);
        while slot < self.shape.slot_count {
            if seen(self.word(slot).load(Ordering::Acquire), round) == Seen::End {
                return slot + 1;
            }
            slot += 1;
        }
        self.shape.slot_count
    }

    /// Makes sure no copy of the key is published at `slot` in `round` from
    /// now on: a claim of its fingerprint there is given up, and a published
    /// copy of the key answers true.
    fn give_up<K: Key>(&self, slot: usize, round: u32, fp: u64, key: K) -> bool {
        loop {
            let w = self.word(slot).load(Ordering::SeqCst);
            if w & FP_MASK != fp || round_of(w) != round {
                return false;
            }
            match w & STATE_MASK {
                CLAIMED => {
                    let skipped = slot_word(round, fp, SKIPPED);
                    if self.word(slot).compare_exchange(w, skipped, Ordering::AcqRel, Ordering::Relaxed).is_ok() {
                        note!(self, GaveUp, slot, round, key.trace(), w, 0);
                        return false;
                    }
                }
                FULL => {
                    if let Some(same) = self.holds(slot, w, key) {
                        note!(self, FoundCopy, slot, round, key.trace(), w, u64::from(same));
                        return same;
                    }
                }
                _ => return false,
            }
        }
    }

    /// Free a claim this writer made in `round`, whether it still holds it
    /// or another writer or a pass gave it up. A list's slot first has its
    /// filter lane zeroed, which every probe visits, so a writer that takes
    /// the freed slot is found before it writes its own lane.
    fn free_own(&self, slot: usize, round: u32, fp: u64) {
        note!(self, Freed, slot, round, 0, fp, 0);
        if self.shape.kind == Kind::List {
            let shift = (lane_slot(slot % 4) * 16) as u32;
            self.quad(slot / 4).fetch_and(!(0xFFFF_u64 << shift), Ordering::Release);
        }
        let free = slot_word(round, fp, FREE);
        let freed = [CLAIMED, SKIPPED]
            .into_iter()
            .any(|held| self.word(slot).compare_exchange(slot_word(round, fp, held), free, Ordering::AcqRel, Ordering::Relaxed).is_ok());
        debug_assert!(freed, "only its own writer moves a claim that was given up");
    }

    /// A copy this writer published stands down for an earlier one. A copy a
    /// later round already retired or took needs nothing more.
    fn stand_down(&self, slot: usize, round: u32, fp: u64) {
        note!(self, StoodDown, slot, round, 0, fp, 0);
        let full = slot_word(round, fp, FULL);
        if let Err(now) = self.word(slot).compare_exchange(full, slot_word(round, fp, TOMB), Ordering::AcqRel, Ordering::Relaxed) {
            debug_assert!(round_of(now) != round || now & STATE_MASK == TOMB, "only a later round moves a published copy");
        }
    }

    /// Probe `round` for the key, recording in `watch` each slot passed on
    /// the way that [`Watch`] keeps; below a list's hint, which the filter
    /// reads, the claims of the key's fingerprint. `LIST` is the table's
    /// kind.
    #[inline(always)]
    fn probe_in<const LIST: bool, K: Key>(&self, round: u32, h: u64, key: K, watch: &mut Watch) -> Probe {
        let fp = fingerprint(h);
        'again: loop {
            watch.clear();
            let mut slot = self.start_in::<LIST>(h);
            let mut probes = 0;
            if LIST {
                let hint = self.hint(round);
                let mut found = Some(false);
                self.visit_lanes(0, hint, lane_of(h), |j| {
                    let w = self.word(j).load(Ordering::Acquire);
                    if w & FP_MASK != fp {
                        return false;
                    }
                    match seen(w, round) {
                        Seen::Key => {
                            found = self.holds(j, w, key);
                            found != Some(false)
                        }
                        Seen::Claim => {
                            watch.push(j);
                            false
                        }
                        _ => false,
                    }
                });
                match found {
                    Some(true) => return Probe::PRESENT,
                    Some(false) => {}
                    None => continue 'again,
                }
                slot = hint;
                probes = hint;
            }
            let mut take = None;
            let mut claim = None;
            while probes < self.shape.slot_count {
                let w = self.word(slot).load(Ordering::Acquire);
                match seen(w, round) {
                    Seen::End => {
                        if w & STATE_MASK == FULL && w & FP_MASK == fp {
                            match self.holds(slot, w, key) {
                                Some(true) => return Probe { present: false, take, revive: Some((slot, w)), claim },
                                Some(false) => {}
                                None => continue 'again,
                            }
                        }
                        return Probe { present: false, take: take.or(Some((slot, w))), revive: None, claim };
                    }
                    Seen::Free => {
                        watch.push(slot);
                        take = take.or(Some((slot, w)));
                    }
                    Seen::Key if w & FP_MASK == fp => match self.holds(slot, w, key) {
                        Some(true) => return Probe::PRESENT,
                        Some(false) => {}
                        None => continue 'again,
                    },
                    Seen::Claim => {
                        claim = claim.or(Some(slot));
                        watch.push(slot);
                    }
                    // A claim of an earlier round, or one given up, may be
                    // freed and taken after this probe has passed it.
                    Seen::Pass if matches!(w & STATE_MASK, CLAIMED | SKIPPED) => watch.push(slot),
                    _ => {}
                }
                slot = self.after_in::<LIST>(slot);
                probes += 1;
            }
            return Probe { present: false, take, revive: None, claim };
        }
    }

    /// Add the key to `round`, `key` its bytes and `h` their [`hash_key`].
    /// A new key is held to the table's capacity and counted. A move's copy,
    /// `carried`, is neither: it carries a key the set already holds, and
    /// the move sets the round's count once with
    /// [`set_count`](Self::set_count). `current` answers whether the set's
    /// state word is still the one this round was read from.
    pub(crate) fn insert(&self, round: u32, h: u64, key: &[u8], carried: bool, current: &impl Fn() -> bool) -> Outcome {
        self.insert_key(round, h, Bytes(key), carried, current)
    }

    /// [`insert`](Self::insert) of a key in any [`Key`] form, `h` its hash.
    pub(crate) fn insert_key<K: Key>(&self, round: u32, h: u64, key: K, carried: bool, current: &impl Fn() -> bool) -> Outcome {
        if self.is_list() {
            self.insert_in::<true, K>(round, h, key, carried, current)
        } else {
            self.insert_in::<false, K>(round, h, key, carried, current)
        }
    }

    /// [`insert_key`](Self::insert_key) into a table of kind `LIST`.
    #[inline]
    fn insert_in<const LIST: bool, K: Key>(&self, round: u32, h: u64, key: K, carried: bool, current: &impl Fn() -> bool) -> Outcome {
        debug_assert_eq!(key.len(), self.shape.key_size);
        let fp = fingerprint(h);
        let mut watch = Watch::new();
        loop {
            let probe = self.probe_in::<LIST, K>(round, h, key, &mut watch);
            note!(
                self,
                Probed,
                probe.revive.or(probe.take).map_or(usize::MAX, |(s, _)| s),
                round,
                key.trace(),
                probe.claim.map_or(u64::MAX, |c| c as u64),
                u64::from(probe.present)
            );
            if probe.present {
                note!(self, Present, usize::MAX, round, key.trace(), 0, 0);
                return Outcome::Present;
            }
            let Some((slot, seen)) = probe.revive.or(probe.take) else {
                return Outcome::Full;
            };
            if !carried && self.len(round).is_some_and(|n| n >= self.shape.capacity) {
                return Outcome::Full;
            }
            let at = self.position_in::<LIST>(h, slot);
            // A claim of the fingerprint passed beyond the slot taken would
            // land after this copy, so it is given up first.
            if !watch.is_empty() && watch.slots().filter(|&j| self.position_in::<LIST>(h, j) > at).any(|j| self.give_up(j, round, fp, key)) {
                note!(self, Present, slot, round, key.trace(), 0, 1);
                return Outcome::Present;
            }
            if probe.revive.is_some() {
                // The stale copy is this key. Taking it back publishes it, and a
                // pass that reaches it later carries it, since a pass retires
                // every stale copy it passes.
                if self.word(slot).compare_exchange(seen, slot_word(round, fp, FULL), Ordering::SeqCst, Ordering::Relaxed).is_err() {
                    note!(self, ReviveLost, slot, round, key.trace(), seen, 0);
                    continue;
                }
                note!(self, Revived, slot, round, key.trace(), seen, 0);
                if LIST {
                    self.raise_hint(round, Self::hint_past(slot, probe.claim));
                }
            } else {
                if self.word(slot).compare_exchange(seen, slot_word(round, fp, CLAIMED), Ordering::SeqCst, Ordering::Relaxed).is_err() {
                    note!(self, ClaimLost, slot, round, key.trace(), seen, 0);
                    continue;
                }
                note!(self, Claimed, slot, round, key.trace(), seen, 0);
                if LIST {
                    // A freed slot the hint has passed is one a writer reading
                    // that hint skips unless its filter lane matches, so a
                    // copy there could go unseen: the claim goes back.
                    if seen & STATE_MASK == FREE && self.hint_seq(round) > slot {
                        self.free_own(slot, round, fp);
                        continue;
                    }
                    // The lane first, so a writer starting past the raised hint
                    // finds the claim through the filter.
                    self.write_lane(slot, lane_of(h));
                    self.raise_hint(round, Self::hint_past(slot, probe.claim));
                }
                // SAFETY: this writer holds the slot's claim, so nobody else
                // writes its key, and nobody reads it as published before it is;
                // the slot holds `key_stride` bytes, eight-byte aligned.
                unsafe { key.put(self.key(slot)) };
                #[cfg(test)]
                crate::test_races::pause_point();
                if !current() {
                    self.free_own(slot, round, fp);
                    note!(self, Moved, slot, round, key.trace(), 0, 0);
                    return Outcome::Moved;
                }
                let published = self.word(slot).compare_exchange(
                    slot_word(round, fp, CLAIMED),
                    slot_word(round, fp, FULL),
                    Ordering::AcqRel,
                    Ordering::Relaxed,
                );
                if published.is_err() {
                    note!(self, PublishLost, slot, round, key.trace(), self.word(slot).load(Ordering::Relaxed), 0);
                    // Given up by a pass, or by a writer of the same fingerprint
                    // whose copy the next probe finds.
                    self.free_own(slot, round, fp);
                    if !current() {
                        note!(self, Moved, slot, round, key.trace(), 0, 1);
                        return Outcome::Moved;
                    }
                    continue;
                }
                note!(self, Published, slot, round, key.trace(), 0, 0);
            }
            if !watch.is_empty() && watch.slots().filter(|&j| self.position_in::<LIST>(h, j) < at).any(|j| self.give_up(j, round, fp, key)) {
                self.stand_down(slot, round, fp);
                note!(self, Present, slot, round, key.trace(), 0, 2);
                return Outcome::Present;
            }
            if !carried {
                note!(self, Counted, slot, round, key.trace(), 0, 0);
                self.count_added(round);
            }
            return Outcome::Added;
        }
    }

    /// Whether a probe of this table starts at its first slot.
    pub(crate) fn is_list(&self) -> bool {
        self.shape.kind == Kind::List
    }

    /// Add a key a move carries into this list as [`insert`](Self::insert)
    /// does, but without probing: take the slot at `cursor` when it is one a
    /// probe of `round` ends at. Every slot below the cursor then holds a key
    /// or claim this carrier made, each of another key, since a move carries
    /// each key of the sealed round once; and a slot every probe has ended at
    /// has no copy of the key past it. The cursor moves to the next slot. It
    /// becomes `None`, and this key and every one after it are probed for as
    /// `insert` does, once the slot at the cursor is any other, or another
    /// writer takes it or gives up the claim first.
    pub(crate) fn carry_after<K: Key>(&self, round: u32, h: u64, key: K, cursor: &mut Option<usize>, current: &impl Fn() -> bool) -> Outcome {
        debug_assert!(self.is_list());
        if let Some(slot) = *cursor
            && slot < self.shape.slot_count
        {
            let fp = fingerprint(h);
            let w = self.word(slot).load(Ordering::Acquire);
            if seen(w, round) == Seen::End
                && self.word(slot).compare_exchange(w, slot_word(round, fp, CLAIMED), Ordering::SeqCst, Ordering::Relaxed).is_ok()
            {
                note!(self, CursorClaimed, slot, round, key.trace(), w, 0);
                self.write_lane(slot, lane_of(h));
                self.raise_hint(round, slot + 1);
                // SAFETY: this carrier holds the slot's claim, as an insert's
                // writer does; the slot holds `key_stride` bytes, eight-byte
                // aligned.
                unsafe { key.put(self.key(slot)) };
                if !current() {
                    self.free_own(slot, round, fp);
                    note!(self, Moved, slot, round, key.trace(), 0, 2);
                    return Outcome::Moved;
                }
                let published = self.word(slot).compare_exchange(
                    slot_word(round, fp, CLAIMED),
                    slot_word(round, fp, FULL),
                    Ordering::AcqRel,
                    Ordering::Relaxed,
                );
                if published.is_ok() {
                    note!(self, Published, slot, round, key.trace(), 0, 1);
                    *cursor = Some(slot + 1);
                    return Outcome::Added;
                }
                note!(self, PublishLost, slot, round, key.trace(), self.word(slot).load(Ordering::Relaxed), 1);
                // Given up by a carrier that probed past the claim and placed
                // the key itself.
                self.free_own(slot, round, fp);
                if !current() {
                    note!(self, Moved, slot, round, key.trace(), 0, 3);
                    return Outcome::Moved;
                }
            } else {
                note!(self, CursorStopped, slot, round, key.trace(), w, 0);
            }
        }
        *cursor = None;
        self.insert_key(round, h, key, true, current)
    }

    /// Whether the key is published in `round`; `h` is its [`table_hash`].
    #[cfg(test)]
    pub(crate) fn contains(&self, round: u32, h: u64, key: &[u8]) -> bool {
        self.find(round, h, Bytes(key)).is_some()
    }

    /// Whether a key in any [`Key`] form is published in `round`; `h` is its
    /// [`table_hash`].
    #[inline]
    pub(crate) fn contains_key<K: Key>(&self, round: u32, h: u64, key: K) -> bool {
        self.find(round, h, key).is_some()
    }

    /// The slot of the first published copy of the key in `round` a probe
    /// meets; `h` is its [`table_hash`].
    #[inline]
    pub(crate) fn find<K: Key>(&self, round: u32, h: u64, key: K) -> Option<usize> {
        let fp = fingerprint(h);
        'again: loop {
            let mut slot = self.start(h);
            let mut probes = 0;
            if self.shape.kind == Kind::List {
                let hint = self.hint(round);
                // Not found yet, found at a slot, or a word moved under the
                // comparison and the probe starts again.
                let mut found = Some(None);
                self.visit_lanes(0, hint, lane_of(h), |j| {
                    let w = self.word(j).load(Ordering::Acquire);
                    if w & FP_MASK != fp || seen(w, round) != Seen::Key {
                        return false;
                    }
                    match self.holds(j, w, key) {
                        Some(true) => found = Some(Some(j)),
                        Some(false) => return false,
                        None => found = None,
                    }
                    true
                });
                match found {
                    Some(Some(j)) => return Some(j),
                    Some(None) => {}
                    None => continue 'again,
                }
                slot = hint;
                probes = hint;
            }
            while probes < self.shape.slot_count {
                let w = self.word(slot).load(Ordering::Acquire);
                match seen(w, round) {
                    Seen::End => return None,
                    Seen::Key if w & FP_MASK == fp => match self.holds(slot, w, key) {
                        Some(true) => return Some(slot),
                        Some(false) => {}
                        None => continue 'again,
                    },
                    _ => {}
                }
                slot = self.after(slot);
                probes += 1;
            }
            return None;
        }
    }

    /// Keys published in `round`, each counted once however many copies it
    /// holds: a copy counts when it is the first a probe for its key meets.
    /// A copy that stands down for an earlier one is never that first, so
    /// the answer holds still while writers of the round finish.
    pub(crate) fn distinct(&self, round: u32) -> usize {
        let mut key = vec![0u8; self.shape.key_size];
        let mut n = 0;
        for slot in 0..self.reach(round) {
            loop {
                let w = self.word(slot).load(Ordering::Acquire);
                if seen(w, round) != Seen::Key {
                    break;
                }
                // SAFETY: the keys region holds `key_stride` bytes, eight-byte
                // aligned, at every slot, and `key` holds `key_size`.
                unsafe { take_key(self.key(slot), &mut key) };
                fence(Ordering::Acquire);
                if self.word(slot).load(Ordering::Relaxed) != w {
                    continue;
                }
                if self.find(round, table_hash(&key), Bytes(&key)) == Some(slot) {
                    n += 1;
                }
                break;
            }
        }
        n
    }

    /// Calls `f` with every key published in `round`, each copied out while
    /// its word held still, so a key a later round is replacing is never
    /// handed out torn.
    pub(crate) fn for_each(&self, round: u32, mut f: impl FnMut(&[u8])) {
        let mut key = vec![0u8; self.shape.key_size];
        for slot in 0..self.reach(round) {
            loop {
                let w = self.word(slot).load(Ordering::Acquire);
                if seen(w, round) != Seen::Key {
                    break;
                }
                // SAFETY: the keys region holds `key_stride` bytes, eight-byte
                // aligned, at every slot, and `key` holds `key_size`.
                unsafe { take_key(self.key(slot), &mut key) };
                fence(Ordering::Acquire);
                if self.word(slot).load(Ordering::Relaxed) == w {
                    f(&key);
                    break;
                }
            }
        }
    }

    /// The pass a sealed round gets: every claim of `round` still open is
    /// given up, every published key of an earlier round is retired, and
    /// `copy` is called with each key published in `round`, copied out while
    /// its word held still. Words of later rounds are left alone. Several
    /// passes may run at once.
    pub(crate) fn close<E>(&self, round: u32, mut copy: impl FnMut(&[u8]) -> Result<(), E>) -> Result<(), E> {
        let mut key = vec![0u8; self.shape.key_size];
        for slot in 0..self.reach(round) {
            loop {
                let w = self.word(slot).load(Ordering::SeqCst);
                let r = round_of(w);
                let next = match w & STATE_MASK {
                    CLAIMED if r == round => slot_word(round, w & FP_MASK, SKIPPED),
                    FULL if r == round => {
                        // SAFETY: the keys region holds `key_stride` bytes,
                        // eight-byte aligned, at every slot, and `key` holds
                        // `key_size`.
                        unsafe { take_key(self.key(slot), &mut key) };
                        fence(Ordering::Acquire);
                        if self.word(slot).load(Ordering::Relaxed) != w {
                            continue;
                        }
                        note!(self, PassCopied, slot, round, crate::set_trace::key_word(&key), w, 0);
                        copy(&key)?;
                        break;
                    }
                    FULL if r < round => (w & !STATE_MASK) | TOMB,
                    _ => break,
                };
                if self.word(slot).compare_exchange(w, next, Ordering::SeqCst, Ordering::Relaxed).is_ok() {
                    #[cfg(test)]
                    if next & STATE_MASK == SKIPPED {
                        note!(self, PassGaveUp, slot, round, 0, w, 0);
                    } else {
                        note!(self, PassRetired, slot, round, 0, w, 0);
                    }
                    break;
                }
            }
        }
        Ok(())
    }

    /// The pass a cleared round gets: every claim of `round` still open is
    /// given up, so its writer frees the slot instead of publishing into a
    /// round probes of the next one have passed. Published keys, the
    /// round's own and older ones, stay where they lie, stale from the
    /// round after on: nothing is copied, and a key revived into the
    /// cleared round only turns one stale copy into another.
    pub(crate) fn discard(&self, round: u32) {
        let claimed = slot_word(round, 0, CLAIMED);
        for slot in 0..self.reach(round) {
            let word = self.word(slot);
            let w = word.load(Ordering::SeqCst);
            if (w & !FP_MASK) == claimed {
                Self::give_up_claim(word, w, round);
            }
        }
    }

    /// Give up the claim of `round` the word `w` holds, and any claim of
    /// `round` the slot holds after it is freed and taken again.
    #[cold]
    #[inline(never)]
    fn give_up_claim(word: &AtomicU64, mut w: u64, round: u32) {
        while w & STATE_MASK == CLAIMED && round_of(w) == round {
            match word.compare_exchange(w, slot_word(round, w & FP_MASK, SKIPPED), Ordering::SeqCst, Ordering::SeqCst) {
                Ok(_) => return,
                Err(now) => w = now,
            }
        }
    }

    /// Claim a slot of `round` for the key and leave it claimed, as a writer
    /// that died between its claim and its publish leaves it; the slot
    /// claimed.
    #[cfg(test)]
    pub(crate) fn abandon_claim(&self, round: u32, key: &[u8]) -> usize {
        let h = table_hash(key);
        let probe = if self.is_list() {
            self.probe_in::<true, _>(round, h, Bytes(key), &mut Watch::new())
        } else {
            self.probe_in::<false, _>(round, h, Bytes(key), &mut Watch::new())
        };
        let Some((slot, seen)) = probe.take else { panic!("the abandoned claim found no slot to take") };
        let claimed = slot_word(round, fingerprint(h), CLAIMED);
        self.word(slot).compare_exchange(seen, claimed, Ordering::SeqCst, Ordering::Relaxed).expect("the slot was taken first");
        slot
    }

    /// The address the test-build trace knows this table by.
    #[cfg(test)]
    pub(crate) fn trace_name(&self) -> usize {
        self.base as usize
    }

    /// The state bits of `slot`'s word.
    #[cfg(test)]
    fn state_of(&self, slot: usize) -> u64 {
        self.word(slot).load(Ordering::Relaxed) & STATE_MASK
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use std::sync::Arc;

    fn scratch(name: &str) -> PathBuf {
        let mut p = std::env::temp_dir();
        p.push(format!("subetha-set-table-{name}-{}.bin", std::process::id()));
        p
    }

    fn remove(p: &Path) {
        match std::fs::remove_file(p) {
            Ok(()) => {}
            Err(e) if e.kind() == io::ErrorKind::NotFound => {}
            Err(e) => panic!("{} not removed: {e}", p.display()),
        }
    }

    fn fresh(name: &str, kind: Kind, capacity: usize) -> (SetTable, PathBuf) {
        let p = scratch(name);
        remove(&p);
        let table = SetTable::create_or_attach(&p, kind, capacity, 8).expect("created");
        (table, p)
    }

    /// The state word always reads as the one the round was read from.
    fn still() -> bool {
        true
    }

    fn add_in(table: &SetTable, round: u32, k: u64) -> Outcome {
        let key = k.to_le_bytes();
        table.insert(round, table_hash(&key), &key, false, &still)
    }

    fn add(table: &SetTable, k: u64) -> Outcome {
        add_in(table, 0, k)
    }

    fn has_in(table: &SetTable, round: u32, k: u64) -> bool {
        let key = k.to_le_bytes();
        table.contains(round, table_hash(&key), &key)
    }

    fn has(table: &SetTable, k: u64) -> bool {
        has_in(table, 0, k)
    }

    fn keys_in(table: &SetTable, round: u32) -> Vec<u64> {
        let mut out = Vec::new();
        table.for_each(round, |k| out.push(u64::from_le_bytes(k.try_into().expect("eight bytes"))));
        out.sort_unstable();
        out
    }

    /// The pass of `round` as a clear runs it: nothing copied.
    fn clear_round(table: &SetTable, round: u32) {
        table.close(round, |_| Ok::<(), ()>(())).expect("the pass");
        table.begin(round + 1);
    }

    #[test]
    fn a_key_is_held_once_on_either_kind() {
        for kind in [Kind::List, Kind::Map] {
            let (t, p) = fresh(&format!("once-{}", kind as u32), kind, 64);
            for k in 0..40u64 {
                assert_eq!(add(&t, k), Outcome::Added, "{kind:?} {k}");
            }
            for k in 0..40u64 {
                assert_eq!(add(&t, k), Outcome::Present, "{kind:?} {k} again");
                assert!(has(&t, k), "{kind:?} holds {k}");
            }
            assert!(!has(&t, 999), "{kind:?} holds nothing else");
            assert_eq!(t.len(0), Some(40));
            assert_eq!(keys_in(&t, 0), (0..40).collect::<Vec<_>>());
            drop(t);
            remove(&p);
        }
    }

    #[test]
    fn a_full_table_refuses_a_new_key_and_still_answers_a_present_one() {
        for kind in [Kind::List, Kind::Map] {
            let (t, p) = fresh(&format!("full-{}", kind as u32), kind, 5);
            for k in 0..5u64 {
                assert_eq!(add(&t, k), Outcome::Added);
            }
            assert_eq!(add(&t, 5), Outcome::Full, "{kind:?} past its capacity");
            assert_eq!(add(&t, 3), Outcome::Present, "{kind:?} a held key at capacity");
            assert_eq!(t.len(0), Some(5));
            drop(t);
            remove(&p);
        }
    }

    #[test]
    fn a_table_of_another_shape_is_refused_by_its_field() {
        let (t, p) = fresh("shape", Kind::Map, 32);
        drop(t);
        let refused = SetTable::open(&p, Kind::List, 32, 8).err().expect("another strategy");
        assert_eq!(Mismatch::carried_by(&refused), Some(Mismatch::new("strategy", 1, 0)));
        let refused = SetTable::open(&p, Kind::Map, 32, 4).err().expect("another key size");
        assert_eq!(Mismatch::carried_by(&refused), Some(Mismatch::new("element size", 8, 4)));
        let again = SetTable::open(&p, Kind::Map, 32, 8).expect("the same shape attaches");
        drop(again);
        remove(&p);
    }

    /// A capacity whose count cannot fit a round's 32-bit half is refused.
    #[test]
    fn a_capacity_past_a_rounds_count_is_refused() {
        let refused = Shape::new(Kind::Map, u32::MAX as usize + 1, 8).expect_err("past u32");
        assert_eq!(refused.kind(), io::ErrorKind::InvalidInput);
    }

    /// A claim whose writer never published is passed: a writer of the same
    /// fingerprint gives it up and keeps its own copy.
    #[test]
    fn a_claim_left_open_is_given_up_and_the_key_lands_once() {
        for kind in [Kind::List, Kind::Map] {
            let (t, p) = fresh(&format!("dead-{}", kind as u32), kind, 32);
            let dead = t.abandon_claim(0, &7u64.to_le_bytes());
            assert_eq!(add(&t, 7), Outcome::Added, "{kind:?}: the open claim does not stop the key");
            assert_eq!(t.state_of(dead), SKIPPED, "{kind:?}: the open claim is given up");
            assert_eq!(add(&t, 7), Outcome::Present);
            assert_eq!(t.len(0), Some(1));
            assert_eq!(keys_in(&t, 0), vec![7]);
            drop(t);
            remove(&p);
        }
    }

    /// Threads adding the same keys in different orders: each key is added
    /// by exactly one of them, and the table holds each once.
    #[test]
    fn writers_adding_the_same_keys_add_each_exactly_once() {
        let cores = std::thread::available_parallelism().expect("the host reports its parallelism").get();
        let threads = cores.clamp(2, 8);
        for kind in [Kind::List, Kind::Map] {
            let (t, p) = fresh(&format!("race-{}", kind as u32), kind, 600);
            let t = Arc::new(t);
            let handles: Vec<_> = (0..threads)
                .map(|n| {
                    let t = Arc::clone(&t);
                    std::thread::spawn(move || {
                        let mut added = vec![0u32; 500];
                        for step in 0..500u64 {
                            let k = (step * (2 * n as u64 + 1) + n as u64 * 37) % 500;
                            if add(&t, k) == Outcome::Added {
                                added[k as usize] += 1;
                            }
                        }
                        added
                    })
                })
                .collect();
            let mut total = vec![0u32; 500];
            for h in handles {
                for (k, n) in h.join().expect("a writer").into_iter().enumerate() {
                    total[k] += n;
                }
            }
            assert!(total.iter().all(|&n| n == 1), "{kind:?}: every key added exactly once: {total:?}");
            assert_eq!(t.len(0), Some(500), "{kind:?}");
            assert_eq!(keys_in(&t, 0), (0..500).collect::<Vec<_>>(), "{kind:?}");
            drop(t);
            remove(&p);
        }
    }

    /// A write whose state word moved under its claim frees the claim and
    /// reports it; the pass copies only what was published before.
    #[test]
    fn a_write_whose_state_moved_reports_it_and_the_pass_copies_what_came_before() {
        for kind in [Kind::List, Kind::Map] {
            let (t, p) = fresh(&format!("moved-{}", kind as u32), kind, 32);
            for k in 0..10u64 {
                assert_eq!(add(&t, k), Outcome::Added);
            }
            let key = 50u64.to_le_bytes();
            assert_eq!(t.insert(0, table_hash(&key), &key, false, &|| false), Outcome::Moved, "{kind:?}");
            assert!(!has(&t, 50), "{kind:?}: nothing published after the move");
            assert_eq!(add(&t, 3), Outcome::Present, "{kind:?}: a held key is still held");
            let mut copied = Vec::new();
            t.close(0, |k| {
                copied.push(u64::from_le_bytes(k.try_into().expect("eight bytes")));
                Ok::<(), ()>(())
            })
            .expect("the pass");
            copied.sort_unstable();
            assert_eq!(copied, (0..10).collect::<Vec<_>>(), "{kind:?}");
            drop(t);
            remove(&p);
        }
    }

    /// A writer stopped between its claim and its state check while the
    /// round is passed: the pass gives its claim up, the writer frees it and
    /// reports the move, and its key is not copied.
    #[test]
    fn a_writer_stopped_inside_its_claim_while_the_round_is_passed_moves_on() {
        for kind in [Kind::List, Kind::Map] {
            let (t, p) = fresh(&format!("stalled-{}", kind as u32), kind, 32);
            let t = Arc::new(t);
            assert_eq!(add(&t, 1), Outcome::Added);
            let sealed = Arc::new(std::sync::atomic::AtomicBool::new(false));
            let (writer, flag) = (Arc::clone(&t), Arc::clone(&sealed));
            let (pause, stalled) = crate::test_races::stopped(move || {
                let key = 99u64.to_le_bytes();
                writer.insert(0, table_hash(&key), &key, false, &|| !flag.load(Ordering::SeqCst))
            });
            sealed.store(true, Ordering::SeqCst);
            let mut copied = Vec::new();
            t.close(0, |k| {
                copied.push(u64::from_le_bytes(k.try_into().expect("eight bytes")));
                Ok::<(), ()>(())
            })
            .expect("the pass");
            pause.release();
            assert_eq!(stalled.join().expect("the writer"), Outcome::Moved, "{kind:?}");
            assert_eq!(copied, vec![1], "{kind:?}: the stalled key is not copied");
            assert!(!has(&t, 99), "{kind:?}");
            drop(t);
            remove(&p);
        }
    }

    /// The filter is read four lanes at a time; a key in any lane of any
    /// word is found, and a key whose lane matches another's is told apart.
    #[test]
    fn a_list_finds_a_key_in_every_lane_of_its_filter() {
        let (t, p) = fresh("lanes", Kind::List, 200);
        for k in 0..199u64 {
            assert_eq!(add(&t, k * 7919), Outcome::Added);
        }
        for k in 0..199u64 {
            assert!(has(&t, k * 7919), "{k}");
            assert_eq!(add(&t, k * 7919), Outcome::Present, "{k}");
        }
        for k in 0..64u64 {
            assert!(!has(&t, k * 7919 + 1), "{k}");
        }
        assert_eq!(t.len(0), Some(199));
        drop(t);
        remove(&p);
    }

    /// A new round starts empty in the same file, a key of the round before
    /// is taken back in its own slot, and a new key takes a stale slot.
    #[test]
    fn a_new_round_starts_empty_and_takes_back_the_keys_of_the_round_before() {
        for kind in [Kind::List, Kind::Map] {
            let (t, p) = fresh(&format!("rounds-{}", kind as u32), kind, 64);
            for k in 0..30u64 {
                assert_eq!(add(&t, k), Outcome::Added);
            }
            clear_round(&t, 0);
            assert_eq!(t.len(1), Some(0), "{kind:?}: the new round counts nothing");
            assert!((0..30).all(|k| !has_in(&t, 1, k)), "{kind:?}: no key of round 0 is in round 1");
            assert_eq!(keys_in(&t, 1), Vec::<u64>::new());
            for k in 0..30u64 {
                assert_eq!(add_in(&t, 1, k), Outcome::Added, "{kind:?}: {k} is new to round 1");
            }
            for k in 100..110u64 {
                assert_eq!(add_in(&t, 1, k), Outcome::Added, "{kind:?}: a new key takes a slot");
            }
            assert_eq!(t.len(1), Some(40));
            assert_eq!(keys_in(&t, 1), (0..30).chain(100..110).collect::<Vec<_>>(), "{kind:?}");
            drop(t);
            remove(&p);
        }
    }

    /// A pass retires every stale key it passes, so none is taken back
    /// behind it; a map's pass passes every slot.
    #[test]
    fn a_pass_retires_the_stale_keys_it_passes() {
        for kind in [Kind::List, Kind::Map] {
            let (t, p) = fresh(&format!("retire-{}", kind as u32), kind, 64);
            for k in 0..20u64 {
                assert_eq!(add(&t, k), Outcome::Added);
            }
            clear_round(&t, 0);
            assert_eq!(add_in(&t, 1, 3), Outcome::Added);
            clear_round(&t, 1);
            if kind == Kind::Map {
                let stale_full = (0..t.shape.slot_count)
                    .filter(|&s| {
                        let w = t.word(s).load(Ordering::Relaxed);
                        w & STATE_MASK == FULL && round_of(w) == 0
                    })
                    .count();
                assert_eq!(stale_full, 0, "round 1's pass retired every key of round 0");
            }
            assert_eq!(add_in(&t, 2, 7), Outcome::Added, "{kind:?}");
            assert_eq!(keys_in(&t, 2), vec![7], "{kind:?}");
            drop(t);
            remove(&p);
        }
    }

    /// Writers adding one key set across several rounds, a clear between
    /// each: every round holds each key once and counts it once.
    #[test]
    fn writers_racing_across_rounds_add_each_key_once_per_round() {
        let (t, p) = fresh("rounds-race", Kind::Map, 300);
        let t = Arc::new(t);
        for round in 0..4u32 {
            let handles: Vec<_> = (0..4u64)
                .map(|n| {
                    let t = Arc::clone(&t);
                    std::thread::spawn(move || (0..200u64).filter(|s| add_in(&t, round, (s * 7 + n * 13) % 200) == Outcome::Added).count())
                })
                .collect();
            let added: usize = handles.into_iter().map(|h| h.join().expect("a writer")).sum();
            assert_eq!(added, 200, "round {round}: each key added once");
            assert_eq!(t.len(round), Some(200));
            assert_eq!(keys_in(&t, round), (0..200).collect::<Vec<_>>());
            clear_round(&t, round);
        }
        drop(t);
        remove(&p);
    }

    /// A run of up to eight bytes reads as the zero-filled little-endian
    /// word and writes back from it, at every length, touching no byte past
    /// its own.
    #[test]
    fn short_runs_read_and_write_as_zero_filled_words() {
        let bytes: [u8; 8] = [0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88];
        for n in 0..=8 {
            let mut padded = [0u8; 8];
            padded[..n].copy_from_slice(&bytes[..n]);
            let word = small_word(&bytes[..n]);
            assert_eq!(word, u64::from_le_bytes(padded), "length {n}");
            let mut out = [0u8; 9];
            put_small(&mut out[..n], word);
            assert_eq!(&out[..n], &bytes[..n], "length {n}");
            assert!(out[n..].iter().all(|&b| b == 0), "length {n} wrote past its bytes");
        }
    }

    /// Keys of every length from one to three words are stored, compared and
    /// read back whole, and a key differing in its last byte is not the
    /// stored one.
    #[test]
    fn keys_of_every_length_round_trip_through_a_slot() {
        for len in 1..=24usize {
            let key: Vec<u8> = (0..len).map(|i| (i as u8).wrapping_mul(37).wrapping_add(1)).collect();
            let mut slot = [u64::MAX; 3];
            let at = slot.as_mut_ptr() as *mut u8;
            // SAFETY: three aligned words hold every key up to 24 bytes.
            unsafe { put_key(at, &key) };
            // SAFETY: as above.
            assert!(unsafe { same_key(at, &key) }, "length {len}");
            let mut other = key.clone();
            if let Some(last) = other.last_mut() {
                *last ^= 1;
            }
            // SAFETY: as above.
            assert!(!unsafe { same_key(at, &other) }, "length {len}: a different last byte matched");
            let mut out = vec![0u8; len];
            // SAFETY: as above.
            unsafe { take_key(at, &mut out) };
            assert_eq!(out, key, "length {len}");
        }
    }

    /// The hash folds every length as the word algorithm states: whole words,
    /// then the zero-filled last word, then the length and the finalizer.
    /// Tables already on disk place their keys by it.
    #[test]
    fn the_hash_of_every_length_follows_the_word_algorithm() {
        fn by_words(key: &[u8]) -> u64 {
            const FOLD: u64 = 0x9E37_79B9_7F4A_7C15;
            let mut h = 0u64;
            for chunk in key.chunks(8) {
                let mut word = [0u8; 8];
                word[..chunk.len()].copy_from_slice(chunk);
                h = (h ^ u64::from_le_bytes(word)).wrapping_mul(FOLD).rotate_left(31);
            }
            h ^= key.len() as u64;
            h ^= h >> 33;
            h = h.wrapping_mul(0xff51_afd7_ed55_8ccd);
            h ^= h >> 33;
            h = h.wrapping_mul(0xc4ce_b9fe_1a85_ec53);
            h ^ (h >> 33)
        }
        let key: Vec<u8> = (0..24u8).map(|i| i.wrapping_mul(91).wrapping_add(7)).collect();
        for len in 0..=24 {
            assert_eq!(hash_key(&key[..len]), by_words(&key[..len]), "length {len}");
        }
    }

    /// A key of one to eight bytes hashes, stores and compares the same as a
    /// word as it does as bytes, so either form finds what the other wrote;
    /// its key hash is [`hash_key`] of its bytes in both forms.
    #[test]
    fn a_short_key_is_the_same_key_as_a_word_and_as_bytes() {
        for len in 1..=8usize {
            let key: Vec<u8> = (0..len).map(|i| (i as u8).wrapping_mul(53).wrapping_add(3)).collect();
            let (word, bytes) = (Word::new(&key), Bytes(&key));
            assert_eq!(word.hash(), bytes.hash(), "length {len}");
            assert_eq!(word.key_hash(), hash_key(&key), "length {len}: a word's key hash");
            assert_eq!(bytes.key_hash(), hash_key(&key), "length {len}: the bytes' key hash");
            assert_eq!(word.trace(), bytes.trace(), "length {len}");
            let mut by_word = [u64::MAX; 1];
            let mut by_bytes = [u64::MAX; 1];
            // SAFETY: one aligned word holds every key up to eight bytes.
            unsafe { word.put(by_word.as_mut_ptr() as *mut u8) };
            // SAFETY: as above.
            unsafe { bytes.put(by_bytes.as_mut_ptr() as *mut u8) };
            assert_eq!(by_word, by_bytes, "length {len}: the forms stored different words");
            let mut other = key.clone();
            other[len - 1] ^= 0x40;
            for (stored, form) in [(by_word, "word"), (by_bytes, "bytes")] {
                let at = stored.as_ptr() as *const u8;
                // SAFETY: as above.
                assert!(unsafe { word.same(at) && bytes.same(at) }, "length {len}: the {form} copy");
                // SAFETY: as above.
                assert!(!unsafe { Word::new(&other).same(at) }, "length {len}: a different last byte matched the {form} copy");
            }
        }
        for kind in [Kind::List, Kind::Map] {
            let (t, p) = fresh(&format!("forms-{kind:?}"), kind, 64);
            for k in 0..40u64 {
                let key = k.to_le_bytes();
                let added = if k % 2 == 0 {
                    t.insert_key(0, Word::new(&key).hash(), Word::new(&key), false, &still)
                } else {
                    t.insert(0, table_hash(&key), &key, false, &still)
                };
                assert_eq!(added, Outcome::Added, "{kind:?} key {k}");
            }
            for k in 0..40u64 {
                let key = k.to_le_bytes();
                assert!(t.contains(0, table_hash(&key), &key), "{kind:?} key {k} as bytes");
                assert!(t.contains_key(0, Word::new(&key).hash(), Word::new(&key)), "{kind:?} key {k} as a word");
                let again = t.insert_key(0, Word::new(&key).hash(), Word::new(&key), false, &still);
                assert_eq!(again, Outcome::Present, "{kind:?} key {k} added twice");
            }
            assert_eq!(t.len(0), Some(40));
            drop(t);
            remove(&p);
        }
    }
}
