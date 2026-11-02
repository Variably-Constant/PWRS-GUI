//! Indexes from a bump counter below a limit. The counter only rises, so
//! each index below the limit goes to one taker.
//!
//! [`take32`](crate::bump::take32) and [`take64`](crate::bump::take64) read nothing before their add: they serve a
//! structure that records its counter's exhaustion in a word its allocate
//! reads anyway, and stops taking once that record is set. An add that
//! lands at or past the limit is given back, so a reader of the counter
//! clamps it to the limit. The limit is read after the add.

use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};

/// A bump counter's limit, the handle field a take reads after its add.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Limit(u64);

impl Limit {
    /// The limit `limit`.
    pub(crate) const fn new(limit: u64) -> Self {
        Self(limit)
    }

    /// The limit itself.
    #[inline(always)]
    pub(crate) const fn value(self) -> u64 {
        self.0
    }
}

/// Why a structure's allocate handed out nothing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Refusal {
    /// The structure's exhaustion record was set, read with no locked
    /// operation.
    Recorded,
    /// The counter refused a take; the refusing allocate sets the record.
    Counter,
}

/// The next index below `limit` from `counter`, or None at the limit: one
/// add, given back when it lands at or past the limit. Nothing is read
/// before the add, and `limit` after it.
#[inline(always)]
pub(crate) fn take32(counter: &AtomicU32, limit: &Limit) -> Option<u32> {
    let idx = counter.fetch_add(1, Ordering::AcqRel);
    if u64::from(idx) >= limit.0 {
        counter.fetch_sub(1, Ordering::AcqRel);
        return None;
    }
    Some(idx)
}

/// [`take32`] for a 64-bit counter.
#[inline(always)]
pub(crate) fn take64(counter: &AtomicU64, limit: &Limit) -> Option<u64> {
    let idx = counter.fetch_add(1, Ordering::AcqRel);
    if idx >= limit.0 {
        counter.fetch_sub(1, Ordering::AcqRel);
        return None;
    }
    Some(idx)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn a_take_hands_out_each_index_below_the_limit_then_refuses() {
        let c = AtomicU32::new(0);
        let limit = Limit::new(5);
        assert_eq!(limit.value(), 5);
        for want in 0..5 {
            assert_eq!(take32(&c, &limit), Some(want));
        }
        assert_eq!(take32(&c, &limit), None);
        assert_eq!(c.load(Ordering::Relaxed), 5, "a refused take leaves the counter at the limit");
        assert_eq!(take32(&c, &Limit::new(0)), None);
        assert_eq!(c.load(Ordering::Relaxed), 5);
        let d = AtomicU64::new(3);
        assert_eq!(take64(&d, &Limit::new(4)), Some(3));
        assert_eq!(take64(&d, &Limit::new(4)), None);
        assert_eq!(d.load(Ordering::Relaxed), 4, "a refused take leaves the counter at the limit");
    }

    /// Eight takers draining one counter together get every index below the
    /// limit once, and the counter ends at the limit.
    #[test]
    fn racing_takes_get_every_index_below_the_limit_once() {
        const LIMIT: u32 = 200_000;
        let limit = Limit::new(u64::from(LIMIT));
        let c32 = AtomicU32::new(0);
        let c64 = AtomicU64::new(0);
        let (got32, got64): (Vec<Vec<u32>>, Vec<Vec<u64>>) = std::thread::scope(|s| {
            let a: Vec<_> = (0..8)
                .map(|_| {
                    let c = &c32;
                    s.spawn(move || std::iter::from_fn(|| take32(c, &limit)).collect::<Vec<u32>>())
                })
                .collect();
            let b: Vec<_> = (0..8)
                .map(|_| {
                    let c = &c64;
                    s.spawn(move || std::iter::from_fn(|| take64(c, &limit)).collect::<Vec<u64>>())
                })
                .collect();
            (
                a.into_iter().map(|h| h.join().expect("a 32-bit taker")).collect(),
                b.into_iter().map(|h| h.join().expect("a 64-bit taker")).collect(),
            )
        });
        let mut seen = HashSet::new();
        for i in got32.into_iter().flatten() {
            assert!(i < LIMIT && seen.insert(i), "32-bit index {i} out of range or handed out twice");
        }
        assert_eq!(seen.len(), LIMIT as usize);
        let mut seen = HashSet::new();
        for i in got64.into_iter().flatten() {
            assert!(i < u64::from(LIMIT) && seen.insert(i), "64-bit index {i} out of range or handed out twice");
        }
        assert_eq!(seen.len(), LIMIT as usize);
        assert_eq!(c32.load(Ordering::Relaxed), LIMIT, "every overshoot was given back");
        assert_eq!(c64.load(Ordering::Relaxed), u64::from(LIMIT), "every overshoot was given back");
    }
}
