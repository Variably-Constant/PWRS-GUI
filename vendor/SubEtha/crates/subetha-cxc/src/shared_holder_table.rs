//! `SharedHolderTable` - a [`HolderTable`] with a file of its own.
//!
//! The bare [`HolderTable`] is a view over slots the caller has already
//! mapped, so it sits inside whatever header layout that caller needs: an
//! epoch counter beside its pins, a payload beside its holders. That is
//! what makes it reusable, and it is also why it cannot be handed to a
//! caller who has nothing to embed it in.
//!
//! This owns a file: a small header carrying the magic and the capacity,
//! then the slots. Everything else is the view's, delegated straight
//! through, so a table reached this way and one embedded in another
//! structure behave identically.
//!
//! What the slots mean is the caller's. The payload is one `u64` per
//! slot, at any encoding that agrees across the processes sharing the
//! file; [`HOLDER_FREE`](crate::holder_table::HOLDER_FREE) and
//! [`HOLDER_RESERVED`](crate::holder_table::HOLDER_RESERVED) are the two values a
//! caller may not use, since the table itself reads them.

use std::fs::File;
use std::mem::size_of;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};

use memmap2::{MmapMut, MmapOptions};

use crate::holder_table::{holder_table_size, HolderSlot, HolderTable};

/// "HOLDERS2": a header followed by the slots.
pub const SHARED_HOLDER_MAGIC: u64 = 0x484F_4C44_4552_5332;

/// The header ahead of the slots. One cache line, so the slots behind it
/// keep the alignment [`HolderTable`] needs.
#[repr(C, align(64))]
struct SharedHolderHeader {
    magic: AtomicU64,
    capacity: AtomicU64,
    _pad: [u8; 48],
}

const _: () = {
    assert!(size_of::<SharedHolderHeader>() == 64);
};

/// Bytes a table of `capacity` slots needs.
pub const fn shared_holder_file_size(capacity: usize) -> usize {
    size_of::<SharedHolderHeader>() + holder_table_size(capacity)
}

#[derive(Debug)]
pub enum SharedHolderError {
    /// The file exists but its magic or capacity is not the one asked
    /// for.
    LayoutMismatch(crate::layout::Mismatch),
    /// A capacity of zero, which has no slot to claim.
    EmptyCapacity,
    /// The operating system refused to open, size or map the file; the
    /// error carries its code and message.
    IoError(std::io::Error),
}

impl From<std::io::Error> for SharedHolderError {
    fn from(e: std::io::Error) -> Self {
        Self::IoError(e)
    }
}

impl std::fmt::Display for SharedHolderError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SharedHolderError::LayoutMismatch(m) => {
                write!(f, "a holder table of another shape: {m}")
            }
            SharedHolderError::EmptyCapacity => write!(f, "a capacity of zero has no slot to claim"),
            SharedHolderError::IoError(e) => write!(f, "io: {e}"),
        }
    }
}

impl std::error::Error for SharedHolderError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            SharedHolderError::IoError(e) => Some(e),
            SharedHolderError::LayoutMismatch(_) | SharedHolderError::EmptyCapacity => None,
        }
    }
}

/// A holder table backed by a file of its own.
pub struct SharedHolderTable {
    _file: File,
    _mmap: MmapMut,
    table: HolderTable,
    capacity: usize,
}

impl std::fmt::Debug for SharedHolderTable {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SharedHolderTable")
            .field("capacity", &self.capacity)
            .field("live", &self.live())
            .finish()
    }
}

fn check_capacity(capacity: usize) -> Result<(), SharedHolderError> {
    if capacity == 0 {
        return Err(SharedHolderError::EmptyCapacity);
    }
    Ok(())
}

/// Build the view over the slots behind the header.
///
/// # Safety
/// `mmap` covers at least `shared_holder_file_size(capacity)` bytes and
/// outlives the returned table.
unsafe fn view(mmap: &MmapMut, capacity: usize) -> HolderTable {
    unsafe { HolderTable::from_ptr(mmap.as_ptr().add(size_of::<SharedHolderHeader>()), capacity) }
}

/// Lay the table out, or finish a layout another creator began, so it is
/// published either way. Every creator makes the same two swaps from
/// zero, the capacity and then the magic, and none waits on another: the
/// slots of a fresh file already read free, and the first swap of the
/// magic publishes the table. A capacity already there that differs is
/// another table's, and a magic other than this layout's another
/// layout's; both are refused as a mismatch, the magic before anything is
/// written.
///
/// # Safety
/// `ptr` addresses at least `shared_holder_file_size(capacity)` bytes,
/// zeroed or laid out by this protocol.
unsafe fn lay_out(ptr: *mut u8, capacity: usize) -> Result<(), SharedHolderError> {
    let hdr = unsafe { &*(ptr as *const SharedHolderHeader) };
    let mismatch = |field, found, wanted| {
        Err(SharedHolderError::LayoutMismatch(crate::layout::Mismatch::new(field, found, wanted)))
    };
    match hdr.magic.load(Ordering::Acquire) {
        SHARED_HOLDER_MAGIC => return Ok(()),
        0 => {}
        found => return mismatch("magic", found, SHARED_HOLDER_MAGIC),
    }
    let wanted = capacity as u64;
    if let Err(held) = hdr.capacity.compare_exchange(0, wanted, Ordering::AcqRel, Ordering::Acquire)
        && held != wanted
    {
        return mismatch("capacity", held, wanted);
    }
    #[cfg(test)]
    crate::test_races::pause_point();
    match hdr.magic.compare_exchange(0, SHARED_HOLDER_MAGIC, Ordering::AcqRel, Ordering::Acquire) {
        Ok(_) | Err(SHARED_HOLDER_MAGIC) => Ok(()),
        Err(found) => mismatch("magic", found, SHARED_HOLDER_MAGIC),
    }
}

impl SharedHolderTable {
    /// Obtain the table at `path` with `capacity` slots: an empty one is
    /// laid out when the file does not exist or is empty, an existing one
    /// is attached with its claims in place, so a late joiner does not
    /// release anyone.
    ///
    /// Creators that reach an empty file together lay it out once: each
    /// makes the same writes from zero and the first to publish the magic
    /// publishes the table, so no claim made on the table is cleared by a
    /// second layout and no creator waits on another. A layout a creator
    /// left unfinished, such as one a process died during, is finished by
    /// the next creator. A file holding a table of another capacity is
    /// refused as `LayoutMismatch` and left as it is.
    pub fn create(path: impl AsRef<Path>, capacity: usize) -> Result<Self, SharedHolderError> {
        check_capacity(capacity)?;
        let total = shared_holder_file_size(capacity);
        let file = crate::region_file::create_or_open(path.as_ref())?;
        // Extending zero-fills, so creators extending an empty file to the
        // same length leave the same bytes whatever their order. A file
        // shorter than the table but not empty holds something else.
        let len = file.metadata()?.len() as usize;
        if len == 0 {
            file.set_len(total as u64)?;
        } else if len < total {
            return Err(SharedHolderError::LayoutMismatch(crate::layout::Mismatch::new(
                "length",
                len as u64,
                total as u64,
            )));
        }
        let mut mmap = unsafe { MmapOptions::new().len(total).map_mut(&file)? };
        // SAFETY: the mapping holds `total` bytes, zeroed by the extension
        // or laid out by this same protocol.
        unsafe { lay_out(mmap.as_mut_ptr(), capacity)? };
        Self::finish(file, mmap, capacity)
    }

    /// Truncate the file at `path` and lay out an empty table, releasing
    /// every slot a live holder owns. For a caller that knows it owns the
    /// path.
    pub fn reset(path: impl AsRef<Path>, capacity: usize) -> Result<Self, SharedHolderError> {
        check_capacity(capacity)?;
        let total = shared_holder_file_size(capacity);
        let file = crate::region_file::create_truncated(path.as_ref())?;
        file.set_len(total as u64)?;
        let mut mmap = unsafe { MmapOptions::new().len(total).map_mut(&file)? };
        let hdr = mmap.as_mut_ptr() as *mut SharedHolderHeader;
        unsafe {
            std::ptr::write(
                hdr,
                SharedHolderHeader {
                    magic: AtomicU64::new(0),
                    capacity: AtomicU64::new(capacity as u64),
                    _pad: [0; 48],
                },
            );
            std::ptr::write_bytes(
                mmap.as_mut_ptr().add(size_of::<SharedHolderHeader>()) as *mut HolderSlot,
                0,
                capacity,
            );
            (*hdr).magic.store(SHARED_HOLDER_MAGIC, Ordering::Release);
        }
        Self::finish(file, mmap, capacity)
    }

    /// Attach to an existing table; the file must be there and must
    /// declare `capacity`.
    pub fn open(path: impl AsRef<Path>, capacity: usize) -> Result<Self, SharedHolderError> {
        check_capacity(capacity)?;
        let total = shared_holder_file_size(capacity);
        let file = crate::region_file::open_existing(path.as_ref())?;
        // The length is checked before the map: a caller declaring more
        // slots than the file holds is a layout disagreement, and
        // mapping past the end of the file names the symptom instead.
        let len = file.metadata()?.len();
        if len < total as u64 {
            return Err(SharedHolderError::LayoutMismatch(crate::layout::Mismatch::new(
                "length",
                len,
                total as u64,
            )));
        }
        let mmap = unsafe { MmapOptions::new().len(total).map_mut(&file)? };
        Self::finish(file, mmap, capacity)
    }

    fn finish(file: File, mmap: MmapMut, capacity: usize) -> Result<Self, SharedHolderError> {
        let hdr = mmap.as_ptr() as *const SharedHolderHeader;
        // The capacity is read only behind the magic, which a layout
        // publishes after writing it.
        let magic = unsafe { (*hdr).magic.load(Ordering::Acquire) };
        if magic != SHARED_HOLDER_MAGIC {
            return Err(SharedHolderError::LayoutMismatch(crate::layout::Mismatch::new(
                "magic",
                magic,
                SHARED_HOLDER_MAGIC,
            )));
        }
        let held = unsafe { (*hdr).capacity.load(Ordering::Acquire) };
        if held != capacity as u64 {
            return Err(SharedHolderError::LayoutMismatch(crate::layout::Mismatch::new(
                "capacity",
                held,
                capacity as u64,
            )));
        }
        let table = unsafe { view(&mmap, capacity) };
        Ok(Self { _file: file, _mmap: mmap, table, capacity })
    }

    /// The view the rest of this crate takes, for a caller that already
    /// speaks [`HolderTable`].
    pub fn table(&self) -> &HolderTable {
        &self.table
    }

    /// Slots the table holds.
    pub fn capacity(&self) -> usize {
        self.capacity
    }

    /// Claim a slot with no payload decided yet, for a caller whose
    /// payload depends on state it must read after the slot is visible.
    /// [`publish`](Self::publish) fills it in.
    pub fn reserve(&self) -> Option<usize> {
        self.table.reserve()
    }

    /// Fill in the payload of a slot [`reserve`](Self::reserve) took.
    ///
    /// # Panics
    /// If `payload` is a reserved state or `slot` is not below the
    /// capacity.
    pub fn publish(&self, slot: usize, payload: u64) {
        self.table.publish(slot, payload);
    }

    /// Claim a slot and set its payload in one step, for a caller whose
    /// payload does not depend on anything read between the two.
    ///
    /// # Panics
    /// If `payload` is a reserved state, before any slot is reserved.
    pub fn claim(&self, payload: u64) -> Option<usize> {
        self.table.claim(payload)
    }

    /// Claim the slot at `i` in particular, for a caller that has already
    /// decided which one it wants. Answers whether it was free.
    ///
    /// # Panics
    /// If `payload` is a reserved state or `i` is not below the capacity.
    pub fn try_claim_slot(&self, i: usize, payload: u64) -> bool {
        self.table.try_claim_slot(i, payload)
    }

    /// Give a slot back.
    ///
    /// # Panics
    /// If `slot` is not below the capacity.
    pub fn release(&self, slot: usize) {
        self.table.release(slot);
    }

    /// Give a slot back only while it still holds the process `pid` and
    /// the payload `payload` the caller observed, so a claimant that has
    /// found a claim stale (its pid now names another process, told by a
    /// creation time in the payload) frees it without freeing a holder
    /// that claimed the slot since. Answers whether it freed the slot.
    ///
    /// # Panics
    /// If `slot` is not below the capacity.
    pub fn release_if(&self, slot: usize, pid: u32, payload: u64) -> bool {
        self.table.release_if(slot, pid, payload)
    }

    /// The payload of a slot, or `None` for one that is free or still
    /// reserved.
    ///
    /// # Panics
    /// If `slot` is not below the capacity.
    pub fn payload(&self, slot: usize) -> Option<u64> {
        self.table.payload(slot)
    }

    /// Slots held right now, across every process.
    pub fn live(&self) -> usize {
        self.table.live()
    }

    /// Release every slot whose owning process is gone, and report how
    /// many went.
    pub fn reap_dead(&self) -> usize {
        self.table.reap_dead()
    }

    /// The process holding the slot at `i`, or zero for one that is free.
    ///
    /// # Panics
    /// If `i` is not below the capacity.
    pub fn owner_pid(&self, i: usize) -> u32 {
        crate::process_identity::holder_pid(self.table.slot(i).owner.load(std::sync::atomic::Ordering::Acquire))
    }

    /// The raw state word of the slot at `i`: [`HOLDER_FREE`],
    /// [`HOLDER_RESERVED`], or the caller's payload.
    ///
    /// # Panics
    /// If `i` is not below the capacity.
    ///
    /// [`HOLDER_FREE`]: crate::holder_table::HOLDER_FREE
    /// [`HOLDER_RESERVED`]: crate::holder_table::HOLDER_RESERVED
    pub fn state(&self, i: usize) -> u64 {
        self.table.slot(i).state.load(std::sync::atomic::Ordering::Acquire)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::holder_table::{HOLDER_FREE, HOLDER_RESERVED};
    use std::time::Instant;

    fn tmp(name: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!("subetha-shared-holders-{name}-{}.bin", std::process::id()))
    }

    /// Remove a scratch file if it is there. An absent file is the state
    /// before the first run and after the last; any other refusal means
    /// the scratch directory is not usable and the test result after it
    /// means nothing.
    fn clear(path: &Path) {
        match crate::region_file::remove(path) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => panic!("the scratch table at {} could not be removed: {e}", path.display()),
        }
    }

    #[test]
    fn slots_are_claimed_released_and_handed_out_again() {
        let path = tmp("claims");
        clear(&path);
        let table = SharedHolderTable::create(&path, 4).unwrap();
        assert_eq!(table.capacity(), 4);
        assert_eq!(table.live(), 0);
        assert_eq!(table.state(0), HOLDER_FREE);

        let first = table.claim(7).expect("a free slot");
        let second = table.claim(9).expect("another");
        assert_eq!(table.live(), 2);
        assert_eq!(table.payload(first), Some(7));
        assert_eq!(table.payload(second), Some(9));
        assert_eq!(table.owner_pid(first), std::process::id());
        assert_eq!(table.payload(3), None, "a free slot carries no payload");

        table.release(first);
        assert_eq!(table.live(), 1);
        assert_eq!(table.payload(first), None);
        assert_eq!(table.claim(11), Some(first), "the freed slot comes back");

        // A reservation is held but reports no payload, which is what
        // stops a reader acting on a claim that is not finished.
        let held = table.reserve().expect("a slot to reserve");
        assert_eq!(table.state(held), HOLDER_RESERVED);
        assert_eq!(table.payload(held), None, "a reservation is not a holder yet");
        table.publish(held, 13);
        assert_eq!(table.payload(held), Some(13));

        // Slots zero, one and two are taken; the fourth fills it, and
        // only then does a claim have nowhere to go.
        table.claim(15).expect("the last free slot");
        assert!(table.claim(19).is_none(), "the table is full");
        assert!(!table.try_claim_slot(held, 17), "a held slot refuses a direct claim");
        table.release(held);
        assert!(table.try_claim_slot(held, 17), "and takes one once it is free");
        assert_eq!(table.payload(held), Some(17));
        drop(table);
        clear(&path);
    }

    #[test]
    fn a_second_handle_shares_the_claims_and_another_capacity_is_refused() {
        let path = tmp("share");
        clear(&path);
        let first = SharedHolderTable::create(&path, 4).unwrap();
        let slot = first.claim(42).expect("a free slot");

        let second = SharedHolderTable::open(&path, 4).unwrap();
        assert_eq!(second.payload(slot), Some(42), "the claim is in the file");
        assert_eq!(second.live(), 1);
        second.release(slot);
        assert_eq!(first.live(), 0, "and so is the release");

        match SharedHolderTable::open(&path, 8) {
            Ok(_) => panic!("an eight-slot handle on a four-slot table was accepted"),
            Err(e) => assert!(
                matches!(e, SharedHolderError::LayoutMismatch(m) if m == crate::layout::Mismatch::new(
                    "length",
                    shared_holder_file_size(4) as u64,
                    shared_holder_file_size(8) as u64,
                )),
                "{e:?}",
            ),
        }
        match SharedHolderTable::create(&path, 0) {
            Ok(_) => panic!("a capacity of zero was accepted"),
            Err(e) => assert!(matches!(e, SharedHolderError::EmptyCapacity), "{e:?}"),
        }

        // A create over an existing table attaches rather than clearing
        // it; a reset is what strips the claims.
        let held = first.claim(99).expect("a free slot");
        drop(second);
        let attached = SharedHolderTable::create(&path, 4).unwrap();
        assert_eq!(attached.payload(held), Some(99), "the create attached");
        drop(attached);
        drop(first);
        let fresh = SharedHolderTable::reset(&path, 4).unwrap();
        assert_eq!(fresh.live(), 0, "the reset released every slot");
        drop(fresh);
        clear(&path);
    }

    /// Two first-time creators of an empty claim file, one stopped midway
    /// through its layout, its capacity written and its magic not, and the
    /// other creating and claiming in that window without waiting on it:
    /// the claim stands, and the stopped creator, released, finds the table
    /// published and the slot held.
    #[test]
    fn a_creator_racing_the_first_layout_keeps_the_claim_made() {
        let path = tmp("first-layout");
        clear(&path);
        std::fs::File::create(&path).expect("an empty claim file, as a launcher pre-creates it");
        let stopped_path = path.clone();
        let (pause, first) = crate::test_races::stopped(move || {
            let table = SharedHolderTable::create(&stopped_path, 1).expect("the stopped create");
            table.claim(7)
        });
        let table = SharedHolderTable::create(&path, 1).expect("the second create");
        assert_eq!(table.claim(9), Some(0), "the second creator claims the one slot");
        pause.release();
        let stopped_claim = first.join().expect("the stopped creator");
        assert_eq!(stopped_claim, None, "the slot is held, so the stopped creator gets none");
        assert_eq!(table.payload(0), Some(9), "the claim made stands");
        drop(table);
        clear(&path);
    }

    /// A one-slot claim file whose header holds `capacity` and `magic`,
    /// as a creator that stopped partway leaves it.
    fn header_left(path: &Path, capacity: u64, magic: u64) {
        let total = shared_holder_file_size(1);
        let file = crate::region_file::create_or_open(path).expect("the claim file");
        file.set_len(total as u64).expect("the claim file takes the table's length");
        let mmap = unsafe { MmapOptions::new().len(total).map_mut(&file).expect("the claim file maps") };
        let hdr = unsafe { &*(mmap.as_ptr() as *const SharedHolderHeader) };
        hdr.capacity.store(capacity, Ordering::Release);
        hdr.magic.store(magic, Ordering::Release);
    }

    /// A layout a creator left unfinished, as one that died between its
    /// capacity and its magic leaves the file, is finished by the next
    /// creator at once rather than waited on or refused.
    #[test]
    fn a_create_finishes_a_layout_a_creator_left_unfinished() {
        let path = tmp("unfinished");
        clear(&path);
        header_left(&path, 1, 0);
        let started = Instant::now();
        let table = SharedHolderTable::create(&path, 1).expect("the create finishes the layout");
        assert!(
            started.elapsed() < crate::mmf_attach::INIT_WAIT / 2,
            "the create took {:?}, as if waiting on the creator that left it",
            started.elapsed()
        );
        assert_eq!(table.claim(3), Some(0), "the finished table takes a claim");
        drop(table);
        clear(&path);
    }

    /// The in-progress magic of a table an earlier SubEtha was laying out
    /// when its creator died is another layout's magic: refused at once as
    /// a mismatch of the magic, and a reset lays the table out again.
    #[test]
    fn an_earlier_in_progress_magic_is_refused_at_once() {
        let path = tmp("earlier-in-progress");
        clear(&path);
        header_left(&path, 0, 1);
        let started = Instant::now();
        match SharedHolderTable::create(&path, 1) {
            Ok(_) => panic!("a table another layout was building was attached"),
            Err(e) => assert!(
                matches!(&e, SharedHolderError::LayoutMismatch(m)
                    if *m == crate::layout::Mismatch::new("magic", 1, SHARED_HOLDER_MAGIC)),
                "{e:?}"
            ),
        }
        assert!(
            started.elapsed() < crate::mmf_attach::INIT_WAIT / 2,
            "refused only after {:?}",
            started.elapsed()
        );
        let table = SharedHolderTable::reset(&path, 1).expect("the reset");
        assert_eq!(table.claim(5), Some(0), "the reset table takes a claim");
        drop(table);
        clear(&path);
    }

    /// A create asking for more slots than the table at the path holds is
    /// refused, and the claims already made on that table stand.
    #[test]
    fn a_create_asking_more_slots_than_the_table_holds_is_refused() {
        let path = tmp("more-slots");
        clear(&path);
        let small = SharedHolderTable::create(&path, 2).unwrap();
        let slot = small.claim(5).expect("a free slot");
        match SharedHolderTable::create(&path, 4) {
            Ok(_) => panic!("a four-slot create over a two-slot table was accepted"),
            Err(e) => assert!(
                matches!(e, SharedHolderError::LayoutMismatch(m) if m == crate::layout::Mismatch::new(
                    "length",
                    shared_holder_file_size(2) as u64,
                    shared_holder_file_size(4) as u64,
                )),
                "{e:?}",
            ),
        }
        assert_eq!(small.payload(slot), Some(5), "the claim made stands");
        drop(small);
        clear(&path);
    }
}
