//! `CrossProcessWaker`: a futex-shaped wait/wake primitive sitting
//! in shared memory (MMF or named-shm), portable across Linux /
//! Windows / macOS / FreeBSD.
//!
//! # The problem
//!
//! SubEtha's bounded rings deliver bytes between threads / processes
//! without any kernel involvement on the hot path. That's a win
//! when the consumer can keep up - try_recv either returns an item
//! or returns `Empty` and the caller decides what to do. The
//! pattern breaks down when the consumer wants to block on an empty
//! ring without spinning: there's no kernel-side handle to wait on,
//! and a busy-wait burns one CPU per blocked consumer.
//!
//! `CrossProcessWaker` closes the gap. It's the userspace `futex`,
//! ported to the substrate. The producer publishes a monotonic
//! sequence atom on every push; a blocked consumer parks on a wake
//! list in shared memory, registering the sequence it wants to be
//! woken at. When the producer's sequence advances past that
//! target, the producer's post-publish path fires a single
//! syscall-level wake and the consumer's `wait` returns.
//!
//! # Cross-platform wake
//!
//! The primitive calls the platform's wait / wake syscalls
//! directly, rather than via the `atomic-wait` crate, which hard-codes
//! `FUTEX_PRIVATE_FLAG` on Linux and so cannot work across
//! processes):
//!
//! - Linux / Android: `futex(FUTEX_WAIT)` / `futex(FUTEX_WAKE)`
//!   without `FUTEX_PRIVATE_FLAG` - the kernel hashes by the page's
//!   physical address so any process that mapped the same MMF
//!   page joins the same wait queue.
//! - FreeBSD: `_umtx_op(UMTX_OP_WAIT_UINT)` /
//!   `_umtx_op(UMTX_OP_WAKE)` - the umtx ops without the `_PRIVATE` suffix, whose
//!   sleep queues the kernel keys by physical address exactly so
//!   process-shared synchronization works (per `_umtx_op(2)`).
//!   Same cross-process semantics as the Linux arm.
//! - Windows: `WaitOnAddress` / `WakeByAddressSingle` for an untimed
//!   waiter on a process-private (anon-backed) waker - those calls are
//!   intra-process only per Microsoft's docs. A cross-process
//!   (file / named-shm backed) waiter waits on the `MONITOR` tier
//!   (`crate::monitor_wait`), then sleeps on a named event whose id
//!   it publishes in its slot, which the waker sets. A timed waiter on
//!   an anon-backed waker sleeps the same way on an unnamed event,
//!   since `WaitOnAddress` ends a timeout on the system timer's tick,
//!   and a timed sleep on either kind of event runs beside the thread's
//!   high-resolution waitable timer, so it ends at its deadline.
//! - macOS / other: polling fallback (correct, but wastes CPU
//!   when idle).
//!
//! All shipping syscalls operate on a user-space address (no
//! kernel handle bookkeeping per primitive), so the cross-process
//! Linux case needs only that the waker's atomic lives in a
//! mapping both processes have (named-shm or file-backed mmap).
//! The kernel sees the same physical page from both sides and
//! the wake reaches the parker.
//!
//! # Storage layout
//!
//! ```text
//! +--------------------------------------+ offset 0
//! | WakerHeader (64 bytes, one cache line)|
//! |   magic: u64                          |
//! |   capacity: u32                       |
//! |   _pad                                |
//! +--------------------------------------+ offset 64
//! | WakerSlot[0] (64 bytes)              |
//! |   state: AtomicU32 (STATE_*)          |
//! |   _pad                                |
//! |   target_seq: AtomicU64               |
//! |   park_event: AtomicU64               |
//! |   _pad                                |
//! +--------------------------------------+
//! | WakerSlot[1] ... WakerSlot[N-1]      |
//! +--------------------------------------+
//! ```
//!
//! Each slot is one cache line so producer's wake-scan and
//! parker's state writes don't false-share across slots.
//!
//! # Wake protocol
//!
//! ## Consumer (parker) side
//!
//! 1. Scan slots for one with `state == STATE_FREE`.
//! 2. CAS that slot's state from `STATE_FREE` to the transient `STATE_RESERVED`.
//! 3. Write `target_seq` (the sequence we want to be woken at).
//! 4. Store state from `STATE_RESERVED` to `STATE_PARKED` with Release ordering -
//!    this publishes the slot to producers and is the
//!    happens-before edge for `target_seq`.
//! 5. Call the platform's wait syscall on `&slot.state` with
//!    expected = `STATE_PARKED`. The kernel verifies `state == STATE_PARKED`
//!    before sleeping (Linux's futex_wait semantics; Windows'
//!    WaitOnAddress likewise); if a producer's wake-CAS already
//!    landed (`state == STATE_WOKEN`), wait returns immediately without
//!    entering the kernel sleep path. On Windows with a file or shm
//!    backing, and for a timed wait on an anonymous one, the kernel
//!    sleep is on the slot's park event instead: store the event's id in
//!    `park_event`, re-check `state`, sleep on the event (a named one
//!    re-checking at least every 20 ms), and store zero on the way out.
//! 6. On return, clear the slot's parked-mask bit and only then store
//!    `STATE_FREE`, so a parker that takes the freed slot keeps the bit
//!    it sets. A wait that ran to its deadline first takes the slot back
//!    from `STATE_PARKED`; when a wake's exchange beat it there, the wait
//!    answers `Ok` rather than `Timeout`, so the wake is not lost.
//!
//! ## Producer (waker) side
//!
//! On every successful publish, call `wake_up_to(producer_seq)`.
//! That scans slots:
//!
//! 1. Acquire-load `state`. If it is not `STATE_PARKED`, skip.
//! 2. Relaxed-load `target_seq`. The Acquire on `state` acquired
//!    the parker's Release-store, so prior writes (incl. target_seq)
//!    are visible.
//! 3. If `producer_seq >= target_seq`, CAS state from `STATE_PARKED` to
//!    `STATE_WOKEN`. On CAS success, set the park event a non-zero
//!    `park_event` names (Windows), call the platform's wake-one
//!    syscall on `&slot.state` and increment the wake counter.
//!
//! The CAS guards against a double-wake when multiple producers
//! race to wake the same slot. It and the `park_event` load pair with
//! the parker's store of `park_event` and its re-check of `state`: all
//! four are SeqCst, so either the parker reads `STATE_WOKEN` before it
//! sleeps or the waker reads the id of the event it sleeps on.
//!
//! # Wake-before-park race
//!
//! Between a blocked-recv's "try_recv returned Empty" check and
//! its `try_park` call, a producer can publish and call wake_up_to
//! that finds zero parked slots. The standard recovery is the
//! double-check in the blocking-recv wrapper: after parking,
//! re-call try_recv before calling wait. If try_recv succeeds,
//! release the token and return. Only if it still returns Empty
//! does the consumer call wait. The double-check holds because
//! `try_park` ends with a SeqCst fence and every wake scan starts
//! with one: publish-then-scan and park-then-re-check are each a
//! store followed by a load, which x86 and ARM64 both reorder
//! without a full fence.
//!
//! # Linux-futex-raw escape hatch
//!
//! The Cargo feature `linux-futex-raw` exposes the direct
//! `libc::syscall(SYS_futex, ...)` surface to callers that need
//! `FUTEX_WAIT_BITSET`, `FUTEX_REQUEUE`, or other ops the portable
//! `atomic-wait` abstraction does not expose. Linux-only.

use std::fs::File;
use std::path::Path;
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::time::{Duration, Instant};

use memmap2::{MmapMut, MmapOptions};

/// Magic bytes identifying a CrossProcessWaker region. Used by
/// `open` to reject the wrong kind of MMF.
pub const WAKER_MAGIC: u64 = 0xE7E7_5742_4B45_5201; // "..WBKER.."

/// Default slot capacity. Caller-overridable on construction.
pub const MAX_WAITERS_DEFAULT: usize = 32;

const STATE_FREE: u32 = 0;
const STATE_RESERVED: u32 = 1;
const STATE_PARKED: u32 = 2;
const STATE_WOKEN: u32 = 3;

#[repr(C, align(64))]
struct WakerHeader {
    magic: u64,
    capacity: u32,
    _pad0: [u8; 4],
    /// One bit per slot index (< 64): set while a parker holds the
    /// slot. Producers' wake scans load this word first; a
    /// zero mask makes the no-waiters case - the overwhelmingly
    /// common one on a healthy ring - a single cache line instead of
    /// `capacity` slot lines. The bit is advisory: stale-set bits
    /// are filtered by the per-slot state check, and a bit a scan
    /// does not see yet is covered by the parker's pre-wait
    /// double-check, which the fences in `try_park` and
    /// `wake_candidates` order against the scan. Capacities > 64
    /// skip the mask and full-scan.
    parked_mask: AtomicU64,
    _pad: [u8; 64 - 24],
}

#[repr(C, align(64))]
struct WakerSlot {
    state: AtomicU32,
    _pad1: [u8; 4],
    target_seq: AtomicU64,
    /// The id of the event a Windows waiter sleeps on while it is in a
    /// kernel park, and zero at every other time.
    #[cfg_attr(not(windows), allow(dead_code))]
    park_event: AtomicU64,
    _pad2: [u8; 64 - 24],
}

const _: () = {
    assert!(std::mem::size_of::<WakerHeader>() == 64);
    assert!(std::mem::size_of::<WakerSlot>() == 64);
};

/// Errors returned by waker operations.
#[derive(Debug)]
pub enum WakerError {
    /// All slots in use. Caller's fallback path: spin until the
    /// thing they care about is ready (the same path they'd use
    /// without a waker).
    Full,
    /// `wait()` returned because its timeout elapsed before any
    /// producer fired a wake.
    Timeout,
    /// `open()` rejected the MMF because magic / capacity did
    /// not match.
    LayoutMismatch(crate::layout::Mismatch),
    /// The operating system refused the mapping, or on Windows the park
    /// event's sleep; the error carries its code and message.
    IoError(std::io::Error),
}

impl From<std::io::Error> for WakerError {
    fn from(e: std::io::Error) -> Self { Self::IoError(e) }
}

impl std::fmt::Display for WakerError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Full => write!(f, "no free waker slot"),
            Self::Timeout => write!(f, "wait timed out"),
            Self::LayoutMismatch(m) => write!(f, "a waker of another shape: {m}"),
            Self::IoError(e) => write!(f, "io: {e}"),
        }
    }
}

impl std::error::Error for WakerError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::IoError(e) => Some(e),
            Self::Full | Self::Timeout | Self::LayoutMismatch(_) => None,
        }
    }
}

/// The first of the header's magic and capacity that differs from a
/// waker of `capacity` slots.
fn header_matches(hdr: &WakerHeader, capacity: usize) -> Result<(), WakerError> {
    crate::layout::Mismatch::first(&[
        ("magic", hdr.magic, WAKER_MAGIC),
        ("capacity", u64::from(hdr.capacity), capacity as u64),
    ])
    .map_err(WakerError::LayoutMismatch)
}

/// A backing of `len` bytes, refused when it is shorter than `wanted`.
fn long_enough(len: u64, wanted: usize) -> Result<(), WakerError> {
    if len < wanted as u64 {
        return Err(WakerError::LayoutMismatch(crate::layout::Mismatch::new(
            "length",
            len,
            wanted as u64,
        )));
    }
    Ok(())
}

/// Returned by `try_park`; identifies which slot the parker
/// reserved. Pass back into `wait` and `release`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WakerToken {
    slot: u32,
}

impl WakerToken {
    /// The slot index this token is bound to. Useful for
    /// debugging / instrumentation.
    pub fn slot_index(&self) -> u32 { self.slot }

    /// A token for `slot`, for a caller that carried the index across a
    /// boundary this type cannot travel over.
    ///
    /// The index is all a token holds, so this reconstructs one exactly.
    /// It also grants whatever the index names: `wait` and `release` act
    /// on that slot whoever parked it, so a caller handing indices out
    /// keeps track of which parker owns which.
    pub fn from_slot(slot: u32) -> Self { Self { slot } }
}

/// Bytes required for a waker region holding `capacity` slots.
pub const fn waker_region_size(capacity: usize) -> usize {
    std::mem::size_of::<WakerHeader>() + capacity * std::mem::size_of::<WakerSlot>()
}

/// Cross-process wake list. See module docs for the protocol.
pub struct CrossProcessWaker {
    _backing: WakerBacking,
    raw_ptr: *mut u8,
    capacity: usize,
    /// The events this waker's waiters sleep on and its wakers set: named
    /// for a file or shm backing, unnamed for an anonymous one.
    #[cfg(windows)]
    park: crate::park_event::ParkEvents,
    /// Waits begun on this handle.
    #[cfg(test)]
    waits_begun: std::sync::atomic::AtomicUsize,
}

unsafe impl Send for CrossProcessWaker {}
unsafe impl Sync for CrossProcessWaker {}

#[allow(dead_code)]
enum WakerBacking {
    Anon(MmapMut),
    File(File, MmapMut),
    Shm(crate::shm_file::ShmFile),
}

unsafe fn init_waker_layout_raw(ptr: *mut u8, capacity: usize) {
    let hdr_ptr = ptr as *mut WakerHeader;
    unsafe {
        std::ptr::write_bytes(hdr_ptr as *mut u8, 0, std::mem::size_of::<WakerHeader>());
        (*hdr_ptr).magic = WAKER_MAGIC;
        (*hdr_ptr).capacity = capacity as u32;
    }
    let slots_base = unsafe { ptr.add(std::mem::size_of::<WakerHeader>()) };
    for i in 0..capacity {
        let slot_ptr = unsafe {
            slots_base.add(i * std::mem::size_of::<WakerSlot>())
        } as *mut WakerSlot;
        unsafe {
            std::ptr::write(slot_ptr, WakerSlot {
                state: AtomicU32::new(STATE_FREE),
                _pad1: [0; 4],
                target_seq: AtomicU64::new(0),
                park_event: AtomicU64::new(0),
                _pad2: [0; 64 - 24],
            });
        }
    }
}

impl CrossProcessWaker {
    /// A waker over `backing`, whose mapping starts at `raw_ptr`.
    fn with_backing(backing: WakerBacking, raw_ptr: *mut u8, capacity: usize) -> Self {
        #[cfg(windows)]
        let park = match &backing {
            WakerBacking::Anon(_) => crate::park_event::ParkEvents::anon(capacity),
            WakerBacking::File(..) => crate::park_event::ParkEvents::file(capacity),
            WakerBacking::Shm(shm) => {
                crate::park_event::ParkEvents::shm(shm.directory(), shm.sddl(), capacity)
            }
        };
        Self {
            _backing: backing,
            raw_ptr,
            capacity,
            #[cfg(windows)]
            park,
            #[cfg(test)]
            waits_begun: std::sync::atomic::AtomicUsize::new(0),
        }
    }

    /// Anon (in-process) waker. Cross-thread only; for cross-
    /// process use `create` (file) or `create_from_shm` (named).
    pub fn create_anon(capacity: usize) -> Result<Self, WakerError> {
        assert!(capacity >= 1, "capacity must be >= 1");
        let total = waker_region_size(capacity);
        let mut mmap = MmapOptions::new().len(total).map_anon()?;
        let raw_ptr = mmap.as_mut_ptr();
        unsafe { init_waker_layout_raw(raw_ptr, capacity); }
        Ok(Self::with_backing(WakerBacking::Anon(mmap), raw_ptr, capacity))
    }

    /// File-backed waker, cross-process visible via the OS page cache.
    /// Initializes the region only when `path` does not yet exist; otherwise
    /// attaches, leaving parked waiters in place. A region built with a
    /// different capacity is a `LayoutMismatch`. [`reset`](Self::reset)
    /// reinitializes.
    pub fn create(path: impl AsRef<Path>, capacity: usize) -> Result<Self, WakerError> {
        assert!(capacity >= 1, "capacity must be >= 1");
        let total = waker_region_size(capacity);
        let (file, mut mmap) = crate::mmf_attach::create_or_attach(
            path.as_ref(),
            total,
            |ptr| unsafe { init_waker_layout_raw(ptr, capacity) },
            |ptr| unsafe { (*(ptr as *const WakerHeader)).magic },
            WAKER_MAGIC,
        )
        .map_err(|e| crate::mmf_attach::attach_error(e, WakerError::LayoutMismatch))?;
        let raw_ptr = mmap.as_mut_ptr();
        let hdr = unsafe { &*(raw_ptr as *const WakerHeader) };
        header_matches(hdr, capacity)?;
        Ok(Self::with_backing(WakerBacking::File(file, mmap), raw_ptr, capacity))
    }

    /// Reinitialize the waker at `path`, discarding any parked waiters a live
    /// peer holds. For a caller that knows it owns the path.
    pub fn reset(path: impl AsRef<Path>, capacity: usize) -> Result<Self, WakerError> {
        assert!(capacity >= 1, "capacity must be >= 1");
        let total = waker_region_size(capacity);
        let (file, mut mmap) = crate::mmf_attach::reset(path.as_ref(), total, |ptr| unsafe {
            init_waker_layout_raw(ptr, capacity)
        })?;
        let raw_ptr = mmap.as_mut_ptr();
        Ok(Self::with_backing(WakerBacking::File(file, mmap), raw_ptr, capacity))
    }

    /// Open an existing file-backed waker. Validates magic +
    /// capacity.
    pub fn open(
        path: impl AsRef<Path>,
        expected_capacity: usize,
    ) -> Result<Self, WakerError> {
        let total = waker_region_size(expected_capacity);
        let file = crate::region_file::open_existing(path.as_ref())?;
        long_enough(file.metadata()?.len(), total)?;
        let mut mmap = unsafe { MmapOptions::new().len(total).map_mut(&file)? };
        let raw_ptr = mmap.as_mut_ptr();
        let hdr = unsafe { &*(raw_ptr as *const WakerHeader) };
        header_matches(hdr, expected_capacity)?;
        Ok(Self::with_backing(WakerBacking::File(file, mmap), raw_ptr, expected_capacity))
    }

    /// Build a fresh waker on top of a named-shm region. Cross-
    /// process visible via the `logical_name` of the underlying
    /// [`ShmFile`](crate::shm_file::ShmFile); RAM-resident.
    pub fn create_from_shm(
        mut shm: crate::shm_file::ShmFile,
        capacity: usize,
    ) -> Result<Self, WakerError> {
        assert!(capacity >= 1, "capacity must be >= 1");
        let total = waker_region_size(capacity);
        long_enough(shm.len() as u64, total)?;
        let raw_ptr = shm.as_mut_slice().as_mut_ptr();
        unsafe { init_waker_layout_raw(raw_ptr, capacity); }
        Ok(Self::with_backing(WakerBacking::Shm(shm), raw_ptr, capacity))
    }

    /// Open an existing named-shm waker without re-initializing
    /// the layout.
    pub fn open_from_shm(
        mut shm: crate::shm_file::ShmFile,
        expected_capacity: usize,
    ) -> Result<Self, WakerError> {
        let total = waker_region_size(expected_capacity);
        long_enough(shm.len() as u64, total)?;
        let raw_ptr = shm.as_mut_slice().as_mut_ptr();
        let hdr = unsafe { &*(raw_ptr as *const WakerHeader) };
        header_matches(hdr, expected_capacity)?;
        Ok(Self::with_backing(WakerBacking::Shm(shm), raw_ptr, expected_capacity))
    }

    /// Slot count fixed at construction.
    pub fn capacity(&self) -> usize { self.capacity }

    #[inline]
    fn slot(&self, idx: usize) -> &WakerSlot {
        let base = unsafe { self.raw_ptr.add(std::mem::size_of::<WakerHeader>()) };
        unsafe { &*(base.add(idx * std::mem::size_of::<WakerSlot>()) as *const WakerSlot) }
    }

    /// Waits begun on this handle: each is counted as it starts, before it
    /// can return.
    #[cfg(test)]
    pub(crate) fn waits_begun(&self) -> usize {
        self.waits_begun.load(Ordering::Acquire)
    }

    /// Return once a wait has begun on this handle, for a test that must act
    /// only after a waiter has parked: a blocking call begins its wait only
    /// after its last look, so from then only a wake or its timeout ends it.
    /// `returned` is set by the waiting thread when its call comes back; a
    /// call that returns before any wait began fails the test.
    #[cfg(test)]
    pub(crate) fn until_a_wait_begins(&self, returned: &std::sync::atomic::AtomicBool) {
        loop {
            let gone = returned.load(Ordering::Acquire);
            if self.waits_begun() > 0 {
                return;
            }
            assert!(!gone, "the call returned before it began a wait");
            std::thread::yield_now();
        }
    }

    /// Reserve a slot and park at `target_seq`. The caller will
    /// be woken when some producer calls `wake_up_to(seq)` with
    /// `seq >= target_seq`. Returns the token the caller passes
    /// to `wait` / `release`.
    ///
    /// On `Err(Full)`, every slot is currently in use; the caller
    /// falls back to spinning. This is the same fallback they'd
    /// use without a waker at all.
    pub fn try_park(&self, target_seq: u64) -> Result<WakerToken, WakerError> {
        for idx in 0..self.capacity {
            let slot = self.slot(idx);
            // STATE_FREE -> STATE_RESERVED CAS. On success, this slot is
            // ours; on failure, another parker beat us to it,
            // try the next slot.
            if slot
                .state
                .compare_exchange(
                    STATE_FREE,
                    STATE_RESERVED,
                    Ordering::Acquire,
                    Ordering::Relaxed,
                )
                .is_ok()
            {
                // Write target_seq before publishing as STATE_PARKED.
                // The Release store on state below is the
                // happens-before edge for this Relaxed store.
                slot.target_seq.store(target_seq, Ordering::Relaxed);
                // Publish the slot to producers.
                slot.state.store(STATE_PARKED, Ordering::Release);
                self.mask_set(idx);
                // The parker's half of the fence pair in wake_candidates:
                // the caller's re-check of its ring comes after this, so
                // either that re-check sees the producer's item or the
                // producer's scan sees this slot.
                std::sync::atomic::fence(Ordering::SeqCst);
                return Ok(WakerToken { slot: idx as u32 });
            }
        }
        Err(WakerError::Full)
    }

    /// Whether this waker's backing is reachable from other
    /// processes (file / named-shm) rather than process-private
    /// anonymous memory. The platform wait and wake calls take their
    /// process-shared form by it, and on Windows a cross-process-backed
    /// wait with no park event stays on the hardware monitor tier,
    /// since `WaitOnAddress` never receives a wake from another process.
    fn is_cross_process(&self) -> bool {
        !matches!(self._backing, WakerBacking::Anon(_))
    }

    fn header(&self) -> &WakerHeader {
        unsafe { &*(self.raw_ptr as *const WakerHeader) }
    }

    #[inline]
    fn mask_set(&self, idx: usize) {
        if idx < 64 {
            self.header()
                .parked_mask
                .fetch_or(1u64 << idx, Ordering::Release);
        }
    }

    #[inline]
    fn mask_clear(&self, idx: usize) {
        if idx < 64 {
            self.header()
                .parked_mask
                .fetch_and(!(1u64 << idx), Ordering::Release);
        }
    }

    /// Iterator over candidate slot indices for a wake scan: the
    /// parked-mask bits when the mask covers every slot, else the
    /// full range.
    ///
    /// It starts with a SeqCst fence, paired with the one at the end of
    /// `try_park`. The caller has just published an item (a ring's head
    /// store), so either this scan sees the parked slot or the parker's
    /// re-check sees the item.
    #[inline]
    fn wake_candidates(&self) -> WakeCandidates {
        std::sync::atomic::fence(Ordering::SeqCst);
        if self.capacity <= 64 {
            WakeCandidates::Mask(
                self.header().parked_mask.load(Ordering::Acquire),
            )
        } else {
            WakeCandidates::Range(0, self.capacity)
        }
    }

    /// Block until either some producer wakes this token or the
    /// optional timeout elapses. After return (Ok or Err), the
    /// token's slot is released; the caller does not need to
    /// call `release` separately.
    ///
    /// If the slot's state was already transitioned to `WOKEN`
    /// before the wait call entered the kernel, the wait returns
    /// immediately (no kernel sleep).
    ///
    /// On Windows a wait on a file or shm backing, or a timed wait on an
    /// anonymous one, that outlasts the monitor budget sleeps on the
    /// slot's park event, a timed one beside a high-resolution timer so
    /// it ends at its timeout rather than at the system timer's next
    /// tick, and a failure of that sleep returns
    /// [`WakerError::IoError`].
    pub fn wait(
        &self,
        token: WakerToken,
        timeout: Option<Duration>,
    ) -> Result<(), WakerError> {
        let plan = crate::wait_active::active_plan(self.park_kind());
        self.wait_with_plan(token, timeout, &plan)
    }

    /// As [`wait`](Self::wait), with `plan`'s monitor phase in place of
    /// the process's active plan.
    pub fn wait_with_plan(
        &self,
        token: WakerToken,
        timeout: Option<Duration>,
        plan: &crate::wait_plan::Plan,
    ) -> Result<(), WakerError> {
        #[cfg(test)]
        self.waits_begun.fetch_add(1, Ordering::Release);
        let idx = token.slot as usize;
        let slot = self.slot(idx);
        let deadline = timeout.map(|d| Instant::now() + d);
        let monitor = plan.monitor.as_ref();
        #[cfg(windows)]
        let parked = self.wait_on_park_event(idx, deadline, monitor);
        #[cfg(not(windows))]
        let parked: Option<Result<(), WakerError>> = None;
        let result = match parked {
            Some(result) => result,
            None => self.wait_on_state(slot, deadline, monitor),
        };
        let result = match result {
            Err(WakerError::Timeout) => Self::timed_out(slot),
            other => other,
        };
        self.free_slot(idx);
        result
    }

    /// The answer for a wait that ran to its deadline: `Timeout`, unless a
    /// wake's exchange landed after the last check, which makes the wake
    /// this slot's. Leaving with it as a timeout would strand the item it
    /// announces from every other parked waiter, so the caller gets `Ok`
    /// and tries again. Taking the slot back from `STATE_PARKED` proves no
    /// wake landed.
    fn timed_out(slot: &WakerSlot) -> Result<(), WakerError> {
        if slot
            .state
            .compare_exchange(STATE_PARKED, STATE_RESERVED, Ordering::AcqRel, Ordering::Acquire)
            .is_ok()
        {
            Err(WakerError::Timeout)
        } else {
            Ok(())
        }
    }

    /// Give slot `idx` back. Its mask bit is cleared while the slot is
    /// still this waiter's and only then is the slot freed: a parker that
    /// takes the freed slot sets the bit again, and a clear after that
    /// would erase it and hide that parker from every wake scan.
    fn free_slot(&self, idx: usize) {
        self.mask_clear(idx);
        #[cfg(test)]
        tests::between_clear_and_free();
        self.slot(idx).state.store(STATE_FREE, Ordering::Release);
    }

    /// How this waker's waiters park: [`ParkKind::CrossProcess`] on a file
    /// or shm backing, [`ParkKind::Local`] on anonymous memory.
    ///
    /// [`ParkKind::CrossProcess`]: crate::wait_plan::ParkKind::CrossProcess
    /// [`ParkKind::Local`]: crate::wait_plan::ParkKind::Local
    pub fn park_kind(&self) -> crate::wait_plan::ParkKind {
        if self.is_cross_process() {
            crate::wait_plan::ParkKind::CrossProcess
        } else {
            crate::wait_plan::ParkKind::Local
        }
    }

    /// The spin rounds a waiter on this waker takes before it parks, from
    /// the process's active plan for this waker's park kind.
    pub fn spin_rounds(&self) -> u32 {
        crate::wait_active::active_plan(self.park_kind()).spin_rounds
    }

    /// The Windows kernel park of a waiter on a file or shm backing, or of
    /// a timed waiter on an anonymous one: the monitor tier once, then the
    /// slot's park event. `None` for an untimed waiter on an anonymous
    /// backing, which sleeps on the state word, or when the slot's event
    /// cannot be created; either leaves the wait to
    /// [`wait_on_state`](Self::wait_on_state).
    #[cfg(windows)]
    fn wait_on_park_event(
        &self,
        idx: usize,
        deadline: Option<Instant>,
        monitor: Option<&crate::wait_plan::MonitorPhase>,
    ) -> Option<Result<(), WakerError>> {
        if deadline.is_none() && !self.is_cross_process() {
            return None;
        }
        let slot = self.slot(idx);
        if slot.state.load(Ordering::Acquire) != STATE_PARKED
            || platform_wait::monitor_tier(&slot.state, STATE_PARKED, monitor)
        {
            return Some(Ok(()));
        }
        self.park.park(idx, &slot.state, STATE_PARKED, &slot.park_event, deadline)
    }

    /// Wait on the slot's state word itself: the monitor tier, then the
    /// platform's futex-shaped wait, until the state leaves
    /// `STATE_PARKED` or `deadline` passes.
    fn wait_on_state(
        &self,
        slot: &WakerSlot,
        deadline: Option<Instant>,
        monitor: Option<&crate::wait_plan::MonitorPhase>,
    ) -> Result<(), WakerError> {
        let cross_process = self.is_cross_process();
        match deadline {
            None => loop {
                if slot.state.load(Ordering::Acquire) != STATE_PARKED {
                    break Ok(());
                }
                platform_wait::wait_forever(
                    &slot.state, STATE_PARKED, cross_process, monitor,
                );
            },
            // Deadline re-check loop. The wait syscall is only a
            // hint: futex and the `MONITOR` / `MWAIT` tier can both
            // wake spuriously, so `state`, not the syscall's return,
            // is the authority. A spurious wake re-loops and waits
            // the remaining time; only a real producer transition
            // (state != STATE_PARKED) returns Ok, and only an elapsed
            // deadline returns Timeout, so a wait neither ends early
            // nor frees the slot before a wake_all can see it.
            Some(deadline) => loop {
                if slot.state.load(Ordering::Acquire) != STATE_PARKED {
                    break Ok(());
                }
                match deadline.checked_duration_since(Instant::now()) {
                    Some(remaining) if !remaining.is_zero() => {
                        platform_wait::wait_with_timeout(
                            &slot.state, STATE_PARKED, remaining, cross_process, monitor,
                        );
                    }
                    _ => {
                        // Deadline reached; one last check catches a
                        // wake that landed at the wire.
                        break if slot.state.load(Ordering::Acquire) != STATE_PARKED {
                            Ok(())
                        } else {
                            Err(WakerError::Timeout)
                        };
                    }
                }
            },
        }
    }

    /// Release a parked slot without waiting. Used by the
    /// blocking-recv wrapper's wake-before-park-race recovery:
    /// after parking, the wrapper double-checks try_recv; if
    /// that succeeds, it calls release to give the slot back
    /// without entering the kernel.
    pub fn release(&self, token: WakerToken) {
        self.free_slot(token.slot as usize);
    }

    /// Producer's post-publish wake call. Scans the candidate slots
    /// (the ones the parked mask marks, or every slot when capacity
    /// exceeds 64); for each `STATE_PARKED` slot whose
    /// `target_seq <= seq`, CASes state to `STATE_WOKEN` and fires a
    /// single-slot wake. Returns the
    /// number of consumers woken.
    pub fn wake_up_to(&self, seq: u64) -> usize {
        let mut count = 0usize;
        for idx in self.wake_candidates() {
            let slot = self.slot(idx);
            let cur_state = slot.state.load(Ordering::Acquire);
            if cur_state != STATE_PARKED {
                continue;
            }
            // The Acquire on state acquires the parker's Release
            // store, so target_seq is safe to read Relaxed.
            let tgt = slot.target_seq.load(Ordering::Relaxed);
            if seq < tgt {
                continue;
            }
            // Try to claim the wake. If the CAS fails another
            // producer already woke this slot (or the parker
            // released it); skip.
            if slot
                .state
                .compare_exchange(
                    STATE_PARKED,
                    STATE_WOKEN,
                    Ordering::SeqCst,
                    Ordering::Relaxed,
                )
                .is_ok()
            {
                self.wake_slot(idx);
                count += 1;
            }
        }
        count
    }

    /// Wake at most one parked slot whose `target_seq <= seq`. Used
    /// by Mesa-style condvar `notify_one`: notifier bumps the
    /// generation, then wakes exactly one waiter (if any) so the
    /// other parked waiters stay parked.
    ///
    /// Returns 1 if a waiter was woken, 0 if none qualified.
    pub fn wake_one_up_to(&self, seq: u64) -> usize {
        for idx in self.wake_candidates() {
            let slot = self.slot(idx);
            let cur_state = slot.state.load(Ordering::Acquire);
            if cur_state != STATE_PARKED {
                continue;
            }
            let tgt = slot.target_seq.load(Ordering::Relaxed);
            if seq < tgt {
                continue;
            }
            if slot
                .state
                .compare_exchange(
                    STATE_PARKED,
                    STATE_WOKEN,
                    Ordering::SeqCst,
                    Ordering::Relaxed,
                )
                .is_ok()
            {
                self.wake_slot(idx);
                return 1;
            }
        }
        0
    }

    /// Wake every `STATE_PARKED` slot regardless of `target_seq`. Used
    /// during shutdown / drain so blocked consumers see the
    /// terminate signal.
    pub fn wake_all(&self) -> usize {
        let mut count = 0usize;
        for idx in self.wake_candidates() {
            let slot = self.slot(idx);
            if slot
                .state
                .compare_exchange(
                    STATE_PARKED,
                    STATE_WOKEN,
                    Ordering::SeqCst,
                    Ordering::Relaxed,
                )
                .is_ok()
            {
                self.wake_slot(idx);
                count += 1;
            }
        }
        count
    }

    /// Wake the waiter of slot `idx`, which this thread has just moved
    /// from `STATE_PARKED` to `STATE_WOKEN`: set its park event when it
    /// is sleeping on one, then fire the platform's wake on the state.
    #[inline]
    fn wake_slot(&self, idx: usize) {
        let slot = self.slot(idx);
        #[cfg(windows)]
        {
            let id = slot.park_event.load(Ordering::SeqCst);
            if id != 0 {
                self.park.signal(idx, id);
            }
        }
        platform_wait::wake_one(&slot.state, self.is_cross_process());
    }
}

/// Wake-scan candidate indices: set bits of the parked mask, or a
/// plain range when the capacity outgrows the 64-bit mask.
enum WakeCandidates {
    Mask(u64),
    Range(usize, usize),
}

impl Iterator for WakeCandidates {
    type Item = usize;
    #[inline]
    fn next(&mut self) -> Option<usize> {
        match self {
            WakeCandidates::Mask(m) => {
                if *m == 0 {
                    return None;
                }
                let idx = m.trailing_zeros() as usize;
                *m &= *m - 1;
                Some(idx)
            }
            WakeCandidates::Range(next, end) => {
                if next < end {
                    let idx = *next;
                    *next += 1;
                    Some(idx)
                } else {
                    None
                }
            }
        }
    }
}

// ============================================================================
// Platform wait / wake, which does not go through the atomic-wait crate
// because that crate hard-codes FUTEX_PRIVATE_FLAG on Linux,
// which restricts the futex to a single process and breaks the
// cross-process wake claim. The waker calls the platform's
// process-shared futex / WaitOnAddress / wake APIs directly.
//
// Every wait first runs the bounded `MONITOR`-class tier (see
// crate::monitor_wait): MONITORX/MWAITX or UMONITOR/UMWAIT light
// sleep on the slot's cache line for ~tens of microseconds, woken
// for free by the producer's state store - cross-process included,
// since hardware monitors are physical-address based. Only when
// that budget expires does the wait escalate to the per-platform
// kernel park below.
//
// Cross-process status per platform:
// - Linux / Android: the process-shared futex (no `FUTEX_PRIVATE_FLAG`) works
//   across processes when the atomic sits in a shared mmap.
// - FreeBSD: _umtx_op with the shared (not _PRIVATE) UMTX_OP_WAIT_UINT /
//   UMTX_OP_WAKE ops; the kernel keys those sleep queues by
//   physical address ("same variable mapped multiple times will
//   give one key value" - _umtx_op(2)), so waiters across
//   processes sharing an MMF page join one queue.
// - Windows: WaitOnAddress is intra-process only per the docs,
//   so it serves untimed waiters on anon-backed wakers. A file /
//   shm-backed waiter parks on its named event (crate::park_event),
//   and a timed anon-backed one on an unnamed event; each reaches
//   this module only when its event cannot be created, and a file /
//   shm-backed one then stays on the monitor tier (the cross_process
//   flag below).
// - macOS / others: polling fallback; correct but wastes CPU
//   under heavy idle.
// ============================================================================

pub(crate) mod platform_wait {
    use std::sync::atomic::AtomicU32;
    use std::time::Duration;

    /// macOS 14.4+ `os_sync_*` public-futex symbols resolved at runtime via
    /// `dlsym`, so the binary links against an older SDK (e.g. 10.15, whose
    /// libsystem has no `os_sync_*`) and degrades to the polling fallback there,
    /// while taking the fast path on 14.4+. The flag constants are plain
    /// integers (no link dependency), so only the three functions are resolved.
    #[cfg(target_os = "macos")]
    mod os_sync_dyn {
        use std::ffi::c_void;
        use std::sync::atomic::{AtomicUsize, Ordering};
        pub type WaitFn = unsafe extern "C" fn(*mut c_void, u64, usize, u32) -> i32;
        pub type WaitTimeoutFn =
            unsafe extern "C" fn(*mut c_void, u64, usize, u32, u32, u64) -> i32;
        pub type WakeFn = unsafe extern "C" fn(*mut c_void, usize, u32) -> i32;
        // 1 = not yet probed, 0 = absent (older macOS), else = resolved fn ptr.
        static WAIT: AtomicUsize = AtomicUsize::new(1);
        static WAIT_TO: AtomicUsize = AtomicUsize::new(1);
        static WAKE: AtomicUsize = AtomicUsize::new(1);
        fn cached(slot: &AtomicUsize, name: &[u8]) -> usize {
            let v = slot.load(Ordering::Relaxed);
            if v != 1 {
                return v;
            }
            // SAFETY: RTLD_DEFAULT lookup of a C symbol by NUL-terminated name.
            let r = unsafe { libc::dlsym(libc::RTLD_DEFAULT, name.as_ptr() as *const _) } as usize;
            slot.store(r, Ordering::Relaxed);
            r
        }
        pub fn wait() -> Option<WaitFn> {
            match cached(&WAIT, b"os_sync_wait_on_address\0") {
                0 => None,
                // SAFETY: a non-null resolution of this symbol has this ABI.
                p => Some(unsafe { std::mem::transmute::<usize, WaitFn>(p) }),
            }
        }
        pub fn wait_timeout() -> Option<WaitTimeoutFn> {
            match cached(&WAIT_TO, b"os_sync_wait_on_address_with_timeout\0") {
                0 => None,
                p => Some(unsafe { std::mem::transmute::<usize, WaitTimeoutFn>(p) }),
            }
        }
        pub fn wake() -> Option<WakeFn> {
            match cached(&WAKE, b"os_sync_wake_by_address_any\0") {
                0 => None,
                p => Some(unsafe { std::mem::transmute::<usize, WakeFn>(p) }),
            }
        }
    }

    /// The monitor tier: MONITORX/MWAITX (AMD) or UMONITOR/UMWAIT
    /// (WAITPKG) light-sleep waiting for a bounded cycle budget
    /// ahead of the kernel park. Two wins when the wait resolves
    /// inside the budget: the producer's wake is its existing
    /// state-CAS (no syscall on either side), and - because
    /// hardware monitors are physical-address based - the wake
    /// crosses process boundaries on shared MMF pages without a
    /// kernel object. Returning `false` (budget expired /
    /// unsupported CPU / SUBETHA_NO_MONITOR_WAIT=1) falls through to
    /// the kernel park, which re-checks the value itself - so the
    /// tier can never lose a wake, only hand off.
    #[inline]
    pub fn monitor_tier(
        atomic: &AtomicU32,
        expected: u32,
        monitor: Option<&crate::wait_plan::MonitorPhase>,
    ) -> bool {
        monitor.is_some_and(|phase| {
            crate::monitor_wait::run_monitor_phase_u32(phase, atomic, expected)
        })
    }

    /// The phase a Windows cross-process wait with no park event re-arms:
    /// the plan's, or the fixed phase for the detected family when the
    /// plan has none. `None` when the host has no family or the
    /// environment switched it off.
    #[cfg(windows)]
    fn monitor_only_phase(
        monitor: Option<&crate::wait_plan::MonitorPhase>,
    ) -> Option<crate::wait_plan::MonitorPhase> {
        let kind = crate::monitor_wait::monitor_wait_kind()?;
        Some(monitor.copied().unwrap_or_else(|| {
            crate::monitor_wait::fixed_phase(
                kind,
                crate::monitor_wait::monitor_wait_budget_cycles(),
            )
        }))
    }

    pub fn wait_forever(
        atomic: &AtomicU32,
        expected: u32,
        _cross_process: bool,
        monitor: Option<&crate::wait_plan::MonitorPhase>,
    ) {
        if monitor_tier(atomic, expected, monitor) {
            return;
        }
        // Windows + cross-process backing whose park event could not
        // be created: WaitOnAddress never receives a wake from
        // another process, so the monitor is the wait, re-armed a
        // phase at a time until the value changes.
        #[cfg(windows)]
        if _cross_process
            && let Some(phase) = monitor_only_phase(monitor)
        {
            while atomic.load(std::sync::atomic::Ordering::Acquire) == expected {
                crate::monitor_wait::run_monitor_phase_u32(&phase, atomic, expected);
            }
            return;
        }
        #[cfg(any(target_os = "linux", target_os = "android"))]
        {
            unsafe {
                libc::syscall(
                    libc::SYS_futex,
                    atomic.as_ptr(),
                    libc::FUTEX_WAIT,
                    expected as libc::c_int,
                    std::ptr::null::<libc::timespec>(),
                );
            }
        }
        #[cfg(target_os = "freebsd")]
        {
            // UMTX_OP_WAIT_UINT (not the _PRIVATE op): the kernel
            // keys the sleep queue by the variable's physical
            // address, so waiters in any process that mapped the
            // same MMF page share one queue - FreeBSD's native
            // equivalent of the no-FUTEX_PRIVATE_FLAG Linux call.
            // Sleeps only while *obj == val, exactly futex_wait.
            unsafe {
                libc::_umtx_op(
                    atomic.as_ptr() as *mut libc::c_void,
                    libc::UMTX_OP_WAIT_UINT,
                    expected as libc::c_ulong,
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                );
            }
        }
        #[cfg(windows)]
        {
            use windows_sys::Win32::System::Threading::{WaitOnAddress, INFINITE};
            let expected_local = expected;
            unsafe {
                WaitOnAddress(
                    atomic.as_ptr() as *const std::ffi::c_void,
                    &expected_local as *const u32 as *const std::ffi::c_void,
                    std::mem::size_of::<u32>(),
                    INFINITE,
                );
            }
        }
        #[cfg(target_os = "macos")]
        {
            // The public futex (macOS 14.4+): compare-and-wait on the address;
            // OS_SYNC_WAIT_ON_ADDRESS_SHARED keys the queue for a shared-memory
            // address, allowing a futex wake from another process (the backing
            // decides the flag). The symbol is resolved at runtime; on older
            // macOS it is absent, so poll like the generic fallback below.
            if let Some(wait_fn) = os_sync_dyn::wait() {
                let flags = if _cross_process {
                    libc::OS_SYNC_WAIT_ON_ADDRESS_SHARED
                } else {
                    libc::OS_SYNC_WAIT_ON_ADDRESS_NONE
                };
                unsafe {
                    wait_fn(
                        atomic.as_ptr() as *mut libc::c_void,
                        expected as u64,
                        std::mem::size_of::<u32>(),
                        flags,
                    );
                }
            } else {
                std::thread::yield_now();
                while atomic.load(std::sync::atomic::Ordering::Acquire) == expected {
                    std::thread::sleep(Duration::from_millis(1));
                }
            }
        }
        #[cfg(not(any(target_os = "linux", target_os = "android",
                      target_os = "freebsd", target_os = "macos", windows)))]
        {
            std::thread::yield_now();
            while atomic.load(std::sync::atomic::Ordering::Acquire) == expected {
                std::thread::sleep(Duration::from_millis(1));
            }
        }
    }

    pub fn wait_with_timeout(
        atomic: &AtomicU32,
        expected: u32,
        timeout: Duration,
        _cross_process: bool,
        monitor: Option<&crate::wait_plan::MonitorPhase>,
    ) -> bool {
        let monitor_start = std::time::Instant::now();
        if monitor_tier(atomic, expected, monitor) {
            return true;
        }
        // The monitor budget counts against the caller's timeout;
        // the kernel park gets the remainder.
        let timeout = match timeout.checked_sub(monitor_start.elapsed()) {
            Some(rest) if !rest.is_zero() => rest,
            _ => {
                return atomic.load(std::sync::atomic::Ordering::Acquire)
                    != expected;
            }
        };
        // Windows + cross-process backing with no park event: stay on
        // the monitor for the whole timeout (see wait_forever).
        #[cfg(windows)]
        if _cross_process
            && let Some(phase) = monitor_only_phase(monitor)
        {
            let deadline = std::time::Instant::now() + timeout;
            loop {
                if crate::monitor_wait::run_monitor_phase_u32(&phase, atomic, expected) {
                    return true;
                }
                if std::time::Instant::now() >= deadline {
                    return atomic.load(std::sync::atomic::Ordering::Acquire)
                        != expected;
                }
            }
        }
        #[cfg(any(target_os = "linux", target_os = "android"))]
        {
            let ts = libc::timespec {
                tv_sec: timeout.as_secs() as _,
                tv_nsec: timeout.subsec_nanos() as libc::c_long,
            };
            unsafe {
                let rc = libc::syscall(
                    libc::SYS_futex,
                    atomic.as_ptr(),
                    libc::FUTEX_WAIT,
                    expected as libc::c_int,
                    &ts as *const libc::timespec,
                    std::ptr::null::<()>(),
                    0u32,
                );
                if rc == -1 {
                    let err = *libc::__errno_location();
                    return err != libc::ETIMEDOUT;
                }
            }
            true
        }
        #[cfg(target_os = "freebsd")]
        {
            // Relative timeout, monotonic clock by default: uaddr2
            // points at the timespec and uaddr carries that
            // structure's size, per _umtx_op(2).
            let mut ts = libc::timespec {
                tv_sec: timeout.as_secs() as libc::time_t,
                tv_nsec: timeout.subsec_nanos() as libc::c_long,
            };
            let rc = unsafe {
                libc::_umtx_op(
                    atomic.as_ptr() as *mut libc::c_void,
                    libc::UMTX_OP_WAIT_UINT,
                    expected as libc::c_ulong,
                    std::mem::size_of::<libc::timespec>() as *mut libc::c_void,
                    &mut ts as *mut libc::timespec as *mut libc::c_void,
                )
            };
            if rc == -1 {
                let err = unsafe { *libc::__error() };
                return err != libc::ETIMEDOUT;
            }
            true
        }
        #[cfg(windows)]
        {
            use windows_sys::Win32::System::Threading::WaitOnAddress;
            let expected_local = expected;
            let ms = crate::park_event::whole_ms(timeout);
            let rc = unsafe {
                WaitOnAddress(
                    atomic.as_ptr() as *const std::ffi::c_void,
                    &expected_local as *const u32 as *const std::ffi::c_void,
                    std::mem::size_of::<u32>(),
                    ms,
                )
            };
            rc != 0
        }
        #[cfg(target_os = "macos")]
        {
            if let Some(wait_fn) = os_sync_dyn::wait_timeout() {
                let flags = if _cross_process {
                    libc::OS_SYNC_WAIT_ON_ADDRESS_SHARED
                } else {
                    libc::OS_SYNC_WAIT_ON_ADDRESS_NONE
                };
                let rc = unsafe {
                    wait_fn(
                        atomic.as_ptr() as *mut libc::c_void,
                        expected as u64,
                        std::mem::size_of::<u32>(),
                        flags,
                        libc::OS_CLOCK_MACH_ABSOLUTE_TIME,
                        timeout.as_nanos().min(u64::MAX as u128) as u64,
                    )
                };
                if rc < 0 {
                    return std::io::Error::last_os_error().raw_os_error()
                        != Some(libc::ETIMEDOUT);
                }
                true
            } else {
                // Older macOS (< 14.4): poll with a deadline, like the generic
                // fallback. Returns true if the value changed, false on timeout.
                let deadline = std::time::Instant::now() + timeout;
                loop {
                    if atomic.load(std::sync::atomic::Ordering::Acquire) != expected {
                        return true;
                    }
                    if std::time::Instant::now() >= deadline {
                        return atomic.load(std::sync::atomic::Ordering::Acquire) != expected;
                    }
                    std::thread::sleep(Duration::from_millis(2));
                }
            }
        }
        #[cfg(not(any(target_os = "linux", target_os = "android",
                      target_os = "freebsd", target_os = "macos", windows)))]
        {
            let deadline = std::time::Instant::now() + timeout;
            let step = Duration::from_millis(2);
            loop {
                if atomic.load(std::sync::atomic::Ordering::Acquire) != expected {
                    return true;
                }
                let now = std::time::Instant::now();
                if now >= deadline {
                    return false;
                }
                let remaining = deadline - now;
                std::thread::sleep(remaining.min(step));
            }
        }
    }

    pub fn wake_one(atomic: &AtomicU32, _cross_process: bool) {
        #[cfg(any(target_os = "linux", target_os = "android"))]
        {
            unsafe {
                libc::syscall(
                    libc::SYS_futex,
                    atomic.as_ptr(),
                    libc::FUTEX_WAKE,
                    1i32,
                );
            }
        }
        #[cfg(target_os = "freebsd")]
        {
            // val = max threads to wake; same shared (not _PRIVATE)
            // physical-address-keyed queue the waiters parked on.
            unsafe {
                libc::_umtx_op(
                    atomic.as_ptr() as *mut libc::c_void,
                    libc::UMTX_OP_WAKE,
                    1 as libc::c_ulong,
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                );
            }
        }
        #[cfg(windows)]
        {
            use windows_sys::Win32::System::Threading::WakeByAddressSingle;
            unsafe {
                WakeByAddressSingle(atomic.as_ptr() as *const std::ffi::c_void);
            }
        }
        #[cfg(target_os = "macos")]
        {
            // Mirror of the wait flag: the process-shared form wakes waiters in any process that
            // mapped the same region. On older macOS the symbol is absent and
            // the waiter polls, so no explicit wake is needed.
            if let Some(wake_fn) = os_sync_dyn::wake() {
                let flags = if _cross_process {
                    libc::OS_SYNC_WAKE_BY_ADDRESS_SHARED
                } else {
                    libc::OS_SYNC_WAKE_BY_ADDRESS_NONE
                };
                unsafe {
                    wake_fn(
                        atomic.as_ptr() as *mut libc::c_void,
                        std::mem::size_of::<u32>(),
                        flags,
                    );
                }
            }
        }
        #[cfg(not(any(target_os = "linux", target_os = "android",
                      target_os = "freebsd", target_os = "macos", windows)))]
        {
            drop(atomic);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;
    use std::sync::Arc;
    use std::thread;
    use std::time::Instant;

    thread_local! {
        /// Run once by `free_slot` on this thread, between clearing the
        /// slot's mask bit and freeing the slot.
        static BETWEEN_CLEAR_AND_FREE: Cell<Option<Box<dyn FnOnce()>>> = const { Cell::new(None) };
    }

    pub(super) fn between_clear_and_free() {
        if let Some(hook) = BETWEEN_CLEAR_AND_FREE.take() {
            hook();
        }
    }

    /// A parker that arrives while a leaving waiter is between clearing
    /// its slot's mask bit and freeing the slot takes a slot of its own,
    /// and the next wake scan finds it.
    #[test]
    fn a_parker_arriving_while_a_slot_is_given_back_is_found_by_the_next_wake() {
        let waker = Arc::new(CrossProcessWaker::create_anon(2).expect("create"));
        let leaving = waker.try_park(1).expect("the leaving waiter parks");
        let arrived = Arc::new(AtomicU32::new(u32::MAX));
        let hook = {
            let waker = Arc::clone(&waker);
            let arrived = Arc::clone(&arrived);
            move || {
                let token = waker.try_park(1).expect("the arriving parker finds a slot");
                arrived.store(token.slot, Ordering::Release);
            }
        };
        BETWEEN_CLEAR_AND_FREE.set(Some(Box::new(hook)));
        waker.release(leaving);
        let slot = arrived.load(Ordering::Acquire);
        assert_ne!(slot, u32::MAX, "the hook ran between the clear and the free");
        assert_eq!(waker.wake_up_to(1), 1, "the arriving parker, on slot {slot}, is found by the next wake");
        assert_ne!(slot, leaving.slot, "the slot being given back was not yet free");
    }

    /// A wait that ran to its deadline answers `Timeout` when no wake
    /// landed, and `Ok` when a wake's exchange landed after its last check.
    #[test]
    fn a_wake_landing_after_the_deadline_check_makes_the_wait_succeed() {
        let waker = CrossProcessWaker::create_anon(1).expect("create");
        let token = waker.try_park(1).expect("park");
        let slot = waker.slot(token.slot as usize);
        assert!(matches!(CrossProcessWaker::timed_out(slot), Err(WakerError::Timeout)));
        assert_eq!(slot.state.load(Ordering::Acquire), STATE_RESERVED, "the timeout took the slot back");
        slot.state.store(STATE_PARKED, Ordering::Release);
        assert_eq!(waker.wake_up_to(1), 1, "a wake lands after the deadline check");
        assert!(CrossProcessWaker::timed_out(slot).is_ok(), "the landed wake makes the wait succeed");
        waker.release(token);
        assert_eq!(slot.state.load(Ordering::Acquire), STATE_FREE);
        assert_eq!(waker.header().parked_mask.load(Ordering::Acquire), 0);
    }

    #[test]
    fn anon_round_trip() {
        let waker = CrossProcessWaker::create_anon(4).expect("create");
        let token = waker.try_park(10).expect("park");
        let waker_c = Arc::new(waker);
        let waker_p = Arc::clone(&waker_c);
        let h = thread::spawn(move || {
            thread::sleep(Duration::from_millis(20));
            let n = waker_p.wake_up_to(15);
            assert!(n >= 1);
        });
        waker_c.wait(token, Some(Duration::from_secs(30))).expect("wake");
        h.join().unwrap();
    }

    #[test]
    fn wait_returns_immediately_if_already_woken() {
        let waker = CrossProcessWaker::create_anon(2).expect("create");
        let token = waker.try_park(5).expect("park");
        assert_eq!(waker.wake_up_to(10), 1);
        let t0 = Instant::now();
        waker.wait(token, Some(Duration::from_secs(1))).expect("ok");
        assert!(t0.elapsed() < Duration::from_millis(50),
                "wait should return fast since wake fired before wait entered");
    }

    #[test]
    fn timeout_works() {
        let waker = CrossProcessWaker::create_anon(2).expect("create");
        let token = waker.try_park(100).expect("park");
        let t0 = Instant::now();
        let err = waker.wait(token, Some(Duration::from_millis(80)));
        assert!(matches!(err, Err(WakerError::Timeout)));
        assert!(t0.elapsed() >= Duration::from_millis(70));
    }

    /// A platform wait asked for less than a millisecond blocks rather than
    /// returning at once. Windows may end the first early, at a timer tick,
    /// so a run of them is held to the timeouts of all but one.
    #[test]
    fn platform_waits_under_a_millisecond_block() {
        const WAITS: u32 = 8;
        let timeout = Duration::from_micros(500);
        let word = AtomicU32::new(0);
        let started = Instant::now();
        for _ in 0..WAITS {
            platform_wait::wait_with_timeout(&word, 0, timeout, false, None);
        }
        let elapsed = started.elapsed();
        assert!(elapsed >= timeout * (WAITS - 1), "{WAITS} waits of {timeout:?} took {elapsed:?}");
    }

    #[test]
    fn full_when_all_slots_taken() {
        let waker = CrossProcessWaker::create_anon(2).expect("create");
        let _a = waker.try_park(1).expect("park 0");
        let _b = waker.try_park(2).expect("park 1");
        assert!(matches!(waker.try_park(3), Err(WakerError::Full)));
    }

    #[test]
    fn release_lets_others_park() {
        let waker = CrossProcessWaker::create_anon(2).expect("create");
        let a = waker.try_park(1).expect("park 0");
        let _b = waker.try_park(2).expect("park 1");
        waker.release(a);
        let _c = waker.try_park(3).expect("re-park 0");
    }

    #[test]
    fn wake_all_drains_blocked_consumers() {
        let waker = Arc::new(CrossProcessWaker::create_anon(4).expect("create"));
        let mut handles = Vec::new();
        for target in 0..4u64 {
            let w = Arc::clone(&waker);
            let token = w.try_park(target + 1000).expect("park");
            handles.push(thread::spawn(move || {
                w.wait(token, Some(Duration::from_secs(30))).expect("woken");
            }));
        }
        thread::sleep(Duration::from_millis(20));
        assert_eq!(waker.wake_all(), 4);
        for h in handles { h.join().unwrap(); }
    }

    /// Rounds the peer process parks through.
    const XPROC_ROUNDS: u64 = 10_000;
    /// Rounds each in-process exchange parks through.
    const INPROC_ROUNDS: u64 = 1_000;
    /// How long one test wait may run before its wake counts as lost.
    const LOST_WAKE: Duration = Duration::from_secs(30);
    /// The kernel-park timeout test's timeout, and how late past it the
    /// wait may return.
    const PARK_TIMEOUT: Duration = Duration::from_millis(200);
    const PARK_TIMEOUT_SLACK: Duration = Duration::from_secs(5);

    fn tmp(name: &str) -> crate::test_paths::TmpFile {
        crate::test_paths::TmpFile::new(format!("subetha-waker-{name}-{}", std::process::id()))
    }

    fn random_seed() -> u64 {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos() as u64)
            .expect("the clock reads after the epoch");
        nanos | 1
    }

    /// A xorshift step: the tests need spread, not quality.
    fn next_random(state: &mut u64) -> u64 {
        let mut x = *state;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        *state = x;
        x
    }

    /// Park on slot 0 at targets 1 to `rounds` and wait each one out,
    /// returning a line for every wait a wake did not end. A wait that
    /// runs to its timeout counts even when it returns `Ok`: its final
    /// check sees a slot a waker exchanged without waking it.
    fn park_rounds(waiter: &CrossProcessWaker, rounds: u64) -> Vec<String> {
        let mut problems = Vec::new();
        for round in 0..rounds {
            let token = match waiter.try_park(round + 1) {
                Ok(token) => token,
                Err(e) => {
                    problems.push(format!("round {round}: no slot to park in: {e}"));
                    continue;
                }
            };
            let start = Instant::now();
            let result = waiter.wait(token, Some(LOST_WAKE));
            let took = start.elapsed();
            if result.is_err() || took >= LOST_WAKE {
                problems.push(format!("round {round}: the wait ended in {result:?} after {took:?}"));
            }
        }
        problems
    }

    /// Wake the waiter parked on slot 0 in `round`, once its state and
    /// its parked-mask bit are both set, since a wake scan reads the mask.
    /// Even rounds wake at a random point up to twice the monitor budget;
    /// odd rounds once the waiter sleeps in the kernel.
    fn wake_round(waker: &CrossProcessWaker, round: u64, rng: &mut u64) {
        let target = round + 1;
        let slot = waker.slot(0);
        let since = Instant::now();
        while slot.state.load(Ordering::Acquire) != STATE_PARKED
            || slot.target_seq.load(Ordering::Relaxed) != target
            || waker.header().parked_mask.load(Ordering::Acquire) & 1 == 0
        {
            assert!(
                since.elapsed() < LOST_WAKE,
                "round {round}: the waiter was not seen parked within {LOST_WAKE:?}; its last wake was lost or it has gone"
            );
            std::hint::spin_loop();
        }
        let budget = crate::monitor_wait::monitor_wait_budget_cycles();
        let spin = |cycles: u64| {
            let start = crate::ordering::read_tsc();
            while crate::ordering::read_tsc().wrapping_sub(start) < cycles {
                std::hint::spin_loop();
            }
        };
        if round.is_multiple_of(2) {
            spin(next_random(rng) % (2 * budget + 1));
        } else {
            #[cfg(windows)]
            while slot.park_event.load(Ordering::SeqCst) == 0 {
                assert!(
                    since.elapsed() < LOST_WAKE,
                    "round {round}: the waiter never reached its park event"
                );
                std::hint::spin_loop();
            }
            #[cfg(not(windows))]
            spin(2 * budget);
        }
        assert_eq!(waker.wake_up_to(target), 1, "round {round}: the wake found its waiter");
    }

    /// `waiter` parks through `INPROC_ROUNDS` on a thread of its own while
    /// `waker`, a second instance over the same backing, wakes it. On
    /// Windows the waiter's heal is off, so a lost event shows as a wait
    /// that runs to its timeout.
    fn exchange_in_process(waiter: CrossProcessWaker, waker: &CrossProcessWaker) {
        #[cfg(windows)]
        waiter.park.turn_heal_off();
        let parked = thread::spawn(move || park_rounds(&waiter, INPROC_ROUNDS));
        let seed = random_seed();
        println!("seed {seed}");
        let mut rng = seed;
        for round in 0..INPROC_ROUNDS {
            wake_round(waker, round, &mut rng);
        }
        let problems = parked.join().expect("the waiting thread");
        assert!(problems.is_empty(), "seed {seed}: {problems:?}");
    }

    /// Every wake crosses from one instance's slot table to the other's,
    /// as it does between processes.
    #[test]
    fn a_file_backed_wait_is_woken_through_a_second_instance() {
        let path = tmp("two-instances");
        let waiter = CrossProcessWaker::create(&path, 1).expect("create");
        let waker = CrossProcessWaker::open(&path, 1).expect("open");
        exchange_in_process(waiter, &waker);
    }

    #[test]
    fn an_open_at_another_capacity_names_both() {
        let path = tmp("capacity");
        let waker = CrossProcessWaker::create(&path, 4).expect("create");
        assert!(matches!(
            CrossProcessWaker::open(&path, 2),
            Err(WakerError::LayoutMismatch(crate::layout::Mismatch { field: "capacity", found: 4, wanted: 2 }))
        ));
        drop(waker);
    }

    #[test]
    fn a_shm_backed_wait_is_woken_through_a_second_instance() {
        let name = format!("waker_two_instances_{}", std::process::id());
        let size = waker_region_size(1);
        let region = crate::shm_file::ShmFile::create_or_open_named(&name, size).expect("create region");
        let second = crate::shm_file::ShmFile::create_or_open_named(&name, size).expect("open region");
        let waiter = CrossProcessWaker::create_from_shm(region, 1).expect("create");
        let waker = CrossProcessWaker::open_from_shm(second, 1).expect("open");
        exchange_in_process(waiter, &waker);
    }

    /// The peer half of the cross-process test: parks through every round
    /// when the test binary is re-run with `SUBETHA_WAKER_PEER` naming the
    /// waker file, and passes at once otherwise.
    #[test]
    fn waker_peer_role() {
        let Some(path) = std::env::var_os("SUBETHA_WAKER_PEER") else {
            return;
        };
        let waiter = CrossProcessWaker::open(&path, 1).expect("the peer opens the waker");
        #[cfg(windows)]
        waiter.park.turn_heal_off();
        let problems = park_rounds(&waiter, XPROC_ROUNDS);
        for problem in &problems {
            eprintln!("waker peer: {problem}");
        }
        std::process::exit(if problems.is_empty() { 0 } else { 1 });
    }

    /// Kills the peer when the test fails before reaping it, so a failed
    /// run leaves no process parking on its own.
    struct Peer(Option<std::process::Child>);

    impl Peer {
        fn reap(mut self) -> std::process::ExitStatus {
            let mut child = self.0.take().expect("the peer is reaped once");
            child.wait().expect("the peer is waited on")
        }
    }

    impl Drop for Peer {
        fn drop(&mut self) {
            if let Some(mut child) = self.0.take() {
                if let Err(e) = child.kill() {
                    eprintln!("the waker peer was not killed: {e}");
                }
                if let Err(e) = child.wait() {
                    eprintln!("the waker peer was not reaped: {e}");
                }
            }
        }
    }

    /// A waiter in another process is woken every round; on Windows half
    /// of the rounds go through its park event.
    #[test]
    fn a_wait_is_woken_from_another_process() {
        let path = tmp("xproc");
        let waker = CrossProcessWaker::create(&path, 1).expect("create");
        let peer = Peer(Some(
            std::process::Command::new(std::env::current_exe().expect("the test binary's own path"))
                .arg("cross_process_waker::tests::waker_peer_role")
                .arg("--exact")
                .arg("--nocapture")
                .env("SUBETHA_WAKER_PEER", &*path)
                .spawn()
                .expect("the peer process spawns"),
        ));
        let seed = random_seed();
        println!("seed {seed}");
        let mut rng = seed;
        for round in 0..XPROC_ROUNDS {
            wake_round(&waker, round, &mut rng);
        }
        let status = peer.reap();
        assert!(status.success(), "seed {seed}: the peer reported lost wakes: {status}");
    }

    /// A wait that outlasts the monitor budget ends at its timeout. On
    /// Windows a set already on its park event, left by a wake whose
    /// waiter had gone, does not end it early, and the wait's first sleep
    /// on the event consumes it.
    #[test]
    fn a_kernel_parked_wait_times_out_on_time() {
        let path = tmp("timeout");
        let waker = CrossProcessWaker::create(&path, 1).expect("create");
        #[cfg(windows)]
        let park = &waker.park;
        #[cfg(windows)]
        {
            // A park on a slot that is not parked makes the slot's event
            // and returns at once.
            let slot = waker.slot(0);
            assert!(matches!(
                park.park(0, &slot.state, STATE_PARKED, &slot.park_event, None),
                Some(Ok(()))
            ));
            park.signal(0, park.own_id(0).expect("the park made the event"));
        }
        let token = waker.try_park(1).expect("park");
        let start = Instant::now();
        assert!(matches!(waker.wait(token, Some(PARK_TIMEOUT)), Err(WakerError::Timeout)));
        let took = start.elapsed();
        assert!(took >= PARK_TIMEOUT, "the wait returned {took:?} into its {PARK_TIMEOUT:?}");
        assert!(
            took < PARK_TIMEOUT + PARK_TIMEOUT_SLACK,
            "the wait overran its {PARK_TIMEOUT:?} to {took:?}"
        );
        #[cfg(windows)]
        assert!(!park.take_set(0), "the wait slept on its event and consumed the late set");
    }

    /// A wake that sets no event, like one from a waker that dies between
    /// its exchange and its set, still ends the wait at the waiter's next
    /// check of its slot.
    #[cfg(windows)]
    #[test]
    fn a_wake_that_sets_no_event_is_healed() {
        let path = tmp("heal");
        let waiter = Arc::new(CrossProcessWaker::create(&path, 1).expect("create"));
        let token = waiter.try_park(1).expect("park");
        let parked = {
            let waiter = Arc::clone(&waiter);
            thread::spawn(move || {
                let start = Instant::now();
                let result = waiter.wait(token, Some(LOST_WAKE));
                (result, start.elapsed())
            })
        };
        let slot = waiter.slot(0);
        let since = Instant::now();
        while slot.park_event.load(Ordering::SeqCst) == 0 {
            assert!(since.elapsed() < LOST_WAKE, "the waiter never reached its park event");
            std::hint::spin_loop();
        }
        // The exchange a waker makes, without the set that follows it.
        let exchanged = slot.state.compare_exchange(
            STATE_PARKED,
            STATE_WOKEN,
            Ordering::SeqCst,
            Ordering::Relaxed,
        );
        exchanged.expect("the slot was parked when the waker exchanged it");
        let (result, took) = parked.join().expect("the waiting thread");
        result.expect("the heal ends the wait as woken");
        assert!(took < LOST_WAKE, "the wait ran to its timeout instead of healing: {took:?}");
    }

    /// A timed wait on an anonymous waker sleeps on its slot's event once
    /// it outlasts the monitor budget, and a wake sets that event. The
    /// unnamed event has no heal, so a lost set shows as a timeout.
    #[cfg(windows)]
    #[test]
    fn a_timed_anon_wait_sleeps_on_its_event_and_a_wake_ends_it() {
        let waiter = Arc::new(CrossProcessWaker::create_anon(1).expect("create"));
        let token = waiter.try_park(1).expect("park");
        let parked = {
            let waiter = Arc::clone(&waiter);
            thread::spawn(move || {
                let start = Instant::now();
                let result = waiter.wait(token, Some(LOST_WAKE));
                (result, start.elapsed())
            })
        };
        let slot = waiter.slot(0);
        let since = Instant::now();
        while slot.park_event.load(Ordering::SeqCst) == 0 {
            assert!(since.elapsed() < LOST_WAKE, "the waiter never reached its park event");
            std::hint::spin_loop();
        }
        assert_eq!(waiter.wake_up_to(1), 1, "the wake found its waiter");
        let (result, took) = parked.join().expect("the waiting thread");
        result.expect("the wake ends the wait");
        assert!(took < LOST_WAKE, "the set was lost and the wait ran to its timeout: {took:?}");
    }

    /// An untimed wait on an anonymous waker stays on the state word and
    /// makes no event.
    #[cfg(windows)]
    #[test]
    fn an_untimed_anon_wait_makes_no_event() {
        let waiter = Arc::new(CrossProcessWaker::create_anon(1).expect("create"));
        let token = waiter.try_park(1).expect("park");
        let parked = {
            let waiter = Arc::clone(&waiter);
            thread::spawn(move || waiter.wait(token, None))
        };
        thread::sleep(Duration::from_millis(20));
        assert_eq!(waiter.slot(0).park_event.load(Ordering::SeqCst), 0);
        assert_eq!(waiter.wake_up_to(1), 1, "the wake found its waiter");
        parked.join().expect("the waiting thread").expect("the wake ends the wait");
        assert!(waiter.park.own_id(0).is_none(), "the untimed wait made no event");
    }

    /// A timed wait on an anonymous waker that runs to its deadline ends on
    /// the thread's high-resolution timer, and without one on the event's
    /// own timeout, and either way not before the deadline.
    #[cfg(windows)]
    #[test]
    fn a_timed_anon_wait_times_out_on_the_timer_or_without_one() {
        let waker = CrossProcessWaker::create_anon(1).expect("create");
        let timeout = Duration::from_millis(10);
        let timed_out = || {
            let token = waker.try_park(1).expect("park");
            let start = Instant::now();
            let result = waker.wait(token, Some(timeout));
            (result, start.elapsed())
        };
        let ends = crate::park_event::timer_ends();
        let (result, took) = timed_out();
        assert!(matches!(result, Err(WakerError::Timeout)), "{result:?}");
        assert!(took >= timeout, "the wait returned {took:?} into its {timeout:?}");
        assert!(crate::park_event::timer_ends() > ends, "the timer ended the sleep");
        let ends = crate::park_event::timer_ends();
        let (result, took) = crate::park_event::without_timer(timed_out);
        assert!(matches!(result, Err(WakerError::Timeout)), "{result:?}");
        assert!(took >= timeout, "the wait returned {took:?} into its {timeout:?}");
        assert_eq!(crate::park_event::timer_ends(), ends, "no timer ended a sleep");
    }
}
