//! `SharedBroadcastRing` - single-producer, multi-consumer pub/sub
//! ring backed by an MMF.
//!
//! Distinct from [`SharedRing`](crate::SharedRing) (MPMC; each slot
//! consumed exactly once). In a broadcast ring, every registered
//! consumer sees every message independently with its own cursor.
//! This is the pub/sub / kafka-topic / log-tail shape.
//!
//! # Layout
//!
//! ```text
//! +---------------------------+
//! | BroadcastHeader (128B)    |
//! |   magic, capacity         |
//! |   producer_seq: AtomicU64 |
//! |   consumer_seqs[MAX]:     |
//! |     [AtomicU64; MAX_CONS] |
//! |   consumer_active[MAX]:   |
//! |     [AtomicU32; MAX_CONS] |
//! +---------------------------+
//! | Slot[0]    (64B)          |  stamp + payload
//! | ...                       |
//! | Slot[capacity - 1]        |
//! +---------------------------+
//! ```
//!
//! The producer writes only a slot no counted consumer still has to read:
//! it clears the slot's stamp, writes the payload, stamps the slot with the
//! message's sequence number plus one, and publishes by advancing
//! `producer_seq`. A consumer checks the stamp before and after copying the
//! payload; any other stamp means the producer has written the slot again,
//! which it does only for a consumer it is not counting. No consumer waits
//! on the producer, and a producer that stops part way through a write
//! leaves a slot no counted consumer reads until the next producer writes
//! it whole.
//!
//! # Protocol
//!
//! ## Producer
//!
//! `try_push(payload)`:
//! 1. Find oldest consumer cursor: `min_consumer = min(active
//!    consumer_seqs)`.
//! 2. If `producer_seq - min_consumer >= capacity`, the ring is
//!    full from at least one consumer's perspective: return
//!    `BroadcastFull`. (A slot in `[min_consumer, producer_seq)`
//!    can't be overwritten without making that consumer miss it.)
//! 3. Clear the stamp of `slot[producer_seq % capacity]`; after a release
//!    fence write the payload, each 8-byte element whole; then store the
//!    stamp `producer_seq + 1` with Release.
//! 4. `producer_seq.fetch_add(1, Release)`.
//!
//! ## Consumer
//!
//! `register()` -> consumer_idx in 0..MAX_CONSUMERS:
//! - CAS the first inactive slot to active; initialize
//!   `consumer_seqs[i]` to current `producer_seq` (so the consumer
//!   starts from "now," not from the beginning of history).
//!
//! `try_recv(consumer_idx, buf)`:
//! 1. Load `my_seq = consumer_seqs[consumer_idx]`.
//! 2. If `my_seq >= producer_seq`, no new messages: return Empty.
//! 3. Check that `slot[my_seq % capacity]`'s stamp is `my_seq + 1`, copy
//!    its payload into `buf`, each 8-byte element whole, and check the
//!    stamp again after an acquire fence. Any other stamp is a later write,
//!    which the producer makes only for a consumer it is not counting:
//!    return InvalidConsumer.
//! 4. `consumer_seqs[consumer_idx].fetch_add(1, Release)`.
//!
//! `unregister(consumer_idx)`: mark slot inactive; producer no
//! longer waits for this consumer's cursor.
//!
//! # Why single-producer?
//!
//! Multi-producer broadcast adds complexity (producers must
//! coordinate slot claim and ordering must be preserved per topic).
//! The single-producer case covers most pub/sub use cases: one
//! source, many subscribers. For multi-producer fan-in, the
//! producers should fan into a SharedRing first, then a single
//! relay process re-emits into a SharedBroadcastRing.

use std::fs::File;
use std::mem::size_of;
use std::path::Path;
use std::sync::atomic::{fence, AtomicU32, AtomicU64, Ordering};
use std::time::{Duration, Instant};

use memmap2::{MmapMut, MmapOptions};

pub const BROADCAST_MAGIC: u32 = 0x4150_4232;
pub const BROADCAST_PAYLOAD_BYTES: usize = 52;
pub const MAX_CONSUMERS: usize = 16;
/// A consumer slot claimed by a registration that has not yet stored its
/// cursor: the producer holds back for it, and nobody counts it.
pub const CONSUMER_REGISTERING: u32 = 1;
/// A consumer slot whose cursor is in place: a registered consumer.
pub const CONSUMER_REGISTERED: u32 = 2;

/// `try_recv`'s error for an index that names no registered consumer. Cold
/// and out of line, so the registered path is the one laid out straight.
#[cold]
#[inline(never)]
fn not_a_consumer() -> BroadcastError {
    BroadcastError::InvalidConsumer
}

/// `try_lag`'s result for an index that names no registered consumer. Cold
/// and out of line, as [`not_a_consumer`].
#[cold]
#[inline(never)]
fn no_lag() -> Option<u64> {
    None
}

/// Header is two cache lines so the producer_seq and the consumer
/// seq array can sit on separate lines, reducing false-sharing
/// between producer-side and consumer-side hot atomics.
#[repr(C, align(64))]
pub struct BroadcastHeader {
    pub magic: u32,
    pub capacity: u32,
    pub producer_seq: AtomicU64,
    _pad1: [u8; 48],

    pub consumer_seqs: [AtomicU64; MAX_CONSUMERS],
    /// Zero while a consumer slot is free, then [`CONSUMER_REGISTERING`]
    /// and [`CONSUMER_REGISTERED`].
    pub consumer_active: [AtomicU32; MAX_CONSUMERS],
}

/// One message: the sequence number it was published as, plus one, or zero
/// while the producer writes it; then the payload, 8-byte aligned, whose
/// last element reads whole into the padding after it.
#[repr(C, align(64))]
pub struct BroadcastSlot {
    pub stamp: AtomicU64,
    pub payload: [u8; BROADCAST_PAYLOAD_BYTES],
    _pad: [u8; 56 - BROADCAST_PAYLOAD_BYTES],
}

/// A slot's payload as a consumer copies it: whole 8-byte elements, the
/// last reaching into the slot's padding.
const COPY_BYTES: usize = BROADCAST_PAYLOAD_BYTES.next_multiple_of(8);

const _: () = {
    assert!(size_of::<BroadcastSlot>() == 64);
    assert!(std::mem::offset_of!(BroadcastSlot, payload) == 8);
    assert!(std::mem::offset_of!(BroadcastSlot, payload) + COPY_BYTES <= size_of::<BroadcastSlot>());
};

pub const fn broadcast_file_size(capacity: usize) -> usize {
    size_of::<BroadcastHeader>() + capacity * size_of::<BroadcastSlot>()
}

#[derive(Debug)]
pub enum BroadcastError {
    Full,
    Empty,
    NoConsumerSlot,
    InvalidConsumer,
    PayloadTooLarge,
    LayoutMismatch(crate::layout::Mismatch),
    /// The operating system refused to open, size or map the backing; the
    /// error carries its code and message.
    IoError(std::io::Error),
}

impl std::fmt::Display for BroadcastError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Full => write!(f, "the slowest consumer is a full ring behind"),
            Self::Empty => write!(f, "nothing new for this consumer"),
            Self::NoConsumerSlot => write!(f, "every consumer slot is taken"),
            Self::InvalidConsumer => write!(f, "the consumer index names no registered consumer"),
            Self::PayloadTooLarge => write!(f, "the payload is larger than a slot"),
            Self::LayoutMismatch(m) => write!(f, "a ring of another shape: {m}"),
            Self::IoError(e) => write!(f, "io: {e}"),
        }
    }
}

impl std::error::Error for BroadcastError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::IoError(e) => Some(e),
            Self::Full
            | Self::Empty
            | Self::NoConsumerSlot
            | Self::InvalidConsumer
            | Self::PayloadTooLarge
            | Self::LayoutMismatch(_) => None,
        }
    }
}

/// The first of the header's magic and capacity that differs from a ring
/// of `capacity` slots.
fn header_matches(hdr: &BroadcastHeader, capacity: usize) -> Result<(), BroadcastError> {
    crate::layout::Mismatch::first(&[
        ("magic", u64::from(hdr.magic), u64::from(BROADCAST_MAGIC)),
        ("capacity", u64::from(hdr.capacity), capacity as u64),
    ])
    .map_err(BroadcastError::LayoutMismatch)
}

/// A backing of `len` bytes, refused when it is shorter than `wanted`.
fn long_enough(len: u64, wanted: usize) -> Result<(), BroadcastError> {
    if len < wanted as u64 {
        return Err(BroadcastError::LayoutMismatch(crate::layout::Mismatch::new(
            "length",
            len,
            wanted as u64,
        )));
    }
    Ok(())
}

impl From<std::io::Error> for BroadcastError {
    fn from(e: std::io::Error) -> Self { Self::IoError(e) }
}

pub struct SharedBroadcastRing {
    _backing: BroadcastBacking,
    raw_ptr: *mut u8,
    capacity: usize,
    header_sidecar: subetha_core::HandshakeHeader,
    ring_sidecar: Box<subetha_core::ObservationRing>,
}

unsafe impl Send for SharedBroadcastRing {}
unsafe impl Sync for SharedBroadcastRing {}

/// Storage backing for a `SharedBroadcastRing`. The hot-path
/// pointer (`raw_ptr` on `SharedBroadcastRing`) is cached at
/// construction; this enum holds the owning resource alive for
/// the lifetime of the ring. The held values are intentionally
/// only kept for ownership / flush-target purposes; the
/// `File` and `ShmFile` payloads are not read at runtime.
#[allow(dead_code)]
enum BroadcastBacking {
    /// In-process anonymous mmap. No file, no shm-name.
    Anon(MmapMut),
    /// File-backed mmap. Cross-process via the OS page cache.
    File(File, MmapMut),
    /// Named shared-memory mmap (ShmFs locale). Cross-process,
    /// RAM-resident, never touches the page cache.
    Shm(crate::shm_file::ShmFile),
}

/// Backing-agnostic layout init. Writes the header magic + capacity
/// and zeroes the slot array at the given raw pointer. Caller
/// guarantees that `ptr` points to at least
/// `broadcast_file_size(capacity)` bytes of mutable, suitably-
/// aligned memory.
unsafe fn init_broadcast_layout_raw(ptr: *mut u8, capacity: usize) {
    let hdr_ptr = ptr as *mut BroadcastHeader;
    unsafe {
        std::ptr::write_bytes(hdr_ptr as *mut u8, 0, size_of::<BroadcastHeader>());
        (*hdr_ptr).magic = BROADCAST_MAGIC;
        (*hdr_ptr).capacity = capacity as u32;
        std::ptr::write_bytes(ptr.add(size_of::<BroadcastHeader>()), 0, capacity * size_of::<BroadcastSlot>());
    }
}

/// Layout init for the elected creator in the attach flow: capacity
/// first, magic last via a volatile store, because attachers spin on
/// the magic. The zeroed region is already the empty slot array, so the
/// slots are not touched.
///
/// # Safety
/// `ptr` addresses at least `broadcast_file_size(capacity)` writable
/// zeroed bytes.
unsafe fn init_broadcast_attach_raw(ptr: *mut u8, capacity: usize) {
    let hdr_ptr = ptr as *mut BroadcastHeader;
    unsafe {
        (*hdr_ptr).capacity = capacity as u32;
        std::ptr::write_volatile(&raw mut (*hdr_ptr).magic, BROADCAST_MAGIC);
    }
}

impl subetha_sidecar::AdaptiveInstance for SharedBroadcastRing {
    fn header(&self) -> &subetha_core::HandshakeHeader { &self.header_sidecar }
    fn ring(&self) -> &subetha_core::ObservationRing { &self.ring_sidecar }
    fn make_policy(&self) -> Box<dyn subetha_sidecar::Policy> {
        Box::new(subetha_sidecar::NoMigrationPolicy)
    }
}

impl SharedBroadcastRing {
    /// File-backed broadcast ring; cross-process visibility via the OS
    /// page cache. Obtains the ring at `path`: it initializes an empty
    /// one only when the path does not yet exist, and otherwise
    /// attaches with published slots intact. A region
    /// built with a different capacity is a `LayoutMismatch`.
    /// [`reset`](Self::reset) reinitializes.
    pub fn create(path: impl AsRef<Path>, capacity: usize) -> Result<Self, BroadcastError> {
        assert!(capacity >= 2);
        assert!(capacity <= u32::MAX as usize);
        let total = broadcast_file_size(capacity);
        let (file, mut mmap) = crate::mmf_attach::create_or_attach(
            path.as_ref(),
            total,
            |ptr| unsafe { init_broadcast_attach_raw(ptr, capacity) },
            |ptr| unsafe { u64::from((*(ptr as *const BroadcastHeader)).magic) },
            u64::from(BROADCAST_MAGIC),
        )
        .map_err(|e| crate::mmf_attach::attach_error(e, BroadcastError::LayoutMismatch))?;
        let raw_ptr = mmap.as_mut_ptr();
        let hdr = unsafe { &*(raw_ptr as *const BroadcastHeader) };
        header_matches(hdr, capacity)?;
        Ok(Self {
            _backing: BroadcastBacking::File(file, mmap),
            raw_ptr, capacity,
            header_sidecar: subetha_core::HandshakeHeader::new(),
            ring_sidecar: Box::new(subetha_core::ObservationRing::new()),
        })
    }

    /// Truncate the ring at `path` and initialize an empty one,
    /// discarding every published slot live peers share. For a caller
    /// that knows it owns the path.
    pub fn reset(path: impl AsRef<Path>, capacity: usize) -> Result<Self, BroadcastError> {
        assert!(capacity >= 2);
        assert!(capacity <= u32::MAX as usize);
        let total = broadcast_file_size(capacity);
        let (file, mut mmap) = crate::mmf_attach::reset(path.as_ref(), total, |ptr| unsafe {
            init_broadcast_attach_raw(ptr, capacity)
        })?;
        let raw_ptr = mmap.as_mut_ptr();
        Ok(Self {
            _backing: BroadcastBacking::File(file, mmap),
            raw_ptr, capacity,
            header_sidecar: subetha_core::HandshakeHeader::new(),
            ring_sidecar: Box::new(subetha_core::ObservationRing::new()),
        })
    }

    /// Anonymous in-process broadcast ring. Fastest construction;
    /// skips file create + ftruncate + first-page-fault. In-process
    /// only - subscribers in other processes cannot connect.
    pub fn create_anon(capacity: usize) -> Result<Self, BroadcastError> {
        assert!(capacity >= 2);
        assert!(capacity <= u32::MAX as usize);
        let total = broadcast_file_size(capacity);
        let mut mmap = MmapOptions::new().len(total).map_anon()?;
        let raw_ptr = mmap.as_mut_ptr();
        unsafe { init_broadcast_layout_raw(raw_ptr, capacity); }
        Ok(Self {
            _backing: BroadcastBacking::Anon(mmap),
            raw_ptr, capacity,
            header_sidecar: subetha_core::HandshakeHeader::new(),
            ring_sidecar: Box::new(subetha_core::ObservationRing::new()),
        })
    }

    /// Open an existing file-backed broadcast ring. Validates
    /// magic + capacity.
    pub fn open(path: impl AsRef<Path>, expected_capacity: usize) -> Result<Self, BroadcastError> {
        let total = broadcast_file_size(expected_capacity);
        let file = crate::region_file::open_existing(path.as_ref())?;
        long_enough(file.metadata()?.len(), total)?;
        let mut mmap = unsafe { MmapOptions::new().len(total).map_mut(&file)? };
        let raw_ptr = mmap.as_mut_ptr();
        let hdr = unsafe { &*(raw_ptr as *const BroadcastHeader) };
        header_matches(hdr, expected_capacity)?;
        Ok(Self {
            _backing: BroadcastBacking::File(file, mmap),
            raw_ptr, capacity: expected_capacity,
            header_sidecar: subetha_core::HandshakeHeader::new(),
            ring_sidecar: Box::new(subetha_core::ObservationRing::new()),
        })
    }

    /// Build a fresh broadcast ring on top of a named RAM-resident
    /// shared-memory backing. Cross-process visible via the
    /// `logical_name` of the underlying
    /// [`ShmFile`](crate::shm_file::ShmFile); never touches
    /// the page cache. The `ShmFile` must be sized to at least
    /// `broadcast_file_size(capacity)` bytes.
    pub fn create_from_shm(
        mut shm: crate::shm_file::ShmFile,
        capacity: usize,
    ) -> Result<Self, BroadcastError> {
        assert!(capacity >= 2);
        assert!(capacity <= u32::MAX as usize);
        let total = broadcast_file_size(capacity);
        long_enough(shm.len() as u64, total)?;
        let raw_ptr = shm.as_mut_slice().as_mut_ptr();
        unsafe { init_broadcast_layout_raw(raw_ptr, capacity); }
        Ok(Self {
            _backing: BroadcastBacking::Shm(shm),
            raw_ptr, capacity,
            header_sidecar: subetha_core::HandshakeHeader::new(),
            ring_sidecar: Box::new(subetha_core::ObservationRing::new()),
        })
    }

    /// Open an existing named ShmFs-backed broadcast ring.
    /// Validates magic + capacity and leaves the layout as it found it - the
    /// layout - the layout must already be present from a prior
    /// `create_from_shm` on the same logical name.
    pub fn open_from_shm(
        mut shm: crate::shm_file::ShmFile,
        expected_capacity: usize,
    ) -> Result<Self, BroadcastError> {
        let total = broadcast_file_size(expected_capacity);
        long_enough(shm.len() as u64, total)?;
        let raw_ptr = shm.as_mut_slice().as_mut_ptr();
        let hdr = unsafe { &*(raw_ptr as *const BroadcastHeader) };
        header_matches(hdr, expected_capacity)?;
        Ok(Self {
            _backing: BroadcastBacking::Shm(shm),
            raw_ptr, capacity: expected_capacity,
            header_sidecar: subetha_core::HandshakeHeader::new(),
            ring_sidecar: Box::new(subetha_core::ObservationRing::new()),
        })
    }


    #[inline]
    pub fn capacity(&self) -> usize { self.capacity }

    fn header(&self) -> &BroadcastHeader {
        unsafe { &*(self.raw_ptr as *const BroadcastHeader) }
    }

    /// Reduce a logical sequence number to a physical slot index. Uses a
    /// bit-mask when capacity is a power of two (the common ring sizing) -
    /// removing the `% capacity` hardware DIV - and the modulo otherwise.
    /// `capacity` is loop-invariant, so the pow2 test folds away in hot
    /// loops; no cached field needed.
    #[inline]
    fn wrap(&self, i: usize) -> usize {
        if self.capacity.is_power_of_two() {
            i & (self.capacity - 1)
        } else {
            i % self.capacity
        }
    }

    /// The slot that sequence number `seq` lands in, 64-byte aligned.
    #[inline]
    fn slot_at(&self, seq: u64) -> *mut u8 {
        // SAFETY: the mapping holds the header and `capacity` slots, and
        // `wrap` names one of them.
        unsafe {
            self.raw_ptr
                .add(size_of::<BroadcastHeader>())
                .add(self.wrap(seq as usize) * size_of::<BroadcastSlot>())
        }
    }

    /// The stamp of `slot`, a slot of this ring.
    #[inline]
    fn stamp(&self, slot: *mut u8) -> &AtomicU64 {
        // SAFETY: a slot starts with its stamp, an aligned AtomicU64 that
        // the mapping keeps while the ring lives.
        unsafe { &*(slot as *const AtomicU64) }
    }

    /// Register as a consumer. Returns a consumer index in
    /// `0..MAX_CONSUMERS`; that index is used for all subsequent
    /// recv calls. Initializes the consumer's cursor to the current
    /// producer_seq (consumer starts reading from "now," not history).
    pub fn register_consumer(&self) -> Result<usize, BroadcastError> {
        for i in 0..MAX_CONSUMERS {
            if self.claim_consumer_slot(i) {
                return Ok(i);
            }
        }
        self.ring_sidecar
            .push_op(crate::sidecar_ops::broadcast_ring::OP_REGISTER, 1); // no consumer slot available
        Err(BroadcastError::NoConsumerSlot)
    }

    /// Claim the consumer slot at `idx` in particular, for a caller that
    /// hands out indices itself, with its cursor at the current producer
    /// position. Answers whether this call claimed it: a slot already held
    /// keeps its holder and its cursor, and an index past the table claims
    /// nothing. The slot is counted only once its cursor is in place, so a
    /// producer that waits for it publishes nothing it misses.
    pub(crate) fn claim_consumer_slot(&self, idx: usize) -> bool {
        if idx >= MAX_CONSUMERS {
            return false;
        }
        let hdr = self.header();
        let claimed = hdr.consumer_active[idx].compare_exchange(0, CONSUMER_REGISTERING, Ordering::AcqRel, Ordering::Acquire);
        if claimed.is_err() {
            return false;
        }
        #[cfg(test)]
        crate::test_races::pause_point();
        let cur_producer = hdr.producer_seq.load(Ordering::Acquire);
        hdr.consumer_seqs[idx].store(cur_producer, Ordering::Release);
        hdr.consumer_active[idx].store(CONSUMER_REGISTERED, Ordering::Release);
        self.ring_sidecar
            .push_op(crate::sidecar_ops::broadcast_ring::OP_REGISTER, 0);
        true
    }

    /// Unregister a consumer. After this, the producer no longer
    /// waits for this cursor when reclaiming slots.
    pub fn unregister_consumer(&self, consumer_idx: usize) {
        if consumer_idx >= MAX_CONSUMERS { return; }
        let hdr = self.header();
        hdr.consumer_active[consumer_idx].store(0, Ordering::Release);
        self.ring_sidecar
            .push_op(crate::sidecar_ops::broadcast_ring::OP_UNREGISTER, 0);
    }

    /// Compute the slot-reclaim safety margin: the producer can push
    /// when `producer_seq - min_active_consumer_seq < capacity`.
    /// Returns the smallest cursor among claimed consumer slots, one still
    /// registering among them so its cursor is never lapped (or
    /// producer_seq when there are none; broadcasts to nobody can always
    /// push).
    fn min_consumer_seq(&self) -> u64 {
        let hdr = self.header();
        let producer = hdr.producer_seq.load(Ordering::Acquire);
        let mut min = u64::MAX;
        let mut any = false;
        for i in 0..MAX_CONSUMERS {
            if hdr.consumer_active[i].load(Ordering::Acquire) != 0 {
                let s = hdr.consumer_seqs[i].load(Ordering::Acquire);
                if s < min { min = s; }
                any = true;
            }
        }
        if !any { producer } else { min }
    }

    /// Push a message. Returns `Err(Full)` when at least one active
    /// consumer hasn't yet read a previous slot we'd overwrite.
    pub fn try_push(&self, payload: &[u8]) -> Result<(), BroadcastError> {
        if payload.len() > BROADCAST_PAYLOAD_BYTES {
            return Err(BroadcastError::PayloadTooLarge);
        }
        let hdr = self.header();
        let producer = hdr.producer_seq.load(Ordering::Acquire);
        let min_consumer = self.min_consumer_seq();
        if producer.saturating_sub(min_consumer) >= self.capacity as u64 {
            self.ring_sidecar
                .push_op(crate::sidecar_ops::broadcast_ring::OP_PUSH, 1); // full
            return Err(BroadcastError::Full);
        }
        let slot = self.slot_at(producer);
        let stamp = self.stamp(slot);
        // The cleared stamp before the payload's stores: a consumer whose
        // copy sees any of them then finds a stamp other than its message's.
        stamp.store(0, Ordering::Relaxed);
        fence(Ordering::Release);
        // SAFETY: the payload is 8-byte aligned and holds the message, and
        // no counted consumer reads this slot until it is published.
        unsafe {
            let dst = slot.add(std::mem::offset_of!(BroadcastSlot, payload)) as *mut [u8; BROADCAST_PAYLOAD_BYTES];
            // A whole message is stored straight from the caller's bytes; a
            // shorter one is padded with zeros first.
            match payload.first_chunk::<BROADCAST_PAYLOAD_BYTES>() {
                Some(whole) => crate::atomic_copy::store(dst, whole),
                None => {
                    let mut message = [0u8; BROADCAST_PAYLOAD_BYTES];
                    message[..payload.len()].copy_from_slice(payload);
                    crate::atomic_copy::store(dst, &message);
                }
            }
        };
        stamp.store(producer + 1, Ordering::Release);
        hdr.producer_seq.fetch_add(1, Ordering::Release);
        self.ring_sidecar
            .push_op(crate::sidecar_ops::broadcast_ring::OP_PUSH, 0);
        Ok(())
    }

    /// Receive the next unread message for `consumer_idx`. Returns
    /// the number of bytes filled (always BROADCAST_PAYLOAD_BYTES;
    /// caller knows the inner-event size from its T contract).
    ///
    /// `InvalidConsumer` when the index names no registered consumer, and
    /// when the producer wrote this consumer's slot again during the copy,
    /// which it does only to a consumer it was not counting, one
    /// unregistered while it read; `out` then holds no message, and its
    /// bytes are unspecified.
    pub fn try_recv(&self, consumer_idx: usize, out: &mut [u8]) -> Result<usize, BroadcastError> {
        if consumer_idx >= MAX_CONSUMERS { return Err(BroadcastError::InvalidConsumer); }
        let hdr = self.header();
        if hdr.consumer_active[consumer_idx].load(Ordering::Acquire) != CONSUMER_REGISTERED {
            return Err(not_a_consumer());
        }
        let my_seq = hdr.consumer_seqs[consumer_idx].load(Ordering::Acquire);
        let producer = hdr.producer_seq.load(Ordering::Acquire);
        if my_seq >= producer {
            self.ring_sidecar
                .push_op(crate::sidecar_ops::broadcast_ring::OP_RECV, 2); // empty
            return Err(BroadcastError::Empty);
        }
        // The producer writes this slot again, clearing its stamp first, only
        // after every consumer it counts has read my_seq: a stamp other than
        // my_seq's, before the copy or after it, is a later write, for a
        // consumer it was not counting.
        let slot = self.slot_at(my_seq);
        let stamp = self.stamp(slot);
        if stamp.load(Ordering::Acquire) != my_seq + 1 {
            return Err(BroadcastError::InvalidConsumer);
        }
        #[cfg(test)]
        crate::test_races::pause_point();
        let n = out.len().min(BROADCAST_PAYLOAD_BYTES);
        // SAFETY: the payload is 8-byte aligned and the slot holds COPY_BYTES
        // of it; bytes the producer is writing read as some bytes, which the
        // stamp checked next discards, and any bytes are a [u8; COPY_BYTES].
        unsafe {
            let src = slot.add(std::mem::offset_of!(BroadcastSlot, payload));
            match out.first_chunk_mut::<BROADCAST_PAYLOAD_BYTES>() {
                // A buffer that holds a whole message takes it straight from
                // the slot, element by element.
                Some(whole) => {
                    const WORDS: usize = BROADCAST_PAYLOAD_BYTES & !7;
                    crate::atomic_copy::load_bytes(whole.as_mut_ptr(), src, WORDS);
                    let last = crate::atomic_copy::load_word(src.add(WORDS) as *const u64).to_ne_bytes();
                    whole[WORDS..].copy_from_slice(&last[..BROADCAST_PAYLOAD_BYTES - WORDS]);
                }
                None => {
                    let copy = crate::atomic_copy::load(src as *const [u8; COPY_BYTES]).assume_init();
                    out[..n].copy_from_slice(&copy[..n]);
                }
            }
        }
        fence(Ordering::Acquire);
        if stamp.load(Ordering::Relaxed) != my_seq + 1 {
            return Err(BroadcastError::InvalidConsumer);
        }
        hdr.consumer_seqs[consumer_idx].fetch_add(1, Ordering::Release);
        self.ring_sidecar
            .push_op(crate::sidecar_ops::broadcast_ring::OP_RECV, 0);
        Ok(n)
    }

    /// Number of messages this consumer has not yet read, or `None` when
    /// `consumer_idx` names no live consumer of this ring.
    ///
    /// Two things name no live consumer. An index past the consumer
    /// table is one. So is a slot nothing holds: `unregister_consumer`
    /// clears the active flag and leaves the cursor where its last
    /// holder stopped, so the distance from the producer to that cursor
    /// is a number about nobody, and it grows with every push.
    ///
    /// What this does not say, which is the thing callers reach for
    /// it to find out: a consumer registers at the head, so one that
    /// registered after everything was published reads zero here and has
    /// missed all of it. Zero means nothing is waiting, not that nothing
    /// was lost. `producer_position` at the moment of registration is
    /// what says how much a consumer will never see.
    pub fn try_lag(&self, consumer_idx: usize) -> Option<u64> {
        if consumer_idx >= MAX_CONSUMERS {
            return None;
        }
        let hdr = self.header();
        if hdr.consumer_active[consumer_idx].load(Ordering::Acquire) != CONSUMER_REGISTERED {
            return no_lag();
        }
        let prod = hdr.producer_seq.load(Ordering::Acquire);
        let my = hdr.consumer_seqs[consumer_idx].load(Ordering::Acquire);
        Some(prod.saturating_sub(my))
    }

    /// Number of messages this consumer has not yet read.
    ///
    /// `u64::MAX` when `consumer_idx` names no live consumer, because
    /// zero there would say "caught up" about something that is not a
    /// reader, which is the value a caller can act on least. `try_lag`
    /// says the same thing as an `Option`.
    pub fn lag(&self, consumer_idx: usize) -> u64 {
        self.try_lag(consumer_idx).unwrap_or(u64::MAX)
    }

    /// Current producer cursor (total messages pushed since creation).
    pub fn producer_position(&self) -> u64 {
        self.header().producer_seq.load(Ordering::Acquire)
    }

    /// Number of consumers registered with their cursors in place; one still
    /// registering is not counted.
    pub fn active_consumer_count(&self) -> usize {
        let hdr = self.header();
        (0..MAX_CONSUMERS)
            .filter(|&i| hdr.consumer_active[i].load(Ordering::Acquire) == CONSUMER_REGISTERED)
            .count()
    }

    /// Wait until `want` consumers have registered, and return how many
    /// are registered when the wait ends.
    ///
    /// A count below `want` means the timeout ran out first, and the
    /// number is the shortfall: it says how many of the readers a
    /// producer is about to publish for actually arrived.
    ///
    /// # Why a producer wants this
    ///
    /// A consumer registers at the head, so everything published before
    /// it registered is lost to it, and nothing reports that: the push
    /// succeeds, the ring does not error, and a reader that started late
    /// looks exactly like one that is slow. The window is however long
    /// registration takes, which across processes is however long
    /// starting one takes, tens to hundreds of milliseconds during which
    /// a producer can write a great deal.
    ///
    /// Publishing only once the readers are here closes that window.
    /// It is the one thing a producer can do about the loss, because
    /// after the fact there is nothing left to detect: `lag` reads zero
    /// for a consumer that missed everything.
    ///
    /// # What it does not promise
    ///
    /// That consumers do not leave afterward. Its count holds for the
    /// moment it returns, and a consumer registered then can unregister
    /// or its process can die immediately after. It is a starting gun,
    /// not a guarantee of attendance.
    #[must_use]
    pub fn wait_for_consumers(&self, want: usize, timeout: Duration) -> usize {
        let deadline = Instant::now() + timeout;
        // Registration is a startup event, so the first look usually
        // settles it and the rest of this never runs. Yielding before
        // sleeping keeps the common case off the clock, whose
        // granularity is a couple of hundred microseconds on some hosts
        // and would otherwise set a floor on how fast this can return.
        for _ in 0..64 {
            if self.active_consumer_count() >= want {
                return self.active_consumer_count();
            }
            std::thread::yield_now();
        }
        loop {
            let have = self.active_consumer_count();
            if have >= want || Instant::now() >= deadline {
                return have;
            }
            std::thread::sleep(Duration::from_micros(500));
        }
    }

    pub fn flush(&self) -> Result<(), BroadcastError> {
        match &self._backing {
            BroadcastBacking::File(_, mmap) => mmap.flush()?,
            BroadcastBacking::Anon(mmap) => mmap.flush()?,
            BroadcastBacking::Shm(_) => {} // RAM-resident; nothing to sync
        }
        Ok(())
    }

    /// Non-blocking flush: schedules a writeback via the OS.
    /// Note: Windows is only partially async (sync to page cache,
    /// not to disk).
    pub fn flush_async(&self) -> Result<(), BroadcastError> {
        match &self._backing {
            BroadcastBacking::File(_, mmap) => mmap.flush_async()?,
            BroadcastBacking::Anon(mmap) => mmap.flush_async()?,
            BroadcastBacking::Shm(_) => {} // RAM-resident; nothing to sync
        }
        Ok(())
    }

    /// Whether every currently-active consumer has read every
    /// item the producer has published. Used by the capacity-morph
    /// wrapper to decide whether a stale broadcast backing can be
    /// dropped (all subscribers have caught up to the frozen
    /// producer position).
    pub fn is_fully_drained(&self) -> bool {
        let hdr = self.header();
        let prod = hdr.producer_seq.load(Ordering::Acquire);
        for i in 0..MAX_CONSUMERS {
            if hdr.consumer_active[i].load(Ordering::Acquire) != 0
                && hdr.consumer_seqs[i].load(Ordering::Acquire) < prod
            {
                return false;
            }
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::thread;
    use std::time::Duration;

    /// A file for one test, removed when the test ends; declared before the
    /// primitive that maps it so it drops after it.
    fn tmp(name: &str) -> crate::test_paths::TmpFile {
        crate::test_paths::TmpFile::new(format!("subetha-broadcast-{name}-{}.bin", std::process::id()))
    }

    fn payload_of(v: u32) -> [u8; BROADCAST_PAYLOAD_BYTES] {
        let mut b = [0u8; BROADCAST_PAYLOAD_BYTES];
        b[0..4].copy_from_slice(&v.to_le_bytes());
        b
    }
    fn unpack(b: &[u8]) -> u32 {
        u32::from_le_bytes(b[0..4].try_into().unwrap())
    }

    #[test]
    fn create_initial_state() {
        let p = tmp("init");
        let r = SharedBroadcastRing::create(&p, 8).unwrap();
        assert_eq!(r.capacity(), 8);
        assert_eq!(r.producer_position(), 0);
        assert_eq!(r.active_consumer_count(), 0);
    }

    /// A registration stopped between claiming its slot and storing its
    /// cursor is not counted, so a producer that waits for it publishes
    /// nothing until it is, and the consumer reads every item published
    /// after.
    #[test]
    fn a_registration_is_counted_only_once_its_cursor_is_in_place() {
        let p = tmp("register-window");
        let ring = Arc::new(SharedBroadcastRing::create(&p, 16).unwrap());
        let consumer = Arc::clone(&ring);
        let (pause, registered) = crate::test_races::stopped(move || consumer.register_consumer());
        assert_eq!(ring.wait_for_consumers(1, Duration::from_millis(20)), 0, "a registration mid-claim is not counted");
        assert_eq!(ring.active_consumer_count(), 0);
        pause.release();
        let idx = registered.join().expect("the consumer's thread").expect("a consumer slot");
        assert_eq!(ring.wait_for_consumers(1, Duration::from_secs(5)), 1);
        for v in 0..8 {
            ring.try_push(&payload_of(v)).unwrap();
        }
        let mut out = [0u8; BROADCAST_PAYLOAD_BYTES];
        for v in 0..8 {
            ring.try_recv(idx, &mut out).unwrap();
            assert_eq!(unpack(&out), v, "every item published after the count is read");
        }
        drop(ring);
    }

    /// A write that stopped part way through, as a producer that died in
    /// the middle of one leaves it, holds up nobody: the consumer reads
    /// what was published, finds nothing more, and reads the next
    /// producer's write of that slot whole.
    #[test]
    fn a_write_stopped_part_way_through_leaves_no_consumer_waiting() {
        let p = tmp("stopped-write");
        let ring = SharedBroadcastRing::create(&p, 4).unwrap();
        let c = ring.register_consumer().unwrap();
        ring.try_push(&payload_of(1)).unwrap();
        // The slot the next push lands in, which nobody reads until it is
        // published: its stamp cleared and half its payload written.
        let slot = ring.slot_at(ring.producer_position());
        ring.stamp(slot).store(0, Ordering::Relaxed);
        // SAFETY: the payload lies inside the slot, and nobody reads it.
        unsafe { std::ptr::write_bytes(slot.add(std::mem::offset_of!(BroadcastSlot, payload)), 0xAB, BROADCAST_PAYLOAD_BYTES / 2) };
        let mut out = [0u8; BROADCAST_PAYLOAD_BYTES];
        ring.try_recv(c, &mut out).unwrap();
        assert_eq!(unpack(&out), 1);
        assert!(matches!(ring.try_recv(c, &mut out), Err(BroadcastError::Empty)), "the stopped write is not read");
        let next = SharedBroadcastRing::create(&p, 4).unwrap();
        next.try_push(&payload_of(2)).unwrap();
        ring.try_recv(c, &mut out).unwrap();
        assert_eq!(out, payload_of(2), "the next producer's write is read whole");
    }

    /// A ring a consumer has let fill reads that consumer's oldest message,
    /// and every one after it, in order.
    #[test]
    fn a_full_ring_reads_its_oldest_message() {
        let p = tmp("full");
        let ring = SharedBroadcastRing::create(&p, 4).unwrap();
        let c = ring.register_consumer().unwrap();
        for v in 0..4 {
            ring.try_push(&payload_of(v)).unwrap();
        }
        assert!(matches!(ring.try_push(&payload_of(4)), Err(BroadcastError::Full)));
        let mut out = [0u8; BROADCAST_PAYLOAD_BYTES];
        for v in 0..4 {
            ring.try_recv(c, &mut out).unwrap();
            assert_eq!(unpack(&out), v);
        }
    }

    /// A consumer unregistered while it reads, whose slot the producer
    /// writes again before the copy, is told it is no consumer rather than
    /// handed the later message as its own.
    #[test]
    fn a_read_whose_slot_is_written_again_is_refused() {
        let p = tmp("lapped");
        let ring = Arc::new(SharedBroadcastRing::create(&p, 4).unwrap());
        let c = ring.register_consumer().unwrap();
        ring.try_push(&payload_of(1)).unwrap();
        let reader = Arc::clone(&ring);
        let (pause, read) = crate::test_races::stopped(move || {
            let mut out = [0u8; BROADCAST_PAYLOAD_BYTES];
            reader.try_recv(c, &mut out).map(|_| unpack(&out))
        });
        ring.unregister_consumer(c);
        for v in 2..=5 {
            ring.try_push(&payload_of(v)).unwrap();
        }
        pause.release();
        let result = read.join().expect("the reader's thread");
        assert!(matches!(result, Err(BroadcastError::InvalidConsumer)), "the read of a slot written again returned {result:?}");
        drop(ring);
    }

    /// A second create attaches with published slots in place; reset
    /// is what strips them.
    #[test]
    fn second_create_attaches_and_keeps_slots() {
        let p = tmp("attach");
        let r = SharedBroadcastRing::create(&p, 8).unwrap();
        r.try_push(&payload_of(42)).unwrap();

        let r2 = SharedBroadcastRing::create(&p, 8).unwrap();
        assert_eq!(r2.producer_position(), 1, "attach rewound the producer");
        assert!(matches!(
            SharedBroadcastRing::create(&p, 4),
            Err(BroadcastError::LayoutMismatch(crate::layout::Mismatch { field: "capacity", found: 8, wanted: 4 })),
        ));

        // Windows refuses to truncate a mapped file, so every handle goes
        // before the reset.
        drop(r);
        drop(r2);
        let fresh = SharedBroadcastRing::reset(&p, 8).unwrap();
        assert_eq!(fresh.producer_position(), 0, "reset kept a published slot");
        drop(fresh);
    }

    #[test]
    fn one_producer_one_consumer_round_trip() {
        let p = tmp("1p1c");
        let r = SharedBroadcastRing::create(&p, 8).unwrap();
        let c = r.register_consumer().unwrap();
        r.try_push(&payload_of(42)).unwrap();
        let mut buf = [0u8; BROADCAST_PAYLOAD_BYTES];
        let n = r.try_recv(c, &mut buf).unwrap();
        assert_eq!(n, BROADCAST_PAYLOAD_BYTES);
        assert_eq!(unpack(&buf), 42);
        assert!(matches!(r.try_recv(c, &mut buf).err(), Some(BroadcastError::Empty)));
    }

    #[test]
    fn three_consumers_each_see_all_messages() {
        let p = tmp("1p3c");
        let r = SharedBroadcastRing::create(&p, 16).unwrap();
        let c0 = r.register_consumer().unwrap();
        let c1 = r.register_consumer().unwrap();
        let c2 = r.register_consumer().unwrap();
        for i in 0..5u32 { r.try_push(&payload_of(i * 10)).unwrap(); }
        let mut buf = [0u8; BROADCAST_PAYLOAD_BYTES];
        for c in [c0, c1, c2] {
            for i in 0..5u32 {
                r.try_recv(c, &mut buf).unwrap();
                assert_eq!(unpack(&buf), i * 10,
                    "consumer {c} should see message {i}");
            }
            assert!(matches!(r.try_recv(c, &mut buf).err(), Some(BroadcastError::Empty)));
        }
    }

    #[test]
    fn lagging_consumer_blocks_producer() {
        let p = tmp("lag-blocks");
        let r = SharedBroadcastRing::create(&p, 4).unwrap();
        let c = r.register_consumer().unwrap();
        let _c = c;
        // Fill the ring.
        for i in 0..4u32 { r.try_push(&payload_of(i)).unwrap(); }
        // Consumer hasn't read anything; next push should fail Full.
        assert!(matches!(r.try_push(&payload_of(99)).err(), Some(BroadcastError::Full)));
    }

    #[test]
    fn unregistering_lagging_consumer_unblocks_producer() {
        let p = tmp("unreg-unblocks");
        let r = SharedBroadcastRing::create(&p, 4).unwrap();
        let c = r.register_consumer().unwrap();
        for i in 0..4u32 { r.try_push(&payload_of(i)).unwrap(); }
        assert!(matches!(r.try_push(&payload_of(99)).err(), Some(BroadcastError::Full)));
        r.unregister_consumer(c);
        // With no active consumers, producer can push freely.
        r.try_push(&payload_of(99)).unwrap();
    }

    #[test]
    fn consumer_registered_late_starts_at_current_producer() {
        let p = tmp("late-consumer");
        let r = SharedBroadcastRing::create(&p, 8).unwrap();
        // Push 3 messages before registering any consumer.
        for i in 0..3u32 { r.try_push(&payload_of(i)).unwrap(); }
        // Now register; this consumer should see only new messages.
        let c = r.register_consumer().unwrap();
        let mut buf = [0u8; BROADCAST_PAYLOAD_BYTES];
        assert!(matches!(r.try_recv(c, &mut buf).err(), Some(BroadcastError::Empty)));
        r.try_push(&payload_of(100)).unwrap();
        r.try_recv(c, &mut buf).unwrap();
        assert_eq!(unpack(&buf), 100);
    }

    #[test]
    fn lag_returns_pending_count() {
        let p = tmp("lag");
        let r = SharedBroadcastRing::create(&p, 8).unwrap();
        let c = r.register_consumer().unwrap();
        for i in 0..3u32 { r.try_push(&payload_of(i)).unwrap(); }
        assert_eq!(r.lag(c), 3);
        let mut buf = [0u8; BROADCAST_PAYLOAD_BYTES];
        r.try_recv(c, &mut buf).unwrap();
        assert_eq!(r.lag(c), 2);
    }

    /// An index the consumer table does not reach names no consumer, and
    /// saying so is the whole point: zero there reads as "caught up"
    /// about a reader that does not exist.
    #[test]
    fn lag_says_nothing_for_an_index_past_the_table() {
        let p = tmp("lag-past-table");
        let r = SharedBroadcastRing::create(&p, 8).unwrap();
        for i in 0..3u32 {
            r.try_push(&payload_of(i)).unwrap();
        }
        assert_eq!(r.try_lag(MAX_CONSUMERS), None);
        assert_eq!(r.lag(MAX_CONSUMERS), u64::MAX);
    }

    /// A slot nobody holds names no consumer either, whether it was
    /// never taken or has been given back. Its cursor stays where its
    /// last holder left it, so the distance to the producer is a number
    /// about nobody that grows with every push.
    #[test]
    fn lag_says_nothing_for_a_slot_nobody_holds() {
        let p = tmp("lag-dead-slot");
        let r = SharedBroadcastRing::create(&p, 8).unwrap();

        // Never taken.
        assert_eq!(r.try_lag(0), None);

        let c = r.register_consumer().unwrap();
        for i in 0..3u32 {
            r.try_push(&payload_of(i)).unwrap();
        }
        assert_eq!(r.try_lag(c), Some(3));

        // Given back. What it reads now must not depend on how much the
        // producer goes on to publish.
        r.unregister_consumer(c);
        assert_eq!(r.try_lag(c), None);
        assert_eq!(r.lag(c), u64::MAX);
        for i in 3..6u32 {
            r.try_push(&payload_of(i)).unwrap();
        }
        assert_eq!(r.try_lag(c), None);
    }

    /// The barrier returns at once when the readers are already there,
    /// and reports the shortfall rather than hanging when they are not.
    #[test]
    fn waiting_for_consumers_reports_how_many_arrived() {
        let p = tmp("await-cons");
        let r = SharedBroadcastRing::create(&p, 8).unwrap();

        // Nobody wanted, nobody needed.
        assert_eq!(r.wait_for_consumers(0, Duration::from_millis(50)), 0);

        r.register_consumer().unwrap();
        r.register_consumer().unwrap();
        assert_eq!(r.wait_for_consumers(2, Duration::from_millis(50)), 2);

        // Short by one. The count returned is what arrived, which is what a
        // producer needs in order to say who it is missing.
        let began = Instant::now();
        assert_eq!(r.wait_for_consumers(3, Duration::from_millis(50)), 2);
        assert!(
            began.elapsed() >= Duration::from_millis(40),
            "it must have actually waited, not given up on the first look"
        );
    }

    /// A consumer arriving late releases the producer, which is the
    /// whole point: the window this closes is however long starting a
    /// reader takes.
    #[test]
    fn a_late_consumer_releases_the_barrier() {
        let p = tmp("await-late");
        let r = std::sync::Arc::new(SharedBroadcastRing::create(&p, 8).unwrap());

        let joining = {
            let r = std::sync::Arc::clone(&r);
            std::thread::spawn(move || {
                std::thread::sleep(Duration::from_millis(20));
                r.register_consumer().unwrap()
            })
        };

        assert_eq!(r.wait_for_consumers(1, Duration::from_secs(5)), 1);
        let consumer = joining.join().unwrap();

        // And because the producer waited, the reader sees what it
        // publishes rather than starting past it.
        r.try_push(&payload_of(7)).unwrap();
        let mut buf = [0u8; BROADCAST_PAYLOAD_BYTES];
        r.try_recv(consumer, &mut buf).unwrap();
        assert_eq!(unpack(&buf), 7);
    }

    #[test]
    fn max_consumers_returns_no_slot_when_full() {
        let p = tmp("max-cons");
        let r = SharedBroadcastRing::create(&p, 4).unwrap();
        for _ in 0..MAX_CONSUMERS {
            r.register_consumer().unwrap();
        }
        assert!(matches!(r.register_consumer().err(), Some(BroadcastError::NoConsumerSlot)));
    }

    #[test]
    fn cross_handle_pub_sub() {
        let p = tmp("cross-handle");
        let pub_handle = SharedBroadcastRing::create(&p, 8).unwrap();
        let sub_handle = SharedBroadcastRing::open(&p, 8).unwrap();
        let c = sub_handle.register_consumer().unwrap();
        pub_handle.try_push(&payload_of(7777)).unwrap();
        let mut buf = [0u8; BROADCAST_PAYLOAD_BYTES];
        sub_handle.try_recv(c, &mut buf).unwrap();
        assert_eq!(unpack(&buf), 7777);
    }

    #[test]
    fn payload_too_large_rejected_at_push() {
        let p = tmp("oversized");
        let r = SharedBroadcastRing::create(&p, 4).unwrap();
        let _c = r.register_consumer().unwrap();
        let big = vec![0u8; BROADCAST_PAYLOAD_BYTES + 1];
        assert!(matches!(r.try_push(&big).err(), Some(BroadcastError::PayloadTooLarge)));
    }

    #[test]
    fn concurrent_consumers_all_drain_correctly() {
        let p = tmp("concurrent");
        let r = Arc::new(SharedBroadcastRing::create(&p, 256).unwrap());
        let n_consumers = 4;
        let n_msgs = 100u32;
        let consumer_ids: Vec<usize> = (0..n_consumers).map(|_| r.register_consumer().unwrap()).collect();

        let r_p = r.clone();
        let producer = thread::spawn(move || {
            for i in 0..n_msgs {
                while r_p.try_push(&payload_of(i)).is_err() {
                    thread::yield_now();
                }
            }
        });

        let mut handles = vec![];
        for &c in &consumer_ids {
            let r = r.clone();
            handles.push(thread::spawn(move || {
                let mut received = vec![];
                let mut buf = [0u8; BROADCAST_PAYLOAD_BYTES];
                while received.len() < n_msgs as usize {
                    match r.try_recv(c, &mut buf) {
                        Ok(_) => received.push(unpack(&buf)),
                        Err(BroadcastError::Empty) => thread::yield_now(),
                        Err(e) => panic!("unexpected error: {e:?}"),
                    }
                }
                received
            }));
        }
        producer.join().unwrap();
        for h in handles {
            let got = h.join().unwrap();
            assert_eq!(got.len(), n_msgs as usize);
            for (i, v) in got.iter().enumerate() {
                assert_eq!(*v, i as u32, "message order must be preserved");
            }
        }
    }

    #[test]
    fn slow_consumer_doesnt_break_fast_consumer() {
        let p = tmp("slow-fast");
        let r = Arc::new(SharedBroadcastRing::create(&p, 8).unwrap());
        let _slow = r.register_consumer().unwrap();
        let fast = r.register_consumer().unwrap();

        for i in 0..4u32 { r.try_push(&payload_of(i)).unwrap(); }
        // Fast consumer drains; slow consumer hasn't moved.
        let mut buf = [0u8; BROADCAST_PAYLOAD_BYTES];
        for i in 0..4u32 {
            r.try_recv(fast, &mut buf).unwrap();
            assert_eq!(unpack(&buf), i);
        }
        // Producer is still bounded by slow consumer's cursor though;
        // try to push 5 more - should hit Full at some point.
        let mut pushed = 0;
        for i in 100..200u32 {
            if r.try_push(&payload_of(i)).is_err() { break; }
            pushed += 1;
        }
        assert!(pushed <= 4, "should be bounded by slow consumer; pushed {pushed}");
    }

    #[test]
    fn observer_can_wait_for_publisher() {
        let p = tmp("wait");
        let r = Arc::new(SharedBroadcastRing::create(&p, 8).unwrap());
        let c = r.register_consumer().unwrap();

        let r_p = r.clone();
        let pusher = thread::spawn(move || {
            thread::sleep(Duration::from_millis(20));
            r_p.try_push(&payload_of(555)).unwrap();
        });
        let mut buf = [0u8; BROADCAST_PAYLOAD_BYTES];
        let r_c = r.clone();
        let consumer = thread::spawn(move || {
            loop {
                match r_c.try_recv(c, &mut buf) {
                    Ok(_) => break unpack(&buf),
                    Err(BroadcastError::Empty) => thread::yield_now(),
                    Err(e) => panic!("{e:?}"),
                }
            }
        });
        pusher.join().unwrap();
        assert_eq!(consumer.join().unwrap(), 555);
    }

    #[test]
    fn disk_persistence_survives_reopen() {
        let p = tmp("disk");
        {
            let r = SharedBroadcastRing::create(&p, 8).unwrap();
            let _c = r.register_consumer().unwrap();
            r.try_push(&payload_of(1234)).unwrap();
            r.try_push(&payload_of(5678)).unwrap();
            r.flush().unwrap();
        }
        let r2 = SharedBroadcastRing::open(&p, 8).unwrap();
        // Producer seq should be 2; consumer 0 should still be at 0.
        assert_eq!(r2.producer_position(), 2);
        assert_eq!(r2.lag(0), 2);
        let mut buf = [0u8; BROADCAST_PAYLOAD_BYTES];
        r2.try_recv(0, &mut buf).unwrap();
        assert_eq!(unpack(&buf), 1234);
        r2.try_recv(0, &mut buf).unwrap();
        assert_eq!(unpack(&buf), 5678);
    }
}
