//! `SharedVec<T>` - cross-process bounded indexable sequence.
//!
//! Distinct from [`SharedRing`](crate::SharedRing) (FIFO; drain
//! semantics): SharedVec is random-access, accumulates monotonically
//! up to capacity, and supports `get(i)` for any prior index.
//!
//! # If your elements are written once
//!
//! A slot is one cache line for an element of at most 8 bytes and two for a
//! larger one, however small: a 12-byte row of a lookup table costs 128
//! bytes of file, and a table of 290,000 such rows occupies 37 MB to hold
//! 3.5 MB of data. Those lines buy the slot's state beside its element, or a
//! larger element's two buffers and head, which are what let a reader take
//! an element while a writer replaces it, neither waiting on the other.
//!
//! A table baked at build time and never written again pays that and
//! uses none of it. [`SharedArray`](crate::shared_array::SharedArray) is
//! the flat alternative: stride exactly the element size, no per-element
//! word, read-only for every process once sealed. It cannot be written
//! while it is read, which is the whole of what it gives up.
//!
//! # Layout
//!
//! Single MMF file:
//!
//! ```text
//! +----------------------------+
//! | VecHeader  (64B aligned)   |  magic, capacity, top word, slot geometry,
//! |                            |  writer table size
//! +----------------------------+
//! | Slot[0]                    |  64B word slot: element word + state word
//! | Slot[1]                    |  or 128B pair: head + claim + 2 x payload
//! | ...                        |
//! | Slot[capacity - 1]         |
//! +----------------------------+
//! | Writer table               |  16B per entry: holder word + registrations
//! +----------------------------+
//! ```
//!
//! The writer table holds one entry per handle writing the vec at once, as
//! many as its creator asked for (see [`SharedVec::create`]): the holder
//! word of the handle's process beside the count of threads writing
//! through it, so a writer can give back the count of a process that died.
//!
//! An element of at most 8 bytes lives in a word slot, a cache line holding
//! the element's word beside a state word; a larger one, up to
//! [`VEC_PAYLOAD_BYTES`], in a pair of value buffers and a head naming the
//! current one (see [`VecSlot`]). No two slots share a cache line.
//!
//! # Concurrency
//!
//! The raw vec shares this protocol. Every thread that writes through a
//! handle is counted in the header, and an element of at most 8 bytes is
//! written one of two ways by that count:
//!
//! - With one writer, `push_back` claims slot `len` by one compare-and-swap
//!   that writes the element, then raises `len` past it, and `pop_back`
//!   lowers `len` by one compare-and-swap, leaving the element in place.
//! - With two or more, `push_back` takes slot `len` by one atomic add to
//!   `len` and then claims it, or near the capacity claims it first and
//!   raises `len` after, and `pop_back` takes slot `len - 1` by one
//!   compare-and-swap, then lowers `len` past it. A lowering that loses to
//!   a push leaves a hole below `len`, which the next pop steps past.
//! - A second writer's first write switches the vec to the second way, and
//!   the writer left switches it back. Nothing waits on another participant
//!   either way, and `Full` at the capacity is one load of the top word.
//!
//! A larger element is always pushed by claiming slot `len` and raising
//! `len`, and popped the second way.
//!
//! - `get(i)`: below `len`, read the slot, which answers None for a hole. A
//!   read never waits on a writer.
//! - `set(i, v)`: one compare-and-swap on a slot holding an element, so it
//!   orders with a pop of that element; a hole is `OutOfBounds`.
//! - `clear`: one compare-and-swap of the top word to length zero. Slot
//!   payloads stay on disk, unreachable through the bounded index.
//!
//! # Capacity
//!
//! Fixed at create time. The MMF is pre-allocated to the full size;
//! no resize-on-grow protocol. The unbounded variant (with
//! coordinator-mediated MMF resize) is a separate primitive.

use std::fs::File;
use std::marker::PhantomData;
use std::mem::{size_of, MaybeUninit};
use std::path::Path;
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};

use memmap2::{Mmap, MmapMut, MmapOptions};

use crate::slot_pair::{self, SlotGeometry};
use crate::vec_core::{Shape, VecSlots, WriterEntry, Writers};

pub const VEC_MAGIC: u32 = 0x4150_5633;
pub const VEC_PAYLOAD_BYTES: usize = 56;

/// How this process mapped the file. `MmapMut` demands a read+write
/// file handle, which a consumer holding read access alone cannot get.
pub(crate) enum Mapping {
    Writable(MmapMut),
    ReadOnly(Mmap),
}

impl Mapping {
    #[inline]
    pub(crate) fn as_ptr(&self) -> *const u8 {
        match self {
            Mapping::Writable(m) => m.as_ptr(),
            Mapping::ReadOnly(m) => m.as_ptr(),
        }
    }

    #[inline]
    pub(crate) fn is_writable(&self) -> bool {
        matches!(self, Mapping::Writable(_))
    }

    /// A no-op on a read-only mapping, so a caller flushing on a timer
    /// need not know which kind it holds.
    pub(crate) fn flush(&self) -> Result<(), std::io::Error> {
        match self {
            Mapping::Writable(m) => m.flush(),
            Mapping::ReadOnly(_) => Ok(()),
        }
    }

    fn flush_async(&self) -> Result<(), std::io::Error> {
        match self {
            Mapping::Writable(m) => m.flush_async(),
            Mapping::ReadOnly(_) => Ok(()),
        }
    }
}

#[repr(C, align(64))]
pub struct VecHeader {
    pub magic: u32,
    pub slot_payload_size: u32,
    pub capacity: u64,
    /// The top word's low half: the length in the low 32 bits, every slot
    /// below it holding an element or a hole, and above it a version that
    /// every lowering of the length and every clear steps. With `epoch`, one
    /// 16-byte word swapped whole (see [`vec_core`](crate::vec_core)).
    pub(crate) top: AtomicU64,
    /// The top word's high half: the epoch every clear steps.
    pub(crate) epoch: AtomicU64,
    /// Full stride of one slot, its state words included.
    pub slot_stride: u32,
    /// Where the element lies within its slot.
    pub payload_offset: u32,
    /// Bytes one element takes.
    pub element_size: u32,
    /// The element's alignment, as the creator stated it.
    pub alignment: u32,
    /// A tag the creator chose for the element type; an open that states
    /// another is refused.
    pub layout_tag: u64,
    /// Entries in the writer table after the slots, the creator's choice:
    /// one per writing handle, its process's pid and start stamp beside its
    /// live registrations, so a dead process's are given back. An open that
    /// states another number is refused.
    pub writer_slots: u32,
    _pad: [u8; 4],
}

impl VecHeader {
    /// The length the top word holds, at most the capacity, which adds that
    /// found the vec full can pass: one 4-byte read and one comparison.
    #[inline(always)]
    pub(crate) fn len(&self) -> u64 {
        let at = if cfg!(target_endian = "little") { 0 } else { 4 };
        // SAFETY: the length is the top word's low 32 bits: an aligned 4-byte
        // word inside it, which every processor with a 16-byte swap reads
        // whole.
        let length = unsafe { &*((&self.top as *const AtomicU64 as *const u8).add(at) as *const AtomicU32) };
        u64::from(length.load(Ordering::Acquire)).min(self.capacity)
    }

    /// The 16-byte top word, `top` and `epoch` together.
    #[inline(always)]
    pub(crate) fn top_word(&self) -> &crate::atomic_pair::AtomicPair {
        // SAFETY: `top` lies at offset 16 of a 64-byte-aligned header with
        // `epoch` after it, and the two are accessed only as this pair and by
        // aligned 8-byte reads of `top`, which a processor with a 16-byte
        // swap reads whole.
        unsafe { crate::atomic_pair::AtomicPair::at(&self.top as *const AtomicU64 as *mut u8) }
    }
}

const _: () = assert!(std::mem::offset_of!(VecHeader, top) == 16 && std::mem::offset_of!(VecHeader, epoch) == 24);
const _: () = assert!(std::mem::offset_of!(VecHeader, writer_slots) == 56);
const _: () = assert!(size_of::<WriterEntry>() == 16);

/// Where a vec of `T` with `capacity` slots keeps its writer table: right
/// past the slots, which start right after the header.
pub(crate) const fn vec_table_offset<T>(capacity: usize) -> usize {
    size_of::<VecHeader>() + capacity * vec_stride(size_of::<T>(), std::mem::align_of::<T>())
}

/// One slot: the head names the current buffer in bit 0 and counts the
/// writes that landed above it, and the claim is 0 while no write holds the
/// slot, else the writer's pid and process start stamp. A write fills the
/// buffer the head does not name and makes it current with one
/// compare-and-swap of the head.
#[repr(C, align(64))]
pub struct VecSlot {
    pub head: AtomicU64,
    pub claim: AtomicU64,
    pub buffers: [[u8; VEC_PAYLOAD_BYTES]; 2],
}

const _: () = {
    assert!(size_of::<VecHeader>() == 64);
    assert!(size_of::<VecSlot>() == 128);
    assert!(std::mem::offset_of!(VecSlot, buffers) == slot_pair::SLOT_PREFIX);
};

/// Where a vec slot's parts lie for an element of `size` bytes, at most
/// [`VEC_PAYLOAD_BYTES`]: the buffers of [`VecSlot`] whatever the size, so
/// every element type of a fitting size shares one slot shape.
pub(crate) const fn vec_geometry(size: usize) -> SlotGeometry {
    SlotGeometry { size, ..SlotGeometry::new(VEC_PAYLOAD_BYTES, 8) }
}

/// How a vec's slot holds an element of `size` bytes aligned to
/// `alignment`, at most [`VEC_PAYLOAD_BYTES`]: a word slot up to 8 bytes at
/// an alignment of at most 8, else the pair of buffers of [`VecSlot`].
pub(crate) fn vec_shape(size: usize, alignment: usize) -> Shape {
    Shape::for_element(size, alignment, vec_geometry(size))
}

/// Bytes one slot takes for an element of `size` bytes aligned to
/// `alignment`, as [`vec_shape`] lays it out.
const fn vec_stride(size: usize, alignment: usize) -> usize {
    if size <= 8 && alignment <= 8 { crate::vec_core::WORD_STRIDE } else { size_of::<VecSlot>() }
}

/// Bytes the file of a `SharedVec<T>` of `capacity` slots and a writer table
/// of `writers` entries takes.
pub const fn vec_file_size<T>(capacity: usize, writers: usize) -> usize {
    vec_table_offset::<T>(capacity) + writers * size_of::<WriterEntry>()
}

#[derive(Debug)]
pub enum VecError {
    Full,
    OutOfBounds,
    LayoutMismatch(crate::layout::Mismatch),
    /// The call passed what the vector cannot take; the text says which
    /// argument and what it must be.
    InvalidArgument(&'static str),
    PayloadTooLarge,
    /// The vec was opened read-only and something tried to write it.
    ReadOnly,
    /// The operating system refused to open, size or map the file; the
    /// error carries its code and message.
    IoError(std::io::Error),
}

impl std::fmt::Display for VecError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Full => write!(f, "the vector is at capacity"),
            Self::OutOfBounds => write!(f, "the index names no element: it is at or past the vector's length, or a hole"),
            Self::LayoutMismatch(m) => write!(f, "a vector of another shape: {m}"),
            Self::InvalidArgument(what) => write!(f, "{what}"),
            Self::PayloadTooLarge => write!(f, "the value type is larger than a slot"),
            Self::ReadOnly => write!(f, "the vector was opened read-only"),
            Self::IoError(e) => write!(f, "io: {e}"),
        }
    }
}

impl std::error::Error for VecError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::IoError(e) => Some(e),
            Self::Full
            | Self::OutOfBounds
            | Self::LayoutMismatch(_)
            | Self::InvalidArgument(_)
            | Self::PayloadTooLarge
            | Self::ReadOnly => None,
        }
    }
}

impl From<std::io::Error> for VecError {
    fn from(e: std::io::Error) -> Self { Self::IoError(e) }
}

pub struct SharedVec<T: Copy + 'static> {
    _file: File,
    mmap: Mapping,
    capacity: usize,
    _phantom: PhantomData<T>,
    header_sidecar: subetha_core::HandshakeHeader,
    ring_sidecar: Box<subetha_core::ObservationRing>,
    /// The threads that wrote through this handle, registered with the vec.
    writers: Writers,
}

impl<T: Copy + 'static> Drop for SharedVec<T> {
    fn drop(&mut self) {
        if self.mmap.is_writable() {
            self.writers.leave(self.header().top_word());
        }
    }
}

unsafe impl<T: Copy + Send + 'static> Send for SharedVec<T> {}
unsafe impl<T: Copy + Sync + 'static> Sync for SharedVec<T> {}

impl<T: Copy + Send + Sync + 'static> subetha_sidecar::AdaptiveInstance for SharedVec<T> {
    fn header(&self) -> &subetha_core::HandshakeHeader { &self.header_sidecar }
    fn ring(&self) -> &subetha_core::ObservationRing { &self.ring_sidecar }
    fn make_policy(&self) -> Box<dyn subetha_sidecar::Policy> {
        Box::new(subetha_sidecar::NoMigrationPolicy)
    }
}

impl<T: Copy + 'static> SharedVec<T> {
    /// Obtain the vec at `path`, initializing an empty one if the path
    /// does not yet exist and attaching to it if it does. Attaching
    /// leaves live elements and `len` in place; a region built with a
    /// different capacity or writer table is a `LayoutMismatch`.
    /// [`reset`](Self::reset) reinitializes.
    ///
    /// `writers` is how many entries the vec's writer table holds: one per
    /// handle that writes the vec at the same time, each counting the
    /// threads writing through that handle, so the next writer can give
    /// back the count of a process that died. A handle past the table's
    /// size writes all the same; if its process dies without dropping it,
    /// the vec keeps working as though that process still wrote, which is
    /// correct but slower, until it is reset.
    pub fn create(path: impl AsRef<Path>, capacity: usize, writers: usize) -> Result<Self, VecError> {
        Self::admit(capacity, writers)?;
        let path = path.as_ref();
        let (file, mmap) = crate::mmf_attach::create_or_attach(
            path,
            vec_file_size::<T>(capacity, writers),
            |ptr| unsafe { Self::init_region(ptr, capacity, writers) },
            |ptr| unsafe { u64::from((*(ptr as *const VecHeader)).magic) },
            u64::from(VEC_MAGIC),
        )
        .map_err(|e| match crate::mmf_attach::attach_error(e, VecError::LayoutMismatch) {
            VecError::LayoutMismatch(short) if short.field == "length" => Self::short_file(path, short, capacity, writers),
            other => other,
        })?;
        let this = Self::assemble(file, Mapping::Writable(mmap), capacity, writers);
        this.validate(capacity, writers)?;
        this.slots().scan_writers();
        Ok(this)
    }

    /// Truncate the vec at `path` and initialize an empty one,
    /// discarding every element live peers share. For a caller that
    /// knows it owns the path. `writers` is as for [`create`](Self::create).
    pub fn reset(path: impl AsRef<Path>, capacity: usize, writers: usize) -> Result<Self, VecError> {
        Self::admit(capacity, writers)?;
        let (file, mmap) = crate::mmf_attach::reset(path.as_ref(), vec_file_size::<T>(capacity, writers), |ptr| unsafe {
            Self::init_region(ptr, capacity, writers)
        })?;
        Ok(Self::assemble(file, Mapping::Writable(mmap), capacity, writers))
    }

    /// The handle over a mapping of a vec of `capacity` slots and a writer
    /// table of `writers` entries, its header not yet checked.
    fn assemble(file: File, mmap: Mapping, capacity: usize, writers: usize) -> Self {
        let table = mmap.as_ptr().wrapping_add(vec_table_offset::<T>(capacity));
        Self {
            _file: file,
            mmap,
            capacity,
            _phantom: PhantomData,
            header_sidecar: subetha_core::HandshakeHeader::new(),
            ring_sidecar: Box::new(subetha_core::ObservationRing::new()),
            // SAFETY: the table follows the slots in the mapping the handle
            // holds, 16-byte aligned since a slot is 64 or 128 bytes, `writers`
            // entries once the header is checked, and the writers reach it
            // only on a write, which the check precedes; `admit` checked the
            // processor's 16-byte swap.
            writers: unsafe { Writers::new(table, writers) },
        }
    }

    /// What an attach to the vec file at `path` whose length fell short of
    /// the one asked for is refused with: the header field that differs from
    /// what was asked, since a capacity or a writer table of another size
    /// changes the length; else the length mismatch `short` itself.
    fn short_file(path: &Path, short: crate::layout::Mismatch, capacity: usize, writers: usize) -> VecError {
        match crate::region_file::open_read_only(path) {
            Ok(file) => match Self::check_header_of(&file, capacity, writers) {
                Ok(()) => VecError::LayoutMismatch(short),
                Err(named) => named,
            },
            Err(e) => VecError::IoError(e),
        }
    }

    /// Check the header at the start of `file` as
    /// [`check_header`](Self::check_header) does, when the file holds one.
    fn check_header_of(file: &File, capacity: usize, writers: usize) -> Result<(), VecError> {
        if file.metadata()?.len() < size_of::<VecHeader>() as u64 {
            return Ok(());
        }
        // SAFETY: a read-only mapping of the header alone, which the file
        // holds; its fields other than the top word are written once, before
        // the magic.
        let head = unsafe { MmapOptions::new().len(size_of::<VecHeader>()).map(file)? };
        // SAFETY: the mapping holds a whole header, 64-byte aligned.
        Self::check_header(unsafe { &*(head.as_ptr() as *const VecHeader) }, capacity, writers)
    }

    /// How a slot holds a `T`.
    #[inline(always)]
    fn shape() -> Shape {
        vec_shape(size_of::<T>(), std::mem::align_of::<T>())
    }

    /// Whether a vec of `T` with `capacity` slots and a writer table of
    /// `writers` entries can be made here: a `T` that fits a slot, a capacity
    /// the top word's length can count, a table the header's 32-bit field can
    /// size, and a processor that swaps 16 bytes, as the top word, a word
    /// slot and a table entry take.
    fn admit(capacity: usize, writers: usize) -> Result<(), VecError> {
        if size_of::<T>() > VEC_PAYLOAD_BYTES {
            return Err(VecError::PayloadTooLarge);
        }
        assert!(capacity >= 1);
        if capacity > crate::vec_core::MAX_CAPACITY {
            return Err(VecError::InvalidArgument("a vector holds at most 4294967295 elements"));
        }
        if writers > u32::MAX as usize {
            return Err(VecError::InvalidArgument("a vector's writer table holds at most 4294967295 entries"));
        }
        if !crate::atomic_pair::supported() {
            return Err(VecError::IoError(std::io::Error::new(
                std::io::ErrorKind::Unsupported,
                "this processor has no lock-free 16-byte compare-and-swap",
            )));
        }
        Ok(())
    }

    /// Lay out an empty vec: sizes first, magic last, because attachers
    /// spin on it. The zeroed region is already the empty writer table, the
    /// empty slot array (every slot holding no element) and a top word of
    /// length 0.
    ///
    /// # Safety
    /// `ptr` addresses at least `vec_file_size::<T>(capacity, writers)`
    /// writable zeroed bytes.
    unsafe fn init_region(ptr: *mut u8, capacity: usize, writers: usize) {
        let hdr = ptr as *mut VecHeader;
        let shape = Self::shape();
        unsafe {
            (*hdr).slot_payload_size = shape.payload_size() as u32;
            (*hdr).capacity = capacity as u64;
            (*hdr).slot_stride = shape.stride() as u32;
            (*hdr).payload_offset = shape.payload_offset() as u32;
            (*hdr).element_size = size_of::<T>() as u32;
            (*hdr).alignment = std::mem::align_of::<T>() as u32;
            (*hdr).layout_tag = 0;
            (*hdr).writer_slots = writers as u32;
            std::ptr::write_volatile(&raw mut (*hdr).magic, VEC_MAGIC);
        }
    }

    /// Attach to the vec at `path`, which must exist, built with
    /// `expected_capacity` slots and a writer table of `writers` entries.
    pub fn open(path: impl AsRef<Path>, expected_capacity: usize, writers: usize) -> Result<Self, VecError> {
        Self::admit(expected_capacity, writers)?;
        let file = crate::region_file::open_existing(path.as_ref())?;
        let total = Self::long_enough(&file, expected_capacity, writers)?;
        let mmap = unsafe { MmapOptions::new().len(total).map_mut(&file)? };
        let this = Self::assemble(file, Mapping::Writable(mmap), expected_capacity, writers);
        this.validate(expected_capacity, writers)?;
        this.slots().scan_writers();
        Ok(this)
    }

    /// The bytes a vec of `capacity` slots and a writer table of `writers`
    /// entries takes, once `file` is found to hold them; a shorter file is
    /// refused naming the header field that differs from what was asked,
    /// when one does, else its length.
    fn long_enough(file: &File, capacity: usize, writers: usize) -> Result<usize, VecError> {
        let total = vec_file_size::<T>(capacity, writers);
        let len = file.metadata()?.len();
        if len < total as u64 {
            Self::check_header_of(file, capacity, writers)?;
            return Err(VecError::LayoutMismatch(crate::layout::Mismatch::new("length", len, total as u64)));
        }
        Ok(total)
    }

    /// Open a vec this process may only read, built with
    /// `expected_capacity` slots and a writer table of `writers` entries.
    ///
    /// [`open`](Self::open) needs a read+write file handle, which a
    /// consumer of a privileged producer's vec does not have - granting
    /// it would let any reader corrupt the state for all of them.
    /// Reads behave identically, each checked against its slot's state;
    /// writes return [`VecError::ReadOnly`].
    pub fn open_read_only(path: impl AsRef<Path>, expected_capacity: usize, writers: usize) -> Result<Self, VecError> {
        Self::admit(expected_capacity, writers)?;
        if matches!(Self::shape(), Shape::Word) && !crate::atomic_pair::plain_sixteen() {
            return Err(VecError::IoError(std::io::Error::new(
                std::io::ErrorKind::Unsupported,
                "this processor reads 16 bytes whole only by a swap, which a read-only mapping refuses",
            )));
        }
        let file = crate::region_file::open_read_only(path.as_ref())?;
        let total = Self::long_enough(&file, expected_capacity, writers)?;
        let mmap = unsafe { MmapOptions::new().len(total).map(&file)? };
        let this = Self::assemble(file, Mapping::ReadOnly(mmap), expected_capacity, writers);
        this.validate(expected_capacity, writers)?;
        Ok(this)
    }

    /// Whether the header on disk is the one this mapping expects, as
    /// [`check_header`](Self::check_header) says.
    fn validate(&self, expected_capacity: usize, writers: usize) -> Result<(), VecError> {
        Self::check_header(self.header(), expected_capacity, writers)
    }

    /// Whether `hdr` is the header of a vec of `T` with `capacity` slots and
    /// a writer table of `writers` entries. A geometry, element size or tag
    /// field at zero records nothing and constrains nothing; one that is set
    /// must be this type's.
    fn check_header(hdr: &VecHeader, capacity: usize, writers: usize) -> Result<(), VecError> {
        let shape = Self::shape();
        use crate::layout::Mismatch;
        Mismatch::first(&[
            ("magic", u64::from(hdr.magic), u64::from(VEC_MAGIC)),
            ("capacity", hdr.capacity, capacity as u64),
            ("writer slots", u64::from(hdr.writer_slots), writers as u64),
            Mismatch::recorded("slot stride", u64::from(hdr.slot_stride), shape.stride() as u64),
            Mismatch::recorded("payload offset", u64::from(hdr.payload_offset), shape.payload_offset() as u64),
            Mismatch::recorded("element size", u64::from(hdr.element_size), size_of::<T>() as u64),
            ("layout tag", hdr.layout_tag, 0),
        ])
        .map_err(VecError::LayoutMismatch)
    }

    /// Whether this mapping may be written.
    #[inline]
    pub fn is_writable(&self) -> bool {
        self.mmap.is_writable()
    }

    #[inline]
    pub fn capacity(&self) -> usize { self.capacity }

    /// Entries in the writer table, as the vec's creator sized it: the
    /// `writers` an [`open`](Self::open) must be given.
    #[inline]
    pub fn writer_slots(&self) -> usize {
        self.header().writer_slots as usize
    }

    /// Slots holding an element or a hole: the top word's length. A hole is
    /// a slot a pop took whose lowering of the length lost to a push; `get`
    /// answers None for it and the next pop steps past it.
    #[inline]
    pub fn len(&self) -> usize {
        self.header().len() as usize
    }

    #[inline]
    pub fn is_empty(&self) -> bool { self.len() == 0 }

    fn header(&self) -> &VecHeader {
        unsafe { &*(self.mmap.as_ptr() as *const VecHeader) }
    }

    /// The vec's slots, under the protocol both vecs share.
    #[inline(always)]
    fn slots(&self) -> VecSlots<'_> {
        // SAFETY: the mapping holds the validated header and then `capacity`
        // slots of this shape, 64-byte aligned, from the header's end, and
        // `admit` checked the processor's 16-byte swap.
        unsafe {
            VecSlots::new(self.header().top_word(), self.mmap.as_ptr().add(size_of::<VecHeader>()), self.capacity, Self::shape(), &self.writers)
        }
    }

    /// The start of slot `i`, for the tests' reach into a pair's claim.
    #[cfg(test)]
    fn slot(&self, i: usize) -> *const u8 {
        assert!(i < self.capacity, "slot index {i} out of bounds for cap {}", self.capacity);
        // SAFETY: the mapping holds the header and then `capacity` slots.
        unsafe { self.mmap.as_ptr().add(size_of::<VecHeader>() + i * Self::shape().stride()) }
    }

    /// `v` as a word slot's word.
    #[inline(always)]
    fn word_of(v: &T) -> u64 {
        // SAFETY: a T is readable for its size, at most 8 in a word slot.
        unsafe { crate::vec_core::word_from(v as *const T as *const u8, size_of::<T>()) }
    }

    /// The `T` a word slot's word holds, as a push wrote it.
    #[inline(always)]
    fn from_word(word: u64) -> T {
        let mut out = MaybeUninit::<T>::uninit();
        // SAFETY: the place holds a T's bytes, and a word slot's word starts
        // with the bytes of a T a push wrote.
        unsafe {
            crate::vec_core::word_into(word, out.as_mut_ptr() as *mut u8, size_of::<T>());
            out.assume_init()
        }
    }

    /// Append a value. Returns the index it landed at.
    /// Returns `Err(Full)` when the vec is at capacity, read with no locked
    /// operation.
    #[inline(always)]
    pub fn push_back(&self, v: T) -> Result<usize, VecError> {
        if !self.mmap.is_writable() {
            return Err(VecError::ReadOnly);
        }
        let landed = if matches!(Self::shape(), Shape::Word) {
            let slots = self.slots();
            slots.enter();
            slots.push_word(Self::word_of(&v))
        } else {
            // SAFETY: `v` is readable for a T's size, and the vec is writable.
            unsafe { self.slots().push(&v as *const T as *const u8, size_of::<T>()) }
        };
        match landed {
            Some(idx) => {
                self.ring_sidecar.push_op(crate::sidecar_ops::ordered::OP_INSERT, 0);
                Ok(idx)
            }
            None => self.refuse_push(),
        }
    }

    /// A refused push's return: records the refusal. Out of line, so a push
    /// holds nothing across a call.
    #[cold]
    #[inline(never)]
    fn refuse_push(&self) -> Result<usize, VecError> {
        self.ring_sidecar
            .push_op(crate::sidecar_ops::ordered::OP_INSERT, 1); // full
        Err(VecError::Full)
    }

    /// Remove and return the last element. Returns None when empty.
    #[inline(always)]
    pub fn pop_back(&self) -> Option<T> {
        if !self.mmap.is_writable() {
            return None;
        }
        let popped = if matches!(Self::shape(), Shape::Word) {
            let slots = self.slots();
            slots.enter();
            slots.pop_word().map(Self::from_word)
        } else {
            let mut out = MaybeUninit::<T>::uninit();
            // SAFETY: the place is writable for a T, and the vec is writable.
            if unsafe { self.slots().pop(out.as_mut_ptr() as *mut u8, size_of::<T>()) } {
                // SAFETY: the pop copied in the bytes of a T a push wrote.
                Some(unsafe { out.assume_init() })
            } else {
                None
            }
        };
        let code = if popped.is_some() { 0 } else { 2 }; // 2: empty
        self.ring_sidecar.push_op(crate::sidecar_ops::ordered::OP_POP, code);
        popped
    }

    /// Read the value at index `i`. Returns None when `i >= len` or names a
    /// hole.
    #[inline(always)]
    pub fn get(&self, i: usize) -> Option<T> {
        let got = self.read(i);
        let code = if got.is_some() { 0 } else { 2 }; // 2: out of bounds or a hole
        self.ring_sidecar.push_op(crate::sidecar_ops::ordered::OP_GET, code);
        got
    }

    /// Element `i`, or None past the length or at a hole.
    #[inline(always)]
    fn read(&self, i: usize) -> Option<T> {
        if matches!(Self::shape(), Shape::Word) {
            return self.slots().get_word(i).map(Self::from_word);
        }
        let mut out = MaybeUninit::<T>::uninit();
        // SAFETY: as in `pop_back`.
        if unsafe { self.slots().get(i, out.as_mut_ptr() as *mut u8, size_of::<T>()) } {
            // SAFETY: as in `pop_back`.
            Some(unsafe { out.assume_init() })
        } else {
            None
        }
    }

    /// Overwrite the value at index `i`. Returns `Err(OutOfBounds)`
    /// when `i >= len` or names a hole.
    pub fn set(&self, i: usize, v: T) -> Result<(), VecError> {
        if !self.mmap.is_writable() {
            return Err(VecError::ReadOnly);
        }
        let landed = if matches!(Self::shape(), Shape::Word) {
            let slots = self.slots();
            slots.enter();
            slots.set_word(i, Self::word_of(&v))
        } else {
            // SAFETY: as in `push_back`.
            unsafe { self.slots().set(i, &v as *const T as *const u8, size_of::<T>()) }
        };
        if !landed {
            self.ring_sidecar
                .push_op(crate::sidecar_ops::ordered::OP_INSERT, 1); // out of bounds (positional write rejected)
            return Err(VecError::OutOfBounds);
        }
        self.ring_sidecar
            .push_op(crate::sidecar_ops::ordered::OP_INSERT, 0);
        Ok(())
    }

    /// Clear the vec: the length to zero by one compare-and-swap of the top
    /// word. Slot payloads are not zeroed; they become unreachable through
    /// bounded indexing.
    pub fn clear(&self) {
        if !self.mmap.is_writable() {
            return;
        }
        self.slots().clear();
        self.ring_sidecar
            .push_op(crate::sidecar_ops::ordered::OP_REMOVE, 0);
    }

    /// Snapshot all current values into a Vec, holes left out. Best-effort:
    /// under concurrent writers, each value is one its slot held during the
    /// call, in index order over `len` as of entry.
    pub fn snapshot(&self) -> Vec<T> {
        let n = self.len();
        let mut out = Vec::with_capacity(n);
        for i in 0..n {
            if let Some(v) = self.read(i) {
                out.push(v);
            }
        }
        self.ring_sidecar
            .push_op(crate::sidecar_ops::ordered::OP_ITER, 0);
        out
    }

    /// Read every live slot in order and hand each to `f`, without
    /// building a `Vec`.
    ///
    /// For a consumer that looks at every element rather than keeping
    /// them. [`snapshot`](Self::snapshot) costs an allocation the size
    /// of the data plus a pass to fill it; this costs neither.
    ///
    /// Each slot is copied into a local as `get` copies it, so `f` never
    /// sees a torn value. A reference into the mapping would let a writer
    /// change the bytes under the reader.
    ///
    /// `f` is called in index order, over `len` as of entry, for each slot
    /// holding an element.
    pub fn for_each<F: FnMut(usize, &T)>(&self, f: F) {
        self.for_each_range(0, self.len(), f);
    }

    /// [`for_each`](Self::for_each) over `start..end`, clamped to the
    /// live length. The range form lets workers take disjoint spans
    /// without materializing their share first.
    pub fn for_each_range<F: FnMut(usize, &T)>(&self, start: usize, end: usize, mut f: F) {
        let end = end.min(self.len());
        for i in start..end {
            if let Some(v) = self.read(i) {
                f(i, &v);
            }
        }
        self.ring_sidecar
            .push_op(crate::sidecar_ops::ordered::OP_ITER, 0);
    }

    pub fn flush(&self) -> Result<(), VecError> {
        self.mmap.flush()?;
        Ok(())
    }

    /// Non-blocking flush: schedules a writeback via the OS.
    /// Note: Windows is only partially async (sync to page cache,
    /// not to disk).
    pub fn flush_async(&self) -> Result<(), VecError> {
        self.mmap.flush_async()?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::thread;

    /// The writer table the tests' vecs are made with.
    const WRITERS: usize = 4;

    /// A second create attaches with live elements in place; reset is
    /// what strips them.
    #[test]
    fn second_create_attaches_and_keeps_elements() {
        let p = tmp("attach");
        let v = SharedVec::<u64>::create(&p, 16, WRITERS).unwrap();
        v.push_back(777).unwrap();

        let v2 = SharedVec::<u64>::create(&p, 16, WRITERS).unwrap();
        assert_eq!(v2.len(), 1, "attach emptied a live vec");
        assert_eq!(v2.get(0), Some(777));
        let refused = |capacity, writers| match SharedVec::<u64>::create(&p, capacity, writers) {
            Err(VecError::LayoutMismatch(m)) => m,
            Err(e) => panic!("an attach of another shape was refused with another error: {e}"),
            Ok(_) => panic!("an attach of another shape was attached"),
        };
        let mismatch = crate::layout::Mismatch::new;
        assert_eq!(refused(8, WRITERS), mismatch("capacity", 16, 8));
        assert_eq!(refused(32, WRITERS), mismatch("capacity", 16, 32), "a longer file asked for names the field");
        assert_eq!(refused(16, WRITERS - 1), mismatch("writer slots", WRITERS as u64, WRITERS as u64 - 1));
        assert_eq!(refused(16, WRITERS + 8), mismatch("writer slots", WRITERS as u64, WRITERS as u64 + 8));

        // Windows refuses to truncate a mapped file, so every handle goes
        // before the reset.
        drop(v);
        drop(v2);
        let fresh = SharedVec::<u64>::reset(&p, 16, WRITERS).unwrap();
        assert_eq!(fresh.len(), 0, "reset kept an element");
        drop(fresh);
    }

    #[test]
    fn for_each_visits_every_live_slot_in_order() {
        let p = tmp("foreach");
        let v = SharedVec::<u64>::create(&p, 256, WRITERS).unwrap();
        for i in 0..100u64 {
            v.push_back(i * 3).unwrap();
        }
        let mut seen = Vec::new();
        v.for_each(|i, x| seen.push((i, *x)));
        assert_eq!(seen.len(), 100, "capacity beyond len is not visited");
        assert!(seen.iter().enumerate().all(|(n, (i, x))| *i == n && *x == n as u64 * 3));
    }

    #[test]
    fn for_each_range_is_clamped_and_disjoint() {
        let p = tmp("foreach-range");
        let v = SharedVec::<u64>::create(&p, 64, WRITERS).unwrap();
        for i in 0..40u64 {
            v.push_back(i).unwrap();
        }
        let mut first = Vec::new();
        let mut second = Vec::new();
        v.for_each_range(0, 20, |_, x| first.push(*x));
        v.for_each_range(20, 40, |_, x| second.push(*x));
        // Together the spans are the whole vec, once each.
        first.extend_from_slice(&second);
        assert_eq!(first, (0..40u64).collect::<Vec<_>>());
        // An end past len is clamped, not out of bounds.
        let mut over = Vec::new();
        v.for_each_range(30, 9999, |_, x| over.push(*x));
        assert_eq!(over.len(), 10);
    }

    #[test]
    fn for_each_agrees_with_snapshot() {
        let p = tmp("foreach-vs-snapshot");
        let v = SharedVec::<u64>::create(&p, 512, WRITERS).unwrap();
        for i in 0..300u64 {
            v.push_back(i ^ 0xa5a5).unwrap();
        }
        let mut walked = Vec::new();
        v.for_each(|_, x| walked.push(*x));
        assert_eq!(walked, v.snapshot());
    }

    #[test]
    fn a_read_only_vec_reads_the_same_and_refuses_writes() {
        let p = tmp("readonly");
        {
            let w = SharedVec::<u64>::create(&p, 64, WRITERS).unwrap();
            for i in 0..10u64 {
                w.push_back(i * 7).unwrap();
            }
            w.flush().unwrap();
        }
        let r = SharedVec::<u64>::open_read_only(&p, 64, WRITERS).unwrap();
        assert!(!r.is_writable());
        assert_eq!(r.len(), 10);
        assert_eq!(r.get(3), Some(21));
        assert_eq!(r.snapshot(), (0..10u64).map(|i| i * 7).collect::<Vec<_>>());
        assert!(matches!(r.push_back(1), Err(VecError::ReadOnly)));
        assert!(matches!(r.set(0, 1), Err(VecError::ReadOnly)));
        assert_eq!(r.pop_back(), None);
        r.clear();
        assert_eq!(r.len(), 10, "clear on a read-only vec changes nothing");
        r.flush().unwrap();
    }

    #[test]
    fn a_read_only_open_still_validates_the_header() {
        let p = tmp("readonly-mismatch");
        {
            let w = SharedVec::<u64>::create(&p, 64, WRITERS).unwrap();
            w.push_back(1).unwrap();
            w.flush().unwrap();
        }
        assert!(matches!(
            SharedVec::<u64>::open_read_only(&p, 32, WRITERS).err(),
            Some(VecError::LayoutMismatch(crate::layout::Mismatch {
                field: "capacity",
                found: 64,
                wanted: 32,
            }))
        ));
    }

    /// A file for one test, removed when the test ends; declared before the
    /// primitive that maps it so it drops after it.
    fn tmp(name: &str) -> crate::test_paths::TmpFile {
        crate::test_paths::TmpFile::new(format!("subetha-vec-{name}-{}.bin", std::process::id()))
    }

    /// A push into a full vec is refused at the top word's length, which it
    /// reads and leaves as it stands; a pop makes room, and clear empties it.
    #[test]
    fn a_full_vec_refuses_at_its_length_and_a_pop_makes_room() {
        let p = tmp("full-length");
        let v: SharedVec<u32> = SharedVec::create(&p, 4, WRITERS).unwrap();
        for i in 0..4u32 { v.push_back(i).unwrap(); }
        let top = || v.header().top.load(Ordering::Acquire);
        let before = top();
        assert!(matches!(v.push_back(8), Err(VecError::Full)));
        assert!(matches!(v.push_back(9), Err(VecError::Full)));
        assert_eq!(top(), before, "a refused push moves no word");
        assert_eq!(v.pop_back(), Some(3));
        assert_eq!(v.push_back(9).unwrap(), 3);
        assert!(matches!(v.push_back(10), Err(VecError::Full)));
        v.clear();
        assert_eq!(v.push_back(5).unwrap(), 0);
    }

    /// The live writers the vec's top word counts.
    fn writers<T: Copy + 'static>(v: &SharedVec<T>) -> u64 {
        crate::vec_core::writers_of(v.header().top_word().load())
    }

    /// A second writer flips the vec to the multi-writer protocol while it
    /// runs; once it exits, the first writer's next write gives its
    /// registration back and the vec has one writer again.
    #[test]
    fn a_second_writer_that_exits_returns_the_vec_to_one_writer() {
        let p = tmp("second-writer-exit");
        let v = Arc::new(SharedVec::<u64>::create(&p, 64, WRITERS).unwrap());
        v.push_back(1).unwrap();
        assert_eq!(writers(&v), 1);
        let other = Arc::clone(&v);
        thread::spawn(move || {
            other.push_back(2).unwrap();
            assert_eq!(writers(&other), 2, "two writers");
        })
        .join()
        .unwrap();
        assert_eq!(writers(&v), 2, "the exited writer waits for the next write");
        assert_eq!(v.pop_back(), Some(2));
        assert_eq!(writers(&v), 1, "the first writer's next write gave it back");
        assert_eq!(v.pop_back(), Some(1));
        assert_eq!(v.pop_back(), None);
    }

    /// A second writer's every later write finds it registered: the count
    /// stays at two however often it pushes and pops, and is one again once
    /// it has exited and the first writer writes.
    #[test]
    fn a_second_writer_is_counted_once_however_often_it_writes() {
        let p = tmp("second-writer-count");
        let v = Arc::new(SharedVec::<u64>::create(&p, 256, WRITERS).unwrap());
        v.push_back(0).unwrap();
        let other = Arc::clone(&v);
        thread::spawn(move || {
            for i in 1..=100u64 {
                other.push_back(i).unwrap();
                assert_eq!(writers(&other), 2, "write {i}");
                if i % 3 == 0 {
                    assert_eq!(other.pop_back(), Some(i));
                }
            }
        })
        .join()
        .unwrap();
        v.push_back(1000).unwrap();
        assert_eq!(writers(&v), 1);
        assert_eq!(v.len(), 1 + 100 - 33 + 1);
    }

    /// Threads that each write through one handle and exit, one after
    /// another, each find the last one's registration given back by their own
    /// first write and so write alone; the count follows the threads alive.
    #[test]
    fn writer_threads_that_exit_leave_the_count_at_the_live_ones() {
        let p = tmp("writer-churn");
        let v = Arc::new(SharedVec::<u64>::create(&p, 64, WRITERS).unwrap());
        for i in 0..32u64 {
            let mine = Arc::clone(&v);
            thread::spawn(move || {
                mine.push_back(i).unwrap();
                assert_eq!(writers(&mine), 1, "thread {i} writes alone");
            })
            .join()
            .unwrap();
        }
        assert_eq!(writers(&v), 1, "the last thread's registration waits for the next write");
        v.push_back(99).unwrap();
        assert_eq!(writers(&v), 1, "the main thread writes alone");
        assert_eq!(v.snapshot(), (0..32u64).chain([99]).collect::<Vec<_>>());
    }

    /// A handle that drops gives back the registration of every thread that
    /// wrote through it, still running or not, and such a thread exiting
    /// after the drop gives back nothing more.
    #[test]
    fn a_dropped_handle_gives_back_its_running_writers() {
        let p = tmp("drop-running-writer");
        let v = Arc::new(SharedVec::<u64>::create(&p, 64, WRITERS).unwrap());
        let watcher = SharedVec::<u64>::create(&p, 64, WRITERS).unwrap();
        v.push_back(1).unwrap();
        let (wrote_tx, wrote_rx) = std::sync::mpsc::channel();
        let (go_tx, go_rx) = std::sync::mpsc::channel::<()>();
        let other = Arc::clone(&v);
        let runner = thread::spawn(move || {
            other.push_back(2).unwrap();
            drop(other);
            wrote_tx.send(()).unwrap();
            go_rx.recv().unwrap();
        });
        wrote_rx.recv().unwrap();
        assert_eq!(writers(&watcher), 2);
        drop(v);
        assert_eq!(writers(&watcher), 0, "the drop gave back both, one thread still running");
        go_tx.send(()).unwrap();
        runner.join().unwrap();
        assert_eq!(writers(&watcher), 0, "the thread's exit gave back nothing more");
        assert_eq!(watcher.snapshot(), vec![1, 2]);
    }

    /// One thread writing through two handles of one vec pushes and pops
    /// through either, every value once, and the two handles' drops leave no
    /// writer counted.
    #[test]
    fn one_thread_writing_through_two_handles_loses_and_repeats_nothing() {
        let p = tmp("two-handles");
        let a = SharedVec::<u64>::create(&p, 64, WRITERS).unwrap();
        let b = SharedVec::<u64>::create(&p, 64, WRITERS).unwrap();
        for i in 0..40u64 {
            if i % 2 == 0 { a.push_back(i).unwrap() } else { b.push_back(i).unwrap() };
            if i % 3 == 2 {
                let popped = if i % 2 == 0 { b.pop_back() } else { a.pop_back() };
                assert_eq!(popped, Some(i), "the pop took the last push");
            }
        }
        let left: Vec<u64> = (0..40u64).filter(|i| i % 3 != 2).collect();
        assert_eq!(a.snapshot(), left);
        drop(a);
        assert_eq!(writers(&b), 1, "the first handle's drop gave back its registration");
        drop(b);
        let watcher = SharedVec::<u64>::create(&p, 64, WRITERS).unwrap();
        assert_eq!(writers(&watcher), 0);
    }

    /// Entry `i` of the vec's writer table.
    fn table_entry<T: Copy + 'static>(v: &SharedVec<T>, i: usize) -> &crate::atomic_pair::AtomicPair {
        assert!(i < WRITERS);
        // SAFETY: entry `i` of the table after the slots, 16-byte aligned,
        // which the mapping holds.
        unsafe { crate::atomic_pair::AtomicPair::at(v.mmap.as_ptr().add(vec_table_offset::<T>(v.capacity) + i * 16) as *mut u8) }
    }

    /// Entry `i` of the vec's writer table: its holder word and the
    /// registrations it counts.
    fn entry<T: Copy + 'static>(v: &SharedVec<T>, i: usize) -> (u64, u64) {
        let held = table_entry(v, i).load();
        (held as u64, (held >> 64) as u64)
    }

    /// Write entry `i` as a handle of the process `holder` names holds it
    /// with one registration, counted in the top word as that handle's first
    /// registration counts it.
    fn plant_writer<T: Copy + 'static>(v: &SharedVec<T>, i: usize, holder: u64) {
        table_entry(v, i).store(u128::from(holder) | (1 << 64));
        v.slots().add_writer_with_entry();
    }

    /// A holder word that names no process.
    fn gone_holder() -> u64 {
        crate::process_identity::holder(u32::MAX - 1, 0)
    }

    /// A handle's first write takes the first free entry for its process, its
    /// writing threads counted in it, and its drop frees the entry.
    #[test]
    fn a_handle_takes_one_entry_and_frees_it_when_it_drops() {
        let p = tmp("entry-take-free");
        let v = Arc::new(SharedVec::<u64>::create(&p, 64, WRITERS).unwrap());
        let watcher = SharedVec::<u64>::open(&p, 64, WRITERS).unwrap();
        assert_eq!(entry(&watcher, 0), (0, 0), "attaching takes no entry");
        v.push_back(1).unwrap();
        let me = crate::process_identity::current();
        assert_eq!(entry(&watcher, 0), (me, 1));
        let other = Arc::clone(&v);
        let (wrote_tx, wrote_rx) = std::sync::mpsc::channel();
        let (go_tx, go_rx) = std::sync::mpsc::channel::<()>();
        let runner = thread::spawn(move || {
            other.push_back(2).unwrap();
            wrote_tx.send(()).unwrap();
            go_rx.recv().unwrap();
        });
        wrote_rx.recv().unwrap();
        assert_eq!(entry(&watcher, 0), (me, 2), "a second writing thread counts in the same entry");
        assert_eq!(entry(&watcher, 1), (0, 0));
        go_tx.send(()).unwrap();
        runner.join().unwrap();
        drop(v);
        assert_eq!(entry(&watcher, 0), (0, 0), "the drop gave every registration back and freed the entry");
        assert_eq!(writers(&watcher), 0);
    }

    /// Writers past the table's size write all the same, counted in the top
    /// word without an entry, and their handle's drop gives them back.
    #[test]
    fn a_full_writer_table_leaves_writers_counted_without_an_entry() {
        let p = tmp("entry-full");
        let v = SharedVec::<u64>::create(&p, 64, 1).unwrap();
        let second = SharedVec::<u64>::open(&p, 64, 1).unwrap();
        v.push_back(1).unwrap();
        second.push_back(2).unwrap();
        assert_eq!(writers(&v), 2, "both handles' writers counted");
        let one = |v: &SharedVec<u64>| {
            // SAFETY: the single entry of the table after the slots.
            unsafe { crate::atomic_pair::AtomicPair::at(v.mmap.as_ptr().add(vec_table_offset::<u64>(64)) as *mut u8) }.load()
        };
        assert_eq!(one(&v), u128::from(crate::process_identity::current()) | (1 << 64), "the first handle's entry");
        assert_eq!(second.pop_back(), Some(2));
        assert_eq!(v.pop_back(), Some(1));
        drop(second);
        assert_eq!(writers(&v), 1, "the drop gave back the writer counted without an entry");
        v.push_back(3).unwrap();
        assert_eq!(v.snapshot(), vec![3]);
    }

    /// An entry whose holder is gone is given back when another handle
    /// attaches: its registration leaves the top word's count and the entry
    /// is free again.
    #[test]
    fn attaching_gives_back_a_gone_processes_registrations() {
        let p = tmp("reap-on-attach");
        let v = SharedVec::<u64>::create(&p, 16, WRITERS).unwrap();
        v.push_back(1).unwrap();
        plant_writer(&v, 1, gone_holder());
        assert_eq!(writers(&v), 2, "the planted writer is counted");
        let other = SharedVec::<u64>::open(&p, 16, WRITERS).unwrap();
        assert_eq!(writers(&other), 1, "the attach gave the gone writer's registration back");
        assert_eq!(entry(&other, 1), (0, 0), "and freed its entry");
        assert_eq!(entry(&other, 0), (crate::process_identity::current(), 1), "this process's entry stands");
    }

    /// A writer that finds the table generation moved looks at the table on
    /// its next multi-writer push, gives back the registrations of a holder
    /// that is gone, and writes alone again.
    #[test]
    fn a_multi_writer_push_gives_back_a_gone_processes_registrations() {
        let p = tmp("reap-on-push");
        let v = SharedVec::<u64>::create(&p, 16, WRITERS).unwrap();
        v.push_back(1).unwrap();
        plant_writer(&v, 1, gone_holder());
        assert_eq!(writers(&v), 2);
        v.push_back(2).unwrap();
        assert_eq!(writers(&v), 1, "the push looked at the table and gave the gone writer back");
        assert_eq!(entry(&v, 1), (0, 0));
        assert_eq!(v.pop_back(), Some(2));
        assert_eq!(v.snapshot(), vec![1]);
    }

    /// A gone writer's reservation that raised the length over an element a
    /// clear removed, never claimed, reads as a hole once its registrations
    /// are given back and the vec has one writer again, whose reads judge a
    /// slot by its kind alone.
    #[test]
    fn a_gone_writers_unclaimed_reservation_reads_as_a_hole() {
        let p = tmp("reap-reservation");
        let v = SharedVec::<u64>::create(&p, 16, WRITERS).unwrap();
        for i in 1..=3 {
            v.push_back(i).unwrap();
        }
        v.clear();
        plant_writer(&v, 1, gone_holder());
        v.header().top_word().fetch_add_lo(1);
        assert_eq!(v.len(), 1, "the reservation's add");
        assert_eq!(v.get(0), None, "with two writers a read judges the claim's epoch");
        let other = SharedVec::<u64>::open(&p, 16, WRITERS).unwrap();
        assert_eq!(writers(&other), 1, "the attach gave the gone writer back");
        v.push_back(9).unwrap();
        assert!(v.header().top.load(Ordering::Acquire) & (1 << 32) == 0, "the push flipped the vec back");
        assert_eq!(v.get(0), None, "the slot the reservation uncovered is a hole");
        assert_eq!(v.get(1), Some(9));
        assert_eq!(v.snapshot(), vec![9]);
    }

    /// A writer that finds a live holder in the table watches it; once that
    /// process exits, its next multi-writer push gives the registration back
    /// and the vec has one writer again.
    #[test]
    fn a_watched_writer_process_that_exits_is_given_back() {
        let exe = std::env::current_exe().unwrap();
        let mut child = std::process::Command::new(exe)
            .args(["--exact", "exit_watch::tests::sleeper_never_runs_on_its_own", "--ignored", "--nocapture"])
            .env("SUBETHA_EXIT_WATCH_SLEEP", "1")
            .spawn()
            .unwrap();
        let p = tmp("reap-on-exit");
        let v = SharedVec::<u64>::create(&p, 64, WRITERS).unwrap();
        v.push_back(1).unwrap();
        let holder = crate::process_identity::holder_for(child.id());
        plant_writer(&v, 1, holder);
        v.push_back(2).unwrap();
        assert_eq!(writers(&v), 2, "a live writer process stays counted");
        assert_eq!(crate::exit_watch::told(holder), Some(false), "and is watched");
        child.kill().unwrap();
        child.wait().unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while writers(&v) == 2 {
            assert!(std::time::Instant::now() < deadline, "the exit was given back");
            v.push_back(3).unwrap();
            assert_eq!(v.pop_back(), Some(3));
            thread::sleep(std::time::Duration::from_millis(1));
        }
        assert_eq!(writers(&v), 1);
        assert_eq!(entry(&v, 1), (0, 0));
        assert_eq!(v.snapshot(), vec![1, 2]);
    }

    /// Pushers racing a popper at the capacity keep landing until the popper
    /// has taken POPS values, under any scheduling, and a drained vec takes a
    /// push. The popper runs in its own thread so the deadline holds even if
    /// a pop never returned. A push the full vec refuses, or a pop that finds
    /// it empty, yields its thread's turn, so the workers keep the values
    /// moving when they outnumber the CPUs.
    #[test]
    fn racing_pushes_and_a_pop_keep_landing_and_a_drained_vec_takes_a_push() {
        const CAPACITY: u64 = 8;
        const POPS: u64 = 2_000;
        let p = tmp("full-race");
        let v: Arc<SharedVec<u64>> = Arc::new(SharedVec::create(&p, CAPACITY as usize, WRITERS).unwrap());
        let popped = Arc::new(AtomicU64::new(0));
        let pushers: Vec<_> = (0..3u64)
            .map(|t| {
                let v = v.clone();
                let popped = popped.clone();
                thread::spawn(move || {
                    let mut i = 0u64;
                    while popped.load(Ordering::Relaxed) < POPS {
                        match v.push_back((t << 32) | i) {
                            Ok(_) => {}
                            Err(VecError::Full) => thread::yield_now(),
                            Err(e) => panic!("a push failed other than full: {e:?}"),
                        }
                        i += 1;
                    }
                })
            })
            .collect();
        let popper = {
            let (v, popped) = (v.clone(), popped.clone());
            thread::spawn(move || {
                while popped.load(Ordering::Relaxed) < POPS {
                    if v.pop_back().is_some() {
                        popped.fetch_add(1, Ordering::Relaxed);
                    } else {
                        thread::yield_now();
                    }
                }
            })
        };
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(120);
        while popped.load(Ordering::Relaxed) < POPS {
            assert!(
                std::time::Instant::now() < deadline,
                "pops stopped at {} of {POPS}: top word {:#x}",
                popped.load(Ordering::Relaxed),
                v.header().top.load(Ordering::Acquire),
            );
            thread::sleep(std::time::Duration::from_millis(10));
        }
        popper.join().unwrap();
        for p in pushers {
            p.join().unwrap();
        }
        while v.pop_back().is_some() {}
        assert_eq!(v.len(), 0, "a drained vec has no length left");
        v.push_back(1).unwrap();
    }

    #[test]
    fn create_initial_state_is_empty() {
        let p = tmp("init");
        let v: SharedVec<u32> = SharedVec::create(&p, 16, WRITERS).unwrap();
        assert_eq!(v.capacity(), 16);
        assert_eq!(v.len(), 0);
        assert!(v.is_empty());
        assert_eq!(v.get(0), None);
    }

    #[test]
    fn push_back_advances_len_and_get_round_trip() {
        let p = tmp("push");
        let v: SharedVec<u32> = SharedVec::create(&p, 8, WRITERS).unwrap();
        for i in 0..5u32 {
            let idx = v.push_back(i * 10).unwrap();
            assert_eq!(idx, i as usize);
        }
        assert_eq!(v.len(), 5);
        for i in 0..5 {
            assert_eq!(v.get(i), Some((i as u32) * 10));
        }
        assert_eq!(v.get(5), None);
    }

    #[test]
    fn full_capacity_returns_error() {
        let p = tmp("full");
        let v: SharedVec<u32> = SharedVec::create(&p, 4, WRITERS).unwrap();
        for i in 0..4u32 { v.push_back(i).unwrap(); }
        assert!(matches!(v.push_back(99).err(), Some(VecError::Full)));
        assert_eq!(v.len(), 4);  // rolled back
    }

    #[test]
    fn pop_back_returns_last_then_none() {
        let p = tmp("pop");
        let v: SharedVec<u32> = SharedVec::create(&p, 8, WRITERS).unwrap();
        v.push_back(10).unwrap();
        v.push_back(20).unwrap();
        v.push_back(30).unwrap();
        assert_eq!(v.pop_back(), Some(30));
        assert_eq!(v.pop_back(), Some(20));
        assert_eq!(v.len(), 1);
        assert_eq!(v.pop_back(), Some(10));
        assert_eq!(v.pop_back(), None);
    }

    #[test]
    fn set_replaces_value_at_index() {
        let p = tmp("set");
        let v: SharedVec<u32> = SharedVec::create(&p, 8, WRITERS).unwrap();
        v.push_back(1).unwrap();
        v.push_back(2).unwrap();
        v.set(0, 100).unwrap();
        assert_eq!(v.get(0), Some(100));
        assert_eq!(v.get(1), Some(2));
        assert!(matches!(v.set(2, 200).err(), Some(VecError::OutOfBounds)));
    }

    #[test]
    fn clear_resets_len_to_zero() {
        let p = tmp("clear");
        let v: SharedVec<u32> = SharedVec::create(&p, 8, WRITERS).unwrap();
        for i in 0..5u32 { v.push_back(i).unwrap(); }
        assert_eq!(v.len(), 5);
        v.clear();
        assert_eq!(v.len(), 0);
        assert_eq!(v.get(0), None);
        // After clear, push works fresh.
        v.push_back(42).unwrap();
        assert_eq!(v.get(0), Some(42));
    }

    #[test]
    fn snapshot_returns_consistent_prefix() {
        let p = tmp("snapshot");
        let v: SharedVec<u32> = SharedVec::create(&p, 16, WRITERS).unwrap();
        for i in 0..7u32 { v.push_back(i + 100).unwrap(); }
        let snap = v.snapshot();
        assert_eq!(snap, vec![100, 101, 102, 103, 104, 105, 106]);
    }

    #[test]
    fn cross_handle_visibility() {
        let p = tmp("cross-handle");
        let writer: SharedVec<u32> = SharedVec::create(&p, 8, WRITERS).unwrap();
        let reader: SharedVec<u32> = SharedVec::open(&p, 8, WRITERS).unwrap();
        writer.push_back(777).unwrap();
        assert_eq!(reader.get(0), Some(777));
        reader.push_back(888).unwrap();
        assert_eq!(writer.get(1), Some(888));
        assert_eq!(writer.len(), 2);
    }

    #[test]
    fn concurrent_pushers_all_land_at_distinct_indices() {
        let p = tmp("concurrent");
        let v: Arc<SharedVec<u32>> = Arc::new(SharedVec::create(&p, 1024, WRITERS).unwrap());
        let n_threads = 4;
        let per_thread = 50u32;
        let mut handles = vec![];
        for t in 0..n_threads {
            let v = v.clone();
            handles.push(thread::spawn(move || {
                let mut indices = vec![];
                for i in 0..per_thread {
                    let val = (t as u32) * per_thread + i;
                    let idx = v.push_back(val).unwrap();
                    indices.push(idx);
                }
                indices
            }));
        }
        let mut all_indices: Vec<usize> = handles.into_iter()
            .flat_map(|h| h.join().unwrap())
            .collect();
        all_indices.sort();
        for (expected, actual) in all_indices.iter().enumerate() {
            assert_eq!(*actual, expected,
                "indices must form a contiguous 0..N sequence");
        }
        assert_eq!(v.len(), (n_threads * per_thread as usize));
    }

    #[test]
    fn payload_too_large_at_create() {
        #[allow(dead_code)] // size_of<Big> is the test signal, not the field
        struct Big([u8; VEC_PAYLOAD_BYTES + 1]);
        impl Copy for Big {}
        impl Clone for Big { fn clone(&self) -> Self { *self } }
        let p = tmp("too-large");
        let r = SharedVec::<Big>::create(&p, 4, WRITERS);
        assert!(matches!(r.err(), Some(VecError::PayloadTooLarge)));
    }

    #[test]
    fn struct_payload_round_trip() {
        #[derive(Clone, Copy, Debug, PartialEq)]
        #[repr(C)]
        struct Point { x: f64, y: f64, z: f64 }
        let p = tmp("struct");
        let v: SharedVec<Point> = SharedVec::create(&p, 8, WRITERS).unwrap();
        v.push_back(Point { x: 1.0, y: 2.0, z: 3.0 }).unwrap();
        v.push_back(Point { x: -1.5, y: 0.0, z: 7.25 }).unwrap();
        assert_eq!(v.get(0), Some(Point { x: 1.0, y: 2.0, z: 3.0 }));
        assert_eq!(v.get(1), Some(Point { x: -1.5, y: 0.0, z: 7.25 }));
    }

    #[test]
    fn disk_persistence_data_survives_reopen() {
        let p = tmp("disk");
        {
            let v: SharedVec<u32> = SharedVec::create(&p, 8, WRITERS).unwrap();
            for i in 0..4u32 { v.push_back(i * 100).unwrap(); }
            v.flush().unwrap();
        }
        let v2: SharedVec<u32> = SharedVec::open(&p, 8, WRITERS).unwrap();
        assert_eq!(v2.len(), 4);
        for i in 0..4 {
            assert_eq!(v2.get(i), Some((i as u32) * 100));
        }
    }

    /// A slot the length covers before its element is claimed reads as a
    /// hole, never as the zeroed region: many pushers race readers that read
    /// every slot below the length on every pass, and any value but the
    /// sentinel is the race. The window between an index's reservation and
    /// its claim is only a few instructions wide, so both sides are
    /// hammered. With no pop, every reservation is claimed by the time the
    /// pushers stop.
    #[test]
    fn a_slot_below_the_length_reads_its_element_or_a_hole() {
        let p = tmp("commit-order");
        let v: Arc<SharedVec<u32>> = Arc::new(SharedVec::create(&p, 4096, WRITERS).unwrap());
        let stop = Arc::new(std::sync::atomic::AtomicBool::new(false));

        // Every pusher writes the same sentinel. A slot the length covers
        // before its payload landed still reads as the zeroed region, so
        // any zero a reader sees is the race.
        const WRITTEN: u32 = 0xABCD_EF01;

        let readers: Vec<_> = (0..3)
            .map(|_| {
                let v = v.clone();
                let stop = stop.clone();
                thread::spawn(move || {
                    while !stop.load(Ordering::Acquire) {
                        let n = v.len();
                        for i in 0..n {
                            let got = v.get(i);
                            assert!(matches!(got, Some(WRITTEN) | None), "slot {i} read {got:?} before its element was written");
                        }
                    }
                })
            })
            .collect();

        let writers: Vec<_> = (0..4)
            .map(|_| {
                let v = v.clone();
                thread::spawn(move || loop {
                    match v.push_back(WRITTEN) {
                        Ok(idx) if idx >= 4000 => break,
                        Ok(_) => {}
                        Err(e) => panic!("a push below the capacity failed: {e:?}"),
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
        assert!((0..v.len()).all(|i| v.get(i) == Some(WRITTEN)), "every reservation was claimed");
    }

    /// Slot `i`'s claim word.
    fn claim_of<T: Copy>(v: &SharedVec<T>, i: usize) -> &AtomicU64 {
        // SAFETY: the slot lies in the mapping, laid out as a VecSlot.
        unsafe { &(*(v.slot(i) as *const VecSlot)).claim }
    }

    /// The pid of a child process that has been killed and reaped.
    fn exited_pid() -> u32 {
        let mut child = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "exit_watch::tests::sleeper_never_runs_on_its_own", "--ignored", "--nocapture"])
            .env("SUBETHA_EXIT_WATCH_SLEEP", "1")
            .spawn()
            .unwrap();
        let pid = child.id();
        child.kill().unwrap();
        child.wait().unwrap();
        pid
    }

    /// A writer that died holding a slot's claim mid-copy leaves the slot
    /// readable at its last value, and the next write takes the claim over.
    #[test]
    fn a_claim_left_by_a_dead_writer_is_taken_over_by_the_next_write() {
        let p = tmp("dead-writer");
        let v: SharedVec<[u64; 7]> = SharedVec::create(&p, 4, WRITERS).unwrap();
        v.push_back([1; 7]).unwrap();
        claim_of(&v, 0).store(crate::process_identity::holder(exited_pid(), 0), Ordering::Release);
        assert_eq!(v.get(0), Some([1; 7]), "a read does not wait on the dead writer");
        v.set(0, [2; 7]).unwrap();
        assert_eq!(v.get(0), Some([2; 7]));
        assert_eq!(claim_of(&v, 0).load(Ordering::Acquire), 0, "the write freed the claim it took over");
    }

    /// A read returns at once while a live writer holds the slot's claim.
    #[test]
    fn a_read_answers_while_a_live_writer_holds_the_slot() {
        let p = tmp("live-claim");
        let v: SharedVec<[u64; 7]> = SharedVec::create(&p, 4, WRITERS).unwrap();
        v.push_back([5; 7]).unwrap();
        claim_of(&v, 0).store(crate::process_identity::current(), Ordering::Release);
        assert_eq!(v.get(0), Some([5; 7]));
        claim_of(&v, 0).store(0, Ordering::Release);
    }

    /// Writers racing on one index each write a value whose seven words are
    /// equal, and readers check every value they read: a torn copy would
    /// mix two writers' words.
    #[test]
    fn writers_racing_on_one_index_never_tear_a_value() {
        let p = tmp("racing-set");
        let v: Arc<SharedVec<[u64; 7]>> = Arc::new(SharedVec::create(&p, 4, WRITERS).unwrap());
        v.push_back([0; 7]).unwrap();
        let done = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let readers: Vec<_> = (0..2)
            .map(|_| {
                let (v, done) = (v.clone(), done.clone());
                thread::spawn(move || {
                    while !done.load(Ordering::Acquire) {
                        let value = v.get(0).unwrap();
                        assert!(value.iter().all(|&w| w == value[0]), "a torn value: {value:x?}");
                    }
                })
            })
            .collect();
        let writers: Vec<_> = (0..4u64)
            .map(|w| {
                let v = v.clone();
                thread::spawn(move || {
                    for round in 0..10_000u64 {
                        v.set(0, [(w << 32) | round; 7]).unwrap();
                    }
                })
            })
            .collect();
        for w in writers {
            w.join().unwrap();
        }
        done.store(true, Ordering::Release);
        for r in readers {
            r.join().unwrap();
        }
    }

    #[test]
    fn concurrent_reader_during_writes_sees_consistent_data() {
        let p = tmp("read-during-write");
        let v: Arc<SharedVec<u32>> = Arc::new(SharedVec::create(&p, 256, WRITERS).unwrap());
        let v_w = v.clone();
        let writer = thread::spawn(move || {
            for i in 0..100u32 {
                v_w.push_back(i).unwrap();
            }
        });
        let v_r = v.clone();
        let reader = thread::spawn(move || {
            let mut last_len = 0;
            loop {
                let n = v_r.len();
                if n == 100 { break; }
                // Read every visible slot; values must equal index.
                for i in last_len..n {
                    let got = v_r.get(i);
                    assert_eq!(got, Some(i as u32),
                        "slot {i} should hold {i}, got {got:?}");
                }
                last_len = n;
                std::thread::yield_now();
            }
        });
        writer.join().unwrap();
        reader.join().unwrap();
    }
}
