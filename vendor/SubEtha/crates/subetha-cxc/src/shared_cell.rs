//! `SharedCell<T>` - a cross-process single-value cell over a
//! memory-mapped file, which any number of processes write and read at
//! once without any of them waiting on another.
//!
//! `T: Copy` plus a stable `#[repr(C)]` layout is the contract; readers in
//! different processes copy the same bytes and interpret them identically.
//!
//! # Layout
//!
//! A header line the attach checks, then two hot lines: the `head` word and
//! the two hot buffers, each value's size rounded up to 8 bytes. A value of
//! 24 bytes or less keeps both hot buffers in the head's own line, and a
//! larger one keeps the first there and the second in the next line, so no
//! hot buffer crosses a line. The hot buffers' claim words follow on a line
//! no read touches, then [`BUFFERS`] less two overflow lines, each a claim
//! word and a value, for writers that find both hot buffers held. The head
//! holds, in its low eleven bits, where the current value lies from the
//! start of the region and, above them, counts the publishes that made a
//! buffer current: the cell's generation. The region is [`CELL_FILE_SIZE`]
//! bytes, so any offset those bits hold, with a value after it, lies inside
//! it. A claim word is zero while its buffer is free and, while a writer
//! fills it, the holder word of the writer's process: its pid and start
//! time.
//!
//! # Writes
//!
//! The first thread to write through a handle claims a pair of buffers for
//! that handle, the hot pair when it is free, and keeps them while the
//! handle lives. Its writes fill whichever of the two it did not publish
//! last, which is never current, and publish it with one compare-and-swap
//! of the head. The thread is known by its thread pointer, which no two
//! live threads share. Owned pairs come from the first [`OWNED_BUFFERS`]
//! buffers only.
//!
//! Any other write takes a buffer that is not current by a
//! compare-and-swap of its claim from zero to its holder word, loads the head
//! again so a buffer made current since it looked is never written, copies
//! the value in, makes the buffer current with one compare-and-swap of the
//! head, and frees the claim. Writers racing to publish each land in turn
//! with [`set`](SharedCell::set); with
//! [`compare_and_set`](SharedCell::compare_and_set) exactly one lands on the
//! generation they all saw and the rest keep their own values.
//!
//! # Reads
//!
//! A read loads the head, copies the value at the offset it holds, and
//! loads the head again. The generation moves on every publish, and a
//! buffer is filled only while it is not current, so an unchanged head
//! means the copy is whole. A read retries only when a publish landed
//! during it, and never waits on a writer.
//!
//! # A writer that dies
//!
//! A writer that dies holding a claim, or a process that dies holding owned
//! pairs, takes only those buffers out of use. A write that claims and
//! finds no free buffer takes over one whose holder's process is gone, by a
//! compare-and-swap from that exact holder word, so exactly one takes it. A
//! holder is gone when its pid names no process, or names one that started
//! after the holder did, so a reused pid never keeps a buffer held. Such a
//! write waits only while every buffer but the current one is held by a
//! live writer mid-copy or by a live handle's pair, and pairs leave
//! [`BUFFERS`] less [`OWNED_BUFFERS`] for it.

use std::cell::UnsafeCell;
use std::fs::File;
use std::marker::PhantomData;
use std::mem::{align_of, offset_of, size_of};
use std::path::Path;
use std::sync::atomic::{fence, AtomicU32, AtomicU64, AtomicUsize, Ordering};

use memmap2::{MmapMut, MmapOptions};

/// Magic of the cell's layout: "CEL5".
pub const CELL_MAGIC: u32 = 0x354C_4543;
/// Value buffers a cell keeps: the current one, and one for each writer
/// that may be filling one at the same moment.
pub const BUFFERS: usize = 16;
/// Buffers in the hot lines beside the head; the rest are overflow lines.
const HOT_BUFFERS: usize = 2;
/// The buffers owned pairs are taken from, in pairs from the first; the
/// rest serve only writes that claim a buffer per write.
pub const OWNED_BUFFERS: usize = 12;
/// The largest value a cell holds.
pub const PAYLOAD_BYTES: usize = 56;
/// The head's low bits, which hold where the current value lies from the
/// start of the region.
const OFFSET_BITS: u32 = 11;
const OFFSET_MASK: u64 = (1 << OFFSET_BITS) - 1;
/// Lines the head and the hot buffers take.
const HOT_LINES: usize = 2;

/// The cell's first line, read when a handle attaches.
#[repr(C, align(64))]
pub struct CellHeader {
    pub magic: u32,
    pub size: u32,
    pub buffers: u32,
    _pad: u32,
}

/// The lines every read touches.
#[repr(C, align(64))]
struct HotLines {
    /// Where the current value lies in the low [`OFFSET_BITS`] bits; the
    /// generation above them.
    head: AtomicU64,
    /// The hot buffers, where [`hot_offsets`] places them.
    values: UnsafeCell<[u8; HOT_LINES * 64 - 8]>,
}

/// The hot buffers' claim words, on a line no read touches: zero while a
/// buffer is free, else the holder word, pid and start stamp, of the
/// process whose writer fills it.
#[repr(C, align(64))]
struct ClaimLine {
    claims: [AtomicU64; HOT_BUFFERS],
    _pad: [u8; 64 - HOT_BUFFERS * 8],
}

/// A buffer on a line of its own, for a writer that finds both hot buffers
/// held.
#[repr(C, align(64))]
struct OverflowBuffer {
    /// Zero while free; the holder word of the process whose writer fills
    /// it otherwise.
    claim: AtomicU64,
    value: UnsafeCell<[u8; PAYLOAD_BYTES]>,
}

const _: () = assert!(size_of::<CellHeader>() == 64);
const _: () = assert!(size_of::<HotLines>() == HOT_LINES * 64 && offset_of!(HotLines, values) == 8);
// A read reaches the head as the first word of the hot lines.
const _: () = assert!(offset_of!(HotLines, head) == 0);
const _: () = assert!(size_of::<ClaimLine>() == 64);
const _: () = assert!(size_of::<OverflowBuffer>() == 64);
const _: () = assert!(BUFFERS.is_power_of_two() && BUFFERS <= 256);
// A claim tries the other hot buffer by name.
const _: () = assert!(HOT_BUFFERS == 2);
// Owned pairs are whole pairs from the hot pair up, and claimed writes keep
// at least two buffers no pair takes.
const _: () = assert!(OWNED_BUFFERS.is_multiple_of(2) && OWNED_BUFFERS >= HOT_BUFFERS && OWNED_BUFFERS + 2 <= BUFFERS);
// The largest value fits a line of its own.
const _: () = assert!(PAYLOAD_BYTES <= 64);

/// Where the hot lines start.
const HOT_AT: usize = size_of::<CellHeader>();
/// Where the hot buffers' bytes start.
const HOT_VALUES_AT: usize = HOT_AT + offset_of!(HotLines, values);
/// Where the hot buffers' claim line starts.
const CLAIMS_AT: usize = HOT_AT + size_of::<HotLines>();
/// Where the first overflow line starts.
const OVERFLOW_AT: usize = CLAIMS_AT + size_of::<ClaimLine>();
/// Where the last overflow buffer's value lies.
const LAST_VALUE_AT: usize =
    OVERFLOW_AT + (BUFFERS - HOT_BUFFERS - 1) * size_of::<OverflowBuffer>() + offset_of!(OverflowBuffer, value);

/// The region's size: every line, padded to a page, so any offset the
/// head's bits hold, with a value after it, lies inside the region.
pub const CELL_FILE_SIZE: usize = 4096;

// Every value's offset fits the head's bits, and every offset those bits
// hold, with a value after it, lies inside the region.
const _: () = assert!(LAST_VALUE_AT <= OFFSET_MASK as usize);
const _: () = assert!(LAST_VALUE_AT + PAYLOAD_BYTES <= CELL_FILE_SIZE);
const _: () = assert!(OFFSET_MASK as usize + PAYLOAD_BYTES <= CELL_FILE_SIZE);

/// Where the two hot buffers' values lie from the start of the region, for
/// values `stride` bytes apart: both in the head's line while they fit
/// beside it, else the first there and the second in the next line, so
/// neither crosses a line.
const fn hot_offsets(stride: usize) -> [usize; HOT_BUFFERS] {
    if HOT_VALUES_AT - HOT_AT + 2 * stride <= 64 {
        [HOT_VALUES_AT, HOT_VALUES_AT + stride]
    } else {
        [HOT_VALUES_AT, HOT_AT + 64]
    }
}

// The largest value fits beside the head, and alone in the line after it.
const _: () = assert!(HOT_VALUES_AT - HOT_AT + PAYLOAD_BYTES.next_multiple_of(8) <= 64);

/// Where the value a head word names lies from the start of the region.
/// Every value lies at a multiple of 8, and the mask keeps a head a damaged
/// file carries inside the region and at a multiple of 8 too.
#[inline(always)]
fn value_at(head: u64) -> usize {
    (head & OFFSET_MASK & !7) as usize
}

/// The generation a head word carries.
#[inline(always)]
fn generation_of(head: u64) -> u64 {
    head >> OFFSET_BITS
}

/// The head one publish after `head`, naming the value at `offset`.
#[inline(always)]
fn after(head: u64, offset: usize) -> u64 {
    (head | OFFSET_MASK) + 1 + offset as u64
}

/// A word no two live threads share: the calling thread's thread pointer
/// where one instruction reads it, else the address of a thread-local. A
/// thread started after another exits may be handed the same word.
#[inline(always)]
fn thread_mark() -> usize {
    #[cfg(all(target_arch = "x86_64", target_os = "windows"))]
    {
        let mark: usize;
        // SAFETY: gs:[0x30] holds the address of the calling thread's TEB on
        // x64 Windows, and reading it has no other effect.
        unsafe {
            std::arch::asm!("mov {}, qword ptr gs:[0x30]", out(reg) mark, options(nostack, readonly, preserves_flags, pure))
        };
        mark
    }
    #[cfg(all(target_arch = "x86_64", any(target_os = "linux", target_os = "android", target_os = "freebsd")))]
    {
        let mark: usize;
        // SAFETY: the x86-64 ELF TLS ABI keeps the thread control block's own
        // address in its first word, at fs:[0], and reading it has no other
        // effect.
        unsafe {
            std::arch::asm!("mov {}, qword ptr fs:[0]", out(reg) mark, options(nostack, readonly, preserves_flags, pure))
        };
        mark
    }
    #[cfg(all(target_arch = "aarch64", any(target_os = "linux", target_os = "android", target_os = "freebsd")))]
    {
        let mark: usize;
        // SAFETY: TPIDR_EL0 holds the calling thread's thread pointer, and
        // reading it has no other effect.
        unsafe { std::arch::asm!("mrs {}, tpidr_el0", out(reg) mark, options(nomem, nostack, preserves_flags, pure)) };
        mark
    }
    #[cfg(not(any(
        all(target_arch = "x86_64", any(target_os = "windows", target_os = "linux", target_os = "android", target_os = "freebsd")),
        all(target_arch = "aarch64", any(target_os = "linux", target_os = "android", target_os = "freebsd")),
    )))]
    {
        thread_local! {
            static MARK: u8 = const { 0 };
        }
        MARK.with(|m| m as *const u8 as usize)
    }
}

/// [`CellCore::owner`] while a thread takes a pair, and after the first
/// writing thread found none: no thread mark is ever this.
const NO_OWNER: usize = usize::MAX;

#[derive(Debug)]
pub enum SharedCellError {
    LayoutMismatch(crate::layout::Mismatch),
    PayloadTooLarge,
    NotInitialized,
    /// The operating system refused to open, size or map the file; the
    /// error carries its code and message.
    IoError(std::io::Error),
}

impl std::fmt::Display for SharedCellError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::LayoutMismatch(m) => write!(f, "a cell of another shape: {m}"),
            Self::PayloadTooLarge => write!(
                f,
                "the value is empty, too large or aligned past 8, or a slice is the wrong size"
            ),
            Self::NotInitialized => write!(f, "the cell has not been initialized"),
            Self::IoError(e) => write!(f, "io: {e}"),
        }
    }
}

impl std::error::Error for SharedCellError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::IoError(e) => Some(e),
            Self::LayoutMismatch(_) | Self::PayloadTooLarge | Self::NotInitialized => None,
        }
    }
}

impl From<std::io::Error> for SharedCellError {
    fn from(e: std::io::Error) -> Self { Self::IoError(e) }
}

/// The first of the header's magic, value size and buffer count that
/// differs from a cell of `value_size` bytes.
/// [`RawCell`](crate::raw_cell::RawCell) checks through here too, so the
/// two cells refuse the same regions.
pub(crate) fn header_matches(header: &CellHeader, value_size: usize) -> Result<(), SharedCellError> {
    crate::layout::Mismatch::first(&[
        ("magic", u64::from(header.magic), u64::from(CELL_MAGIC)),
        ("value size", u64::from(header.size), value_size as u64),
        ("buffers", u64::from(header.buffers), BUFFERS as u64),
    ])
    .map_err(SharedCellError::LayoutMismatch)
}

/// The file, refused when it is shorter than a cell.
pub(crate) fn long_enough(file: &File) -> Result<(), SharedCellError> {
    let len = file.metadata()?.len();
    if len < CELL_FILE_SIZE as u64 {
        return Err(SharedCellError::LayoutMismatch(crate::layout::Mismatch::new(
            "length",
            len,
            CELL_FILE_SIZE as u64,
        )));
    }
    Ok(())
}

/// Lay out an empty cell for values of `value_size` bytes: the zeroed
/// region is already generation 0 with every buffer holding zero, so the
/// size, the buffer count and the head naming the first hot buffer are
/// written, then the magic, last, because attachers wait on it.
///
/// # Safety
/// `ptr` addresses at least [`CELL_FILE_SIZE`] writable zeroed bytes.
pub(crate) unsafe fn init_region(ptr: *mut u8, value_size: usize) {
    let hdr = ptr as *mut CellHeader;
    let first = hot_offsets(value_size.next_multiple_of(8))[0];
    unsafe {
        (*hdr).size = value_size as u32;
        (*hdr).buffers = BUFFERS as u32;
        (*(ptr.add(HOT_AT) as *const HotLines)).head.store(first as u64, Ordering::Relaxed);
        std::ptr::write_volatile(&raw mut (*hdr).magic, CELL_MAGIC);
    }
}

/// What a read through a handle uses, on lines of its own, so a read that
/// records a new head never touches the line an owner's write reads. The
/// cache starts in the first line, so a value of up to 32 bytes is read
/// from that line alone.
#[repr(C, align(64))]
struct ReadLine {
    base: *mut u8,
    /// The last head a read through this handle confirmed with a second
    /// load: its value was whole then and stays whole while the head holds
    /// it, so the next read copies it before loading the head at all.
    confirmed: AtomicU64,
    /// The head whose value `cache` holds, or NONE: a typed read whose head
    /// still equals it returns the cache without touching the region.
    cached: AtomicU64,
    /// Held by the read refilling `cache`, so two refills never interleave.
    filling: AtomicU32,
    /// Whether one plain 16-byte load is atomic here, found when the handle
    /// opens: a read of the cache takes 16-byte lanes on it without asking
    /// the processor again.
    lanes: bool,
    cache: [AtomicU64; CACHE_WORDS],
}

/// Words of a cached value: enough for the largest value a cell holds, so
/// every typed read can take the cache.
const CACHE_WORDS: usize = PAYLOAD_BYTES.div_ceil(8);
const CACHE_BYTES: usize = CACHE_WORDS * 8;
const _: () = assert!(CACHE_BYTES >= PAYLOAD_BYTES);
/// A `cached` no head equals: a head's offset is a multiple of 8.
const NONE: u64 = u64::MAX;

/// The cell protocol over one mapped region, for a value of `size` bytes.
/// The read line leads, and the fields an owner's write uses fill the line
/// after it.
#[repr(C)]
pub(crate) struct CellCore {
    read: ReadLine,
    base: *mut u8,
    /// The head the owner's last publish left: the value its next publish
    /// compares against first. Its offset names the pair's buffer the owner
    /// published last, so the other is the one it fills next.
    seen: AtomicU64,
    /// The offsets of the pair's two values exclusive-ored, so the offset a
    /// publish names gives the other.
    pair_xor: AtomicUsize,
    /// The [`thread_mark`] of the thread that owns this handle's pair; zero
    /// until a thread writes, [`NO_OWNER`] while one takes the pair or once
    /// the first writer found none.
    owner: AtomicUsize,
    /// The owner's process, so a forked child, whose thread carries the
    /// same mark, claims per write instead.
    owner_pid: AtomicU32,
    /// Where this handle's next write starts looking for a free buffer, so
    /// writers through different handles spread over the buffers.
    next: AtomicU32,
    /// The first buffer of the owned pair; the second follows it.
    pair: AtomicUsize,
    size: usize,
    /// Where the two hot buffers' values lie, per [`hot_offsets`].
    hot: [usize; HOT_BUFFERS],
}

impl CellCore {
    /// The protocol over the region at `base`.
    ///
    /// # Safety
    /// `base` addresses a laid-out cell region of [`CELL_FILE_SIZE`] bytes
    /// that outlives the core, for values of `size` bytes, at most
    /// [`PAYLOAD_BYTES`].
    pub(crate) unsafe fn new(base: *mut u8, size: usize) -> Self {
        let spread = crate::peer_directory::current_pid().wrapping_mul(0x9E37_79B9) ^ (base as usize as u32);
        // SAFETY: the region is laid out, per the caller, and its hot lines
        // follow the header line.
        let head = unsafe { (*(base.add(HOT_AT) as *const HotLines)).head.load(Ordering::Acquire) };
        Self {
            read: ReadLine {
                base,
                confirmed: AtomicU64::new(head),
                cached: AtomicU64::new(NONE),
                cache: [const { AtomicU64::new(0) }; CACHE_WORDS],
                filling: AtomicU32::new(0),
                lanes: crate::atomic_pair::plain_sixteen(),
            },
            base,
            seen: AtomicU64::new(0),
            pair_xor: AtomicUsize::new(0),
            owner: AtomicUsize::new(0),
            owner_pid: AtomicU32::new(0),
            next: AtomicU32::new(spread),
            pair: AtomicUsize::new(0),
            size,
            hot: hot_offsets(size.next_multiple_of(8)),
        }
    }

    fn hot(&self) -> &HotLines {
        // SAFETY: the hot lines follow the header line, per `new`.
        unsafe { &*(self.base.add(HOT_AT) as *const HotLines) }
    }

    fn head(&self) -> &AtomicU64 {
        &self.hot().head
    }

    /// Overflow buffer `i`, for `i` from [`HOT_BUFFERS`] up to [`BUFFERS`].
    fn overflow(&self, i: usize) -> &OverflowBuffer {
        debug_assert!((HOT_BUFFERS..BUFFERS).contains(&i));
        // SAFETY: the region holds BUFFERS - HOT_BUFFERS overflow lines after
        // the hot lines, and every caller passes an index among them.
        unsafe { &*(self.base.add(OVERFLOW_AT + (i - HOT_BUFFERS) * size_of::<OverflowBuffer>()) as *const OverflowBuffer) }
    }

    fn claim_line(&self) -> &ClaimLine {
        // SAFETY: the claim line follows the hot lines, per `new`.
        unsafe { &*(self.base.add(CLAIMS_AT) as *const ClaimLine) }
    }

    fn claim_word(&self, i: usize) -> &AtomicU64 {
        if i < HOT_BUFFERS { &self.claim_line().claims[i] } else { &self.overflow(i).claim }
    }

    /// Where buffer `i`'s value lies from the start of the region.
    fn value_offset(&self, i: usize) -> usize {
        if i < HOT_BUFFERS {
            self.hot[i]
        } else {
            OVERFLOW_AT + (i - HOT_BUFFERS) * size_of::<OverflowBuffer>() + offset_of!(OverflowBuffer, value)
        }
    }

    /// The value at `offset` from the start of the region.
    ///
    /// # Safety
    /// `offset` is at most the head's mask, or a buffer's value offset, so
    /// it and a value after it lie inside the region.
    #[inline(always)]
    unsafe fn at(&self, offset: usize) -> *mut u8 {
        // SAFETY: inside the region, by the caller.
        unsafe { self.base.add(offset) }
    }

    /// Where buffer `i`'s value lies.
    fn value(&self, i: usize) -> *mut u8 {
        // SAFETY: a buffer's value offset lies inside the region.
        unsafe { self.at(self.value_offset(i)) }
    }

    /// The generation: how many publishes have landed.
    pub(crate) fn generation(&self) -> u64 {
        generation_of(self.head().load(Ordering::Acquire))
    }

    /// The head word, reached from the read line.
    #[inline(always)]
    fn read_head(&self) -> &AtomicU64 {
        // SAFETY: the head is the first word of the hot lines, which follow
        // the header line, per `new`.
        unsafe { &*(self.read.base.add(HOT_AT) as *const AtomicU64) }
    }

    /// Hand the current value's bytes to `copy` and answer the generation
    /// that made it current, and whether a publish landing during the copy
    /// made the read take it again. `copy` may be handed a buffer a writer
    /// is filling; whatever it copied then is taken again, so it must copy
    /// bytes and nothing more.
    ///
    /// The copy starts from the head this handle last confirmed, and one
    /// load of the head after it says whether that head still holds: its
    /// value has been current, and so unwritten, since it was confirmed.
    ///
    /// # Safety
    /// `copy` reads at most `size` bytes from the address it is handed.
    #[inline]
    pub(crate) unsafe fn load_with(&self, mut copy: impl FnMut(*const u8)) -> (u64, bool) {
        let base = self.read.base;
        let head = self.read_head();
        let mut seen = self.read.confirmed.load(Ordering::Acquire);
        let mut retried = false;
        let mut first = true;
        loop {
            // SAFETY: the offset is masked to the head's bits.
            copy(unsafe { base.add(value_at(seen)) });
            fence(Ordering::Acquire);
            let now = head.load(Ordering::Acquire);
            if now == seen {
                if !first {
                    self.read.confirmed.store(now, Ordering::Release);
                }
                return (generation_of(now), retried);
            }
            retried |= !first;
            first = false;
            seen = now;
        }
    }

    /// The current value as a `T`, its generation, and whether a publish
    /// landing during the copy made the read take it again. The copy is of
    /// bytes, so a copy a writer raced makes no `T` before the head check
    /// confirms it whole.
    ///
    /// # Safety
    /// `size_of::<T>()` is the core's value size, `align_of::<T>()` is at
    /// most 8, and every value written to the cell is a `T`, as is the zero a
    /// fresh cell holds.
    #[inline(always)]
    pub(crate) unsafe fn load_value<T: Copy>(&self) -> (u64, bool, T) {
        if size_of::<T>() <= CACHE_BYTES {
            let cached = self.read.cached.load(Ordering::Acquire);
            // SAFETY: the cache is 16-byte aligned in the read line and holds
            // CACHE_BYTES, at least size_of::<T>() rounded up to 8, and `lanes`
            // is what plain_sixteen returned.
            let value = unsafe { crate::atomic_copy::load_aligned(self.read.cache.as_ptr() as *const T, self.read.lanes) };
            fence(Ordering::Acquire);
            let now = self.read_head().load(Ordering::Acquire);
            if now == cached {
                // SAFETY: a refill stores `cached` after the bytes it names
                // and starts only once the head has left the value the cache
                // held, so a head still equal to `cached` makes these that
                // value's bytes: a T a writer stored, per the caller.
                return (generation_of(now), false, unsafe { value.assume_init() });
            }
            // SAFETY: as for this call, and the value fits the cache.
            return unsafe { self.load_value_refill(now) };
        }
        let confirmed = self.read.confirmed.load(Ordering::Acquire);
        // SAFETY: as in `load_value_from`.
        let value = unsafe { crate::atomic_copy::load_with_lanes(self.read.base.add(value_at(confirmed)) as *const T, self.read.lanes) };
        fence(Ordering::Acquire);
        let now = self.read_head().load(Ordering::Acquire);
        if now == confirmed {
            // SAFETY: the head this handle last confirmed still holds, so its
            // value has been current since and the copy is whole: a T a
            // writer stored, or a fresh cell's zero, per the caller.
            return (generation_of(now), false, unsafe { value.assume_init() });
        }
        // SAFETY: as for this call.
        unsafe { self.load_value_from(now) }
    }

    /// [`load_value`](Self::load_value) when the head differs from the one
    /// this handle confirmed: copies from `seen` until a second load of the
    /// head finds it unchanged, and records that head as confirmed.
    ///
    /// # Safety
    /// As for `load_value`, and `seen` came from an acquire load of the head.
    #[inline(never)]
    unsafe fn load_value_from<T: Copy>(&self, mut seen: u64) -> (u64, bool, T) {
        let base = self.read.base;
        let head = self.read_head();
        let mut retried = false;
        loop {
            // SAFETY: the offset is masked to the head's bits and a multiple
            // of 8, and T is no larger than a value and aligned to at most 8,
            // per the caller. The copy is a MaybeUninit<T>, which any bytes
            // are, so a copy a writer raced is never a T.
            let value = unsafe { crate::atomic_copy::load_with_lanes(base.add(value_at(seen)) as *const T, self.read.lanes) };
            fence(Ordering::Acquire);
            let now = head.load(Ordering::Acquire);
            if now == seen {
                self.read.confirmed.store(now, Ordering::Release);
                // SAFETY: the head did not change, so the copy is whole, as in
                // `load_value`.
                return (generation_of(now), retried, unsafe { value.assume_init() });
            }
            retried = true;
            seen = now;
        }
    }

    /// [`load_value`](Self::load_value) for a cached value whose head differs
    /// from the one this handle confirmed: copies from `seen` until a second
    /// load of the head finds it unchanged, then refills the cache with that
    /// value and records its head, unless another read through this handle
    /// is refilling it.
    ///
    /// # Safety
    /// As for `load_value`, `size_of::<T>()` is at most CACHE_BYTES, and
    /// `seen` came from an acquire load of the head.
    #[inline]
    unsafe fn load_value_refill<T: Copy>(&self, mut seen: u64) -> (u64, bool, T) {
        let base = self.read.base;
        let head = self.read_head();
        let mut retried = false;
        loop {
            // SAFETY: as in `load_value_from`.
            let value = unsafe { crate::atomic_copy::load_with_lanes(base.add(value_at(seen)) as *const T, self.read.lanes) };
            fence(Ordering::Acquire);
            let now = head.load(Ordering::Acquire);
            if now == seen {
                // SAFETY: the head did not change, so the copy is whole, as in
                // `load_value`.
                let value = unsafe { value.assume_init() };
                if self.read.filling.compare_exchange(0, 1, Ordering::Acquire, Ordering::Relaxed).is_ok() {
                    // Refill only while the head still names this value: heads
                    // only move forward, so no earlier refill named a later
                    // head, and a reader that copies any byte stored below
                    // also loads the head after it, finds it past whatever its
                    // `cached` named before, and never takes a partial refill.
                    if head.load(Ordering::Acquire) == now {
                        fence(Ordering::Release);
                        // SAFETY: the cache holds CACHE_BYTES, 16-byte
                        // aligned, and `filling` keeps every other refill out.
                        unsafe { crate::atomic_copy::store(self.read.cache.as_ptr() as *mut T, &value) };
                        self.read.cached.store(now, Ordering::Release);
                    }
                    self.read.filling.store(0, Ordering::Release);
                }
                return (generation_of(now), retried, value);
            }
            retried = true;
            seen = now;
        }
    }

    /// [`CellCore::load_with`] into the `size` bytes at `out`.
    ///
    /// # Safety
    /// `out` addresses `size` writable bytes.
    pub(crate) unsafe fn load(&self, out: *mut u8) -> (u64, bool) {
        let size = self.size;
        // SAFETY: the buffer holds PAYLOAD_BYTES and `size` is at most that,
        // and `out` holds `size` bytes by the caller's promise.
        unsafe { self.load_with(|value| std::ptr::copy_nonoverlapping(value, out, size)) }
    }

    /// Make the value `fill` writes current, after any publish that lands
    /// first, and answer its generation: through this handle's owned pair
    /// when the calling thread owns it, or takes it by writing first, and
    /// through a buffer claimed for this write otherwise.
    ///
    /// # Safety
    /// `fill` writes at most `size` bytes at the address it is handed.
    #[inline]
    pub(crate) unsafe fn set_with(&self, fill: impl FnOnce(*mut u8)) -> u64 {
        if self.owns() {
            // SAFETY: the calling thread owns the pair, and `fill` is as the
            // caller promised.
            unsafe { self.set_owned(fill) }
        } else {
            // SAFETY: `fill` is as the caller promised.
            unsafe { self.set_claimed(fill) }
        }
    }

    /// Whether the calling thread owns this handle's pair, taking the pair
    /// when no thread has written through the handle yet. An owner is never
    /// pid zero, so a pid not yet cached sends the check to
    /// [`owns_slow`](Self::owns_slow), which asks for it.
    #[inline(always)]
    pub(crate) fn owns(&self) -> bool {
        let me = thread_mark();
        let mine = (self.owner.load(Ordering::Relaxed) == me)
            & (self.owner_pid.load(Ordering::Relaxed) == crate::peer_directory::cached_pid());
        mine || self.owns_slow(me)
    }

    /// [`owns`](Self::owns) when the fast check missed: a pair to take, a
    /// pid to cache, or another thread's or process's handle.
    #[cold]
    #[inline(never)]
    fn owns_slow(&self, me: usize) -> bool {
        match self.owner.load(Ordering::Relaxed) {
            0 => self.take_pair(me),
            owner => owner == me && self.owner_pid.load(Ordering::Relaxed) == crate::peer_directory::current_pid(),
        }
    }

    /// Where the owner's next write goes, the pair's buffer its last publish
    /// did not name, and the head that publish left.
    #[inline(always)]
    fn idle(&self) -> (usize, u64) {
        let seen = self.seen.load(Ordering::Relaxed);
        (value_at(seen) ^ self.pair_xor.load(Ordering::Relaxed), seen)
    }

    /// Record the owner's publish as head `next`, which names the buffer it
    /// filled, so the pair's other buffer is the next to fill. The fence
    /// orders the publish before that fill's stores, since a reader that
    /// took the other buffer as current may still be copying it and checks
    /// the head after.
    #[inline(always)]
    fn published(&self, next: u64) {
        fence(Ordering::Release);
        self.seen.store(next, Ordering::Relaxed);
    }

    /// A write by the owner of this handle's pair: the pair's idle buffer,
    /// filled and published with one compare-and-swap of the head.
    ///
    /// # Safety
    /// The calling thread owns the pair, and `fill` writes at most `size`
    /// bytes at the address it is handed.
    #[inline]
    pub(crate) unsafe fn set_owned(&self, fill: impl FnOnce(*mut u8)) -> u64 {
        let (idle, seen) = self.idle();
        let head = self.head();
        // Only the owner publishes the pair, and its last publish was the
        // other buffer or neither, so this one is not current; no claiming
        // writer takes either.
        // SAFETY: the idle offset is one of the pair's value offsets.
        fill(unsafe { self.at(idle) });
        self.publish_owned(head, idle, seen)
    }

    /// [`set_owned`](Self::set_owned) of a `T` handed over whole and stored
    /// into the idle buffer as a `T`, so a value its caller built in
    /// registers goes there from them.
    ///
    /// # Safety
    /// The calling thread owns the pair, and `T` is no larger than the
    /// value size and aligned to at most 8.
    #[inline(always)]
    pub(crate) unsafe fn set_owned_value<T: Copy>(&self, value: T) -> u64 {
        let (idle, seen) = self.idle();
        let head = self.head();
        // SAFETY: as in `set_owned`; every value lies at a multiple of 8, and
        // T fits a value, per the caller.
        unsafe { crate::atomic_copy::store(self.at(idle) as *mut T, &value) };
        self.publish_owned(head, idle, seen)
    }

    /// Make the owner's idle buffer, at `idle` and filled, current with one
    /// compare-and-swap of `head`, expected to be `seen`, after any publish
    /// that lands first. The head and `seen` are read before the fill, whose
    /// stores the compiler cannot tell from the handle's.
    #[inline(always)]
    fn publish_owned(&self, head: &AtomicU64, idle: usize, mut seen: u64) -> u64 {
        loop {
            let next = after(seen, idle);
            match head.compare_exchange_weak(seen, next, Ordering::AcqRel, Ordering::Acquire) {
                Ok(_) => {
                    self.published(next);
                    return generation_of(next);
                }
                Err(now) => seen = now,
            }
        }
    }

    /// Take a pair of buffers for `me`, the first thread to write through
    /// this handle: the first two-buffer step of the first [`OWNED_BUFFERS`]
    /// whose claims are both free. Whether `me` owns one now. A handle whose
    /// first writer found none claims per write from then on, as every write
    /// through it does while another thread is taking the pair.
    #[cold]
    fn take_pair(&self, me: usize) -> bool {
        if self.owner.compare_exchange(0, NO_OWNER, Ordering::Relaxed, Ordering::Relaxed).is_err() {
            return false;
        }
        let holder = crate::process_identity::current();
        for a in (0..OWNED_BUFFERS).step_by(2) {
            let b = a + 1;
            if self.claim_word(a).compare_exchange(0, holder, Ordering::AcqRel, Ordering::Relaxed).is_err() {
                continue;
            }
            if self.claim_word(b).compare_exchange(0, holder, Ordering::AcqRel, Ordering::Relaxed).is_err() {
                self.release(a);
                continue;
            }
            // A buffer whose claim was free may be the current one; the first
            // owned write fills the other. The owner starts from the head as
            // though its last publish had named the pair's other buffer; when
            // the head names one outside the pair, its first compare-and-swap
            // fails once and retries on the head it finds.
            let (at_a, at_b) = (self.value_offset(a), self.value_offset(b));
            let head = self.head().load(Ordering::Acquire);
            let last = if value_at(head) == at_a { at_a } else { at_b };
            self.pair.store(a, Ordering::Relaxed);
            self.pair_xor.store(at_a ^ at_b, Ordering::Relaxed);
            self.seen.store((head & !OFFSET_MASK) | last as u64, Ordering::Relaxed);
            self.owner_pid.store(crate::process_identity::holder_pid(holder), Ordering::Relaxed);
            self.owner.store(me, Ordering::Relaxed);
            return true;
        }
        false
    }

    /// [`CellCore::set_with`] through a buffer claimed for this write.
    ///
    /// # Safety
    /// `fill` writes at most `size` bytes at the address it is handed.
    #[inline(never)]
    pub(crate) unsafe fn set_claimed(&self, fill: impl FnOnce(*mut u8)) -> u64 {
        let (buffer, seen) = self.claim();
        // No other writer holds this claim, and a read keeps no copy of a
        // buffer that is not current.
        fill(self.value(buffer));
        self.publish_claimed(buffer, seen)
    }

    /// [`claim`](Self::claim), out of line, for a write that fills the
    /// buffer it hands back itself.
    #[inline(never)]
    fn claim_for_write(&self) -> (usize, u64) {
        self.claim()
    }

    /// Make buffer `buffer`, claimed under head `seen` and filled, current
    /// after any publish that lands first, give its claim back, and answer
    /// its generation.
    #[inline(never)]
    fn publish_claimed(&self, buffer: usize, mut seen: u64) -> u64 {
        let offset = self.value_offset(buffer);
        let head = self.head();
        loop {
            let next = after(seen, offset);
            match head.compare_exchange_weak(seen, next, Ordering::AcqRel, Ordering::Acquire) {
                Ok(_) => {
                    self.release(buffer);
                    return generation_of(next);
                }
                Err(now) => seen = now,
            }
        }
    }

    /// Make buffer `buffer`, claimed under head `now` and filled, current if
    /// the head is still `now`, and give its claim back: the new generation,
    /// or the one a publish that landed first left.
    #[inline(never)]
    fn land_claimed(&self, buffer: usize, now: u64) -> Result<u64, u64> {
        let next = after(now, self.value_offset(buffer));
        let landed = self
            .head()
            .compare_exchange(now, next, Ordering::AcqRel, Ordering::Acquire)
            .map(|_| generation_of(next))
            .map_err(generation_of);
        self.release(buffer);
        landed
    }

    /// [`CellCore::set_with`] from the `size` bytes at `value`.
    ///
    /// # Safety
    /// `value` addresses `size` readable bytes.
    pub(crate) unsafe fn set(&self, value: *const u8) -> u64 {
        let size = self.size;
        // SAFETY: the buffer holds PAYLOAD_BYTES and `size` is at most that,
        // and `value` holds `size` bytes by the caller's promise.
        unsafe { self.set_with(|buffer| std::ptr::copy_nonoverlapping(value, buffer, size)) }
    }

    /// Make the value `fill` writes current if the generation is still
    /// `seen`: the new generation, or the generation a publish that landed
    /// first left, with the value never made current.
    ///
    /// # Safety
    /// `fill` writes at most `size` bytes at the address it is handed.
    #[inline]
    pub(crate) unsafe fn compare_and_set_with(&self, seen: u64, fill: impl FnOnce(*mut u8)) -> Result<u64, u64> {
        if self.owns() {
            // SAFETY: the calling thread owns the pair, and `fill` is as the
            // caller promised.
            unsafe { self.compare_and_set_owned(seen, fill) }
        } else {
            // SAFETY: `fill` is as the caller promised.
            unsafe { self.compare_and_set_claimed(seen, fill) }
        }
    }

    /// [`CellCore::compare_and_set_with`] by the owner of this handle's pair,
    /// through the pair's idle buffer.
    ///
    /// # Safety
    /// The calling thread owns the pair, and `fill` writes at most `size`
    /// bytes at the address it is handed.
    #[inline]
    pub(crate) unsafe fn compare_and_set_owned(&self, seen: u64, fill: impl FnOnce(*mut u8)) -> Result<u64, u64> {
        let now = self.head().load(Ordering::Acquire);
        if generation_of(now) != seen {
            return Err(generation_of(now));
        }
        let (idle, _) = self.idle();
        // Not current, and no claiming writer takes it, as in `set_owned`.
        // SAFETY: the idle offset is one of the pair's value offsets.
        fill(unsafe { self.at(idle) });
        self.land_owned(idle, now)
    }

    /// [`compare_and_set_owned`](Self::compare_and_set_owned) of a `T`
    /// handed over whole, as [`set_owned_value`](Self::set_owned_value) is.
    ///
    /// # Safety
    /// As for `set_owned_value`.
    #[inline(always)]
    pub(crate) unsafe fn compare_and_set_owned_value<T: Copy>(&self, seen: u64, value: T) -> Result<u64, u64> {
        let now = self.head().load(Ordering::Acquire);
        if generation_of(now) != seen {
            return Err(generation_of(now));
        }
        let (idle, _) = self.idle();
        // SAFETY: as in `set_owned_value`.
        unsafe { crate::atomic_copy::store(self.at(idle) as *mut T, &value) };
        self.land_owned(idle, now)
    }

    /// Make the owner's idle buffer, at `idle` and filled, current if the
    /// head is still `now`: the new generation, or the one a publish that
    /// landed first left.
    #[inline(always)]
    fn land_owned(&self, idle: usize, now: u64) -> Result<u64, u64> {
        let next = after(now, idle);
        match self.head().compare_exchange(now, next, Ordering::AcqRel, Ordering::Acquire) {
            Ok(_) => {
                self.published(next);
                Ok(generation_of(next))
            }
            Err(moved) => Err(generation_of(moved)),
        }
    }

    /// [`CellCore::compare_and_set_with`] through a buffer claimed for this
    /// write.
    ///
    /// # Safety
    /// `fill` writes at most `size` bytes at the address it is handed.
    #[inline(never)]
    pub(crate) unsafe fn compare_and_set_claimed(&self, seen: u64, fill: impl FnOnce(*mut u8)) -> Result<u64, u64> {
        let (buffer, now) = self.claim();
        if generation_of(now) != seen {
            self.release(buffer);
            return Err(generation_of(now));
        }
        // No other writer holds this claim, and a read keeps no copy of a
        // buffer that is not current.
        fill(self.value(buffer));
        self.land_claimed(buffer, now)
    }

    /// [`CellCore::compare_and_set_with`] from the `size` bytes at `value`.
    ///
    /// # Safety
    /// `value` addresses `size` readable bytes.
    pub(crate) unsafe fn compare_and_set(&self, seen: u64, value: *const u8) -> Result<u64, u64> {
        let size = self.size;
        // SAFETY: the buffer holds PAYLOAD_BYTES and `size` is at most that,
        // and `value` holds `size` bytes by the caller's promise.
        unsafe { self.compare_and_set_with(seen, |buffer| std::ptr::copy_nonoverlapping(value, buffer, size)) }
    }

    fn release(&self, buffer: usize) {
        self.claim_word(buffer).store(0, Ordering::Release);
    }

    /// A buffer that is not current, claimed for this process, and the
    /// head it was found not current under. While a hot buffer is current
    /// the other hot one is tried first, so reads keep finding the current
    /// value beside the head.
    #[inline]
    fn claim(&self) -> (usize, u64) {
        let me = crate::process_identity::current();
        let current = value_at(self.head().load(Ordering::Acquire));
        if let Some(now) = self.hot.iter().position(|&at| at == current) {
            let other = now ^ 1;
            let claim = &self.claim_line().claims[other];
            if claim.load(Ordering::Relaxed) == 0
                && claim.compare_exchange(0, me, Ordering::AcqRel, Ordering::Relaxed).is_ok()
                && let Some(seen) = self.not_current(other)
            {
                return (other, seen);
            }
        }
        self.claim_any(me)
    }

    /// [`claim`](Self::claim) when the other hot buffer is held, or an
    /// overflow buffer is current: every buffer is tried, from a point
    /// that spreads handles over them, and then any whose holder's process
    /// is gone, judged by its pid and start.
    #[cold]
    fn claim_any(&self, me: u64) -> (usize, u64) {
        let head = self.head();
        // A starting point only: threads sharing this handle that read the
        // same one race on the claims, which settle it.
        let start = self.next.load(Ordering::Relaxed);
        self.next.store(start.wrapping_add(1), Ordering::Relaxed);
        let start = start as usize;
        let overflows = BUFFERS - HOT_BUFFERS;
        let order = (0..HOT_BUFFERS)
            .map(|k| (start + k) % HOT_BUFFERS)
            .chain((0..overflows).map(|k| HOT_BUFFERS + (start + k) % overflows));
        loop {
            let current = value_at(head.load(Ordering::Acquire));
            for i in order.clone() {
                if self.value_offset(i) != current
                    && self.claim_word(i).load(Ordering::Relaxed) == 0
                    && self.claim_word(i).compare_exchange(0, me, Ordering::AcqRel, Ordering::Relaxed).is_ok()
                    && let Some(seen) = self.not_current(i)
                {
                    return (i, seen);
                }
            }
            for i in 0..BUFFERS {
                let holder = self.claim_word(i).load(Ordering::Acquire);
                if self.value_offset(i) != current
                    && holder != 0
                    && crate::process_identity::holder_gone(holder)
                    && self.claim_word(i).compare_exchange(holder, me, Ordering::AcqRel, Ordering::Relaxed).is_ok()
                    && let Some(seen) = self.not_current(i)
                {
                    return (i, seen);
                }
            }
            std::thread::yield_now();
        }
    }

    /// The head, once buffer `i`, just claimed, is confirmed not current; or
    /// `None`, with the claim given back, when a publish had made it
    /// current.
    fn not_current(&self, i: usize) -> Option<u64> {
        let seen = self.head().load(Ordering::Acquire);
        if value_at(seen) == self.value_offset(i) {
            self.release(i);
            return None;
        }
        Some(seen)
    }

    /// The buffer the head names.
    #[cfg(test)]
    fn current(&self) -> usize {
        let at = value_at(self.head().load(Ordering::Acquire));
        (0..BUFFERS).find(|&i| self.value_offset(i) == at).expect("the head names a buffer")
    }
}

/// A handle's owned pair is freed when it drops, for another handle to take.
/// A forked child's copy of the handle frees nothing: the pair is its
/// parent's.
impl Drop for CellCore {
    fn drop(&mut self) {
        let owner = *self.owner.get_mut();
        if owner != 0 && owner != NO_OWNER && *self.owner_pid.get_mut() == crate::peer_directory::current_pid() {
            let a = *self.pair.get_mut();
            for buffer in [a, a + 1] {
                self.release(buffer);
            }
        }
    }
}

/// A handle on one cell. The core leads, so a read touches the handle's
/// first line and an owner's write its second.
#[repr(C)]
pub struct SharedCell<T: Copy + 'static> {
    core: CellCore,
    ring_sidecar: Box<subetha_core::ObservationRing>,
    _file: File,
    mmap: MmapMut,
    _phantom: PhantomData<T>,
    header_sidecar: subetha_core::HandshakeHeader,
}

unsafe impl<T: Copy + Send + 'static> Send for SharedCell<T> {}
unsafe impl<T: Copy + Sync + 'static> Sync for SharedCell<T> {}

impl<T: Copy + Send + Sync + 'static> subetha_sidecar::AdaptiveInstance for SharedCell<T> {
    fn header(&self) -> &subetha_core::HandshakeHeader { &self.header_sidecar }
    fn ring(&self) -> &subetha_core::ObservationRing { &self.ring_sidecar }
    fn make_policy(&self) -> Box<dyn subetha_sidecar::Policy> {
        Box::new(subetha_sidecar::NoMigrationPolicy)
    }
}

impl<T: Copy + 'static> SharedCell<T> {
    /// Obtain the cell at `path`, initializing an empty one if the path
    /// does not yet exist and attaching to it if it does. Attaching
    /// leaves the current value and generation in place; a region built
    /// for a different payload type is a `LayoutMismatch`.
    /// [`reset`](Self::reset) reinitializes.
    pub fn create(path: impl AsRef<Path>) -> Result<Self, SharedCellError> {
        Self::check_layout()?;
        let (file, mmap) = crate::mmf_attach::create_or_attach(
            path.as_ref(),
            CELL_FILE_SIZE,
            |ptr| unsafe { init_region(ptr, size_of::<T>()) },
            |ptr| unsafe { u64::from((*(ptr as *const CellHeader)).magic) },
            u64::from(CELL_MAGIC),
        )
        .map_err(|e| crate::mmf_attach::attach_error(e, SharedCellError::LayoutMismatch))?;
        Self::from_region(file, mmap)
    }

    /// Truncate the cell at `path` and initialize an empty one,
    /// discarding whatever value a live peer holds. For a caller that
    /// knows it owns the path.
    pub fn reset(path: impl AsRef<Path>) -> Result<Self, SharedCellError> {
        Self::check_layout()?;
        let (file, mmap) = crate::mmf_attach::reset(path.as_ref(), CELL_FILE_SIZE, |ptr| unsafe {
            init_region(ptr, size_of::<T>())
        })?;
        Self::from_region(file, mmap)
    }

    /// Wrap an initialized region, refusing one built for a different
    /// payload type.
    fn from_region(file: File, mut mmap: MmapMut) -> Result<Self, SharedCellError> {
        let header = unsafe { &*(mmap.as_ptr() as *const CellHeader) };
        header_matches(header, size_of::<T>())?;
        // SAFETY: the mapping is a laid-out cell of CELL_FILE_SIZE bytes for
        // values of size_of::<T>(), held as long as the core.
        let core = unsafe { CellCore::new(mmap.as_mut_ptr(), size_of::<T>()) };
        Ok(Self {
            core,
            _file: file,
            mmap,
            _phantom: PhantomData,
            header_sidecar: subetha_core::HandshakeHeader::new(),
            ring_sidecar: Box::new(subetha_core::ObservationRing::new()),
        })
    }

    pub fn open(path: impl AsRef<Path>) -> Result<Self, SharedCellError> {
        Self::check_layout()?;
        let file = crate::region_file::open_existing(path.as_ref())?;
        long_enough(&file)?;
        let mmap = unsafe { MmapOptions::new().len(CELL_FILE_SIZE).map_mut(&file)? };
        Self::from_region(file, mmap)
    }

    fn check_layout() -> Result<(), SharedCellError> {
        if size_of::<T>() > PAYLOAD_BYTES {
            return Err(SharedCellError::PayloadTooLarge);
        }
        if align_of::<T>() > 8 {
            return Err(SharedCellError::PayloadTooLarge);
        }
        Ok(())
    }

    /// Make `value` current, after any publish that lands first, and
    /// answer its generation.
    #[inline(always)]
    pub fn set(&self, value: T) -> u64 {
        let generation = if self.core.owns() {
            // SAFETY: T is the core's value size and aligned to at most 8, per
            // `check_layout`, and the calling thread owns the pair.
            unsafe { self.core.set_owned_value(value) }
        } else {
            // The claim and the publish run out of line and the value is
            // stored here, so no call takes it and it stays where the caller
            // built it on the owner's path too.
            let (buffer, seen) = self.core.claim_for_write();
            Self::put(self.core.value(buffer), value);
            self.core.publish_claimed(buffer, seen)
        };
        if subetha_core::any_observer_armed() {
            self.observe(crate::sidecar_ops::cell::OP_SET, 0);
        }
        generation
    }

    /// Push an operation onto this handle's observation ring, out of line:
    /// an operation reaches here only while some observer in the process is
    /// armed, so the ring's pointer stays off the handle's hot lines.
    #[cold]
    #[inline(never)]
    fn observe(&self, op: u16, flags: u16) {
        self.ring_sidecar.push_op(op, flags);
    }

    /// Store `value` in the buffer at `buffer` as a `T`.
    #[inline(always)]
    fn put(buffer: *mut u8, value: T) {
        // SAFETY: the core hands a buffer of PAYLOAD_BYTES at a multiple of 8,
        // and `check_layout` holds T to that size and to an alignment of at
        // most 8.
        unsafe { crate::atomic_copy::store(buffer as *mut T, &value) };
    }

    /// Make `value` current if the cell's generation is still `seen`, the
    /// generation a [`load`](Self::load) answered: the new generation, or
    /// the generation another publish that landed first left, in which case
    /// `value` was never visible.
    pub fn compare_and_set(&self, seen: u64, value: T) -> Result<u64, u64> {
        let landed = if self.core.owns() {
            // SAFETY: T is the core's value size and aligned to at most 8, per
            // `check_layout`, and the calling thread owns the pair.
            unsafe { self.core.compare_and_set_owned_value(seen, value) }
        } else {
            // Out of line around the value, as in `set`.
            let (buffer, now) = self.core.claim_for_write();
            if generation_of(now) == seen {
                Self::put(self.core.value(buffer), value);
                self.core.land_claimed(buffer, now)
            } else {
                self.core.release(buffer);
                Err(generation_of(now))
            }
        };
        if subetha_core::any_observer_armed() {
            self.observe(crate::sidecar_ops::cell::OP_SET, if landed.is_ok() { 0 } else { 1 });
        }
        landed
    }

    /// The current value and the generation that made it current.
    #[inline]
    pub fn load(&self) -> (u64, T) {
        // SAFETY: the core was built for values of size_of::<T>() bytes, and
        // every write through this type stores a T.
        let (generation, retried, value) = unsafe { self.core.load_value::<T>() };
        if subetha_core::any_observer_armed() {
            self.observe(crate::sidecar_ops::cell::OP_GET, u16::from(retried));
        }
        (generation, value)
    }

    /// The current value.
    #[inline]
    pub fn get(&self) -> T {
        self.load().1
    }

    /// The generation: how many publishes have landed.
    pub fn version(&self) -> u64 {
        self.core.generation()
    }

    /// Non-blocking flush: schedules a writeback via the OS.
    /// Note: Windows is only partially async (sync to page cache,
    /// not to disk).
    pub fn flush_async(&self) -> Result<(), SharedCellError> {
        self.mmap.flush_async()?;
        Ok(())
    }

    pub fn flush(&self) -> Result<(), SharedCellError> {
        self.mmap.flush()?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicBool;
    use std::sync::Arc;
    use std::thread;

    /// The holder word of a pid no process holds, its start unknown: its
    /// holder's process is gone.
    const ABSENT_PID: u64 = 0x7FFF_FFF0;

    /// A file for one test, removed when the test ends; declared before the
    /// primitive that maps it so it drops after it.
    fn tmp(name: &str) -> crate::test_paths::TmpFile {
        crate::test_paths::TmpFile::new(format!("subetha-cell-{name}-{}.bin", std::process::id()))
    }

    /// The generation a compare-and-set that must land made, or a panic
    /// naming the generation that beat it.
    fn landed(outcome: Result<u64, u64>) -> u64 {
        match outcome {
            Ok(generation) => generation,
            Err(now) => panic!("the write lost to generation {now}"),
        }
    }

    /// The generation a compare-and-set that must lose was beaten by, or a
    /// panic naming the generation it made.
    fn lost(outcome: Result<u64, u64>) -> u64 {
        match outcome {
            Ok(generation) => panic!("the write landed as generation {generation}"),
            Err(now) => now,
        }
    }

    /// A write through a buffer claimed for it, whichever thread makes it.
    fn set_claimed<T: Copy>(c: &SharedCell<T>, value: T) -> u64 {
        // SAFETY: the copy writes size_of::<T>() bytes, the core's value size.
        unsafe { c.core.set_claimed(move |buffer| SharedCell::<T>::put(buffer, value)) }
    }

    /// The buffer the head names.
    fn current<T: Copy>(c: &SharedCell<T>) -> usize {
        c.core.current()
    }

    #[test]
    fn round_trip_simple_payload() {
        let p = tmp("round-trip");
        let c: SharedCell<u64> = SharedCell::create(&p).unwrap();
        c.set(42);
        assert_eq!(c.get(), 42);
        c.set(99);
        assert_eq!(c.get(), 99);
    }

    /// A second create attaches with the current value in place; reset
    /// is what strips it.
    #[test]
    fn second_create_attaches_and_keeps_the_value() {
        let p = tmp("attach");
        let c: SharedCell<u64> = SharedCell::create(&p).unwrap();
        c.set(777);

        let c2: SharedCell<u64> = SharedCell::create(&p).unwrap();
        assert_eq!(c2.get(), 777, "attach clobbered the value");

        // Windows refuses to truncate a mapped file, so every handle goes
        // before the reset.
        drop(c);
        drop(c2);
        let fresh: SharedCell<u64> = SharedCell::reset(&p).unwrap();
        assert_eq!(fresh.get(), 0, "reset left a value behind");
        drop(fresh);
    }

    /// Attaching with a different payload type is refused.
    #[test]
    fn create_refuses_a_mismatched_region() {
        let p = tmp("mismatch");
        let c: SharedCell<u64> = SharedCell::create(&p).unwrap();
        assert!(matches!(
            SharedCell::<u32>::create(&p),
            Err(SharedCellError::LayoutMismatch(crate::layout::Mismatch {
                field: "value size",
                found: 8,
                wanted: 4,
            })),
        ));
        drop(c);
    }

    /// A file laid out as a one-line cell is too short to be a cell.
    #[test]
    fn a_one_line_region_is_refused() {
        let p = tmp("one-line");
        std::fs::write(&p, [0u8; 64]).unwrap();
        assert!(matches!(
            SharedCell::<u64>::open(&p),
            Err(SharedCellError::LayoutMismatch(crate::layout::Mismatch { field: "length", .. })),
        ));
    }

    #[test]
    fn cross_handle_visibility() {
        let p = tmp("cross-handle");
        let writer: SharedCell<u64> = SharedCell::create(&p).unwrap();
        let reader: SharedCell<u64> = SharedCell::open(&p).unwrap();
        writer.set(0xDEAD_BEEF);
        assert_eq!(reader.get(), 0xDEAD_BEEF);
    }

    #[test]
    fn the_generation_advances_by_one_on_each_set() {
        let p = tmp("version");
        let c: SharedCell<u32> = SharedCell::create(&p).unwrap();
        let v0 = c.version();
        assert_eq!(c.set(1), v0 + 1);
        assert_eq!(c.set(2), v0 + 2);
        assert_eq!(c.version(), v0 + 2);
        assert_eq!(c.load(), (v0 + 2, 2));
    }

    /// A typed read of a value the read line caches takes the cached copy
    /// only while the head still names it: once writes refill the buffer the
    /// cached head names, under a newer generation, the read returns the
    /// current value and caches the head it found.
    #[test]
    fn a_read_from_a_stale_confirmed_head_returns_the_current_value() {
        let p = tmp("confirmed");
        let reader: SharedCell<[u64; 3]> = SharedCell::create(&p).unwrap();
        let writer: SharedCell<[u64; 3]> = SharedCell::open(&p).unwrap();
        writer.set([1; 3]);
        assert_eq!(reader.load(), (1, [1; 3]));
        let confirmed = reader.core.read.cached.load(Ordering::Acquire);
        assert_eq!(confirmed, reader.core.head().load(Ordering::Acquire), "the read cached the head it found");
        writer.set([2; 3]);
        writer.set([3; 3]);
        let head = writer.core.head().load(Ordering::Acquire);
        assert_eq!(value_at(head), value_at(confirmed), "the owner's pair refilled the cached head's buffer");
        assert_ne!(head, confirmed);
        assert_eq!(reader.load(), (3, [3; 3]));
        assert_eq!(reader.core.read.cached.load(Ordering::Acquire), head, "the read cached the new head");
    }

    /// The largest value a cell holds goes through the read line's cache
    /// too: a read caches the head it found, a repeat read takes the cache,
    /// and a write elsewhere sends the next read back to the region.
    #[test]
    fn a_value_of_the_largest_size_is_cached() {
        let p = tmp("cache-largest");
        let reader: SharedCell<[u64; 7]> = SharedCell::create(&p).unwrap();
        let writer: SharedCell<[u64; 7]> = SharedCell::open(&p).unwrap();
        let first: [u64; 7] = core::array::from_fn(|i| 0x0101_0101_0101_0101 * (i as u64 + 1));
        writer.set(first);
        assert_eq!(reader.load(), (1, first));
        let head = reader.core.head().load(Ordering::Acquire);
        assert_eq!(reader.core.read.cached.load(Ordering::Acquire), head, "the read cached the head it found");
        assert_eq!(reader.get(), first, "a repeat read takes the cache");
        let second: [u64; 7] = core::array::from_fn(|i| !(i as u64));
        writer.set(second);
        assert_eq!(reader.load(), (2, second));
        assert_eq!(reader.core.read.cached.load(Ordering::Acquire), writer.core.head().load(Ordering::Acquire));
    }

    #[test]
    fn disk_persistence_survives_reopen() {
        let p = tmp("disk-persist");
        {
            let c: SharedCell<u64> = SharedCell::create(&p).unwrap();
            c.set(7777);
            c.flush().unwrap();
        }
        let c2: SharedCell<u64> = SharedCell::open(&p).unwrap();
        assert_eq!(c2.get(), 7777);
    }

    #[test]
    fn open_rejects_wrong_payload_size() {
        let p = tmp("wrong-size");
        let _c: SharedCell<u64> = SharedCell::create(&p).unwrap();
        match SharedCell::<u32>::open(&p) {
            Err(SharedCellError::LayoutMismatch(crate::layout::Mismatch {
                field: "value size",
                found: 8,
                wanted: 4,
            })) => {}
            other => panic!("expected a value size of 8 where 4 was asked for, got {:?}", other.as_ref().err()),
        }
    }

    #[test]
    fn struct_payload_round_trip() {
        #[derive(Clone, Copy, Debug, PartialEq)]
        #[repr(C)]
        struct Point { x: f64, y: f64, z: f64 }
        let p = tmp("struct");
        let c: SharedCell<Point> = SharedCell::create(&p).unwrap();
        let pt = Point { x: 1.0, y: 2.0, z: 3.0 };
        c.set(pt);
        assert_eq!(c.get(), pt);
    }

    #[test]
    fn concurrent_readers_during_writes() {
        let p = tmp("concurrent-rw");
        let c: Arc<SharedCell<u64>> = Arc::new(SharedCell::create(&p).unwrap());
        c.set(0);
        let writer_c = c.clone();
        let writer = thread::spawn(move || {
            for i in 1..1000u64 {
                writer_c.set(i);
            }
            999u64
        });
        let mut handles = vec![];
        for _ in 0..4 {
            let reader_c = c.clone();
            handles.push(thread::spawn(move || {
                let mut last = 0u64;
                for _ in 0..1000 {
                    let v = reader_c.get();
                    // One writer publishes in order, so a reader never sees
                    // the value go back.
                    assert!(v >= last, "torn read detected: v={v} last={last}");
                    last = v;
                }
            }));
        }
        let final_w = writer.join().unwrap();
        for h in handles { h.join().unwrap(); }
        assert!(c.get() >= final_w);
    }

    /// Writers through their own handles publish at once while readers read;
    /// every read sees one writer's whole value, never parts of two.
    #[test]
    fn concurrent_writers_never_tear_a_value() {
        const READERS: usize = 2;
        let p = tmp("concurrent-writers");
        let first: SharedCell<[u64; 6]> = SharedCell::create(&p).unwrap();
        let stop = Arc::new(AtomicBool::new(false));
        // Each reader counts itself in after its first read, and the writers
        // start once every reader has, so every write lands while they read.
        let reading = Arc::new(AtomicUsize::new(0));
        let readers: Vec<_> = (0..READERS)
            .map(|_| {
                let cell: SharedCell<[u64; 6]> = SharedCell::open(&p).unwrap();
                let (stop, reading) = (Arc::clone(&stop), Arc::clone(&reading));
                thread::spawn(move || {
                    let mut reads = 0u64;
                    loop {
                        let v = cell.get();
                        assert!(v.iter().all(|&x| x == v[0]), "a read mixed two writes: {v:?}");
                        reads += 1;
                        if reads == 1 {
                            reading.fetch_add(1, Ordering::AcqRel);
                        }
                        if stop.load(Ordering::Acquire) {
                            return reads;
                        }
                    }
                })
            })
            .collect();
        let writers: Vec<_> = (1..=4u64)
            .map(|w| {
                let cell: SharedCell<[u64; 6]> = SharedCell::open(&p).unwrap();
                let reading = Arc::clone(&reading);
                thread::spawn(move || {
                    while reading.load(Ordering::Acquire) < READERS {
                        thread::yield_now();
                    }
                    for i in 0..20_000u64 {
                        cell.set([w << 32 | i; 6]);
                    }
                })
            })
            .collect();
        for w in writers {
            w.join().unwrap();
        }
        stop.store(true, Ordering::Release);
        for r in readers {
            r.join().unwrap();
        }
        assert_eq!(first.version(), 4 * 20_000, "every publish landed once");
    }

    /// Readers sharing one handle, so their cache refills race, read while
    /// writers through their own handles publish values the cache holds:
    /// every read is one writer's whole value, no read goes back to an
    /// earlier value of a writer than one it already saw, and a read after
    /// the writers stop serves the last value.
    #[test]
    fn readers_sharing_a_handle_never_tear_or_go_back() {
        const READERS: usize = 4;
        const WRITES: u64 = 20_000;
        let p = tmp("shared-handle-readers");
        let shared: Arc<SharedCell<[u64; 4]>> = Arc::new(SharedCell::create(&p).unwrap());
        let stop = Arc::new(AtomicBool::new(false));
        let reading = Arc::new(AtomicUsize::new(0));
        let readers: Vec<_> = (0..READERS)
            .map(|_| {
                let (cell, stop, reading) = (Arc::clone(&shared), Arc::clone(&stop), Arc::clone(&reading));
                thread::spawn(move || {
                    let mut last = [0u64; 5];
                    let mut reads = 0u64;
                    loop {
                        let v = cell.get();
                        assert!(v.iter().all(|&x| x == v[0]), "a read mixed two writes: {v:?}");
                        let (w, i) = ((v[0] >> 32) as usize, v[0] & 0xFFFF_FFFF);
                        assert!(i >= last[w], "writer {w}'s value went back from {} to {i}", last[w]);
                        last[w] = i;
                        reads += 1;
                        if reads == 1 {
                            reading.fetch_add(1, Ordering::AcqRel);
                        }
                        if stop.load(Ordering::Acquire) {
                            return reads;
                        }
                    }
                })
            })
            .collect();
        let writers: Vec<_> = (1..=4u64)
            .map(|w| {
                let cell: SharedCell<[u64; 4]> = SharedCell::open(&p).unwrap();
                let reading = Arc::clone(&reading);
                thread::spawn(move || {
                    while reading.load(Ordering::Acquire) < READERS {
                        thread::yield_now();
                    }
                    for i in 1..=WRITES {
                        cell.set([w << 32 | i; 4]);
                    }
                })
            })
            .collect();
        for w in writers {
            w.join().unwrap();
        }
        stop.store(true, Ordering::Release);
        for r in readers {
            r.join().unwrap();
        }
        assert_eq!(shared.version(), 4 * WRITES, "every publish landed once");
        assert_eq!(shared.get()[0] & 0xFFFF_FFFF, WRITES, "a read after the writers stopped serves the last value");
    }

    /// Of writers that all saw one generation, exactly one lands; the rest
    /// learn the generation it made and their values are never seen.
    #[test]
    fn compare_and_set_lands_exactly_one_of_racing_writers() {
        let p = tmp("cas-race");
        let first: SharedCell<u64> = SharedCell::create(&p).unwrap();
        let (seen, _) = first.load();
        let barrier = Arc::new(std::sync::Barrier::new(8));
        let racers: Vec<_> = (1..=8u64)
            .map(|w| {
                let cell: SharedCell<u64> = SharedCell::open(&p).unwrap();
                let barrier = Arc::clone(&barrier);
                thread::spawn(move || {
                    barrier.wait();
                    (w, cell.compare_and_set(seen, w))
                })
            })
            .collect();
        let outcomes: Vec<_> = racers.into_iter().map(|r| r.join().unwrap()).collect();
        let winners: Vec<u64> = outcomes.iter().filter(|(_, o)| o.is_ok()).map(|(w, _)| *w).collect();
        assert_eq!(winners.len(), 1, "exactly one lands: {outcomes:?}");
        for (w, outcome) in &outcomes {
            if *w == winners[0] {
                assert_eq!(landed(*outcome), seen + 1);
            } else {
                assert_eq!(lost(*outcome), seen + 1, "a loser learns the winner's generation");
            }
        }
        assert_eq!(first.load(), (seen + 1, winners[0]));
    }

    /// A write whose generation moved since it looked never lands.
    #[test]
    fn compare_and_set_on_a_stale_generation_changes_nothing() {
        let p = tmp("cas-stale");
        let a: SharedCell<u64> = SharedCell::create(&p).unwrap();
        let b: SharedCell<u64> = SharedCell::open(&p).unwrap();
        let (seen, _) = a.load();
        assert_eq!(landed(b.compare_and_set(seen, 5)), seen + 1);
        assert_eq!(lost(a.compare_and_set(seen, 6)), seen + 1);
        assert_eq!(a.load(), (seen + 1, 5));
    }

    /// Buffers claimed by a process that is gone are taken over, so a
    /// claiming write lands although every buffer but the current one is
    /// claimed; and a handle whose first writer finds no free pair claims.
    #[test]
    fn a_write_takes_over_buffers_whose_writers_are_gone() {
        let p = tmp("dead-claims");
        let c: SharedCell<u64> = SharedCell::create(&p).unwrap();
        set_claimed(&c, 1);
        let now = current(&c);
        for i in (0..BUFFERS).filter(|&i| i != now) {
            c.core.claim_word(i).store(ABSENT_PID, Ordering::Release);
        }
        assert_eq!(set_claimed(&c, 2), 2, "the write lands on a buffer taken over");
        assert_eq!(c.get(), 2);
        let still_dead = (0..BUFFERS).filter(|&i| c.core.claim_word(i).load(Ordering::Acquire) == ABSENT_PID).count();
        assert_eq!(still_dead, BUFFERS - 2, "one dead claim was taken over and freed, the rest stay");

        let now = current(&c);
        for i in (0..BUFFERS).filter(|&i| i != now) {
            c.core.claim_word(i).store(ABSENT_PID, Ordering::Release);
        }
        assert_eq!(c.set(3), 3, "a first writer with no free pair claims instead");
        assert_eq!(c.core.owner.load(Ordering::Relaxed), NO_OWNER);
        assert_eq!(c.get(), 3);
    }

    /// A claim holding this process's pid with another start was left by a
    /// process that held the pid before this one: gone, though the pid is
    /// alive, so a claiming write takes it over.
    #[test]
    fn a_claim_under_a_reused_pid_is_taken_over() {
        let p = tmp("reused-claims");
        let c: SharedCell<u64> = SharedCell::create(&p).unwrap();
        set_claimed(&c, 1);
        let me = crate::process_identity::current();
        let stamp = crate::process_identity::holder_stamp(me);
        assert_ne!(stamp, 0, "this process reads its own start");
        let earlier = crate::process_identity::holder(crate::process_identity::holder_pid(me), stamp.wrapping_add(1).max(1));
        let now = current(&c);
        for i in (0..BUFFERS).filter(|&i| i != now) {
            c.core.claim_word(i).store(earlier, Ordering::Release);
        }
        assert_eq!(set_claimed(&c, 2), 2, "the write lands on a buffer taken over");
        assert_eq!(c.get(), 2);
        let still_held = (0..BUFFERS).filter(|&i| c.core.claim_word(i).load(Ordering::Acquire) == earlier).count();
        assert_eq!(still_held, BUFFERS - 2, "one claim was taken over and freed, the rest stay");
    }

    /// With both hot buffers held by writers mid-copy, a claiming write lands
    /// in an overflow line and reads find it there; once a hot buffer is
    /// free the next lands beside the head again.
    #[test]
    fn a_write_overflows_while_both_hot_buffers_are_held() {
        let p = tmp("overflow");
        let c: SharedCell<[u64; 3]> = SharedCell::create(&p).unwrap();
        let me = crate::process_identity::current();
        let spare = 1 - current(&c);
        c.core.claim_word(spare).store(me, Ordering::Release);
        let other: SharedCell<[u64; 3]> = SharedCell::open(&p).unwrap();
        assert_eq!(set_claimed(&other, [7; 3]), 1);
        let landed = current(&other);
        assert!(landed >= HOT_BUFFERS, "the write took overflow buffer {landed}");
        assert_eq!(c.load(), (1, [7; 3]), "a read finds the value in the overflow line");
        c.core.release(spare);
        assert_eq!(set_claimed(&other, [8; 3]), 2);
        assert!(current(&c) < HOT_BUFFERS, "a free hot buffer is taken first");
        assert_eq!(c.get(), [8; 3]);
    }

    /// The first thread to write through a handle owns the hot pair while it
    /// is free, writes alternate within the pair, and dropping the handle
    /// frees it.
    #[test]
    fn the_first_writing_thread_owns_the_hot_pair_and_writes_alternate_within_it() {
        let p = tmp("owned");
        let c: SharedCell<u64> = SharedCell::create(&p).unwrap();
        let me = crate::process_identity::current();
        assert_eq!(c.set(1), 1);
        assert!((0..HOT_BUFFERS).all(|i| c.core.claim_word(i).load(Ordering::Acquire) == me), "the hot pair is held");
        let first = current(&c);
        assert_eq!(c.set(2), 2);
        let second = current(&c);
        assert!(first < HOT_BUFFERS && second < HOT_BUFFERS && first != second, "{first} then {second}");
        assert_eq!(c.set(3), 3);
        assert_eq!(current(&c), first);
        assert_eq!(c.load(), (3, 3));
        drop(c);
        let again: SharedCell<u64> = SharedCell::open(&p).unwrap();
        assert!((0..BUFFERS).all(|i| again.core.claim_word(i).load(Ordering::Acquire) == 0), "a dropped handle frees its pair");
        assert_eq!(again.get(), 3);
    }

    /// Another thread writing through the owner's handle claims a buffer
    /// outside the pair for its write and frees it, and the owner's writes
    /// go on through the pair.
    #[test]
    fn another_thread_writing_through_the_owners_handle_claims_a_buffer() {
        let p = tmp("owner-and-other");
        let c: Arc<SharedCell<u64>> = Arc::new(SharedCell::create(&p).unwrap());
        assert_eq!(c.set(1), 1);
        let shared = Arc::clone(&c);
        let (generation, landed) = thread::spawn(move || (shared.set(2), current(&shared))).join().unwrap();
        assert_eq!(generation, 2);
        assert!(landed >= HOT_BUFFERS, "the other thread's write took buffer {landed}, outside the hot pair");
        assert_eq!(c.get(), 2);
        assert_eq!(c.set(3), 3);
        assert!(current(&c) < HOT_BUFFERS, "the owner writes through its pair");
        assert!((HOT_BUFFERS..BUFFERS).all(|i| c.core.claim_word(i).load(Ordering::Acquire) == 0), "the claimed buffer was freed");
        assert_eq!(c.get(), 3);
    }

    /// Pairs come only from the owned buffers: past them a first writer
    /// claims, and the buffers past the owned ones stay free for it.
    #[test]
    fn owned_pairs_stop_short_of_the_buffers_claimed_writes_need() {
        let p = tmp("pair-budget");
        let first: SharedCell<u64> = SharedCell::create(&p).unwrap();
        let path = p.to_path_buf();
        let handles: Vec<SharedCell<u64>> = (1..=OWNED_BUFFERS / 2 + 1)
            .map(|w| {
                let path = path.clone();
                thread::spawn(move || {
                    let cell: SharedCell<u64> = SharedCell::open(&path).unwrap();
                    assert_eq!(cell.set(w as u64), w as u64);
                    cell
                })
                .join()
                .unwrap()
            })
            .collect();
        let me = crate::process_identity::current();
        assert!((0..OWNED_BUFFERS).all(|i| first.core.claim_word(i).load(Ordering::Acquire) == me), "every pair is owned");
        assert!((OWNED_BUFFERS..BUFFERS).all(|i| first.core.claim_word(i).load(Ordering::Acquire) == 0), "no pair reaches past the owned buffers");
        let last = handles.last().expect("the handles");
        assert_eq!(last.core.owner.load(Ordering::Relaxed), NO_OWNER, "the last writer found no pair and claimed");
        assert_eq!(first.get(), (OWNED_BUFFERS / 2 + 1) as u64);
        drop(handles);
        assert!((0..BUFFERS).all(|i| first.core.claim_word(i).load(Ordering::Acquire) == 0), "every pair is freed with its handle");
    }

    /// A writer stalled mid-copy holds only its own buffer: readers keep
    /// reading the current value and other writers keep publishing.
    #[test]
    fn a_stalled_writer_holds_up_no_reader_and_no_writer() {
        let p = tmp("stalled");
        let c: SharedCell<u64> = SharedCell::create(&p).unwrap();
        c.set(10);
        let (stalled, _) = c.core.claim();
        let other: SharedCell<u64> = SharedCell::open(&p).unwrap();
        assert_eq!(other.get(), 10, "a read does not wait on the stalled claim");
        assert_eq!(other.set(11), 2, "a write lands on another buffer");
        assert_eq!(c.get(), 11);
        c.core.release(stalled);
    }

    #[test]
    fn payload_too_large_at_create() {
        #[allow(dead_code)] // size_of<Big> is the test signal, not the field
        struct Big([u8; PAYLOAD_BYTES + 1]);
        impl Copy for Big {}
        impl Clone for Big { fn clone(&self) -> Self { *self } }
        let p = tmp("too-large");
        match SharedCell::<Big>::create(&p) {
            Err(SharedCellError::PayloadTooLarge) => {}
            other => panic!("expected PayloadTooLarge, got {:?}", other.as_ref().err()),
        }
    }
}
