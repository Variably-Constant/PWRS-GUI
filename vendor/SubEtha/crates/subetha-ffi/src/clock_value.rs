//! The two clocks through the C ABI, as values the caller holds. A hybrid
//! logical clock is the `subetha_hlc` the fence clock reads and writes, so
//! a fence clock's tick can be advanced, merged and compared here
//! directly; a causal clock is sixteen counts in a `subetha_causal_clock`,
//! one per participant, the clock Python and PowerShell call
//! `CausalClock`. As with the Bloom filters there is no handle and nothing
//! shared: these entry points compute on the caller's own values, none
//! takes a mode, and `subetha_init` is not required. A step from a count
//! already at `UINT64_MAX` is refused with `SUBETHA_E_INVALID_ARGUMENT`
//! and changes nothing, since no clock orders after it.

use std::cmp::Ordering;

use subetha_pointers::versioned_pointer::{HybridLogicalClock, VectorClock};

use crate::error::{fail, SUBETHA_E_INVALID_ARGUMENT, SUBETHA_OK};
use crate::fence_clock::subetha_hlc;
use crate::runtime::entry;

/// Participants a causal clock counts for, numbered from zero.
pub const SUBETHA_CAUSAL_CLOCK_NODES: usize = 16;

/// From `subetha_causal_clock_compare`: the first clock happened before
/// the second.
pub const SUBETHA_CAUSAL_BEFORE: i32 = -1;
/// From `subetha_causal_clock_compare`: every count matches.
pub const SUBETHA_CAUSAL_EQUAL: i32 = 0;
/// From `subetha_causal_clock_compare`: the first clock happened after the
/// second.
pub const SUBETHA_CAUSAL_AFTER: i32 = 1;
/// From `subetha_causal_clock_compare`: neither clock happened before the
/// other, so the two events are concurrent.
pub const SUBETHA_CAUSAL_CONCURRENT: i32 = 2;

/// One count per participant, which answers whether one event caused
/// another or whether the two happened independently. All zero is the
/// clock before anything has happened.
#[repr(C)]
#[allow(non_camel_case_types)]
#[derive(Clone, Copy, Default)]
pub struct subetha_causal_clock {
    /// Every count, in participant order.
    pub counts: [u64; SUBETHA_CAUSAL_CLOCK_NODES],
}

fn hybrid(clock: subetha_hlc) -> HybridLogicalClock {
    HybridLogicalClock::new(clock.physical_us, clock.logical)
}

fn hlc_of(clock: HybridLogicalClock) -> subetha_hlc {
    subetha_hlc { physical_us: clock.physical, logical: clock.logical }
}

fn vector(clock: subetha_causal_clock) -> VectorClock<SUBETHA_CAUSAL_CLOCK_NODES> {
    VectorClock { clock: clock.counts }
}

/// Write a stepped reading into `out`, or refuse a step from the ceiling.
///
/// # Safety
/// `out` is non-null and writable, as the caller checked.
unsafe fn write_step(out: *mut subetha_hlc, stepped: Option<HybridLogicalClock>, refusal: &str) -> i32 {
    match stepped {
        Some(clock) => {
            // SAFETY: the caller checked `out` non-null; it is writable.
            unsafe { *out = hlc_of(clock) };
            SUBETHA_OK
        }
        None => fail(SUBETHA_E_INVALID_ARGUMENT, refusal),
    }
}

/// The wall clock now, in microseconds since the epoch, with the logical
/// part at zero, into `out`.
///
/// # Safety
/// `out` is a valid pointer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn subetha_hlc_now(out: *mut subetha_hlc) -> i32 {
    entry(|| {
        if out.is_null() {
            return fail(SUBETHA_E_INVALID_ARGUMENT, "out is null");
        }
        // SAFETY: checked non-null; the caller guarantees it is writable.
        unsafe { *out = hlc_of(HybridLogicalClock::now()) };
        SUBETHA_OK
    })
}

/// The reading after `clock` at the physical time `physical_us`, into
/// `out`: `physical_us` with the logical part at zero when it is later
/// than `clock`'s, otherwise `clock` with its logical part stepped, so the
/// new reading orders after the old one either way. This is how an event
/// is recorded. `SUBETHA_E_INVALID_ARGUMENT` when the logical part would
/// step from `UINT64_MAX`.
///
/// # Safety
/// `out` is a valid pointer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn subetha_hlc_advance(clock: subetha_hlc, physical_us: u64, out: *mut subetha_hlc) -> i32 {
    entry(|| {
        if out.is_null() {
            return fail(SUBETHA_E_INVALID_ARGUMENT, "out is null");
        }
        unsafe {
            write_step(
                out,
                hybrid(clock).checked_advance(physical_us),
                "the logical part is at UINT64_MAX, so no reading orders after it",
            )
        }
    })
}

/// The reading a receiver takes, into `out`, from its own clock `local`,
/// the `received` one that traveled beside a message, and its physical
/// time `physical_us`: the latest physical part of the three, with the
/// logical part stepped past every clock that shares it, so the received
/// event orders before anything the receiver does next.
/// `SUBETHA_E_INVALID_ARGUMENT` when the logical part the merge steps is
/// already `UINT64_MAX`, which a received clock can carry.
///
/// # Safety
/// `out` is a valid pointer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn subetha_hlc_merge(
    local: subetha_hlc,
    received: subetha_hlc,
    physical_us: u64,
    out: *mut subetha_hlc,
) -> i32 {
    entry(|| {
        if out.is_null() {
            return fail(SUBETHA_E_INVALID_ARGUMENT, "out is null");
        }
        unsafe {
            write_step(
                out,
                hybrid(local).checked_merge(&hybrid(received), physical_us),
                "the logical part the merge steps is at UINT64_MAX, so no reading orders after it",
            )
        }
    })
}

/// How `a` stands to `b`, into `out`: -1 when `a` reads before `b`, 0
/// when the two are the same, 1 when after. Physical parts compare first,
/// and the logical part orders two readings that share one.
///
/// # Safety
/// `out` is a valid pointer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn subetha_hlc_compare(a: subetha_hlc, b: subetha_hlc, out: *mut i32) -> i32 {
    entry(|| {
        if out.is_null() {
            return fail(SUBETHA_E_INVALID_ARGUMENT, "out is null");
        }
        let order = match hybrid(a).cmp(&hybrid(b)) {
            Ordering::Less => -1,
            Ordering::Equal => 0,
            Ordering::Greater => 1,
        };
        // SAFETY: checked non-null; the caller guarantees it is writable.
        unsafe { *out = order };
        SUBETHA_OK
    })
}

/// Step participant `node`'s count in `*clock`, which is what a
/// participant does when something happens to it.
/// `SUBETHA_E_INVALID_ARGUMENT`, with the clock left as it was, when
/// `node` is not below `SUBETHA_CAUSAL_CLOCK_NODES` or its count is
/// already `UINT64_MAX`.
///
/// # Safety
/// `clock` is a valid pointer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn subetha_causal_clock_tick(clock: *mut subetha_causal_clock, node: u32) -> i32 {
    entry(|| {
        if clock.is_null() {
            return fail(SUBETHA_E_INVALID_ARGUMENT, "clock is null");
        }
        let index = node as usize;
        if index >= SUBETHA_CAUSAL_CLOCK_NODES {
            return fail(
                SUBETHA_E_INVALID_ARGUMENT,
                format!("a causal clock counts for {SUBETHA_CAUSAL_CLOCK_NODES} participants, numbered from zero; node {node} is past them"),
            );
        }
        // SAFETY: checked non-null; the caller guarantees it is valid.
        let clock = unsafe { &mut *clock };
        match vector(*clock).checked_increment(index) {
            Some(stepped) => {
                clock.counts = stepped.clock;
                SUBETHA_OK
            }
            None => fail(
                SUBETHA_E_INVALID_ARGUMENT,
                format!("participant {node}'s count is at UINT64_MAX, so no clock orders after it"),
            ),
        }
    })
}

/// Take `*other` into `*clock`: each count becomes the higher of the two,
/// which is the clock a receiver holds after taking a sender's in.
/// `clock` and `other` may be the same clock.
///
/// # Safety
/// `clock` and `other` are valid pointers.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn subetha_causal_clock_merge(
    clock: *mut subetha_causal_clock,
    other: *const subetha_causal_clock,
) -> i32 {
    entry(|| {
        if clock.is_null() {
            return fail(SUBETHA_E_INVALID_ARGUMENT, "clock is null");
        }
        if other.is_null() {
            return fail(SUBETHA_E_INVALID_ARGUMENT, "other is null");
        }
        // SAFETY: both checked non-null; the caller guarantees they are
        // valid. Each is copied out before `clock` is written, so the two
        // may address the same clock.
        let (mine, theirs) = unsafe { (*clock, *other) };
        let merged = vector(mine).merge(&vector(theirs));
        unsafe { (*clock).counts = merged.clock };
        SUBETHA_OK
    })
}

/// How `*a` stands to `*b`, into `out`: `SUBETHA_CAUSAL_BEFORE` when `a`
/// happened before `b` (no count higher and at least one lower),
/// `SUBETHA_CAUSAL_AFTER` the other way round, `SUBETHA_CAUSAL_EQUAL` when
/// every count matches, and `SUBETHA_CAUSAL_CONCURRENT` when neither
/// caused the other.
///
/// # Safety
/// `a`, `b` and `out` are valid pointers.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn subetha_causal_clock_compare(
    a: *const subetha_causal_clock,
    b: *const subetha_causal_clock,
    out: *mut i32,
) -> i32 {
    entry(|| {
        if a.is_null() {
            return fail(SUBETHA_E_INVALID_ARGUMENT, "a is null");
        }
        if b.is_null() {
            return fail(SUBETHA_E_INVALID_ARGUMENT, "b is null");
        }
        if out.is_null() {
            return fail(SUBETHA_E_INVALID_ARGUMENT, "out is null");
        }
        // SAFETY: all three checked non-null; the caller guarantees they
        // are valid.
        let (a, b) = unsafe { (*a, *b) };
        let order = match vector(a).causal_cmp(&vector(b)) {
            Some(Ordering::Less) => SUBETHA_CAUSAL_BEFORE,
            Some(Ordering::Equal) => SUBETHA_CAUSAL_EQUAL,
            Some(Ordering::Greater) => SUBETHA_CAUSAL_AFTER,
            None => SUBETHA_CAUSAL_CONCURRENT,
        };
        unsafe { *out = order };
        SUBETHA_OK
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The hybrid clock steps, merges and compares as the Rust type does,
    /// and a step from the ceiling is refused without writing `out`.
    #[test]
    fn a_hybrid_clock_steps_merges_and_compares_as_rust_does() {
        let start = subetha_hlc { physical_us: 100, logical: 4 };
        let mut out = subetha_hlc::default();
        let mut order = 9;
        // SAFETY: valid out pointers throughout.
        unsafe {
            assert_eq!(subetha_hlc_advance(start, 100, &mut out), SUBETHA_OK);
            assert_eq!((out.physical_us, out.logical), (100, 5));
            assert_eq!(subetha_hlc_advance(start, 200, &mut out), SUBETHA_OK);
            assert_eq!((out.physical_us, out.logical), (200, 0));

            let received = subetha_hlc { physical_us: 100, logical: 9 };
            assert_eq!(subetha_hlc_merge(start, received, 50, &mut out), SUBETHA_OK);
            assert_eq!((out.physical_us, out.logical), (100, 10));
            assert_eq!(subetha_hlc_compare(received, out, &mut order), SUBETHA_OK);
            assert_eq!(order, -1, "the merged reading orders after the one it took in");
            assert_eq!(subetha_hlc_compare(out, out, &mut order), SUBETHA_OK);
            assert_eq!(order, 0);

            let top = subetha_hlc { physical_us: 100, logical: u64::MAX };
            let before = out;
            assert_eq!(subetha_hlc_advance(top, 100, &mut out), SUBETHA_E_INVALID_ARGUMENT);
            assert_eq!(subetha_hlc_merge(start, top, 0, &mut out), SUBETHA_E_INVALID_ARGUMENT);
            assert_eq!((out.physical_us, out.logical), (before.physical_us, before.logical));
            assert_eq!(subetha_hlc_advance(top, 101, &mut out), SUBETHA_OK);
            assert_eq!((out.physical_us, out.logical), (101, 0));

            assert_eq!(subetha_hlc_now(&mut out), SUBETHA_OK);
            assert!(out.physical_us > 0 && out.logical == 0);
            assert_eq!(subetha_hlc_now(std::ptr::null_mut()), SUBETHA_E_INVALID_ARGUMENT);
        }
    }

    /// A causal clock ticks, merges and compares as the Rust type does,
    /// and a node past the sixteen or a count at the ceiling is refused
    /// with the clock left as it was.
    #[test]
    fn a_causal_clock_ticks_merges_and_compares_as_rust_does() {
        let mut a = subetha_causal_clock::default();
        let mut b = subetha_causal_clock::default();
        let mut order = 9;
        // SAFETY: valid clocks and out pointers throughout.
        unsafe {
            assert_eq!(subetha_causal_clock_tick(&mut a, 0), SUBETHA_OK);
            assert_eq!(subetha_causal_clock_tick(&mut b, 15), SUBETHA_OK);
            assert_eq!(subetha_causal_clock_compare(&a, &b, &mut order), SUBETHA_OK);
            assert_eq!(order, SUBETHA_CAUSAL_CONCURRENT);

            let mut merged = a;
            assert_eq!(subetha_causal_clock_merge(&mut merged, &b), SUBETHA_OK);
            assert_eq!((merged.counts[0], merged.counts[15]), (1, 1));
            assert_eq!(subetha_causal_clock_compare(&a, &merged, &mut order), SUBETHA_OK);
            assert_eq!(order, SUBETHA_CAUSAL_BEFORE);
            assert_eq!(subetha_causal_clock_compare(&merged, &b, &mut order), SUBETHA_OK);
            assert_eq!(order, SUBETHA_CAUSAL_AFTER);
            assert_eq!(subetha_causal_clock_compare(&merged, &merged, &mut order), SUBETHA_OK);
            assert_eq!(order, SUBETHA_CAUSAL_EQUAL);

            let alias: *mut subetha_causal_clock = &mut merged;
            assert_eq!(subetha_causal_clock_merge(alias, alias), SUBETHA_OK);
            assert_eq!((merged.counts[0], merged.counts[15]), (1, 1), "merging a clock into itself changes nothing");
            let mut rust = VectorClock::<SUBETHA_CAUSAL_CLOCK_NODES>::zero();
            rust.increment(0);
            rust.increment(15);
            assert_eq!(merged.counts, rust.clock, "the counts C holds are the counts Rust holds after the same events");

            assert_eq!(subetha_causal_clock_tick(&mut a, 16), SUBETHA_E_INVALID_ARGUMENT);
            a.counts[3] = u64::MAX;
            let held = a;
            assert_eq!(subetha_causal_clock_tick(&mut a, 3), SUBETHA_E_INVALID_ARGUMENT);
            assert_eq!(a.counts, held.counts, "a refused tick leaves the clock as it was");
            assert_eq!(subetha_causal_clock_tick(std::ptr::null_mut(), 0), SUBETHA_E_INVALID_ARGUMENT);
        }
    }
}
