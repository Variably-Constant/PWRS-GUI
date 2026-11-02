//! Fixed-size slots in chunk files beside a structure's main file, for a
//! structure that grows while other processes read it.
//!
//! A pool hands out 32-bit slot indexes: the high bits name a chunk and
//! the low bits a slot in it, so an index names the same slot in every
//! process whatever address it mapped the chunk at. A chunk file is made
//! when the bump cursor first reaches it, through the create-or-attach
//! election every file-backed structure uses, so two processes reaching a
//! new chunk at once make it once. A process maps a chunk the first time
//! it meets an index in it.
//!
//! Every slot begins with a 16-byte header the pool owns: a link and a
//! stamp. The structure built on the pool uses the bytes after it and
//! never touches the header. A slot goes back in one of two ways. One no
//! other thread can have seen goes straight onto the free list. One a
//! reader may still be on is retired: it joins a first-in, first-out
//! queue stamped with the epoch table's published epoch, and reaches the
//! free list only once the reclaim horizon is past that stamp, when no pin
//! that could have seen it is left. Retiring writes only the header, so a
//! reader still on a retired slot reads what the structure left there.
//!
//! The free list and the queue are lock-free. Their heads and every slot
//! link pack a 32-bit counter beside the index, so a slot that leaves and
//! comes back between one thread's load and its compare-and-swap fails
//! the swap.
//!
//! A writer allocates from slots it holds alone: a run of the bump cursor,
//! then a chain of up to [`CHAIN_MAX`] slots it took off the free list in
//! one swap, so concurrent writers do not meet on the cursor or the list's
//! top for every slot.

use std::fs::File;
use std::io;
use std::mem::size_of;
use std::path::{Path, PathBuf};
use std::ptr::{null_mut, NonNull};
use std::sync::atomic::{AtomicPtr, AtomicU32, AtomicU64, Ordering};

use memmap2::MmapMut;

use crate::shared_epochs::Epoch;

/// Bytes at the start of every slot that belong to the pool.
pub(crate) const SLOT_HEADER: usize = 16;

/// Bytes at the start of a chunk file before its first slot.
const CHUNK_HEADER: usize = 64;

/// "VCHUNK_1".
const CHUNK_MAGIC: u64 = 0x5643_4855_4E4B_5F31;

/// The pool's part of a slot.
#[repr(C)]
struct SlotHeader {
    /// The free list's, the retirement queue's or a writer's chain's next
    /// slot, encoded, beside a counter that moves on every change.
    link: AtomicU64,
    /// The published epoch at which the slot was retired.
    stamp: AtomicU64,
}

const _: () = assert!(size_of::<SlotHeader>() == SLOT_HEADER);

/// A chunk file's header.
#[repr(C)]
struct ChunkHeader {
    magic: u64,
    slot_size: u64,
    chunk: u64,
    _pad: [u8; 40],
}

const _: () = assert!(size_of::<ChunkHeader>() == CHUNK_HEADER);

/// A pool's shared state, kept in the structure's main file.
#[repr(C, align(64))]
pub(crate) struct PoolHeader {
    slot_size: u32,
    chunk_bits: u32,
    /// Slots the bump cursor has handed out, slot 0 among them as the
    /// retirement queue's first dummy. Only rises.
    bump: AtomicU64,
    /// The free list's top, encoded, beside a counter.
    free: AtomicU64,
    /// The retirement queue's dummy, encoded, beside a counter.
    queue_head: AtomicU64,
    /// The retirement queue's last slot, encoded, beside a counter.
    queue_tail: AtomicU64,
    _pad: [u8; 24],
}

const _: () = assert!(size_of::<PoolHeader>() == 64);

/// Indexes are stored plus one, so zeroed memory reads as no slot.
#[inline]
fn enc(index: u32) -> u32 {
    index + 1
}

#[inline]
fn dec(stored: u32) -> u32 {
    stored - 1
}

#[inline]
fn pack(counter: u32, stored: u32) -> u64 {
    (u64::from(counter) << 32) | u64::from(stored)
}

#[inline]
fn stored_of(word: u64) -> u32 {
    word as u32
}

#[inline]
fn counter_of(word: u64) -> u32 {
    (word >> 32) as u32
}

impl PoolHeader {
    /// Lay out an empty pool whose slots are `slot_size` bytes, the
    /// pool's header included, in chunks of `1 << chunk_bits` slots.
    ///
    /// # Safety
    /// `this` addresses a zeroed `PoolHeader` no other thread uses yet.
    pub(crate) unsafe fn init(this: *mut PoolHeader, slot_size: usize, chunk_bits: u32) {
        unsafe {
            (*this).slot_size = slot_size as u32;
            (*this).chunk_bits = chunk_bits;
            (*this).bump.store(1, Ordering::Relaxed);
            (*this).queue_head.store(pack(0, enc(0)), Ordering::Relaxed);
            (*this).queue_tail.store(pack(0, enc(0)), Ordering::Relaxed);
        }
    }

    /// The slot size this pool was laid out with, its header included.
    pub(crate) fn slot_size(&self) -> usize {
        self.slot_size as usize
    }

    /// Log2 of the slots one chunk holds.
    pub(crate) fn chunk_bits(&self) -> u32 {
        self.chunk_bits
    }
}

/// The slot size a pool needs for `body` bytes of the structure's own,
/// aligned to `align`: the pool's header, then the body, rounded up to a
/// multiple of 16 and of `align`. `None` when that does not fit the pool
/// header's 32-bit slot size.
pub(crate) fn slot_size_for(body: usize, align: usize) -> Option<usize> {
    let unit = align.max(16);
    let bytes = SLOT_HEADER.next_multiple_of(unit).checked_add(body)?.checked_next_multiple_of(unit)?;
    (bytes <= u32::MAX as usize).then_some(bytes)
}

/// Where a slot's body starts past its header, for a body aligned to
/// `align`.
pub(crate) const fn body_offset(align: usize) -> usize {
    SLOT_HEADER.next_multiple_of(if align > 16 { align } else { 16 })
}

/// One chunk mapped in this process.
struct ChunkMap {
    mmap: MmapMut,
    _file: File,
}

/// The slots a writer has reserved and not yet handed out: `[next, end)`
/// from the bump cursor, with the length of the last run it claimed, and a
/// chain of slots it took off the free list. Process-local and touched by
/// one writer at a time (a ticket slot is held exclusively), so a dead
/// writer strands only an unused `[next, end)` tail and the rest of its
/// chain, which nobody else is handed. On its own cache line so two
/// writers' runs never false-share.
#[repr(C, align(64))]
struct Run {
    next: AtomicU32,
    end: AtomicU32,
    last_len: AtomicU32,
    /// The chain's first slot, encoded, or 0 for none; each slot's link
    /// names the next.
    chain: AtomicU32,
}

/// The first run a writer claims; each later run is twice the last, capped
/// at a chunk. One slot keeps the pre-run behavior for a writer that
/// allocates once and stops.
const FIRST_RUN: u32 = 1;

/// The most slots a writer takes off the free list in one compare-and-swap:
/// enough to keep the list's word off its allocations, few enough that the
/// walk inside the swap stays short.
const CHAIN_MAX: u32 = 64;

/// A pool's slot geometry, read once for a run of lookups such as one
/// search's hops. Its fields are plain values, so a lookup needs no load
/// beyond the slot itself while the slot lies in chunk 0, and one load of
/// its chunk's base past it. A lookup through [`ChunkPool::body`] reads the
/// pool's fields each time, since an atomic load between two lookups keeps
/// the compiler from reusing them.
#[derive(Clone, Copy)]
pub(crate) struct Slots<'a> {
    pool: &'a ChunkPool,
    first: *mut u8,
    bases: &'a [AtomicPtr<u8>],
    chunk_bits: u32,
    slot_size: usize,
}

impl Slots<'_> {
    /// [`ChunkPool::body`] for slot `index`.
    #[inline]
    pub(crate) fn body(&self, index: u32, offset: usize) -> io::Result<*mut u8> {
        let chunk = (index >> self.chunk_bits) as usize;
        let base = if chunk == 0 {
            self.first
        } else {
            match self.bases.get(chunk) {
                Some(base) => match base.load(Ordering::Acquire) {
                    mapped if !mapped.is_null() => mapped,
                    _ => self.pool.map_chunk(chunk)?,
                },
                None => return Err(self.pool.past_capacity(index)),
            }
        };
        let within = (index as usize) & ((1usize << self.chunk_bits) - 1);
        // SAFETY: `within` is below the chunk's slot count, so the slot lies
        // inside the chunk's mapping, and `offset` is within the slot by the
        // structure's layout.
        Ok(unsafe { base.add(CHUNK_HEADER + within * self.slot_size + offset) })
    }

    /// Where byte `offset` of slot 0 lies. The same byte of slot `i` of
    /// chunk 0 lies `i * slot_size()` past it, for every `i` below
    /// `1 << chunk_bits()`.
    #[inline]
    pub(crate) fn first_at(&self, offset: usize) -> *mut u8 {
        // SAFETY: chunk 0 holds at least one slot and stays mapped for the
        // pool's life, and `offset` is within a slot by the structure's
        // layout.
        unsafe { self.first.add(CHUNK_HEADER + offset) }
    }

    #[inline]
    pub(crate) fn slot_size(&self) -> usize {
        self.slot_size
    }

    #[inline]
    pub(crate) fn chunk_bits(&self) -> u32 {
        self.chunk_bits
    }
}

/// A process's view of one pool.
pub(crate) struct ChunkPool {
    header: NonNull<PoolHeader>,
    main: PathBuf,
    name: &'static str,
    slot_size: usize,
    chunk_bits: u32,
    maps: Box<[AtomicPtr<ChunkMap>]>,
    /// Each installed mapping's first byte, so reaching a mapped chunk is
    /// one load rather than a load of the mapping and then of its pointer.
    bases: Box<[AtomicPtr<u8>]>,
    /// Chunk 0's first byte. Chunk 0 is mapped at attach and stays mapped
    /// for the pool's life, so its base is a plain field.
    first: *mut u8,
    /// One reservation per writer, indexed by its epoch-table ticket slot.
    runs: Box<[Run]>,
}

// SAFETY: the header lives in a shared mapping the owning structure keeps
// alive for the pool's life, and every field reached through it is atomic
// or written once before the pool is shared; the chunk maps are installed
// with one compare-and-swap and dropped only by `Drop`, and each base, and
// `first`, points into the installed map of its chunk.
unsafe impl Send for ChunkPool {}
unsafe impl Sync for ChunkPool {}

impl std::fmt::Debug for ChunkPool {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ChunkPool")
            .field("name", &self.name)
            .field("slot_size", &self.slot_size)
            .field("chunk_bits", &self.chunk_bits)
            .field("max_chunks", &self.maps.len())
            .field("writers", &self.runs.len())
            .finish()
    }
}

impl ChunkPool {
    /// The pool whose header is at `header` in the mapping of the file at
    /// `main`, with chunk files named after `main` and `name`, up to
    /// `max_chunks` of them, serving `writers` writers one run each. Maps
    /// chunk 0, where the retirement queue's first dummy lives, making it if
    /// this is the first attach.
    ///
    /// # Safety
    /// `header` addresses a `PoolHeader` laid out by
    /// [`PoolHeader::init`] in a mapping that outlives the returned pool.
    pub(crate) unsafe fn attach(
        header: *mut PoolHeader,
        main: &Path,
        name: &'static str,
        max_chunks: usize,
        writers: usize,
    ) -> io::Result<Self> {
        let Some(header) = NonNull::new(header) else {
            return Err(io::Error::new(io::ErrorKind::InvalidInput, "a pool header at null"));
        };
        // SAFETY: the caller vouches for a laid-out header.
        let h = unsafe { header.as_ref() };
        let mut pool = Self {
            header,
            main: main.to_path_buf(),
            name,
            slot_size: h.slot_size(),
            chunk_bits: h.chunk_bits(),
            maps: (0..max_chunks).map(|_| AtomicPtr::new(null_mut())).collect(),
            bases: (0..max_chunks).map(|_| AtomicPtr::new(null_mut())).collect(),
            first: null_mut(),
            runs: (0..writers)
                .map(|_| Run {
                    next: AtomicU32::new(0),
                    end: AtomicU32::new(0),
                    last_len: AtomicU32::new(0),
                    chain: AtomicU32::new(0),
                })
                .collect(),
        };
        pool.first = pool.chunk_base(0)?;
        Ok(pool)
    }

    /// The pool's slot geometry, read once for a run of lookups.
    #[inline]
    pub(crate) fn slots(&self) -> Slots<'_> {
        Slots { pool: self, first: self.first, bases: &self.bases, chunk_bits: self.chunk_bits, slot_size: self.slot_size }
    }

    fn header(&self) -> &PoolHeader {
        // SAFETY: `attach`'s caller vouched for the header and its mapping.
        unsafe { self.header.as_ref() }
    }

    #[inline]
    fn slots_per_chunk(&self) -> usize {
        1usize << self.chunk_bits
    }

    /// Slots the pool can ever hold.
    pub(crate) fn capacity(&self) -> usize {
        self.maps.len() << self.chunk_bits
    }

    fn chunk_path(&self, chunk: usize) -> io::Result<PathBuf> {
        let Some(stem) = self.main.file_name() else {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("{} does not name a file", self.main.display()),
            ));
        };
        let mut name = stem.to_os_string();
        name.push(format!(".{}{chunk}", self.name));
        Ok(self.main.with_file_name(name))
    }

    /// Where chunk `chunk` is mapped in this process, mapping it first if
    /// it is not, and making the file if no process has. Every access to a
    /// slot past chunk 0 comes through here, so the mapped case is one load
    /// with no frame of its own, and mapping lives in
    /// [`ChunkPool::map_chunk`].
    #[inline]
    fn chunk_base(&self, chunk: usize) -> io::Result<*mut u8> {
        let base = self.bases[chunk].load(Ordering::Acquire);
        if !base.is_null() {
            return Ok(base);
        }
        self.map_chunk(chunk)
    }

    /// Maps chunk `chunk` in this process, making the file if no process
    /// has, and installs the mapping and its base for every later access.
    /// A racing thread's mapping is dropped and the installed one returned.
    #[cold]
    #[inline(never)]
    fn map_chunk(&self, chunk: usize) -> io::Result<*mut u8> {
        let mapped = self.maps[chunk].load(Ordering::Acquire);
        if !mapped.is_null() {
            // SAFETY: installed below and dropped only by `Drop`.
            return Ok(self.publish_base(chunk, unsafe { (*mapped).mmap.as_ptr() as *mut u8 }));
        }
        let total = CHUNK_HEADER + self.slots_per_chunk() * self.slot_size;
        let path = self.chunk_path(chunk)?;
        let slot_size = self.slot_size as u64;
        let (file, mmap) = crate::mmf_attach::create_or_attach(
            &path,
            total,
            |ptr| {
                let h = ptr as *mut ChunkHeader;
                // SAFETY: a zeroed staging mapping of `total` bytes.
                unsafe {
                    (*h).slot_size = slot_size;
                    (*h).chunk = chunk as u64;
                    std::ptr::write_volatile(&raw mut (*h).magic, CHUNK_MAGIC);
                }
            },
            // SAFETY: the mapping is at least the header long.
            |ptr| unsafe { std::ptr::read_volatile(ptr as *const u64) },
            CHUNK_MAGIC,
        )?;
        // SAFETY: published with the magic, which is checked above.
        let h = unsafe { &*(mmap.as_ptr() as *const ChunkHeader) };
        crate::layout::Mismatch::first(&[
            ("chunk slot size", h.slot_size, slot_size),
            ("chunk number", h.chunk, chunk as u64),
        ])
        .map_err(|m| io::Error::new(io::ErrorKind::InvalidData, m))?;
        let ours = Box::into_raw(Box::new(ChunkMap { mmap, _file: file }));
        match self.maps[chunk].compare_exchange(null_mut(), ours, Ordering::AcqRel, Ordering::Acquire) {
            // SAFETY: just installed, dropped only by `Drop`.
            Ok(_) => Ok(self.publish_base(chunk, unsafe { (*ours).mmap.as_ptr() as *mut u8 })),
            Err(theirs) => {
                // SAFETY: `ours` was never shared.
                drop(unsafe { Box::from_raw(ours) });
                // SAFETY: installed by the winner, dropped only by `Drop`.
                Ok(self.publish_base(chunk, unsafe { (*theirs).mmap.as_ptr() as *mut u8 }))
            }
        }
    }

    /// Record `base` as chunk `chunk`'s first byte, the installed mapping's,
    /// for [`ChunkPool::chunk_base`]. Every caller stores the same pointer,
    /// so racing stores agree.
    fn publish_base(&self, chunk: usize, base: *mut u8) -> *mut u8 {
        self.bases[chunk].store(base, Ordering::Release);
        base
    }

    /// Where slot `index` starts, its header included.
    #[inline]
    fn slot_start(&self, index: u32) -> io::Result<*mut u8> {
        let chunk = (index >> self.chunk_bits) as usize;
        if chunk >= self.maps.len() {
            return Err(self.past_capacity(index));
        }
        let within = (index as usize) & (self.slots_per_chunk() - 1);
        let base = if chunk == 0 { self.first } else { self.chunk_base(chunk)? };
        // SAFETY: `within` is below the chunk's slot count, so the slot lies
        // inside the chunk's mapping.
        Ok(unsafe { base.add(CHUNK_HEADER + within * self.slot_size) })
    }

    /// The error for a slot index past every chunk the pool can hold.
    #[cold]
    #[inline(never)]
    fn past_capacity(&self, index: u32) -> io::Error {
        io::Error::new(io::ErrorKind::InvalidData, format!("slot {index} is past the {} the pool holds", self.capacity()))
    }

    fn slot_header(&self, index: u32) -> io::Result<&SlotHeader> {
        // SAFETY: every slot starts with its header, and the mapping lives as
        // long as the pool.
        Ok(unsafe { &*(self.slot_start(index)? as *const SlotHeader) })
    }

    /// The structure's bytes in slot `index`, past the pool's header at the
    /// offset [`body_offset`] gives.
    #[inline]
    pub(crate) fn body(&self, index: u32, offset: usize) -> io::Result<*mut u8> {
        // SAFETY: `offset` is within the slot by the structure's layout.
        Ok(unsafe { self.slot_start(index)?.add(offset) })
    }

    /// A slot no other thread holds, for the writer at ticket slot `writer`:
    /// one left in that writer's run, else one off the shared free list, else
    /// the first of a fresh run claimed from the bump cursor. `None` when
    /// every slot the pool can hold is in use or retired.
    ///
    /// Only the writer holding ticket slot `writer` touches that run, so the
    /// run's Relaxed loads and stores race with no other thread. A writer
    /// with a run takes slots off the free list a chain at a time, so the
    /// list's word is not on each of its allocations. A `writer` at or past
    /// the run count touches no run: it takes a slot off the free list or a
    /// single slot from the bump cursor, which any number of threads may do
    /// at once.
    pub(crate) fn alloc(&self, writer: usize) -> io::Result<Option<u32>> {
        let run = self.runs.get(writer);
        if let Some(run) = run {
            let next = run.next.load(Ordering::Relaxed);
            if next < run.end.load(Ordering::Relaxed) {
                run.next.store(next + 1, Ordering::Relaxed);
                self.slot_start(next)?;
                return Ok(Some(next));
            }
            if let Some(index) = self.take_chained(run)? {
                return Ok(Some(index));
            }
        } else if let Some(index) = self.pop_free()? {
            return Ok(Some(index));
        }
        let want = match run {
            Some(run) => {
                let last = run.last_len.load(Ordering::Relaxed);
                let doubled = if last == 0 { FIRST_RUN } else { last.saturating_mul(2) };
                doubled.min(self.slots_per_chunk() as u32).max(1)
            }
            None => 1,
        };
        let start = self.header().bump.fetch_add(u64::from(want), Ordering::AcqRel);
        if start >= self.capacity() as u64 {
            return Ok(None);
        }
        let index = start as u32;
        let end = (start + u64::from(want)).min(self.capacity() as u64) as u32;
        if let Some(run) = run {
            run.next.store(index + 1, Ordering::Relaxed);
            run.end.store(end, Ordering::Relaxed);
            run.last_len.store(want, Ordering::Relaxed);
        }
        // Map it now, making its chunk if this is the chunk's first slot,
        // so every later reader finds the file there.
        self.slot_start(index)?;
        Ok(Some(index))
    }

    /// The next slot of `run`'s chain, refilling the chain off the free list
    /// when it is empty; `None` when the free list is empty too.
    fn take_chained(&self, run: &Run) -> io::Result<Option<u32>> {
        let mut chained = run.chain.load(Ordering::Relaxed);
        if chained == 0 {
            chained = self.pop_chain()?;
            if chained == 0 {
                return Ok(None);
            }
        }
        let index = dec(chained);
        let next = self.slot_header(index)?.link.load(Ordering::Acquire);
        run.chain.store(stored_of(next), Ordering::Relaxed);
        Ok(Some(index))
    }

    /// Take up to [`CHAIN_MAX`] slots off the free list in one
    /// compare-and-swap, cut the last one's link, and return the first,
    /// encoded, or 0 when the list is empty. Every push and pop steps the
    /// counter beside the list's top, so a swap that succeeds saw no other
    /// thread move the slots it walked.
    fn pop_chain(&self) -> io::Result<u32> {
        let h = self.header();
        loop {
            let top = h.free.load(Ordering::Acquire);
            let first = stored_of(top);
            if first == 0 {
                return Ok(0);
            }
            let mut last = dec(first);
            let mut after = self.slot_header(last)?.link.load(Ordering::Acquire);
            for _ in 1..CHAIN_MAX {
                if stored_of(after) == 0 {
                    break;
                }
                last = dec(stored_of(after));
                after = self.slot_header(last)?.link.load(Ordering::Acquire);
            }
            let popped = pack(counter_of(top).wrapping_add(1), stored_of(after));
            if h.free.compare_exchange(top, popped, Ordering::AcqRel, Ordering::Acquire).is_ok() {
                let link = &self.slot_header(last)?.link;
                link.store(pack(counter_of(after).wrapping_add(1), 0), Ordering::Release);
                return Ok(first);
            }
        }
    }

    /// Pop a slot off the shared free list, or `None` when it is empty.
    fn pop_free(&self) -> io::Result<Option<u32>> {
        let h = self.header();
        loop {
            let top = h.free.load(Ordering::Acquire);
            if stored_of(top) == 0 {
                return Ok(None);
            }
            let index = dec(stored_of(top));
            let next = self.slot_header(index)?.link.load(Ordering::Acquire);
            let popped = pack(counter_of(top).wrapping_add(1), stored_of(next));
            if h.free.compare_exchange(top, popped, Ordering::AcqRel, Ordering::Acquire).is_ok() {
                return Ok(Some(index));
            }
        }
    }

    /// Put a slot no other thread can have seen back on the free list.
    pub(crate) fn free_unseen(&self, index: u32) -> io::Result<()> {
        let h = self.header();
        let link = &self.slot_header(index)?.link;
        loop {
            let top = h.free.load(Ordering::Acquire);
            let was = link.load(Ordering::Acquire);
            link.store(pack(counter_of(was).wrapping_add(1), stored_of(top)), Ordering::Release);
            let pushed = pack(counter_of(top).wrapping_add(1), enc(index));
            if h.free.compare_exchange(top, pushed, Ordering::AcqRel, Ordering::Acquire).is_ok() {
                return Ok(());
            }
        }
    }

    /// Move a queue word from `from` to `to`. False when another thread
    /// moved it first, which leaves it where this call was taking it.
    fn swing(word: &AtomicU64, from: u64, to: u64) -> bool {
        word.compare_exchange(from, to, Ordering::AcqRel, Ordering::Acquire).is_ok()
    }

    /// Retire a slot readers may still be on, stamped with `stamp`, the
    /// published epoch once the slot could no longer be reached. It is
    /// reused only after [`reclaim`](Self::reclaim) is given a horizon past
    /// the stamp.
    pub(crate) fn retire(&self, index: u32, stamp: Epoch) -> io::Result<()> {
        let h = self.header();
        let mine = self.slot_header(index)?;
        mine.stamp.store(stamp, Ordering::Relaxed);
        let was = mine.link.load(Ordering::Acquire);
        mine.link.store(pack(counter_of(was).wrapping_add(1), 0), Ordering::Release);
        loop {
            let tail = h.queue_tail.load(Ordering::Acquire);
            let last = self.slot_header(dec(stored_of(tail)))?;
            let next = last.link.load(Ordering::Acquire);
            if tail != h.queue_tail.load(Ordering::Acquire) {
                continue;
            }
            if stored_of(next) != 0 {
                Self::swing(&h.queue_tail, tail, pack(counter_of(tail).wrapping_add(1), stored_of(next)));
                continue;
            }
            let linked = pack(counter_of(next).wrapping_add(1), enc(index));
            if last.link.compare_exchange(next, linked, Ordering::AcqRel, Ordering::Acquire).is_ok() {
                Self::swing(&h.queue_tail, tail, pack(counter_of(tail).wrapping_add(1), enc(index)));
                return Ok(());
            }
        }
    }

    /// Retire `slots`, in order, as [`retire`](Self::retire) would one by
    /// one, all stamped `stamp`: linked to each other first and joined to
    /// the queue's tail with one swap, so writers retiring at once meet on
    /// the tail once per run of slots rather than once per slot.
    pub(crate) fn retire_all(&self, slots: &[u32], stamp: Epoch) -> io::Result<()> {
        let (Some(&first), Some(&last)) = (slots.first(), slots.last()) else {
            return Ok(());
        };
        for pair in slots.windows(2) {
            let mine = self.slot_header(pair[0])?;
            mine.stamp.store(stamp, Ordering::Relaxed);
            let was = mine.link.load(Ordering::Acquire);
            mine.link.store(pack(counter_of(was).wrapping_add(1), enc(pair[1])), Ordering::Release);
        }
        let end = self.slot_header(last)?;
        end.stamp.store(stamp, Ordering::Relaxed);
        let was = end.link.load(Ordering::Acquire);
        end.link.store(pack(counter_of(was).wrapping_add(1), 0), Ordering::Release);
        let h = self.header();
        loop {
            let tail = h.queue_tail.load(Ordering::Acquire);
            let at = self.slot_header(dec(stored_of(tail)))?;
            let next = at.link.load(Ordering::Acquire);
            if tail != h.queue_tail.load(Ordering::Acquire) {
                continue;
            }
            if stored_of(next) != 0 {
                Self::swing(&h.queue_tail, tail, pack(counter_of(tail).wrapping_add(1), stored_of(next)));
                continue;
            }
            let linked = pack(counter_of(next).wrapping_add(1), enc(first));
            if at.link.compare_exchange(next, linked, Ordering::AcqRel, Ordering::Acquire).is_ok() {
                Self::swing(&h.queue_tail, tail, pack(counter_of(tail).wrapping_add(1), enc(last)));
                return Ok(());
            }
        }
    }

    /// Move every retired slot stamped below `horizon` to the free list,
    /// oldest first, stopping at the first that is not. Returns how many
    /// moved.
    pub(crate) fn reclaim(&self, horizon: Epoch) -> io::Result<usize> {
        let h = self.header();
        let mut moved = 0usize;
        loop {
            let head = h.queue_head.load(Ordering::Acquire);
            let tail = h.queue_tail.load(Ordering::Acquire);
            let dummy = dec(stored_of(head));
            let next = self.slot_header(dummy)?.link.load(Ordering::Acquire);
            if head != h.queue_head.load(Ordering::Acquire) {
                continue;
            }
            if stored_of(next) == 0 {
                return Ok(moved);
            }
            if stored_of(head) == stored_of(tail) {
                Self::swing(&h.queue_tail, tail, pack(counter_of(tail).wrapping_add(1), stored_of(next)));
                continue;
            }
            let first = self.slot_header(dec(stored_of(next)))?;
            if first.stamp.load(Ordering::Acquire) >= horizon {
                return Ok(moved);
            }
            // The first retired slot becomes the dummy; the old dummy, retired
            // and past the horizon when it became the dummy, is free.
            if Self::swing(&h.queue_head, head, pack(counter_of(head).wrapping_add(1), stored_of(next))) {
                self.free_unseen(dummy)?;
                moved += 1;
            }
        }
    }

    /// Whether any retired slot waits in the queue.
    #[cfg(test)]
    fn has_retired(&self) -> io::Result<bool> {
        let head = self.header().queue_head.load(Ordering::Acquire);
        let dummy = dec(stored_of(head));
        Ok(stored_of(self.slot_header(dummy)?.link.load(Ordering::Acquire)) != 0)
    }
}

impl Drop for ChunkPool {
    fn drop(&mut self) {
        for map in self.maps.iter() {
            let mapped = map.swap(null_mut(), Ordering::AcqRel);
            if !mapped.is_null() {
                // SAFETY: installed by `chunk_base` from `Box::into_raw` and
                // never freed elsewhere.
                drop(unsafe { Box::from_raw(mapped) });
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;
    use std::sync::atomic::AtomicBool;

    const MAIN_MAGIC: u64 = 0x5045_5354_504F_4F4C;

    /// A main file holding one pool header, as a structure would lay it
    /// out: the pool's header after an 8-byte magic padded to 64.
    struct Main {
        mmap: MmapMut,
        _file: File,
        path: PathBuf,
    }

    fn tmp(name: &str) -> PathBuf {
        let mut p = std::env::temp_dir();
        p.push(format!("subetha-chunk-pool-{name}-{}.bin", std::process::id()));
        remove_family(&p);
        p
    }

    /// Remove the main file and every chunk file beside it, so a test
    /// starts from nothing and leaves nothing.
    fn remove_family(main: &Path) {
        let dir = main.parent().expect("the path has a directory");
        let stem = main.file_name().expect("the path names a file").to_string_lossy().to_string();
        for entry in std::fs::read_dir(dir).expect("the temp directory lists") {
            let entry = entry.expect("the entry reads");
            let name = entry.file_name().to_string_lossy().to_string();
            if name == stem || name.starts_with(&format!("{stem}.")) {
                match std::fs::remove_file(entry.path()) {
                    Ok(()) => {}
                    Err(e) if e.kind() == io::ErrorKind::NotFound => {}
                    Err(e) => panic!("{} not removed: {e}", entry.path().display()),
                }
            }
        }
    }

    fn main_file(path: &Path, slot_size: usize, chunk_bits: u32) -> Main {
        let (file, mmap) = crate::mmf_attach::create_or_attach(
            path,
            128,
            |ptr| unsafe {
                PoolHeader::init(ptr.add(64) as *mut PoolHeader, slot_size, chunk_bits);
                std::ptr::write_volatile(ptr as *mut u64, MAIN_MAGIC);
            },
            |ptr| unsafe { std::ptr::read_volatile(ptr as *const u64) },
            MAIN_MAGIC,
        )
        .expect("the main file is made or attached");
        Main { mmap, _file: file, path: path.to_path_buf() }
    }

    /// Writers each test pool serves, one run apiece.
    const WRITERS: usize = 8;

    fn pool(main: &Main, max_chunks: usize) -> ChunkPool {
        let header = unsafe { main.mmap.as_ptr().add(64) as *mut PoolHeader };
        unsafe { ChunkPool::attach(header, &main.path, "t", max_chunks, WRITERS) }.expect("the pool attaches")
    }

    #[test]
    fn slots_cross_chunks_and_each_chunk_is_a_file() {
        let path = tmp("cross");
        let main = main_file(&path, slot_size_for(8, 8).expect("an eight-byte body fits a slot"), 2);
        let p = pool(&main, 4);
        assert_eq!(p.capacity(), 16);
        let mut got = Vec::new();
        while let Some(i) = p.alloc(0).expect("alloc") {
            unsafe { std::ptr::write(p.body(i, body_offset(8)).expect("body") as *mut u64, u64::from(i) * 3) };
            got.push(i);
        }
        assert_eq!(got, (1..16).collect::<Vec<u32>>(), "slot 0 is the queue's dummy, the rest in order");
        for chunk in 0..4 {
            assert!(p.chunk_path(chunk).expect("path").exists(), "chunk {chunk} is a file");
        }
        for &i in &got {
            let v = unsafe { std::ptr::read(p.body(i, body_offset(8)).expect("body") as *const u64) };
            assert_eq!(v, u64::from(i) * 3, "slot {i} keeps what was written");
        }
        drop(p);
        drop(main);
        remove_family(&path);
    }

    #[test]
    fn a_second_view_reads_what_the_first_wrote_and_shares_its_cursor() {
        let path = tmp("views");
        let main = main_file(&path, slot_size_for(8, 8).expect("an eight-byte body fits a slot"), 3);
        let a = pool(&main, 8);
        let i = a.alloc(0).expect("alloc").expect("a slot");
        unsafe { std::ptr::write(a.body(i, body_offset(8)).expect("body") as *mut u64, 0xFEED) };
        let other = main_file(&path, slot_size_for(8, 8).expect("an eight-byte body fits a slot"), 3);
        let b = pool(&other, 8);
        let v = unsafe { std::ptr::read(b.body(i, body_offset(8)).expect("body") as *const u64) };
        assert_eq!(v, 0xFEED);
        let j = b.alloc(0).expect("alloc").expect("a slot");
        assert_ne!(i, j, "both views draw from one cursor");
        drop((a, b, main, other));
        remove_family(&path);
    }

    /// A writer hands out what is left of its run first, then a slot freed
    /// unseen, before it claims a fresh run from the bump cursor.
    #[test]
    fn a_slot_freed_unseen_is_handed_out_once_the_writers_run_is_spent() {
        let path = tmp("free");
        let main = main_file(&path, slot_size_for(8, 8).expect("an eight-byte body fits a slot"), 4);
        let p = pool(&main, 2);
        // A run of one, then a run of two whose second slot stays reserved.
        let i = p.alloc(0).expect("alloc").expect("a slot");
        let j = p.alloc(0).expect("alloc").expect("a slot");
        assert_eq!((i, j), (1, 2), "slot 0 is the queue's dummy; runs of one then two");
        p.free_unseen(i).expect("free");
        assert_eq!(p.alloc(0).expect("alloc"), Some(3), "the rest of the run comes first");
        assert_eq!(p.alloc(0).expect("alloc"), Some(i), "then the slot freed unseen, before a fresh run");
        drop((p, main));
        remove_family(&path);
    }

    /// A writer whose run is spent takes up to `CHAIN_MAX` slots off the
    /// free list in one swap and hands them out itself; the list keeps the
    /// rest, cut cleanly where the chain ends.
    #[test]
    fn a_writer_takes_free_slots_a_chain_at_a_time() {
        let path = tmp("chain");
        let main = main_file(&path, slot_size_for(8, 8).expect("an eight-byte body fits a slot"), 10);
        let p = pool(&main, 4);
        let freed: Vec<u32> = (0..100).map(|_| p.alloc(0).expect("alloc").expect("a slot")).collect();
        for &i in &freed {
            p.free_unseen(i).expect("free");
        }
        let freed: HashSet<u32> = freed.into_iter().collect();
        let chained: HashSet<u32> =
            (0..CHAIN_MAX).map(|_| p.alloc(1).expect("alloc").expect("a slot")).collect();
        assert_eq!(chained.len(), CHAIN_MAX as usize, "the chain hands each slot out once");
        assert!(chained.is_subset(&freed), "the chain comes off the free list");
        // A writer outside the run table pops the list one slot at a time:
        // exactly the slots the chain left, then a fresh one.
        let rest: HashSet<u32> = (0..100 - CHAIN_MAX).map(|_| p.alloc(WRITERS).expect("alloc").expect("a slot")).collect();
        assert_eq!(rest.len(), (100 - CHAIN_MAX) as usize);
        assert!(rest.is_subset(&freed) && rest.is_disjoint(&chained), "the list kept the rest");
        let fresh = p.alloc(WRITERS).expect("alloc").expect("a slot");
        assert!(!freed.contains(&fresh), "with the list empty the bump cursor gives slot {fresh}");
        drop((p, main));
        remove_family(&path);
    }

    #[test]
    fn a_retired_slot_waits_until_the_horizon_is_past_its_stamp() {
        let path = tmp("retire");
        let main = main_file(&path, slot_size_for(8, 8).expect("an eight-byte body fits a slot"), 4);
        let p = pool(&main, 2);
        let a = p.alloc(0).expect("alloc").expect("a slot");
        let b = p.alloc(0).expect("alloc").expect("a slot");
        unsafe { std::ptr::write(p.body(a, body_offset(8)).expect("body") as *mut u64, 77) };
        p.retire(a, 10).expect("retire a");
        p.retire(b, 12).expect("retire b");
        let still = unsafe { std::ptr::read(p.body(a, body_offset(8)).expect("body") as *const u64) };
        assert_eq!(still, 77, "retiring leaves the structure's bytes for readers still on it");
        assert_eq!(p.reclaim(10).expect("reclaim"), 0, "a horizon at the stamp frees nothing");
        assert!(p.has_retired().expect("queue"));
        assert_eq!(p.reclaim(11).expect("reclaim"), 1, "past a's stamp: the dummy before a goes free");
        assert_eq!(p.reclaim(13).expect("reclaim"), 1, "past b's stamp: a, now the dummy, goes free");
        assert!(!p.has_retired().expect("queue"), "b is the dummy and the queue is empty");
        let mut back = HashSet::new();
        for _ in 0..2 {
            back.insert(p.alloc(0).expect("alloc").expect("a slot"));
        }
        assert!(back.contains(&a), "a is handed out again: {back:?}");
        drop((p, main));
        remove_family(&path);
    }

    /// A run retired at once joins the queue in order with one stamp, and
    /// reclaims as single retirements would: everything before the last
    /// slot, which stays as the queue's dummy.
    #[test]
    fn a_run_retired_at_once_reclaims_as_single_retirements_would() {
        let path = tmp("retire-all");
        let main = main_file(&path, slot_size_for(8, 8).expect("an eight-byte body fits a slot"), 4);
        let p = pool(&main, 2);
        let run: Vec<u32> = (0..5).map(|_| p.alloc(0).expect("alloc").expect("a slot")).collect();
        p.retire_all(&run, 10).expect("retire the run");
        assert_eq!(p.reclaim(10).expect("reclaim"), 0, "a horizon at the stamp frees nothing");
        assert_eq!(p.reclaim(11).expect("reclaim"), 5, "the old dummy and all but the last of the run go free");
        assert!(!p.has_retired().expect("queue"), "the run's last slot is the dummy and the queue is empty");
        drop((p, main));
        remove_family(&path);
    }

    /// Threads allocating, retiring and reclaiming at once never hold one
    /// slot twice: each marks a slot it takes and clears the mark before
    /// giving it back, and a slot found already marked is a double grant.
    #[test]
    fn racing_threads_never_hold_one_slot_twice() {
        let path = tmp("race");
        let main = main_file(&path, slot_size_for(8, 8).expect("an eight-byte body fits a slot"), 6);
        let p = pool(&main, 64);
        let held: Vec<AtomicBool> = (0..p.capacity()).map(|_| AtomicBool::new(false)).collect();
        let epoch = AtomicU64::new(1);
        std::thread::scope(|s| {
            for t in 0..8u64 {
                let (p, held, epoch) = (&p, &held, &epoch);
                s.spawn(move || {
                    let mut mine = Vec::new();
                    for round in 0..4000u64 {
                        if let Some(i) = p.alloc(t as usize).expect("alloc") {
                            assert!(
                                !held[i as usize].swap(true, Ordering::AcqRel),
                                "slot {i} granted twice (thread {t}, round {round})"
                            );
                            mine.push(i);
                        }
                        if mine.len() > 16 {
                            let i = mine.remove(0);
                            held[i as usize].store(false, Ordering::Release);
                            if round % 2 == 0 {
                                p.free_unseen(i).expect("free");
                            } else {
                                p.retire(i, epoch.fetch_add(1, Ordering::AcqRel)).expect("retire");
                            }
                        }
                        if round % 64 == 0 {
                            p.reclaim(epoch.load(Ordering::Acquire).saturating_sub(8)).expect("reclaim");
                        }
                    }
                    for i in mine {
                        held[i as usize].store(false, Ordering::Release);
                        p.free_unseen(i).expect("free");
                    }
                });
            }
        });
        drop((p, main));
        remove_family(&path);
    }

    /// Threads that all pass one writer outside the run table share no run,
    /// so they never hold one slot twice either.
    #[test]
    fn writers_outside_the_run_table_never_hold_one_slot_twice() {
        let path = tmp("unrun");
        let main = main_file(&path, slot_size_for(8, 8).expect("an eight-byte body fits a slot"), 6);
        let p = pool(&main, 64);
        let held: Vec<AtomicBool> = (0..p.capacity()).map(|_| AtomicBool::new(false)).collect();
        std::thread::scope(|s| {
            for t in 0..8u64 {
                let (p, held) = (&p, &held);
                s.spawn(move || {
                    let mut mine = Vec::new();
                    for round in 0..4000u64 {
                        if let Some(i) = p.alloc(WRITERS).expect("alloc") {
                            assert!(
                                !held[i as usize].swap(true, Ordering::AcqRel),
                                "slot {i} granted twice (thread {t}, round {round})"
                            );
                            mine.push(i);
                        }
                        if mine.len() > 16 {
                            let i = mine.remove(0);
                            held[i as usize].store(false, Ordering::Release);
                            p.free_unseen(i).expect("free");
                        }
                    }
                    for i in mine {
                        held[i as usize].store(false, Ordering::Release);
                        p.free_unseen(i).expect("free");
                    }
                });
            }
        });
        drop((p, main));
        remove_family(&path);
    }
}
