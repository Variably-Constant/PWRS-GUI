//! The open-addressing table of Purcell and Harris (2005, University of
//! Cambridge technical report UCAM-CL-TR-639) for entries of at most 8
//! bytes. Each bucket keeps the probe bound of the keys homed at it, and a
//! lookup scans a key's home bucket out to that bound, so a slot is free the
//! moment its key is removed and no tombstone is left. No operation waits on
//! another: an insert passes over a copy of its key still being claimed,
//! finishes or collides one being checked, and, once no slot is free, frees
//! every slot a dead writer left, a remover's or an insert's.
//!
//! A bucket is 32 bytes: its probe bound, the holder word of the insert
//! that claimed its slot, then a 16-byte slot whose head word (a version
//! above three state bits) and entry word change together by one
//! compare-and-swap. The bound is the furthest offset along the linear probe
//! sequence at which a key homed here may sit, shifted left one, with bit 0
//! set while a remove scans to lower it. A slot's version moves on every
//! time it is freed, so a swap from a stale reading of it fails. The holder
//! word is written by the claimant just after its claim and zeroed before
//! every free, so a claimed slot whose word names a process that is gone
//! was left by a dead insert, and an insert that finds no slot free frees
//! it; a claimant that dies between its claim and its word leaves a slot no
//! one can judge. A collided slot is otherwise freed by the insert that
//! claimed it.
//!
//! An entry holds its key in its low `key_size` bytes, in memory order, and
//! its value above them.

use std::sync::atomic::{AtomicU64, Ordering};

use crate::atomic_pair::AtomicPair;
use crate::process_identity;

/// Bytes one bucket takes.
pub(crate) const BUCKET: usize = 32;

const EMPTY: u64 = 0;
/// Claimed by an insert that has not yet begun checking for other copies of
/// its key; written by its claimant alone.
const VISIBLE: u64 = 1;
/// An insert checking the bound's range for other copies of its key.
const INSERTING: u64 = 2;
const MEMBER: u64 = 3;
/// An insert that lost to another copy of its key.
const COLLIDED: u64 = 4;
/// A remove lowering the bound; the entry word holds the remover's holder
/// word.
const BUSY: u64 = 5;
const STATE: u64 = 7;

#[inline(always)]
fn head(version: u64, state: u64) -> u64 {
    (version << 3) | state
}

#[inline(always)]
fn state_of(h: u64) -> u64 {
    h & STATE
}

#[inline(always)]
fn version_of(h: u64) -> u64 {
    h >> 3
}

#[inline(always)]
fn pair(h: u64, e: u64) -> u128 {
    u128::from(h) | (u128::from(e) << 64)
}

#[inline(always)]
fn split(p: u128) -> (u64, u64) {
    (p as u64, (p >> 64) as u64)
}

/// What an insert did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Placed {
    /// The key was absent and now holds the entry.
    Inserted,
    /// The key held this entry, which the new one replaced.
    Replaced(u64),
    /// The key held this entry, left as it was.
    Present(u64),
}

/// No bucket along the key's probe sequence is free.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Full;

/// A table of `capacity` buckets in memory another participant may share.
pub(crate) struct W8Table {
    base: *mut u8,
    capacity: usize,
    /// `capacity - 1` when `capacity` is a power of two above 1, so a home
    /// is found by a mask; 0 otherwise, for the modulo.
    mask: usize,
    key_size: usize,
    /// The bits of an entry that hold its key.
    key_mask: u64,
    /// Whether a plain 16-byte load or store is atomic here, read once.
    plain: bool,
}

// SAFETY: every access to the buckets is an atomic load, store or swap.
unsafe impl Send for W8Table {}
// SAFETY: as for Send.
unsafe impl Sync for W8Table {}

impl W8Table {
    /// The table over `capacity` buckets at `base`, for keys of `key_size`
    /// bytes.
    ///
    /// # Safety
    /// `base` is 16-byte aligned and addresses `capacity * BUCKET` bytes,
    /// readable and writable for the table's life and accessed only through
    /// tables over them; `capacity` is at least 1, `key_size` is 1 to 8, and
    /// [`crate::atomic_pair::supported`] returned true.
    pub(crate) unsafe fn new(base: *mut u8, capacity: usize, key_size: usize) -> Self {
        debug_assert!(base.addr().is_multiple_of(16) && capacity > 0 && (1..=8).contains(&key_size));
        let mask = if capacity > 1 && capacity.is_power_of_two() { capacity - 1 } else { 0 };
        let key_mask = if key_size == 8 { u64::MAX } else { (1u64 << (8 * key_size)) - 1 };
        let plain = crate::atomic_pair::plain_sixteen();
        Self { base, capacity, mask, key_size, key_mask, plain }
    }

    /// A table over no buckets, held by a map that keeps its entries some
    /// other way and never calls it.
    pub(crate) const fn unused() -> Self {
        Self { base: std::ptr::null_mut(), capacity: 0, mask: 0, key_size: 0, key_mask: 0, plain: false }
    }

    #[inline(always)]
    fn key_of(&self, e: u64) -> u64 {
        e & self.key_mask
    }

    /// The home bucket of `key`: FNV-1a over its bytes, modulo the capacity.
    #[inline]
    pub(crate) fn home(&self, key: u64) -> usize {
        self.home_of(crate::shared_hash_map::fnv1a_64(&key.to_le_bytes()[..self.key_size]))
    }

    /// The home bucket of a key whose FNV-1a hash over its bytes is `hash`,
    /// for a caller that hashes its key at a size it knows when compiled.
    #[inline(always)]
    pub(crate) fn home_of(&self, hash: u64) -> usize {
        if self.mask != 0 { hash as usize & self.mask } else { (hash % self.capacity as u64) as usize }
    }

    /// The bucket `off` along the probe sequence from `home`; both are below
    /// the capacity.
    #[inline(always)]
    fn at(&self, home: usize, off: usize) -> usize {
        let i = home + off;
        if i >= self.capacity { i - self.capacity } else { i }
    }

    #[inline(always)]
    fn bound(&self, i: usize) -> &AtomicU64 {
        // SAFETY: bucket `i` lies in the table and starts with its bound.
        unsafe { &*(self.base.add(i * BUCKET) as *const AtomicU64) }
    }

    /// The holder word of the insert that claimed bucket `i`'s slot, 0 while
    /// the slot is free or its claimant has not yet written it.
    #[inline(always)]
    fn claimant(&self, i: usize) -> &AtomicU64 {
        // SAFETY: bucket `i` lies in the table, its holder word 8 bytes in.
        unsafe { &*(self.base.add(i * BUCKET + 8) as *const AtomicU64) }
    }

    /// Free slot `i`, at version `ver` and held by this process alone (busy,
    /// or the collided slot of this process's insert): zero its holder word,
    /// then write it empty at the next version.
    #[inline(always)]
    fn free(&self, i: usize, ver: u64) {
        self.claimant(i).store(0, Ordering::Release);
        self.write(i, pair(head(ver.wrapping_add(1), EMPTY), 0));
    }

    #[inline(always)]
    fn slot(&self, i: usize) -> &AtomicPair {
        // SAFETY: bucket `i` lies in the table and ends with its slot,
        // 16-byte aligned, accessed only as a pair.
        unsafe { AtomicPair::at(self.base.add(i * BUCKET + 16)) }
    }

    /// Slot `i`'s word, read whole.
    #[inline(always)]
    fn read(&self, i: usize) -> u128 {
        self.slot(i).load_as(self.plain)
    }

    /// Write slot `i`'s word whole.
    #[inline(always)]
    fn write(&self, i: usize, v: u128) {
        self.slot(i).store_as(v, self.plain)
    }

    #[inline(always)]
    fn bound_of(&self, home: usize) -> usize {
        ((self.bound(home).load(Ordering::Acquire) >> 1) as usize).min(self.capacity - 1)
    }

    /// The member slot holding `key`: its offset from `home`, its bucket and
    /// its pair.
    #[inline(always)]
    fn find_member(&self, home: usize, key: u64) -> Option<(usize, usize, u128)> {
        let span = self.bound_of(home) + 1;
        let mut i = home;
        for off in 0..span {
            let p = self.read(i);
            let (h, e) = split(p);
            if state_of(h) == MEMBER && self.key_of(e) == key {
                return Some((off, i, p));
            }
            i += 1;
            if i == self.capacity {
                i = 0;
            }
        }
        None
    }

    /// The entry holding `key`, homed at `home`, if it is present.
    #[inline(always)]
    pub(crate) fn get(&self, home: usize, key: u64) -> Option<u64> {
        self.find_member(home, key).map(|(_, _, p)| split(p).1)
    }

    /// Insert `entry` for its key, homed at `home`. A present key keeps its
    /// entry unless `replace`, when one swap replaces it.
    #[inline(always)]
    pub(crate) fn insert(&self, home: usize, entry: u64, replace: bool) -> Result<Placed, Full> {
        let key = self.key_of(entry);
        loop {
            if let Some((_, i, p)) = self.find_member(home, key) {
                let (h, old) = split(p);
                if !replace {
                    return Ok(Placed::Present(old));
                }
                if self.slot(i).compare_exchange(p, pair(h, entry)).is_ok() {
                    return Ok(Placed::Replaced(old));
                }
                continue;
            }
            if self.insert_absent(home, key, entry)? {
                return Ok(Placed::Inserted);
            }
        }
    }

    /// Replace the entry of `expected`'s key, homed at `home`, with `new` if
    /// it is `expected`: `Ok` when replaced, `Err` with the entry found, or
    /// `None` when the key is absent.
    #[inline(always)]
    pub(crate) fn compare_exchange(&self, home: usize, expected: u64, new: u64) -> Option<Result<(), u64>> {
        let key = self.key_of(expected);
        loop {
            let (_, i, p) = self.find_member(home, key)?;
            let (h, found) = split(p);
            if found != expected {
                return Some(Err(found));
            }
            if self.slot(i).compare_exchange(p, pair(h, new)).is_ok() {
                return Some(Ok(()));
            }
        }
    }

    /// Take `key`, homed at `home`, out: the entry it held.
    #[inline(always)]
    pub(crate) fn remove(&self, home: usize, key: u64) -> Option<u64> {
        loop {
            let (off, i, p) = self.find_member(home, key)?;
            let (h, e) = split(p);
            let busy = pair(head(version_of(h), BUSY), process_identity::current());
            if self.slot(i).compare_exchange(p, busy).is_err() {
                continue;
            }
            self.lower_bound(home, off);
            // Only this remover writes the slot while it holds it busy.
            self.free(i, version_of(h));
            return Some(e);
        }
    }

    /// Insert `entry`, whose key is absent: true when it went in, false when
    /// another copy of the key won.
    #[inline(never)]
    fn insert_absent(&self, home: usize, key: u64, entry: u64) -> Result<bool, Full> {
        let (off, ver) = self.claim(home, entry)?;
        let i = self.at(home, off);
        self.raise_bound(home, off);
        // A visible slot is written by its claimant alone.
        self.write(i, pair(head(ver, INSERTING), entry));
        self.assist(home, key, off, ver, entry);
        Ok(self.outcome(home, key, off, ver, entry))
    }

    /// Whether the insert that claimed slot `off` from `home` at version
    /// `ver` went in. A slot past that version was a member and was removed
    /// since; one still inserting is finished here; a collided one lost and
    /// is freed by this, its claimant.
    fn outcome(&self, home: usize, key: u64, off: usize, ver: u64, entry: u64) -> bool {
        let i = self.at(home, off);
        loop {
            let (h, _) = split(self.read(i));
            if version_of(h) != ver {
                return true;
            }
            match state_of(h) {
                MEMBER | BUSY => return true,
                INSERTING => self.assist(home, key, off, ver, entry),
                _ => {
                    self.lower_bound(home, off);
                    self.free(i, ver);
                    return false;
                }
            }
        }
    }

    /// Claim the first free slot along `home`'s probe sequence for `entry`,
    /// as visible, and write this process's holder word beside it: its
    /// offset and version. When no slot is free and some slot holds a writer
    /// in flight, every slot a writer that is gone left is freed and the
    /// sequence read again; `Full` once every slot holds a member, or a pass
    /// frees none.
    fn claim(&self, home: usize, entry: u64) -> Result<(usize, u64), Full> {
        let me = process_identity::current();
        loop {
            let mut in_flight = false;
            for off in 0..self.capacity {
                let i = self.at(home, off);
                loop {
                    let p = self.read(i);
                    let (h, _) = split(p);
                    let state = state_of(h);
                    if state != EMPTY {
                        in_flight |= state != MEMBER;
                        break;
                    }
                    if self.slot(i).compare_exchange(p, pair(head(version_of(h), VISIBLE), entry)).is_ok() {
                        self.claimant(i).store(me, Ordering::Release);
                        return Ok((off, version_of(h)));
                    }
                }
            }
            if !in_flight || !self.free_the_dead(me) {
                return Err(Full);
            }
        }
    }

    /// Free every slot a writer that is gone left: busy under a remover
    /// that is gone, or claimed by an insert whose holder word names a
    /// process that is gone. This process is never judged gone, and a slot
    /// whose holder word is still 0 cannot be judged. Whether any slot was
    /// freed.
    #[cold]
    #[inline(never)]
    fn free_the_dead(&self, me: u64) -> bool {
        let mut freed = false;
        for i in 0..self.capacity {
            let p = self.read(i);
            let (h, e) = split(p);
            let state = state_of(h);
            // A remover writes its word with the swap that makes the slot
            // busy; a claimant writes its word just after its claim.
            let holder = match state {
                BUSY => e,
                VISIBLE | INSERTING | COLLIDED => match self.claimant(i).load(Ordering::Acquire) {
                    0 => continue,
                    word => word,
                },
                _ => continue,
            };
            if process_identity::holder_pid(holder) == process_identity::holder_pid(me)
                || !process_identity::holder_gone(holder)
            {
                continue;
            }
            // Hold the slot busy under this process, so its free is this
            // process's alone; a swap that fails found the slot moved on.
            let ver = version_of(h);
            if self.slot(i).compare_exchange(p, pair(head(ver, BUSY), me)).is_err() {
                continue;
            }
            if state != BUSY {
                // A claimed slot counts toward its key's home bound.
                let home = self.home(self.key_of(e));
                let off = if i >= home { i - home } else { i + self.capacity - home };
                self.lower_bound(home, off);
            }
            self.free(i, ver);
            freed = true;
        }
        freed
    }

    /// Check `home`'s range for other copies of `key` on behalf of the copy
    /// at offset `off`, version `ver`: an earlier inserting copy wins and is
    /// finished, a later one is collided, a member collides this one; with
    /// none left, this one becomes the member. A visible copy is passed over,
    /// since it checks the range itself once it is inserting. Every swap here
    /// is from an exact pair, so one a helper already made fails harmlessly,
    /// and the claimant reads its slot afterwards to learn what became of it.
    #[inline(always)]
    fn assist(&self, home: usize, key: u64, mut off: usize, mut ver: u64, mut entry: u64) {
        'copy: loop {
            let i = self.at(home, off);
            let mine = pair(head(ver, INSERTING), entry);
            let collided = pair(head(ver, COLLIDED), entry);
            let span = self.bound_of(home) + 1;
            let mut j = home;
            for j_off in 0..span {
                let this = j;
                j += 1;
                if j == self.capacity {
                    j = 0;
                }
                if j_off == off {
                    continue;
                }
                let p = self.read(this);
                let (h, e) = split(p);
                if self.key_of(e) != key {
                    continue;
                }
                match state_of(h) {
                    MEMBER => {
                        let lost = self.slot(i).compare_exchange(mine, collided);
                        debug_assert!(lost.is_ok() || self.read(i) != mine);
                        return;
                    }
                    INSERTING if j_off < off => {
                        if self.slot(i).compare_exchange(mine, collided).is_err() {
                            return;
                        }
                        // Finish the earlier copy as this one would have been.
                        (off, ver, entry) = (j_off, version_of(h), e);
                        continue 'copy;
                    }
                    INSERTING => {
                        if self.read(i) != mine {
                            return;
                        }
                        if self.slot(this).compare_exchange(p, pair(head(version_of(h), COLLIDED), e)).is_err() {
                            // That copy moved on: read the range again.
                            continue 'copy;
                        }
                    }
                    _ => {}
                }
            }
            let won = self.slot(i).compare_exchange(mine, pair(head(ver, MEMBER), entry));
            debug_assert!(won.is_ok() || self.read(i) != mine);
            return;
        }
    }

    /// Raise `home`'s bound to cover offset `off`, clearing a scan in
    /// progress so a remove lowering it reads the range again. The slot at
    /// `off` is claimed before this, so a scan that starts later counts it.
    fn raise_bound(&self, home: usize, off: usize) {
        let w = self.bound(home);
        let mut cur = w.load(Ordering::Acquire);
        loop {
            let bound = (cur >> 1) as usize;
            if bound >= off && cur & 1 == 0 {
                return;
            }
            let new = (bound.max(off) as u64) << 1;
            match w.compare_exchange(cur, new, Ordering::AcqRel, Ordering::Acquire) {
                Ok(_) => return,
                Err(now) => cur = now,
            }
        }
    }

    /// Lower `home`'s bound past offset `off`, being freed, when `off` is the
    /// bound: scan back for the furthest slot still holding a key homed
    /// there, and publish it unless a raise cleared the scan meanwhile.
    #[inline(always)]
    fn lower_bound(&self, home: usize, off: usize) {
        let w = self.bound(home);
        if w.load(Ordering::Acquire) & 1 != 0 {
            w.fetch_and(!1, Ordering::AcqRel);
        }
        if off != 0 {
            self.scan_bound_down(home, off);
        }
    }

    /// [`lower_bound`](Self::lower_bound)'s scan, for an offset past the home.
    #[inline(never)]
    fn scan_bound_down(&self, home: usize, off: usize) {
        let w = self.bound(home);
        let at = (off as u64) << 1;
        while w.compare_exchange(at, at | 1, Ordering::AcqRel, Ordering::Acquire).is_ok() {
            let mut back = off - 1;
            while back > 0 && !self.holds_homed(home, back) {
                back -= 1;
            }
            if w.compare_exchange(at | 1, (back as u64) << 1, Ordering::AcqRel, Ordering::Acquire).is_ok() {
                return;
            }
        }
    }

    /// Whether the slot at offset `off` from `home` holds a key homed there,
    /// claimed, being inserted or present.
    fn holds_homed(&self, home: usize, off: usize) -> bool {
        let (h, e) = split(self.read(self.at(home, off)));
        matches!(state_of(h), VISIBLE | INSERTING | MEMBER) && self.home(self.key_of(e)) == home
    }

    /// The entry slot `i`, below the capacity, holds when it is a member.
    pub(crate) fn member_at(&self, i: usize) -> Option<u64> {
        let (h, e) = split(self.read(i));
        (state_of(h) == MEMBER).then_some(e)
    }

    /// Every present entry, in slot order.
    pub(crate) fn entries(&self) -> Vec<u64> {
        (0..self.capacity)
            .filter_map(|i| {
                let (h, e) = split(self.read(i));
                (state_of(h) == MEMBER).then_some(e)
            })
            .collect()
    }

    /// Empty every slot and bound. Every other writer is stopped.
    pub(crate) fn clear(&self) {
        for i in 0..self.capacity {
            self.bound(i).store(0, Ordering::Release);
            let (h, _) = split(self.read(i));
            self.free(i, version_of(h));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicBool;

    /// Zeroed memory for `capacity` buckets, 16-byte aligned.
    fn memory(capacity: usize) -> Vec<u128> {
        vec![0u128; capacity * BUCKET / 16]
    }

    fn table(mem: &mut [u128], capacity: usize) -> W8Table {
        // SAFETY: `mem` holds `capacity` zeroed buckets, 16-byte aligned, and
        // outlives the table in each test.
        unsafe { W8Table::new(mem.as_mut_ptr() as *mut u8, capacity, 4) }
    }

    fn entry(k: u32, v: u32) -> u64 {
        u64::from(k) | (u64::from(v) << 32)
    }

    /// Insert `(k, v)`, homed as the table hashes `k`.
    fn put(t: &W8Table, k: u32, v: u32, replace: bool) -> Result<Placed, Full> {
        t.insert(t.home(u64::from(k)), entry(k, v), replace)
    }

    fn look(t: &W8Table, k: u32) -> Option<u64> {
        t.get(t.home(u64::from(k)), u64::from(k))
    }

    fn take(t: &W8Table, k: u32) -> Option<u64> {
        t.remove(t.home(u64::from(k)), u64::from(k))
    }

    fn swap(t: &W8Table, k: u32, expected: u32, new: u32) -> Option<Result<(), u64>> {
        t.compare_exchange(t.home(u64::from(k)), entry(k, expected), entry(k, new))
    }

    #[test]
    fn insert_get_replace_and_remove_round_trip() {
        let mut mem = memory(16);
        let t = table(&mut mem, 16);
        assert_eq!(put(&t, 7, 70, true), Ok(Placed::Inserted));
        assert_eq!(look(&t, 7), Some(entry(7, 70)));
        assert_eq!(put(&t, 7, 71, false), Ok(Placed::Present(entry(7, 70))));
        assert_eq!(put(&t, 7, 72, true), Ok(Placed::Replaced(entry(7, 70))));
        assert_eq!(swap(&t, 7, 1, 2), Some(Err(entry(7, 72))));
        assert_eq!(swap(&t, 7, 72, 73), Some(Ok(())));
        assert_eq!(take(&t, 7), Some(entry(7, 73)));
        assert_eq!(look(&t, 7), None);
        assert_eq!(take(&t, 7), None);
        assert_eq!(swap(&t, 7, 73, 74), None);
    }

    /// Removes free their slots at once: churn through many more keys than
    /// the table holds never fills it, and a full table refuses a new key.
    #[test]
    fn churn_never_fills_and_a_full_table_refuses() {
        let mut mem = memory(8);
        let t = table(&mut mem, 8);
        for k in 0..10_000u32 {
            assert_eq!(put(&t, k, k, true), Ok(Placed::Inserted), "key {k}");
            assert_eq!(take(&t, k), Some(entry(k, k)));
        }
        for k in 0..8u32 {
            assert_eq!(put(&t, k, k, true), Ok(Placed::Inserted));
        }
        assert_eq!(put(&t, 99, 0, true), Err(Full));
        assert_eq!(t.entries().len(), 8);
    }

    /// Threads inserting the same keys at once, and removing them, never
    /// leave a key present twice, and readers never see a torn entry.
    #[test]
    fn racing_inserts_and_removes_never_duplicate_a_key() {
        let mut mem = memory(64);
        let t = table(&mut mem, 64);
        let done = AtomicBool::new(false);
        std::thread::scope(|s| {
            let writers: Vec<_> = (0..6u32)
                .map(|w| {
                    let t = &t;
                    s.spawn(move || {
                        for round in 0..20_000u32 {
                            let k = round % 24;
                            if (round + w) % 3 == 0 {
                                take(t, k);
                            } else {
                                assert!(put(t, k, k ^ 0xABCD, true).is_ok(), "the table filled holding at most 24 keys");
                            }
                        }
                    })
                })
                .collect();
            s.spawn(|| {
                while !done.load(Ordering::Acquire) {
                    for k in 0..24u32 {
                        if let Some(e) = look(&t, k) {
                            assert_eq!(e, entry(k, k ^ 0xABCD), "a torn or foreign entry for key {k}");
                        }
                    }
                }
            });
            for w in writers {
                w.join().unwrap();
            }
            done.store(true, Ordering::Release);
        });
        let mut keys: Vec<u64> = t.entries().iter().map(|&e| e & 0xFFFF_FFFF).collect();
        let n = keys.len();
        keys.sort_unstable();
        keys.dedup();
        assert_eq!(keys.len(), n, "a key is present twice");
    }

    /// A remove that died holding its slot busy leaves nothing waiting: the
    /// next insert that meets the slot frees it once the remover is gone.
    #[test]
    fn a_dead_removers_busy_slot_is_freed_by_the_next_insert() {
        let mut mem = memory(1);
        let t = table(&mut mem, 1);
        assert_eq!(put(&t, 5, 50, true), Ok(Placed::Inserted));
        let gone = process_identity::holder(0, 0);
        let p = t.slot(0).load();
        let (h, _) = split(p);
        assert!(t.slot(0).compare_exchange(p, pair(head(version_of(h), BUSY), gone)).is_ok());
        assert_eq!(look(&t, 5), None, "a busy slot holds no key");
        assert_eq!(put(&t, 6, 60, true), Ok(Placed::Inserted), "the dead remover's slot was freed and reused");
        assert_eq!(look(&t, 6), Some(entry(6, 60)));
    }

    /// The holder word of a process that is gone: no pid lies past the
    /// system's range.
    const GONE: u64 = process_identity::holder(u32::MAX, 1);

    /// A two-bucket table holding key 1, with its other slot left in `state`
    /// for key 2 by a claimant whose holder word is `who`, as an insert that
    /// stopped there leaves it.
    fn left_in_flight(mem: &mut [u128], state: u64, who: u64) -> W8Table {
        let t = table(mem, 2);
        assert_eq!(put(&t, 1, 10, true), Ok(Placed::Inserted));
        let i = (0..2).find(|&i| state_of(split(t.read(i)).0) == EMPTY).expect("one slot is free");
        let p = t.read(i);
        let (h, _) = split(p);
        assert!(t.slot(i).compare_exchange(p, pair(head(version_of(h), state), entry(2, 20))).is_ok());
        t.claimant(i).store(who, Ordering::Release);
        t
    }

    /// An insert that died after its claim leaves its slot visible,
    /// inserting or collided; once no slot is free, the next insert frees it
    /// and goes in, and the dead insert's key reads as absent.
    #[test]
    fn a_dead_inserts_slot_is_freed_once_no_slot_is_free() {
        for state in [VISIBLE, INSERTING, COLLIDED] {
            let mut mem = memory(2);
            let t = left_in_flight(&mut mem, state, GONE);
            assert_eq!(put(&t, 3, 30, true), Ok(Placed::Inserted), "state {state}: the dead insert's slot was freed");
            assert_eq!(look(&t, 3), Some(entry(3, 30)));
            assert_eq!(look(&t, 1), Some(entry(1, 10)), "state {state}: the member stays");
            assert_eq!(look(&t, 2), None, "state {state}: the dead insert never went in");
        }
    }

    /// A slot claimed by this process, or by a claimant that has not yet
    /// written its holder word, is never judged: the table reads as full.
    #[test]
    fn a_live_or_unwritten_claimants_slot_is_left() {
        for who in [process_identity::current(), 0] {
            for state in [VISIBLE, INSERTING, COLLIDED] {
                let mut mem = memory(2);
                let t = left_in_flight(&mut mem, state, who);
                assert_eq!(put(&t, 3, 30, true), Err(Full), "state {state}, holder {who:#x}: the slot was freed");
                assert_eq!(look(&t, 1), Some(entry(1, 10)));
            }
        }
    }

    /// Keys homed at one bucket push its bound out; removing the furthest
    /// brings it back to the furthest that remains.
    #[test]
    fn removing_the_furthest_key_lowers_its_homes_bound() {
        let mut mem = memory(64);
        let t = table(&mut mem, 64);
        let home = t.home(0);
        let same: Vec<u32> = (0..u32::MAX).filter(|&k| t.home(u64::from(k)) == home).take(3).collect();
        for &k in &same {
            assert_eq!(put(&t, k, k, true), Ok(Placed::Inserted));
        }
        let furthest = (t.bound(home).load(Ordering::Acquire) >> 1) as usize;
        assert!(furthest >= 2, "three keys of one home reach offset 2 at least: {furthest}");
        let last = same
            .iter()
            .copied()
            .find(|&k| t.find_member(home, u64::from(k)).map(|(o, _, _)| o) == Some(furthest))
            .expect("a key sits at the bound");
        assert_eq!(take(&t, last), Some(entry(last, last)));
        assert!(((t.bound(home).load(Ordering::Acquire) >> 1) as usize) < furthest, "the bound came back");
        for &k in same.iter().filter(|&&k| k != last) {
            assert_eq!(look(&t, k), Some(entry(k, k)), "key {k} still found under the lowered bound");
        }
    }
}
