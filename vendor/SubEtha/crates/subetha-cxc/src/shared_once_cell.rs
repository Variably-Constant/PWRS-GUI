//! `SharedOnceCell<T>` - cross-process set-once cell.
//!
//! A caller that finds the cell empty produces its own value, copies it
//! into a free slot and marks the slot filled; one swap of the cell's
//! winner word from zero to that slot publishes it, and from then on every
//! caller reads the winner's slot. No caller waits on another's producer:
//! until a winner is published the cell reads as empty, and a caller whose
//! swap loses reads the value that won. A producer can run once per caller
//! that finds the cell empty.
//!
//! A slot whose copier is gone, judged by its pid and process start (see
//! `process_identity`), is freed by the next caller that needs one, and a
//! filled slot nobody published is published by the next caller that finds
//! every slot taken. A caller waits only while every slot is mid-copy by a
//! live caller.
//!
//! A producer that must run at most once at a time, such as a configuration
//! fetch, goes through [`SharedOnceCell::get_or_fetch`] instead: it runs only
//! under the cell's one fetch claim, and returns [`Fetch::Pending`] at once
//! to any other caller. A fetch claim whose holder is gone is broken by the
//! next caller.

use std::fs::File;
use std::marker::PhantomData;
use std::mem::{align_of, size_of};
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};

use memmap2::{MmapMut, MmapOptions};

use crate::process_identity;

/// "ONC4": a header line holding the inline slot, then [`ONCE_SLOTS`] slot
/// lines.
pub const ONCE_MAGIC: u32 = 0x4F4E_4334;
/// Largest value a cell holds.
pub const ONCE_PAYLOAD_BYTES: usize = 56;
/// Largest value the header line's inline slot holds; a cell of a value
/// this size or smaller tries the inline slot before the slot lines.
pub const ONCE_INLINE_BYTES: usize = 32;
/// Slot lines a value can be copied into before one is published, beside
/// the inline slot.
pub const ONCE_SLOTS: usize = 8;

/// What [`SharedOnceCellDyn::state`] returns while no value is published
/// and no caller is copying or fetching one.
pub const STATE_EMPTY: u8 = 0;
/// No value published yet, and a caller is copying or fetching one.
pub const STATE_INITIALIZING: u8 = 1;
/// A value is published and stays.
pub const STATE_INITIALIZED: u8 = 2;

/// A slot's state once its value is whole. No holder word takes it: that
/// would be pid `u32::MAX`, which no system hands out.
const FILLED: u64 = u64::MAX;

#[repr(C, align(64))]
pub struct OnceHeader {
    pub magic: u32,
    /// Bytes of the value.
    pub size: u32,
    /// Zero until a value is published, then 1 for the inline slot or
    /// 2 + i for slot line i. Written once.
    pub winner: AtomicU64,
    /// The holder word of the caller running a fetch under the cell's
    /// fetch claim, or zero.
    pub fetcher: AtomicU64,
    /// The inline slot's state, as a slot line's.
    pub inline_state: AtomicU64,
    /// The inline slot's value, for a cell of at most
    /// [`ONCE_INLINE_BYTES`].
    pub inline_payload: [u8; ONCE_INLINE_BYTES],
}

#[repr(C, align(64))]
pub struct OnceSlot {
    /// Zero while free, the copier's holder word while it copies, and
    /// [`u64::MAX`] once the value in the slot is whole.
    pub state: AtomicU64,
    pub payload: [u8; ONCE_PAYLOAD_BYTES],
}

const _: () = assert!(size_of::<OnceHeader>() == 64 && size_of::<OnceSlot>() == 64);

/// Bytes a cell's file holds: the header line, then the slots.
pub const ONCE_FILE_SIZE: usize = size_of::<OnceHeader>() + ONCE_SLOTS * size_of::<OnceSlot>();

/// What [`SharedOnceCell::get_or_fetch`] returns.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Fetch<T> {
    /// The published value: this caller's fetch, or one published before
    /// or beside it.
    Ready(T),
    /// Another caller holds the fetch claim and has not published yet.
    Pending,
}

#[derive(Debug)]
pub enum SharedOnceError {
    LayoutMismatch(crate::layout::Mismatch),
    PayloadTooLarge,
    /// The operating system refused to open, size or map the file; the
    /// error carries its code and message.
    IoError(std::io::Error),
}

impl std::fmt::Display for SharedOnceError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::LayoutMismatch(m) => write!(f, "a cell of another shape: {m}"),
            Self::PayloadTooLarge => {
                write!(f, "the value is empty, larger than the cell holds, or aligned past 8")
            }
            Self::IoError(e) => write!(f, "io: {e}"),
        }
    }
}

impl std::error::Error for SharedOnceError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::IoError(e) => Some(e),
            Self::LayoutMismatch(_) | Self::PayloadTooLarge => None,
        }
    }
}

impl From<std::io::Error> for SharedOnceError {
    fn from(e: std::io::Error) -> Self { Self::IoError(e) }
}

/// The first of the header's magic and value size that differs from a
/// cell of `value_bytes` bytes. The typed cell and the runtime-sized one
/// both check through here, so they refuse the same regions.
fn header_matches(header: &OnceHeader, value_bytes: usize) -> Result<(), SharedOnceError> {
    crate::layout::Mismatch::first(&[
        ("magic", u64::from(header.magic), u64::from(ONCE_MAGIC)),
        ("value size", u64::from(header.size), value_bytes as u64),
    ])
    .map_err(SharedOnceError::LayoutMismatch)
}

/// The file, refused when it is shorter than a cell.
fn long_enough(file: &File) -> Result<(), SharedOnceError> {
    let len = file.metadata()?.len();
    if len < ONCE_FILE_SIZE as u64 {
        return Err(SharedOnceError::LayoutMismatch(crate::layout::Mismatch::new(
            "length",
            len,
            ONCE_FILE_SIZE as u64,
        )));
    }
    Ok(())
}

/// Map the cell at `path`, laying out an empty one of `value_bytes` bytes
/// when none is published there.
fn create_region(path: &Path, value_bytes: usize) -> Result<(File, MmapMut), SharedOnceError> {
    crate::mmf_attach::create_or_attach(
        path,
        ONCE_FILE_SIZE,
        |ptr| unsafe { init_region(ptr, value_bytes) },
        |ptr| unsafe { u64::from((*(ptr as *const OnceHeader)).magic) },
        u64::from(ONCE_MAGIC),
    )
    .map_err(|e| crate::mmf_attach::attach_error(e, SharedOnceError::LayoutMismatch))
}

/// Map the cell another process created at `path`.
fn open_region(path: &Path) -> Result<(File, MmapMut), SharedOnceError> {
    let file = crate::region_file::open_existing(path)?;
    long_enough(&file)?;
    let mmap = unsafe { MmapOptions::new().len(ONCE_FILE_SIZE).map_mut(&file)? };
    Ok((file, mmap))
}

/// Lay out an empty cell: the zeroed region already has no winner, no
/// fetch claim and every slot free, so only the size and the magic are
/// written.
///
/// # Safety
/// `ptr` addresses at least [`ONCE_FILE_SIZE`] writable zeroed bytes.
unsafe fn init_region(ptr: *mut u8, value_bytes: usize) {
    let hdr = ptr as *mut OnceHeader;
    unsafe {
        (*hdr).size = value_bytes as u32;
        std::ptr::write_volatile(&raw mut (*hdr).magic, ONCE_MAGIC);
    }
}

/// Wait between two looks at slots every one of which a live caller is
/// copying into: a yield for each of the first 64 looks, then a 500
/// microsecond sleep, so a copier the system has stopped costs no
/// processor.
fn pause(looks: u32) {
    if looks < 64 {
        std::thread::yield_now();
    } else {
        std::thread::sleep(std::time::Duration::from_micros(500));
    }
}

/// The protocol both cells run over the mapping at `base`.
struct Cell {
    base: *mut u8,
    value_bytes: usize,
}

impl Cell {
    fn header(&self) -> &OnceHeader {
        unsafe { &*(self.base as *const OnceHeader) }
    }

    /// The slots this cell copies into, in the order a publish tries them:
    /// id 0 is the header's inline slot, for a value of at most
    /// [`ONCE_INLINE_BYTES`], and id `i + 1` is slot line `i`.
    fn ids(&self) -> std::ops::Range<usize> {
        let first = if self.value_bytes <= ONCE_INLINE_BYTES { 0 } else { 1 };
        first..ONCE_SLOTS + 1
    }

    fn slot_state(&self, id: usize) -> &AtomicU64 {
        debug_assert!(id <= ONCE_SLOTS);
        if id == 0 {
            return &self.header().inline_state;
        }
        unsafe { &*(self.base.add(size_of::<OnceHeader>() + (id - 1) * size_of::<OnceSlot>()) as *const AtomicU64) }
    }

    fn payload(&self, id: usize) -> *mut u8 {
        debug_assert!(id <= ONCE_SLOTS);
        if id == 0 {
            return unsafe { self.base.add(std::mem::offset_of!(OnceHeader, inline_payload)) };
        }
        unsafe { self.base.add(size_of::<OnceHeader>() + (id - 1) * size_of::<OnceSlot>() + size_of::<AtomicU64>()) }
    }

    /// Bytes slot `id`'s payload holds.
    fn room(id: usize) -> usize {
        if id == 0 { ONCE_INLINE_BYTES } else { ONCE_PAYLOAD_BYTES }
    }

    /// The slot holding the published value, if one is published.
    #[inline]
    fn winner(&self) -> Option<usize> {
        match self.header().winner.load(Ordering::Acquire) {
            0 => None,
            w => Some(w as usize - 1),
        }
    }

    fn state(&self) -> u8 {
        let header = self.header();
        if header.winner.load(Ordering::Acquire) != 0 {
            return STATE_INITIALIZED;
        }
        if header.fetcher.load(Ordering::Acquire) != 0
            || (0..=ONCE_SLOTS).any(|id| self.slot_state(id).load(Ordering::Acquire) != 0)
        {
            return STATE_INITIALIZING;
        }
        STATE_EMPTY
    }

    /// Copy the value in slot `i` into `out`, up to the cell's size.
    fn read(&self, i: usize, out: &mut [u8]) {
        let len = self.value_bytes.min(out.len());
        unsafe { std::ptr::copy_nonoverlapping(self.payload(i), out.as_mut_ptr(), len) };
    }

    /// Publish `value` unless a value is published first. Returns whether
    /// this caller's value won and the slot holding the published value.
    fn publish(&self, value: &[u8]) -> (bool, usize) {
        let me = process_identity::current();
        let mut looks = 0u32;
        loop {
            if let Some(w) = self.winner() {
                return (false, w);
            }
            for id in self.ids() {
                let state = self.slot_state(id);
                if state.load(Ordering::Relaxed) == 0
                    && state.compare_exchange(0, me, Ordering::AcqRel, Ordering::Relaxed).is_ok()
                {
                    // SAFETY: this caller holds slot `id` from the swap until
                    // the store of FILLED, and nothing reads it before then;
                    // the cell's value fits every slot `ids` names.
                    unsafe {
                        let dst = self.payload(id);
                        let len = self.value_bytes.min(value.len());
                        std::ptr::copy_nonoverlapping(value.as_ptr(), dst, len);
                        std::ptr::write_bytes(dst.add(len), 0, Self::room(id) - len);
                    }
                    state.store(FILLED, Ordering::Release);
                    return self.choose(id);
                }
            }
            if let Some(id) = self.ids().find(|&id| self.slot_state(id).load(Ordering::Acquire) == FILLED) {
                let (_, w) = self.choose(id);
                return (false, w);
            }
            let mut freed = false;
            for id in self.ids() {
                let state = self.slot_state(id);
                let copier = state.load(Ordering::Acquire);
                if copier != 0 && copier != FILLED && copier != me && process_identity::holder_gone(copier) {
                    freed |= state.compare_exchange(copier, 0, Ordering::AcqRel, Ordering::Relaxed).is_ok();
                }
            }
            if !freed {
                pause(looks);
                looks = looks.saturating_add(1);
            }
        }
    }

    /// Publish the filled slot `i` unless another is published first:
    /// whether `i` won, and the slot that did.
    fn choose(&self, i: usize) -> (bool, usize) {
        match self.header().winner.compare_exchange(0, i as u64 + 1, Ordering::AcqRel, Ordering::Acquire) {
            Ok(_) => (true, i),
            Err(w) => (false, w as usize - 1),
        }
    }

    /// Take the cell's fetch claim in the name of the holder word `word`:
    /// `true` once it is taken, `false` while a live holder has it. A claim
    /// whose holder is gone is broken and taken.
    fn claim_as(&self, word: u64) -> bool {
        let fetcher = &self.header().fetcher;
        loop {
            match fetcher.compare_exchange(0, word, Ordering::AcqRel, Ordering::Acquire) {
                Ok(_) => return true,
                Err(held) => {
                    if held == word || !process_identity::holder_gone(held) {
                        return false;
                    }
                    if fetcher.compare_exchange(held, 0, Ordering::AcqRel, Ordering::Acquire).is_err() {
                        return false;
                    }
                }
            }
        }
    }
}

/// `SharedOnceCellDyn::try_get` for a winner word other than the first
/// slot's: `false` while nothing is published, else the value a contended
/// publish left in a later slot, copied into `out`.
#[cold]
#[inline(never)]
fn try_get_unless_first(cell: &Cell, w: u64, out: &mut [u8]) -> bool {
    if w == 0 {
        return false;
    }
    cell.read(w as usize - 1, out);
    true
}

/// Gives the fetch claim back when dropped, on the normal path and when
/// a fetch panics.
struct FetchClaim<'a>(&'a AtomicU64);

impl Drop for FetchClaim<'_> {
    fn drop(&mut self) {
        self.0.store(0, Ordering::Release);
    }
}

pub struct SharedOnceCell<T: Copy + 'static> {
    _file: File,
    mmap: MmapMut,
    _phantom: PhantomData<T>,
    header_sidecar: subetha_core::HandshakeHeader,
    ring_sidecar: Box<subetha_core::ObservationRing>,
}

unsafe impl<T: Copy + Send + 'static> Send for SharedOnceCell<T> {}
unsafe impl<T: Copy + Sync + 'static> Sync for SharedOnceCell<T> {}

impl<T: Copy + Send + Sync + 'static> subetha_sidecar::AdaptiveInstance for SharedOnceCell<T> {
    fn header(&self) -> &subetha_core::HandshakeHeader { &self.header_sidecar }
    fn ring(&self) -> &subetha_core::ObservationRing { &self.ring_sidecar }
    fn make_policy(&self) -> Box<dyn subetha_sidecar::Policy> {
        Box::new(subetha_sidecar::NoMigrationPolicy)
    }
}

impl<T: Copy + 'static> SharedOnceCell<T> {
    /// Obtain the cell at `path`, laying out an empty one when none is
    /// published there and attaching to it when one is. Attaching leaves a
    /// published value in place; a region built for a different value type
    /// is a `LayoutMismatch`. [`reset`](Self::reset) reinitializes.
    pub fn create(path: impl AsRef<Path>) -> Result<Self, SharedOnceError> {
        Self::check_layout()?;
        let (file, mmap) = create_region(path.as_ref(), size_of::<T>())?;
        Self::from_region(file, mmap)
    }

    /// Truncate the cell at `path` and lay out an empty one, discarding
    /// whatever value a live peer holds. For a caller that knows it owns
    /// the path.
    pub fn reset(path: impl AsRef<Path>) -> Result<Self, SharedOnceError> {
        Self::check_layout()?;
        let (file, mmap) = crate::mmf_attach::reset(path.as_ref(), ONCE_FILE_SIZE, |ptr| unsafe {
            init_region(ptr, size_of::<T>())
        })?;
        Self::from_region(file, mmap)
    }

    pub fn open(path: impl AsRef<Path>) -> Result<Self, SharedOnceError> {
        Self::check_layout()?;
        let (file, mmap) = open_region(path.as_ref())?;
        Self::from_region(file, mmap)
    }

    /// Wrap a laid-out region, refusing one built for a different value
    /// type.
    fn from_region(file: File, mmap: MmapMut) -> Result<Self, SharedOnceError> {
        let header = unsafe { &*(mmap.as_ptr() as *const OnceHeader) };
        header_matches(header, size_of::<T>())?;
        Ok(Self {
            _file: file, mmap, _phantom: PhantomData,
            header_sidecar: subetha_core::HandshakeHeader::new(),
            ring_sidecar: Box::new(subetha_core::ObservationRing::new()),
        })
    }

    fn check_layout() -> Result<(), SharedOnceError> {
        if size_of::<T>() > ONCE_PAYLOAD_BYTES || align_of::<T>() > 8 {
            return Err(SharedOnceError::PayloadTooLarge);
        }
        Ok(())
    }

    fn cell(&self) -> Cell {
        Cell { base: self.mmap.as_ptr() as *mut u8, value_bytes: size_of::<T>() }
    }

    /// The winner word an uncontended publish leaves: the inline slot's for
    /// a value of at most [`ONCE_INLINE_BYTES`], slot line 0's otherwise.
    const FIRST_WINNER: u64 = if size_of::<T>() <= ONCE_INLINE_BYTES { 1 } else { 2 };

    /// The value in slot `id`.
    fn value_at(&self, id: usize) -> T {
        unsafe { std::ptr::read_unaligned(self.cell().payload(id) as *const T) }
    }

    /// True when a value has been published.
    pub fn is_initialized(&self) -> bool {
        self.cell().winner().is_some()
    }

    /// The published value, or `None` before one is published. Never runs
    /// a producer. The slot an uncontended publish takes is read at a fixed
    /// address behind one compare.
    pub fn get(&self) -> Option<T> {
        let w = self.cell().header().winner.load(Ordering::Acquire);
        if w != Self::FIRST_WINNER {
            return self.get_unless_first(w);
        }
        let value = self.value_at(Self::FIRST_WINNER as usize - 1);
        self.ring_sidecar.push_op(crate::sidecar_ops::cell::OP_GET, 0);
        Some(value)
    }

    /// [`get`](Self::get) for a winner word other than the first slot's:
    /// nothing published yet, or a value a contended publish left in a
    /// later slot.
    #[cold]
    #[inline(never)]
    fn get_unless_first(&self, w: u64) -> Option<T> {
        if w == 0 {
            self.ring_sidecar.push_op(crate::sidecar_ops::cell::OP_GET, 2); // empty
            return None;
        }
        let value = self.value_at(w as usize - 1);
        self.ring_sidecar.push_op(crate::sidecar_ops::cell::OP_GET, 0);
        Some(value)
    }

    /// Publish `value` unless a value is already published: `true` when
    /// `value` is now the cell's value.
    pub fn set(&self, value: T) -> bool {
        let (won, _) = self.cell().publish(Self::bytes_of(&value));
        self.ring_sidecar.push_op(crate::sidecar_ops::cell::OP_SET, if won { 0 } else { 1 });
        won
    }

    /// The published value, or `init`'s, published. A caller that finds
    /// the cell empty runs `init` and publishes what it returns; a value
    /// published first, its own or another caller's, is the one every
    /// caller gets. No caller waits on another's `init`, so `init` can run
    /// once per caller that finds the cell empty.
    pub fn get_or_init<F: FnOnce() -> T>(&self, init: F) -> T {
        if self.cell().header().winner.load(Ordering::Acquire) == Self::FIRST_WINNER {
            let value = self.value_at(Self::FIRST_WINNER as usize - 1);
            self.ring_sidecar.push_op(crate::sidecar_ops::cell::OP_GET, 0);
            return value;
        }
        self.get_or_init_unless_first(init)
    }

    /// [`get_or_init`](Self::get_or_init) when the winner word does not name
    /// the first slot: nothing published yet, or a value a contended publish
    /// left in a later slot.
    #[cold]
    #[inline(never)]
    fn get_or_init_unless_first<F: FnOnce() -> T>(&self, init: F) -> T {
        if let Some(v) = self.get() {
            return v;
        }
        let value = init();
        let (won, i) = self.cell().publish(Self::bytes_of(&value));
        self.ring_sidecar.push_op(crate::sidecar_ops::cell::OP_SET, if won { 0 } else { 1 });
        if won { value } else { self.value_at(i) }
    }

    /// The published value, or `fetch`'s, run under the cell's fetch claim
    /// so that at most one caller fetches at a time across every process.
    /// For a caller that finds the claim held by a live caller it returns
    /// [`Fetch::Pending`] at once rather than waiting; a claim whose holder
    /// is gone is broken and taken. A `fetch` that panics gives the claim
    /// back as the panic leaves.
    pub fn get_or_fetch<F: FnOnce() -> T>(&self, fetch: F) -> Fetch<T> {
        if let Some(v) = self.get() {
            return Fetch::Ready(v);
        }
        let cell = self.cell();
        if !cell.claim_as(process_identity::current()) {
            return match self.get() {
                Some(v) => Fetch::Ready(v),
                None => Fetch::Pending,
            };
        }
        let claim = FetchClaim(&cell.header().fetcher);
        if let Some(i) = cell.winner() {
            drop(claim);
            return Fetch::Ready(self.value_at(i));
        }
        let value = fetch();
        let (won, i) = cell.publish(Self::bytes_of(&value));
        drop(claim);
        self.ring_sidecar.push_op(crate::sidecar_ops::cell::OP_SET, if won { 0 } else { 1 });
        Fetch::Ready(if won { value } else { self.value_at(i) })
    }

    fn bytes_of(value: &T) -> &[u8] {
        unsafe { std::slice::from_raw_parts(value as *const T as *const u8, size_of::<T>()) }
    }

    /// Non-blocking flush: schedules a writeback via the OS.
    /// Note: Windows is only partially async (sync to page cache,
    /// not to disk).
    pub fn flush_async(&self) -> Result<(), SharedOnceError> {
        self.mmap.flush_async()?;
        Ok(())
    }

    pub fn flush(&self) -> Result<(), SharedOnceError> {
        self.mmap.flush()?;
        Ok(())
    }
}

#[cfg(test)]
impl<T: Copy + 'static> SharedOnceCell<T> {
    /// Set the fetch claim's holder word, for a test that needs the claim
    /// another process left.
    pub(crate) fn set_fetch_claim_for_tests(&self, word: u64) {
        self.cell().header().fetcher.store(word, Ordering::Release);
    }
}

/// A once-cell whose value size is given at run time, for a caller with
/// no closures, whose value one producer makes: the caller that takes the
/// claim produces the value and publishes it, and every other caller is
/// refused the claim and reads the value with
/// [`try_get`](Self::try_get) once it is published, never waiting on the
/// producer. A claim whose process is gone is taken over by the next
/// caller that asks for it.
pub struct SharedOnceCellDyn {
    _file: File,
    mmap: MmapMut,
    value_bytes: usize,
}

unsafe impl Send for SharedOnceCellDyn {}
unsafe impl Sync for SharedOnceCellDyn {}

impl std::fmt::Debug for SharedOnceCellDyn {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SharedOnceCellDyn")
            .field("value_len", &self.value_bytes)
            .field("state", &self.state())
            .finish()
    }
}

impl SharedOnceCellDyn {
    /// Obtain the cell at `path` for a value of `value_bytes`, laying out
    /// an empty one when none is published there. A value past
    /// [`ONCE_PAYLOAD_BYTES`], or of zero bytes, is a `PayloadTooLarge`.
    pub fn create(path: impl AsRef<Path>, value_bytes: usize) -> Result<Self, SharedOnceError> {
        Self::check_size(value_bytes)?;
        let (file, mmap) = create_region(path.as_ref(), value_bytes)?;
        Self::from_region(file, mmap, value_bytes)
    }

    /// Attach to the cell another process created at `path`, whose value
    /// must be `value_bytes` long.
    pub fn open(path: impl AsRef<Path>, value_bytes: usize) -> Result<Self, SharedOnceError> {
        Self::check_size(value_bytes)?;
        let (file, mmap) = open_region(path.as_ref())?;
        Self::from_region(file, mmap, value_bytes)
    }

    fn check_size(value_bytes: usize) -> Result<(), SharedOnceError> {
        if value_bytes == 0 || value_bytes > ONCE_PAYLOAD_BYTES {
            return Err(SharedOnceError::PayloadTooLarge);
        }
        Ok(())
    }

    fn from_region(file: File, mmap: MmapMut, value_bytes: usize) -> Result<Self, SharedOnceError> {
        let header = unsafe { &*(mmap.as_ptr() as *const OnceHeader) };
        header_matches(header, value_bytes)?;
        Ok(Self { _file: file, mmap, value_bytes })
    }

    fn cell(&self) -> Cell {
        Cell { base: self.mmap.as_ptr() as *mut u8, value_bytes: self.value_bytes }
    }

    /// Bytes the value holds.
    #[inline]
    pub fn value_len(&self) -> usize {
        self.value_bytes
    }

    /// One of `STATE_EMPTY`, `STATE_INITIALIZING` or `STATE_INITIALIZED`.
    pub fn state(&self) -> u8 {
        self.cell().state()
    }

    /// The published value into `out`, or `false` while none is
    /// published. Non-blocking. The slot an uncontended publish takes is
    /// read at a fixed address behind one compare.
    pub fn try_get(&self, out: &mut [u8]) -> bool {
        let cell = self.cell();
        let first = cell.ids().start as u64 + 1;
        let w = cell.header().winner.load(Ordering::Acquire);
        if w != first {
            return try_get_unless_first(&cell, w, out);
        }
        cell.read(first as usize - 1, out);
        true
    }

    /// Take the right to produce the value, stamping the claim with `pid`'s
    /// process: `true` to the one caller that takes it. A published value,
    /// or a claim a live process holds, refuses it; a claim whose process
    /// is gone is broken and taken. A value published between this call's
    /// look and its claim leaves the claim standing, and the producer's
    /// publish then keeps the value already there.
    pub fn claim(&self, pid: u32) -> bool {
        let cell = self.cell();
        cell.winner().is_none() && cell.claim_as(process_identity::holder_for(pid))
    }

    /// Publish the value produced under the claim `pid`'s process holds,
    /// and give the claim back. `false` unless that claim is the one
    /// standing, so a caller that never took it cannot overwrite the
    /// producer's value. A value published first, by a typed handle's
    /// racing set, stays. A `value` shorter than the cell's size is
    /// published zero-filled.
    pub fn publish(&self, pid: u32, value: &[u8]) -> bool {
        let cell = self.cell();
        let fetcher = &cell.header().fetcher;
        let held = fetcher.load(Ordering::Acquire);
        if held == 0 || process_identity::holder_pid(held) != pid {
            return false;
        }
        cell.publish(value);
        fetcher.compare_exchange(held, 0, Ordering::AcqRel, Ordering::Acquire).is_ok()
    }

    pub fn flush(&self) -> Result<(), SharedOnceError> {
        self.mmap.flush()?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::sync::atomic::AtomicU32;
    use std::thread;

    /// A file for one test, removed when the test ends; declared before the
    /// primitive that maps it so it drops after it.
    fn tmp(name: &str) -> crate::test_paths::TmpFile {
        crate::test_paths::TmpFile::new(format!("subetha-once-{name}-{}.bin", std::process::id()))
    }

    /// The holder word of a process that is gone: no system hands out pid
    /// `u32::MAX - 1`, and a stamp of 0 leaves liveness to the pid alone.
    fn gone_holder() -> u64 {
        process_identity::holder(u32::MAX - 1, 0)
    }

    fn cell_of(c: &SharedOnceCellDyn) -> Cell {
        c.cell()
    }

    impl SharedOnceCellDyn {
        /// Publish `value` as the racing publish does, and the published
        /// value: whether `value` won, and what the cell holds.
        fn race_publish(&self, value: &[u8]) -> (bool, Vec<u8>) {
            let cell = self.cell();
            let (won, i) = cell.publish(value);
            let mut out = vec![0u8; self.value_bytes];
            cell.read(i, &mut out);
            (won, out)
        }
    }

    /// Callers publishing together end on one value: exactly one of them
    /// wins, and every one of them reads the winner's value.
    #[test]
    fn racing_publishers_end_on_one_value() {
        let p = tmp("race");
        let c: Arc<SharedOnceCell<u64>> = Arc::new(SharedOnceCell::create(&p).unwrap());
        let gate = Arc::new(std::sync::Barrier::new(8));
        let hs: Vec<_> = (1..=8u64)
            .map(|v| {
                let (c, gate) = (Arc::clone(&c), Arc::clone(&gate));
                thread::spawn(move || {
                    gate.wait();
                    let won = c.set(v);
                    (won, v, c.get().expect("a value is published after a set"))
                })
            })
            .collect();
        let results: Vec<(bool, u64, u64)> = hs.into_iter().map(|h| h.join().unwrap()).collect();
        assert_eq!(results.iter().filter(|r| r.0).count(), 1, "one publisher won: {results:?}");
        let winner = results.iter().find(|r| r.0).unwrap().1;
        assert!(results.iter().all(|r| r.2 == winner), "every publisher reads the winner: {results:?}");
        assert_eq!(c.get(), Some(winner));
        assert_eq!(c.state_for_tests(), STATE_INITIALIZED);
    }

    /// Slots whose copiers are gone are freed by the next publisher, which
    /// publishes at once rather than waiting on them.
    #[test]
    fn slots_whose_copiers_are_gone_are_freed() {
        let p = tmp("gone-copiers");
        let c = SharedOnceCellDyn::create(&p, 2).unwrap();
        for id in 0..=ONCE_SLOTS {
            cell_of(&c).slot_state(id).store(gone_holder(), Ordering::Release);
        }
        assert_eq!(c.state(), STATE_INITIALIZING);
        let began = std::time::Instant::now();
        assert_eq!(c.race_publish(&[7, 8]), (true, vec![7, 8]), "a freed slot takes the value");
        assert!(began.elapsed() < std::time::Duration::from_secs(1), "it waited {:?}", began.elapsed());
    }

    /// A filled slot nobody published, as a copier that died between its
    /// fill and its swap leaves it, is published by the next caller that
    /// finds every slot taken; that caller reads the filled value.
    #[test]
    fn a_filled_slot_nobody_published_is_published_by_the_next_caller() {
        let p = tmp("filled-unpublished");
        let c = SharedOnceCellDyn::create(&p, 2).unwrap();
        let cell = cell_of(&c);
        let me = process_identity::current();
        for id in 0..=ONCE_SLOTS {
            cell.slot_state(id).store(me, Ordering::Release);
        }
        unsafe { std::ptr::copy_nonoverlapping([3u8, 4].as_ptr(), cell.payload(5), 2) };
        cell.slot_state(5).store(FILLED, Ordering::Release);
        assert_eq!(c.race_publish(&[9, 9]), (false, vec![3, 4]), "the filled slot's value is published, not this one");
    }

    /// While every slot is mid-copy by a live caller, a publisher waits for
    /// one of them to fill, and then reads that value: the one wait the
    /// cell has.
    #[test]
    fn a_publisher_waits_only_while_every_slot_is_mid_copy() {
        let p = tmp("all-mid-copy");
        let c = Arc::new(SharedOnceCellDyn::create(&p, 2).unwrap());
        let me = process_identity::current();
        for id in 0..=ONCE_SLOTS {
            cell_of(&c).slot_state(id).store(me, Ordering::Release);
        }
        let publisher = {
            let c = Arc::clone(&c);
            thread::spawn(move || {
                let (won, out) = c.race_publish(&[1, 1]);
                (won, [out[0], out[1]])
            })
        };
        thread::sleep(std::time::Duration::from_millis(20));
        assert!(!publisher.is_finished(), "nothing was free and nothing filled");
        let cell = cell_of(&c);
        unsafe { std::ptr::copy_nonoverlapping([6u8, 6].as_ptr(), cell.payload(2), 2) };
        cell.slot_state(2).store(FILLED, Ordering::Release);
        assert_eq!(publisher.join().unwrap(), (false, [6, 6]), "the publisher takes the slot that filled");
    }

    /// An uncontended value lands in the first slot its size allows: the
    /// header line's inline slot for at most ONCE_INLINE_BYTES, slot line 0
    /// past that; each is read back by the dynamic and the typed reads.
    #[test]
    fn an_uncontended_value_lands_in_the_first_slot_its_size_allows() {
        let small = tmp("first-inline");
        let c = SharedOnceCellDyn::create(&small, ONCE_INLINE_BYTES).unwrap();
        assert_eq!(c.race_publish(&[5; ONCE_INLINE_BYTES]), (true, vec![5; ONCE_INLINE_BYTES]));
        assert_eq!(cell_of(&c).header().winner.load(Ordering::Acquire), 1, "the inline slot holds it");

        let big = tmp("first-line");
        let d = SharedOnceCellDyn::create(&big, ONCE_INLINE_BYTES + 1).unwrap();
        assert_eq!(d.race_publish(&[6; ONCE_INLINE_BYTES + 1]), (true, vec![6; ONCE_INLINE_BYTES + 1]));
        assert_eq!(cell_of(&d).header().winner.load(Ordering::Acquire), 2, "slot line 0 holds it");
        let mut out = vec![0u8; ONCE_INLINE_BYTES + 1];
        assert!(d.try_get(&mut out));
        assert_eq!(out, vec![6; ONCE_INLINE_BYTES + 1]);

        #[derive(Clone, Copy, PartialEq, Debug)]
        struct Wide([u64; 5]);
        let wide = tmp("first-typed-wide");
        let w: SharedOnceCell<Wide> = SharedOnceCell::create(&wide).unwrap();
        assert!(w.set(Wide([7; 5])));
        assert_eq!(w.get(), Some(Wide([7; 5])));
        assert_eq!(w.get_or_init(|| panic!("the value is published")), Wide([7; 5]));
    }

    /// One caller takes the claim and the rest are refused, reading nothing
    /// until the producer publishes; a publish under a pid that does not
    /// hold the claim is refused; once published, nobody claims again.
    #[test]
    fn one_caller_claims_and_the_rest_read_once_it_publishes() {
        let p = tmp("dyn-claim");
        let a = SharedOnceCellDyn::create(&p, 4).unwrap();
        let b = SharedOnceCellDyn::open(&p, 4).unwrap();
        assert_eq!(a.state(), STATE_EMPTY);

        let mine = std::process::id();
        assert!(a.claim(mine), "the first caller takes the claim");
        assert!(!b.claim(mine), "the second is refused while one stands");
        assert_eq!(b.state(), STATE_INITIALIZING);

        let mut out = [0u8; 4];
        assert!(!b.try_get(&mut out), "nothing is published yet");
        assert!(!b.publish(mine + 1, &[9, 9, 9, 9]), "a pid that does not hold the claim is refused");

        assert!(a.publish(mine, &[1, 2, 3, 4]));
        assert!(b.try_get(&mut out));
        assert_eq!(out, [1, 2, 3, 4], "the other handle reads the producer's value");
        assert_eq!(b.state(), STATE_INITIALIZED);
        assert!(!a.claim(mine), "a published value refuses every claim");
    }

    /// A claim whose process is gone is taken over by the next claim at
    /// once, and that caller's value is published.
    #[test]
    fn a_claim_whose_process_is_gone_is_taken_over() {
        let p = tmp("dyn-gone");
        let c = SharedOnceCellDyn::create(&p, 2).unwrap();
        cell_of(&c).header().fetcher.store(gone_holder(), Ordering::Release);
        let mine = std::process::id();
        let began = std::time::Instant::now();
        assert!(c.claim(mine), "the gone claimant's claim is broken and taken");
        assert!(began.elapsed() < std::time::Duration::from_secs(1), "it waited {:?}", began.elapsed());
        assert!(c.publish(mine, &[4, 5]));
        let mut out = [0u8; 2];
        assert!(c.try_get(&mut out));
        assert_eq!(out, [4, 5]);
    }

    /// The value size is part of the layout, so a size that disagrees is
    /// refused, and one past the cell's room is too.
    #[test]
    fn a_dyn_cell_at_the_wrong_size_is_refused() {
        let p = tmp("dyn-size");
        let _c = SharedOnceCellDyn::create(&p, 8).unwrap();
        assert!(matches!(
            SharedOnceCellDyn::open(&p, 4).unwrap_err(),
            SharedOnceError::LayoutMismatch(crate::layout::Mismatch { field: "value size", found: 8, wanted: 4 })
        ));
        assert!(matches!(
            SharedOnceCellDyn::create(tmp("dyn-big"), ONCE_PAYLOAD_BYTES + 1).unwrap_err(),
            SharedOnceError::PayloadTooLarge,
        ));
        assert!(matches!(
            SharedOnceCellDyn::create(tmp("dyn-zero"), 0).unwrap_err(),
            SharedOnceError::PayloadTooLarge,
        ));
    }

    /// A cell an earlier SubEtha laid out has another magic and is refused
    /// as a mismatch of it.
    #[test]
    fn an_earlier_cell_format_is_refused_by_its_magic() {
        let p = tmp("earlier-format");
        let mut bytes = vec![0u8; ONCE_FILE_SIZE];
        bytes[..4].copy_from_slice(&0x4F4E_4332u32.to_ne_bytes());
        bytes[4..8].copy_from_slice(&8u32.to_ne_bytes());
        std::fs::write(&p, &bytes).unwrap();
        assert!(matches!(
            SharedOnceCell::<u64>::create(&p),
            Err(SharedOnceError::LayoutMismatch(crate::layout::Mismatch { field: "magic", found: 0x4F4E_4332, .. })),
        ));
    }

    #[test]
    fn fresh_cell_is_empty() {
        let p = tmp("fresh");
        let c: SharedOnceCell<u64> = SharedOnceCell::create(&p).unwrap();
        assert!(!c.is_initialized());
        assert_eq!(c.get(), None);
    }

    #[test]
    fn first_set_wins_subsequent_sets_lose() {
        let p = tmp("first-wins");
        let c: SharedOnceCell<u64> = SharedOnceCell::create(&p).unwrap();
        assert!(c.set(42));
        assert!(!c.set(99));
        assert_eq!(c.get(), Some(42));
    }

    /// A second create attaches with the published value in place -
    /// truncation here would break the once guarantee; reset is what
    /// strips it.
    #[test]
    fn second_create_attaches_and_keeps_the_value() {
        let p = tmp("attach");
        let c: SharedOnceCell<u64> = SharedOnceCell::create(&p).unwrap();
        assert!(c.set(42));

        let c2: SharedOnceCell<u64> = SharedOnceCell::create(&p).unwrap();
        assert_eq!(c2.get(), Some(42), "attach lost the published value");
        assert!(!c2.set(99), "attach reopened a spent cell");

        // Windows refuses to truncate a mapped file, so every handle goes
        // before the reset.
        drop(c);
        drop(c2);
        let fresh: SharedOnceCell<u64> = SharedOnceCell::reset(&p).unwrap();
        assert_eq!(fresh.get(), None, "reset left a value behind");
        assert!(fresh.set(7));
        drop(fresh);
    }

    /// Attaching with a different value type is refused.
    #[test]
    fn create_refuses_a_mismatched_region() {
        let p = tmp("mismatch");
        let c: SharedOnceCell<u64> = SharedOnceCell::create(&p).unwrap();
        assert!(matches!(
            SharedOnceCell::<u32>::create(&p),
            Err(SharedOnceError::LayoutMismatch(crate::layout::Mismatch {
                field: "value size",
                found: 8,
                wanted: 4,
            })),
        ));
        drop(c);
    }

    #[test]
    fn cross_handle_init_visible_to_other_handle() {
        let p = tmp("cross-handle");
        let writer: SharedOnceCell<u64> = SharedOnceCell::create(&p).unwrap();
        let reader: SharedOnceCell<u64> = SharedOnceCell::open(&p).unwrap();
        assert!(!reader.is_initialized());
        assert!(writer.set(7777));
        assert!(reader.is_initialized());
        assert_eq!(reader.get(), Some(7777));
    }

    /// Callers that find the cell empty each run their initializer, none
    /// waiting on another, and all of them get the one value published.
    #[test]
    fn get_or_init_hands_every_caller_the_one_value_published() {
        let p = tmp("get-or-init");
        let c: Arc<SharedOnceCell<u64>> = Arc::new(SharedOnceCell::create(&p).unwrap());
        let runs = Arc::new(AtomicU32::new(0));
        let gate = Arc::new(std::sync::Barrier::new(8));
        let hs: Vec<_> = (0..8u64)
            .map(|v| {
                let (c, runs, gate) = (Arc::clone(&c), Arc::clone(&runs), Arc::clone(&gate));
                thread::spawn(move || {
                    gate.wait();
                    c.get_or_init(|| {
                        runs.fetch_add(1, Ordering::AcqRel);
                        1000 + v
                    })
                })
            })
            .collect();
        let results: Vec<u64> = hs.into_iter().map(|h| h.join().unwrap()).collect();
        assert!(results.iter().all(|&v| v == results[0]), "callers got different values: {results:?}");
        assert_eq!(c.get(), Some(results[0]));
        let ran = runs.load(Ordering::Acquire);
        assert!((1..=8).contains(&ran), "{ran} initializers for 8 callers");
    }

    /// An initializer that panics leaves the cell as it found it: nothing
    /// is claimed while it runs, so the next caller publishes its own.
    #[test]
    fn a_panicking_initializer_leaves_the_cell_empty() {
        let p = tmp("typed-panic");
        let c: SharedOnceCell<u64> = SharedOnceCell::create(&p).unwrap();
        let failed = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            c.get_or_init(|| -> u64 { panic!("the initializer failed") })
        }));
        assert!(failed.is_err(), "the panic reached the caller");
        assert!(!c.is_initialized());
        assert_eq!(c.get_or_init(|| 7), 7, "the next caller published its value");
    }

    #[test]
    fn disk_persistence_survives_reopen() {
        let p = tmp("disk-persist");
        {
            let c: SharedOnceCell<u64> = SharedOnceCell::create(&p).unwrap();
            assert!(c.set(8888));
            c.flush().unwrap();
        }
        let c2: SharedOnceCell<u64> = SharedOnceCell::open(&p).unwrap();
        assert_eq!(c2.get(), Some(8888));
        assert!(c2.is_initialized());
        assert!(!c2.set(9999), "a published cell keeps its value");
        assert_eq!(c2.get(), Some(8888));
    }

    #[test]
    fn payload_too_large_at_create() {
        #[allow(dead_code)] // size_of<Big> is the test signal, not the field
        struct Big([u8; ONCE_PAYLOAD_BYTES + 1]);
        impl Copy for Big {}
        impl Clone for Big { fn clone(&self) -> Self { *self } }
        let p = tmp("too-large");
        match SharedOnceCell::<Big>::create(&p) {
            Err(SharedOnceError::PayloadTooLarge) => {}
            other => panic!("expected PayloadTooLarge, got {:?}", other.as_ref().err()),
        }
    }

    #[test]
    fn open_rejects_wrong_payload_size() {
        let p = tmp("wrong-size");
        let _c: SharedOnceCell<u64> = SharedOnceCell::create(&p).unwrap();
        match SharedOnceCell::<u32>::open(&p) {
            Err(SharedOnceError::LayoutMismatch(crate::layout::Mismatch {
                field: "value size",
                found: 8,
                wanted: 4,
            })) => {}
            other => panic!("expected a value size of 8 where 4 was asked for, got {:?}", other.as_ref().err()),
        }
    }

    /// One caller fetches at a time: while a live caller holds the fetch
    /// claim, the call returns Pending to another at once and its fetch
    /// does not run; once the value is published every caller gets it.
    #[test]
    fn a_fetch_in_flight_returns_pending_to_other_callers() {
        let p = tmp("fetch-pending");
        let c: Arc<SharedOnceCell<u64>> = Arc::new(SharedOnceCell::create(&p).unwrap());
        let (started_tx, started_rx) = std::sync::mpsc::channel();
        let (release_tx, release_rx) = std::sync::mpsc::channel::<()>();
        let fetcher = {
            let c = Arc::clone(&c);
            thread::spawn(move || {
                c.get_or_fetch(|| {
                    started_tx.send(()).unwrap();
                    release_rx.recv().unwrap();
                    55
                })
            })
        };
        started_rx.recv().unwrap();
        let began = std::time::Instant::now();
        assert_eq!(c.get_or_fetch(|| panic!("a second fetch ran")), Fetch::Pending);
        assert!(began.elapsed() < std::time::Duration::from_secs(1), "Pending took {:?}", began.elapsed());
        assert_eq!(c.state_for_tests(), STATE_INITIALIZING);
        release_tx.send(()).unwrap();
        assert_eq!(fetcher.join().unwrap(), Fetch::Ready(55));
        assert_eq!(c.get_or_fetch(|| panic!("a fetch after publication ran")), Fetch::Ready(55));
    }

    /// A fetch claim whose holder is gone is broken and taken by the next
    /// caller, whose fetch then runs.
    #[test]
    fn a_fetch_claim_whose_holder_is_gone_is_taken_over() {
        let p = tmp("fetch-gone");
        let c: SharedOnceCell<u64> = SharedOnceCell::create(&p).unwrap();
        c.cell().header().fetcher.store(gone_holder(), Ordering::Release);
        assert_eq!(c.get_or_fetch(|| 31), Fetch::Ready(31));
        assert_eq!(c.cell().header().fetcher.load(Ordering::Acquire), 0, "the claim went back after the fetch");
    }

    /// A fetch that panics gives the claim back, so the next caller fetches.
    #[test]
    fn a_panicking_fetch_gives_its_claim_back() {
        let p = tmp("fetch-panic");
        let c: SharedOnceCell<u64> = SharedOnceCell::create(&p).unwrap();
        let failed = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            c.get_or_fetch(|| -> u64 { panic!("the fetch failed") })
        }));
        assert!(failed.is_err(), "the panic reached the caller");
        assert_eq!(c.cell().header().fetcher.load(Ordering::Acquire), 0, "the claim went back");
        assert_eq!(c.get_or_fetch(|| 12), Fetch::Ready(12));
    }

    /// Words in a file every racing process maps: the count of racers that
    /// have started, then each racer's outcome (0 unwritten, 1 lost, 2 won)
    /// and the value it read.
    struct Results(MmapMut);

    impl Results {
        fn len(racers: usize) -> u64 {
            ((1 + 2 * racers) * size_of::<u64>()) as u64
        }

        fn create(path: &Path, racers: usize) -> Self {
            let file = std::fs::OpenOptions::new()
                .read(true)
                .write(true)
                .create(true)
                .truncate(true)
                .open(path)
                .expect("the results file");
            file.set_len(Self::len(racers)).expect("the results file sized");
            Self(unsafe { MmapMut::map_mut(&file) }.expect("the results mapped"))
        }

        fn open(path: &Path, racers: usize) -> Self {
            let file = std::fs::OpenOptions::new().read(true).write(true).open(path).expect("the results file");
            assert_eq!(file.metadata().expect("the results' size").len(), Self::len(racers));
            Self(unsafe { MmapMut::map_mut(&file) }.expect("the results mapped"))
        }

        fn word(&self, i: usize) -> &AtomicU64 {
            unsafe { &*(self.0.as_ptr().add(i * size_of::<u64>()) as *const AtomicU64) }
        }
    }

    /// One publisher process: waits until every racer has started, creates
    /// the cell at the shared path, sets its own value, and records whether
    /// it won and what the cell then holds. Runs only when the cross-process
    /// test starts it.
    #[test]
    fn once_publisher_role() {
        let Some(spec) = std::env::var_os("SUBETHA_ONCE_PUBLISHER") else {
            return;
        };
        let spec = spec.to_string_lossy().to_string();
        let parts: Vec<&str> = spec.split('|').collect();
        let [cell, results, racer, racers] = parts[..] else {
            eprintln!("publisher: a spec of {} parts: {spec}", parts.len());
            std::process::exit(2);
        };
        let racer: usize = racer.parse().expect("the racer index parses");
        let racers: usize = racers.parse().expect("the racer count parses");
        let results = Results::open(Path::new(results), racers);
        results.word(0).fetch_add(1, Ordering::AcqRel);
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
        while results.word(0).load(Ordering::Acquire) < racers as u64 {
            if std::time::Instant::now() > deadline {
                eprintln!("publisher {racer}: the other racers never started");
                std::process::exit(3);
            }
            thread::yield_now();
        }
        let c: SharedOnceCell<u64> = SharedOnceCell::create(cell).expect("the publisher creates the cell");
        let won = c.set(1000 + racer as u64);
        let seen = c.get().expect("a value is published after a set");
        results.word(2 + 2 * racer).store(seen, Ordering::Release);
        results.word(1 + 2 * racer).store(if won { 2 } else { 1 }, Ordering::Release);
        std::process::exit(0);
    }

    /// A racing process, killed and reaped if the test ends before it is.
    struct Racer(Option<std::process::Child>);

    impl Racer {
        fn reap(mut self) -> std::process::ExitStatus {
            let mut child = self.0.take().expect("the racer is reaped once");
            child.wait().expect("the racer is waited on")
        }
    }

    impl Drop for Racer {
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

    /// Publisher processes started together create one new cell and set it:
    /// one set wins, and every process reads the winner's value.
    #[test]
    fn publisher_processes_racing_on_a_new_path_end_on_one_value() {
        const RACERS: usize = 4;
        let cell = tmp("processes");
        let results_file = tmp("processes-results");
        let results = Results::create(&results_file, RACERS);
        let exe = std::env::current_exe().expect("the test binary's own path");
        let racers: Vec<Racer> = (0..RACERS)
            .map(|r| {
                Racer(Some(
                    std::process::Command::new(&exe)
                        .arg("shared_once_cell::tests::once_publisher_role")
                        .arg("--exact")
                        .arg("--nocapture")
                        .env(
                            "SUBETHA_ONCE_PUBLISHER",
                            format!("{}|{}|{r}|{RACERS}", cell.display(), results_file.display()),
                        )
                        .spawn()
                        .expect("a racer process spawns"),
                ))
            })
            .collect();
        for racer in racers {
            let status = racer.reap();
            assert!(status.success(), "a racer failed: {status}");
        }
        let outcomes: Vec<(u64, u64)> = (0..RACERS)
            .map(|r| (results.word(1 + 2 * r).load(Ordering::Acquire), results.word(2 + 2 * r).load(Ordering::Acquire)))
            .collect();
        let winners: Vec<usize> = (0..RACERS).filter(|&r| outcomes[r].0 == 2).collect();
        assert_eq!(winners.len(), 1, "one racer won: {outcomes:?}");
        let value = 1000 + winners[0] as u64;
        assert!(
            outcomes.iter().all(|&(outcome, seen)| outcome != 0 && seen == value),
            "every racer read the winner's value: {outcomes:?}"
        );
        let c: SharedOnceCell<u64> = SharedOnceCell::open(&cell).expect("the cell the racers published");
        assert_eq!(c.get(), Some(value));
    }

    /// A child process running this test binary's sleeper, killed and
    /// reaped when dropped.
    struct Sleeper(std::process::Child);

    impl Sleeper {
        fn start() -> Self {
            let child = std::process::Command::new(std::env::current_exe().unwrap())
                .args(["--exact", "exit_watch::tests::sleeper_never_runs_on_its_own", "--ignored", "--nocapture"])
                .env("SUBETHA_EXIT_WATCH_SLEEP", "1")
                .spawn()
                .unwrap();
            Self(child)
        }

        fn pid(&self) -> u32 {
            self.0.id()
        }

        /// Kill and reap the child.
        fn end(mut self) {
            self.0.kill().unwrap();
            self.0.wait().unwrap();
        }
    }

    impl Drop for Sleeper {
        fn drop(&mut self) {
            if self.0.try_wait().unwrap().is_none() {
                self.0.kill().unwrap();
                self.0.wait().unwrap();
            }
        }
    }

    /// A claim taken in the name of a live process stands against every
    /// other caller; once that process has exited, the next claim takes it
    /// over and publishes.
    #[test]
    fn a_claim_held_for_a_process_that_exited_is_taken_over() {
        let p = tmp("dyn-exited");
        let c = SharedOnceCellDyn::create(&p, 2).unwrap();
        let child = Sleeper::start();
        assert!(c.claim(child.pid()), "the claim is taken in the live child's name");
        let mine = std::process::id();
        assert!(!c.claim(mine), "a live process's claim stands");
        child.end();
        assert!(c.claim(mine), "the exited process's claim is taken over");
        assert!(c.publish(mine, &[6, 7]));
        let mut out = [0u8; 2];
        assert!(c.try_get(&mut out));
        assert_eq!(out, [6, 7]);
    }

    impl<T: Copy + 'static> SharedOnceCell<T> {
        fn state_for_tests(&self) -> u8 {
            self.cell().state()
        }
    }
}
