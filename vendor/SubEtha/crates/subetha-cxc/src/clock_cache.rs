//! The cache behind [`SharedLRUCache`](crate::shared_lru_cache::SharedLRUCache)
//! and [`RawLruCache`](crate::raw_lru_cache::RawLruCache): keys and values of
//! sizes fixed at creation, in one mapped file that any number of processes
//! write at once, none of them waiting on another, evicted by CLOCK.
//!
//! # Frames
//!
//! An entry lives in a frame: its key, its value and the key's hash, behind
//! one word holding the frame's state, the process id of the writer that
//! filled it and a generation that steps each time the frame is freed or
//! refilled, an identity word beside it holding that writer's pid and start
//! time, and a mark a put or a touch sets. The word and the identity word
//! are one 16-byte pair: a claim of a frame another process filled swaps
//! both at once, so a cache opens only on a processor with a lock-free
//! 16-byte compare-and-swap. A frame does not change while it holds an
//! entry. A put of a new value fills another frame and swaps the index to
//! it. A reader copies a key and a value no writer is touching, then checks
//! the frame's word to know the frame was not freed meanwhile.
//!
//! # Index
//!
//! The index is one 64-byte bucket per two entries the cache holds, rounded
//! up to a power of two: eight entry words. An entry word names a frame and
//! carries a fingerprint of the key's hash, the low bits of the frame's
//! generation and a state: pending, committed or given up. A key lives in
//! the bucket its hash names, so a lookup reads one bucket and the frame it
//! names, and sees committed entries only.
//!
//! An insert fills a frame and publishes a pending entry for it in the first
//! free word of the bucket, then reads the bucket's other words. A committed
//! entry of its key means another insert landed first: it takes its own
//! entry out and updates that one. A pending entry of its key in an earlier
//! word wins: it commits that one for its writer and updates it. A pending
//! entry of its key in a later word it gives up. With neither, it commits
//! its own entry with one compare-and-swap; a commit another writer made for
//! it counts as its own. Every insert reads the bucket after it publishes,
//! so of two inserts of one key one sees the other, and exactly one commits
//! as the insert. A bucket whose eight words all hold entries makes room: a
//! word whose frame has moved on, or that holds a given-up entry, is taken
//! back first, then a committed entry is evicted. A bucket of pending
//! entries has its first one settled for its writer, as that writer would
//! settle it, so the next pass takes it out either way, and no put waits on
//! a writer stalled mid-put. A given-up entry's writer may still run, so its
//! frame is marked filling, in its generation, before the word is taken
//! back: a writer finding its word taken knows by the mark that its entry
//! never committed. A writer whose entry lost moves its frame to the next
//! generation before it publishes again.
//!
//! # Eviction
//!
//! A put of a new key into a full cache evicts, as does
//! [`evict`](crate::clock_cache::CacheCore::evict). The hand is one word naming a frame: an
//! evictor scans the frames on from it, clearing each mark it passes and
//! taking out the first entry it finds unmarked, then moves the hand to the
//! frame after the last it scanned. A put and a touch set the mark, and a
//! get leaves it. Whoever takes an entry out of the index owns its frame,
//! and a put that evicted fills the frame it took instead of freeing it and
//! taking another.
//!
//! # Dead writers
//!
//! The hand frees a frame whose writer is gone while the frame is being
//! filled, or while it is live but no committed entry names it, as a
//! writer that died before its entry committed leaves it. A writer is gone
//! when its pid names no process, or names one that started after the
//! writer did, so a reused pid never keeps a dead writer's frame. The owner
//! of a frame another process wrote frees or claims it with one swap of its
//! word, as the hand does, so the two never both free it; a frame its own
//! process wrote, by pid and start, is the owner's alone, since that writer
//! is alive. An entry its writer published and died before committing
//! stays pending until the next insert of its key, or a put making room in
//! its bucket, settles it for the writer, or until the hand frees its frame
//! and its word is taken back.

use std::fs::File;
use std::io;
use std::mem::size_of;
use std::path::{Path, PathBuf};
use std::sync::atomic::{fence, AtomicU32, AtomicU64, Ordering};

use memmap2::{MmapMut, MmapOptions};

use crate::atomic_pair::AtomicPair;
use crate::layout::Mismatch;
use crate::peer_directory::current_pid;
use crate::process_identity;
use crate::set_table::{hash_key, take_key, Bytes, Key, Word};

/// Record an event about `$cache` in the test-build trace, when a test named
/// the cache: `$slot` a bucket word, numbered across buckets eight apiece,
/// `$key` the key's bytes, `form $form` a key in a [`Key`] form, or `frame
/// $frame` for the key frame `$frame` holds. Outside a test build it is
/// nothing.
macro_rules! note {
    ($cache:expr, $what:ident, $slot:expr, frame $frame:expr, $word:expr, $extra:expr) => {
        #[cfg(test)]
        crate::set_trace::record(
            $cache.base as usize,
            crate::set_trace::What::$what,
            $slot,
            0,
            $cache.trace_key($frame),
            $word,
            $extra,
        );
    };
    ($cache:expr, $what:ident, $slot:expr, form $form:expr, $word:expr, $extra:expr) => {
        #[cfg(test)]
        crate::set_trace::record(
            $cache.base as usize,
            crate::set_trace::What::$what,
            $slot,
            0,
            $form.trace(),
            $word,
            $extra,
        );
    };
    ($cache:expr, $what:ident, $slot:expr, $key:expr, $word:expr, $extra:expr) => {
        #[cfg(test)]
        crate::set_trace::record(
            $cache.base as usize,
            crate::set_trace::What::$what,
            $slot,
            0,
            crate::set_trace::key_word($key),
            $word,
            $extra,
        );
    };
}

const CACHE_MAGIC: u32 = 0x4B43_4C43;
const CACHE_FORMAT: u32 = 3;

/// Entry words in a bucket.
const WAYS: usize = 8;

/// A frame word: the state in its low two bits, the writer's process id in
/// bits 8 to 39, and the generation in the top 24 bits.
const STATE_MASK: u64 = 0b11;
const FREE: u64 = 0;
const FILLING: u64 = 1;
const LIVE: u64 = 2;
const GENERATION_MASK: u32 = (1 << 24) - 1;

/// Where a frame's fields lie from its start: the word and the identity
/// word, one 16-byte pair, then the key's hash, the free-list link, the
/// mark, then the key and the value. A frame's stride is a multiple of 16.
const WRITER_AT: usize = 8;
const HASH_AT: usize = 16;
const NEXT_AT: usize = 24;
const MARK_AT: usize = 28;
const KEY_AT: usize = 32;

fn generation_of(w: u64) -> u32 {
    (w >> 40) as u32
}

/// The pid of the process that filled the frame whose word is `w`; zero for
/// a free one.
fn writer_of(w: u64) -> u32 {
    (w >> 8) as u32
}

fn frame_word(generation: u32, writer: u32, state: u64) -> u64 {
    (u64::from(generation & GENERATION_MASK) << 40) | (u64::from(writer) << 8) | state
}

/// An entry word's state, in bits 40 and 41: zero for a committed entry.
const ENTRY_STATE: u64 = 0b11 << 40;
/// An entry its insert has published and not yet committed: no lookup sees
/// it.
const ENTRY_PENDING: u64 = 0b01 << 40;
/// A pending entry another insert gave up: it never commits, and its word
/// is taken back.
const ENTRY_GIVEN_UP: u64 = 0b10 << 40;

/// The fingerprint an entry word carries: the hash's top 22 bits.
fn fingerprint(h: u64) -> u64 {
    h >> 42
}

/// A committed entry word: the fingerprint in the top 22 bits, the state
/// below it, the low 8 bits of the frame's generation below that, and one
/// past the frame in the low half.
fn entry_word(fp: u64, generation: u32, frame: usize) -> u64 {
    (fp << 42) | (u64::from(generation & 0xFF) << 32) | (frame as u64 + 1)
}

fn fp_of(e: u64) -> u64 {
    e >> 42
}

/// Bit `i` set for each word `i` of `bucket` whose fingerprint reads `fp`,
/// from one pass over the bucket's line: four aligned 16-byte loads, each
/// 8-byte word read whole, in place of eight loads. A caller loads again,
/// atomically, every word it acts on. On x86-64 a locked operation orders
/// every later load after it, so a bucket read after a put's publishing
/// swap sees what eight loads in its place would.
#[cfg(target_arch = "x86_64")]
#[inline]
fn fingerprint_mask(bucket: &[AtomicU64; WAYS], fp: u64) -> u32 {
    use std::arch::x86_64::{
        __m128i, _mm_castsi128_ps, _mm_cmpeq_epi32, _mm_movemask_ps, _mm_set1_epi64x, _mm_srli_epi64,
    };
    let (w0, w1, w2, w3): (__m128i, __m128i, __m128i, __m128i);
    // SAFETY: a bucket is 64 bytes at a 64-byte-aligned offset of a mapping
    // that starts on a page, and readable; the loads sit in `asm!`, outside
    // the language's memory model, so they race no atomic access.
    unsafe {
        std::arch::asm!(
            "movdqa {w0}, xmmword ptr [{p}]",
            "movdqa {w1}, xmmword ptr [{p} + 16]",
            "movdqa {w2}, xmmword ptr [{p} + 32]",
            "movdqa {w3}, xmmword ptr [{p} + 48]",
            p = in(reg) bucket.as_ptr(),
            w0 = out(xmm_reg) w0,
            w1 = out(xmm_reg) w1,
            w2 = out(xmm_reg) w2,
            w3 = out(xmm_reg) w3,
            options(nostack, preserves_flags, readonly),
        );
    }
    // A fingerprint is a word's top 22 bits: shifted down 42 it fills the
    // low 32-bit half of its word and leaves the high half 0, as `fp` does,
    // so the low halves' 32-bit compare answers each word.
    // SAFETY: SSE2 is part of every x86-64 processor, so these intrinsics
    // run wherever this compiles.
    unsafe {
        let want = _mm_set1_epi64x(fp as i64);
        let pair = |w: __m128i| -> u32 {
            let m = _mm_movemask_ps(_mm_castsi128_ps(_mm_cmpeq_epi32(_mm_srli_epi64::<42>(w), want))) as u32;
            (m & 1) | ((m >> 1) & 2)
        };
        pair(w0) | (pair(w1) << 2) | (pair(w2) << 4) | (pair(w3) << 6)
    }
}

/// Every word of the bucket, for the caller to load and test one by one.
#[cfg(not(target_arch = "x86_64"))]
#[inline]
fn fingerprint_mask(_bucket: &[AtomicU64; WAYS], _fp: u64) -> u32 {
    (1 << WAYS) - 1
}

fn tag_of(e: u64) -> u32 {
    ((e >> 32) & 0xFF) as u32
}

/// The frame an entry word names; `None` for an empty word.
fn frame_of(e: u64) -> Option<usize> {
    ((e & 0xFFFF_FFFF) as usize).checked_sub(1)
}

/// Why an operation on the cache could not be carried out.
#[derive(Debug)]
pub(crate) enum CacheError {
    /// The file holds a cache of another shape.
    Mismatch(Mismatch),
    /// The operating system refused to open, size or map the file.
    Io(io::Error),
    /// No frame could be freed for a new entry: every frame is being filled
    /// by a writer still alive.
    Full,
}

impl From<io::Error> for CacheError {
    fn from(e: io::Error) -> Self {
        match Mismatch::carried_by(&e) {
            Some(m) => Self::Mismatch(m),
            None => Self::Io(e),
        }
    }
}

/// The cache file's first four lines.
#[repr(C, align(64))]
struct CacheHeader {
    magic: u32,
    format: u32,
    key_size: u32,
    value_size: u32,
    capacity: u64,
    frame_count: u64,
    bucket_count: u64,
    _line0: [u8; 24],
    /// Entries in the index, a two's-complement count: an entry can be taken
    /// out, and counted gone, before the put that inserted it counts it,
    /// which leaves the count below zero for that moment.
    len: AtomicU64,
    _line1: [u8; 56],
    /// The frame the hand's next scan starts at.
    hand: AtomicU64,
    /// One past the highest frame ever taken off the free list: no frame at
    /// or above it has held an entry, so the hand wraps here. Only grows; 0
    /// in a cache no put has used yet.
    used: AtomicU64,
    _line2: [u8; 48],
    /// The free list: a tag in the high half, one past the frame on top in the
    /// low half, zero when no frame is free.
    free: AtomicU64,
    _line3: [u8; 56],
}

const _: () = assert!(size_of::<CacheHeader>() == 256);

/// Where everything in a cache file lies, for one capacity, key size and
/// value size.
#[derive(Debug, Clone, Copy)]
struct Shape {
    key_size: usize,
    value_size: usize,
    key_stride: usize,
    capacity: usize,
    frame_count: usize,
    bucket_count: usize,
    frame_stride: usize,
    buckets_at: usize,
    frames_at: usize,
    total: usize,
}

impl Shape {
    /// An eighth more frames than entries, at least sixteen, so a put of a new
    /// value finds a free frame without evicting; and a bucket per two entries.
    fn new(capacity: usize, key_size: usize, value_size: usize) -> io::Result<Self> {
        let invalid = |what: &str| io::Error::new(io::ErrorKind::InvalidInput, what.to_string());
        if capacity == 0 {
            return Err(invalid("a cache holds at least one entry"));
        }
        if key_size == 0 {
            return Err(invalid("a key is at least one byte"));
        }
        if u32::try_from(key_size).is_err() || u32::try_from(value_size).is_err() {
            return Err(invalid("a key or a value is under four gigabytes"));
        }
        let too_large = || invalid("the cache for this capacity, key size and value size does not fit in memory");
        let frame_count = capacity
            .checked_add((capacity / 8).max(16))
            .filter(|&n| n < u32::MAX as usize)
            .ok_or_else(|| invalid("a cache holds fewer than 4294967295 frames"))?;
        let bucket_count = capacity.div_ceil(2).checked_next_power_of_two().ok_or_else(too_large)?;
        let key_stride = key_size.checked_next_multiple_of(8).ok_or_else(too_large)?;
        let value_stride = value_size.checked_next_multiple_of(8).ok_or_else(too_large)?;
        let frame_stride = KEY_AT
            .checked_add(key_stride)
            .and_then(|n| n.checked_add(value_stride))
            .and_then(|n| n.checked_next_multiple_of(16))
            .ok_or_else(too_large)?;
        let buckets_at = size_of::<CacheHeader>();
        let frames_at = bucket_count.checked_mul(64).and_then(|n| n.checked_add(buckets_at)).ok_or_else(too_large)?;
        let total = frame_count.checked_mul(frame_stride).and_then(|n| n.checked_add(frames_at)).ok_or_else(too_large)?;
        Ok(Self { key_size, value_size, key_stride, capacity, frame_count, bucket_count, frame_stride, buckets_at, frames_at, total })
    }

    /// Lay a fresh file out: every field and the free list first, the magic
    /// last, because an attacher spins on it.
    ///
    /// # Safety
    /// `ptr` addresses at least `self.total` writable zeroed bytes, 64-byte
    /// aligned.
    unsafe fn init(&self, ptr: *mut u8) {
        let hdr = ptr as *mut CacheHeader;
        unsafe {
            (*hdr).format = CACHE_FORMAT;
            (*hdr).key_size = self.key_size as u32;
            (*hdr).value_size = self.value_size as u32;
            (*hdr).capacity = self.capacity as u64;
            (*hdr).frame_count = self.frame_count as u64;
            (*hdr).bucket_count = self.bucket_count as u64;
            // Each frame's link names the one above it, so the list pops
            // frame 0 first. The frame count is below u32::MAX, so every link
            // fits its 32 bits.
            for f in 0..self.frame_count {
                let next = &*(ptr.add(self.frames_at + f * self.frame_stride + NEXT_AT) as *const AtomicU32);
                next.store(if f + 1 < self.frame_count { (f + 2) as u32 } else { 0 }, Ordering::Relaxed);
            }
            (*hdr).free.store(1, Ordering::Relaxed);
            std::ptr::write_volatile(&raw mut (*hdr).magic, CACHE_MAGIC);
        }
    }

    /// The first header field of the cache at `base` that differs from this
    /// shape.
    fn check(&self, base: *const u8) -> io::Result<()> {
        // SAFETY: the caller mapped at least a header's length at `base`.
        let hdr = unsafe { &*(base as *const CacheHeader) };
        Mismatch::first(&[
            ("magic", u64::from(hdr.magic), u64::from(CACHE_MAGIC)),
            ("format", u64::from(hdr.format), u64::from(CACHE_FORMAT)),
            ("key size", u64::from(hdr.key_size), self.key_size as u64),
            ("value size", u64::from(hdr.value_size), self.value_size as u64),
            ("capacity", hdr.capacity, self.capacity as u64),
            ("frames", hdr.frame_count, self.frame_count as u64),
            ("buckets", hdr.bucket_count, self.bucket_count as u64),
        ])
        .map_err(io::Error::from)
    }
}

/// An entry found for a key: the bucket word holding it, the entry, its
/// frame, and the frame's word as it read while the frame held the key.
#[derive(Debug, Clone, Copy)]
struct Found {
    slot: usize,
    entry: u64,
    frame: usize,
    word: u64,
}

/// One cache, mapped.
pub(crate) struct CacheCore {
    _file: File,
    mmap: MmapMut,
    base: *mut u8,
    shape: Shape,
    bucket_mask: usize,
}

// SAFETY: every shared access goes through the atomic header words, bucket
// words and frame words, and a frame's key and value are written only by the
// writer that claimed the frame, before the release that publishes them.
unsafe impl Send for CacheCore {}
// SAFETY: as for Send.
unsafe impl Sync for CacheCore {}

impl CacheCore {
    /// The cache file beside `base`.
    pub(crate) fn path(base: &Path) -> PathBuf {
        let mut p = base.as_os_str().to_owned();
        p.push(".cache.bin");
        PathBuf::from(p)
    }

    fn from_parts(file: File, mut mmap: MmapMut, shape: Shape) -> Self {
        let base = mmap.as_mut_ptr();
        Self { _file: file, mmap, base, shape, bucket_mask: shape.bucket_count - 1 }
    }

    /// A cache claims a frame another process filled with a 16-byte swap; a
    /// processor without one is refused.
    fn supported() -> Result<(), CacheError> {
        if crate::atomic_pair::supported() {
            return Ok(());
        }
        Err(CacheError::Io(io::Error::new(io::ErrorKind::Unsupported, "this processor has no lock-free 16-byte compare-and-swap")))
    }

    /// Obtain the cache at `base`: an empty one is laid out when the file does
    /// not exist, and an existing one is attached with its entries in place.
    pub(crate) fn create(base: &Path, capacity: usize, key_size: usize, value_size: usize) -> Result<Self, CacheError> {
        Self::supported()?;
        let shape = Shape::new(capacity, key_size, value_size)?;
        let (file, mmap) = crate::mmf_attach::create_or_attach(
            &Self::path(base),
            shape.total,
            // SAFETY: the builder hands `init` a zeroed, page-aligned mapping of
            // `shape.total` bytes.
            |p| unsafe { shape.init(p) },
            // SAFETY: an attacher hands the reader a mapping of at least a header.
            |p| unsafe { u64::from((*(p as *const CacheHeader)).magic) },
            u64::from(CACHE_MAGIC),
        )?;
        shape.check(mmap.as_ptr())?;
        Ok(Self::from_parts(file, mmap, shape))
    }

    /// Attach to the cache at `base`, which must exist with this shape.
    pub(crate) fn open(base: &Path, capacity: usize, key_size: usize, value_size: usize) -> Result<Self, CacheError> {
        Self::supported()?;
        let shape = Shape::new(capacity, key_size, value_size)?;
        let file = crate::region_file::open_existing(&Self::path(base))?;
        let len = file.metadata()?.len();
        let header = size_of::<CacheHeader>();
        if len < shape.total as u64 {
            // A file of another shape is named by the field it differs in,
            // whenever it holds a whole header to read the field from.
            if len >= header as u64 {
                // SAFETY: the file is at least a header long, checked above.
                let head = unsafe { MmapOptions::new().len(header).map(&file)? };
                shape.check(head.as_ptr())?;
            }
            return Err(CacheError::Mismatch(Mismatch::new("length", len, shape.total as u64)));
        }
        // SAFETY: the file is at least `shape.total` bytes, checked above.
        let mmap = unsafe { MmapOptions::new().len(shape.total).map_mut(&file)? };
        shape.check(mmap.as_ptr())?;
        Ok(Self::from_parts(file, mmap, shape))
    }

    pub(crate) fn capacity(&self) -> usize {
        self.shape.capacity
    }

    pub(crate) fn key_size(&self) -> usize {
        self.shape.key_size
    }

    pub(crate) fn value_size(&self) -> usize {
        self.shape.value_size
    }

    /// Entries in the index, counting a second entry a racing insert holds
    /// for a moment, and none while the count is below zero.
    #[inline]
    pub(crate) fn len(&self) -> usize {
        (self.header().len.load(Ordering::Acquire) as i64).max(0) as usize
    }

    pub(crate) fn flush(&self) -> io::Result<()> {
        self.mmap.flush()
    }

    pub(crate) fn flush_async(&self) -> io::Result<()> {
        self.mmap.flush_async()
    }

    #[inline]
    fn header(&self) -> &CacheHeader {
        // SAFETY: the mapping starts with a header.
        unsafe { &*(self.base as *const CacheHeader) }
    }

    #[inline]
    fn bucket(&self, b: usize) -> &[AtomicU64; 8] {
        debug_assert!(b < self.shape.bucket_count);
        // SAFETY: the buckets region holds `bucket_count` aligned 64-byte buckets.
        unsafe { &*(self.base.add(self.shape.buckets_at + b * 64) as *const [AtomicU64; 8]) }
    }

    #[inline]
    fn frame_at(&self, f: usize) -> *mut u8 {
        debug_assert!(f < self.shape.frame_count);
        // SAFETY: the frames region holds `frame_count` frames of `frame_stride`.
        unsafe { self.base.add(self.shape.frames_at + f * self.shape.frame_stride) }
    }

    #[inline]
    fn word(&self, f: usize) -> &AtomicU64 {
        // SAFETY: a frame starts with its aligned word.
        unsafe { &*(self.frame_at(f) as *const AtomicU64) }
    }

    /// Frame `f`'s identity word: the holder word, pid and start stamp, of
    /// the process the frame's last claim was for.
    #[inline]
    fn writer_word(&self, f: usize) -> &AtomicU64 {
        // SAFETY: the identity word lies WRITER_AT bytes into the frame, aligned.
        unsafe { &*(self.frame_at(f).add(WRITER_AT) as *const AtomicU64) }
    }

    /// Frame `f`'s word and identity word, as one pair.
    #[inline]
    fn pair_at(&self, f: usize) -> &AtomicPair {
        // SAFETY: frames start 16-byte aligned, their strides multiples of 16,
        // with the word and the identity word, which are read and swapped as
        // one pair or a half at a time; a cache opens only where a 16-byte
        // swap exists.
        unsafe { AtomicPair::at(self.frame_at(f)) }
    }

    /// Whether this process filled frame `f`, whose word read `w`: the word
    /// names this pid and the identity word this process's start, so a
    /// frame an earlier process with the same pid left is not this one's.
    #[inline]
    fn written_here(&self, f: usize, w: u64) -> bool {
        writer_of(w) == current_pid() && self.writer_word(f).load(Ordering::Relaxed) == process_identity::current()
    }

    /// Whether the process that filled frame `f`, whose word read `w`, is
    /// gone: its pid names no process, or names one that started after it.
    /// The word and the identity word read as one; a frame that moved on
    /// since `w`, or that this process filled, is not.
    fn writer_gone(&self, f: usize, w: u64) -> bool {
        if writer_of(w) == 0 {
            return false;
        }
        let pair = self.pair_at(f).load();
        let writer = (pair >> 64) as u64;
        pair as u64 == w && writer != process_identity::current() && process_identity::holder_gone(writer)
    }

    #[inline]
    fn hash_word(&self, f: usize) -> &AtomicU64 {
        // SAFETY: the hash lies HASH_AT bytes into the frame, aligned.
        unsafe { &*(self.frame_at(f).add(HASH_AT) as *const AtomicU64) }
    }

    #[inline]
    fn next(&self, f: usize) -> &AtomicU32 {
        // SAFETY: the link lies NEXT_AT bytes into the frame, aligned.
        unsafe { &*(self.frame_at(f).add(NEXT_AT) as *const AtomicU32) }
    }

    #[inline]
    fn mark(&self, f: usize) -> &AtomicU32 {
        // SAFETY: the mark lies MARK_AT bytes into the frame, aligned.
        unsafe { &*(self.frame_at(f).add(MARK_AT) as *const AtomicU32) }
    }

    #[inline]
    fn key_at(&self, f: usize) -> *mut u8 {
        // SAFETY: the key lies KEY_AT bytes into the frame.
        unsafe { self.frame_at(f).add(KEY_AT) }
    }

    #[inline]
    fn value_at(&self, f: usize) -> *mut u8 {
        // SAFETY: the value follows the key's stride.
        unsafe { self.key_at(f).add(self.shape.key_stride) }
    }

    #[inline]
    fn bucket_of(&self, h: u64) -> usize {
        h as usize & self.bucket_mask
    }

    /// Whether the frame an entry word names holds that entry still: live,
    /// in the generation the entry was made for.
    #[inline]
    fn frame_for(&self, e: u64) -> Option<(usize, u64)> {
        let f = frame_of(e).filter(|&f| f < self.shape.frame_count)?;
        let w = self.word(f).load(Ordering::Acquire);
        (w & STATE_MASK == LIVE && generation_of(w) & 0xFF == tag_of(e)).then_some((f, w))
    }

    /// Where `key`, of hash `h`, has a committed entry, skipping the bucket
    /// word `skip`; the first in bucket order.
    #[inline]
    fn find_from<K: Key>(&self, h: u64, key: K, skip: Option<usize>) -> Option<Found> {
        let bucket = self.bucket(self.bucket_of(h));
        let fp = fingerprint(h);
        'slots: for (slot, word) in bucket[..WAYS].iter().enumerate() {
            if Some(slot) == skip {
                continue;
            }
            let mut e = word.load(Ordering::Acquire);
            loop {
                if e == 0 || fp_of(e) != fp || e & ENTRY_STATE != 0 {
                    continue 'slots;
                }
                if let Some((f, w)) = self.frame_for(e) {
                    // SAFETY: a frame holds `key_stride` key bytes, eight-byte
                    // aligned, at KEY_AT; a key a writer is replacing reads as
                    // some bytes, which the word checked next discards.
                    let same = unsafe { key.same(self.key_at(f)) };
                    fence(Ordering::Acquire);
                    if self.word(f).load(Ordering::Relaxed) == w {
                        if same {
                            return Some(Found { slot, entry: e, frame: f, word: w });
                        }
                        continue 'slots;
                    }
                }
                // The frame moved under the entry, so the entry was taken out
                // or replaced after it was read: read it again.
                let again = word.load(Ordering::Acquire);
                if again == e {
                    continue 'slots;
                }
                e = again;
            }
        }
        None
    }

    /// Copy the value of the frame `found` names into `out`; false when the
    /// frame was freed during the copy.
    #[inline]
    fn take_value(&self, found: &Found, out: &mut [u8]) -> bool {
        // SAFETY: a frame holds the value's stride, eight-byte aligned, after
        // the key, and `out` holds `value_size` bytes.
        unsafe { take_key(self.value_at(found.frame), out) };
        fence(Ordering::Acquire);
        self.word(found.frame).load(Ordering::Relaxed) == found.word
    }

    /// Copy `key`'s value into `out`, leaving its mark; whether the cache
    /// holds it.
    #[inline]
    pub(crate) fn get(&self, key: &[u8], out: &mut [u8]) -> bool {
        let h = hash_key(key);
        loop {
            let Some(found) = self.find_from(h, Bytes(key), None) else { return false };
            if self.take_value(&found, out) {
                return true;
            }
        }
    }

    #[inline]
    pub(crate) fn contains(&self, key: &[u8]) -> bool {
        self.find_from(hash_key(key), Bytes(key), None).is_some()
    }

    /// Mark `key`, and copy its value into `out` when given one; whether the
    /// cache holds it.
    #[inline]
    pub(crate) fn touch(&self, key: &[u8], out: Option<&mut [u8]>) -> bool {
        let h = hash_key(key);
        let found = match out {
            Some(out) => loop {
                let Some(found) = self.find_from(h, Bytes(key), None) else { return false };
                if self.take_value(&found, out) {
                    break found;
                }
            },
            None => match self.find_from(h, Bytes(key), None) {
                Some(found) => found,
                None => return false,
            },
        };
        self.mark_used(found.frame);
        true
    }

    /// Set frame `f`'s mark. A frame already marked costs no write; a frame
    /// refilled since its entry was found takes the mark for its new entry,
    /// which costs that entry one more pass of the hand at most.
    #[inline]
    fn mark_used(&self, f: usize) {
        let mark = self.mark(f);
        if mark.load(Ordering::Relaxed) == 0 {
            mark.store(1, Ordering::Relaxed);
        }
    }

    /// Store `value` under `key`. True when the key was present, its value
    /// then copied into `old`, when given, before it was replaced.
    pub(crate) fn put(&self, key: &[u8], value: &[u8], old: Option<&mut [u8]>) -> Result<bool, CacheError> {
        if (1..=8).contains(&key.len()) && (1..=8).contains(&value.len()) {
            self.put_as(Word::new(key), Word::new(value), old)
        } else {
            self.put_as(Bytes(key), Bytes(value), old)
        }
    }

    /// [`put`](Self::put) of the key `form` and the value `value`, in
    /// [`Key`] forms. The key is hashed as [`hash_key`] hashes its bytes,
    /// which is how every lookup hashes it.
    fn put_as<K: Key, V: Key>(&self, form: K, value: V, mut old: Option<&mut [u8]>) -> Result<bool, CacheError> {
        let h = form.key_hash();
        let fp = fingerprint(h);
        let b = self.bucket_of(h);
        let bucket = self.bucket(b);
        // The frame holding the new value: filled once, kept across retries.
        let mut mine: Option<(usize, u32)> = None;
        // Whether this put took an entry out to make room without counting
        // it gone: its own entry then stands in that one's place in `len`.
        let mut evicted = false;
        loop {
            let looked = self.find_from(h, form, None);
            note!(self, CacheLooked, looked.as_ref().map_or(usize::MAX, |f| b * 8 + f.slot), form form, 0, 0);
            if let Some(found) = looked {
                if let Some(old) = old.as_deref_mut()
                    && !self.take_value(&found, old)
                {
                    continue;
                }
                let (f, g) = match mine {
                    Some(m) => m,
                    None => *mine.insert(self.fill_or_settle(h, form, value, evicted)?),
                };
                if bucket[found.slot].compare_exchange(found.entry, entry_word(fp, g, f), Ordering::AcqRel, Ordering::Acquire).is_ok() {
                    note!(self, CacheReplaced, b * 8 + found.slot, form form, found.entry, 0);
                    self.free_frame(found.frame, found.word);
                    if evicted {
                        self.shrink_len();
                    }
                    self.drop_others(h, form, found.slot);
                    return Ok(true);
                }
                continue;
            }
            if mine.is_none()
                && self.len() >= self.shape.capacity
                && let Some((f, w)) = self.take_next(None)
            {
                evicted = true;
                // The frame is this put's now: it is filled again rather than
                // freed and another taken.
                if let Some(g) = self.claim(f, w) {
                    mine = Some(self.write_frame(f, g, h, form, value));
                }
                // Puts racing at a full cache can each have inserted without
                // evicting; the excess comes out so the cache returns to its
                // capacity.
                while self.len() > self.shape.capacity {
                    let Some((f, w)) = self.take_next(None) else { break };
                    self.free_frame(f, w);
                    self.shrink_len();
                }
            }
            let (f, g) = match mine {
                Some(m) => m,
                None => *mine.insert(self.fill_or_settle(h, form, value, evicted)?),
            };
            let Some(slot) = bucket.iter().position(|word| word.load(Ordering::Acquire) == 0) else {
                self.evict_in_bucket(b);
                continue;
            };
            let pending = entry_word(fp, g, f) | ENTRY_PENDING;
            if bucket[slot].compare_exchange(0, pending, Ordering::SeqCst, Ordering::Relaxed).is_err() {
                continue;
            }
            note!(self, CacheEntered, b * 8 + slot, form form, pending, 0);
            if self.settle(b, slot, pending, f, frame_word(g, current_pid(), LIVE)) {
                if !evicted {
                    self.header().len.fetch_add(1, Ordering::AcqRel);
                }
                return Ok(false);
            }
            // Another entry of the key holds the insert: this one comes out,
            // and the put becomes an update of that one with the same frame,
            // in its next generation.
            Self::withdraw(&bucket[slot], pending);
            note!(self, CacheWithdrew, b * 8 + slot, form form, pending, 0);
            let g = self.step_frame(f, g);
            note!(self, CacheStepped, b * 8 + slot, form form, frame_word(g, current_pid(), LIVE), 0);
            mine = Some((f, g));
        }
    }

    /// Settle the pending entry `mine`, in word `slot` of bucket `b`, against
    /// the other entries of its key, the key in frame `own`, whose word reads
    /// `live` while it holds the entry: true when it is committed, by this
    /// call or another; false when it never will be, since a committed entry
    /// of the key is there or a pending one wins. Its writer settles it, and
    /// so does a put making room in a bucket of pending entries; the answer
    /// is exact for the writer.
    ///
    /// Of the key's pending entries, the one in the earliest word wins, and
    /// every other is given up. A settle whose entry does not win gives it up
    /// first, so no writer commits it from then on, and then commits the
    /// winner for its writer, so no put waits on another. Every load and swap
    /// here is in one order with every publish, and every put reads the
    /// bucket after it publishes, so of two puts of the key one sees the
    /// other; a swap that finds its word changed reads the bucket again. The
    /// read is [`fingerprint_mask`]'s pass over the bucket and then a load of
    /// each word it names.
    ///
    /// The frame `own` changes only once the entry has left its word for
    /// good: committed and taken out; given up and taken back, which marks
    /// the frame filling in its generation first; or given up and withdrawn
    /// by its writer, which steps the frame on; or when its writer is gone
    /// and the hand frees it. So the key read from the frame holds while its
    /// word reads `live`, and for a living writer a word holding none of the
    /// entry's pending, committed or given-up forms, beside a frame not so
    /// marked, means the entry committed and was taken out since.
    fn settle(&self, b: usize, slot: usize, mine: u64, own: usize, live: u64) -> bool {
        let bucket = self.bucket(b);
        let fp = fp_of(mine);
        let committed = mine & !ENTRY_STATE;
        let given_up = committed | ENTRY_GIVEN_UP;
        let taken_back = (live & !STATE_MASK) | FILLING;
        // What a swap of the entry's word found says of it: true when another
        // writer committed it.
        let landed = |now: u64| now != mine && now != given_up && self.word(own).load(Ordering::Acquire) != taken_back;
        'read: loop {
            // The key's pending entries in the other words, and the earliest
            // word holding one, the settled entry's own included.
            let mut pending = [0u64; WAYS];
            // Whether `pending` holds any entry, so a key with no other
            // pending entry skips the give-up pass.
            let mut others = false;
            let mut winner = slot;
            let mut beaten = false;
            // The other words whose fingerprint matches, lowest first; each
            // is loaded again here, in the one order with every publish.
            let mut candidates = fingerprint_mask(bucket, fp) & !(1 << slot);
            while candidates != 0 {
                let i = candidates.trailing_zeros() as usize;
                candidates &= candidates - 1;
                let e = bucket[i].load(Ordering::SeqCst);
                if e == 0 || fp_of(e) != fp || e & ENTRY_STATE == ENTRY_GIVEN_UP {
                    continue;
                }
                match self.entry_holds(e, own) {
                    Some(true) => {}
                    Some(false) => continue,
                    None => continue 'read,
                }
                if e & ENTRY_STATE == 0 {
                    note!(self, CacheBeaten, b * 8 + i, frame own, e, 0);
                    beaten = true;
                    break;
                }
                pending[i] = e;
                others = true;
                winner = winner.min(i);
            }
            // The comparisons read the key in `own`, which held if the
            // frame's word still reads `live`.
            fence(Ordering::Acquire);
            if self.word(own).load(Ordering::Relaxed) != live {
                return landed(bucket[slot].load(Ordering::SeqCst));
            }
            if beaten || winner != slot {
                if let Err(now) = bucket[slot].compare_exchange(mine, given_up, Ordering::SeqCst, Ordering::Acquire)
                    && landed(now)
                {
                    note!(self, CacheCommitted, b * 8 + slot, frame own, now, 1);
                    return true;
                }
                note!(self, CacheGaveOwnUp, b * 8 + slot, frame own, mine, 0);
                if beaten {
                    return false;
                }
            }
            if others {
                for (i, &e) in pending.iter().enumerate() {
                    if e == 0 || i == winner {
                        continue;
                    }
                    if bucket[i].compare_exchange(e, (e & !ENTRY_STATE) | ENTRY_GIVEN_UP, Ordering::SeqCst, Ordering::Relaxed).is_err() {
                        continue 'read;
                    }
                    note!(self, CacheGaveUp, b * 8 + i, frame own, e, 0);
                }
            }
            if winner != slot {
                let e = pending[winner];
                if bucket[winner].compare_exchange(e, e & !ENTRY_STATE, Ordering::SeqCst, Ordering::Relaxed).is_ok() {
                    note!(self, CacheHelped, b * 8 + winner, frame own, e, 0);
                }
                return false;
            }
            return match bucket[slot].compare_exchange(mine, committed, Ordering::SeqCst, Ordering::Acquire) {
                Ok(_) => {
                    note!(self, CacheCommitted, b * 8 + slot, frame own, mine, 0);
                    true
                }
                Err(now) => {
                    let landed = landed(now);
                    if landed {
                        note!(self, CacheCommitted, b * 8 + slot, frame own, now, 1);
                    }
                    landed
                }
            };
        }
    }

    /// Whether the entry `e`, read from a bucket in any state, names a live
    /// frame of its generation holding the key in frame `own`; `None` when
    /// the entry's frame changed during the comparison.
    fn entry_holds(&self, e: u64, own: usize) -> Option<bool> {
        let Some((f, w)) = self.frame_for(e) else { return Some(false) };
        let same = self.same_frame_key(f, own);
        fence(Ordering::Acquire);
        (self.word(f).load(Ordering::Relaxed) == w).then_some(same)
    }

    /// Whether frames `f` and `g` hold the same key: the key's stride
    /// compared a word at a time, the bytes past the key zero in both. A
    /// frame refilled meanwhile reads as some bytes, which the caller's check
    /// of that frame's word discards.
    fn same_frame_key(&self, f: usize, g: usize) -> bool {
        let (a, b) = (self.key_at(f) as *const u64, self.key_at(g) as *const u64);
        (0..self.shape.key_stride >> 3).all(|i| {
            // SAFETY: a frame holds `key_stride` key bytes at KEY_AT, whole
            // eight-byte-aligned words.
            unsafe { crate::atomic_copy::load_word(a.add(i)) == crate::atomic_copy::load_word(b.add(i)) }
        })
    }

    /// Move frame `f`, which this put filled as generation `g` and whose
    /// entry lost, on to the next generation, live, and return that
    /// generation. A put taking the lost entry back may have marked the frame
    /// filling in generation `g`; the step clears the mark, and no word names
    /// the frame in generation `g` again.
    fn step_frame(&self, f: usize, g: u32) -> u32 {
        let next = g.wrapping_add(1) & GENERATION_MASK;
        self.word(f).store(frame_word(next, current_pid(), LIVE), Ordering::Release);
        next
    }

    /// Take this put's entry out of `word`, where it published it as
    /// `mine`, pending or given up since; a word another writer has already
    /// taken back is left alone.
    fn withdraw(word: &AtomicU64, mine: u64) {
        let given_up = (mine & !ENTRY_STATE) | ENTRY_GIVEN_UP;
        for held in [mine, given_up] {
            if word.compare_exchange(held, 0, Ordering::AcqRel, Ordering::Relaxed).is_ok() {
                return;
            }
        }
    }

    /// A frame filled with `key`, `value` and `h`, as `fill` gives one. When none can be had, an entry this put already evicted
    /// without counting it gone is counted gone before the error goes back.
    fn fill_or_settle<K: Key, V: Key>(&self, h: u64, key: K, value: V, evicted: bool) -> Result<(usize, u32), CacheError> {
        self.fill_as(h, key, value).inspect_err(|_| {
            if evicted {
                self.shrink_len();
            }
        })
    }

    /// Take `key`'s entry out and copy its value into `out`; whether the cache
    /// held it. Any other entry the key has goes with it.
    pub(crate) fn remove(&self, key: &[u8], out: &mut [u8]) -> bool {
        let h = hash_key(key);
        let bucket = self.bucket(self.bucket_of(h));
        loop {
            let Some(found) = self.find_from(h, Bytes(key), None) else { return false };
            if !self.take_value(&found, out) {
                continue;
            }
            if bucket[found.slot].compare_exchange(found.entry, 0, Ordering::AcqRel, Ordering::Acquire).is_ok() {
                self.free_frame(found.frame, found.word);
                self.shrink_len();
                self.drop_others(h, Bytes(key), found.slot);
                return true;
            }
        }
    }

    /// Evict the entry the hand comes to first unmarked, copying its key and
    /// value out; false when the cache is empty.
    pub(crate) fn evict(&self, key_out: &mut [u8], value_out: &mut [u8]) -> bool {
        let Some((f, w)) = self.take_next(Some((key_out, value_out))) else { return false };
        self.free_frame(f, w);
        self.shrink_len();
        true
    }

    /// A frame claimed for this process and filled with `key`, `value` and
    /// `h`, marked: the frame and its generation.
    fn fill_as<K: Key, V: Key>(&self, h: u64, key: K, value: V) -> Result<(usize, u32), CacheError> {
        let (f, g) = self.take_frame()?;
        Ok(self.write_frame(f, g, h, key, value))
    }

    /// [`fill_as`](Self::fill_as) of a key and value given as bytes.
    #[cfg(test)]
    fn fill(&self, h: u64, key: &[u8], value: &[u8]) -> Result<(usize, u32), CacheError> {
        self.fill_as(h, Bytes(key), Bytes(value))
    }

    /// A frame claimed for this process, being filled: one from the free
    /// list, else one the hand takes out of the index. Its generation.
    fn take_frame(&self) -> Result<(usize, u32), CacheError> {
        loop {
            if let Some(f) = self.pop_free() {
                let g = generation_of(self.word(f).load(Ordering::Acquire));
                // A frame off the free list is this caller's alone, so the claim
                // needs no swap. The identity word goes first, so a word read
                // filling has this process beside it.
                self.writer_word(f).store(process_identity::current(), Ordering::Relaxed);
                self.word(f).store(frame_word(g, current_pid(), FILLING), Ordering::Release);
                return Ok((f, g));
            }
            let Some((f, w)) = self.take_next(None) else { return Err(CacheError::Full) };
            self.shrink_len();
            if let Some(g) = self.claim(f, w) {
                return Ok((f, g));
            }
            // The hand freed it first, its writer being gone: it is on the
            // free list for the next pass.
        }
    }

    /// Claim frame `f`, which its caller took out of the index at word `w`,
    /// for this process to fill as the next generation, which this returns.
    /// None when the hand freed it first, its writer being gone.
    fn claim(&self, f: usize, w: u64) -> Option<u32> {
        let g = generation_of(w).wrapping_add(1) & GENERATION_MASK;
        let claimed = frame_word(g, current_pid(), FILLING);
        if self.written_here(f, w) {
            // The hand frees a frame out of the index only when its writer is
            // gone, and this process wrote it.
            self.word(f).store(claimed, Ordering::Release);
            return Some(g);
        }
        // One swap of the pair claims the frame and records this process
        // beside the word; one that finds the word unchanged beside another
        // identity tries again.
        let pair = self.pair_at(f);
        let mine = u128::from(claimed) | (u128::from(process_identity::current()) << 64);
        let mut seen = u128::from(w) | (u128::from(self.writer_word(f).load(Ordering::Relaxed)) << 64);
        loop {
            match pair.compare_exchange(seen, mine) {
                Ok(_was) => return Some(g),
                Err(found) if found as u64 == w => seen = found,
                Err(_freed) => return None,
            }
        }
    }

    /// Fill frame `f`, which this process claimed as generation `g`, with
    /// `key`, `value` and `h`, marked, and make it live.
    fn write_frame<K: Key, V: Key>(&self, f: usize, g: u32, h: u64, key: K, value: V) -> (usize, u32) {
        // The claim before the stores below: a reader copying the frame's last
        // entry checks the word after.
        fence(Ordering::Release);
        // SAFETY: this writer claimed the frame, so nobody else writes it; the
        // frame holds the key's and the value's strides, eight-byte aligned.
        unsafe {
            key.put(self.key_at(f));
            value.put(self.value_at(f));
        }
        self.hash_word(f).store(h, Ordering::Relaxed);
        self.mark(f).store(1, Ordering::Relaxed);
        self.word(f).store(frame_word(g, current_pid(), LIVE), Ordering::Release);
        (f, g)
    }

    fn pop_free(&self) -> Option<usize> {
        let free = &self.header().free;
        let mut head = free.load(Ordering::Acquire);
        loop {
            let top = (head & 0xFFFF_FFFF) as usize;
            let f = top.checked_sub(1).filter(|&f| f < self.shape.frame_count)?;
            let below = u64::from(self.next(f).load(Ordering::Acquire));
            let next = ((head >> 32).wrapping_add(1) << 32) | below;
            match free.compare_exchange_weak(head, next, Ordering::AcqRel, Ordering::Acquire) {
                Ok(_) => {
                    self.note_used(f);
                    return Some(f);
                }
                Err(now) => head = now,
            }
        }
    }

    /// Raise the high-water mark past frame `f`, when `f` is at or above it:
    /// only a frame's first trip off the free list writes.
    #[inline]
    fn note_used(&self, f: usize) {
        let used = &self.header().used;
        let past = f as u64 + 1;
        if past > used.load(Ordering::Relaxed) {
            used.fetch_max(past, Ordering::Release);
        }
    }

    /// How many frames the hand walks: those below the high-water mark, or
    /// every frame in a file whose mark reads 0.
    #[inline]
    fn used_frames(&self) -> usize {
        let used = self.header().used.load(Ordering::Acquire) as usize;
        if used == 0 { self.shape.frame_count } else { used.min(self.shape.frame_count) }
    }

    fn push_free(&self, f: usize) {
        let free = &self.header().free;
        let mut head = free.load(Ordering::Acquire);
        loop {
            self.next(f).store((head & 0xFFFF_FFFF) as u32, Ordering::Relaxed);
            let next = ((head >> 32).wrapping_add(1) << 32) | (f as u64 + 1);
            match free.compare_exchange_weak(head, next, Ordering::AcqRel, Ordering::Acquire) {
                Ok(_) => return,
                Err(now) => head = now,
            }
        }
    }

    /// Free frame `f`, at word `w`: it steps to the next generation and goes
    /// on the free list. Whoever took the frame's entry out of the index
    /// frees it, as does the hand for a frame whose writer is gone; the swap
    /// lets only one of them.
    fn free_frame(&self, f: usize, w: u64) {
        let freed = frame_word(generation_of(w).wrapping_add(1), 0, FREE);
        if self.written_here(f, w) {
            // The hand frees only a frame whose writer is gone, and this
            // process wrote it, so the caller owns it.
            self.word(f).store(freed, Ordering::Release);
            self.push_free(f);
        } else if self.word(f).compare_exchange(w, freed, Ordering::AcqRel, Ordering::Relaxed).is_ok() {
            self.push_free(f);
        }
    }

    /// One fewer entry. An entry taken out before the put that inserted it
    /// counts it is counted gone first, so the count can stand below zero for
    /// that moment; a writer that died between committing an entry and
    /// counting it leaves the count one short.
    fn shrink_len(&self) {
        self.header().len.fetch_sub(1, Ordering::AcqRel);
    }

    /// Take out every entry for `key` but the one in bucket word `keep`.
    fn drop_others<K: Key>(&self, h: u64, key: K, keep: usize) {
        let bucket = self.bucket(self.bucket_of(h));
        while let Some(other) = self.find_from(h, key, Some(keep)) {
            if bucket[other.slot].compare_exchange(other.entry, 0, Ordering::AcqRel, Ordering::Acquire).is_ok() {
                self.free_frame(other.frame, other.word);
                self.shrink_len();
            }
        }
    }

    /// Make room in bucket `b`, whose words all hold entries, or find that
    /// another writer made it. A word whose entry names a frame that has
    /// moved on is taken back first, then a given-up entry's word, by
    /// [`take_back`](Self::take_back); else an unmarked committed entry is
    /// evicted, else one whose mark this cleared. A bucket left holding only
    /// pending entries has its first one settled for its writer, which
    /// commits it or gives it up, so the next call evicts it or takes it
    /// back.
    fn evict_in_bucket(&self, b: usize) {
        let bucket = self.bucket(b);
        for word in bucket {
            let e = word.load(Ordering::SeqCst);
            if e == 0 {
                return;
            }
            match self.frame_for(e) {
                None => {
                    if word.compare_exchange(e, 0, Ordering::SeqCst, Ordering::Relaxed).is_ok() {
                        note!(self, CacheTookBack, self.trace_slot(b, word), &[], e, 0);
                        return;
                    }
                }
                Some((f, w)) => {
                    if e & ENTRY_STATE == ENTRY_GIVEN_UP && self.take_back(word, e, f, w) {
                        note!(self, CacheTookBack, self.trace_slot(b, word), frame f, e, 1);
                        return;
                    }
                }
            }
        }
        for pass in 0..2 {
            for word in bucket {
                let e = word.load(Ordering::Acquire);
                if e == 0 {
                    return;
                }
                if e & ENTRY_STATE != 0 {
                    continue;
                }
                let Some((f, w)) = self.frame_for(e) else { continue };
                let mark = self.mark(f);
                if pass == 0 && mark.load(Ordering::Relaxed) != 0 {
                    mark.store(0, Ordering::Relaxed);
                    continue;
                }
                if word.compare_exchange(e, 0, Ordering::AcqRel, Ordering::Acquire).is_ok() {
                    note!(self, CacheEvicted, self.trace_slot(b, word), frame f, e, 0);
                    self.free_frame(f, w);
                    self.shrink_len();
                    return;
                }
            }
        }
        for (i, word) in bucket.iter().enumerate() {
            let e = word.load(Ordering::SeqCst);
            if e == 0 {
                return;
            }
            if e & ENTRY_STATE != ENTRY_PENDING {
                continue;
            }
            if let Some((f, w)) = self.frame_for(e) {
                self.settle(b, i, e, f, w);
                note!(self, CacheSettledFor, b * 8 + i, frame f, e, word.load(Ordering::SeqCst));
            }
            return;
        }
    }

    /// Take back the given-up entry `e` in `word`, whose frame `f` reads `w`,
    /// live: the frame is marked filling in its generation first, so the
    /// entry's writer, if it still runs, knows by the mark that its entry
    /// never committed. True when this took the word back. A mark stays when
    /// another put takes the word back first or the writer withdraws it,
    /// until the writer steps the frame on or the hand frees a dead
    /// writer's.
    fn take_back(&self, word: &AtomicU64, e: u64, f: usize, w: u64) -> bool {
        let marked = (w & !STATE_MASK) | FILLING;
        if let Err(now) = self.word(f).compare_exchange(w, marked, Ordering::SeqCst, Ordering::SeqCst)
            && now != marked
        {
            return false;
        }
        word.compare_exchange(e, 0, Ordering::SeqCst, Ordering::Relaxed).is_ok()
    }

    /// Scan on from the hand until an entry comes out of the index, copying
    /// its key and value out first when asked to: its frame, now the
    /// caller's to fill or free, and the frame's word. The scan reads the
    /// frames with plain loads, clearing each mark it passes, and moves the
    /// hand to the frame after the last it scanned once, at the end. None
    /// when the cache holds no entry the hand can take.
    fn take_next(&self, mut out: Option<(&mut [u8], &mut [u8])>) -> Option<(usize, u64)> {
        // The scan walks the frames below the high-water mark, read again
        // each time the hand wraps, so a frame first used during the scan
        // joins it from the next wrap.
        let mut n = self.used_frames();
        let hand = &self.header().hand;
        // The scan steps and wraps the frame by compare, and asks whether the
        // cache emptied once per lap.
        let mut next = self.hand_frame(n);
        let mut passed = 0usize;
        let mut lap_left = 0usize;
        let mut taken = None;
        while passed < 3 * n {
            if lap_left == 0 {
                if self.len() == 0 {
                    break;
                }
                lap_left = n;
            }
            lap_left -= 1;
            let f = next;
            next += 1;
            if next >= n {
                next = 0;
                n = self.used_frames();
            }
            passed += 1;
            let w = self.word(f).load(Ordering::Acquire);
            match w & STATE_MASK {
                LIVE => {
                    let mark = self.mark(f);
                    if passed <= 2 * n && mark.load(Ordering::Relaxed) != 0 {
                        mark.store(0, Ordering::Relaxed);
                    } else if self.take_out(f, w, out.as_mut().map(|(k, v)| (&mut **k, &mut **v))) {
                        taken = Some((f, w));
                        break;
                    } else if self.writer_gone(f, w) {
                        // Named by no committed entry, its writer gone: it
                        // died before its entry committed.
                        self.free_frame(f, w);
                    }
                }
                FILLING if self.writer_gone(f, w) => self.free_frame(f, w),
                _ => {}
            }
        }
        // A store, not a swap: an evictor racing this one can move the hand
        // back over frames this scan passed, and a frame passed twice is
        // evicted a pass early, which CLOCK's order tolerates.
        hand.store(next as u64, Ordering::Relaxed);
        taken
    }

    /// The frame the hand names, or the first when the hand is at or past
    /// `n`, the frames walked: past the frames in use, or past the last
    /// frame, which only a damaged file holds.
    #[inline]
    fn hand_frame(&self, n: usize) -> usize {
        let at = self.header().hand.load(Ordering::Relaxed);
        if at < n as u64 { at as usize } else { 0 }
    }

    /// Take frame `f`'s entry out of the index, `w` being the frame's word as
    /// the hand read it, copying its key and value out first when asked to.
    /// True when this call took it out.
    fn take_out(&self, f: usize, w: u64, out: Option<(&mut [u8], &mut [u8])>) -> bool {
        let h = self.hash_word(f).load(Ordering::Acquire);
        if let Some((key_out, value_out)) = out
            && !self.copy_out(f, w, key_out, value_out)
        {
            return false;
        }
        // A hash read from a frame refilled since `w` names an entry of
        // another generation, which no bucket word matches.
        let want = entry_word(fingerprint(h), generation_of(w), f);
        let bucket = self.bucket(self.bucket_of(h));
        bucket[..WAYS]
            .iter()
            .any(|word| word.load(Ordering::Acquire) == want && word.compare_exchange(want, 0, Ordering::AcqRel, Ordering::Acquire).is_ok())
    }

    /// Copy frame `f`'s key and value into `key_out` and `value_out`: false
    /// when the frame left word `w` during the copy. Out of line, so a hand
    /// that copies nothing out carries none of the copy's setup.
    #[inline(never)]
    fn copy_out(&self, f: usize, w: u64, key_out: &mut [u8], value_out: &mut [u8]) -> bool {
        // SAFETY: a frame holds its key's and value's strides, eight-byte
        // aligned, and the outputs hold the declared sizes.
        unsafe {
            take_key(self.key_at(f), key_out);
            take_key(self.value_at(f), value_out);
        }
        fence(Ordering::Acquire);
        self.word(f).load(Ordering::Relaxed) == w
    }

    /// Calls `f` with each entry's key and value in the order eviction would
    /// take them: from the hand on, the unmarked entries, then the marked
    /// ones. Entries change under a running walk.
    pub(crate) fn for_each_in_eviction_order(&self, mut f: impl FnMut(&[u8], &[u8])) {
        let n = self.shape.frame_count;
        let start = self.hand_frame(n);
        let mut key = vec![0u8; self.shape.key_size];
        let mut value = vec![0u8; self.shape.value_size];
        for marked in [false, true] {
            for frame in (start..n).chain(0..start) {
                let w = self.word(frame).load(Ordering::Acquire);
                if w & STATE_MASK != LIVE || (self.mark(frame).load(Ordering::Relaxed) != 0) != marked {
                    continue;
                }
                let h = self.hash_word(frame).load(Ordering::Acquire);
                // SAFETY: as in `copy_out`.
                unsafe {
                    take_key(self.key_at(frame), &mut key);
                    take_key(self.value_at(frame), &mut value);
                }
                fence(Ordering::Acquire);
                if self.word(frame).load(Ordering::Relaxed) != w {
                    continue;
                }
                let want = entry_word(fingerprint(h), generation_of(w), frame);
                let bucket = self.bucket(self.bucket_of(h));
                if bucket[..WAYS].iter().any(|word| word.load(Ordering::Acquire) == want) {
                    f(&key, &value);
                }
            }
        }
    }

    /// The address the test-build trace knows this cache by.
    #[cfg(test)]
    pub(crate) fn trace_name(&self) -> usize {
        self.base as usize
    }

    /// The first eight bytes of the key in frame `f`, as the test-build trace
    /// names keys.
    #[cfg(test)]
    fn trace_key(&self, f: usize) -> u64 {
        // SAFETY: a frame holds at least one aligned key word at KEY_AT, the
        // bytes past a shorter key zero.
        u64::from_le(unsafe { crate::atomic_copy::load_word(self.key_at(f) as *const u64) })
    }

    /// The test-build trace's number for word `word` of bucket `b`.
    #[cfg(test)]
    fn trace_slot(&self, b: usize, word: &AtomicU64) -> usize {
        b * 8 + (std::ptr::from_ref(word) as usize - self.bucket(b).as_ptr() as usize) / 8
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    fn scratch(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!("subetha_clock_cache_{name}_{}", std::process::id()))
    }

    fn remove(base: &Path) {
        match std::fs::remove_file(CacheCore::path(base)) {
            Ok(()) => {}
            Err(e) if e.kind() == io::ErrorKind::NotFound => {}
            Err(e) => panic!("the cache file was not removed: {e}"),
        }
    }

    fn fresh(name: &str, capacity: usize) -> (CacheCore, PathBuf) {
        let base = scratch(name);
        remove(&base);
        (CacheCore::create(&base, capacity, 8, 8).expect("create"), base)
    }

    fn put(c: &CacheCore, k: u64, v: u64) -> Option<u64> {
        let mut old = [0u8; 8];
        c.put(&k.to_le_bytes(), &v.to_le_bytes(), Some(&mut old)).expect("put").then(|| u64::from_le_bytes(old))
    }

    fn get(c: &CacheCore, k: u64) -> Option<u64> {
        let mut out = [0u8; 8];
        c.get(&k.to_le_bytes(), &mut out).then(|| u64::from_le_bytes(out))
    }

    fn evict(c: &CacheCore) -> Option<(u64, u64)> {
        let (mut k, mut v) = ([0u8; 8], [0u8; 8]);
        c.evict(&mut k, &mut v).then(|| (u64::from_le_bytes(k), u64::from_le_bytes(v)))
    }

    /// The high-water mark counts the frames puts have taken off the free
    /// list, and evicting puts reuse frames below it, so the hand's walk
    /// stays short of the slack frames a full cache never fills.
    #[test]
    fn the_high_water_mark_counts_the_frames_puts_have_used() {
        let (c, base) = fresh("high-water", 10);
        assert_eq!(c.header().used.load(Ordering::Acquire), 0, "a fresh cache has used no frame");
        assert_eq!(c.used_frames(), c.shape.frame_count, "a mark of 0 walks every frame");
        for k in 0..10u64 {
            put(&c, k, k);
        }
        assert_eq!(c.header().used.load(Ordering::Acquire), 10, "ten new keys took frames 0 to 9");
        for k in 10..60u64 {
            put(&c, k, k);
            assert_eq!(get(&c, k), Some(k), "an evicting put's key is there");
        }
        let used = c.header().used.load(Ordering::Acquire) as usize;
        assert!(used < c.shape.frame_count, "evicting puts reuse frames: {used} of {} used", c.shape.frame_count);
        assert_eq!(c.used_frames(), used);
        assert_eq!(c.len(), 10);
        assert_every_frame_accounted(&c);
        drop(c);
        remove(&base);
    }

    /// The bucket pass reports exactly the words whose fingerprint matches,
    /// as testing each word alone does: empty words, matching entries, near
    /// misses, and fingerprints at both ends of their 22 bits.
    #[cfg(target_arch = "x86_64")]
    #[test]
    fn the_fingerprint_mask_names_each_word_testing_it_alone_would() {
        #[repr(C, align(64))]
        struct Line([AtomicU64; WAYS]);
        let line = Line(Default::default());
        let mut seed = 0x9E37_79B9_7F4A_7C15u64;
        let mut next = move || {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            seed
        };
        for round in 0..4096 {
            let fp = match round % 4 {
                0 => 0,
                1 => (1 << 22) - 1,
                _ => fingerprint(next()),
            };
            for word in &line.0 {
                let r = next();
                let e = match r % 4 {
                    0 => 0,
                    1 => entry_word(fp, r as u32, (r >> 40) as usize & 0xFFFF) | ((r >> 8) & ENTRY_STATE),
                    2 => entry_word(fp ^ (1 << (r % 22)), r as u32, 7),
                    _ => r,
                };
                word.store(e, Ordering::Relaxed);
            }
            let alone = (0..WAYS).filter(|&i| fp_of(line.0[i].load(Ordering::Relaxed)) == fp).fold(0u32, |m, i| m | 1 << i);
            assert_eq!(fingerprint_mask(&line.0, fp), alone, "round {round}, fingerprint {fp:#x}");
        }
    }

    /// A pid no live process has, as a writer that died would leave in a
    /// frame's word.
    const GONE: u32 = u32::MAX - 7;

    /// Leave frame `f` reading `word`, its identity word naming `writer`, as
    /// a claim for that holder leaves it.
    fn leave(c: &CacheCore, f: usize, word: u64, writer: u64) {
        c.writer_word(f).store(writer, Ordering::Release);
        c.word(f).store(word, Ordering::Release);
    }

    /// Every frame is on the free list once or holds exactly one entry in the
    /// index, and `len` counts the entries: what a cache at rest must show.
    fn assert_every_frame_accounted(c: &CacheCore) {
        let n = c.shape.frame_count;
        let mut seen = vec![false; n];
        let mut top = (c.header().free.load(Ordering::Acquire) & 0xFFFF_FFFF) as usize;
        while let Some(f) = top.checked_sub(1) {
            assert!(!seen[f], "frame {f} is on the free list twice");
            seen[f] = true;
            assert_eq!(c.word(f).load(Ordering::Acquire) & STATE_MASK, FREE, "frame {f} is listed free");
            top = c.next(f).load(Ordering::Acquire) as usize;
        }
        let mut indexed = 0;
        for b in 0..c.shape.bucket_count {
            for word in &c.bucket(b)[..WAYS] {
                let e = word.load(Ordering::Acquire);
                if e == 0 {
                    continue;
                }
                let (f, _) = c.frame_for(e).expect("an entry names a live frame of its generation");
                assert!(!seen[f], "frame {f} is free or indexed twice");
                seen[f] = true;
                indexed += 1;
            }
        }
        assert_eq!(indexed, c.len(), "len counts the entries in the index");
        let lost: Vec<usize> = (0..n).filter(|&f| !seen[f]).collect();
        assert!(lost.is_empty(), "frames neither free nor indexed: {lost:?}");
    }

    #[test]
    fn a_put_value_is_read_back_and_replaced_by_the_next() {
        let (c, base) = fresh("roundtrip", 16);
        assert_eq!(put(&c, 1, 10), None);
        assert_eq!(get(&c, 1), Some(10));
        assert_eq!(put(&c, 1, 11), Some(10), "a put of a present key answers the value it replaced");
        assert_eq!(get(&c, 1), Some(11));
        assert_eq!(c.len(), 1);
        assert_eq!(get(&c, 2), None);
        let mut out = [0u8; 8];
        assert!(c.remove(&1u64.to_le_bytes(), &mut out));
        assert_eq!(u64::from_le_bytes(out), 11);
        assert_eq!(get(&c, 1), None);
        assert_eq!(c.len(), 0);
        assert_every_frame_accounted(&c);
        drop(c);
        remove(&base);
    }

    #[test]
    fn a_full_cache_evicts_unreferenced_entries_first() {
        let (c, base) = fresh("clock", 3);
        for k in 0..3 {
            put(&c, k, k * 10);
        }
        // Every entry starts marked; the hand clears each mark once around and
        // then takes the first it reaches again.
        assert_eq!(evict(&c), Some((0, 0)));
        assert_eq!(c.len(), 2);
        assert!(c.touch(&1u64.to_le_bytes(), None));
        assert_eq!(evict(&c), Some((2, 20)), "the touched entry is passed over");
        assert_eq!(evict(&c), Some((1, 10)));
        assert_eq!(evict(&c), None);
        assert_every_frame_accounted(&c);
        drop(c);
        remove(&base);
    }

    #[test]
    fn churn_far_past_the_capacity_never_fills_the_cache() {
        let (c, base) = fresh("churn", 10);
        for k in 0..10_000u64 {
            put(&c, k, k);
            assert!(c.len() <= 10, "a put into a full cache evicts first");
        }
        for k in 9_990..10_000u64 {
            assert_eq!(get(&c, k), Some(k), "the newest entries are held");
        }
        assert_every_frame_accounted(&c);
        drop(c);
        remove(&base);
    }

    #[test]
    fn writers_putting_the_same_keys_count_each_once() {
        let (c, base) = fresh("race", 4096);
        crate::set_trace::name(&[c.trace_name()]);
        let c = Arc::new(c);
        let inserted: Vec<Vec<u64>> = std::thread::scope(|s| {
            let handles: Vec<_> = (0..8)
                .map(|t| {
                    let c = Arc::clone(&c);
                    s.spawn(move || (0..512u64).filter(|&k| put(&c, k, k * 100 + t).is_none()).collect::<Vec<u64>>())
                })
                .collect();
            handles.into_iter().map(|h| h.join().expect("a writer")).collect()
        });
        let mut inserts = [0usize; 512];
        for &k in inserted.iter().flatten() {
            inserts[k as usize] += 1;
        }
        let wrong: Vec<usize> = (0..512).filter(|&k| inserts[k] != 1).collect();
        let stories: String = wrong
            .iter()
            .map(|&k| format!("key {k} inserted {} times:\n{}", inserts[k], crate::set_trace::story(k as u64)))
            .collect();
        assert!(wrong.is_empty(), "each key is inserted once and updated after:\n{stories}");
        assert_eq!(c.len(), 512);
        for k in 0..512u64 {
            let v = get(&c, k).expect("every key is held");
            assert_eq!(v / 100, k, "a value is one some writer put for this key");
        }
        assert_every_frame_accounted(&c);
        let c = Arc::into_inner(c).expect("the writers are done");
        drop(c);
        remove(&base);
    }

    #[test]
    fn writers_evicting_at_once_lose_no_frame_and_mix_no_values() {
        let (c, base) = fresh("evicting", 64);
        std::thread::scope(|s| {
            for t in 0..8u64 {
                let c = &c;
                s.spawn(move || {
                    for i in 0..20_000u64 {
                        let k = (t << 32) | i;
                        put(c, k, k ^ 0x5A5A);
                        let earlier = (t << 32) | i.saturating_sub(3);
                        if let Some(v) = get(c, earlier) {
                            assert_eq!(v, earlier ^ 0x5A5A, "a value is read under its own key");
                        }
                    }
                });
            }
        });
        // Puts racing at the threshold can each insert without evicting; the
        // next put that evicts takes the excess out.
        assert!(c.len() <= 64 + 8, "len {} past the capacity and the writers", c.len());
        put(&c, u64::MAX, 1);
        assert!(c.len() <= 64, "len {} after a put that evicts", c.len());
        assert_every_frame_accounted(&c);
        drop(c);
        remove(&base);
    }

    #[test]
    fn a_frame_left_filling_by_a_dead_writer_is_freed_by_the_hand() {
        let (c, base) = fresh("dead", 1);
        let f = c.pop_free().expect("a free frame");
        let g = generation_of(c.word(f).load(Ordering::Acquire));
        leave(&c, f, frame_word(g, GONE, FILLING), u64::from(GONE));
        put(&c, 1, 1);
        assert_eq!(evict(&c), Some((1, 1)));
        let w = c.word(f).load(Ordering::Acquire);
        assert_eq!(w & STATE_MASK, FREE, "the hand freed the dead writer's frame");
        assert_eq!(generation_of(w), g + 1);
        assert_every_frame_accounted(&c);
        drop(c);
        remove(&base);
    }

    #[test]
    fn a_frame_left_live_in_no_bucket_by_a_dead_writer_is_freed_by_the_hand() {
        let (c, base) = fresh("orphan", 1);
        let f = c.pop_free().expect("a free frame");
        let g = generation_of(c.word(f).load(Ordering::Acquire));
        // A writer that filled the frame and died before publishing it.
        leave(&c, f, frame_word(g, GONE, LIVE), u64::from(GONE));
        put(&c, 1, 1);
        assert_eq!(evict(&c), Some((1, 1)), "the live entry is the one evicted");
        let w = c.word(f).load(Ordering::Acquire);
        assert_eq!(w & STATE_MASK, FREE, "the hand freed the frame no bucket names");
        assert_eq!(generation_of(w), g + 1);
        assert_every_frame_accounted(&c);
        drop(c);
        remove(&base);
    }

    /// A frame left filling under this process's pid with another start was
    /// left by a process that held the pid before this one: not this
    /// process's, and the hand frees it, though the pid is alive.
    #[test]
    fn a_frame_left_filling_under_a_reused_pid_is_freed_by_the_hand() {
        let (c, base) = fresh("reused", 1);
        let f = c.pop_free().expect("a free frame");
        let g = generation_of(c.word(f).load(Ordering::Acquire));
        let stamp = process_identity::holder_stamp(process_identity::current());
        assert_ne!(stamp, 0, "this process reads its own start");
        let earlier = process_identity::holder(current_pid(), stamp.wrapping_add(1).max(1));
        let word = frame_word(g, current_pid(), FILLING);
        leave(&c, f, word, earlier);
        assert!(!c.written_here(f, word), "the frame is not this process's");
        assert!(c.writer_gone(f, word), "its writer is gone");
        put(&c, 1, 1);
        let key = 1u64.to_le_bytes();
        let mine = c.find_from(hash_key(&key), Bytes(&key), None).expect("the entry");
        assert!(c.written_here(mine.frame, mine.word), "a frame this process filled is its own");
        assert!(!c.writer_gone(mine.frame, mine.word));
        assert_eq!(evict(&c), Some((1, 1)));
        let w = c.word(f).load(Ordering::Acquire);
        assert_eq!(w & STATE_MASK, FREE, "the hand freed the earlier process's frame");
        assert_eq!(generation_of(w), g + 1);
        assert_every_frame_accounted(&c);
        drop(c);
        remove(&base);
    }

    #[test]
    fn a_second_committed_entry_of_a_key_goes_with_the_next_remove() {
        let (c, base) = fresh("double", 16);
        put(&c, 5, 50);
        // A second committed entry for the key, which settling never leaves:
        // an index a damaged file carries.
        let h = hash_key(&5u64.to_le_bytes());
        let (f, g) = c.fill(h, &5u64.to_le_bytes(), &55u64.to_le_bytes()).expect("a frame");
        let bucket = c.bucket(c.bucket_of(h));
        let slot = (0..WAYS).find(|&s| bucket[s].load(Ordering::Acquire) == 0).expect("a free word");
        bucket[slot].store(entry_word(fingerprint(h), g, f), Ordering::Release);
        c.header().len.fetch_add(1, Ordering::AcqRel);
        let mut out = [0u8; 8];
        assert!(c.remove(&5u64.to_le_bytes(), &mut out));
        assert_eq!(get(&c, 5), None, "no entry for the key is left");
        assert_eq!(c.len(), 0);
        assert_every_frame_accounted(&c);
        drop(c);
        remove(&base);
    }

    /// A pending entry is read by no lookup until it is committed.
    #[test]
    fn a_pending_entry_is_read_once_it_is_committed() {
        let (c, base) = fresh("pending", 16);
        let key = 7u64.to_le_bytes();
        let h = hash_key(&key);
        let (f, g) = c.fill(h, &key, &70u64.to_le_bytes()).expect("a frame");
        let bucket = c.bucket(c.bucket_of(h));
        let committed = entry_word(fingerprint(h), g, f);
        bucket[0].store(committed | ENTRY_PENDING, Ordering::Release);
        assert_eq!(get(&c, 7), None, "a pending entry is not read");
        assert!(!c.contains(&key));
        bucket[0].store(committed, Ordering::Release);
        c.header().len.fetch_add(1, Ordering::AcqRel);
        assert_eq!(get(&c, 7), Some(70), "a committed entry is read");
        assert_every_frame_accounted(&c);
        drop(c);
        remove(&base);
    }

    /// A pending entry a writer left on dying, in the bucket's first word,
    /// wins: the next put of its key commits it for the writer and becomes
    /// an update of it.
    #[test]
    fn a_dead_writers_pending_entry_is_committed_by_the_next_put_of_its_key() {
        let (c, base) = fresh("dead-pending", 16);
        let key = 9u64.to_le_bytes();
        let h = hash_key(&key);
        let (f, g) = c.fill(h, &key, &90u64.to_le_bytes()).expect("a frame");
        leave(&c, f, frame_word(g, GONE, LIVE), u64::from(GONE));
        let bucket = c.bucket(c.bucket_of(h));
        bucket[0].store(entry_word(fingerprint(h), g, f) | ENTRY_PENDING, Ordering::Release);
        assert_eq!(put(&c, 9, 91), Some(90), "the dead writer's insert landed first");
        assert_eq!(get(&c, 9), Some(91));
        assert_eq!(c.word(f).load(Ordering::Acquire) & STATE_MASK, FREE, "the replaced entry's frame is freed");
        let live = bucket.iter().filter(|w| w.load(Ordering::Acquire) != 0).count();
        assert_eq!(live, 1, "one entry of the key, and no word left pending or given up");
        drop(c);
        remove(&base);
    }

    /// A put whose own entry lies in the earliest word gives up a pending
    /// entry of its key in a later word and commits its own.
    #[test]
    fn a_later_pending_entry_of_the_key_is_given_up() {
        let (c, base) = fresh("later", 16);
        let key = 11u64.to_le_bytes();
        let h = hash_key(&key);
        let (f, g) = c.fill(h, &key, &110u64.to_le_bytes()).expect("a frame");
        let bucket = c.bucket(c.bucket_of(h));
        let other = entry_word(fingerprint(h), g, f) | ENTRY_PENDING;
        bucket[5].store(other, Ordering::Release);
        assert_eq!(put(&c, 11, 111), None, "the put in the earlier word is the insert");
        assert_eq!(bucket[5].load(Ordering::Acquire), (other & !ENTRY_STATE) | ENTRY_GIVEN_UP, "the later entry is given up");
        assert_eq!(get(&c, 11), Some(111));
        // The given-up entry's writer takes it out and frees its frame.
        CacheCore::withdraw(&bucket[5], other);
        let w = c.word(f).load(Ordering::Acquire);
        c.free_frame(f, w);
        assert_every_frame_accounted(&c);
        drop(c);
        remove(&base);
    }

    /// `n` keys whose hashes name bucket `b`.
    fn keys_in_bucket(c: &CacheCore, b: usize, n: usize) -> Vec<u64> {
        (0u64..).filter(|k| c.bucket_of(hash_key(&k.to_le_bytes())) == b).take(n).collect()
    }

    /// A put of `k` stalled just after publishing: its frame `f`, filled as
    /// generation `g`, and its pending entry in bucket word `slot`.
    struct Stalled {
        k: u64,
        f: usize,
        g: u32,
        slot: usize,
        pending: u64,
    }

    /// Stall a put of `k` with value `k`: fill a frame and publish its
    /// pending entry in the first free word of the key's bucket; None, the
    /// frame freed, when no word is free.
    fn try_stall(c: &CacheCore, k: u64) -> Option<Stalled> {
        let key = k.to_le_bytes();
        let h = hash_key(&key);
        let (f, g) = c.fill(h, &key, &key).expect("a frame");
        let bucket = c.bucket(c.bucket_of(h));
        let pending = entry_word(fingerprint(h), g, f) | ENTRY_PENDING;
        match (0..WAYS).find(|&s| bucket[s].compare_exchange(0, pending, Ordering::SeqCst, Ordering::Relaxed).is_ok()) {
            Some(slot) => Some(Stalled { k, f, g, slot, pending }),
            None => {
                c.free_frame(f, frame_word(g, current_pid(), LIVE));
                None
            }
        }
    }

    fn stall(c: &CacheCore, k: u64) -> Stalled {
        try_stall(c, k).expect("a free word")
    }

    /// Run a stalled put on from its publish as `put` does: settle, and count
    /// the insert when it landed; else take the entry out, step the frame
    /// on, free it, and put the key again. Whether the entry landed.
    fn resume(c: &CacheCore, s: &Stalled) -> bool {
        let b = c.bucket_of(hash_key(&s.k.to_le_bytes()));
        if c.settle(b, s.slot, s.pending, s.f, frame_word(s.g, current_pid(), LIVE)) {
            c.header().len.fetch_add(1, Ordering::AcqRel);
            return true;
        }
        CacheCore::withdraw(&c.bucket(b)[s.slot], s.pending);
        let g = c.step_frame(s.f, s.g);
        c.free_frame(s.f, frame_word(g, current_pid(), LIVE));
        put(c, s.k, s.k);
        false
    }

    /// Making room takes back a word whose entry names a frame that has
    /// moved on, and leaves the pending entries alone.
    #[test]
    fn a_word_whose_frame_moved_on_is_taken_back_first() {
        let (c, base) = fresh("moved-on", 64);
        let stalled: Vec<Stalled> = keys_in_bucket(&c, 0, WAYS).into_iter().map(|k| stall(&c, k)).collect();
        let bucket = c.bucket(0);
        let freed = &stalled[WAYS - 1];
        c.free_frame(freed.f, frame_word(freed.g, current_pid(), LIVE));
        c.evict_in_bucket(0);
        assert_eq!(bucket[freed.slot].load(Ordering::Acquire), 0, "the word whose frame moved on is taken back");
        for s in &stalled[..WAYS - 1] {
            assert_eq!(bucket[s.slot].load(Ordering::Acquire), s.pending, "a pending entry stays");
            assert!(resume(&c, s), "an insert with no rival commits");
        }
        assert_every_frame_accounted(&c);
        drop(c);
        remove(&base);
    }

    /// A living writer's given-up entry is taken back once its frame is
    /// marked filling in its generation; the writer, resuming, reads the
    /// mark as an entry that never committed, and steps the frame on.
    #[test]
    fn a_given_up_entry_is_taken_back_after_its_writers_frame_is_marked() {
        let (c, base) = fresh("given-up", 64);
        let keys = keys_in_bucket(&c, 0, WAYS + 1);
        let stalled: Vec<Stalled> = keys[..WAYS].iter().map(|&k| stall(&c, k)).collect();
        let bucket = c.bucket(0);
        let lost = &stalled[3];
        bucket[lost.slot].store((lost.pending & !ENTRY_STATE) | ENTRY_GIVEN_UP, Ordering::SeqCst);
        c.evict_in_bucket(0);
        assert_eq!(bucket[lost.slot].load(Ordering::Acquire), 0, "the given-up word is taken back");
        assert_eq!(c.word(lost.f).load(Ordering::Acquire), frame_word(lost.g, current_pid(), FILLING), "its frame is marked first");
        assert_eq!(put(&c, keys[WAYS], 1), None, "another put takes the word");
        assert!(!c.settle(0, lost.slot, lost.pending, lost.f, frame_word(lost.g, current_pid(), LIVE)), "the writer's entry never committed");
        let g = c.step_frame(lost.f, lost.g);
        assert_eq!(c.word(lost.f).load(Ordering::Acquire), frame_word(g, current_pid(), LIVE), "the step clears the mark");
        c.free_frame(lost.f, frame_word(g, current_pid(), LIVE));
        for s in stalled.iter().filter(|s| s.slot != lost.slot) {
            assert!(resume(&c, s));
        }
        assert_eq!(get(&c, keys[WAYS]), Some(1));
        assert_every_frame_accounted(&c);
        drop(c);
        remove(&base);
    }

    /// A dead writer's given-up entry is taken back the same way, and the
    /// hand frees the frame the mark left filling.
    #[test]
    fn a_dead_writers_given_up_entry_is_taken_back_and_its_frame_freed() {
        let (c, base) = fresh("dead-given-up", 64);
        let stalled: Vec<Stalled> = keys_in_bucket(&c, 0, WAYS).into_iter().map(|k| stall(&c, k)).collect();
        let bucket = c.bucket(0);
        let dead = &stalled[2];
        leave(&c, dead.f, frame_word(dead.g, GONE, LIVE), u64::from(GONE));
        bucket[dead.slot].store((dead.pending & !ENTRY_STATE) | ENTRY_GIVEN_UP, Ordering::SeqCst);
        c.evict_in_bucket(0);
        assert_eq!(bucket[dead.slot].load(Ordering::Acquire), 0, "the given-up word is taken back");
        assert_eq!(c.word(dead.f).load(Ordering::Acquire), frame_word(dead.g, GONE, FILLING));
        for s in stalled.iter().filter(|s| s.slot != dead.slot) {
            assert!(resume(&c, s));
        }
        while evict(&c).is_some() {}
        assert_eq!(c.word(dead.f).load(Ordering::Acquire) & STATE_MASK, FREE, "the hand freed the marked frame");
        assert_every_frame_accounted(&c);
        drop(c);
        remove(&base);
    }

    /// A bucket of pending entries settles its first one for its writer:
    /// committed, the next call evicts it, and the writer, resuming, counts
    /// the insert that landed against the eviction's early decrement.
    #[test]
    fn a_bucket_of_pending_entries_settles_the_first_for_its_writer() {
        let (c, base) = fresh("all-pending", 64);
        let stalled: Vec<Stalled> = keys_in_bucket(&c, 0, WAYS).into_iter().map(|k| stall(&c, k)).collect();
        let bucket = c.bucket(0);
        let first = &stalled[0];
        c.evict_in_bucket(0);
        assert_eq!(bucket[first.slot].load(Ordering::Acquire), first.pending & !ENTRY_STATE, "the first pending entry is committed");
        assert_eq!(get(&c, first.k), Some(first.k), "and read");
        c.evict_in_bucket(0);
        assert_eq!(bucket[first.slot].load(Ordering::Acquire), 0, "then evicted");
        assert_eq!(c.len(), 0, "a count below zero reads as none");
        assert!(resume(&c, first), "its writer finds the insert landed");
        assert_eq!(c.header().len.load(Ordering::Acquire), 0, "and its count meets the eviction's");
        for s in &stalled[1..] {
            assert!(resume(&c, s));
        }
        assert_eq!(c.len(), WAYS - 1);
        assert_every_frame_accounted(&c);
        drop(c);
        remove(&base);
    }

    /// A put into a bucket whose every word holds a stalled writer's pending
    /// entry completes without waiting on any of them.
    #[test]
    fn a_put_completes_while_every_word_holds_a_stalled_writers_entry() {
        let (c, base) = fresh("stalled", 64);
        let keys = keys_in_bucket(&c, 0, WAYS + 1);
        let stalled: Vec<Stalled> = keys[..WAYS].iter().map(|&k| stall(&c, k)).collect();
        assert_eq!(put(&c, keys[WAYS], 7), None, "the put inserts");
        assert_eq!(get(&c, keys[WAYS]), Some(7));
        for s in &stalled {
            assert!(resume(&c, s), "each stalled insert landed");
        }
        assert_eq!(get(&c, keys[0]), None, "the first was settled and evicted to make room");
        assert_eq!(c.len(), WAYS);
        assert_every_frame_accounted(&c);
        drop(c);
        remove(&base);
    }

    /// Writers crowding one bucket with three times its words in keys read
    /// every value under its own key and leave every frame accounted for.
    /// The test prints how often each way of making room ran.
    #[test]
    fn writers_crowding_one_bucket_lose_no_frame_and_mix_no_values() {
        let (c, base) = fresh("crowding", 256);
        crate::set_trace::name(&[c.trace_name()]);
        let keys = keys_in_bucket(&c, 0, 3 * WAYS);
        std::thread::scope(|s| {
            for t in 0..16u64 {
                let (c, keys) = (&c, &keys);
                s.spawn(move || {
                    for i in 0..5_000u64 {
                        let k = keys[((i * 7 + t * 5) % keys.len() as u64) as usize];
                        put(c, k, (k << 16) | t);
                        if let Some(v) = get(c, k) {
                            assert_eq!(v >> 16, k, "a value is read under its own key");
                        }
                    }
                });
            }
        });
        let (events, dropped) = crate::set_trace::recorded();
        let count = |what: crate::set_trace::What, extra: Option<u64>| {
            events.iter().filter(|e| e.what == what && extra.is_none_or(|x| e.extra == x)).count()
        };
        use crate::set_trace::What::{CacheEvicted, CacheGaveUp, CacheHelped, CacheSettledFor, CacheStepped, CacheTookBack};
        println!(
            "room made: {} stale words and {} given-up words taken back, {} committed entries evicted, {} pending entries settled for their writers; \
             {} given up and {} helped by writers of their key, {} frames stepped on; {} events past the trace's capacity",
            count(CacheTookBack, Some(0)),
            count(CacheTookBack, Some(1)),
            count(CacheEvicted, None),
            count(CacheSettledFor, None),
            count(CacheGaveUp, None),
            count(CacheHelped, None),
            count(CacheStepped, None),
            dropped,
        );
        assert_every_frame_accounted(&c);
        drop(c);
        remove(&base);
    }

    /// Writers that stall between publishing and settling, beside writers
    /// crowding their bucket, have their entries settled for them when the
    /// bucket fills with pending entries, and every frame stays accounted
    /// for. The test prints how often that settling ran.
    #[test]
    fn stalled_writers_beside_crowding_puts_lose_no_frame() {
        let (c, base) = fresh("stalling", 256);
        crate::set_trace::name(&[c.trace_name()]);
        let keys = keys_in_bucket(&c, 0, 3 * WAYS);
        std::thread::scope(|s| {
            for t in 0..16u64 {
                let (c, keys) = (&c, &keys);
                s.spawn(move || {
                    for i in 0..2_000u64 {
                        let k = keys[((i * 7 + t * 5) % keys.len() as u64) as usize];
                        if t % 2 == 1 {
                            put(c, k, k);
                        } else if let Some(stalled) = try_stall(c, k) {
                            for _ in 0..i % 64 {
                                std::thread::yield_now();
                            }
                            resume(c, &stalled);
                        }
                        if let Some(v) = get(c, k) {
                            assert_eq!(v, k, "a value is read under its own key");
                        }
                    }
                });
            }
        });
        let (events, dropped) = crate::set_trace::recorded();
        let settled: Vec<u64> = events.iter().filter(|e| e.what == crate::set_trace::What::CacheSettledFor).map(|e| e.extra).collect();
        let committed = settled.iter().filter(|&&w| w != 0 && w & ENTRY_STATE == 0).count();
        println!(
            "{} pending entries settled for their writers, {committed} of them left committed; {dropped} events past the trace's capacity",
            settled.len()
        );
        assert_every_frame_accounted(&c);
        drop(c);
        remove(&base);
    }

    #[test]
    fn a_cache_of_another_shape_is_refused_by_its_field() {
        let (c, base) = fresh("shape", 8);
        drop(c);
        match CacheCore::open(&base, 8, 8, 16) {
            Err(CacheError::Mismatch(m)) => assert_eq!(m.field, "value size"),
            other => panic!("a cache of another value size was attached: {:?}", other.err()),
        }
        remove(&base);
    }
}
