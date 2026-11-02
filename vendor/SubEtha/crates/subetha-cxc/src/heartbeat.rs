//! Per-process heartbeat slots stored in an MMF.
//!
//! Each participating process owns one [`HeartbeatSlot`] in the
//! shared table. On each scan tick, the process advances its slot's
//! `last_seen_epoch`. A watchdog (separate module) compares the
//! slot's epoch against a global `epoch` counter; if the process
//! hasn't advanced its heartbeat within the configured grace, its
//! work is presumed dead and reclaimed.
//!
//! Layout:
//! ```text
//! +-----------------------------+
//! | HeartbeatHeader (64B)       |
//! |   - magic, capacity, epoch  |
//! +-----------------------------+
//! | HeartbeatSlot[0]  (64B)     |
//! | HeartbeatSlot[1]  (64B)     |
//! | ...                         |
//! | HeartbeatSlot[N - 1]        |
//! +-----------------------------+
//! ```
//!
//! Each slot is one cache line so cross-process writes to different
//! slots never false-share.

use std::fs::File;
use std::path::Path;
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};

use memmap2::{MmapMut, MmapOptions};

use crate::process_identity;

pub const HEARTBEAT_MAGIC: u64 = 0x4150_4D46_4842_4132;

/// Unused slot ID (no pid).
pub const EMPTY_PID: u32 = 0;

/// The occupant's pid: the low half of a slot's identity word.
const PID_MASK: u64 = 0xFFFF_FFFF;
/// Where the slot's generation starts in the identity word.
const GENERATION_SHIFT: u32 = 32;
/// The slot's generation: bits 32 to 62 of the identity word.
const GENERATION_MASK: u64 = 0x7FFF_FFFF << GENERATION_SHIFT;
/// Set in the identity word from a register's claim of the slot until it
/// publishes; a slot carrying it reads as vacant.
const CLAIMING: u64 = 1 << 63;

/// The occupant's pid in an identity word.
fn identity_pid(identity: u64) -> u32 {
    (identity & PID_MASK) as u32
}

/// Whether an identity word names a published registration.
fn published(identity: u64) -> bool {
    identity & PID_MASK != u64::from(EMPTY_PID) && identity & CLAIMING == 0
}

/// The vacant identity word that follows `identity`: no pid, the next
/// generation.
fn vacated(identity: u64) -> u64 {
    ((identity & GENERATION_MASK) + (1 << GENERATION_SHIFT)) & GENERATION_MASK
}

/// Maximum number of in-flight work items a slot tracks. Each bit in
/// `in_flight_bitmap` represents one work unit; on failover those
/// bits are reclaimable.
pub const IN_FLIGHT_SLOTS: usize = 64;

#[repr(C, align(64))]
pub struct HeartbeatHeader {
    pub magic: u64,
    pub capacity: u64,
    /// Global epoch counter; the watchdog advances this each scan.
    pub epoch: AtomicU64,
    _reserved: [u8; 40],
}

#[repr(C, align(64))]
pub struct HeartbeatSlot {
    /// Who holds the slot: the occupant's pid in the low 32 bits
    /// (`EMPTY_PID` when vacant), the slot's generation in bits 32 to 62,
    /// advanced each time the slot is vacated, and bit 63 set while a
    /// register is writing the slot. While bit 63 is set the low 32 bits
    /// name the process writing the registration, which need not be the pid
    /// it registers. The slot is registered only while it carries a pid and
    /// bit 63 is clear.
    pub identity: AtomicU64,
    /// Last global epoch at which this process incremented its
    /// heartbeat. Watchdog reclaims when `global.epoch -
    /// last_seen_epoch > grace_epochs`.
    pub last_seen_epoch: AtomicU64,
    /// Bitmap of work units currently assigned to this process.
    /// Watchdog reclaims set bits on failover.
    pub in_flight_bitmap: AtomicU64,
    /// Process role: 0 = worker, 1 = coordinator.
    pub role: AtomicU32,
    _pad_role: [u8; 4],
    /// The holder word (pid and start stamp, see `process_identity`) of the
    /// process that wrote the slot's registration, written right after its
    /// claim and cleared before the slot is vacated, so a claim names
    /// either no stamp or its own writer's.
    pub claimant: AtomicU64,
    _pad: [u8; 24],
}

const _: () = assert!(std::mem::size_of::<HeartbeatSlot>() == 64);

/// Total file size for a heartbeat table with `capacity` slots.
pub const fn heartbeat_file_size(capacity: usize) -> usize {
    std::mem::size_of::<HeartbeatHeader>() + capacity * std::mem::size_of::<HeartbeatSlot>()
}

/// Cross-process heartbeat registry. Each process opens this and
/// reserves one slot via [`HeartbeatTable::register`].
pub struct HeartbeatTable {
    _file: File,
    mmap: MmapMut,
    capacity: usize,
    header_sidecar: subetha_core::HandshakeHeader,
    ring_sidecar: Box<subetha_core::ObservationRing>,
}

unsafe impl Send for HeartbeatTable {}
unsafe impl Sync for HeartbeatTable {}

impl subetha_sidecar::AdaptiveInstance for HeartbeatTable {
    fn header(&self) -> &subetha_core::HandshakeHeader { &self.header_sidecar }
    fn ring(&self) -> &subetha_core::ObservationRing { &self.ring_sidecar }
    fn make_policy(&self) -> Box<dyn subetha_sidecar::Policy> {
        Box::new(subetha_sidecar::NoMigrationPolicy)
    }
}

#[derive(Debug)]
pub enum HeartbeatError {
    LayoutMismatch(crate::layout::Mismatch),
    TableFull,
    /// The operating system refused to open, size or map the file; the
    /// error carries its code and message.
    IoError(std::io::Error),
}

impl std::fmt::Display for HeartbeatError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::LayoutMismatch(m) => write!(f, "a heartbeat table of another shape: {m}"),
            Self::TableFull => write!(f, "every heartbeat slot is taken"),
            Self::IoError(e) => write!(f, "io: {e}"),
        }
    }
}

impl std::error::Error for HeartbeatError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::IoError(e) => Some(e),
            Self::LayoutMismatch(_) | Self::TableFull => None,
        }
    }
}

impl From<std::io::Error> for HeartbeatError {
    fn from(e: std::io::Error) -> Self { Self::IoError(e) }
}

impl HeartbeatTable {
    /// Obtain the table at `path`, initializing it only when the path does
    /// not yet exist. Attaching leaves live registrations in place; a table
    /// built with a different capacity is a `LayoutMismatch`. Use
    /// [`reset`](Self::reset) to deliberately clear it.
    pub fn create(path: impl AsRef<Path>, capacity: usize) -> Result<Self, HeartbeatError> {
        assert!(capacity >= 1);
        let total = heartbeat_file_size(capacity);
        let (file, mmap) = crate::mmf_attach::create_or_attach(
            path.as_ref(),
            total,
            |ptr| unsafe { Self::init_region(ptr, capacity) },
            |ptr| unsafe { (*(ptr as *const HeartbeatHeader)).magic },
            HEARTBEAT_MAGIC,
        )
        .map_err(|e| crate::mmf_attach::attach_error(e, HeartbeatError::LayoutMismatch))?;
        let hdr = unsafe { &*(mmap.as_ptr() as *const HeartbeatHeader) };
        if hdr.capacity != capacity as u64 {
            return Err(HeartbeatError::LayoutMismatch(crate::layout::Mismatch::new(
                "capacity",
                hdr.capacity,
                capacity as u64,
            )));
        }
        Ok(Self {
            _file: file, mmap, capacity,
            header_sidecar: subetha_core::HandshakeHeader::new(),
            ring_sidecar: Box::new(subetha_core::ObservationRing::new()),
        })
    }

    /// Reinitialize the table at `path`, discarding every live registration.
    /// For a caller that knows it owns the path.
    pub fn reset(path: impl AsRef<Path>, capacity: usize) -> Result<Self, HeartbeatError> {
        assert!(capacity >= 1);
        let total = heartbeat_file_size(capacity);
        let (file, mmap) = crate::mmf_attach::reset(path.as_ref(), total, |ptr| unsafe {
            Self::init_region(ptr, capacity)
        })?;
        Ok(Self {
            _file: file, mmap, capacity,
            header_sidecar: subetha_core::HandshakeHeader::new(),
            ring_sidecar: Box::new(subetha_core::ObservationRing::new()),
        })
    }

    /// Lay out a fresh table: slots first, magic last, because attachers spin
    /// on the magic.
    ///
    /// # Safety
    /// `ptr` addresses at least `heartbeat_file_size(capacity)` writable
    /// zeroed bytes.
    unsafe fn init_region(ptr: *mut u8, capacity: usize) {
        let hdr_ptr = ptr as *mut HeartbeatHeader;
        unsafe {
            std::ptr::write(hdr_ptr, HeartbeatHeader {
                magic: 0,
                capacity: capacity as u64,
                epoch: AtomicU64::new(0),
                _reserved: [0; 40],
            });
            let slots_base = ptr.add(std::mem::size_of::<HeartbeatHeader>());
            for i in 0..capacity {
                let slot_ptr =
                    slots_base.add(i * std::mem::size_of::<HeartbeatSlot>()) as *mut HeartbeatSlot;
                std::ptr::write(slot_ptr, HeartbeatSlot {
                    identity: AtomicU64::new(u64::from(EMPTY_PID)),
                    last_seen_epoch: AtomicU64::new(0),
                    in_flight_bitmap: AtomicU64::new(0),
                    role: AtomicU32::new(0),
                    _pad_role: [0; 4],
                    claimant: AtomicU64::new(0),
                    _pad: [0; 24],
                });
            }
            std::ptr::write_volatile(std::ptr::addr_of_mut!((*hdr_ptr).magic), HEARTBEAT_MAGIC);
        }
    }

    pub fn open(path: impl AsRef<Path>, expected_capacity: usize) -> Result<Self, HeartbeatError> {
        let file = crate::region_file::open_existing(path.as_ref())?;
        let total = heartbeat_file_size(expected_capacity);
        // The length is checked before the map rather than after: a
        // caller declaring more slots than the file holds is a layout
        // disagreement, and mapping past the end of the file reports it
        // as an operating-system failure instead, which names the
        // symptom rather than the cause.
        let len = file.metadata()?.len();
        if len < total as u64 {
            return Err(HeartbeatError::LayoutMismatch(crate::layout::Mismatch::new(
                "length",
                len,
                total as u64,
            )));
        }
        let mmap = unsafe { MmapOptions::new().len(total).map_mut(&file)? };
        let header = unsafe { &*(mmap.as_ptr() as *const HeartbeatHeader) };
        crate::layout::Mismatch::first(&[
            ("magic", header.magic, HEARTBEAT_MAGIC),
            ("capacity", header.capacity, expected_capacity as u64),
        ])
        .map_err(HeartbeatError::LayoutMismatch)?;
        Ok(Self {
            _file: file, mmap, capacity: expected_capacity,
            header_sidecar: subetha_core::HandshakeHeader::new(),
            ring_sidecar: Box::new(subetha_core::ObservationRing::new()),
        })
    }

    pub fn capacity(&self) -> usize { self.capacity }

    pub fn header(&self) -> &HeartbeatHeader {
        unsafe { &*(self.mmap.as_ptr() as *const HeartbeatHeader) }
    }

    /// The slot at `idx`. Panics, in every build, when `idx` is not below
    /// the capacity: past it the offset leaves the mapping.
    fn slot(&self, idx: usize) -> &HeartbeatSlot {
        assert!(idx < self.capacity, "slot {idx} is past the {} the table holds", self.capacity);
        // SAFETY: the mapping holds the header and then `capacity` slots,
        // and `idx` is below the capacity.
        let base = unsafe {
            self.mmap.as_ptr().add(std::mem::size_of::<HeartbeatHeader>())
        };
        unsafe {
            &*(base.add(idx * std::mem::size_of::<HeartbeatSlot>()) as *const HeartbeatSlot)
        }
    }

    /// Register `pid`, usually this process's own. Returns the slot index:
    /// the first vacant slot, or, when every slot is held, the first one
    /// whose registration was left unpublished by a process now gone. Reads
    /// no other process's start time, whatever `pid` names.
    pub fn register(&self, pid: u32) -> Result<usize, HeartbeatError> {
        let claimed = (0..self.capacity)
            .find(|&i| self.try_claim(self.slot(i), pid))
            .or_else(|| (0..self.capacity).find(|&i| self.reclaim_and_claim(self.slot(i), pid)));
        self.ring_sidecar
            .push_op(crate::sidecar_ops::liveness::OP_REGISTER, u16::from(claimed.is_none()));
        claimed.ok_or(HeartbeatError::TableFull)
    }

    /// Claim `slot` for `pid` if it is vacant: mark it claiming in this
    /// process's name, record this process's holder word, write the fields,
    /// then publish `pid`. False when the slot is held, or was taken back
    /// from this claim before it published.
    fn try_claim(&self, slot: &HeartbeatSlot, pid: u32) -> bool {
        let vacant = slot.identity.load(Ordering::Acquire);
        if vacant & (PID_MASK | CLAIMING) != u64::from(EMPTY_PID) {
            return false;
        }
        let writer = process_identity::current();
        let claim = vacant | CLAIMING | u64::from(process_identity::holder_pid(writer));
        if slot.identity.compare_exchange(vacant, claim, Ordering::AcqRel, Ordering::Acquire).is_err() {
            return false;
        }
        slot.claimant.store(writer, Ordering::Release);
        slot.last_seen_epoch.store(self.header().epoch.load(Ordering::Acquire), Ordering::Release);
        slot.in_flight_bitmap.store(0, Ordering::Release);
        slot.role.store(0, Ordering::Release);
        slot.identity
            .compare_exchange(claim, vacant | u64::from(pid), Ordering::AcqRel, Ordering::Acquire)
            .is_ok()
    }

    /// Vacate `slot` when it was left claiming by a process that is gone,
    /// then claim it for `pid`. A claim names the process writing it, not
    /// the pid it registers, and that writer is judged by its pid and the
    /// start its claim recorded, so a pid the system has since given to
    /// another process reads as gone; a claim that recorded no start yet is
    /// judged by its pid alone.
    fn reclaim_and_claim(&self, slot: &HeartbeatSlot, pid: u32) -> bool {
        let claim = slot.identity.load(Ordering::Acquire);
        if claim & CLAIMING == 0 {
            return false;
        }
        let claimed_by = identity_pid(claim);
        let recorded = slot.claimant.load(Ordering::Acquire);
        let stamp = if process_identity::holder_pid(recorded) == claimed_by {
            process_identity::holder_stamp(recorded)
        } else {
            0
        };
        if !process_identity::gone(claimed_by, stamp) {
            return false;
        }
        // Cleared before the vacate, and only while it still holds what was
        // read, so the next claim names no start but its own claimant's.
        if recorded != 0
            && slot.claimant.compare_exchange(recorded, 0, Ordering::AcqRel, Ordering::Acquire).is_err()
        {
            return false;
        }
        slot.identity
            .compare_exchange(claim, vacated(claim), Ordering::AcqRel, Ordering::Acquire)
            .is_ok()
            && self.try_claim(slot, pid)
    }

    /// Release the slot at `idx`. Call before process exit. A slot that is
    /// vacant, or that a register is still writing, is left as it is.
    ///
    /// # Panics
    /// If `idx` is not below the capacity.
    pub fn unregister(&self, idx: usize) {
        let slot = self.slot(idx);
        let held = slot.identity.load(Ordering::Acquire);
        if !published(held) {
            return;
        }
        slot.in_flight_bitmap.store(0, Ordering::Release);
        slot.claimant.store(0, Ordering::Release);
        if let Err(now) =
            slot.identity.compare_exchange(held, vacated(held), Ordering::AcqRel, Ordering::Acquire)
        {
            debug_assert_ne!(
                now & GENERATION_MASK,
                held & GENERATION_MASK,
                "only a vacate moves a published slot's identity, and it advances the generation"
            );
        }
    }

    /// Heartbeat: advance this slot's `last_seen_epoch` to match the
    /// global epoch. Call once per scan tick.
    ///
    /// # Panics
    /// If `idx` is not below the capacity.
    pub fn beat(&self, idx: usize) {
        let global = self.header().epoch.load(Ordering::Acquire);
        let slot = self.slot(idx);
        slot.last_seen_epoch.store(global, Ordering::Release);
        self.ring_sidecar
            .push_op(crate::sidecar_ops::liveness::OP_BEAT, 0);
    }

    /// Advance the global epoch. Watchdog calls this once per scan
    /// interval. Returns the new epoch value.
    pub fn tick_global_epoch(&self) -> u64 {
        let v = self.header().epoch.fetch_add(1, Ordering::AcqRel) + 1;
        self.ring_sidecar
            .push_op(crate::sidecar_ops::liveness::OP_TICK_EPOCH, 0);
        v
    }

    pub fn global_epoch(&self) -> u64 {
        self.header().epoch.load(Ordering::Acquire)
    }

    /// Mark a work unit as in-flight for `slot_idx`.
    ///
    /// # Panics
    /// If `slot_idx` is not below the capacity or `bit` is not below
    /// `IN_FLIGHT_SLOTS`, in every build.
    pub fn mark_in_flight(&self, slot_idx: usize, bit: u8) {
        assert!((bit as usize) < IN_FLIGHT_SLOTS, "in-flight bit {bit} is past the {IN_FLIGHT_SLOTS} a slot holds");
        let slot = self.slot(slot_idx);
        slot.in_flight_bitmap.fetch_or(1u64 << bit, Ordering::AcqRel);
    }

    /// Clear a work unit's in-flight mark for `slot_idx`.
    ///
    /// # Panics
    /// As [`mark_in_flight`](Self::mark_in_flight).
    pub fn clear_in_flight(&self, slot_idx: usize, bit: u8) {
        assert!((bit as usize) < IN_FLIGHT_SLOTS, "in-flight bit {bit} is past the {IN_FLIGHT_SLOTS} a slot holds");
        let slot = self.slot(slot_idx);
        slot.in_flight_bitmap.fetch_and(!(1u64 << bit), Ordering::AcqRel);
    }

    /// Snapshot a slot. Returns `None` if the slot is vacant or a register
    /// has not yet published it. Every field comes from one registration:
    /// the read retries only when the slot was vacated while it read.
    ///
    /// # Panics
    /// If `idx` is not below the capacity.
    pub fn snapshot(&self, idx: usize) -> Option<HeartbeatSnapshot> {
        let slot = self.slot(idx);
        loop {
            let identity = slot.identity.load(Ordering::Acquire);
            if !published(identity) {
                return None;
            }
            let last = slot.last_seen_epoch.load(Ordering::Acquire);
            let inflight = slot.in_flight_bitmap.load(Ordering::Acquire);
            let role = slot.role.load(Ordering::Acquire);
            if slot.identity.load(Ordering::Acquire) == identity {
                return Some(HeartbeatSnapshot {
                    pid: identity_pid(identity), last_seen_epoch: last,
                    in_flight_bitmap: inflight, role,
                });
            }
        }
    }
}

/// Snapshot of one slot's state. Cheap to copy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HeartbeatSnapshot {
    pub pid: u32,
    pub last_seen_epoch: u64,
    pub in_flight_bitmap: u64,
    pub role: u32,
}

/// Crate-internal accessor for the watchdog module. Not pub-exported
/// from the crate (only re-exported intra-crate).
#[doc(hidden)]
pub fn __slot_for_watchdog(table: &HeartbeatTable, idx: usize) -> &HeartbeatSlot {
    table.slot(idx)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A file for one test, removed when the test ends; declared before the
    /// primitive that maps it so it drops after it.
    fn tmp_path(name: &str) -> crate::test_paths::TmpFile {
        crate::test_paths::TmpFile::new(format!("subetha-hb-{name}-{}.bin", std::process::id()))
    }

    /// A slot past the capacity panics in every build rather than reading
    /// and writing past the mapping.
    #[test]
    #[should_panic(expected = "slot 4 is past the 4 the table holds")]
    fn a_slot_past_the_capacity_panics_rather_than_leaving_the_mapping() {
        let p = tmp_path("past-capacity");
        let t = HeartbeatTable::create(&p, 4).unwrap();
        t.beat(4);
    }

    /// An in-flight bit past the 64 a slot holds panics in every build
    /// rather than wrapping onto another bit.
    #[test]
    #[should_panic(expected = "in-flight bit 64 is past the 64 a slot holds")]
    fn an_in_flight_bit_past_the_slot_panics_rather_than_wrapping() {
        let p = tmp_path("bit-past");
        let t = HeartbeatTable::create(&p, 4).unwrap();
        let s = t.register(1).unwrap();
        t.mark_in_flight(s, 64);
    }

    #[test]
    fn register_returns_slot_indices() {
        let p = tmp_path("register");
        let t = HeartbeatTable::create(&p, 4).unwrap();
        let s0 = t.register(1001).unwrap();
        let s1 = t.register(1002).unwrap();
        assert_ne!(s0, s1);
    }

    #[test]
    fn table_full_returns_error() {
        let p = tmp_path("table-full");
        let t = HeartbeatTable::create(&p, 2).unwrap();
        let _val = t.register(1).unwrap();
        let _val = t.register(2).unwrap();
        assert!(matches!(t.register(3).unwrap_err(), HeartbeatError::TableFull));
    }

    #[test]
    fn beat_advances_last_seen_epoch() {
        let p = tmp_path("beat");
        let t = HeartbeatTable::create(&p, 1).unwrap();
        let s = t.register(99).unwrap();
        for _ in 0..5 { t.tick_global_epoch(); }
        let snap_before = t.snapshot(s).unwrap();
        let global_after_tick = t.global_epoch();
        t.beat(s);
        let snap_after = t.snapshot(s).unwrap();
        assert!(snap_after.last_seen_epoch > snap_before.last_seen_epoch);
        assert_eq!(snap_after.last_seen_epoch, global_after_tick);
    }

    #[test]
    fn unregister_frees_slot_for_reuse() {
        let p = tmp_path("unreg");
        let t = HeartbeatTable::create(&p, 2).unwrap();
        let s0 = t.register(11).unwrap();
        let _s1 = t.register(22).unwrap();
        t.unregister(s0);
        // Now there should be a free slot.
        let new = t.register(33).unwrap();
        assert_eq!(new, s0);
    }

    #[test]
    fn in_flight_bitmap_mark_and_clear() {
        let p = tmp_path("inflight");
        let t = HeartbeatTable::create(&p, 1).unwrap();
        let s = t.register(7).unwrap();
        t.mark_in_flight(s, 3);
        t.mark_in_flight(s, 5);
        let snap = t.snapshot(s).unwrap();
        assert_eq!(snap.in_flight_bitmap, (1u64 << 3) | (1u64 << 5));
        t.clear_in_flight(s, 3);
        let snap = t.snapshot(s).unwrap();
        assert_eq!(snap.in_flight_bitmap, 1u64 << 5);
    }

    #[test]
    fn a_registered_slot_snapshots_its_own_fields() {
        let p = tmp_path("snap");
        let t = HeartbeatTable::create(&p, 1).unwrap();
        let s = t.register(42).unwrap();
        t.tick_global_epoch();
        t.beat(s);
        assert_eq!(
            t.snapshot(s),
            Some(HeartbeatSnapshot { pid: 42, last_seen_epoch: 1, in_flight_bitmap: 0, role: 0 })
        );
    }

    /// Leave slot `idx` claimed in the name of the writer `pid` and
    /// unpublished, as a register leaves it when its process dies between
    /// the claim and the publish.
    fn leave_claiming(t: &HeartbeatTable, idx: usize, pid: u32) {
        t.slot(idx).identity.store(CLAIMING | u64::from(pid), Ordering::Release);
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
        assert!(!crate::peer_directory::process_alive(pid), "the killed child {pid} is gone");
        pid
    }

    /// A live child process, killed and reaped when dropped, and the start
    /// stamp the system reports for it.
    struct LiveChild(std::process::Child, u32);

    impl LiveChild {
        fn start() -> Self {
            let child = std::process::Command::new(std::env::current_exe().unwrap())
                .args(["--exact", "exit_watch::tests::sleeper_never_runs_on_its_own", "--ignored", "--nocapture"])
                .env("SUBETHA_EXIT_WATCH_SLEEP", "1")
                .spawn()
                .unwrap();
            let stamp = match process_identity::probe(child.id()) {
                process_identity::Probe::Started(stamp) => stamp,
                other => panic!("a child of this process reads as started, not {other:?}"),
            };
            Self(child, stamp)
        }
    }

    impl Drop for LiveChild {
        fn drop(&mut self) {
            self.0.kill().unwrap();
            self.0.wait().unwrap();
        }
    }

    #[test]
    fn a_registration_records_its_claimant_and_an_unregister_clears_it() {
        let p = tmp_path("claimant");
        let t = HeartbeatTable::create(&p, 2).unwrap();
        let s = t.register(std::process::id()).unwrap();
        assert_eq!(t.slot(s).claimant.load(Ordering::Acquire), process_identity::current());
        t.unregister(s);
        assert_eq!(t.slot(s).claimant.load(Ordering::Acquire), 0);
    }

    /// A registration for another pid records this process, its writer, as
    /// the claimant, and publishes the pid it was asked to register.
    #[test]
    fn a_registration_for_another_pid_records_this_process_as_its_writer() {
        let p = tmp_path("other-pid");
        let t = HeartbeatTable::create(&p, 2).unwrap();
        let other = std::process::id().wrapping_add(1);
        let s = t.register(other).unwrap();
        assert_eq!(t.snapshot(s).map(|snap| snap.pid), Some(other));
        assert_eq!(t.slot(s).claimant.load(Ordering::Acquire), process_identity::current());
    }

    /// A claim left by a writer that died is taken back by a full table,
    /// though the pid that writer was registering names a live process:
    /// the claim names its writer, not that pid.
    #[test]
    fn a_full_table_takes_back_a_dead_writers_claim_whatever_pid_it_was_registering() {
        let p = tmp_path("dead-writer");
        let t = HeartbeatTable::create(&p, 1).unwrap();
        let registered = LiveChild::start();
        let writer = exited_pid();
        leave_claiming(&t, 0, writer);
        t.slot(0).claimant.store(process_identity::holder(writer, 0), Ordering::Release);
        assert_eq!(t.register(registered.0.id()).unwrap(), 0);
        assert_eq!(t.snapshot(0).map(|snap| snap.pid), Some(registered.0.id()));
        assert_eq!(t.slot(0).claimant.load(Ordering::Acquire), process_identity::current());
    }

    /// A claim whose writer's pid now names a live process that started at
    /// another time than the claim recorded, as a pid the system handed on
    /// after that writer died, is taken back by a full table.
    #[test]
    fn a_full_table_takes_back_a_claim_whose_pid_now_names_another_process() {
        let p = tmp_path("reused-pid");
        let t = HeartbeatTable::create(&p, 1).unwrap();
        let child = LiveChild::start();
        let pid = child.0.id();
        leave_claiming(&t, 0, pid);
        t.slot(0).claimant.store(process_identity::holder(pid, child.1.wrapping_add(1).max(1)), Ordering::Release);
        assert_eq!(t.register(std::process::id()).unwrap(), 0);
        assert_eq!(t.snapshot(0).map(|snap| snap.pid), Some(std::process::id()));
    }

    /// A claim whose pid and recorded start name a live process stays
    /// held: a full table refuses.
    #[test]
    fn a_full_table_leaves_a_claim_alone_whose_pid_and_start_are_live() {
        let p = tmp_path("live-start");
        let t = HeartbeatTable::create(&p, 1).unwrap();
        let child = LiveChild::start();
        let pid = child.0.id();
        leave_claiming(&t, 0, pid);
        t.slot(0).claimant.store(process_identity::holder(pid, child.1), Ordering::Release);
        assert!(matches!(t.register(std::process::id()).unwrap_err(), HeartbeatError::TableFull));
        assert_eq!(t.slot(0).identity.load(Ordering::Acquire), CLAIMING | u64::from(pid));
    }

    /// A table file carrying the magic of the layout before the identity
    /// word is refused: `open` and `create` both name the magic.
    #[test]
    fn a_table_file_of_the_earlier_layout_is_refused() {
        let p = tmp_path("earlier-layout");
        let mut t = HeartbeatTable::create(&p, 2).unwrap();
        // SAFETY: the mapping starts with the header, whose first field is
        // the magic, and nothing else borrows the mapping.
        unsafe { (t.mmap.as_mut_ptr() as *mut u64).write_volatile(0x4150_4D46_4842_4154) };
        assert!(matches!(
            HeartbeatTable::open(&p, 2),
            Err(HeartbeatError::LayoutMismatch(m)) if m.field == "magic"
        ));
        assert!(matches!(
            HeartbeatTable::create(&p, 2),
            Err(HeartbeatError::LayoutMismatch(m)) if m.field == "magic"
        ));
    }

    /// A slot whose registrant died mid-register reads as vacant at once,
    /// and a register passes it by while another slot is vacant.
    #[test]
    fn a_registrant_dead_mid_register_leaves_its_slot_vacant_to_snapshots() {
        let p = tmp_path("dead-claim-snapshot");
        let t = HeartbeatTable::create(&p, 2).unwrap();
        leave_claiming(&t, 0, exited_pid());
        assert_eq!(t.snapshot(0), None);
        assert_eq!(t.register(7).unwrap(), 1);
    }

    /// A full table takes back the slot of a registrant whose process
    /// exited before publishing; the slot then reads as the new
    /// registration, one generation on.
    #[test]
    fn a_full_table_takes_back_a_slot_whose_registrant_died_claiming() {
        let p = tmp_path("dead-claim-reclaim");
        let t = HeartbeatTable::create(&p, 2).unwrap();
        assert_eq!(t.register(11).unwrap(), 0);
        leave_claiming(&t, 1, exited_pid());
        assert_eq!(t.register(22).unwrap(), 1);
        assert_eq!(t.snapshot(1).map(|snap| snap.pid), Some(22));
        assert_eq!(t.slot(1).identity.load(Ordering::Acquire) & GENERATION_MASK, 1 << GENERATION_SHIFT);
    }

    /// A full table leaves alone a claim whose pid names a live process.
    #[test]
    fn a_full_table_leaves_a_live_registrants_claim_alone() {
        let p = tmp_path("live-claim");
        let t = HeartbeatTable::create(&p, 1).unwrap();
        leave_claiming(&t, 0, std::process::id());
        assert!(matches!(t.register(22).unwrap_err(), HeartbeatError::TableFull));
        assert_eq!(t.slot(0).identity.load(Ordering::Acquire), CLAIMING | u64::from(std::process::id()));
    }

    /// An unregister leaves a vacant slot and a claim in progress as they
    /// are, and a slot it vacates reads vacant one generation on.
    #[test]
    fn unregister_vacates_only_a_published_slot() {
        let p = tmp_path("unreg-published");
        let t = HeartbeatTable::create(&p, 2).unwrap();
        t.unregister(0);
        assert_eq!(t.slot(0).identity.load(Ordering::Acquire), 0);
        leave_claiming(&t, 1, 31);
        t.unregister(1);
        assert_eq!(t.slot(1).identity.load(Ordering::Acquire), CLAIMING | 31);
        let s = t.register(5).unwrap();
        t.unregister(s);
        assert_eq!(t.snapshot(s), None);
        assert_eq!(t.slot(s).identity.load(Ordering::Acquire), 1 << GENERATION_SHIFT);
    }

    /// Registrations coming and going on one slot never let a snapshot
    /// pair one registrant's pid with another's in-flight bits.
    #[test]
    fn a_snapshot_never_mixes_two_registrations() {
        let p = tmp_path("no-mix");
        let t = HeartbeatTable::create(&p, 1).unwrap();
        let done = std::sync::atomic::AtomicBool::new(false);
        std::thread::scope(|s| {
            s.spawn(|| {
                for round in 0..200_000u32 {
                    let pid = 1 + round % 63;
                    let idx = t.register(pid).unwrap();
                    t.mark_in_flight(idx, pid as u8);
                    t.unregister(idx);
                }
                done.store(true, Ordering::Release);
            });
            while !done.load(Ordering::Acquire) {
                if let Some(snap) = t.snapshot(0) {
                    assert_eq!(
                        snap.in_flight_bitmap & !(1u64 << snap.pid), 0,
                        "pid {} read with bits {:#x}", snap.pid, snap.in_flight_bitmap
                    );
                }
            }
        });
    }
}
