//! `AtomicPair`: two 64-bit halves in shared memory, read, written and
//! swapped as one aligned 16-byte word, for a link that carries a slot and
//! two generations.
//!
//! Which instructions run is found once per process:
//!
//! - x86-64 swaps with `lock cmpxchg16b`. Where the processor reports AVX it
//!   reads and writes with `vmovdqa`: Intel and AMD guarantee an aligned
//!   16-byte `movdqa` to be atomic on every processor that sets
//!   CPUID.01H:ECX.AVX. Elsewhere a read is a `lock cmpxchg16b` of zero for
//!   zero, which writes back only the zero it read, and a write is a swap
//!   loop.
//! - aarch64 swaps with `caspal` where the processor has LSE, and with an
//!   exclusive-pair loop elsewhere. Where it has LSE2, which makes an aligned
//!   `ldp` or `stp` of two registers single-copy atomic, it reads and writes
//!   with those; elsewhere with exclusive pairs, each read confirmed by
//!   writing back what it read.
//!
//! An x86-64 processor without `cmpxchg16b`, and any other architecture,
//! has no lock-free 16-byte swap: [`supported`](crate::atomic_pair::supported) answers false there, and a
//! structure built on pairs refuses to open.
//!
//! Either half can also be read or written alone, and the high half swapped
//! alone, as one aligned 8-byte word, which every processor with a 16-byte
//! swap reads, writes and swaps whole.
//!
//! A read of the whole word orders as Acquire, a write as Release, a swap as
//! both. A read or a write of one half orders as Relaxed; a swap of the high
//! half as both.

use std::cell::UnsafeCell;
use std::sync::atomic::{AtomicU8, AtomicU64, Ordering};

/// Two 64-bit halves, the low half at the lower address, accessed only as
/// one 16-byte word.
#[repr(C, align(16))]
pub(crate) struct AtomicPair(UnsafeCell<u128>);

// SAFETY: every access to the word goes through the 16-byte atomic forms
// below, or reads a half, or swaps the high half, as one atomic 8-byte word.
unsafe impl Sync for AtomicPair {}
// SAFETY: as for Sync.
unsafe impl Send for AtomicPair {}

/// What the processor offers, found once: zero until found.
static FORMS: AtomicU8 = AtomicU8::new(0);
const FOUND: u8 = 1;
/// A 16-byte swap exists.
const SWAP: u8 = 2;
/// The swap is one instruction: `caspal` on aarch64; x86-64 always has it
/// when it has a swap at all.
const SWAP_ONE: u8 = 4;
/// Plain 16-byte reads and writes are atomic.
const PLAIN: u8 = 8;

#[cfg(target_arch = "x86_64")]
fn detect() -> u8 {
    let mut forms = FOUND;
    if std::arch::is_x86_feature_detected!("cmpxchg16b") {
        forms |= SWAP | SWAP_ONE;
        if std::arch::is_x86_feature_detected!("avx") {
            forms |= PLAIN;
        }
    }
    forms
}

#[cfg(target_arch = "aarch64")]
fn detect() -> u8 {
    let mut forms = FOUND | SWAP;
    if std::arch::is_aarch64_feature_detected!("lse") {
        forms |= SWAP_ONE;
    }
    if std::arch::is_aarch64_feature_detected!("lse2") {
        forms |= PLAIN;
    }
    forms
}

#[cfg(not(any(target_arch = "x86_64", target_arch = "aarch64")))]
fn detect() -> u8 {
    FOUND
}

#[inline]
fn forms() -> u8 {
    let forms = FORMS.load(Ordering::Relaxed);
    if forms != 0 {
        return forms;
    }
    let found = detect();
    FORMS.store(found, Ordering::Relaxed);
    found
}

/// Whether this processor swaps 16 bytes without a lock.
pub(crate) fn supported() -> bool {
    forms() & SWAP != 0
}

/// Whether one plain load or store of 16 aligned bytes is single-copy
/// atomic here: AVX on x86-64, LSE2 on aarch64.
#[inline]
pub(crate) fn plain_sixteen() -> bool {
    forms() & PLAIN != 0
}

impl AtomicPair {
    /// A pair holding `v`, for a static or another owner of its own.
    pub(crate) const fn new(v: u128) -> Self {
        Self(UnsafeCell::new(v))
    }

    /// The pair at `at`.
    ///
    /// # Safety
    /// `at` is 16-byte aligned and addresses 16 bytes, readable and writable
    /// for `'a`, that are accessed only as an `AtomicPair`, and [`supported`]
    /// answered true.
    #[inline]
    pub(crate) unsafe fn at<'a>(at: *mut u8) -> &'a Self {
        debug_assert!(at.addr().is_multiple_of(16));
        // SAFETY: the caller's contract.
        unsafe { &*(at as *const Self) }
    }

    #[inline]
    fn ptr(&self) -> *mut u128 {
        self.0.get()
    }

    /// The word, read whole. Reads the forms [`supported`] found, which
    /// [`at`](Self::at) requires to have been asked; forms not yet found
    /// read as no plain reads, and the read is a swap.
    #[inline]
    pub(crate) fn load(&self) -> u128 {
        self.load_in(FORMS.load(Ordering::Relaxed))
    }

    /// The word, read whole by the instructions `forms` names: the
    /// processor's own, or in the tests a subset of them.
    #[inline]
    fn load_in(&self, forms: u8) -> u128 {
        // SAFETY: the pair is aligned and live, and a plain read is taken
        // only when the forms say the processor has one.
        unsafe {
            if forms & PLAIN != 0 {
                imp::load_plain(self.ptr())
            } else {
                imp::load_swap(self.ptr(), swap_forms())
            }
        }
    }

    /// The low half alone, one aligned 8-byte read, ordered as Relaxed.
    #[inline]
    pub(crate) fn load_lo(&self) -> u64 {
        // SAFETY: the pair is aligned and live, so its low half is an aligned
        // eight-byte word, which every processor with a 16-byte swap reads
        // whole.
        unsafe { (*(self.ptr() as *const AtomicU64)).load(Ordering::Relaxed) }
    }

    /// The high half alone, as for [`load_lo`](Self::load_lo).
    #[inline]
    pub(crate) fn load_hi(&self) -> u64 {
        // SAFETY: as for `load_lo`, eight bytes on.
        unsafe { (*(self.ptr() as *const AtomicU64).add(1)).load(Ordering::Relaxed) }
    }

    /// Replace the high half with `new` if it is `current`, the low half left
    /// as it stands: `Ok` with the half swapped out, or `Err` with the half
    /// found. Ordered as both Acquire and Release.
    #[inline]
    pub(crate) fn compare_exchange_hi(&self, current: u64, new: u64) -> Result<u64, u64> {
        // SAFETY: as for `load_hi`; every processor with a 16-byte swap swaps an
        // aligned eight-byte word whole.
        unsafe { (*(self.ptr() as *const AtomicU64).add(1)).compare_exchange(current, new, Ordering::AcqRel, Ordering::Acquire) }
    }

    /// Replace the low half with `new` if it is `current`, the high half left
    /// as it stands, as for [`compare_exchange_hi`](Self::compare_exchange_hi).
    #[inline]
    pub(crate) fn compare_exchange_lo(&self, current: u64, new: u64) -> Result<u64, u64> {
        // SAFETY: as for `load_lo`; every processor with a 16-byte swap swaps an
        // aligned eight-byte word whole.
        unsafe { (*(self.ptr() as *const AtomicU64)).compare_exchange(current, new, Ordering::AcqRel, Ordering::Acquire) }
    }

    /// Add `v` to the low half, the high half left as it stands, wrapping:
    /// the half found before the add. Ordered as both Acquire and Release.
    #[inline]
    pub(crate) fn fetch_add_lo(&self, v: u64) -> u64 {
        // SAFETY: as for `compare_exchange_lo`.
        unsafe { (*(self.ptr() as *const AtomicU64)).fetch_add(v, Ordering::AcqRel) }
    }

    /// Write the low half alone, one aligned 8-byte write, ordered as
    /// Relaxed.
    #[inline]
    pub(crate) fn store_lo(&self, v: u64) {
        // SAFETY: as for `load_lo`; every processor with a 16-byte swap writes an
        // aligned eight-byte word whole.
        unsafe { (*(self.ptr() as *const AtomicU64)).store(v, Ordering::Relaxed) }
    }

    /// Write the high half alone, as for [`store_lo`](Self::store_lo).
    #[inline]
    pub(crate) fn store_hi(&self, v: u64) {
        // SAFETY: as for `store_lo`, eight bytes on.
        unsafe { (*(self.ptr() as *const AtomicU64).add(1)).store(v, Ordering::Relaxed) }
    }

    /// Write `v` whole.
    #[inline]
    pub(crate) fn store(&self, v: u128) {
        self.store_in(v, forms());
    }

    /// Write `v` whole by the instructions `forms` names, as for
    /// [`load_in`](Self::load_in).
    #[inline]
    fn store_in(&self, v: u128, forms: u8) {
        // SAFETY: as for `load`.
        unsafe {
            if forms & PLAIN != 0 {
                imp::store_plain(self.ptr(), v);
            } else {
                let mut seen = imp::load_swap(self.ptr(), forms);
                loop {
                    let (was, swapped) = imp::swap(self.ptr(), seen, v, forms);
                    if swapped {
                        return;
                    }
                    seen = was;
                }
            }
        }
    }

    /// Replace `current` with `new` if the word is `current`: `Ok` with the
    /// word swapped out, or `Err` with the word found instead.
    #[inline]
    pub(crate) fn compare_exchange(&self, current: u128, new: u128) -> Result<u128, u128> {
        // SAFETY: as for `load`.
        let (was, swapped) = unsafe { imp::swap(self.ptr(), current, new, swap_forms()) };
        if swapped { Ok(was) } else { Err(was) }
    }
}

/// The forms a swap chooses its instruction by: on x86-64 the one swap every
/// processor with a swap has, read from nothing; elsewhere those found.
#[inline]
fn swap_forms() -> u8 {
    #[cfg(target_arch = "x86_64")]
    {
        FOUND | SWAP | SWAP_ONE
    }
    #[cfg(not(target_arch = "x86_64"))]
    {
        forms()
    }
}

#[cfg(target_arch = "x86_64")]
mod imp {
    use core::arch::asm;
    use core::arch::x86_64::__m128i;

    use super::SWAP_ONE;

    /// # Safety
    /// `src` is 16-byte aligned and readable, and the processor has AVX.
    #[inline]
    pub(super) unsafe fn load_plain(src: *mut u128) -> u128 {
        let v: __m128i;
        // SAFETY: the caller's contract; `vmovdqa` of an aligned 16-byte word
        // is atomic on a processor with AVX.
        unsafe {
            asm!(
                "vmovdqa {v}, xmmword ptr [{src}]",
                src = in(reg) src,
                v = out(xmm_reg) v,
                options(nostack, preserves_flags, readonly),
            );
            core::mem::transmute::<__m128i, u128>(v)
        }
    }

    /// # Safety
    /// `dst` is 16-byte aligned and writable, and the processor has AVX.
    #[inline]
    pub(super) unsafe fn store_plain(dst: *mut u128, v: u128) {
        // SAFETY: the two types are sixteen bytes each.
        let v = unsafe { core::mem::transmute::<u128, __m128i>(v) };
        // SAFETY: the caller's contract, as for `load_plain`.
        unsafe {
            asm!(
                "vmovdqa xmmword ptr [{dst}], {v}",
                dst = in(reg) dst,
                v = in(xmm_reg) v,
                options(nostack, preserves_flags),
            );
        }
    }

    /// The word, read by a swap of zero for zero.
    ///
    /// # Safety
    /// `src` is 16-byte aligned, readable and writable, and the processor
    /// has `cmpxchg16b`.
    #[inline]
    pub(super) unsafe fn load_swap(src: *mut u128, forms: u8) -> u128 {
        // SAFETY: the caller's contract.
        unsafe { swap(src, 0, 0, forms).0 }
    }

    /// `lock cmpxchg16b`: the word found, and whether it was `current` and so
    /// replaced by `new`.
    ///
    /// # Safety
    /// `dst` is 16-byte aligned, readable and writable, and the processor
    /// has `cmpxchg16b`.
    #[inline]
    pub(super) unsafe fn swap(dst: *mut u128, current: u128, new: u128, forms: u8) -> (u128, bool) {
        debug_assert!(forms & SWAP_ONE != 0);
        let mut lo = current as u64;
        let mut hi = (current >> 64) as u64;
        // SAFETY: the caller's contract. The instruction takes the new low half
        // in rbx, which cannot be named as an operand and which a register-class
        // operand may still be given, so every operand is a named register: the
        // low half goes into rbx through r8, and rbx comes back from r8 after.
        // The word handed back in rdx:rax is `current` exactly when the swap
        // took place.
        unsafe {
            asm!(
                "xchg r8, rbx",
                "lock cmpxchg16b xmmword ptr [rsi]",
                "mov rbx, r8",
                in("rsi") dst,
                inout("r8") new as u64 => _,
                inout("rax") lo,
                inout("rdx") hi,
                in("rcx") (new >> 64) as u64,
                options(nostack),
            );
        }
        let was = (u128::from(hi) << 64) | u128::from(lo);
        (was, was == current)
    }
}

#[cfg(target_arch = "aarch64")]
mod imp {
    use core::arch::asm;

    use super::SWAP_ONE;

    fn halves(v: u128) -> (u64, u64) {
        (v as u64, (v >> 64) as u64)
    }

    fn joined(lo: u64, hi: u64) -> u128 {
        (u128::from(hi) << 64) | u128::from(lo)
    }

    /// # Safety
    /// `src` is 16-byte aligned and readable, and the processor has LSE2.
    #[inline]
    pub(super) unsafe fn load_plain(src: *mut u128) -> u128 {
        let (lo, hi): (u64, u64);
        // SAFETY: the caller's contract; an aligned `ldp` of two registers is
        // single-copy atomic with LSE2, and the barrier after makes it Acquire.
        unsafe {
            asm!(
                "ldp {lo}, {hi}, [{src}]",
                "dmb ishld",
                src = in(reg) src,
                lo = out(reg) lo,
                hi = out(reg) hi,
                options(nostack, preserves_flags),
            );
        }
        joined(lo, hi)
    }

    /// # Safety
    /// `dst` is 16-byte aligned and writable, and the processor has LSE2.
    #[inline]
    pub(super) unsafe fn store_plain(dst: *mut u128, v: u128) {
        let (lo, hi) = halves(v);
        // SAFETY: as for `load_plain`; the barrier before makes it Release.
        unsafe {
            asm!(
                "dmb ish",
                "stp {lo}, {hi}, [{dst}]",
                dst = in(reg) dst,
                lo = in(reg) lo,
                hi = in(reg) hi,
                options(nostack, preserves_flags),
            );
        }
    }

    /// The word, read by an exclusive pair confirmed by writing back what it
    /// read.
    ///
    /// # Safety
    /// `src` is 16-byte aligned, readable and writable.
    #[inline]
    pub(super) unsafe fn load_swap(src: *mut u128, _forms: u8) -> u128 {
        let (lo, hi): (u64, u64);
        // SAFETY: the caller's contract.
        unsafe {
            asm!(
                "2:",
                "ldaxp {lo}, {hi}, [{src}]",
                "stxp {status:w}, {lo}, {hi}, [{src}]",
                "cbnz {status:w}, 2b",
                src = in(reg) src,
                lo = out(reg) lo,
                hi = out(reg) hi,
                status = out(reg) _,
                options(nostack, preserves_flags),
            );
        }
        joined(lo, hi)
    }

    /// `caspal` where the processor has LSE, else an exclusive-pair loop: the
    /// word found, and whether it was `current` and so replaced by `new`.
    ///
    /// # Safety
    /// `dst` is 16-byte aligned, readable and writable.
    #[inline]
    pub(super) unsafe fn swap(dst: *mut u128, current: u128, new: u128, forms: u8) -> (u128, bool) {
        let was = if forms & SWAP_ONE != 0 {
            // SAFETY: the caller's contract, and the forms say LSE is present.
            unsafe { swap_lse(dst, current, new) }
        } else {
            // SAFETY: the caller's contract.
            unsafe { swap_exclusive(dst, current, new) }
        };
        (was, was == current)
    }

    /// # Safety
    /// As for [`swap`], on a processor with LSE.
    #[target_feature(enable = "lse")]
    unsafe fn swap_lse(dst: *mut u128, current: u128, new: u128) -> u128 {
        let (mut lo, mut hi) = halves(current);
        let (new_lo, new_hi) = halves(new);
        // SAFETY: the caller's contract. `caspal` takes even-odd register pairs,
        // the first of each pair the half at the lower address.
        unsafe {
            asm!(
                "caspal x0, x1, x2, x3, [{dst}]",
                dst = in(reg) dst,
                inout("x0") lo,
                inout("x1") hi,
                in("x2") new_lo,
                in("x3") new_hi,
                options(nostack, preserves_flags),
            );
        }
        joined(lo, hi)
    }

    /// # Safety
    /// As for [`swap`].
    unsafe fn swap_exclusive(dst: *mut u128, current: u128, new: u128) -> u128 {
        let (current_lo, current_hi) = halves(current);
        let (new_lo, new_hi) = halves(new);
        let (lo, hi): (u64, u64);
        // SAFETY: the caller's contract. A pair that differs from `current` is
        // written back as read, so the read is confirmed single-copy atomic.
        unsafe {
            asm!(
                "2:",
                "ldaxp {lo}, {hi}, [{dst}]",
                "cmp {lo}, {current_lo}",
                "ccmp {hi}, {current_hi}, #0, eq",
                "b.ne 3f",
                "stlxp {status:w}, {new_lo}, {new_hi}, [{dst}]",
                "cbnz {status:w}, 2b",
                "b 4f",
                "3:",
                "stlxp {status:w}, {lo}, {hi}, [{dst}]",
                "cbnz {status:w}, 2b",
                "4:",
                dst = in(reg) dst,
                current_lo = in(reg) current_lo,
                current_hi = in(reg) current_hi,
                new_lo = in(reg) new_lo,
                new_hi = in(reg) new_hi,
                lo = out(reg) lo,
                hi = out(reg) hi,
                status = out(reg) _,
                options(nostack),
            );
        }
        joined(lo, hi)
    }
}

#[cfg(not(any(target_arch = "x86_64", target_arch = "aarch64")))]
mod imp {
    /// # Safety
    /// Never called: [`super::supported`] answers false here.
    pub(super) unsafe fn load_plain(_src: *mut u128) -> u128 {
        unreachable!("no lock-free 16-byte word on this architecture")
    }

    /// # Safety
    /// As for [`load_plain`].
    pub(super) unsafe fn store_plain(_dst: *mut u128, _v: u128) {
        unreachable!("no lock-free 16-byte word on this architecture")
    }

    /// # Safety
    /// As for [`load_plain`].
    pub(super) unsafe fn load_swap(_src: *mut u128, _forms: u8) -> u128 {
        unreachable!("no lock-free 16-byte word on this architecture")
    }

    /// # Safety
    /// As for [`load_plain`].
    pub(super) unsafe fn swap(_dst: *mut u128, _current: u128, _new: u128, _forms: u8) -> (u128, bool) {
        unreachable!("no lock-free 16-byte word on this architecture")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    /// Run `f` once on the forms this processor has, and once on its swap
    /// alone, with plain reads and writes switched off, so both paths a
    /// host can take are exercised on this one. `f` reads and writes by the
    /// forms it is handed, which no other test sees.
    fn on_each_form(f: impl Fn(u8)) {
        let found = detect();
        assert!(found & SWAP != 0, "the processors this crate gates on all swap 16 bytes");
        for forms in [found, found & !PLAIN] {
            f(forms);
        }
    }

    fn pair(v: u128) -> Box<AtomicPair> {
        Box::new(AtomicPair(UnsafeCell::new(v)))
    }

    #[test]
    fn a_swap_lands_only_on_the_word_it_expected() {
        on_each_form(|forms| {
            let p = pair(1 << 64 | 2);
            assert_eq!(p.compare_exchange(5, 7), Err(1 << 64 | 2), "a wrong guess changes nothing");
            assert_eq!(p.load_in(forms), 1 << 64 | 2);
            assert_eq!(p.compare_exchange(1 << 64 | 2, 3 << 64 | 4), Ok(1 << 64 | 2));
            assert_eq!(p.load_in(forms), 3 << 64 | 4);
            // A guess right in one half only is wrong.
            assert_eq!(p.compare_exchange(9 << 64 | 4, 0), Err(3 << 64 | 4));
            assert_eq!(p.compare_exchange(3 << 64 | 9, 0), Err(3 << 64 | 4));
            p.store_in(u128::MAX, forms);
            assert_eq!(p.load_in(forms), u128::MAX);
            assert_eq!(p.compare_exchange(u128::MAX, 0), Ok(u128::MAX));
            assert_eq!(p.load_in(forms), 0);
        });
    }

    #[test]
    fn readers_never_see_the_halves_of_two_writes() {
        on_each_form(|forms| {
            // Every word written has equal halves, so a read with unequal
            // halves joined two writes.
            let p = Arc::new(pair(0));
            let stop = Arc::new(std::sync::atomic::AtomicBool::new(false));
            std::thread::scope(|s| {
                for w in 0..2u64 {
                    let (p, stop) = (Arc::clone(&p), Arc::clone(&stop));
                    s.spawn(move || {
                        let mut i = w;
                        while !stop.load(Ordering::Relaxed) {
                            i = i.wrapping_add(2);
                            let next = u128::from(i) << 64 | u128::from(i);
                            if i % 3 == 0 {
                                p.store_in(next, forms);
                            } else {
                                // A swap may lose to the other writer; the word
                                // it hands back is whole either way.
                                let seen = p.load_in(forms);
                                match p.compare_exchange(seen, next) {
                                    Ok(was) => assert_eq!(was, seen),
                                    Err(now) => assert_eq!(now as u64, (now >> 64) as u64, "a swap found two writes joined: {now:#x}"),
                                }
                            }
                        }
                    });
                }
                for _ in 0..2 {
                    let (p, stop) = (Arc::clone(&p), Arc::clone(&stop));
                    s.spawn(move || {
                        for _ in 0..200_000 {
                            let v = p.load_in(forms);
                            assert_eq!(v as u64, (v >> 64) as u64, "a read joined two writes: {v:#x}");
                        }
                        stop.store(true, Ordering::Relaxed);
                    });
                }
            });
        });
    }

    #[test]
    fn racing_swaps_each_land_once() {
        on_each_form(|forms| {
            // Eight threads each add one to both halves a thousand times by
            // swap; every add lands exactly once.
            let p = Arc::new(pair(0));
            std::thread::scope(|s| {
                for _ in 0..8 {
                    let p = Arc::clone(&p);
                    s.spawn(move || {
                        for _ in 0..1000 {
                            let mut seen = p.load_in(forms);
                            loop {
                                let next = (((seen >> 64) + 1) << 64) | u128::from((seen as u64) + 1);
                                match p.compare_exchange(seen, next) {
                                    Ok(_) => break,
                                    Err(now) => seen = now,
                                }
                            }
                        }
                    });
                }
            });
            assert_eq!(p.load_in(forms), 8000 << 64 | 8000);
        });
    }
}
