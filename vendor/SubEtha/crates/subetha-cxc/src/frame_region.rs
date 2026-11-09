//! `FrameRegion` - concurrent fixed-block payload region for the
//! self-describing offset path shared by every `AdaptiveRing` shape.
//!
//! Records too large to inline in a ring slot spill here: the producer
//! allocates a block, copies the payload in, and writes the block index
//! into the ring descriptor; the consumer reads the block and frees it.
//! Because the offset payloads of every shape (SPSC / MPSC / MPMC /
//! Vyukov) land in one region, the allocator must be safe for many
//! producers allocating and many consumers freeing at once, in any
//! order. That is a Treiber-stack free list with an ABA counter plus a
//! bump high-water mark - the same allocator
//! [`SharedRegion`](crate::shared_region::SharedRegion) ships, here with
//! a runtime block size instead of a const-generic `T` so the
//! `AdaptiveRing` can size its blocks to the workload.
//!
//! Reclaim order does not matter: a freed block returns to the stack
//! and is handed to the next allocation regardless of which consumer
//! freed it, so no FIFO bookkeeping is needed across consumers.

use std::fs::File;
use std::path::Path;
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};

use memmap2::{MmapMut, MmapOptions};

use crate::shared_ring::RingError;

/// Magic identifying a `FrameRegion` layout. ASCII "FRGN" + version.
pub const FRAME_REGION_MAGIC: u64 = 0x4652_474e_0000_0002;

/// Free-list sentinel: "no next block".
const NIL: u32 = u32::MAX;

/// The free-list head's index once the list is empty and an allocate has
/// found the bump cursor spent: an allocate that finds it refuses with no
/// locked operation. No block has this index, since a region holds
/// fewer than [`NIL`] blocks. A block freed onto it carries it as its link,
/// so the list empties back to it.
const EXHAUSTED: u32 = NIL - 1;

/// Smallest block: must hold the 4-byte free-list link.
pub const MIN_BLOCK_SIZE: usize = 8;

/// The free-list head word for `index` under ABA count `counter`. The index
/// is stored XORed with [`NIL`], so the empty list reads as zero, the
/// [`EXHAUSTED`] mark as one, and a fresh region's head is already empty
/// with nothing written to it.
#[inline]
fn pack(counter: u32, index: u32) -> u64 {
    ((counter as u64) << 32) | ((index ^ NIL) as u64)
}
/// The ABA count and block index a free-list head word holds.
#[inline]
fn unpack(v: u64) -> (u32, u32) {
    ((v >> 32) as u32, (v as u32) ^ NIL)
}

/// Header: metadata line, then the bump cursor and the free-list head
/// each on their own cache line so allocators and freers do not
/// false-share.
#[repr(C, align(64))]
struct FrameRegionHeader {
    magic: AtomicU64,
    block_size: u64,
    block_count: u64,
    /// Which laying-out of the region this is, drawn afresh, and nonzero,
    /// each time this crate lays one out. An offset frame's descriptor
    /// carries it, so a frame sent into a region that has since been
    /// replaced under the same name is told apart from one sent into this
    /// region. A zero is a region laid out by a maker that records none.
    instance: u64,
    _pad_meta: [u8; 64 - 32],
    /// Bump high-water mark (next never-yet-allocated block).
    bump_next: AtomicU32,
    _pad_bump: [u8; 64 - 4],
    /// Treiber-stack free-list head, ABA-tagged: see [`pack`].
    free_head: AtomicU64,
    _pad_free: [u8; 64 - 8],
}

/// Total mapped bytes for `block_count` blocks of `block_size`: the header,
/// an allocation tag per block, then the blocks.
pub const fn frame_region_file_size(block_size: usize, block_count: usize) -> usize {
    std::mem::size_of::<FrameRegionHeader>() + tags_bytes(block_count) + block_size * block_count
}

/// Bytes the allocation tags take: eight per block, rounded up to whole
/// cache lines so the blocks start on one.
const fn tags_bytes(block_count: usize) -> usize {
    (block_count * 8).next_multiple_of(64)
}

#[allow(dead_code)]
enum RegionBacking {
    Anon(MmapMut),
    File(File, MmapMut),
    Shm(crate::shm_file::ShmFile),
}

/// Concurrent fixed-block region. Multi-producer `alloc`,
/// multi-consumer `free`, any-order reclaim.
pub struct FrameRegion {
    _backing: RegionBacking,
    raw_ptr: *mut u8,
    block_size: usize,
    block_count: usize,
    blocks_base: usize,
    /// The bump cursor's limit, the block count, as a take reads it.
    bump: crate::bump::Limit,
}

unsafe impl Send for FrameRegion {}
unsafe impl Sync for FrameRegion {}

fn validate(block_size: usize, block_count: usize) -> Result<(), RingError> {
    if block_size < MIN_BLOCK_SIZE || !block_size.is_multiple_of(8) {
        return Err(RingError::InvalidArgument("the block size must be a nonzero multiple of 8"));
    }
    if block_count < 1 || block_count >= NIL as usize {
        return Err(RingError::InvalidArgument("the block count must be from 1 to 4294967294"));
    }
    Ok(())
}

/// A region instance no other laying-out draws: the standard hasher's
/// per-process random keys over this process's id and the time. Never zero.
fn fresh_instance() -> u64 {
    use std::hash::{BuildHasher, Hash, Hasher};
    let mut h = std::collections::hash_map::RandomState::new().build_hasher();
    std::process::id().hash(&mut h);
    std::time::SystemTime::now().hash(&mut h);
    h.finish().max(1)
}

/// Lay out a fresh region. The magic is published last with `Release`, so a
/// handle that finds it finds the header and cursors behind it.
unsafe fn init_region(ptr: *mut u8, block_size: usize, block_count: usize) {
    unsafe {
        std::ptr::write(ptr as *mut FrameRegionHeader, FrameRegionHeader {
            magic: AtomicU64::new(0),
            block_size: block_size as u64,
            block_count: block_count as u64,
            instance: fresh_instance(),
            _pad_meta: [0; 64 - 32],
            bump_next: AtomicU32::new(0),
            _pad_bump: [0; 64 - 4],
            free_head: AtomicU64::new(pack(0, NIL)),
            _pad_free: [0; 64 - 8],
        });
        // Every block starts free, its tag even.
        std::ptr::write_bytes(ptr.add(std::mem::size_of::<FrameRegionHeader>()), 0, tags_bytes(block_count));
        (*(ptr as *const FrameRegionHeader))
            .magic
            .store(FRAME_REGION_MAGIC, std::sync::atomic::Ordering::Release);
    }
}

impl FrameRegion {
    /// Anonymous in-process region.
    pub fn create_anon(block_size: usize, block_count: usize) -> Result<Self, RingError> {
        validate(block_size, block_count)?;
        let total = frame_region_file_size(block_size, block_count);
        let mut mmap = MmapOptions::new().len(total).map_anon()?;
        unsafe { init_region(mmap.as_mut_ptr(), block_size, block_count) };
        let raw_ptr = mmap.as_mut_ptr();
        Ok(Self::from_parts(RegionBacking::Anon(mmap), raw_ptr, block_size, block_count))
    }

    /// File-backed region; cross-process via the page cache. Obtains the
    /// region at `path`: initializes it only when the path does not yet
    /// exist, and otherwise attaches with allocated blocks and the free
    /// list in place, so a late joiner never wipes a region a producer
    /// already filled. A region built with a different geometry is a
    /// `LayoutMismatch`. [`reset`](Self::reset) reinitializes.
    pub fn create(
        path: impl AsRef<Path>, block_size: usize, block_count: usize,
    ) -> Result<Self, RingError> {
        Self::create_or_open_file(path, block_size, block_count)
    }

    /// Truncate the region at `path` and initialize an empty one,
    /// invalidating every offset live peers hold. For a caller that
    /// knows it owns the path.
    pub fn reset(
        path: impl AsRef<Path>, block_size: usize, block_count: usize,
    ) -> Result<Self, RingError> {
        validate(block_size, block_count)?;
        let total = frame_region_file_size(block_size, block_count);
        let file = crate::region_file::create_truncated(path.as_ref())?;
        file.set_len(total as u64)?;
        let mut mmap = unsafe { MmapOptions::new().len(total).map_mut(&file)? };
        unsafe { init_region(mmap.as_mut_ptr(), block_size, block_count) };
        let raw_ptr = mmap.as_mut_ptr();
        Ok(Self::from_parts(RegionBacking::File(file, mmap), raw_ptr, block_size, block_count))
    }

    /// Open an existing file-backed region. Validates the header.
    pub fn open(
        path: impl AsRef<Path>, block_size: usize, block_count: usize,
    ) -> Result<Self, RingError> {
        validate(block_size, block_count)?;
        let total = frame_region_file_size(block_size, block_count);
        let file = crate::region_file::open_existing(path.as_ref())?;
        let len = file.metadata()?.len();
        if (len as usize) < total {
            return Err(RingError::LayoutMismatch(crate::layout::Mismatch::new(
                "length",
                len,
                total as u64,
            )));
        }
        let mut mmap = unsafe { MmapOptions::new().len(total).map_mut(&file)? };
        Self::check_header(mmap.as_ptr(), block_size, block_count)?;
        let raw_ptr = mmap.as_mut_ptr();
        Ok(Self::from_parts(RegionBacking::File(file, mmap), raw_ptr, block_size, block_count))
    }

    /// Build a region on a named RAM-resident shared-memory backing.
    pub fn create_from_shm(
        mut shm: crate::shm_file::ShmFile, block_size: usize, block_count: usize,
    ) -> Result<Self, RingError> {
        validate(block_size, block_count)?;
        let wanted = frame_region_file_size(block_size, block_count);
        if shm.len() < wanted {
            return Err(RingError::LayoutMismatch(crate::layout::Mismatch::new(
                "length",
                shm.len() as u64,
                wanted as u64,
            )));
        }
        let raw_ptr = shm.as_mut_slice().as_mut_ptr();
        unsafe { init_region(raw_ptr, block_size, block_count) };
        Ok(Self::from_parts(RegionBacking::Shm(shm), raw_ptr, block_size, block_count))
    }

    /// Open an existing named ShmFs-backed region (no re-init).
    pub fn open_from_shm(
        mut shm: crate::shm_file::ShmFile, block_size: usize, block_count: usize,
    ) -> Result<Self, RingError> {
        validate(block_size, block_count)?;
        let wanted = frame_region_file_size(block_size, block_count);
        if shm.len() < wanted {
            return Err(RingError::LayoutMismatch(crate::layout::Mismatch::new(
                "length",
                shm.len() as u64,
                wanted as u64,
            )));
        }
        let raw_ptr = shm.as_mut_slice().as_mut_ptr();
        Self::check_header(raw_ptr, block_size, block_count)?;
        Ok(Self::from_parts(RegionBacking::Shm(shm), raw_ptr, block_size, block_count))
    }

    /// Open the named shared-memory region as it stands, taking its block
    /// size and count from its header rather than from the caller: for a
    /// handle adopting a region another handle made. Nothing is created,
    /// so a region that is not there is `IoError(NotFound)`, and one whose
    /// layout is not finished is `LayoutMismatch`.
    pub(crate) fn open_shm_as_found(
        name: &str,
        namespace: crate::shm_file::ShmNamespace,
        sddl: Option<&str>,
    ) -> Result<Self, RingError> {
        let header = std::mem::size_of::<FrameRegionHeader>();
        let mut head = crate::shm_file::ShmFile::open_named_secured(name, header, namespace, sddl)
            .map_err(|e| crate::mmf_attach::attach_error(e, RingError::LayoutMismatch))?;
        let (block_size, block_count) = Self::geometry_at(head.as_mut_slice().as_ptr())?;
        drop(head);
        let total = frame_region_file_size(block_size, block_count);
        let shm = crate::shm_file::ShmFile::open_named_secured(name, total, namespace, sddl)
            .map_err(|e| crate::mmf_attach::attach_error(e, RingError::LayoutMismatch))?;
        Self::open_from_shm(shm, block_size, block_count)
    }

    /// As [`open_shm_as_found`](Self::open_shm_as_found), for the
    /// file-backed region at `path`.
    pub(crate) fn open_file_as_found(path: impl AsRef<Path>) -> Result<Self, RingError> {
        let path = path.as_ref();
        let file = crate::region_file::open_existing(path)?;
        let header = std::mem::size_of::<FrameRegionHeader>();
        let len = file.metadata()?.len();
        if (len as usize) < header {
            return Err(RingError::LayoutMismatch(crate::layout::Mismatch::new(
                "length",
                len,
                header as u64,
            )));
        }
        let head = unsafe { MmapOptions::new().len(header).map(&file)? };
        let (block_size, block_count) = Self::geometry_at(head.as_ptr())?;
        drop(head);
        Self::open(path, block_size, block_count)
    }

    /// The block size and count a finished region's header records.
    fn geometry_at(ptr: *const u8) -> Result<(usize, usize), RingError> {
        let h = unsafe { &*(ptr as *const FrameRegionHeader) };
        let magic = h.magic.load(Ordering::Acquire);
        if magic != FRAME_REGION_MAGIC {
            return Err(RingError::LayoutMismatch(crate::layout::Mismatch::new(
                "magic",
                magic,
                FRAME_REGION_MAGIC,
            )));
        }
        let (block_size, block_count) = (h.block_size as usize, h.block_count as usize);
        validate(block_size, block_count)?;
        Ok((block_size, block_count))
    }

    /// Create-or-open a named ShmFs frame region. Every handle that finds
    /// the layout missing lays it out with the same swaps from zero and the
    /// first swap of the magic publishes it, so no handle waits on another
    /// and a late-joining consumer never wipes a region a producer already
    /// filled. This is the shared
    /// payload region the cross-process offset-frame path needs: the
    /// producer create-or-opens it on the first offset `send_frame`, and
    /// a consumer only opens the region as it stands, so one that is gone
    /// is an error rather than a fresh region read as the payload.
    pub fn create_or_open_shm(
        name: &str, block_size: usize, block_count: usize,
    ) -> Result<Self, RingError> {
        Self::create_or_open_shm_in(
            name, block_size, block_count, crate::shm_file::ShmNamespace::Session,
        )
    }

    /// As [`create_or_open_shm`](Self::create_or_open_shm), naming the
    /// region in `namespace`.
    ///
    /// A ring creates its payload region on the first oversized frame
    /// rather than at construction, so the namespace its backings were
    /// named in has to reach this call for the region to land beside
    /// them. A region named session-scoped behind machine-scoped
    /// backings splits a ring that has worked until the moment someone
    /// sends a payload too large to inline.
    pub fn create_or_open_shm_in(
        name: &str, block_size: usize, block_count: usize,
        namespace: crate::shm_file::ShmNamespace,
    ) -> Result<Self, RingError> {
        Self::create_or_open_shm_secured(name, block_size, block_count, namespace, None)
    }

    /// As [`create_or_open_shm_in`](Self::create_or_open_shm_in), with
    /// `sddl` as the security descriptor a create applies. A ring builds
    /// this region on its first oversized frame, so a descriptor that
    /// reaches the backings has to reach here too, or a ring that has
    /// carried small payloads all along refuses the first large one.
    pub fn create_or_open_shm_secured(
        name: &str, block_size: usize, block_count: usize,
        namespace: crate::shm_file::ShmNamespace,
        sddl: Option<&str>,
    ) -> Result<Self, RingError> {
        validate(block_size, block_count)?;
        let total = frame_region_file_size(block_size, block_count);
        let mut shm = crate::shm_file::ShmFile::create_or_open_named_secured(
            name, total, namespace, sddl,
        )
        .map_err(|e| crate::mmf_attach::attach_error(e, RingError::LayoutMismatch))?;
        if shm.len() < total {
            return Err(RingError::LayoutMismatch(crate::layout::Mismatch::new(
                "length",
                shm.len() as u64,
                total as u64,
            )));
        }
        let raw_ptr = shm.as_mut_slice().as_mut_ptr();
        Self::lay_out(raw_ptr, block_size, block_count)?;
        Ok(Self::from_parts(RegionBacking::Shm(shm), raw_ptr, block_size, block_count))
    }

    /// Create-or-open a file-backed frame region: the file-locale peer
    /// of [`create_or_open_shm`](Self::create_or_open_shm), for rings
    /// backed by [`AdaptiveRing::create`](crate::adaptive_ring::AdaptiveRing::create)
    /// / `open`.
    pub fn create_or_open_file(
        path: impl AsRef<Path>, block_size: usize, block_count: usize,
    ) -> Result<Self, RingError> {
        validate(block_size, block_count)?;
        let total = frame_region_file_size(block_size, block_count);
        let file = crate::region_file::create_or_open(path.as_ref())?;
        if (file.metadata()?.len() as usize) < total {
            file.set_len(total as u64)?;
        }
        let mut mmap = unsafe { MmapOptions::new().len(total).map_mut(&file)? };
        let raw_ptr = mmap.as_mut_ptr();
        Self::lay_out(raw_ptr, block_size, block_count)?;
        Ok(Self::from_parts(RegionBacking::File(file, mmap), raw_ptr, block_size, block_count))
    }

    /// Lay the region out, or finish a layout another handle began, so it
    /// is published either way; used by both create-or-open paths. Every
    /// handle makes the same swaps from zero, the block size, the block
    /// count and the instance, then the magic, and none waits on another:
    /// a fresh region's cursors, free-list head and tags already read as an
    /// empty region with every block free, and the first swap of the magic
    /// publishes it. The instance is drawn by whichever handle swaps it in
    /// first and kept by the rest. A geometry already there that differs
    /// is another region's, and a magic other than this layout's another
    /// layout's; both are refused as a mismatch, the magic before anything
    /// is written.
    fn lay_out(
        raw_ptr: *mut u8, block_size: usize, block_count: usize,
    ) -> Result<(), RingError> {
        let hdr = raw_ptr as *mut FrameRegionHeader;
        let magic = unsafe { &(*hdr).magic };
        let mismatch = |field, found, wanted| {
            Err(RingError::LayoutMismatch(crate::layout::Mismatch::new(field, found, wanted)))
        };
        match magic.load(Ordering::Acquire) {
            FRAME_REGION_MAGIC => {}
            0 => {
                // SAFETY: the header is 64-byte aligned, so these words are
                // 8-byte aligned, and every handle reaches them atomically
                // while a layout can be under way.
                let (size_word, count_word, instance_word) = unsafe {
                    (
                        AtomicU64::from_ptr(std::ptr::addr_of_mut!((*hdr).block_size)),
                        AtomicU64::from_ptr(std::ptr::addr_of_mut!((*hdr).block_count)),
                        AtomicU64::from_ptr(std::ptr::addr_of_mut!((*hdr).instance)),
                    )
                };
                for (field, word, wanted) in [
                    ("block size", size_word, block_size as u64),
                    ("block count", count_word, block_count as u64),
                ] {
                    if let Err(found) = word.compare_exchange(0, wanted, Ordering::AcqRel, Ordering::Acquire)
                        && found != wanted
                    {
                        return mismatch(field, found, wanted);
                    }
                }
                if let Err(held) =
                    instance_word.compare_exchange(0, fresh_instance(), Ordering::AcqRel, Ordering::Acquire)
                {
                    debug_assert_ne!(held, 0, "a swap from zero lost only to an instance drawn");
                }
                match magic.compare_exchange(0, FRAME_REGION_MAGIC, Ordering::AcqRel, Ordering::Acquire) {
                    Ok(_) | Err(FRAME_REGION_MAGIC) => {}
                    Err(found) => return mismatch("magic", found, FRAME_REGION_MAGIC),
                }
            }
            found => return mismatch("magic", found, FRAME_REGION_MAGIC),
        }
        let h = unsafe { &*(raw_ptr as *const FrameRegionHeader) };
        crate::layout::Mismatch::first(&[
            ("block size", h.block_size, block_size as u64),
            ("block count", h.block_count, block_count as u64),
        ])
        .map_err(RingError::LayoutMismatch)?;
        Ok(())
    }

    /// Give up a shared-memory region's name, so it outlives this handle:
    /// see [`ShmFile::keep_name`](crate::shm_file::ShmFile::keep_name).
    pub(crate) fn keep_name(&mut self) {
        if let RegionBacking::Shm(shm) = &mut self._backing {
            shm.keep_name();
        }
    }

    fn from_parts(
        backing: RegionBacking, raw_ptr: *mut u8, block_size: usize, block_count: usize,
    ) -> Self {
        Self {
            _backing: backing, raw_ptr, block_size, block_count,
            blocks_base: std::mem::size_of::<FrameRegionHeader>() + tags_bytes(block_count),
            bump: crate::bump::Limit::new(block_count as u64),
        }
    }

    fn check_header(ptr: *const u8, block_size: usize, block_count: usize) -> Result<(), RingError> {
        let h = unsafe { &*(ptr as *const FrameRegionHeader) };
        crate::layout::Mismatch::first(&[
            ("magic", h.magic.load(Ordering::Acquire), FRAME_REGION_MAGIC),
            ("block size", h.block_size, block_size as u64),
            ("block count", h.block_count, block_count as u64),
        ])
        .map_err(RingError::LayoutMismatch)?;
        Ok(())
    }

    /// Largest payload a block holds.
    pub fn block_size(&self) -> usize { self.block_size }
    /// Number of blocks.
    pub fn block_count(&self) -> usize { self.block_count }

    /// Which laying-out of the region this handle maps, drawn afresh each
    /// time the region is laid out; zero only for a region whose maker
    /// records none.
    pub(crate) fn instance(&self) -> u64 { self.header().instance }

    fn header(&self) -> &FrameRegionHeader {
        unsafe { &*(self.raw_ptr as *const FrameRegionHeader) }
    }

    fn block_ptr(&self, idx: u32) -> *mut u8 {
        unsafe { self.raw_ptr.add(self.blocks_base + idx as usize * self.block_size) }
    }

    /// Block `idx`'s allocation tag: odd while an offset frame holds the
    /// block, even while it is free. `idx` is below the block count.
    fn tag(&self, idx: u32) -> &AtomicU64 {
        // SAFETY: the tags lie between the header and the blocks, one aligned
        // eight-byte word per block, and `idx` is below the block count.
        unsafe {
            &*(self.raw_ptr.add(std::mem::size_of::<FrameRegionHeader>() + idx as usize * 8) as *const AtomicU64)
        }
    }

    /// The block's first 4 bytes reinterpreted as the free-list link
    /// (only meaningful while the block is free).
    fn next_link(&self, idx: u32) -> &AtomicU32 {
        unsafe { &*(self.block_ptr(idx) as *const AtomicU32) }
    }

    /// Allocate a block. Free list first, then bump. `None` when full: the
    /// free-list head holds the mark of an exhausted region, read with no
    /// locked operation, or the bump cursor refused.
    pub fn alloc(&self) -> Option<u32> {
        loop {
            let head = self.header().free_head.load(Ordering::Acquire);
            let (counter, idx) = unpack(head);
            // Both tests compare with EXHAUSTED, so one compare's flags serve
            // them; above it is NIL, the empty list.
            if idx < EXHAUSTED {
                let next = self.next_link(idx).load(Ordering::Acquire);
                let new_head = pack(counter.wrapping_add(1), next);
                if self.header().free_head.compare_exchange(
                    head, new_head, Ordering::AcqRel, Ordering::Acquire,
                ).is_ok() {
                    return Some(idx);
                }
                continue;
            }
            if idx == EXHAUSTED {
                return None;
            }
            break;
        }
        let Some(idx) = crate::bump::take32(&self.header().bump_next, &self.bump) else {
            self.mark_exhausted();
            return None;
        };
        Some(idx)
    }

    /// Put [`EXHAUSTED`] in the free-list head while the list is empty, after
    /// the bump cursor refused an allocate. A block freed first leaves the
    /// mark to the next allocate that finds the list empty and the cursor
    /// spent.
    #[cold]
    #[inline(never)]
    fn mark_exhausted(&self) {
        let free = &self.header().free_head;
        let mut head = free.load(Ordering::Acquire);
        loop {
            let (counter, idx) = unpack(head);
            if idx != NIL {
                return;
            }
            let marked = pack(counter.wrapping_add(1), EXHAUSTED);
            match free.compare_exchange_weak(head, marked, Ordering::AcqRel, Ordering::Acquire) {
                Ok(_) => return,
                Err(now) => head = now,
            }
        }
    }

    /// Return a block to the free list. Any consumer may free any block.
    pub fn free(&self, idx: u32) {
        if idx as usize >= self.block_count {
            return;
        }
        loop {
            let head = self.header().free_head.load(Ordering::Acquire);
            let (counter, old_top) = unpack(head);
            self.next_link(idx).store(old_top, Ordering::Release);
            let new_head = pack(counter.wrapping_add(1), idx);
            if self.header().free_head.compare_exchange(
                head, new_head, Ordering::AcqRel, Ordering::Acquire,
            ).is_ok() {
                return;
            }
        }
    }

    /// Copy `payload` into block `idx`. Caller guarantees
    /// `payload.len() <= block_size`.
    pub fn write_block(&self, idx: u32, payload: &[u8]) {
        debug_assert!(payload.len() <= self.block_size);
        unsafe {
            std::ptr::copy_nonoverlapping(
                payload.as_ptr(), self.block_ptr(idx), payload.len(),
            );
        }
    }

    /// Copy `len` bytes out of block `idx` into `out`, for a caller that
    /// already has the buffer. Returns what it copied, which is `len`
    /// clamped to the block and to the buffer.
    pub fn read_block(&self, idx: u32, len: usize, out: &mut [u8]) -> usize {
        if idx as usize >= self.block_count {
            return 0;
        }
        let n = len.min(self.block_size).min(out.len());
        // SAFETY: the index is inside the region and `n` inside the block.
        unsafe {
            std::ptr::copy_nonoverlapping(self.block_ptr(idx), out.as_mut_ptr(), n);
        }
        n
    }

    /// Copy `len` bytes out of block `idx` into `out` (appended).
    pub fn read_block_into(&self, idx: u32, len: usize, out: &mut Vec<u8>) {
        debug_assert!(len <= self.block_size);
        out.reserve(len);
        unsafe {
            std::ptr::copy_nonoverlapping(
                self.block_ptr(idx),
                out.spare_capacity_mut().as_mut_ptr() as *mut u8,
                len,
            );
            let new_len = out.len() + len;
            out.set_len(new_len);
        }
    }

    /// Allocate a block for an offset frame: its index and the tag the
    /// frame's descriptor carries, odd and unlike any the block carried
    /// before. `None` when full.
    pub(crate) fn alloc_tagged(&self) -> Option<(u32, u64)> {
        let idx = self.alloc()?;
        let tag = self.tag(idx);
        let held = tag.load(Ordering::Relaxed).wrapping_add(1) | 1;
        tag.store(held, Ordering::Relaxed);
        Some((idx, held))
    }

    /// Free block `idx`, held by the frame of tag `held`: the tag turns even
    /// before the block returns to the free list, so no descriptor matches a
    /// free block.
    pub(crate) fn free_tagged(&self, idx: u32, held: u64) {
        self.tag(idx).store(held.wrapping_add(1), Ordering::Release);
        self.free(idx);
    }

    /// Append the `len` bytes the frame of tag `held` keeps in block `idx` to
    /// `out`, and free the block. Refused, with nothing read or freed, when
    /// the index is past the region, the length past the block, or the block
    /// does not carry that tag while held.
    pub(crate) fn take_tagged(&self, idx: u32, held: u64, len: usize, out: &mut Vec<u8>) -> Result<(), RingError> {
        if idx as usize >= self.block_count {
            return Err(not_a_frame("the descriptor names a block past the region"));
        }
        if len > self.block_size {
            return Err(not_a_frame("the descriptor names a length past the block"));
        }
        if held & 1 == 0 || self.tag(idx).load(Ordering::Acquire) != held {
            return Err(not_a_frame("the block does not carry the descriptor's allocation tag"));
        }
        self.read_block_into(idx, len, out);
        self.free_tagged(idx, held);
        Ok(())
    }
}

/// The error for a slot read as a frame that is none: `InvalidData`, saying
/// which check it failed.
pub(crate) fn not_a_frame(why: &'static str) -> RingError {
    RingError::IoError(std::io::Error::new(std::io::ErrorKind::InvalidData, why))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::sync::atomic::AtomicUsize;
    use std::thread;

    #[test]
    fn alloc_write_read_free_cycle() {
        let r = FrameRegion::create_anon(256, 8).unwrap();
        let idx = r.alloc().unwrap();
        let payload = vec![0xABu8; 200];
        r.write_block(idx, &payload);
        let mut out = Vec::new();
        r.read_block_into(idx, 200, &mut out);
        assert_eq!(out, payload);
        r.free(idx);
        // Freed block is reused by the next alloc.
        let idx2 = r.alloc().unwrap();
        assert_eq!(idx2, idx, "freed block returns to the stack");
    }

    #[test]
    fn a_tagged_take_reads_and_frees_only_the_block_its_tag_holds() {
        let r = FrameRegion::create_anon(64, 4).unwrap();
        let (idx, held) = r.alloc_tagged().unwrap();
        assert_eq!(held & 1, 1, "a held block's tag is odd");
        r.write_block(idx, b"a frame's payload");
        let mut out = Vec::new();
        assert!(r.take_tagged(idx, held + 1, 17, &mut out).is_err(), "an even tag is refused");
        assert!(r.take_tagged(idx, held + 2, 17, &mut out).is_err(), "another tag is refused");
        assert!(r.take_tagged(4, held, 17, &mut out).is_err(), "a block past the region is refused");
        assert!(r.take_tagged(idx, held, 65, &mut out).is_err(), "a length past the block is refused");
        assert!(out.is_empty(), "a refused take reads nothing");
        r.take_tagged(idx, held, 17, &mut out).unwrap();
        assert_eq!(out, b"a frame's payload", "the refusals left the block held, and read it");
        assert!(r.take_tagged(idx, held, 17, &mut out).is_err(), "a freed block refuses the tag it was held under");
        let (again, next) = r.alloc_tagged().unwrap();
        assert_eq!(again, idx, "the freed block returns to the free list");
        assert!(next & 1 == 1 && next != held, "it is held again under a new odd tag");
    }

    #[test]
    fn exhausts_then_full() {
        let r = FrameRegion::create_anon(64, 4).unwrap();
        let a: Vec<u32> = (0..4).map(|_| r.alloc().unwrap()).collect();
        assert_eq!(a.len(), 4);
        assert!(r.alloc().is_none(), "region full");
        r.free(a[1]);
        assert!(r.alloc().is_some(), "freeing reopens a block");
    }

    /// The allocate the bump cursor refuses puts the exhausted mark in the
    /// free-list head: a later allocate refuses without moving the cursor, a
    /// block freed onto the mark is handed out again, and the list empties
    /// back to the mark.
    #[test]
    fn an_exhausted_region_refuses_from_its_free_list_head() {
        let r = FrameRegion::create_anon(64, 4).unwrap();
        let a: Vec<u32> = (0..4).map(|_| r.alloc().unwrap()).collect();
        assert_eq!(a, [0, 1, 2, 3]);
        let head = || unpack(r.header().free_head.load(Ordering::Acquire)).1;
        let cursor = || r.header().bump_next.load(Ordering::Acquire);
        assert_eq!(head(), NIL, "taking the last block leaves the list empty, unmarked");
        assert_eq!(cursor(), 4, "no take passed the limit");
        assert!(r.alloc().is_none());
        assert_eq!(head(), EXHAUSTED, "the refused allocate left the mark");
        assert_eq!(cursor(), 4, "the refused take was given back");
        assert!(r.alloc().is_none());
        assert_eq!(cursor(), 4, "a refusal from the mark moves no counter");
        r.free(2);
        assert_eq!(r.alloc(), Some(2), "a block freed onto the mark is handed out again");
        assert_eq!(head(), EXHAUSTED, "the list empties back to the mark");
        assert!(r.alloc().is_none());
    }

    /// A region whose cursor is at its limit with no mark in its empty free
    /// list, as taking the last block leaves it, overshoots once, gives the
    /// overshoot back and marks the list.
    #[test]
    fn an_empty_list_without_the_mark_overshoots_once_then_marks() {
        let r = FrameRegion::create_anon(64, 2).unwrap();
        assert_eq!((r.alloc(), r.alloc()), (Some(0), Some(1)));
        r.header().free_head.store(pack(7, NIL), Ordering::Release);
        assert!(r.alloc().is_none());
        assert_eq!(r.header().bump_next.load(Ordering::Acquire), 2, "the overshoot was given back");
        assert_eq!(unpack(r.header().free_head.load(Ordering::Acquire)), (8, EXHAUSTED));
    }

    /// Allocates racing past the limit get every block once between them,
    /// and the region ends marked with its cursor at the limit.
    #[test]
    fn racing_allocates_take_every_block_once_and_leave_the_mark() {
        const BLOCKS: usize = 4096;
        let r = Arc::new(FrameRegion::create_anon(64, BLOCKS).unwrap());
        let takers: Vec<_> = (0..8)
            .map(|_| {
                let r = r.clone();
                thread::spawn(move || std::iter::from_fn(|| r.alloc()).collect::<Vec<u32>>())
            })
            .collect();
        let mut seen = std::collections::HashSet::new();
        for idx in takers.into_iter().flat_map(|h| h.join().unwrap()) {
            assert!((idx as usize) < BLOCKS && seen.insert(idx), "block {idx} out of range or handed out twice");
        }
        assert_eq!(seen.len(), BLOCKS);
        assert_eq!(r.header().bump_next.load(Ordering::Acquire), BLOCKS as u32);
        assert_eq!(unpack(r.header().free_head.load(Ordering::Acquire)).1, EXHAUSTED);
    }

    #[test]
    fn concurrent_alloc_free_no_double_issue() {
        // Many threads alloc + free in a loop; assert no index is ever
        // held by two threads at once (a double-issue would corrupt).
        let r = Arc::new(FrameRegion::create_anon(64, 64).unwrap());
        let held: Arc<Vec<AtomicUsize>> =
            Arc::new((0..64).map(|_| AtomicUsize::new(0)).collect());
        let mut handles = Vec::new();
        for _ in 0..8 {
            let r = r.clone();
            let held = held.clone();
            handles.push(thread::spawn(move || {
                for _ in 0..20_000 {
                    if let Some(idx) = r.alloc() {
                        let prev = held[idx as usize].fetch_add(1, Ordering::AcqRel);
                        assert_eq!(prev, 0, "block {idx} double-issued");
                        held[idx as usize].fetch_sub(1, Ordering::AcqRel);
                        r.free(idx);
                    }
                }
            }));
        }
        for h in handles {
            h.join().unwrap();
        }
    }

    #[test]
    fn shm_cross_handle() {
        use crate::shm_file::ShmFile;
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let name = format!("frame_region_{}_{}", std::process::id(), nonce);
        let (bs, bc) = (256usize, 8usize);
        let size = frame_region_file_size(bs, bc);
        let a = FrameRegion::create_from_shm(
            ShmFile::create_or_open_named(&name, size).unwrap(), bs, bc).unwrap();
        let b = FrameRegion::open_from_shm(
            ShmFile::create_or_open_named(&name, size).unwrap(), bs, bc).unwrap();
        let idx = a.alloc().unwrap();
        a.write_block(idx, b"shared across handles");
        let mut out = Vec::new();
        b.read_block_into(idx, 21, &mut out);
        assert_eq!(out, b"shared across handles");
    }

    #[test]
    fn rejects_bad_params() {
        assert!(matches!(FrameRegion::create_anon(7, 8), Err(RingError::InvalidArgument(_))));
        assert!(matches!(FrameRegion::create_anon(64, 0), Err(RingError::InvalidArgument(_))));
    }

    /// A second create attaches with allocated blocks in place; reset is
    /// what strips them.
    #[test]
    fn second_create_attaches_and_keeps_blocks() {
        let p = std::env::temp_dir().join(format!(
            "subetha-frame-region-attach-{}.bin", std::process::id(),
        ));
        std::fs::remove_file(&p).ok();
        let (bs, bc) = (64usize, 8usize);

        let r = FrameRegion::create(&p, bs, bc).unwrap();
        let first = r.alloc().expect("alloc");

        let r2 = FrameRegion::create(&p, bs, bc).unwrap();
        assert_ne!(
            r2.alloc().expect("second alloc"),
            first,
            "attach handed back a block the first handle already holds",
        );
        assert!(matches!(
            FrameRegion::create(&p, bs, bc * 2),
            Err(RingError::LayoutMismatch(_)),
        ));

        // Windows refuses to truncate a mapped file, so every handle goes
        // before the reset.
        drop(r);
        drop(r2);
        let fresh = FrameRegion::reset(&p, bs, bc).unwrap();
        assert_eq!(fresh.alloc().expect("alloc after reset"), first,
                   "reset kept an allocation");
        drop(fresh);
        std::fs::remove_file(&p).ok();
    }

    /// A fresh path for one test: whatever an earlier run left there is
    /// removed, and a removal refused for any reason but absence fails the
    /// test rather than gating it on a stale region.
    fn fresh_path(tag: &str) -> std::path::PathBuf {
        let p = std::env::temp_dir()
            .join(format!("subetha-frame-region-{tag}-{}.bin", std::process::id()));
        match std::fs::remove_file(&p) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => panic!("stale region {} not removed: {e}", p.display()),
        }
        p
    }

    /// A region file a handle left half laid out, its geometry written and
    /// its magic not, is finished by the next create at once rather than
    /// waited on, every block free and an instance drawn; one an earlier
    /// SubEtha left in progress is refused at once as another layout's
    /// magic.
    #[test]
    fn a_layout_left_unfinished_is_finished_and_an_earlier_one_refused_at_once() {
        let (bs, bc) = (64usize, 8usize);
        let total = frame_region_file_size(bs, bc);
        for (tag, magic) in [("unfinished", 0u64), ("earlier", 1u64)] {
            let p = fresh_path(&format!("left-{tag}"));
            let file = crate::region_file::create_or_open(&p).expect("the region file");
            file.set_len(total as u64).expect("the file takes the region's length");
            let mut left = unsafe { MmapOptions::new().len(total).map_mut(&file).expect("the file maps") };
            let hdr = left.as_mut_ptr() as *mut FrameRegionHeader;
            unsafe {
                (*hdr).block_size = bs as u64;
                (*hdr).block_count = bc as u64;
                (*hdr).magic.store(magic, Ordering::Release);
            }
            drop(left);
            drop(file);
            let started = std::time::Instant::now();
            let found = FrameRegion::create(&p, bs, bc);
            assert!(
                started.elapsed() < crate::mmf_attach::INIT_WAIT / 2,
                "{tag}: returned only after {:?}",
                started.elapsed()
            );
            if magic == 0 {
                let r = found.expect("the next create finishes the layout");
                assert_ne!(r.instance(), 0, "the finished layout draws an instance");
                let blocks: Vec<u32> = (0..bc).map(|_| r.alloc().expect("a free block")).collect();
                assert!(r.alloc().is_none(), "every block was free, and only those");
                r.free(blocks[2]);
                assert_eq!(r.alloc(), Some(blocks[2]), "the free list takes a block back");
                drop(r);
            } else {
                match found {
                    Ok(_) => panic!("a region another layout was building was attached"),
                    Err(e) => assert!(
                        matches!(&e, RingError::LayoutMismatch(m)
                            if *m == crate::layout::Mismatch::new("magic", 1, FRAME_REGION_MAGIC)),
                        "{e:?}"
                    ),
                }
            }
            std::fs::remove_file(&p).expect("the region file is unmapped and removable");
        }
    }

    /// Handles that create one region together each lay it out from zero,
    /// none waiting on another, and all of them end on one layout: one
    /// instance, and blocks allocated through any of them are distinct.
    #[test]
    fn handles_racing_to_lay_out_a_region_all_end_on_one_layout() {
        let (bs, bc) = (64usize, 16usize);
        let p = fresh_path("race");
        let gate = Arc::new(std::sync::Barrier::new(8));
        let handles: Vec<_> = (0..8)
            .map(|_| {
                let (p, gate) = (p.clone(), Arc::clone(&gate));
                thread::spawn(move || {
                    gate.wait();
                    FrameRegion::create(&p, bs, bc).expect("each handle lays out or attaches")
                })
            })
            .collect();
        let regions: Vec<FrameRegion> =
            handles.into_iter().map(|h| h.join().expect("the handle's thread")).collect();
        let instance = regions[0].instance();
        assert_ne!(instance, 0, "the layout drew an instance");
        assert!(regions.iter().all(|r| r.instance() == instance), "the handles hold different layouts");
        let mut taken = std::collections::HashSet::new();
        for i in 0..bc {
            let idx = regions[i % regions.len()].alloc().expect("a free block");
            assert!(taken.insert(idx), "block {idx} was issued through two handles");
        }
        assert!(regions.iter().all(|r| r.alloc().is_none()), "the handles share one set of blocks");
        drop(regions);
        std::fs::remove_file(&p).expect("every mapping is dropped and the file removable");
    }
}
