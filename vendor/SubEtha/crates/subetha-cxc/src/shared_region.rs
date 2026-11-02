//! `SharedRegion<T>` - cross-process typed arena with position-
//! independent `OffsetPtr<T>` references.
//!
//! The foundational building block for cross-process pointer-bearing
//! data structures (BTree nodes, trie nodes, linked lists, any
//! pointer-graph). Pointers are `OffsetPtr<T> { index: u32 }` which
//! resolve via `mmap_base + index * size_of::<T>()` in any process.
//!
//! # Layout
//!
//! ```text
//! +----------------------------+
//! | RegionHeader (64B aligned) |
//! |   magic, capacity          |
//! |   bump_next: AtomicU64     |
//! |   free_head: AtomicU64     |  (counter << 32 | index)
//! +----------------------------+
//! | next[capacity]             |  (free-list links, when slot free)
//! +----------------------------+
//! | slots[capacity]            |  (T payloads, size_of<T> each)
//! +----------------------------+
//! ```
//!
//! Free-list links live in a separate `next[capacity]` array (not
//! union'd into T storage) because T need not be 4-byte aligned. The
//! cost is `4 * capacity` extra bytes; the gain is layout simplicity
//! and zero overhead on the T storage.
//!
//! # Concurrent allocation protocol
//!
//! Allocate:
//! 1. Try to pop from `free_head` (lock-free Treiber stack):
//!    - Load packed `(counter, index)`.
//!    - If `index == NIL_INDEX`, free list is empty; fall through.
//!    - If `index == REGION_EXHAUSTED`, the list is empty and every slot
//!      has been bumped: return Full, with no locked operation.
//!    - Read `next[index]`; CAS `free_head` to `(counter+1, next)`.
//!    - On success, return `OffsetPtr { index }`.
//! 2. Bump alloc: take an index below the capacity from `bump_next`.
//!    - If the cursor refuses, mark the list and return Full.
//!    - Write `value` into `slots[index]`; return `OffsetPtr { index }`.
//!
//! Free:
//! 1. Read current `(counter, head)` from `free_head`.
//! 2. Write `head` into `next[ptr.index]`.
//! 3. CAS `free_head` to `(counter+1, ptr.index)`.
//! 4. On CAS lose, retry from step 1.
//!
//! # ABA safety
//!
//! The 32-bit counter prevents ABA: every push bumps the counter, so
//! the packed word is different even when an index repeats. 32 bits
//! of counter span 4B operations, which is far beyond any realistic
//! concrete race window.
//!
//! # No drop semantics
//!
//! T: Copy + Sized. Allocated T values go undropped on `free`
//! (Copy types don't need drop, and we can't run drop glue on bytes
//! living in shared memory anyway). `free` returns the value as a
//! by-copy.

use std::fs::File;
use std::marker::PhantomData;
use std::mem::size_of;
use std::path::Path;
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};

use memmap2::{MmapMut, MmapOptions};

use crate::bump::Refusal;

pub const REGION_MAGIC: u32 = 0x4150_5232;
pub const NIL_INDEX: u32 = u32::MAX;

/// The free-list head's index once the list is empty and an allocate has
/// found the bump cursor spent: an allocate that finds it refuses with no
/// locked operation. No slot has this index, since a region holds fewer
/// than [`NIL_INDEX`] slots. A slot freed onto it carries it as its link,
/// so the list empties back to it.
pub(crate) const REGION_EXHAUSTED: u32 = NIL_INDEX - 1;

#[repr(C, align(64))]
pub struct RegionHeader {
    pub magic: u32,
    pub capacity: u32,
    pub slot_size: u32,
    _pad1: u32,
    pub bump_next: AtomicU64,
    pub free_head: AtomicU64,
    /// Where the slot array starts in the file.
    pub slots_offset: u32,
    /// The element's alignment, as the creator stated it.
    pub alignment: u32,
    /// A tag the creator chose for the element type; an open that states
    /// another is refused.
    pub layout_tag: u64,
    _pad2: [u8; 16],
}

const _: () = {
    assert!(size_of::<RegionHeader>() == 64);
};

pub const fn region_file_size(capacity: usize, slot_size: usize) -> usize {
    size_of::<RegionHeader>()
        + capacity * size_of::<AtomicU32>()    // next[] array
        + capacity * slot_size                 // slots[]
}

#[derive(Debug)]
pub enum RegionError {
    Full,
    InvalidPtr,
    PayloadTooLarge,
    LayoutMismatch(crate::layout::Mismatch),
    /// The call passed what the region cannot take; the text says which
    /// argument and what it must be.
    InvalidArgument(&'static str),
    /// The operating system refused to open, size or map the file; the
    /// error carries its code and message.
    IoError(std::io::Error),
}

impl std::fmt::Display for RegionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Full => write!(f, "every slot is allocated"),
            Self::InvalidPtr => write!(f, "the pointer names no allocated slot"),
            Self::PayloadTooLarge => {
                write!(f, "the value does not fit a slot, or a slice is the wrong size")
            }
            Self::LayoutMismatch(m) => write!(f, "a region of another shape: {m}"),
            Self::InvalidArgument(what) => write!(f, "{what}"),
            Self::IoError(e) => write!(f, "io: {e}"),
        }
    }
}

impl std::error::Error for RegionError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::IoError(e) => Some(e),
            Self::Full
            | Self::InvalidPtr
            | Self::PayloadTooLarge
            | Self::LayoutMismatch(_)
            | Self::InvalidArgument(_) => None,
        }
    }
}

impl From<std::io::Error> for RegionError {
    fn from(e: std::io::Error) -> Self { Self::IoError(e) }
}

/// Position-independent pointer to a slot in a SharedRegion. Stable
/// across processes because the underlying MMF is byte-identical;
/// adding `mmap_base + ptr.index * size_of::<T>()` resolves in any
/// process.
#[derive(Debug)]
#[repr(C)]
pub struct OffsetPtr<T> {
    pub index: u32,
    _phantom: PhantomData<T>,
}

impl<T> Clone for OffsetPtr<T> {
    fn clone(&self) -> Self { *self }
}
impl<T> Copy for OffsetPtr<T> {}
impl<T> PartialEq for OffsetPtr<T> {
    fn eq(&self, other: &Self) -> bool { self.index == other.index }
}
impl<T> Eq for OffsetPtr<T> {}
impl<T> std::hash::Hash for OffsetPtr<T> {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.index.hash(state);
    }
}

impl<T> OffsetPtr<T> {
    pub const NIL: Self = Self { index: NIL_INDEX, _phantom: PhantomData };

    #[inline]
    pub fn new(index: u32) -> Self {
        Self { index, _phantom: PhantomData }
    }

    #[inline]
    pub fn is_nil(self) -> bool { self.index == NIL_INDEX }
}

#[inline]
fn pack(counter: u32, index: u32) -> u64 {
    ((counter as u64) << 32) | (index as u64)
}
#[inline]
fn unpack(v: u64) -> (u32, u32) {
    ((v >> 32) as u32, v as u32)
}

/// Put [`REGION_EXHAUSTED`] in the free-list head `free` while the list is
/// empty, after the bump cursor refused an allocate. A slot freed first
/// leaves the mark to the next allocate that finds the list empty and the
/// cursor spent.
#[cold]
#[inline(never)]
pub(crate) fn mark_exhausted(free: &AtomicU64) {
    let mut head = free.load(Ordering::Acquire);
    loop {
        let (counter, idx) = unpack(head);
        if idx != NIL_INDEX {
            return;
        }
        let marked = pack(counter.wrapping_add(1), REGION_EXHAUSTED);
        match free.compare_exchange_weak(head, marked, Ordering::AcqRel, Ordering::Acquire) {
            Ok(_) => return,
            Err(now) => head = now,
        }
    }
}

pub struct SharedRegion<T: Copy + 'static> {
    _file: File,
    mmap: MmapMut,
    capacity: usize,
    next_offset: usize,
    slots_offset: usize,
    /// The bump cursor's limit, the capacity, as a take reads it.
    bump: crate::bump::Limit,
    _phantom: PhantomData<T>,
    header_sidecar: subetha_core::HandshakeHeader,
    ring_sidecar: Box<subetha_core::ObservationRing>,
}

unsafe impl<T: Copy + Send + 'static> Send for SharedRegion<T> {}
unsafe impl<T: Copy + Sync + 'static> Sync for SharedRegion<T> {}

impl<T: Copy + Send + Sync + 'static> subetha_sidecar::AdaptiveInstance for SharedRegion<T> {
    fn header(&self) -> &subetha_core::HandshakeHeader { &self.header_sidecar }
    fn ring(&self) -> &subetha_core::ObservationRing { &self.ring_sidecar }
    fn make_policy(&self) -> Box<dyn subetha_sidecar::Policy> {
        Box::new(subetha_sidecar::NoMigrationPolicy)
    }
}

impl<T: Copy + 'static> SharedRegion<T> {
    /// Obtain the region at `path`, initializing an empty one if the
    /// path does not yet exist and attaching to it if it does.
    /// Attaching leaves allocated slots and the free list in place, so
    /// outstanding [`OffsetPtr`]s stay resolvable; a region built with
    /// a different capacity or payload type is a `LayoutMismatch`.
    /// [`reset`](Self::reset) reinitializes.
    pub fn create(
        path: impl AsRef<Path>, capacity: usize,
    ) -> Result<Self, RegionError> {
        assert!(capacity >= 1);
        assert!(capacity < NIL_INDEX as usize, "capacity must be < u32::MAX");
        let (file, mut mmap) = crate::mmf_attach::create_or_attach(
            path.as_ref(),
            region_file_size(capacity, size_of::<T>()),
            |ptr| unsafe { Self::init_region(ptr, capacity) },
            |ptr| unsafe { u64::from((*(ptr as *const RegionHeader)).magic) },
            u64::from(REGION_MAGIC),
        )
        .map_err(|e| crate::mmf_attach::attach_error(e, RegionError::LayoutMismatch))?;
        crate::mmf_warm::warm_mmap(&mut mmap);
        Self::from_region(file, mmap, capacity)
    }

    /// Truncate the region at `path` and initialize an empty one,
    /// invalidating every OffsetPtr live peers hold. For a caller that
    /// knows it owns the path.
    pub fn reset(
        path: impl AsRef<Path>, capacity: usize,
    ) -> Result<Self, RegionError> {
        assert!(capacity >= 1);
        assert!(capacity < NIL_INDEX as usize, "capacity must be < u32::MAX");
        let (file, mut mmap) = crate::mmf_attach::reset(
            path.as_ref(),
            region_file_size(capacity, size_of::<T>()),
            |ptr| unsafe { Self::init_region(ptr, capacity) },
        )?;
        crate::mmf_warm::warm_mmap(&mut mmap);
        Self::from_region(file, mmap, capacity)
    }

    /// Lay out an empty region: config, the zero bump cursor and the
    /// NIL free head first, magic last, because attachers spin on it.
    /// The zeroed next[] array is the not-on-a-free-chain state.
    ///
    /// # Safety
    /// `ptr` addresses at least `region_file_size(capacity,
    /// size_of::<T>())` writable zeroed bytes.
    unsafe fn init_region(ptr: *mut u8, capacity: usize) {
        let hdr = ptr as *mut RegionHeader;
        unsafe {
            (*hdr).capacity = capacity as u32;
            (*hdr).slot_size = size_of::<T>() as u32;
            (*hdr).slots_offset = Self::slots_offset_for(capacity) as u32;
            (*hdr).alignment = std::mem::align_of::<T>() as u32;
            (*hdr).layout_tag = 0;
            std::ptr::write(&raw mut (*hdr).free_head, AtomicU64::new(pack(0, NIL_INDEX)));
            std::ptr::write_volatile(&raw mut (*hdr).magic, REGION_MAGIC);
        }
    }

    /// Where the slot array starts: right behind the free links.
    fn slots_offset_for(capacity: usize) -> usize {
        size_of::<RegionHeader>() + capacity * size_of::<AtomicU32>()
    }

    /// Wrap an initialized region, refusing one built with a different
    /// capacity or payload type. A slot start or tag at zero records
    /// nothing and constrains nothing; one that is set must be this
    /// type's, and the tag must be the typed region's own, zero.
    fn from_region(
        file: File,
        mmap: MmapMut,
        capacity: usize,
    ) -> Result<Self, RegionError> {
        let next_offset = size_of::<RegionHeader>();
        let slots_offset = Self::slots_offset_for(capacity);
        let hdr = unsafe { &*(mmap.as_ptr() as *const RegionHeader) };
        use crate::layout::Mismatch;
        if let Err(m) = Mismatch::first(&[
            ("magic", u64::from(hdr.magic), u64::from(REGION_MAGIC)),
            ("capacity", u64::from(hdr.capacity), capacity as u64),
            ("slot size", u64::from(hdr.slot_size), size_of::<T>() as u64),
            Mismatch::recorded("slot start", u64::from(hdr.slots_offset), slots_offset as u64),
            ("layout tag", hdr.layout_tag, 0),
        ]) {
            return Err(RegionError::LayoutMismatch(m));
        }
        Ok(Self {
            _file: file, mmap, capacity, next_offset, slots_offset,
            bump: crate::bump::Limit::new(capacity as u64),
            _phantom: PhantomData,
            header_sidecar: subetha_core::HandshakeHeader::new(),
            ring_sidecar: Box::new(subetha_core::ObservationRing::new()),
        })
    }

    pub fn open(
        path: impl AsRef<Path>, expected_capacity: usize,
    ) -> Result<Self, RegionError> {
        let total = region_file_size(expected_capacity, size_of::<T>());
        let file = crate::region_file::open_existing(path.as_ref())?;
        let len = file.metadata()?.len();
        if len < total as u64 {
            return Err(RegionError::LayoutMismatch(crate::layout::Mismatch::new(
                "length",
                len,
                total as u64,
            )));
        }
        let mut mmap = unsafe { MmapOptions::new().len(total).map_mut(&file)? };
        crate::mmf_warm::warm_mmap(&mut mmap);
        Self::from_region(file, mmap, expected_capacity)
    }

    #[inline]
    pub fn capacity(&self) -> usize { self.capacity }

    /// Raw pointer to the start of the MMF. Useful for primitives
    /// built on top of SharedRegion that need direct memory access
    /// (e.g., SharedLinkedList reads node next/prev/value fields directly
    /// at computed offsets instead of copying the whole node).
    #[inline]
    pub fn mmap_ptr(&self) -> *const u8 { self.mmap.as_ptr() }

    fn header(&self) -> &RegionHeader {
        unsafe { &*(self.mmap.as_ptr() as *const RegionHeader) }
    }

    fn next_link(&self, idx: usize) -> &AtomicU32 {
        let base = unsafe { self.mmap.as_ptr().add(self.next_offset) };
        unsafe { &*(base.add(idx * size_of::<AtomicU32>()) as *const AtomicU32) }
    }

    fn slot_ptr(&self, idx: usize) -> *mut T {
        let base = unsafe { self.mmap.as_ptr().add(self.slots_offset) };
        unsafe { base.add(idx * size_of::<T>()) as *mut T }
    }

    /// Number of slots currently allocated (bump high-water minus
    /// free-list length). This is a snapshot; concurrent
    /// alloc/free may race.
    pub fn len(&self) -> usize {
        let bump = self.header().bump_next.load(Ordering::Acquire) as usize;
        bump.saturating_sub(self.free_count())
    }

    pub fn is_empty(&self) -> bool { self.len() == 0 }

    /// Walk the free list (best-effort under concurrent activity)
    /// and return its length. O(N_free).
    pub fn free_count(&self) -> usize {
        let mut count = 0usize;
        let (_, mut idx) = unpack(self.header().free_head.load(Ordering::Acquire));
        let mut visited = 0;
        while idx < REGION_EXHAUSTED && (visited as usize) < self.capacity {
            count += 1;
            visited += 1;
            idx = self.next_link(idx as usize).load(Ordering::Acquire);
        }
        count
    }

    /// Allocate a slot. Tries the free list first, then bump
    /// allocates. Returns `Err(Full)` when both are exhausted: the free-list
    /// head holds the mark of an exhausted region, read with no locked
    /// operation, or the bump cursor refused.
    pub fn allocate(&self, value: T) -> Result<OffsetPtr<T>, RegionError> {
        // 1. Try Treiber-stack pop from the free list.
        loop {
            let head = self.header().free_head.load(Ordering::Acquire);
            let (counter, idx) = unpack(head);
            // Both tests compare with REGION_EXHAUSTED, so one compare's flags
            // serve them; above it is NIL_INDEX, the empty list.
            if idx < REGION_EXHAUSTED {
                let next_idx = self.next_link(idx as usize).load(Ordering::Acquire);
                let new_head = pack(counter.wrapping_add(1), next_idx);
                if self.header().free_head.compare_exchange(
                    head, new_head, Ordering::AcqRel, Ordering::Acquire,
                ).is_ok() {
                    unsafe { std::ptr::write(self.slot_ptr(idx as usize), value); }
                    self.ring_sidecar
                        .push_op(crate::sidecar_ops::region::OP_ALLOCATE, 0);
                    return Ok(OffsetPtr::new(idx));
                }
                continue; // CAS lost; retry.
            }
            if idx == REGION_EXHAUSTED {
                return self.refuse_allocate(Refusal::Recorded);
            }
            break; // free list empty; bump path
        }
        // 2. Bump allocation.
        let Some(idx) = crate::bump::take64(&self.header().bump_next, &self.bump) else {
            return self.refuse_allocate(Refusal::Counter);
        };
        unsafe { std::ptr::write(self.slot_ptr(idx as usize), value); }
        self.ring_sidecar
            .push_op(crate::sidecar_ops::region::OP_ALLOCATE, 0);
        Ok(OffsetPtr::new(idx as u32))
    }

    /// A refused allocate's return: marks the free-list head exhausted when
    /// the bump cursor refused, then records the refusal. Out of line, so an
    /// allocate holds nothing across a call.
    #[cold]
    #[inline(never)]
    fn refuse_allocate(&self, why: Refusal) -> Result<OffsetPtr<T>, RegionError> {
        if why == Refusal::Counter {
            mark_exhausted(&self.header().free_head);
        }
        self.ring_sidecar
            .push_op(crate::sidecar_ops::region::OP_ALLOCATE, 1); // full
        Err(RegionError::Full)
    }

    /// Free a slot, returning the T it held. Pushes onto the
    /// Treiber-stack free list.
    pub fn free(&self, ptr: OffsetPtr<T>) -> Result<T, RegionError> {
        if ptr.is_nil() || (ptr.index as usize) >= self.capacity {
            self.ring_sidecar
                .push_op(crate::sidecar_ops::region::OP_FREE, 1); // invalid ptr
            return Err(RegionError::InvalidPtr);
        }
        let value = unsafe { std::ptr::read(self.slot_ptr(ptr.index as usize)) };
        loop {
            let head = self.header().free_head.load(Ordering::Acquire);
            let (counter, old_top) = unpack(head);
            self.next_link(ptr.index as usize).store(old_top, Ordering::Release);
            let new_head = pack(counter.wrapping_add(1), ptr.index);
            if self.header().free_head.compare_exchange(
                head, new_head, Ordering::AcqRel, Ordering::Acquire,
            ).is_ok() {
                self.ring_sidecar
                    .push_op(crate::sidecar_ops::region::OP_FREE, 0);
                return Ok(value);
            }
            // CAS lost; retry.
        }
    }

    /// Read the value at `ptr`. Returns `Err(InvalidPtr)` for nil or
    /// out-of-bounds. Caller is responsible for ensuring the ptr
    /// refers to a still-allocated slot (free'd slots may be
    /// reallocated and contain a different value).
    pub fn get(&self, ptr: OffsetPtr<T>) -> Result<T, RegionError> {
        if ptr.is_nil() || (ptr.index as usize) >= self.capacity {
            self.ring_sidecar
                .push_op(crate::sidecar_ops::region::OP_GET, 1); // invalid ptr
            return Err(RegionError::InvalidPtr);
        }
        let v = unsafe { std::ptr::read(self.slot_ptr(ptr.index as usize)) };
        self.ring_sidecar
            .push_op(crate::sidecar_ops::region::OP_GET, 0);
        Ok(v)
    }

    /// Overwrite the value at `ptr`. As with `get`, the caller must hold
    /// a still-valid pointer.
    pub fn set(&self, ptr: OffsetPtr<T>, value: T) -> Result<(), RegionError> {
        if ptr.is_nil() || (ptr.index as usize) >= self.capacity {
            self.ring_sidecar
                .push_op(crate::sidecar_ops::region::OP_SET, 1); // invalid ptr
            return Err(RegionError::InvalidPtr);
        }
        unsafe { std::ptr::write(self.slot_ptr(ptr.index as usize), value); }
        self.ring_sidecar
            .push_op(crate::sidecar_ops::region::OP_SET, 0);
        Ok(())
    }

    /// Reset the region to empty: bump pointer back to 0 and free
    /// list back to NIL. Existing `OffsetPtr` values become stale;
    /// callers must drop them. Useful for steady-state benches that
    /// need to reset accumulated state between iterations.
    ///
    /// Not thread-safe: do not call concurrently with `allocate` /
    /// `free` from other threads. Intended for single-threaded reset
    /// (e.g. test setup, bench iter setup).
    pub fn clear(&self) {
        self.header().bump_next.store(0, Ordering::Release);
        self.header().free_head.store(pack(0, NIL_INDEX), Ordering::Release);
    }

    pub fn flush(&self) -> Result<(), RegionError> {
        self.mmap.flush()?;
        Ok(())
    }

    /// Non-blocking flush: schedules a writeback via the OS.
    /// Note: Windows is only partially async (sync to page cache,
    /// not to disk).
    pub fn flush_async(&self) -> Result<(), RegionError> {
        self.mmap.flush_async()?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::thread;

    /// A file for one test, removed when the test ends; declared before the
    /// primitive that maps it so it drops after it.
    fn tmp(name: &str) -> crate::test_paths::TmpFile {
        crate::test_paths::TmpFile::new(format!("subetha-region-{name}-{}.bin", std::process::id()))
    }

    #[test]
    fn create_initial_state_is_empty() {
        let p = tmp("init");
        let r: SharedRegion<u64> = SharedRegion::create(&p, 16).unwrap();
        assert_eq!(r.capacity(), 16);
        assert_eq!(r.len(), 0);
        assert!(r.is_empty());
        assert_eq!(r.free_count(), 0);
    }

    /// A second create attaches with allocated slots in place; reset
    /// is what strips them.
    #[test]
    fn second_create_attaches_and_keeps_allocations() {
        let p = tmp("attach");
        let r: SharedRegion<u64> = SharedRegion::create(&p, 16).unwrap();
        let ptr = r.allocate(777).unwrap();

        let r2: SharedRegion<u64> = SharedRegion::create(&p, 16).unwrap();
        assert_eq!(r2.get(ptr).unwrap(), 777, "attach lost an allocation");
        assert!(matches!(
            SharedRegion::<u64>::create(&p, 8),
            Err(RegionError::LayoutMismatch(_)),
        ));

        // Windows refuses to truncate a mapped file, so every handle goes
        // before the reset.
        drop(r);
        drop(r2);
        let fresh: SharedRegion<u64> = SharedRegion::reset(&p, 16).unwrap();
        assert!(fresh.is_empty(), "reset kept an allocation");
        drop(fresh);
    }

    #[test]
    fn allocate_and_get_round_trip() {
        let p = tmp("rt");
        let r: SharedRegion<u64> = SharedRegion::create(&p, 8).unwrap();
        let p1 = r.allocate(100).unwrap();
        let p2 = r.allocate(200).unwrap();
        let p3 = r.allocate(300).unwrap();
        assert_eq!(p1.index, 0);
        assert_eq!(p2.index, 1);
        assert_eq!(p3.index, 2);
        assert_eq!(r.get(p1).unwrap(), 100);
        assert_eq!(r.get(p2).unwrap(), 200);
        assert_eq!(r.get(p3).unwrap(), 300);
        assert_eq!(r.len(), 3);
    }

    #[test]
    fn full_region_returns_error_on_bump() {
        let p = tmp("full");
        let r: SharedRegion<u32> = SharedRegion::create(&p, 4).unwrap();
        for i in 0..4u32 { r.allocate(i).unwrap(); }
        assert!(matches!(r.allocate(99).err(), Some(RegionError::Full)));
        assert_eq!(r.len(), 4);  // rolled back
    }

    /// The allocate the bump cursor refuses puts the exhausted mark in the
    /// free-list head: a later allocate refuses without moving the cursor,
    /// the free-list walk stops at the mark, a slot freed onto the mark is
    /// handed out again, the list empties back to the mark, and clear takes
    /// the mark away.
    #[test]
    fn an_exhausted_region_refuses_from_its_free_list_head() {
        let p = tmp("exhausted");
        let r: SharedRegion<u32> = SharedRegion::create(&p, 4).unwrap();
        let ptrs: Vec<_> = (0..4u32).map(|i| r.allocate(i).unwrap()).collect();
        let free = || unpack(r.header().free_head.load(Ordering::Acquire)).1;
        let cursor = || r.header().bump_next.load(Ordering::Acquire);
        assert_eq!(free(), NIL_INDEX, "taking the last slot leaves the list empty, unmarked");
        assert_eq!(cursor(), 4, "no take passed the capacity");
        assert!(matches!(r.allocate(8), Err(RegionError::Full)));
        assert_eq!(free(), REGION_EXHAUSTED, "the refused allocate left the mark");
        assert_eq!(cursor(), 4, "the refused take was given back");
        assert!(matches!(r.allocate(9), Err(RegionError::Full)));
        assert_eq!(cursor(), 4, "a refusal from the mark moves no counter");
        assert_eq!((r.free_count(), r.len()), (0, 4), "the walk stops at the mark");
        assert_eq!(r.free(ptrs[2]).unwrap(), 2);
        assert_eq!((r.free_count(), r.len()), (1, 3));
        assert_eq!(r.allocate(7).unwrap(), ptrs[2], "a slot freed onto the mark is handed out again");
        assert_eq!(free(), REGION_EXHAUSTED, "the list empties back to the mark");
        r.clear();
        assert_eq!(free(), NIL_INDEX, "clear takes the mark away");
        assert_eq!(r.allocate(5).unwrap().index, 0);
    }

    /// Allocates racing past the capacity take every slot once between them,
    /// and the region ends marked with its cursor at the capacity.
    #[test]
    fn racing_allocates_take_every_slot_once_and_leave_the_mark() {
        const CAPACITY: usize = 4096;
        let p = tmp("race-mark");
        let r: Arc<SharedRegion<u64>> = Arc::new(SharedRegion::create(&p, CAPACITY).unwrap());
        let takers: Vec<_> = (0..8u64)
            .map(|t| {
                let r = r.clone();
                thread::spawn(move || {
                    let mut got = Vec::new();
                    loop {
                        match r.allocate(t) {
                            Ok(ptr) => got.push(ptr.index),
                            Err(RegionError::Full) => return got,
                            Err(e) => panic!("an allocate failed other than full: {e:?}"),
                        }
                    }
                })
            })
            .collect();
        let mut seen = std::collections::HashSet::new();
        for idx in takers.into_iter().flat_map(|h| h.join().unwrap()) {
            assert!((idx as usize) < CAPACITY && seen.insert(idx), "slot {idx} out of range or handed out twice");
        }
        assert_eq!(seen.len(), CAPACITY);
        assert_eq!(r.header().bump_next.load(Ordering::Acquire), CAPACITY as u64);
        assert_eq!(unpack(r.header().free_head.load(Ordering::Acquire)).1, REGION_EXHAUSTED);
    }

    #[test]
    fn free_returns_value_and_decrements_len() {
        let p = tmp("free");
        let r: SharedRegion<u64> = SharedRegion::create(&p, 8).unwrap();
        let ptr1 = r.allocate(111).unwrap();
        let ptr2 = r.allocate(222).unwrap();
        assert_eq!(r.len(), 2);
        let v = r.free(ptr1).unwrap();
        assert_eq!(v, 111);
        assert_eq!(r.len(), 1);
        assert_eq!(r.free_count(), 1);
        // ptr2 still valid.
        assert_eq!(r.get(ptr2).unwrap(), 222);
    }

    #[test]
    fn free_then_allocate_reuses_slot() {
        let p = tmp("reuse");
        let r: SharedRegion<u64> = SharedRegion::create(&p, 8).unwrap();
        let p1 = r.allocate(100).unwrap();
        let _p2 = r.allocate(200).unwrap();
        r.free(p1).unwrap();
        // Next allocate should reuse p1's slot (LIFO Treiber stack).
        let p3 = r.allocate(999).unwrap();
        assert_eq!(p3.index, p1.index, "free list should have returned slot 0");
        assert_eq!(r.get(p3).unwrap(), 999);
    }

    #[test]
    fn free_invalid_ptr_returns_error() {
        let p = tmp("invalid");
        let r: SharedRegion<u64> = SharedRegion::create(&p, 4).unwrap();
        assert!(matches!(r.free(OffsetPtr::NIL).err(), Some(RegionError::InvalidPtr)));
        let oob = OffsetPtr::<u64>::new(100);
        assert!(matches!(r.free(oob).err(), Some(RegionError::InvalidPtr)));
    }

    #[test]
    fn set_overwrites_value_in_place() {
        let p = tmp("set");
        let r: SharedRegion<u64> = SharedRegion::create(&p, 4).unwrap();
        let ptr = r.allocate(42).unwrap();
        r.set(ptr, 999).unwrap();
        assert_eq!(r.get(ptr).unwrap(), 999);
    }

    #[test]
    fn cross_handle_visibility() {
        let p = tmp("cross-handle");
        let writer: SharedRegion<u64> = SharedRegion::create(&p, 16).unwrap();
        let reader: SharedRegion<u64> = SharedRegion::open(&p, 16).unwrap();
        let ptr = writer.allocate(7777).unwrap();
        assert_eq!(reader.get(ptr).unwrap(), 7777);
        writer.set(ptr, 8888).unwrap();
        assert_eq!(reader.get(ptr).unwrap(), 8888);
    }

    #[test]
    fn concurrent_allocations_get_distinct_indices() {
        let p = tmp("concurrent");
        let r: Arc<SharedRegion<u64>> = Arc::new(SharedRegion::create(&p, 1024).unwrap());
        let n_threads = 4;
        let per_thread = 100;
        let mut handles = vec![];
        for t in 0..n_threads as u64 {
            let r = r.clone();
            handles.push(thread::spawn(move || {
                let mut ptrs = vec![];
                for i in 0..per_thread as u64 {
                    let v = t * 1000 + i;
                    let ptr = r.allocate(v).unwrap();
                    ptrs.push((v, ptr));
                }
                ptrs
            }));
        }
        let all: Vec<(u64, OffsetPtr<u64>)> = handles.into_iter()
            .flat_map(|h| h.join().unwrap()).collect();

        // All indices distinct.
        let mut indices: Vec<u32> = all.iter().map(|(_, p)| p.index).collect();
        indices.sort();
        for w in indices.windows(2) {
            assert_ne!(w[0], w[1], "two threads got the same slot index");
        }
        // Every allocation reads back its written value.
        for (expected, ptr) in &all {
            assert_eq!(r.get(*ptr).unwrap(), *expected);
        }
    }

    #[test]
    fn concurrent_free_and_realloc_no_corruption() {
        let p = tmp("free-realloc");
        let r: Arc<SharedRegion<u64>> = Arc::new(SharedRegion::create(&p, 64).unwrap());
        // Pre-populate.
        let initial: Vec<OffsetPtr<u64>> = (0..64u64)
            .map(|i| r.allocate(i * 10).unwrap()).collect();
        let r_a = r.clone();
        let _init = initial.clone();
        // Worker A: free even-indexed slots.
        let freer = thread::spawn(move || {
            for (i, ptr) in _init.iter().enumerate() {
                if i % 2 == 0 { r_a.free(*ptr).expect("a live allocation frees"); }
            }
        });
        // Worker B: allocate new slots.
        let r_b = r.clone();
        let alloc = thread::spawn(move || {
            let mut new_ptrs = vec![];
            for i in 100u64..132 {
                if let Ok(p) = r_b.allocate(i) { new_ptrs.push((i, p)); }
            }
            new_ptrs
        });
        freer.join().unwrap();
        let new_ptrs = alloc.join().unwrap();
        // Each new ptr resolves to its value.
        for (val, ptr) in new_ptrs {
            assert_eq!(r.get(ptr).unwrap(), val);
        }
    }

    #[test]
    fn offset_ptr_is_position_independent() {
        // The whole point: the same index resolves to the same value
        // in two independently-mapped views of the same file.
        let p = tmp("position-indep");
        let writer: SharedRegion<u64> = SharedRegion::create(&p, 4).unwrap();
        let ptr = writer.allocate(0xCAFE_BABE).unwrap();
        let reader: SharedRegion<u64> = SharedRegion::open(&p, 4).unwrap();
        // Verify the ptr's raw index field (u32) is the same in both.
        let same_ptr = OffsetPtr::<u64>::new(ptr.index);
        assert_eq!(reader.get(same_ptr).unwrap(), 0xCAFE_BABE);
    }

    #[test]
    fn struct_payload_round_trip() {
        #[derive(Clone, Copy, Debug, PartialEq)]
        #[repr(C)]
        struct Node { left: u32, right: u32, key: u64 }
        let p = tmp("struct");
        let r: SharedRegion<Node> = SharedRegion::create(&p, 16).unwrap();
        let n = Node { left: 1, right: 2, key: 42 };
        let ptr = r.allocate(n).unwrap();
        assert_eq!(r.get(ptr).unwrap(), n);
    }

    #[test]
    fn nil_ptr_round_trips() {
        let p: OffsetPtr<u64> = OffsetPtr::NIL;
        assert!(p.is_nil());
        assert_eq!(p.index, NIL_INDEX);
    }

    #[test]
    fn offset_ptr_equality_and_hash() {
        use std::collections::HashSet;
        let a: OffsetPtr<u64> = OffsetPtr::new(5);
        let b: OffsetPtr<u64> = OffsetPtr::new(5);
        let c: OffsetPtr<u64> = OffsetPtr::new(6);
        assert_eq!(a, b);
        assert_ne!(a, c);
        let mut s = HashSet::new();
        s.insert(a);
        assert!(s.contains(&b));
        assert!(!s.contains(&c));
    }

    #[test]
    fn disk_persistence_survives_reopen() {
        let p = tmp("disk");
        let saved_ptr_index;
        {
            let r: SharedRegion<u64> = SharedRegion::create(&p, 16).unwrap();
            let p1 = r.allocate(1111).unwrap();
            let _p2 = r.allocate(2222).unwrap();
            r.flush().unwrap();
            saved_ptr_index = p1.index;
        }
        let r2: SharedRegion<u64> = SharedRegion::open(&p, 16).unwrap();
        let restored = OffsetPtr::<u64>::new(saved_ptr_index);
        assert_eq!(r2.get(restored).unwrap(), 1111);
        // Continue allocating.
        let p3 = r2.allocate(3333).unwrap();
        assert_eq!(p3.index, 2);
        assert_eq!(r2.get(p3).unwrap(), 3333);
    }
}
