//! `SharedSlab<T>` - a fixed-capacity MMF slab of records, each slot a
//! pair of buffers with a head naming the current one, addressed by an
//! index the caller chooses.
//!
//! # If your records are written once
//!
//! Every slot carries a 16-byte prefix and two copies of its record, and
//! is rounded up to whole 64-byte lines, so a `SharedSlab<u8>` spends 64
//! bytes per byte of payload, a `SharedSlab<u32>` 16 bytes per byte, and a
//! `SharedSlab<u64>` 8. That buys the slot's second buffer and its head,
//! which are what let a reader take a record while a writer replaces it,
//! neither waiting on the other.
//!
//! A table baked at build time and never written again pays that and
//! uses none of it. [`SharedArray`](crate::shared_array::SharedArray) is
//! the flat alternative: stride exactly the element size, no per-element
//! word, read-only for every process once sealed. It cannot be written
//! while it is read, which is the whole of what it gives up.
//!
//! # Packing, when the protocol is wanted and the padding is not
//!
//! The waste falls on each slot rather than on each byte, so an element
//! that fills two 56-byte buffers pays only the second copy:
//!
//! ```text
//!   SharedSlab<u64>       64.000 bytes per value      8.00x
//!   SharedSlab<[u64; 7]>  18.286 bytes per value      2.29x
//!   SharedSlab<u8>        64.000 bytes per byte      64.00x
//!   SharedSlab<[u8; 56]>   2.286 bytes per byte       2.29x
//! ```
//!
//! A caller that batches its elements into a slot-sized array keeps the
//! protocol and loses the padding, at the cost of writing a group rather
//! than an element. Note the cliff on the other side: 56 bytes of payload
//! fills a 128-byte slot and 57 rounds each buffer to 64, so the slot is
//! `(16 + 64 + 64).next_multiple_of(64)`, 192, and the overhead jumps to
//! 3.37x.
//!
//! # Where it sits
//!
//! [`SharedVec`](crate::shared_vec::SharedVec) is append-plus-index: a
//! pusher reserves a slot, writes it, and publishes an index only once
//! every earlier reservation has committed. Its slot is two cache lines,
//! so `VEC_PAYLOAD_BYTES` caps a payload at 56 bytes.
//!
//! [`SharedRegion`](crate::shared_region::SharedRegion) carries records
//! of any size and hands out slots from a free list, but its accessors
//! are a plain read and write, so a reader racing a writer tears.
//!
//! This is the third point: records of any size, addressed by the
//! caller's own index, read through the same per-slot pair of buffers.
//! There is no
//! length, no allocator and no free list. A caller that persists ids -
//! a write-ahead log naming a slot, a snapshot restoring one - needs the
//! index to be its own, and needs a released id never to address another
//! record.
//!
//! # Slot layout
//!
//! ```text
//! | SlabHeader (64B) | Slot 0 | Slot 1 | ... |
//!
//! Slot = | head: u64 | claim: u64 | buffer 0 | buffer 1 | pad |
//! ```
//!
//! Each buffer holds the record rounded up to 8 bytes. A slot is rounded up
//! to a multiple of 64 bytes and the array is 64-byte aligned, so no slot's
//! head shares a cache line with another slot. A record spanning several
//! lines does not weaken the protocol: a write fills only the buffer the
//! head does not name, and a reader checks the head after its copy, so a
//! copy a write overtook is retried whatever its length. What a larger
//! payload costs is retry work, not correctness: a reader retries only when
//! a write landed during its copy.
//!
//! # Reading a slot nothing has written
//!
//! A fresh region is zeroed, so an untouched slot reads as the zero bit
//! pattern of `T` rather than reporting itself as empty. There is no
//! occupancy bit: a caller that has to tell absent from written encodes
//! that in `T`, which a record whose zero value already means absent
//! gets for free.
//!
//! # Concurrency
//!
//! Any number of writers and readers, no coordination between slots.
//! Writers of one slot take its claim in turn, each landing whole, and a
//! writer that dies holding it is taken over; a reader never waits on a
//! writer.

use std::fs::File;
use std::marker::PhantomData;
use std::mem::size_of;
use std::path::Path;
use memmap2::{Mmap, MmapMut, MmapOptions};

use crate::slot_pair::{self, SlotGeometry};

pub const SLAB_MAGIC: u32 = 0x4150_5332;

/// Bytes a slot spends on its head and claim words before its first
/// buffer.
pub const SLAB_SLOT_PREFIX: usize = slot_pair::SLOT_PREFIX;

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

    pub(crate) fn flush(&self) -> Result<(), std::io::Error> {
        match self {
            Mapping::Writable(m) => m.flush(),
            Mapping::ReadOnly(_) => Ok(()),
        }
    }
}

#[repr(C, align(64))]
pub struct SlabHeader {
    pub magic: u32,
    /// Full stride of one slot, version word included.
    pub slot_size: u32,
    pub capacity: u64,
    /// Where the record lies within its slot.
    pub payload_offset: u32,
    /// Bytes one record takes.
    pub element_size: u32,
    /// The record's alignment, as the creator stated it.
    pub alignment: u32,
    _pad0: u32,
    /// A tag the creator chose for the record type; an open that states
    /// another is refused.
    pub layout_tag: u64,
    _pad: [u8; 24],
}

const _: () = {
    assert!(size_of::<SlabHeader>() == 64);
};

/// Where a slot's parts lie for a record of `size` bytes, placed as for an
/// alignment of 8 whatever the record's own, so a typed slab and a raw one
/// of alignment at most 8 share one slot shape.
pub(crate) const fn slab_geometry(size: usize) -> SlotGeometry {
    SlotGeometry::new(size, 8)
}

/// Stride of one slot holding a `T`: the head and claim words plus two
/// copies of the record, each rounded up to 8 bytes, rounded up to a whole
/// number of cache lines.
pub const fn slab_slot_size<T>() -> usize {
    slab_geometry(size_of::<T>()).stride
}

pub const fn slab_file_size<T>(capacity: usize) -> usize {
    size_of::<SlabHeader>() + capacity * slab_slot_size::<T>()
}

#[derive(Debug)]
pub enum SlabError {
    OutOfBounds,
    LayoutMismatch(crate::layout::Mismatch),
    /// The call passed what the slab cannot take; the text says which
    /// argument and what it must be.
    InvalidArgument(&'static str),
    ReadOnly,
    /// A record argument is not the record size, or an output buffer is
    /// shorter than one.
    PayloadTooLarge,
    /// The operating system refused to open, size or map the file; the
    /// error carries its code and message.
    IoError(std::io::Error),
}

impl From<std::io::Error> for SlabError {
    fn from(e: std::io::Error) -> Self {
        SlabError::IoError(e)
    }
}

impl std::fmt::Display for SlabError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SlabError::OutOfBounds => write!(f, "slot index out of bounds"),
            SlabError::LayoutMismatch(m) => write!(f, "a slab of another shape: {m}"),
            SlabError::InvalidArgument(what) => write!(f, "{what}"),
            SlabError::ReadOnly => write!(f, "slab opened read-only"),
            SlabError::PayloadTooLarge => write!(f, "record bytes are not the record size"),
            SlabError::IoError(e) => write!(f, "slab io: {e}"),
        }
    }
}

impl std::error::Error for SlabError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            SlabError::IoError(e) => Some(e),
            SlabError::OutOfBounds
            | SlabError::LayoutMismatch(_)
            | SlabError::InvalidArgument(_)
            | SlabError::ReadOnly
            | SlabError::PayloadTooLarge => None,
        }
    }
}

pub struct SharedSlab<T: Copy + 'static> {
    _file: File,
    mmap: Mapping,
    capacity: usize,
    _phantom: PhantomData<T>,
    header_sidecar: subetha_core::HandshakeHeader,
    ring_sidecar: Box<subetha_core::ObservationRing>,
}

unsafe impl<T: Copy + Send + 'static> Send for SharedSlab<T> {}
unsafe impl<T: Copy + Sync + 'static> Sync for SharedSlab<T> {}

impl<T: Copy + 'static> SharedSlab<T> {
    /// Obtain the slab at `path`, initializing an empty one if the path
    /// does not yet exist and attaching to it if it does. Attaching
    /// leaves live records in place; a region built for a different
    /// capacity or a different record size is a `LayoutMismatch`.
    pub fn create(path: impl AsRef<Path>, capacity: usize) -> Result<Self, SlabError> {
        assert!(capacity >= 1);
        let (file, mmap) = crate::mmf_attach::create_or_attach(
            path.as_ref(),
            slab_file_size::<T>(capacity),
            |ptr| unsafe { Self::init_region(ptr, capacity, slab_slot_size::<T>()) },
            |ptr| unsafe { u64::from((*(ptr as *const SlabHeader)).magic) },
            u64::from(SLAB_MAGIC),
        )
        .map_err(|e| crate::mmf_attach::attach_error(e, SlabError::LayoutMismatch))?;
        let this = Self {
            _file: file,
            mmap: Mapping::Writable(mmap),
            capacity,
            _phantom: PhantomData,
            header_sidecar: subetha_core::HandshakeHeader::new(),
            ring_sidecar: Box::new(subetha_core::ObservationRing::new()),
        };
        this.validate(capacity)?;
        Ok(this)
    }

    /// Truncate the slab at `path` and initialize an empty one,
    /// discarding every record live peers share. For a caller that knows
    /// it owns the path.
    pub fn reset(path: impl AsRef<Path>, capacity: usize) -> Result<Self, SlabError> {
        assert!(capacity >= 1);
        let (file, mmap) = crate::mmf_attach::reset(
            path.as_ref(),
            slab_file_size::<T>(capacity),
            |ptr| unsafe { Self::init_region(ptr, capacity, slab_slot_size::<T>()) },
        )?;
        Ok(Self {
            _file: file,
            mmap: Mapping::Writable(mmap),
            capacity,
            _phantom: PhantomData,
            header_sidecar: subetha_core::HandshakeHeader::new(),
            ring_sidecar: Box::new(subetha_core::ObservationRing::new()),
        })
    }

    /// Attach to an existing slab.
    pub fn open(path: impl AsRef<Path>, expected_capacity: usize) -> Result<Self, SlabError> {
        let file = crate::region_file::open_existing(path.as_ref())?;
        let total = slab_file_size::<T>(expected_capacity);
        let len = file.metadata()?.len();
        if len < total as u64 {
            return Err(SlabError::LayoutMismatch(crate::layout::Mismatch::new(
                "length",
                len,
                total as u64,
            )));
        }
        let mmap = unsafe { MmapOptions::new().len(total).map_mut(&file)? };
        let this = Self {
            _file: file,
            mmap: Mapping::Writable(mmap),
            capacity: expected_capacity,
            _phantom: PhantomData,
            header_sidecar: subetha_core::HandshakeHeader::new(),
            ring_sidecar: Box::new(subetha_core::ObservationRing::new()),
        };
        this.validate(expected_capacity)?;
        Ok(this)
    }

    /// Open a slab this process may only read.
    ///
    /// [`open`](Self::open) needs a read+write file handle, which a
    /// consumer of a privileged producer's slab does not have. Reads
    /// behave identically, each checked against its slot's head; writes
    /// return [`SlabError::ReadOnly`].
    pub fn open_read_only(
        path: impl AsRef<Path>,
        expected_capacity: usize,
    ) -> Result<Self, SlabError> {
        let file = crate::region_file::open_read_only(path.as_ref())?;
        let total = slab_file_size::<T>(expected_capacity);
        let len = file.metadata()?.len();
        if len < total as u64 {
            return Err(SlabError::LayoutMismatch(crate::layout::Mismatch::new(
                "length",
                len,
                total as u64,
            )));
        }
        let mmap = unsafe { MmapOptions::new().len(total).map(&file)? };
        let this = Self {
            _file: file,
            mmap: Mapping::ReadOnly(mmap),
            capacity: expected_capacity,
            _phantom: PhantomData,
            header_sidecar: subetha_core::HandshakeHeader::new(),
            ring_sidecar: Box::new(subetha_core::ObservationRing::new()),
        };
        this.validate(expected_capacity)?;
        Ok(this)
    }

    /// Lay out an empty slab: sizes first, magic last, because attachers
    /// spin on it. The zeroed region is already the empty slot array,
    /// every head and claim 0.
    ///
    /// # Safety
    /// `ptr` addresses at least `slab_file_size::<T>(capacity)` writable
    /// zeroed bytes.
    unsafe fn init_region(ptr: *mut u8, capacity: usize, slot_size: usize) {
        let hdr = ptr as *mut SlabHeader;
        unsafe {
            (*hdr).slot_size = slot_size as u32;
            (*hdr).capacity = capacity as u64;
            (*hdr).payload_offset = SLAB_SLOT_PREFIX as u32;
            (*hdr).element_size = size_of::<T>() as u32;
            (*hdr).alignment = std::mem::align_of::<T>() as u32;
            (*hdr).layout_tag = 0;
            std::ptr::write_volatile(&raw mut (*hdr).magic, SLAB_MAGIC);
        }
    }

    #[inline]
    fn header(&self) -> &SlabHeader {
        unsafe { &*(self.mmap.as_ptr() as *const SlabHeader) }
    }

    /// Whether the header on disk is the one this mapping expects. The
    /// slot size is checked as well as the capacity: two callers whose
    /// `T` differs in size derive different strides from the same file
    /// and would read each other's records at the wrong offset. A payload
    /// offset or element size at zero records nothing and constrains
    /// nothing; one that is set must be this type's, and the tag must be
    /// the typed slab's own, zero.
    fn validate(&self, expected_capacity: usize) -> Result<(), SlabError> {
        let hdr = self.header();
        use crate::layout::Mismatch;
        Mismatch::first(&[
            ("magic", u64::from(hdr.magic), u64::from(SLAB_MAGIC)),
            ("capacity", hdr.capacity, expected_capacity as u64),
            ("slot size", u64::from(hdr.slot_size), slab_slot_size::<T>() as u64),
            Mismatch::recorded("payload offset", u64::from(hdr.payload_offset), SLAB_SLOT_PREFIX as u64),
            Mismatch::recorded("element size", u64::from(hdr.element_size), size_of::<T>() as u64),
            ("layout tag", hdr.layout_tag, 0),
        ])
        .map_err(SlabError::LayoutMismatch)?;
        Ok(())
    }

    #[inline]
    fn slot_ptr(&self, i: usize) -> *const u8 {
        unsafe {
            self.mmap
                .as_ptr()
                .add(size_of::<SlabHeader>())
                .add(i * slab_slot_size::<T>())
        }
    }

    /// Slots this slab addresses.
    pub fn capacity(&self) -> usize {
        self.capacity
    }

    /// Whether this mapping may write.
    pub fn is_writable(&self) -> bool {
        self.mmap.is_writable()
    }

    /// Read the record at `i`.
    ///
    /// Copies the buffer the slot's head names and checks the head again,
    /// retrying only when a write landed during the copy, so the value
    /// returned was never observed half-written and the read never waits
    /// on a writer. A slot nothing has written reads as the zero bit
    /// pattern of `T`.
    pub fn get(&self, i: usize) -> Result<T, SlabError> {
        if i >= self.capacity {
            return Err(SlabError::OutOfBounds);
        }
        // SAFETY: the slot lies in the mapping, laid out by the slab's
        // geometry for T; every value written to it is a T, as is the zero
        // a fresh slot holds.
        let (_, out) = unsafe { slot_pair::load::<T>(self.slot_ptr(i), &slab_geometry(size_of::<T>())) };
        self.ring_sidecar
            .push_op(crate::sidecar_ops::ordered::OP_GET, 0);
        Ok(out)
    }

    /// Write the record at `i`: into the slot's idle buffer, made current
    /// with one compare-and-swap of its head. Writers of one slot take its
    /// claim in turn and each lands whole.
    pub fn set(&self, i: usize, value: T) -> Result<(), SlabError> {
        if i >= self.capacity {
            return Err(SlabError::OutOfBounds);
        }
        if !self.mmap.is_writable() {
            return Err(SlabError::ReadOnly);
        }
        // SAFETY: the slot lies in this writable mapping, laid out by the
        // slab's geometry for T.
        unsafe { slot_pair::store(self.slot_ptr(i), &slab_geometry(size_of::<T>()), &value) };
        self.ring_sidecar
            .push_op(crate::sidecar_ops::ordered::OP_INSERT, 0);
        Ok(())
    }

    /// How many times slot `i` has been written, doubled: even at rest, odd
    /// while a writer holds the slot.
    pub fn slot_version(&self, i: usize) -> Result<u32, SlabError> {
        if i >= self.capacity {
            return Err(SlabError::OutOfBounds);
        }
        // SAFETY: the slot lies in the mapping.
        Ok(unsafe { slot_pair::version(self.slot_ptr(i)) } as u32)
    }

    pub fn flush(&self) -> Result<(), SlabError> {
        self.mmap.flush()?;
        Ok(())
    }
}

impl<T: Copy + Send + Sync + 'static> subetha_sidecar::AdaptiveInstance for SharedSlab<T> {
    fn header(&self) -> &subetha_core::HandshakeHeader {
        &self.header_sidecar
    }
    fn ring(&self) -> &subetha_core::ObservationRing {
        &self.ring_sidecar
    }
    fn make_policy(&self) -> Box<dyn subetha_sidecar::Policy> {
        Box::new(subetha_sidecar::NoMigrationPolicy)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[repr(C)]
    #[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
    struct Big {
        a: [u64; 20],
        b: u32,
        c: u8,
    }

    /// A file for one test, removed when the test ends; declared before the
    /// slab that maps it so it drops after it.
    fn tmp(tag: &str) -> crate::test_paths::TmpFile {
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("the wall clock is after the epoch")
            .as_nanos();
        crate::test_paths::TmpFile::new(format!("subetha_slab_{tag}_{}_{nonce}.bin", std::process::id()))
    }

    /// The record SharedVec cannot hold is the case this exists for.
    #[test]
    fn a_record_past_the_vec_payload_cap_round_trips() {
        assert!(
            size_of::<Big>() > crate::shared_vec::VEC_PAYLOAD_BYTES,
            "the fixture must exceed the cap this primitive exists to clear"
        );
        let p = tmp("big");
        let s: SharedSlab<Big> = SharedSlab::create(&p, 64).unwrap();

        let mut v = Big::default();
        v.a[0] = 0xDEAD_BEEF;
        v.a[19] = 0xFEED_FACE;
        v.b = 7;
        v.c = 9;
        s.set(5, v).unwrap();
        assert_eq!(s.get(5).unwrap(), v);

        // Untouched slots read as the zero pattern.
        assert_eq!(s.get(6).unwrap(), Big::default());
        drop(s);
    }

    /// A slot spans six cache lines at this size, and the stride keeps
    /// every head on a line of its own.
    #[test]
    fn a_slot_is_whole_cache_lines_and_holds_the_record() {
        let stride = slab_slot_size::<Big>();
        assert_eq!(stride % 64, 0, "stride must be whole cache lines");
        assert!(stride >= SLAB_SLOT_PREFIX + 2 * size_of::<Big>());
        assert_eq!(stride, 384, "two 168-byte buffers plus a 16-byte prefix round to 384");
        assert_eq!(slab_slot_size::<u32>(), 64, "a small record still takes a line");
        assert_eq!(slab_slot_size::<[u8; 56]>(), 128, "a 56-byte record fills two lines");
    }

    /// Slot `i`'s claim word.
    fn claim_of<T: Copy>(s: &SharedSlab<T>, i: usize) -> &std::sync::atomic::AtomicU64 {
        // SAFETY: a slot's second 8 bytes are its claim word.
        unsafe { &*(s.slot_ptr(i).add(8) as *const std::sync::atomic::AtomicU64) }
    }

    /// A writer that died holding a slot's claim leaves the record
    /// readable, the version odd, and a claim the next write takes over.
    #[test]
    fn a_claim_left_by_a_dead_writer_is_taken_over_by_the_next_write() {
        let p = tmp("dead-writer");
        let s: SharedSlab<Big> = SharedSlab::create(&p, 4).unwrap();
        let v = Big { b: 1, ..Big::default() };
        s.set(2, v).unwrap();
        let mut child = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "exit_watch::tests::sleeper_never_runs_on_its_own", "--ignored", "--nocapture"])
            .env("SUBETHA_EXIT_WATCH_SLEEP", "1")
            .spawn()
            .unwrap();
        let dead = child.id();
        child.kill().unwrap();
        child.wait().unwrap();
        claim_of(&s, 2).store(crate::process_identity::holder(dead, 0), std::sync::atomic::Ordering::Release);
        assert_eq!(s.slot_version(2).unwrap(), 3, "a held claim reads odd");
        assert_eq!(s.get(2).unwrap(), v, "a read does not wait on the dead writer");
        let w = Big { b: 2, ..Big::default() };
        s.set(2, w).unwrap();
        assert_eq!(s.get(2).unwrap(), w);
        assert_eq!(s.slot_version(2).unwrap(), 4);
    }

    /// Two writers on one slot each land whole: every record a reader sees
    /// agrees with itself.
    #[test]
    fn two_writers_on_one_slot_never_tear_a_record() {
        use std::sync::atomic::{AtomicBool, Ordering as AtomOrd};
        use std::sync::Arc;

        let p = tmp("two-writers");
        let s: Arc<SharedSlab<Big>> = Arc::new(SharedSlab::create(&p, 4).unwrap());
        let stop = Arc::new(AtomicBool::new(false));
        let writers: Vec<_> = (0..2u64)
            .map(|w| {
                let s = Arc::clone(&s);
                std::thread::spawn(move || {
                    for round in 1..=5_000u64 {
                        let seed = (w << 32) | round;
                        let mut v = Big::default();
                        for (k, slot) in v.a.iter_mut().enumerate() {
                            *slot = seed.wrapping_mul(k as u64 + 1);
                        }
                        v.b = seed as u32;
                        s.set(1, v).unwrap();
                    }
                })
            })
            .collect();
        let reader = {
            let (s, stop) = (Arc::clone(&s), Arc::clone(&stop));
            std::thread::spawn(move || {
                while !stop.load(AtomOrd::Acquire) {
                    let v = s.get(1).unwrap();
                    let seed = v.a[0];
                    for (k, slot) in v.a.iter().enumerate() {
                        assert_eq!(*slot, seed.wrapping_mul(k as u64 + 1), "torn record at field {k}");
                    }
                    assert_eq!(v.b, seed as u32, "torn record across the tail");
                }
            })
        };
        for w in writers {
            w.join().unwrap();
        }
        stop.store(true, AtomOrd::Release);
        reader.join().unwrap();
        assert_eq!(s.slot_version(1).unwrap(), 20_000, "every write landed once");
    }

    /// Two mappings of one file address the same records.
    #[test]
    fn a_second_handle_sees_the_first_handles_writes() {
        let p = tmp("share");
        let a: SharedSlab<Big> = SharedSlab::create(&p, 32).unwrap();
        let b: SharedSlab<Big> = SharedSlab::open(&p, 32).unwrap();

        let v = Big { b: 0x1234, ..Big::default() };
        a.set(9, v).unwrap();
        assert_eq!(b.get(9).unwrap(), v);
        assert_eq!(b.slot_version(9).unwrap(), 2, "one write leaves an even version");
        drop(a);
        drop(b);
    }

    /// A reader racing a writer on one slot never observes a mixture of
    /// two records. Every field of the fixture is derived from the same
    /// seed, so a torn read is detectable by the record disagreeing with
    /// itself rather than by comparing against an expected value.
    #[test]
    fn a_concurrent_reader_never_sees_a_half_written_record() {
        use std::sync::atomic::{AtomicBool, Ordering as AtomOrd};
        use std::sync::Arc;

        let p = tmp("tear");
        let s: Arc<SharedSlab<Big>> = Arc::new(SharedSlab::create(&p, 8).unwrap());
        let stop = Arc::new(AtomicBool::new(false));

        let w = {
            let s = Arc::clone(&s);
            let stop = Arc::clone(&stop);
            std::thread::spawn(move || {
                let mut seed = 1u64;
                while !stop.load(AtomOrd::Acquire) {
                    let mut v = Big::default();
                    for (k, slot) in v.a.iter_mut().enumerate() {
                        *slot = seed.wrapping_mul(k as u64 + 1);
                    }
                    v.b = seed as u32;
                    v.c = (seed & 0xFF) as u8;
                    s.set(3, v).unwrap();
                    seed = seed.wrapping_add(1);
                }
            })
        };

        for _ in 0..50_000 {
            let v = s.get(3).unwrap();
            let seed = v.a[0];
            if seed == 0 {
                continue; // never written yet
            }
            for (k, slot) in v.a.iter().enumerate() {
                assert_eq!(
                    *slot,
                    seed.wrapping_mul(k as u64 + 1),
                    "record disagrees with itself: torn read at field {k}"
                );
            }
            assert_eq!(v.b, seed as u32, "torn read across the tail");
        }
        stop.store(true, AtomOrd::Release);
        w.join().unwrap();
        drop(s);
    }

    #[test]
    fn an_index_past_capacity_is_refused() {
        let p = tmp("oob");
        let s: SharedSlab<Big> = SharedSlab::create(&p, 4).unwrap();
        assert!(matches!(s.get(4), Err(SlabError::OutOfBounds)));
        assert!(matches!(s.set(4, Big::default()), Err(SlabError::OutOfBounds)));
        drop(s);
    }

    /// A record of a different size derives a different stride, so the
    /// same file read as the wrong type is refused rather than returning
    /// records sliced at the wrong offset.
    #[test]
    fn a_different_record_size_is_a_layout_mismatch() {
        let p = tmp("layout");
        let s: SharedSlab<Big> = SharedSlab::create(&p, 8).unwrap();
        drop(s);
        assert!(matches!(
            SharedSlab::<u64>::open(&p, 8).err(),
            Some(SlabError::LayoutMismatch(m)) if m == crate::layout::Mismatch::new(
                "slot size",
                slab_slot_size::<Big>() as u64,
                slab_slot_size::<u64>() as u64,
            ),
        ));
    }

    #[test]
    fn a_read_only_mapping_refuses_writes() {
        let p = tmp("ro");
        let w: SharedSlab<Big> = SharedSlab::create(&p, 8).unwrap();
        let v = Big { b: 5, ..Big::default() };
        w.set(1, v).unwrap();

        let r: SharedSlab<Big> = SharedSlab::open_read_only(&p, 8).unwrap();
        assert_eq!(r.get(1).unwrap(), v);
        assert!(!r.is_writable());
        assert!(matches!(r.set(1, Big::default()), Err(SlabError::ReadOnly)));
        drop(w);
        drop(r);
    }
}
