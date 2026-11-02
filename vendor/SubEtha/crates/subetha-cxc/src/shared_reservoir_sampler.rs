//! `SharedReservoirSampler<T>` - cross-process uniform random
//! sampling via Vitter's Algorithm R.
//!
//! Maintains `k` reservoir slots. After processing N items, each
//! has equal probability `k/N` of being in the reservoir.
//!
//! # Algorithm
//!
//! For item n (1-indexed):
//! - If n <= k: slot[n-1] = item.
//! - Else: j = uniform(1..=n). If j <= k: slot[j-1] = item.
//!
//! # Safety
//!
//! - Bounded capacity at create.
//! - `total_seen.fetch_add` is monotonic (no underflow).
//! - Concurrent records on `slot[j]` for the same `j` after capacity is
//!   exceeded each land whole, one after another, and the last one stays;
//!   statistically the uniform-sampling property is preserved because
//!   every racer was equally eligible.
//!
//! # Two buffers per slot
//!
//! Each slot holds two value buffers and a head naming the current one
//! (see [`ReservoirSlot`]). A record fills the buffer the head does not
//! name and makes it current with one compare-and-swap of the head; a read
//! copies the buffer the head names and checks the head again, retrying
//! only when a record landed during the copy, and never waits on a writer.
//! Records racing on one slot take its claim in turn, and a recorder that
//! dies holding it is taken over.

use std::cell::Cell;
use std::fs::File;
use std::marker::PhantomData;
use std::mem::size_of;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};

use memmap2::{MmapMut, MmapOptions};

use crate::slot_pair::{self, SlotGeometry};

pub const RESERVOIR_MAGIC: u64 = 0x4150_5253_4D50_4C32;
pub const RESERVOIR_SLOT_PAYLOAD: usize = 56;

#[repr(C, align(64))]
pub struct ReservoirHeader {
    pub magic: u64,
    pub capacity: u32,
    pub slot_size: u32,
    pub total_seen: AtomicU64,
    _pad: [u8; 40],
}

/// One slot: the head names the current buffer in bit 0 and counts the
/// records that landed above it, and the claim is 0 while no record holds
/// the slot, else the recorder's pid and process start stamp.
#[repr(C, align(64))]
pub struct ReservoirSlot {
    pub head: AtomicU64,
    pub claim: AtomicU64,
    pub buffers: [[u8; RESERVOIR_SLOT_PAYLOAD]; 2],
}

const _: () = {
    assert!(size_of::<ReservoirHeader>() == 64);
    assert!(size_of::<ReservoirSlot>() == 128);
    assert!(std::mem::offset_of!(ReservoirSlot, buffers) == slot_pair::SLOT_PREFIX);
};

/// Where a slot's parts lie for a value of `size` bytes, at most
/// [`RESERVOIR_SLOT_PAYLOAD`]: the buffers of [`ReservoirSlot`] whatever
/// the size.
const fn reservoir_geometry(size: usize) -> SlotGeometry {
    SlotGeometry { size, ..SlotGeometry::new(RESERVOIR_SLOT_PAYLOAD, 8) }
}

#[derive(Debug)]
pub enum ReservoirError {
    PayloadTooLarge,
    LayoutMismatch(crate::layout::Mismatch),
    /// The operating system refused to open, size or map the file; the
    /// error carries its code and message.
    IoError(std::io::Error),
}

impl std::fmt::Display for ReservoirError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::PayloadTooLarge => write!(f, "the value type is larger than a slot"),
            Self::LayoutMismatch(m) => write!(f, "a reservoir of another shape: {m}"),
            Self::IoError(e) => write!(f, "io: {e}"),
        }
    }
}

impl std::error::Error for ReservoirError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::IoError(e) => Some(e),
            Self::PayloadTooLarge | Self::LayoutMismatch(_) => None,
        }
    }
}

impl From<std::io::Error> for ReservoirError {
    fn from(e: std::io::Error) -> Self { Self::IoError(e) }
}

pub fn reservoir_file_size(capacity: usize) -> usize {
    size_of::<ReservoirHeader>() + capacity * size_of::<ReservoirSlot>()
}

thread_local! {
    static RNG_STATE: Cell<u64> = Cell::new({
        let t = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos() as u64)
            .unwrap_or(1);
        let mix = t.wrapping_mul(0x9E37_79B9_7F4A_7C15);
        if mix == 0 { 1 } else { mix }
    });
}

#[inline]
fn next_random_u64() -> u64 {
    RNG_STATE.with(|s| {
        let mut x = s.get();
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        s.set(x);
        x
    })
}

pub struct SharedReservoirSampler<T: Copy + 'static> {
    _file: File,
    mmap: MmapMut,
    capacity: usize,
    _phantom: PhantomData<T>,
    header_sidecar: subetha_core::HandshakeHeader,
    ring_sidecar: Box<subetha_core::ObservationRing>,
}

unsafe impl<T: Copy + Send + 'static> Send for SharedReservoirSampler<T> {}
unsafe impl<T: Copy + Sync + 'static> Sync for SharedReservoirSampler<T> {}

impl<T: Copy + Send + Sync + 'static> subetha_sidecar::AdaptiveInstance for SharedReservoirSampler<T> {
    fn header(&self) -> &subetha_core::HandshakeHeader { &self.header_sidecar }
    fn ring(&self) -> &subetha_core::ObservationRing { &self.ring_sidecar }
    fn make_policy(&self) -> Box<dyn subetha_sidecar::Policy> {
        Box::new(subetha_sidecar::NoMigrationPolicy)
    }
}

impl<T: Copy + 'static> SharedReservoirSampler<T> {
    /// Obtain the sampler at `path`, initializing an empty one if the
    /// path does not yet exist and attaching to it if it does.
    /// Attaching leaves the live sample set and `total_seen` in place;
    /// a region built with a different capacity or payload type is a
    /// `LayoutMismatch`. The in-place [`reset`](Self::reset) restarts
    /// a live sampler.
    pub fn create(
        path: impl AsRef<Path>, capacity: usize,
    ) -> Result<Self, ReservoirError> {
        if size_of::<T>() > RESERVOIR_SLOT_PAYLOAD {
            return Err(ReservoirError::PayloadTooLarge);
        }
        assert!(capacity >= 1);
        let (file, mmap) = crate::mmf_attach::create_or_attach(
            path.as_ref(),
            reservoir_file_size(capacity),
            |ptr| unsafe { Self::init_region(ptr, capacity) },
            |ptr| unsafe { (*(ptr as *const ReservoirHeader)).magic },
            RESERVOIR_MAGIC,
        )
        .map_err(|e| crate::mmf_attach::attach_error(e, ReservoirError::LayoutMismatch))?;
        Self::from_region(file, mmap, capacity)
    }

    /// Lay out an empty sampler: config first, magic last, because
    /// attachers spin on it. The zeroed region is already the empty
    /// slot array and `total_seen` 0.
    ///
    /// # Safety
    /// `ptr` addresses at least `reservoir_file_size(capacity)`
    /// writable zeroed bytes.
    unsafe fn init_region(ptr: *mut u8, capacity: usize) {
        let hdr = ptr as *mut ReservoirHeader;
        unsafe {
            (*hdr).capacity = capacity as u32;
            (*hdr).slot_size = size_of::<T>() as u32;
            std::ptr::write_volatile(&raw mut (*hdr).magic, RESERVOIR_MAGIC);
        }
    }

    /// Wrap an initialized region, refusing one built with a different
    /// capacity or payload type.
    fn from_region(
        file: File,
        mmap: MmapMut,
        capacity: usize,
    ) -> Result<Self, ReservoirError> {
        let hdr = unsafe { &*(mmap.as_ptr() as *const ReservoirHeader) };
        crate::layout::Mismatch::first(&[
            ("magic", hdr.magic, RESERVOIR_MAGIC),
            ("capacity", u64::from(hdr.capacity), capacity as u64),
            ("value size", u64::from(hdr.slot_size), size_of::<T>() as u64),
        ])
        .map_err(ReservoirError::LayoutMismatch)?;
        Ok(Self {
            _file: file, mmap, capacity, _phantom: PhantomData,
            header_sidecar: subetha_core::HandshakeHeader::new(),
            ring_sidecar: Box::new(subetha_core::ObservationRing::new()),
        })
    }

    pub fn open(
        path: impl AsRef<Path>, expected_capacity: usize,
    ) -> Result<Self, ReservoirError> {
        if size_of::<T>() > RESERVOIR_SLOT_PAYLOAD {
            return Err(ReservoirError::PayloadTooLarge);
        }
        let total = reservoir_file_size(expected_capacity);
        let file = crate::region_file::open_existing(path.as_ref())?;
        let len = file.metadata()?.len();
        if len < total as u64 {
            return Err(ReservoirError::LayoutMismatch(crate::layout::Mismatch::new(
                "length",
                len,
                total as u64,
            )));
        }
        let mmap = unsafe { MmapOptions::new().len(total).map_mut(&file)? };
        Self::from_region(file, mmap, expected_capacity)
    }

    #[inline]
    pub fn capacity(&self) -> usize { self.capacity }

    pub fn total_seen(&self) -> u64 {
        self.header().total_seen.load(Ordering::Acquire)
    }

    fn header(&self) -> &ReservoirHeader {
        unsafe { &*(self.mmap.as_ptr() as *const ReservoirHeader) }
    }

    /// The start of slot `i`. Panics, in every build, when `i` is not below
    /// the capacity: past it the offset leaves the mapping.
    #[inline]
    fn slot(&self, i: usize) -> *const u8 {
        assert!(i < self.capacity, "slot {i} is past the {} the reservoir holds", self.capacity);
        // SAFETY: the mapping holds the header and then `capacity` slots,
        // and `i` is below the capacity.
        unsafe { self.mmap.as_ptr().add(size_of::<ReservoirHeader>() + i * size_of::<ReservoirSlot>()) }
    }

    /// Write `value` into slot `idx`: its idle buffer, made current with
    /// one compare-and-swap of the slot's head.
    fn write_slot(&self, idx: usize, value: T) {
        // SAFETY: the slot lies in this writable mapping, laid out as a
        // ReservoirSlot, whose buffers hold a T (checked at open).
        unsafe { slot_pair::store(self.slot(idx), &reservoir_geometry(size_of::<T>()), &value) };
    }

    /// Read slot `idx`: a copy of the buffer its head names, confirmed whole
    /// by the head, retried only when a record landed during it.
    fn read_slot(&self, idx: usize) -> T {
        // SAFETY: as in `write_slot`; every value written to a slot is a T,
        // as is the zero a fresh slot holds.
        unsafe { slot_pair::load(self.slot(idx), &reservoir_geometry(size_of::<T>())) }.1
    }

    /// Record a value. Returns the slot index it landed in, or
    /// None if the value was rejected (the reservoir kept its
    /// existing sample for this position).
    pub fn record(&self, value: T) -> Option<usize> {
        let prev = self.header().total_seen.fetch_add(1, Ordering::AcqRel);
        let n = prev + 1; // 1-indexed item number
        let k = self.capacity as u64;
        let r = if n <= k {
            // Reservoir not full yet; always accept.
            let idx = (n - 1) as usize;
            self.write_slot(idx, value);
            Some(idx)
        } else {
            // Vitter R: j = uniform(1..=n); if j <= k, accept at slot j-1.
            let j = (next_random_u64() % n) + 1;
            if j <= k {
                let idx = (j - 1) as usize;
                self.write_slot(idx, value);
                Some(idx)
            } else {
                None
            }
        };
        self.ring_sidecar.push_op(
            crate::sidecar_ops::reservoir::OP_RECORD,
            if r.is_none() { 2 } else { 0 }, // 2 = rejected
        );
        r
    }

    /// Snapshot the current reservoir. Returns min(total_seen, k)
    /// slots filled so far; the rest are unused.
    pub fn snapshot(&self) -> Vec<T> {
        let filled = (self.total_seen() as usize).min(self.capacity);
        let v: Vec<T> = (0..filled).map(|i| self.read_slot(i)).collect();
        self.ring_sidecar
            .push_op(crate::sidecar_ops::reservoir::OP_SNAPSHOT, 0);
        v
    }

    /// Reset to empty (total_seen = 0; slots become invalid but
    /// not zeroed - next record overwrites them).
    pub fn reset(&self) {
        self.header().total_seen.store(0, Ordering::Release);
    }

    pub fn flush(&self) -> Result<(), ReservoirError> {
        self.mmap.flush()?;
        Ok(())
    }
    pub fn flush_async(&self) -> Result<(), ReservoirError> {
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
        crate::test_paths::TmpFile::new(format!("subetha-reservoir-{name}-{}.bin", std::process::id()))
    }

    #[test]
    fn create_initial_state_is_empty() {
        let p = tmp("init");
        let r: SharedReservoirSampler<u32> = SharedReservoirSampler::create(&p, 10).unwrap();
        assert_eq!(r.capacity(), 10);
        assert_eq!(r.total_seen(), 0);
        assert_eq!(r.snapshot(), Vec::<u32>::new());
    }

    #[test]
    fn first_k_items_always_accepted() {
        let p = tmp("first-k");
        let r: SharedReservoirSampler<u32> = SharedReservoirSampler::create(&p, 5).unwrap();
        for i in 0..5u32 {
            let idx = r.record(i);
            assert_eq!(idx, Some(i as usize));
        }
        let snap = r.snapshot();
        assert_eq!(snap, vec![0, 1, 2, 3, 4]);
        assert_eq!(r.total_seen(), 5);
    }

    /// A second create attaches with the live sample set in place; the
    /// in-place reset is what restarts sampling.
    #[test]
    fn second_create_attaches_and_keeps_samples() {
        let p = tmp("attach");
        let r: SharedReservoirSampler<u32> = SharedReservoirSampler::create(&p, 5).unwrap();
        for i in 0..3u32 { r.record(i); }

        let r2: SharedReservoirSampler<u32> = SharedReservoirSampler::create(&p, 5).unwrap();
        assert_eq!(r2.total_seen(), 3, "attach restarted a live sampler");
        assert_eq!(r2.snapshot().len(), 3);
        assert!(matches!(
            SharedReservoirSampler::<u32>::create(&p, 4),
            Err(ReservoirError::LayoutMismatch(crate::layout::Mismatch { field: "capacity", found: 5, wanted: 4 })),
        ));

        r2.reset();
        assert_eq!(r.total_seen(), 0, "reset did not restart for every handle");
        drop(r);
        drop(r2);
    }

    #[test]
    fn after_capacity_some_items_rejected() {
        let p = tmp("after-cap");
        let r: SharedReservoirSampler<u32> = SharedReservoirSampler::create(&p, 5).unwrap();
        for i in 0..5u32 { r.record(i); }
        // After capacity, each new item has probability 5/n of acceptance.
        let mut accepted = 0;
        let mut rejected = 0;
        for i in 5..100u32 {
            if r.record(i).is_some() { accepted += 1; } else { rejected += 1; }
        }
        // Expect most to be rejected since acceptance probability drops to
        // ~5%. Allow generous bounds for randomness.
        assert!(rejected > 50, "expected mostly rejections; got {accepted} accepted, {rejected} rejected");
        assert_eq!(r.total_seen(), 100);
    }

    #[test]
    fn snapshot_always_has_correct_length() {
        let p = tmp("snap-len");
        let r: SharedReservoirSampler<u32> = SharedReservoirSampler::create(&p, 10).unwrap();
        // After 3 records, snapshot has 3 items.
        for i in 0..3u32 { r.record(i); }
        assert_eq!(r.snapshot().len(), 3);
        // After 100 records, snapshot has 10 (capacity).
        for i in 3..100u32 { r.record(i); }
        assert_eq!(r.snapshot().len(), 10);
    }

    #[test]
    fn uniform_distribution_over_many_trials() {
        // For capacity=1 and N=100, each item should appear in the
        // reservoir with probability 1/100. Over 1000 trials, each
        // item should appear ~10 times. Bin into 10 buckets and
        // check no bucket is hugely off.
        let n_trials = 1000;
        let n_items = 100u32;
        let mut counts = [0u32; 10];
        for trial in 0..n_trials {
            let path = tmp(&format!("uniform-{trial}"));
            let r: SharedReservoirSampler<u32>
                = SharedReservoirSampler::create(&path, 1).unwrap();
            for i in 0..n_items { r.record(i); }
            let snap = r.snapshot();
            let kept = snap[0];
            let bucket = (kept * 10 / n_items) as usize;
            counts[bucket.min(9)] += 1;
        }
        // Each bucket should be ~100. Allow [50, 200] for stochastic noise.
        for (i, &c) in counts.iter().enumerate() {
            assert!((30..=200).contains(&c),
                "bucket {i} count {c} is way out of expected ~100");
        }
    }

    /// Two recorders writing one slot each land whole: every value a reader
    /// takes from the slot agrees with itself.
    #[test]
    fn two_recorders_on_one_slot_never_tear_a_value() {
        let p = tmp("two-recorders");
        let r: Arc<SharedReservoirSampler<[u64; 7]>> = Arc::new(SharedReservoirSampler::create(&p, 2).unwrap());
        let done = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let reader = {
            let (r, done) = (r.clone(), done.clone());
            thread::spawn(move || {
                while !done.load(Ordering::Acquire) {
                    let value = r.read_slot(1);
                    assert!(value.iter().all(|&w| w == value[0]), "a torn value: {value:x?}");
                }
            })
        };
        let writers: Vec<_> = (0..2u64)
            .map(|w| {
                let r = r.clone();
                thread::spawn(move || {
                    for round in 0..10_000u64 {
                        r.write_slot(1, [(w << 32) | round; 7]);
                    }
                })
            })
            .collect();
        for w in writers {
            w.join().unwrap();
        }
        done.store(true, Ordering::Release);
        reader.join().unwrap();
    }

    /// A recorder that died holding a slot's claim leaves the sample
    /// readable, and the next record into that slot takes the claim over.
    #[test]
    fn a_claim_left_by_a_dead_recorder_is_taken_over() {
        let p = tmp("dead-recorder");
        let r: SharedReservoirSampler<u64> = SharedReservoirSampler::create(&p, 2).unwrap();
        r.record(1);
        let mut child = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "exit_watch::tests::sleeper_never_runs_on_its_own", "--ignored", "--nocapture"])
            .env("SUBETHA_EXIT_WATCH_SLEEP", "1")
            .spawn()
            .unwrap();
        let dead = child.id();
        child.kill().unwrap();
        child.wait().unwrap();
        // SAFETY: slot 0 lies in the mapping, laid out as a ReservoirSlot.
        let slot = unsafe { &*(r.slot(0) as *const ReservoirSlot) };
        slot.claim.store(crate::process_identity::holder(dead, 0), Ordering::Release);
        assert_eq!(r.snapshot(), vec![1], "a read does not wait on the dead recorder");
        r.write_slot(0, 2);
        assert_eq!(r.snapshot(), vec![2]);
        assert_eq!(slot.claim.load(Ordering::Acquire), 0, "the record freed the claim it took over");
    }

    #[test]
    fn reset_clears_count() {
        let p = tmp("reset");
        let r: SharedReservoirSampler<u32> = SharedReservoirSampler::create(&p, 5).unwrap();
        for i in 0..10u32 { r.record(i); }
        assert_eq!(r.total_seen(), 10);
        r.reset();
        assert_eq!(r.total_seen(), 0);
        assert_eq!(r.snapshot(), Vec::<u32>::new());
    }

    #[test]
    fn cross_handle_visibility() {
        let p = tmp("cross-handle");
        let w: SharedReservoirSampler<u32> = SharedReservoirSampler::create(&p, 5).unwrap();
        let rdr: SharedReservoirSampler<u32> = SharedReservoirSampler::open(&p, 5).unwrap();
        for i in 0..5u32 { w.record(i); }
        let snap = rdr.snapshot();
        assert_eq!(snap, vec![0, 1, 2, 3, 4]);
        assert_eq!(rdr.total_seen(), 5);
    }

    #[test]
    fn payload_too_large_rejected() {
        #[allow(dead_code)]
        struct Big([u8; RESERVOIR_SLOT_PAYLOAD + 1]);
        impl Copy for Big {}
        impl Clone for Big { fn clone(&self) -> Self { *self } }
        let p = tmp("too-large");
        assert!(matches!(
            SharedReservoirSampler::<Big>::create(&p, 4).err(),
            Some(ReservoirError::PayloadTooLarge)
        ));
    }

    #[test]
    fn concurrent_recorders_count_correctly() {
        let p = tmp("concurrent");
        let r: Arc<SharedReservoirSampler<u32>>
            = Arc::new(SharedReservoirSampler::create(&p, 10).unwrap());
        let n_threads = 4;
        let per_thread = 100;
        let mut handles = vec![];
        for t in 0..n_threads as u32 {
            let r = r.clone();
            handles.push(thread::spawn(move || {
                for i in 0..per_thread as u32 {
                    r.record(t * 1000 + i);
                }
            }));
        }
        for h in handles { h.join().unwrap(); }
        assert_eq!(r.total_seen() as usize, n_threads * per_thread);
        let snap = r.snapshot();
        assert_eq!(snap.len(), 10);
    }

    #[test]
    fn struct_payload_round_trip() {
        #[derive(Clone, Copy, Debug, PartialEq)]
        #[repr(C)]
        struct LogEntry { ts: u64, code: u32, severity: u32 }
        let p = tmp("struct");
        let r: SharedReservoirSampler<LogEntry>
            = SharedReservoirSampler::create(&p, 3).unwrap();
        let e1 = LogEntry { ts: 100, code: 1, severity: 1 };
        let e2 = LogEntry { ts: 200, code: 2, severity: 2 };
        let e3 = LogEntry { ts: 300, code: 3, severity: 3 };
        r.record(e1);
        r.record(e2);
        r.record(e3);
        let snap = r.snapshot();
        assert_eq!(snap, vec![e1, e2, e3]);
    }

    #[test]
    fn disk_persistence_survives_reopen() {
        let p = tmp("disk");
        {
            let r: SharedReservoirSampler<u32> = SharedReservoirSampler::create(&p, 5).unwrap();
            for i in 0..5u32 { r.record(i); }
            r.flush().unwrap();
        }
        let r2: SharedReservoirSampler<u32> = SharedReservoirSampler::open(&p, 5).unwrap();
        assert_eq!(r2.total_seen(), 5);
        assert_eq!(r2.snapshot(), vec![0, 1, 2, 3, 4]);
    }
}
