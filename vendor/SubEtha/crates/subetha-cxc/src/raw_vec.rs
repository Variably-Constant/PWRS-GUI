//! `RawVec`: the bounded, indexable, append-only sequence of
//! [`SharedVec`](crate::shared_vec::SharedVec) at an element size and
//! alignment fixed at run time instead of by a type parameter, for a
//! caller reaching the region from another language. It reads and writes
//! the same file format under the same protocol (see the typed vec's
//! concurrency notes): the header with its top word and writer table, and
//! slots that each claim and publish an element with one swap. The region
//! records the element size, the slot geometry, the writer table's size and
//! a tag the caller chooses, and an open that states another layout is
//! refused.
//!
//! # Slot geometry
//!
//! An element of at most 8 bytes at an alignment of at most 8 lands in a
//! 64-byte word slot, its word at 0, as in the typed vec. A larger one takes
//! a pair of buffers: the slot starts with its head and claim words, then
//! holds two buffers, each at the first offset that honors the element's
//! alignment, 8 at the least, and pads to whole cache lines, or to the
//! alignment when that is wider. An element of at most 56 bytes at an
//! alignment of at most 8 lands in the 128-byte pair the typed vec uses, its
//! buffers at 16 and 72 whatever its size. Either way the two open each
//! other's regions. The slot array starts at the first multiple of the
//! alignment past the header, and the writer table follows the slots.

use std::fs::File;
use std::mem::size_of;
use std::path::Path;

use memmap2::MmapOptions;

use crate::raw_treiber_stack::ElementLayout;
use crate::shared_vec::{vec_geometry, Mapping, VecError, VecHeader, VEC_MAGIC, VEC_PAYLOAD_BYTES};
use crate::slot_pair::{self, SlotGeometry};
use crate::vec_core::{word_from, word_into, Shape, VecSlots, WriterEntry, Writers};

/// Bytes a slot spends on its head and claim words before its first
/// buffer, for an element of alignment at most 8.
pub const VEC_SLOT_PREFIX: usize = slot_pair::SLOT_PREFIX;

/// Where a pair of buffers' parts lie for `layout`: the typed vec's pair
/// for an element it could hold, else two buffers placed for the element.
fn slot_geometry(layout: &ElementLayout) -> SlotGeometry {
    if layout.slot_size <= VEC_PAYLOAD_BYTES && layout.alignment <= 8 {
        vec_geometry(layout.slot_size)
    } else {
        SlotGeometry::new(layout.slot_size, layout.alignment)
    }
}

/// How a slot holds an element of `layout`: a word slot up to 8 bytes at an
/// alignment of at most 8, else a pair of buffers.
fn slot_shape(layout: &ElementLayout) -> Shape {
    Shape::for_element(layout.slot_size, layout.alignment, slot_geometry(layout))
}

/// Where a slot's element starts and how wide the slot is for `layout`:
/// `(payload_offset, stride)`.
pub fn raw_vec_geometry(layout: &ElementLayout) -> (usize, usize) {
    let shape = slot_shape(layout);
    (shape.payload_offset(), shape.stride())
}

/// Where the slot array starts for `layout`: the first multiple of the
/// alignment at or past the header.
pub fn raw_vec_data_offset(layout: &ElementLayout) -> usize {
    size_of::<VecHeader>().next_multiple_of(layout.alignment)
}

/// Where the writer table starts for `capacity` slots of `layout`: right
/// past the slots.
pub fn raw_vec_table_offset(capacity: usize, layout: &ElementLayout) -> usize {
    let (_, stride) = raw_vec_geometry(layout);
    raw_vec_data_offset(layout) + capacity * stride
}

/// Bytes the file holds for `capacity` slots of `layout` and a writer table
/// of `writers` entries.
pub fn raw_vec_file_size(capacity: usize, writers: usize, layout: &ElementLayout) -> usize {
    raw_vec_table_offset(capacity, layout) + writers * size_of::<WriterEntry>()
}

pub struct RawVec {
    _file: File,
    mmap: Mapping,
    capacity: usize,
    /// Entries in the writer table, as the handle asked for them.
    writer_slots: usize,
    layout: ElementLayout,
    shape: Shape,
    data_offset: usize,
    /// The threads that wrote through this handle, registered with the vec.
    writers: Writers,
}

impl Drop for RawVec {
    fn drop(&mut self) {
        if self.mmap.is_writable() {
            self.writers.leave(self.header().top_word());
        }
    }
}

unsafe impl Send for RawVec {}
unsafe impl Sync for RawVec {}

impl std::fmt::Debug for RawVec {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RawVec")
            .field("capacity", &self.capacity)
            .field("layout", &self.layout)
            .field("stride", &self.shape.stride())
            .field("writable", &self.mmap.is_writable())
            .finish()
    }
}

impl RawVec {
    /// A layout the region can hold: a non-empty element that fits the
    /// header's 32-bit size field and an alignment that is a power of two.
    /// Anything else is an `InvalidArgument`: no region carries it.
    fn check_layout(layout: &ElementLayout) -> Result<(), VecError> {
        if layout.slot_size == 0
            || layout.slot_size > u32::MAX as usize
            || layout.alignment == 0
            || !layout.alignment.is_power_of_two()
            || layout.alignment > u32::MAX as usize
        {
            return Err(VecError::InvalidArgument(
                "an element must be 1 byte to 4 GiB and align to a power of two",
            ));
        }
        Ok(())
    }

    /// Whether a vec of `capacity` slots of `layout` and a writer table of
    /// `writers` entries can be made here: a layout
    /// [`check_layout`](Self::check_layout) accepts, a capacity of at least 1
    /// that the top word's length can count, a table the header's 32-bit
    /// field can size, and a processor that swaps 16 bytes, as the top word,
    /// a word slot and a table entry take.
    fn admit(capacity: usize, writers: usize, layout: &ElementLayout) -> Result<(), VecError> {
        Self::check_layout(layout)?;
        if capacity == 0 {
            return Err(VecError::InvalidArgument("a vector needs a capacity of at least 1"));
        }
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

    /// The handle over a mapping of a vec of `capacity` slots of `layout`
    /// and a writer table of `writers` entries, its header not yet checked.
    fn assemble(file: File, mmap: Mapping, capacity: usize, writers: usize, layout: ElementLayout) -> Self {
        let table = mmap.as_ptr().wrapping_add(raw_vec_table_offset(capacity, &layout));
        Self {
            _file: file,
            mmap,
            capacity,
            writer_slots: writers,
            layout,
            shape: slot_shape(&layout),
            data_offset: raw_vec_data_offset(&layout),
            // SAFETY: the table follows the slots in the mapping the handle
            // holds, 16-byte aligned since the slot array starts at 64 or
            // more and its stride is whole cache lines or the alignment,
            // `writers` entries once the header is checked, and the writers
            // reach it only on a write, which the check precedes; `admit`
            // checked the processor's 16-byte swap.
            writers: unsafe { Writers::new(table, writers) },
        }
    }

    /// Obtain the vec at `path`, initializing an empty one if the path
    /// does not yet exist and attaching to it if it does. Attaching leaves
    /// live elements and `len` in place; a region built with a different
    /// capacity, writer table or layout is a `LayoutMismatch`. `writers` is
    /// the size of the vec's writer table, as for
    /// [`SharedVec::create`](crate::shared_vec::SharedVec::create).
    pub fn create(path: impl AsRef<Path>, capacity: usize, writers: usize, layout: ElementLayout) -> Result<Self, VecError> {
        Self::admit(capacity, writers, &layout)?;
        let path = path.as_ref();
        let (file, mmap) = crate::mmf_attach::create_or_attach(
            path,
            raw_vec_file_size(capacity, writers, &layout),
            |ptr| unsafe { Self::init_region(ptr, capacity, writers, &layout) },
            |ptr| unsafe { u64::from((*(ptr as *const VecHeader)).magic) },
            u64::from(VEC_MAGIC),
        )
        .map_err(|e| match crate::mmf_attach::attach_error(e, VecError::LayoutMismatch) {
            VecError::LayoutMismatch(short) if short.field == "length" => match crate::region_file::open_read_only(path) {
                Ok(file) => match Self::check_header_of(&file, capacity, writers, &layout) {
                    Ok(()) => VecError::LayoutMismatch(short),
                    Err(named) => named,
                },
                Err(e) => VecError::IoError(e),
            },
            other => other,
        })?;
        let this = Self::assemble(file, Mapping::Writable(mmap), capacity, writers, layout);
        this.validate()?;
        this.slots().scan_writers();
        Ok(this)
    }

    /// Truncate the vec at `path` and initialize an empty one, discarding
    /// every element live peers share.
    pub fn reset(path: impl AsRef<Path>, capacity: usize, writers: usize, layout: ElementLayout) -> Result<Self, VecError> {
        Self::admit(capacity, writers, &layout)?;
        let (file, mmap) =
            crate::mmf_attach::reset(path.as_ref(), raw_vec_file_size(capacity, writers, &layout), |ptr| unsafe {
                Self::init_region(ptr, capacity, writers, &layout)
            })?;
        Ok(Self::assemble(file, Mapping::Writable(mmap), capacity, writers, layout))
    }

    /// Attach to the vec at `path`; the file must exist.
    pub fn open(path: impl AsRef<Path>, capacity: usize, writers: usize, layout: ElementLayout) -> Result<Self, VecError> {
        Self::admit(capacity, writers, &layout)?;
        let file = crate::region_file::open_existing(path.as_ref())?;
        let total = Self::long_enough(&file, capacity, writers, &layout)?;
        let mmap = unsafe { MmapOptions::new().len(total).map_mut(&file)? };
        let this = Self::assemble(file, Mapping::Writable(mmap), capacity, writers, layout);
        this.validate()?;
        this.slots().scan_writers();
        Ok(this)
    }

    /// Attach to the vec at `path` with read access alone; writes return
    /// [`VecError::ReadOnly`].
    pub fn open_read_only(path: impl AsRef<Path>, capacity: usize, writers: usize, layout: ElementLayout) -> Result<Self, VecError> {
        Self::admit(capacity, writers, &layout)?;
        if matches!(slot_shape(&layout), Shape::Word) && !crate::atomic_pair::plain_sixteen() {
            return Err(VecError::IoError(std::io::Error::new(
                std::io::ErrorKind::Unsupported,
                "this processor reads 16 bytes whole only by a swap, which a read-only mapping refuses",
            )));
        }
        let file = crate::region_file::open_read_only(path.as_ref())?;
        let total = Self::long_enough(&file, capacity, writers, &layout)?;
        let mmap = unsafe { MmapOptions::new().len(total).map(&file)? };
        let this = Self::assemble(file, Mapping::ReadOnly(mmap), capacity, writers, layout);
        this.validate()?;
        Ok(this)
    }

    /// The bytes a vec of `capacity` slots of `layout` and a writer table of
    /// `writers` entries takes, once `file` is found to hold them; a shorter
    /// file is refused naming the header field that differs from what was
    /// asked, when one does, else its length.
    fn long_enough(file: &File, capacity: usize, writers: usize, layout: &ElementLayout) -> Result<usize, VecError> {
        let total = raw_vec_file_size(capacity, writers, layout);
        let len = file.metadata()?.len();
        if len < total as u64 {
            Self::check_header_of(file, capacity, writers, layout)?;
            return Err(VecError::LayoutMismatch(crate::layout::Mismatch::new("length", len, total as u64)));
        }
        Ok(total)
    }

    /// Check the header at the start of `file` as
    /// [`check_header`](Self::check_header) does, when the file holds one.
    fn check_header_of(file: &File, capacity: usize, writers: usize, layout: &ElementLayout) -> Result<(), VecError> {
        if file.metadata()?.len() < size_of::<VecHeader>() as u64 {
            return Ok(());
        }
        // SAFETY: a read-only mapping of the header alone, which the file
        // holds; its fields other than the top word are written once, before
        // the magic.
        let head = unsafe { MmapOptions::new().len(size_of::<VecHeader>()).map(file)? };
        // SAFETY: the mapping holds a whole header, 64-byte aligned.
        Self::check_header(unsafe { &*(head.as_ptr() as *const VecHeader) }, capacity, writers, layout)
    }

    /// Lay out an empty vec: sizes and geometry first, magic last, because
    /// attachers spin on it. The zeroed region is already the empty writer
    /// table and the empty slot array (every slot holding no element) with a
    /// top word of length 0.
    ///
    /// # Safety
    /// `ptr` addresses at least `raw_vec_file_size(capacity, writers,
    /// layout)` writable zeroed bytes.
    unsafe fn init_region(ptr: *mut u8, capacity: usize, writers: usize, layout: &ElementLayout) {
        let shape = slot_shape(layout);
        let hdr = ptr as *mut VecHeader;
        unsafe {
            (*hdr).slot_payload_size = shape.payload_size() as u32;
            (*hdr).capacity = capacity as u64;
            (*hdr).slot_stride = shape.stride() as u32;
            (*hdr).payload_offset = shape.payload_offset() as u32;
            (*hdr).element_size = layout.slot_size as u32;
            (*hdr).alignment = layout.alignment as u32;
            (*hdr).layout_tag = layout.tag;
            (*hdr).writer_slots = writers as u32;
            std::ptr::write_volatile(&raw mut (*hdr).magic, VEC_MAGIC);
        }
    }

    /// Whether the header on disk is the one this mapping expects, as
    /// [`check_header`](Self::check_header) says.
    fn validate(&self) -> Result<(), VecError> {
        Self::check_header(self.header(), self.capacity, self.writer_slots, &self.layout)
    }

    /// Whether `hdr` is the header of a vec of `capacity` slots of `layout`
    /// and a writer table of `writers` entries: the capacity, the table, the
    /// slot geometry, the element size and the tag must all match. The
    /// alignment field is informational; the geometry it produced is what
    /// is checked, so a typed region whose element aligns wider than its
    /// payload offset opens under the alignment that offset honors.
    fn check_header(hdr: &VecHeader, capacity: usize, writers: usize, layout: &ElementLayout) -> Result<(), VecError> {
        let shape = slot_shape(layout);
        crate::layout::Mismatch::first(&[
            ("magic", u64::from(hdr.magic), u64::from(VEC_MAGIC)),
            ("capacity", hdr.capacity, capacity as u64),
            ("writer slots", u64::from(hdr.writer_slots), writers as u64),
            ("slot stride", u64::from(hdr.slot_stride), shape.stride() as u64),
            ("payload offset", u64::from(hdr.payload_offset), shape.payload_offset() as u64),
            ("element size", u64::from(hdr.element_size), layout.slot_size as u64),
            ("layout tag", hdr.layout_tag, layout.tag),
        ])
        .map_err(VecError::LayoutMismatch)
    }

    #[inline]
    fn header(&self) -> &VecHeader {
        unsafe { &*(self.mmap.as_ptr() as *const VecHeader) }
    }

    /// Whether this mapping may be written.
    #[inline]
    pub fn is_writable(&self) -> bool {
        self.mmap.is_writable()
    }

    #[inline]
    pub fn capacity(&self) -> usize {
        self.capacity
    }

    /// Entries in the writer table, as the vec's creator sized it: the
    /// `writers` an [`open`](Self::open) must be given.
    #[inline]
    pub fn writer_slots(&self) -> usize {
        self.writer_slots
    }

    /// The layout this handle opened the region with.
    #[inline]
    pub fn layout(&self) -> ElementLayout {
        self.layout
    }

    /// Bytes from one slot to the next.
    #[inline]
    pub fn slot_stride(&self) -> usize {
        self.shape.stride()
    }

    /// Slots holding an element or a hole: the top word's length, as for
    /// [`SharedVec::len`](crate::shared_vec::SharedVec::len).
    #[inline]
    pub fn len(&self) -> usize {
        self.header().len() as usize
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// The vec's slots, under the protocol both vecs share.
    #[inline(always)]
    fn slots(&self) -> VecSlots<'_> {
        // SAFETY: the mapping holds the validated header and then `capacity`
        // slots of this shape from the data offset, 64-byte aligned for a
        // word slot, and `admit` checked the processor's 16-byte swap.
        unsafe { VecSlots::new(self.header().top_word(), self.mmap.as_ptr().add(self.data_offset), self.capacity, self.shape, &self.writers) }
    }

    /// The vec's slots, for a vec whose shape is [`Shape::Word`], with that
    /// shape a constant.
    #[inline(always)]
    fn word_slots(&self) -> VecSlots<'_> {
        debug_assert!(matches!(self.shape, Shape::Word));
        // SAFETY: as in `slots`, for a vec of word slots.
        unsafe { VecSlots::new(self.header().top_word(), self.mmap.as_ptr().add(self.data_offset), self.capacity, Shape::Word, &self.writers) }
    }

    /// A value argument of exactly the element size.
    fn sized(&self, value: &[u8]) -> Result<(), VecError> {
        if value.len() != self.layout.slot_size {
            return Err(VecError::PayloadTooLarge);
        }
        Ok(())
    }

    /// An output buffer of at least the element size.
    fn room(&self, out: &[u8]) -> Result<(), VecError> {
        if out.len() < self.layout.slot_size {
            return Err(VecError::PayloadTooLarge);
        }
        Ok(())
    }

    /// Append `value`, exactly one element, and return the index it landed
    /// at. `Full` when the vec is at capacity, read with no locked operation.
    #[inline(always)]
    pub fn push_back(&self, value: &[u8]) -> Result<usize, VecError> {
        if !self.mmap.is_writable() {
            return Err(VecError::ReadOnly);
        }
        self.sized(value)?;
        if self.header().len() >= self.capacity as u64 {
            return Err(VecError::Full);
        }
        self.push_landed(value).ok_or(VecError::Full)
    }

    /// The push itself, past the refusal above: the index `value` landed
    /// at, or None when the vec filled meanwhile.
    #[inline(always)]
    fn push_landed(&self, value: &[u8]) -> Option<usize> {
        match self.shape {
            Shape::Word => {
                let slots = self.word_slots();
                slots.enter();
                // SAFETY: `value` holds exactly one element, at most 8 bytes in
                // a vec of word slots.
                slots.push_word(unsafe { word_from(value.as_ptr(), value.len()) })
            }
            // SAFETY: `value` holds exactly one element, and the vec is writable.
            Shape::Pair(_) => unsafe { self.slots().push(value.as_ptr(), value.len()) },
        }
    }

    /// Remove the last element into `out`, at least one element long:
    /// `Ok(true)` with it copied, `Ok(false)` when the vec is empty.
    #[inline(always)]
    pub fn pop_back(&self, out: &mut [u8]) -> Result<bool, VecError> {
        if !self.mmap.is_writable() {
            return Err(VecError::ReadOnly);
        }
        self.room(out)?;
        Ok(match self.shape {
            Shape::Word => {
                let slots = self.word_slots();
                slots.enter();
                match slots.pop_word() {
                    Some(word) => {
                        // SAFETY: `out` holds at least one element, at most 8
                        // bytes in a vec of word slots.
                        unsafe { word_into(word, out.as_mut_ptr(), self.layout.slot_size) };
                        true
                    }
                    None => false,
                }
            }
            // SAFETY: `out` holds at least one element, and the vec is writable.
            Shape::Pair(_) => unsafe { self.slots().pop(out.as_mut_ptr(), self.layout.slot_size) },
        })
    }

    /// Copy the element at `i` into `out`, at least one element long:
    /// `Ok(true)` with it copied, `Ok(false)` when `i` is at or past the
    /// length or names a hole.
    pub fn get(&self, i: usize, out: &mut [u8]) -> Result<bool, VecError> {
        self.room(out)?;
        // SAFETY: `out` holds at least one element.
        Ok(unsafe { self.slots().get(i, out.as_mut_ptr(), self.layout.slot_size) })
    }

    /// Copy each element in `start..end`, clamped to the length as of entry,
    /// and hand it to `f` with its index, in index order, holes left out, as
    /// [`SharedVec::for_each_range`](crate::shared_vec::SharedVec::for_each_range)
    /// does. Each element is copied as [`get`](Self::get) copies it, so `f`
    /// never sees a torn value.
    pub fn for_each_range<F: FnMut(usize, &[u8])>(&self, start: usize, end: usize, mut f: F) {
        let end = end.min(self.len());
        let mut element = vec![0u8; self.layout.slot_size];
        let slots = self.slots();
        for i in start..end {
            // SAFETY: `element` holds one element.
            if unsafe { slots.get(i, element.as_mut_ptr(), self.layout.slot_size) } {
                f(i, &element);
            }
        }
    }

    /// Overwrite the element at `i`, which is below the length and holds an
    /// element, with `value`, exactly one element.
    pub fn set(&self, i: usize, value: &[u8]) -> Result<(), VecError> {
        if !self.mmap.is_writable() {
            return Err(VecError::ReadOnly);
        }
        self.sized(value)?;
        let slots = self.slots();
        if matches!(self.shape, Shape::Word) {
            slots.enter();
        }
        // SAFETY: `value` holds exactly one element, and the vec is writable.
        if unsafe { slots.set(i, value.as_ptr(), value.len()) } { Ok(()) } else { Err(VecError::OutOfBounds) }
    }

    /// Empty the vec: the length to zero by one compare-and-swap of the top
    /// word. Slot bytes stay in place, unreachable through the bounded index.
    pub fn clear(&self) -> Result<(), VecError> {
        if !self.mmap.is_writable() {
            return Err(VecError::ReadOnly);
        }
        self.slots().clear();
        Ok(())
    }

    pub fn flush(&self) -> Result<(), VecError> {
        self.mmap.flush()?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shared_vec::SharedVec;

    /// A file for one test, removed when the test ends; declared before the
    /// primitive that maps it so it drops after it.
    fn tmp(name: &str) -> crate::test_paths::TmpFile {
        crate::test_paths::TmpFile::new(format!("subetha-raw-vec-{name}-{}.bin", std::process::id()))
    }

    const BYTES_24: ElementLayout = ElementLayout { slot_size: 24, alignment: 8, tag: 7 };

    /// The writer table the tests' vecs are made with.
    const WRITERS: usize = 4;

    /// A push into a full raw vec is refused at the top word's length, which
    /// it reads and leaves as it stands, and a pop makes room for the next.
    #[test]
    fn a_full_raw_vec_refuses_at_its_length_and_a_pop_makes_room() {
        let path = tmp("full");
        for layout in [BYTES_24, ElementLayout { slot_size: 4, alignment: 4, tag: 7 }] {
            let v = RawVec::reset(&path, 2, WRITERS, layout).unwrap();
            let one = vec![1u8; layout.slot_size];
            v.push_back(&one).unwrap();
            v.push_back(&vec![2u8; layout.slot_size]).unwrap();
            let top = || v.header().top.load(std::sync::atomic::Ordering::Acquire);
            let before = top();
            assert!(matches!(v.push_back(&one), Err(VecError::Full)));
            assert_eq!(top(), before, "a refused push moves no word");
            let mut out = vec![0u8; layout.slot_size];
            assert!(v.pop_back(&mut out).unwrap());
            assert_eq!(out, vec![2u8; layout.slot_size]);
            assert_eq!(v.push_back(&one).unwrap(), 1);
            assert!(matches!(v.push_back(&one), Err(VecError::Full)));
            v.clear().unwrap();
            assert_eq!(v.push_back(&one).unwrap(), 0, "a cleared vec takes a push at 0");
        }
    }

    /// A range read hands over each element of the range with its index, in
    /// index order, leaving out a hole and stopping at the length.
    #[test]
    fn a_range_read_leaves_out_a_hole_and_stops_at_the_length() {
        let path = tmp("range");
        for layout in [BYTES_24, ElementLayout { slot_size: 4, alignment: 4, tag: 7 }] {
            let v = RawVec::reset(&path, 8, WRITERS, layout).unwrap();
            for i in 1..=4u8 {
                v.push_back(&vec![i; layout.slot_size]).unwrap();
            }
            v.slots().make_hole(1);
            let mut seen = Vec::new();
            v.for_each_range(0, 8, |i, e| seen.push((i, e.to_vec())));
            let want = |i: usize, b: u8| (i, vec![b; layout.slot_size]);
            assert_eq!(seen, [want(0, 1), want(2, 3), want(3, 4)], "{layout:?}");
            seen.clear();
            v.for_each_range(1, 3, |i, e| seen.push((i, e.to_vec())));
            assert_eq!(seen, [want(2, 3)], "{layout:?}");
        }
    }

    /// A typed vec of a word-sized element and a raw one of the same size
    /// open each other's region: both lay it out in word slots.
    #[test]
    fn a_typed_and_a_raw_vec_of_a_word_sized_element_share_a_region() {
        let path = tmp("interop-word");
        let typed: SharedVec<u32> = SharedVec::create(&path, 8, WRITERS).unwrap();
        typed.push_back(0xA1B2_C3D4).unwrap();
        let raw = RawVec::open(&path, 8, WRITERS, ElementLayout { slot_size: 4, alignment: 4, tag: 0 }).unwrap();
        assert_eq!(raw.slot_stride(), crate::vec_core::WORD_STRIDE);
        let mut out = [0u8; 4];
        assert!(raw.get(0, &mut out).unwrap());
        assert_eq!(out, 0xA1B2_C3D4u32.to_ne_bytes());
        assert_eq!(raw.push_back(&7u32.to_ne_bytes()).unwrap(), 1);
        assert_eq!(typed.pop_back(), Some(7));
        assert_eq!(typed.len(), 1);
    }

    fn element(i: u8) -> [u8; 24] {
        let mut e = [i; 24];
        e[0] = i.wrapping_mul(3);
        e
    }

    #[test]
    fn push_get_set_pop_and_clear_round_trip() {
        let path = tmp("round-trip");
        let vec = RawVec::create(&path, 4, WRITERS, BYTES_24).unwrap();
        assert_eq!(vec.layout(), BYTES_24);
        assert!(vec.is_empty());
        let mut out = [0u8; 24];
        assert!(!vec.get(0, &mut out).unwrap());
        for i in 0..4u8 {
            assert_eq!(vec.push_back(&element(i)).unwrap(), i as usize);
        }
        assert!(matches!(vec.push_back(&element(9)).unwrap_err(), VecError::Full));
        assert_eq!(vec.len(), 4);
        assert!(vec.get(2, &mut out).unwrap());
        assert_eq!(out, element(2));
        vec.set(2, &element(42)).unwrap();
        assert!(vec.get(2, &mut out).unwrap());
        assert_eq!(out, element(42));
        assert!(matches!(vec.set(4, &element(1)).unwrap_err(), VecError::OutOfBounds));
        assert!(matches!(vec.push_back(&[1u8; 23]).unwrap_err(), VecError::PayloadTooLarge));
        assert!(matches!(vec.get(0, &mut [0u8; 23]).unwrap_err(), VecError::PayloadTooLarge));
        assert!(vec.pop_back(&mut out).unwrap());
        assert_eq!(out, element(3));
        assert_eq!(vec.len(), 3);
        vec.clear().unwrap();
        assert!(vec.is_empty());
        assert!(!vec.pop_back(&mut out).unwrap());
        assert_eq!(vec.push_back(&element(5)).unwrap(), 0);
    }

    #[test]
    fn a_typed_vec_and_a_raw_one_share_a_region() {
        let path = tmp("interop");
        let typed: SharedVec<[u8; 16]> = SharedVec::create(&path, 8, WRITERS).unwrap();
        typed.push_back(*b"sixteen bytes!!!").unwrap();
        let raw = RawVec::open(&path, 8, WRITERS, ElementLayout { slot_size: 16, alignment: 1, tag: 0 }).unwrap();
        assert_eq!(raw.len(), 1);
        let mut out = [0u8; 16];
        assert!(raw.get(0, &mut out).unwrap());
        assert_eq!(&out, b"sixteen bytes!!!");
        assert_eq!(raw.push_back(b"raw side wrote!!").unwrap(), 1);
        assert_eq!(typed.get(1), Some(*b"raw side wrote!!"));
        assert_eq!(typed.len(), 2);
        let wrong_tag = RawVec::open(&path, 8, WRITERS, ElementLayout { slot_size: 16, alignment: 1, tag: 1 });
        assert!(matches!(
            wrong_tag.unwrap_err(),
            VecError::LayoutMismatch(crate::layout::Mismatch { field: "layout tag", found: 0, wanted: 1 })
        ));
        let wrong_size = RawVec::open(&path, 8, WRITERS, ElementLayout { slot_size: 8, alignment: 1, tag: 0 });
        assert!(matches!(wrong_size.unwrap_err(), VecError::LayoutMismatch(_)));
        let wrong_capacity = RawVec::open(&path, 4, WRITERS, ElementLayout { slot_size: 16, alignment: 1, tag: 0 });
        assert!(matches!(wrong_capacity.unwrap_err(), VecError::LayoutMismatch(_)));
        let bytes_16 = ElementLayout { slot_size: 16, alignment: 1, tag: 0 };
        for writers in [WRITERS - 1, WRITERS + 8] {
            assert!(matches!(
                RawVec::open(&path, 8, writers, bytes_16).unwrap_err(),
                VecError::LayoutMismatch(m) if m == crate::layout::Mismatch::new("writer slots", WRITERS as u64, writers as u64)
            ));
        }
    }

    #[test]
    fn a_wide_alignment_moves_the_element_and_widens_the_slot() {
        assert_eq!(raw_vec_geometry(&BYTES_24), (16, 128));
        assert_eq!(raw_vec_data_offset(&BYTES_24), 64);
        let aligned_32 = ElementLayout { slot_size: 24, alignment: 32, tag: 0 };
        assert_eq!(raw_vec_geometry(&aligned_32), (32, 128));
        assert_eq!(raw_vec_data_offset(&aligned_32), 64);
        let aligned_128 = ElementLayout { slot_size: 24, alignment: 128, tag: 0 };
        assert_eq!(raw_vec_geometry(&aligned_128), (128, 384));
        assert_eq!(raw_vec_data_offset(&aligned_128), 128);
        assert_eq!(raw_vec_table_offset(3, &aligned_128), 128 + 3 * 384, "the table follows the slots");
        assert_eq!(raw_vec_file_size(3, 5, &aligned_128), 128 + 3 * 384 + 5 * 16);
        let big = ElementLayout { slot_size: 100, alignment: 8, tag: 0 };
        assert_eq!(raw_vec_geometry(&big), (16, 256));

        let path = tmp("aligned");
        let vec = RawVec::create(&path, 3, 5, aligned_128).unwrap();
        for i in 0..3u8 {
            vec.push_back(&element(i)).unwrap();
        }
        let mut out = [0u8; 24];
        assert!(vec.get(2, &mut out).unwrap());
        assert_eq!(out, element(2));
        let again = RawVec::open(&path, 3, 5, aligned_128).unwrap();
        assert_eq!(again.len(), 3);
        let narrower = RawVec::open(&path, 3, 5, ElementLayout { slot_size: 24, alignment: 8, tag: 0 });
        assert!(matches!(narrower.unwrap_err(), VecError::LayoutMismatch(_)));
    }

    #[test]
    fn a_read_only_mapping_reads_and_refuses_writes() {
        let path = tmp("read-only");
        let vec = RawVec::create(&path, 4, WRITERS, BYTES_24).unwrap();
        vec.push_back(&element(1)).unwrap();
        let reader = RawVec::open_read_only(&path, 4, WRITERS, BYTES_24).unwrap();
        assert!(!reader.is_writable());
        let mut out = [0u8; 24];
        assert!(reader.get(0, &mut out).unwrap());
        assert_eq!(out, element(1));
        assert!(matches!(reader.push_back(&element(2)).unwrap_err(), VecError::ReadOnly));
        assert!(matches!(reader.pop_back(&mut out).unwrap_err(), VecError::ReadOnly));
        assert!(matches!(reader.set(0, &element(2)).unwrap_err(), VecError::ReadOnly));
        assert!(matches!(reader.clear().unwrap_err(), VecError::ReadOnly));
    }

    #[test]
    fn an_invalid_layout_is_refused_before_any_file_is_touched() {
        let path = tmp("invalid");
        let empty = RawVec::create(&path, 4, WRITERS, ElementLayout { slot_size: 0, alignment: 8, tag: 0 });
        assert!(matches!(empty.unwrap_err(), VecError::InvalidArgument(_)));
        let misaligned = RawVec::create(&path, 4, WRITERS, ElementLayout { slot_size: 8, alignment: 3, tag: 0 });
        assert!(matches!(misaligned.unwrap_err(), VecError::InvalidArgument(_)));
        let no_room = RawVec::create(&path, 0, WRITERS, BYTES_24);
        assert!(matches!(no_room.unwrap_err(), VecError::InvalidArgument(_)));
        let wide_table = RawVec::create(&path, 4, u32::MAX as usize + 1, BYTES_24);
        assert!(matches!(wide_table.unwrap_err(), VecError::InvalidArgument(_)));
        assert!(!path.exists());
    }
}
