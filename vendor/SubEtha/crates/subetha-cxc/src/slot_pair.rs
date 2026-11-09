//! One element slot that any number of processes write and read at once:
//! two value buffers and a head naming the current one, so no read waits on
//! a writer, and a writer that dies mid-copy leaves a claim the next writer
//! takes over.
//!
//! # Layout
//!
//! A slot starts with its head word. Bit 0 names the current buffer, and
//! the bits above count the publishes that made a buffer current: the
//! slot's generation, 0 while nothing has been written, when buffer 0
//! holds the zeroed region. The claim word follows: 0 while no write holds
//! the slot, else the writer's holder word, its pid and process start stamp
//! (see [`process_identity`](crate::process_identity)). Each buffer starts
//! at the first offset past what precedes it that is a multiple of the
//! element's alignment and of 8, and is as wide as the element rounded up
//! to 8. The stride pads the slot to whole cache lines, or to the alignment
//! when that is wider, so an element of at most 56 bytes at an alignment of
//! at most 8 fills a 128-byte slot.
//!
//! # Writes
//!
//! A write takes the claim by a compare-and-swap from 0 to its holder word,
//! loads the head, copies the value into the buffer the head does not name,
//! makes that buffer current with one compare-and-swap of the head, and
//! frees the claim. Only the claim's holder publishes, so the head cannot
//! move while a write holds it. A write that finds the claim held by a live
//! writer waits while that writer copies in; one held by a writer whose
//! process is gone, or whose pid now names a process started at another
//! time, is taken over by a compare-and-swap from that exact word, so
//! exactly one writer takes it.
//!
//! # Reads
//!
//! A read loads the head, copies the buffer it names, and loads the head
//! again. A buffer is written only while it is not current and every
//! publish moves the generation on, so an unchanged head means the copy is
//! whole. A read retries only when a publish landed during it, and never
//! waits on a writer.

use std::mem::MaybeUninit;
use std::sync::atomic::{fence, AtomicU64, Ordering};

use crate::{atomic_copy, process_identity};

/// Bytes a slot spends on its head and claim words before its buffers.
pub(crate) const SLOT_PREFIX: usize = 16;

/// Where the parts of one slot lie, for an element of a given size and
/// alignment.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct SlotGeometry {
    /// Where each buffer starts, from the start of the slot.
    pub(crate) buffers: [usize; 2],
    /// Bytes from one slot to the next.
    pub(crate) stride: usize,
    /// Bytes one element takes.
    pub(crate) size: usize,
}

impl SlotGeometry {
    /// The geometry for an element of `size` bytes aligned to `alignment`,
    /// a power of two.
    pub(crate) const fn new(size: usize, alignment: usize) -> Self {
        let align = if alignment > 8 { alignment } else { 8 };
        let width = size.next_multiple_of(8);
        let first = SLOT_PREFIX.next_multiple_of(align);
        let second = (first + width).next_multiple_of(align);
        let line = if alignment > 64 { alignment } else { 64 };
        Self { buffers: [first, second], stride: (second + width).next_multiple_of(line), size }
    }
}

/// The head word of the slot at `slot`.
///
/// # Safety
/// `slot` is 8-byte aligned and starts a slot of a mapping that outlives
/// `'a`.
#[inline(always)]
unsafe fn head<'a>(slot: *const u8) -> &'a AtomicU64 {
    // SAFETY: a slot's first 8 bytes are its head, per the caller.
    unsafe { &*(slot as *const AtomicU64) }
}

/// The claim word of the slot at `slot`.
///
/// # Safety
/// As for [`head`].
#[inline(always)]
unsafe fn claim<'a>(slot: *const u8) -> &'a AtomicU64 {
    // SAFETY: a slot's second 8 bytes are its claim, per the caller.
    unsafe { &*(slot.add(8) as *const AtomicU64) }
}

/// The head that makes `buffer` current one publish after `seen`.
#[inline(always)]
fn after(seen: u64, buffer: usize) -> u64 {
    (((seen >> 1) + 1) << 1) | buffer as u64
}

/// Read the slot at `slot`: `copy` is handed the address of the buffer the
/// head names and copies from it, and the answer is the slot's generation
/// once a copy is confirmed whole. `copy` runs again whenever a publish
/// landed during it, so only its last run counts.
///
/// # Safety
/// `slot` starts a slot laid out by `geometry` in a mapping that outlives
/// the call, and `copy` reads at most the element's size rounded up to 8
/// bytes from the address it is handed.
#[inline(always)]
pub(crate) unsafe fn read(slot: *const u8, geometry: &SlotGeometry, mut copy: impl FnMut(*const u8)) -> u64 {
    // SAFETY: per the caller.
    let head = unsafe { head(slot) };
    let mut seen = head.load(Ordering::Acquire);
    loop {
        // SAFETY: bit 0 names one of this slot's two buffers.
        copy(unsafe { slot.add(geometry.buffers[(seen & 1) as usize]) });
        fence(Ordering::Acquire);
        let now = head.load(Ordering::Acquire);
        if now == seen {
            return seen >> 1;
        }
        seen = now;
    }
}

/// Write the slot at `slot`: `fill` is handed the address of the buffer
/// the head does not name, held for this write, and writes the element
/// there; the answer is the generation the write made current. `fill` runs
/// again only when the claim was taken over from this write, which takes
/// this process being judged gone.
///
/// # Safety
/// `slot` starts a writable slot laid out by `geometry` in a mapping that
/// outlives the call, and `fill` writes at most the element's size rounded
/// up to 8 bytes at the address it is handed.
pub(crate) unsafe fn write(slot: *const u8, geometry: &SlotGeometry, mut fill: impl FnMut(*mut u8)) -> u64 {
    // SAFETY: per the caller.
    let (head, claim) = unsafe { (head(slot), claim(slot)) };
    let me = process_identity::current();
    loop {
        // SAFETY: per the caller.
        let seen = unsafe { take_claim(claim, head, me) };
        let idle = ((seen & 1) ^ 1) as usize;
        // A reader that took this buffer as current before the publish that
        // retired it may still be copying it and checks the head after, so
        // the head this claim loaded is ordered before the fill's stores.
        fence(Ordering::Release);
        // SAFETY: the claim keeps every other write off this slot, and the
        // head names the other buffer.
        fill(unsafe { slot.add(geometry.buffers[idle]) as *mut u8 });
        let next = after(seen, idle);
        if head.compare_exchange(seen, next, Ordering::AcqRel, Ordering::Acquire).is_ok() {
            claim.store(0, Ordering::Release);
            return next >> 1;
        }
        // A publish this write did not make landed, so the claim was taken
        // over: it is given back only while it is still this write's.
        if claim.compare_exchange(me, 0, Ordering::Release, Ordering::Relaxed).is_err() {
            std::thread::yield_now();
        }
    }
}

/// Read the slot at `slot` as [`read`] does, answering the head that
/// confirmed the last copy whole.
///
/// # Safety
/// As for [`read`].
#[inline(always)]
pub(crate) unsafe fn read_head(slot: *const u8, geometry: &SlotGeometry, mut copy: impl FnMut(*const u8)) -> u64 {
    // SAFETY: per the caller.
    let head = unsafe { head(slot) };
    let mut seen = head.load(Ordering::Acquire);
    loop {
        // SAFETY: bit 0 names one of this slot's two buffers.
        copy(unsafe { slot.add(geometry.buffers[(seen & 1) as usize]) });
        fence(Ordering::Acquire);
        let now = head.load(Ordering::Acquire);
        if now == seen {
            return seen;
        }
        seen = now;
    }
}

/// The head word of the slot at `slot`, as it stands.
///
/// # Safety
/// As for [`head`].
#[inline(always)]
pub(crate) unsafe fn load_head(slot: *const u8) -> u64 {
    // SAFETY: per the caller.
    unsafe { head(slot) }.load(Ordering::Acquire)
}

/// Swap the head word of the slot at `slot` from `current` to `new` without
/// the claim, for a participant that changes what the head says about the
/// buffer it names and writes no buffer: `Ok`, or `Err` with the head found.
///
/// # Safety
/// As for [`head`]; `new` names the buffer `current` names.
#[inline(always)]
pub(crate) unsafe fn swap_head(slot: *const u8, current: u64, new: u64) -> Result<u64, u64> {
    debug_assert_eq!(current & 1, new & 1);
    // SAFETY: per the caller.
    unsafe { head(slot) }.compare_exchange(current, new, Ordering::AcqRel, Ordering::Acquire)
}

/// Write the slot at `slot` as [`write`](fn@write) does, the head the write makes
/// current chosen by `next` from the head found once the claim is held, a
/// head naming the idle buffer; None declines, and the claim goes back with
/// nothing made current. A head that a participant without the claim swapped
/// meanwhile, naming the same buffer, is put to `next` again, and the buffer
/// already filled stands. The answer is the head made current, or the head
/// `next` declined.
///
/// # Safety
/// As for [`write`](fn@write).
pub(crate) unsafe fn write_with(
    slot: *const u8,
    geometry: &SlotGeometry,
    mut fill: impl FnMut(*mut u8),
    mut next: impl FnMut(u64) -> Option<u64>,
) -> Result<u64, u64> {
    // SAFETY: per the caller.
    let (head, claim) = unsafe { (head(slot), claim(slot)) };
    let me = process_identity::current();
    'claim: loop {
        // SAFETY: per the caller.
        let mut seen = unsafe { take_claim(claim, head, me) };
        let Some(mut target) = next(seen) else {
            give_back(claim, me);
            return Err(seen);
        };
        debug_assert_eq!(target & 1, (seen & 1) ^ 1, "a write makes the idle buffer current");
        // Ordered as in `write`: the head this claim loaded before the fill.
        fence(Ordering::Release);
        // SAFETY: the claim keeps every other write off this slot, and the
        // head names the other buffer.
        fill(unsafe { slot.add(geometry.buffers[(target & 1) as usize]) as *mut u8 });
        loop {
            match head.compare_exchange(seen, target, Ordering::AcqRel, Ordering::Acquire) {
                Ok(_) => {
                    claim.store(0, Ordering::Release);
                    return Ok(target);
                }
                // Taken over, or a buffer made current by a write this one did
                // not see: write again from the claim.
                Err(now) if now & 1 != seen & 1 || claim.load(Ordering::Acquire) != me => {
                    give_back(claim, me);
                    continue 'claim;
                }
                Err(now) => {
                    seen = now;
                    match next(seen) {
                        Some(t) => target = t,
                        None => {
                            give_back(claim, me);
                            return Err(seen);
                        }
                    }
                }
            }
        }
    }
}

/// Free the claim `me` holds on a slot, only while it is still `me`'s.
#[inline]
fn give_back(claim: &AtomicU64, me: u64) {
    if claim.compare_exchange(me, 0, Ordering::Release, Ordering::Relaxed).is_err() {
        std::thread::yield_now();
    }
}

/// Take the slot's claim for the holder word `me`, waiting while a live
/// writer holds it and taking it over from a writer that is gone, and
/// answer the head as it stands once the claim is held.
///
/// # Safety
/// `claim` and `head` are one slot's claim and head words.
unsafe fn take_claim(claim: &AtomicU64, head: &AtomicU64, me: u64) -> u64 {
    loop {
        let held = claim.load(Ordering::Acquire);
        let taken = if held == 0 {
            claim.compare_exchange(0, me, Ordering::AcqRel, Ordering::Relaxed).is_ok()
        } else {
            process_identity::gone(process_identity::holder_pid(held), process_identity::holder_stamp(held))
                && claim.compare_exchange(held, me, Ordering::AcqRel, Ordering::Relaxed).is_ok()
        };
        if taken {
            return head.load(Ordering::Acquire);
        }
        if held != 0 {
            std::thread::yield_now();
        }
    }
}

/// The writes that landed in the slot at `slot`, doubled, plus one while a
/// write holds its claim: even at rest, odd while a writer copies in or
/// after one died holding the claim.
///
/// # Safety
/// As for [`head`].
#[inline]
pub(crate) unsafe fn version(slot: *const u8) -> u64 {
    // SAFETY: per the caller.
    let (head, claim) = unsafe { (head(slot), claim(slot)) };
    (head.load(Ordering::Acquire) & !1) | u64::from(claim.load(Ordering::Acquire) != 0)
}

/// The slot's current value as a `T`, with its generation.
///
/// # Safety
/// `slot` starts a slot laid out by `geometry` for `T` in a mapping that
/// outlives the call, and every value written to the slot is a `T`, as is
/// the zero a fresh slot holds.
#[inline(always)]
pub(crate) unsafe fn load<T: Copy>(slot: *const u8, geometry: &SlotGeometry) -> (u64, T) {
    let mut value = MaybeUninit::<T>::uninit();
    // SAFETY: each buffer is 8-byte aligned and the element rounded up to 8
    // bytes wide, which `atomic_copy::load` reads.
    let generation = unsafe { read(slot, geometry, |buffer| value = atomic_copy::load(buffer as *const T)) };
    // SAFETY: the head confirmed the last copy whole, so it holds a T a
    // writer stored, or a fresh slot's zero, per the caller.
    (generation, unsafe { value.assume_init() })
}

/// Write `value` into the slot, answering the generation it made current.
///
/// # Safety
/// `slot` starts a writable slot laid out by `geometry` for `T` in a
/// mapping that outlives the call.
#[inline]
pub(crate) unsafe fn store<T: Copy>(slot: *const u8, geometry: &SlotGeometry, value: &T) -> u64 {
    // SAFETY: each buffer is 8-byte aligned and holds a T.
    unsafe { write(slot, geometry, |buffer| atomic_copy::store(buffer as *mut T, value)) }
}

/// Copy the slot's current element, `geometry.size` bytes, into the front
/// of `out`, answering its generation.
///
/// # Safety
/// `slot` starts a slot laid out by `geometry` in a mapping that outlives
/// the call, and `out` holds at least `geometry.size` bytes.
pub(crate) unsafe fn load_bytes(slot: *const u8, geometry: &SlotGeometry, out: &mut [u8]) -> u64 {
    let size = geometry.size;
    let whole = size & !7;
    let dst = out.as_mut_ptr();
    // SAFETY: the buffer is 8-byte aligned and `size` rounded up to 8 bytes
    // wide; `out` holds `size` bytes, so the whole words land inside it and
    // the tail word is copied from a local.
    unsafe {
        read(slot, geometry, |buffer| {
            atomic_copy::load_bytes(dst, buffer, whole);
            if whole < size {
                let tail = atomic_copy::load_word(buffer.add(whole) as *const u64).to_ne_bytes();
                std::ptr::copy_nonoverlapping(tail.as_ptr(), dst.add(whole), size - whole);
            }
        })
    }
}

/// Write the element at `value`, `geometry.size` bytes, into the slot,
/// answering the generation it made current.
///
/// # Safety
/// `slot` starts a writable slot laid out by `geometry` in a mapping that
/// outlives the call, and `value` holds at least `geometry.size` bytes.
pub(crate) unsafe fn store_bytes(slot: *const u8, geometry: &SlotGeometry, value: &[u8]) -> u64 {
    let size = geometry.size;
    let src = value.as_ptr();
    // SAFETY: the buffer is 8-byte aligned and holds `size` bytes, and
    // `value` holds `size` readable bytes no one else touches.
    unsafe { write(slot, geometry, |buffer| atomic_copy::store_bytes(buffer, src, size)) }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicBool;

    /// Memory for `slots` slots of `geometry`, 128-byte aligned, zeroed.
    fn region(geometry: &SlotGeometry, slots: usize) -> Vec<u128> {
        vec![0u128; (geometry.stride * slots).div_ceil(16) + 8]
    }

    /// The first slot in `memory`, aligned to 64.
    fn slot0(memory: &[u128]) -> *const u8 {
        let base = memory.as_ptr() as *const u8;
        // SAFETY: `region` leaves room for the alignment step.
        unsafe { base.add(base.align_offset(64)) }
    }

    #[test]
    fn the_geometry_places_both_buffers_and_pads_to_lines() {
        assert_eq!(SlotGeometry::new(56, 8), SlotGeometry { buffers: [16, 72], stride: 128, size: 56 });
        assert_eq!(SlotGeometry::new(52, 4), SlotGeometry { buffers: [16, 72], stride: 128, size: 52 });
        assert_eq!(SlotGeometry::new(24, 8), SlotGeometry { buffers: [16, 40], stride: 64, size: 24 });
        assert_eq!(SlotGeometry::new(1, 1), SlotGeometry { buffers: [16, 24], stride: 64, size: 1 });
        assert_eq!(SlotGeometry::new(100, 8), SlotGeometry { buffers: [16, 120], stride: 256, size: 100 });
        assert_eq!(SlotGeometry::new(24, 32), SlotGeometry { buffers: [32, 64], stride: 128, size: 24 });
        assert_eq!(SlotGeometry::new(24, 128), SlotGeometry { buffers: [128, 256], stride: 384, size: 24 });
    }

    #[test]
    fn a_fresh_slot_reads_zero_at_generation_zero_and_each_write_steps_it() {
        let g = SlotGeometry::new(24, 8);
        let memory = region(&g, 1);
        let slot = slot0(&memory);
        // SAFETY: the slot lies in `memory`, laid out by `g` for [u64; 3].
        unsafe {
            assert_eq!(load::<[u64; 3]>(slot, &g), (0, [0; 3]));
            assert_eq!(store(slot, &g, &[1u64, 2, 3]), 1);
            assert_eq!(load::<[u64; 3]>(slot, &g), (1, [1, 2, 3]));
            assert_eq!(store(slot, &g, &[4u64, 5, 6]), 2);
            assert_eq!(load::<[u64; 3]>(slot, &g), (2, [4, 5, 6]));
        }
    }

    /// An element whose size is no multiple of 8 round trips exactly, and
    /// a copy out writes no byte past it.
    #[test]
    fn a_ragged_element_round_trips_exactly() {
        for size in [1usize, 7, 13, 52, 55, 56] {
            let g = SlotGeometry::new(size, 1);
            let memory = region(&g, 1);
            let slot = slot0(&memory);
            let value: Vec<u8> = (0..size as u8).map(|b| b.wrapping_mul(37).wrapping_add(1)).collect();
            let mut out = vec![0xEEu8; size + 8];
            // SAFETY: the slot lies in `memory`, laid out by `g`.
            unsafe {
                assert_eq!(store_bytes(slot, &g, &value), 1);
                assert_eq!(load_bytes(slot, &g, &mut out), 1);
            }
            assert_eq!(&out[..size], &value[..], "size {size}");
            assert!(out[size..].iter().all(|&b| b == 0xEE), "size {size}: a byte past the element was written");
        }
    }

    /// Writers racing on one slot, each writing a value whose seven words
    /// are equal, and readers checking every value they read: a torn copy
    /// would mix two writers' words.
    #[test]
    fn racing_writers_and_readers_never_see_a_torn_value() {
        let g = SlotGeometry::new(56, 8);
        let memory = region(&g, 1);
        let slot = slot0(&memory) as usize;
        let done = AtomicBool::new(false);
        std::thread::scope(|s| {
            let writers: Vec<_> = (0..4u64)
                .map(|w| {
                    s.spawn(move || {
                        for round in 0..20_000u64 {
                            let word = (w << 32) | round;
                            // SAFETY: the slot lies in `memory` for [u64; 7].
                            unsafe { store(slot as *const u8, &g, &[word; 7]) };
                        }
                    })
                })
                .collect();
            for _ in 0..2 {
                s.spawn(|| {
                    while !done.load(Ordering::Acquire) {
                        // SAFETY: as for the writers.
                        let (_, value) = unsafe { load::<[u64; 7]>(slot as *const u8, &g) };
                        assert!(value.iter().all(|&w| w == value[0]), "a torn value: {value:x?}");
                    }
                });
            }
            for w in writers {
                w.join().unwrap();
            }
            done.store(true, Ordering::Release);
        });
        // SAFETY: as above.
        assert_eq!(unsafe { load::<[u64; 7]>(slot as *const u8, &g) }.0, 80_000, "every write landed once");
    }

    /// A claim left by a writer whose process exited is taken over, as is
    /// one whose pid now names a process that started at another time.
    #[test]
    fn a_claim_left_by_a_gone_writer_is_taken_over() {
        let g = SlotGeometry::new(8, 8);
        let memory = region(&g, 1);
        let slot = slot0(&memory);
        let me = process_identity::current();
        let other_start = process_identity::holder_stamp(me).wrapping_add(1).max(1);
        for left in [process_identity::holder(0, 0), process_identity::holder(process_identity::holder_pid(me), other_start)] {
            // SAFETY: the slot lies in `memory`.
            unsafe {
                claim(slot).store(left, Ordering::Release);
                let before = load::<u64>(slot, &g).0;
                assert_eq!(store(slot, &g, &7u64), before + 1, "the write took over {left:#x}");
                assert_eq!(claim(slot).load(Ordering::Acquire), 0, "the claim was freed");
            }
        }
    }

    /// A claim held by a live writer keeps a write waiting, and the write
    /// lands once the claim is freed; a read meanwhile answers at once.
    #[test]
    fn a_write_waits_on_a_live_claim_and_a_read_does_not() {
        let g = SlotGeometry::new(8, 8);
        let memory = region(&g, 1);
        let slot = slot0(&memory) as usize;
        // SAFETY: the slot lies in `memory`.
        unsafe {
            store(slot as *const u8, &g, &1u64);
            claim(slot as *const u8).store(process_identity::current(), Ordering::Release);
        }
        let landed = AtomicBool::new(false);
        std::thread::scope(|s| {
            s.spawn(|| {
                // SAFETY: as above.
                unsafe { store(slot as *const u8, &g, &2u64) };
                landed.store(true, Ordering::Release);
            });
            std::thread::sleep(std::time::Duration::from_millis(50));
            assert!(!landed.load(Ordering::Acquire), "a write landed while a live writer held the claim");
            // SAFETY: as above.
            assert_eq!(unsafe { load::<u64>(slot as *const u8, &g) }, (1, 1), "a read answers while the claim is held");
            // SAFETY: as above.
            unsafe { claim(slot as *const u8).store(0, Ordering::Release) };
        });
        assert!(landed.load(Ordering::Acquire));
        // SAFETY: as above.
        assert_eq!(unsafe { load::<u64>(slot as *const u8, &g) }, (2, 2));
    }
}
