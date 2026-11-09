//! `HolderTable` - a fixed array of claimable slots, each stamped with
//! the process holding it, so a holder that dies can be told from one
//! that is working.
//!
//! # Consumers
//!
//! The peer directory's consumer slots, the pin table behind
//! [`SharedEpochs`](crate::shared_epochs::SharedEpochs), and the
//! holders of a [`SharedArc`](crate::shared_arc::SharedArc). Each takes
//! a numbered slot, holds it, and releases it, and each has to resolve
//! the holder whose process dies without releasing - which a bare count
//! cannot, because a number cannot be asked whether it is still
//! running.
//!
//! # A view, not a mapping
//!
//! The table does not own memory. It is constructed over slots the
//! caller has already mapped, so it sits inside whatever header layout
//! that caller needs - an epoch counter beside its pins, a payload
//! beside its holders.
//!
//! # Reserve, then publish
//!
//! A slot is claimed in two steps. A caller whose payload depends on
//! shared state read at claim time must make the slot visible before
//! reading that state, or another party observes the slot as free, acts
//! on that, and is wrong an instant later.
//! [`reserve`](HolderTable::reserve) publishes the slot with no payload
//! decided; [`publish`](HolderTable::publish) then fills it in.
//! [`claim`](HolderTable::claim) does both for a caller whose payload
//! does not depend on anything read between them.
//!
//! A reader that must not act on a half-formed claim uses
//! [`try_fold`](HolderTable::try_fold), which reports that it saw a
//! reservation instead of reading past it.

use std::mem::size_of;
use std::sync::atomic::{AtomicU64, Ordering};

use crate::process_identity;

/// A slot nothing holds.
pub const HOLDER_FREE: u64 = 0;

/// A slot claimed whose payload has not been decided yet. Held across
/// the caller's decision and never reported as a holder.
pub const HOLDER_RESERVED: u64 = u64::MAX;

/// One claimable slot: a payload word and the process holding it, on
/// its own cache line so two holders never share one.
#[repr(C, align(64))]
pub struct HolderSlot {
    /// [`HOLDER_FREE`], [`HOLDER_RESERVED`], or the caller's payload.
    pub state: AtomicU64,
    /// The holder word (pid and process start stamp, see
    /// `process_identity`) of the process that claimed it, for the
    /// dead-holder reap; zero while the slot is free or mid-claim.
    pub owner: AtomicU64,
    _pad: [u8; 48],
}

const _: () = {
    assert!(size_of::<HolderSlot>() == 64);
};

/// Bytes `capacity` slots occupy.
pub const fn holder_table_size(capacity: usize) -> usize {
    capacity * size_of::<HolderSlot>()
}

/// A view over `capacity` [`HolderSlot`]s the caller has mapped.
pub struct HolderTable {
    base: *const HolderSlot,
    capacity: usize,
}

// The slots are atomics in shared memory; the view is a pointer and a
// length, and every access goes through an atomic.
unsafe impl Send for HolderTable {}
unsafe impl Sync for HolderTable {}

impl HolderTable {
    /// Build a view over `capacity` slots at `base`.
    ///
    /// # Safety
    /// `base` addresses at least `holder_table_size(capacity)` bytes
    /// that are mapped, 64-byte aligned, and live for as long as the
    /// view. A freshly zeroed region is already a table of free slots.
    pub unsafe fn from_ptr(base: *const u8, capacity: usize) -> Self {
        debug_assert!(
            (base as usize).is_multiple_of(64),
            "holder slots must be 64-byte aligned"
        );
        Self { base: base as *const HolderSlot, capacity }
    }

    #[inline]
    pub fn capacity(&self) -> usize {
        self.capacity
    }

    /// The slot at index `i`.
    ///
    /// # Panics
    /// If `i` is not below the capacity, in every build: past it the
    /// offset leaves the table.
    #[inline]
    pub fn slot(&self, i: usize) -> &HolderSlot {
        assert!(i < self.capacity, "slot {i} is past the {} the table holds", self.capacity);
        // SAFETY: `i` is below the capacity, and `from_ptr`'s caller vouched
        // for that many slots at `base`.
        unsafe { &*self.base.add(i) }
    }

    /// Claim a slot without deciding its payload, returning its index.
    ///
    /// The slot is visible as reserved from the moment this returns, so
    /// a caller reading shared state to build its payload reads it
    /// after the claim is observable. Follow with
    /// [`publish`](Self::publish) or [`release`](Self::release); a
    /// reservation left standing blocks [`try_fold`](Self::try_fold)
    /// until the holder's process is found gone.
    pub fn reserve(&self) -> Option<usize> {
        let me = process_identity::current();
        for i in 0..self.capacity {
            let slot = self.slot(i);
            if slot.state.load(Ordering::Acquire) != HOLDER_FREE {
                continue;
            }
            if slot
                .state
                .compare_exchange(
                    HOLDER_FREE,
                    HOLDER_RESERVED,
                    Ordering::AcqRel,
                    Ordering::Acquire,
                )
                .is_ok()
            {
                slot.owner.store(me, Ordering::Release);
                return Some(i);
            }
        }
        None
    }

    /// Fill in a reserved slot's payload.
    ///
    /// # Panics
    /// If `payload` is [`HOLDER_FREE`] or [`HOLDER_RESERVED`], which
    /// would make a held slot read as free or as forever-forming, or if
    /// `slot` is not below the capacity.
    pub fn publish(&self, slot: usize, payload: u64) {
        assert!(
            payload != HOLDER_FREE && payload != HOLDER_RESERVED,
            "payload {payload} collides with a reserved state"
        );
        self.slot(slot).state.store(payload, Ordering::Release);
    }

    /// Claim a slot and publish `payload` in one step, for a caller
    /// whose payload does not depend on anything read between the two.
    ///
    /// # Panics
    /// If `payload` is [`HOLDER_FREE`] or [`HOLDER_RESERVED`], as
    /// [`publish`](Self::publish) does, before any slot is reserved.
    pub fn claim(&self, payload: u64) -> Option<usize> {
        assert!(
            payload != HOLDER_FREE && payload != HOLDER_RESERVED,
            "payload {payload} collides with a reserved state"
        );
        let i = self.reserve()?;
        self.publish(i, payload);
        Some(i)
    }

    /// Claim a single named slot, or report that it is already held.
    ///
    /// [`reserve`](Self::reserve) takes whichever slot is free, which is
    /// what a caller wants when the slots are interchangeable. A caller
    /// for whom they are not - one that must hold the slot standing for
    /// a particular thing, and would be wrong holding any other - names
    /// it here instead. Returns whether the claim succeeded.
    ///
    /// # Panics
    /// If `payload` is [`HOLDER_FREE`] or [`HOLDER_RESERVED`], as
    /// [`publish`](Self::publish) does, or if `i` is not below the
    /// capacity.
    pub fn try_claim_slot(&self, i: usize, payload: u64) -> bool {
        assert!(
            payload != HOLDER_FREE && payload != HOLDER_RESERVED,
            "payload {payload} collides with a reserved state"
        );
        let slot = self.slot(i);
        if slot.state.load(Ordering::Acquire) != HOLDER_FREE {
            return false;
        }
        if slot
            .state
            .compare_exchange(HOLDER_FREE, HOLDER_RESERVED, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            return false;
        }
        slot.owner.store(process_identity::current(), Ordering::Release);
        slot.state.store(payload, Ordering::Release);
        true
    }

    /// Return a slot to the table.
    ///
    /// # Panics
    /// If `slot` is not below the capacity.
    pub fn release(&self, slot: usize) {
        let s = self.slot(slot);
        s.owner.store(0, Ordering::Release);
        s.state.store(HOLDER_FREE, Ordering::Release);
    }

    /// Free `slot` only while it still holds what the caller observed:
    /// process `pid` and payload `payload`. A claimant that has told a
    /// stale claim from a live one by its payload, such as a holder's
    /// process creation time when the pid now names another process,
    /// frees it this way without freeing a holder that claimed the slot
    /// since. Returns whether it freed the slot.
    ///
    /// The owner goes to zero first, which marks the slot mid-claim: a reap
    /// leaves it alone, and no claim can start while its state is held.
    /// The state is then freed only if it is still `payload`; otherwise
    /// the owner is put back.
    ///
    /// # Panics
    /// If `slot` is not below the capacity.
    pub fn release_if(&self, slot: usize, pid: u32, payload: u64) -> bool {
        if pid == 0 || payload == HOLDER_FREE || payload == HOLDER_RESERVED {
            return false;
        }
        let s = self.slot(slot);
        let owner = s.owner.load(Ordering::Acquire);
        if process_identity::holder_pid(owner) != pid
            || s.owner.compare_exchange(owner, 0, Ordering::AcqRel, Ordering::Acquire).is_err()
        {
            return false;
        }
        if s.state
            .compare_exchange(payload, HOLDER_FREE, Ordering::AcqRel, Ordering::Acquire)
            .is_ok()
        {
            return true;
        }
        // The same process holds the slot under another payload. The owner
        // goes back unless a claim that followed the holder's own release
        // has already stamped its own.
        let _restored = s
            .owner
            .compare_exchange(0, owner, Ordering::AcqRel, Ordering::Acquire)
            .is_ok();
        false
    }

    /// The payload a slot holds, or `None` if it is free or still
    /// forming.
    ///
    /// # Panics
    /// If `slot` is not below the capacity.
    pub fn payload(&self, slot: usize) -> Option<u64> {
        match self.slot(slot).state.load(Ordering::Acquire) {
            HOLDER_FREE | HOLDER_RESERVED => None,
            v => Some(v),
        }
    }

    /// Slots held, reservations included.
    pub fn live(&self) -> usize {
        (0..self.capacity)
            .filter(|i| self.slot(*i).state.load(Ordering::Acquire) != HOLDER_FREE)
            .count()
    }

    /// Fold over every published payload, or report that a slot was
    /// mid-claim.
    ///
    /// `None` means a reservation was seen and the fold is not
    /// answerable yet - the caller decides whether to retry, to reap,
    /// or to treat it as a reason to do nothing. Reading past a
    /// reservation would mean acting on a table that is about to gain a
    /// holder.
    pub fn try_fold<T>(&self, init: T, mut f: impl FnMut(T, u64) -> T) -> Option<T> {
        let mut acc = init;
        for i in 0..self.capacity {
            match self.slot(i).state.load(Ordering::Acquire) {
                HOLDER_FREE => {}
                HOLDER_RESERVED => return None,
                v => acc = f(acc, v),
            }
        }
        Some(acc)
    }

    /// Free every slot whose holding process is gone, and report how
    /// many went.
    ///
    /// A slot outlives its process only when that process died holding
    /// it, so it names a holder that will never release. The holder is
    /// judged by its pid and the start its claim recorded, so a pid the
    /// system has handed to another process reads as gone. A slot claimed
    /// but not yet stamped with an owner is mid-claim, not dead, and is
    /// left alone.
    pub fn reap_dead(&self) -> usize {
        let mut freed = 0;
        for i in 0..self.capacity {
            let slot = self.slot(i);
            let state = slot.state.load(Ordering::Acquire);
            if state == HOLDER_FREE {
                continue;
            }
            let owner = slot.owner.load(Ordering::Acquire);
            if owner == 0 || !process_identity::holder_gone(owner) {
                continue;
            }
            if slot
                .state
                .compare_exchange(state, HOLDER_FREE, Ordering::AcqRel, Ordering::Acquire)
                .is_ok()
            {
                freed += 1;
            }
        }
        freed
    }
}

impl std::fmt::Debug for HolderTable {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HolderTable")
            .field("capacity", &self.capacity)
            .field("live", &self.live())
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Slots on the heap, 64-byte aligned, standing in for a mapping.
    struct Backing {
        _buf: Vec<HolderSlot>,
        table: HolderTable,
    }

    fn backing(capacity: usize) -> Backing {
        let mut buf: Vec<HolderSlot> = (0..capacity)
            .map(|_| HolderSlot {
                state: AtomicU64::new(HOLDER_FREE),
                owner: AtomicU64::new(0),
                _pad: [0; 48],
            })
            .collect();
        let table = unsafe { HolderTable::from_ptr(buf.as_mut_ptr() as *const u8, capacity) };
        Backing { _buf: buf, table }
    }

    /// A slot past the capacity panics in every build rather than reading
    /// past the slots.
    #[test]
    #[should_panic(expected = "slot 4 is past the 4 the table holds")]
    fn a_slot_past_the_capacity_panics_rather_than_leaving_the_table() {
        let b = backing(4);
        b.table.release(4);
    }

    /// A claim with a payload the table keeps for itself is refused before
    /// it reserves anything, so the refusal leaves no slot held.
    #[test]
    fn a_claim_refused_for_its_payload_holds_no_slot() {
        let b = backing(4);
        let refused = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| b.table.claim(HOLDER_RESERVED)));
        assert!(refused.is_err(), "a reserved state is not a payload");
        assert_eq!(b.table.live(), 0, "the refused claim reserved nothing");
    }

    #[test]
    fn a_claim_takes_the_first_free_slot_and_a_release_returns_it() {
        let b = backing(4);
        let t = &b.table;
        assert_eq!(t.claim(7), Some(0));
        assert_eq!(t.claim(8), Some(1));
        assert_eq!(t.payload(0), Some(7));
        assert_eq!(t.live(), 2);
        t.release(0);
        assert_eq!(t.payload(0), None);
        assert_eq!(t.claim(9), Some(0), "the released slot is reused");
    }

    /// A claim a claimant has found stale is freed given the pid and
    /// payload it observed.
    #[test]
    fn a_stale_claim_is_freed_given_the_pair_observed() {
        let b = backing(1);
        let t = &b.table;
        assert_eq!(t.claim(7), Some(0));
        let pid = process_identity::holder_pid(t.slot(0).owner.load(Ordering::Acquire));
        assert!(t.release_if(0, pid, 7), "the observed pair frees the slot");
        assert_eq!(t.payload(0), None);
        assert_eq!(t.slot(0).owner.load(Ordering::Acquire), 0);
        assert_eq!(t.claim(9), Some(0), "the freed slot is claimable");
    }

    /// A pair another claimant has replaced frees nothing: a new payload
    /// under the same process, or the old payload under another process.
    #[test]
    fn a_replaced_pair_leaves_the_new_holder_in_place() {
        let b = backing(1);
        let t = &b.table;
        assert_eq!(t.claim(7), Some(0));
        let owner = t.slot(0).owner.load(Ordering::Acquire);
        let pid = process_identity::holder_pid(owner);
        t.release(0);
        assert_eq!(t.claim(9), Some(0));
        assert!(!t.release_if(0, pid, 7), "a new payload stays held");
        assert_eq!(t.payload(0), Some(9));
        assert_eq!(t.slot(0).owner.load(Ordering::Acquire), owner, "and keeps its owner");
        t.slot(0).owner.store(process_identity::holder(pid.wrapping_add(1), 0), Ordering::Release);
        t.slot(0).state.store(7, Ordering::Release);
        assert!(!t.release_if(0, pid, 7), "the same payload under another process stays held");
        assert_eq!(t.payload(0), Some(7));
    }

    #[test]
    fn a_full_table_refuses() {
        let b = backing(2);
        let t = &b.table;
        assert_eq!(t.claim(1), Some(0));
        assert_eq!(t.claim(2), Some(1));
        assert_eq!(t.claim(3), None);
        assert_eq!(t.live(), 2, "the refusal left both holders alone");
    }

    /// The ordering the whole primitive exists for: a slot is visible
    /// before its payload is decided, and a reader must not read past
    /// that.
    #[test]
    fn a_reservation_is_visible_but_not_yet_a_payload() {
        let b = backing(4);
        let t = &b.table;
        let i = t.reserve().unwrap();
        assert_eq!(t.payload(i), None, "reserved is not a published payload");
        assert_eq!(t.live(), 1, "but it does hold the slot");
        assert_eq!(
            t.try_fold(0u64, |a, v| a + v),
            None,
            "a fold must not read past a slot that is mid-claim"
        );
        t.publish(i, 42);
        assert_eq!(t.try_fold(0u64, |a, v| a + v), Some(42));
    }

    #[test]
    fn a_fold_sums_every_published_payload() {
        let b = backing(8);
        let t = &b.table;
        for p in 1..=5u64 {
            t.claim(p).unwrap();
        }
        assert_eq!(t.try_fold(0u64, |a, v| a + v), Some(15));
        assert_eq!(t.try_fold(u64::MAX, |a, v| a.min(v)), Some(1));
    }

    #[test]
    fn a_slot_whose_process_died_is_reaped() {
        let b = backing(4);
        let t = &b.table;
        t.claim(7).unwrap();
        let dead = t.slot(1);
        dead.state.store(9, Ordering::Release);
        dead.owner.store(process_identity::holder(u32::MAX - 1, 0), Ordering::Release);
        assert_eq!(t.live(), 2);
        assert_eq!(t.reap_dead(), 1);
        assert_eq!(t.live(), 1, "the live holder is untouched");
        assert_eq!(t.payload(0), Some(7));
    }

    /// A slot whose owner's pid names this live process but under another
    /// start, as a pid the system handed on after its holder died, is
    /// reaped; this process's own claim is not.
    #[test]
    fn a_slot_whose_pid_names_another_process_is_reaped() {
        let b = backing(2);
        let t = &b.table;
        t.claim(7).unwrap();
        let me = process_identity::current();
        let other_start = process_identity::holder_stamp(me).wrapping_add(1).max(1);
        let stale = t.slot(1);
        stale.state.store(9, Ordering::Release);
        stale.owner.store(process_identity::holder(process_identity::holder_pid(me), other_start), Ordering::Release);
        assert_eq!(t.reap_dead(), 1);
        assert_eq!(t.payload(0), Some(7), "this process's own claim stays");
        assert_eq!(t.payload(1), None);
    }

    #[test]
    fn a_reservation_whose_process_died_is_reaped_too() {
        let b = backing(2);
        let t = &b.table;
        let dead = t.slot(0);
        dead.state.store(HOLDER_RESERVED, Ordering::Release);
        dead.owner.store(process_identity::holder(u32::MAX - 1, 0), Ordering::Release);
        assert_eq!(
            t.try_fold(0u64, |a, v| a + v),
            None,
            "the fold is blocked while it stands"
        );
        assert_eq!(t.reap_dead(), 1);
        assert_eq!(t.try_fold(0u64, |a, v| a + v), Some(0), "and unblocked after");
    }

    #[test]
    #[should_panic(expected = "collides with a reserved state")]
    fn publishing_a_reserved_sentinel_is_refused() {
        let b = backing(2);
        let i = b.table.reserve().unwrap();
        b.table.publish(i, HOLDER_FREE);
    }

    #[test]
    fn concurrent_claims_never_hand_two_callers_one_slot() {
        let b = std::sync::Arc::new(backing(64));
        // Each claimer keeps the slots it won and hands them back on join.
        let mut all: Vec<usize> = std::thread::scope(|s| {
            let claimers: Vec<_> = (0..8)
                .map(|_| {
                    let b = std::sync::Arc::clone(&b);
                    s.spawn(move || {
                        let mut mine = Vec::new();
                        for p in 1..=8u64 {
                            if let Some(i) = b.table.claim(p) {
                                mine.push(i);
                            }
                        }
                        mine
                    })
                })
                .collect();
            claimers
                .into_iter()
                .flat_map(|claimer| claimer.join().expect("a claimer ended by panic"))
                .collect()
        });
        let total = all.len();
        all.sort_unstable();
        all.dedup();
        assert_eq!(all.len(), total, "a slot was handed to two callers");
        assert_eq!(b.table.live(), total);
    }

    unsafe impl Send for Backing {}
    unsafe impl Sync for Backing {}
}
