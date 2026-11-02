//! The waker pairs a ring wakes after a structural move.
//!
//! A shape morph, a capacity morph and a locale migration each change
//! where the next send and receive land, so a producer parked on a full
//! ring can have room, and a consumer parked on an empty one an item,
//! without anything being sent or received. Each blocking type registers
//! its two wakers with the ring it wraps, and the ring wakes every waiter
//! on them after each move made through it in this process, whether the
//! caller, a sidecar or a registration made it. A capacity or locale ring
//! shares its watch with every backing it builds, so a backing's reshape
//! wakes the waiters on the outer ring.

use std::sync::{Arc, Weak};

use subetha_core::SwapCell;

use crate::cross_process_waker::CrossProcessWaker;

/// A consumer waker and a producer waker, held weakly so a blocking type
/// that has gone stops being woken.
type Pair = (Weak<CrossProcessWaker>, Weak<CrossProcessWaker>);

/// The waker pairs registered with one ring.
#[derive(Default)]
pub(crate) struct MoveWatch {
    pairs: SwapCell<Vec<Pair>>,
}

impl MoveWatch {
    /// Register `consumer` and `producer`. A consumer waker already
    /// registered keeps its one entry, and pairs whose wakers have gone
    /// are dropped.
    pub(crate) fn watch(&self, consumer: &Arc<CrossProcessWaker>, producer: &Arc<CrossProcessWaker>) {
        self.pairs.rcu(|pairs| {
            let mut next: Vec<Pair> = pairs
                .iter()
                .filter(|(c, p)| c.strong_count() > 0 && p.strong_count() > 0)
                .cloned()
                .collect();
            if !next.iter().any(|(c, _)| std::ptr::eq(c.as_ptr(), Arc::as_ptr(consumer))) {
                next.push((Arc::downgrade(consumer), Arc::downgrade(producer)));
            }
            next
        });
    }

    /// The registered pairs whose wakers are still held.
    pub(crate) fn live(&self) -> Vec<(Arc<CrossProcessWaker>, Arc<CrossProcessWaker>)> {
        self.pairs
            .load()
            .iter()
            .filter_map(|(c, p)| Some((c.upgrade()?, p.upgrade()?)))
            .collect()
    }

    /// Wake every waiter on every live pair.
    pub(crate) fn moved(&self) {
        for (consumer, producer) in self.live() {
            consumer.wake_all();
            producer.wake_all();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cross_process_waker::MAX_WAITERS_DEFAULT;

    fn pair() -> (Arc<CrossProcessWaker>, Arc<CrossProcessWaker>) {
        (
            Arc::new(CrossProcessWaker::create_anon(MAX_WAITERS_DEFAULT).expect("a consumer waker")),
            Arc::new(CrossProcessWaker::create_anon(MAX_WAITERS_DEFAULT).expect("a producer waker")),
        )
    }

    #[test]
    fn a_pair_registered_twice_is_held_once_and_a_dropped_pair_goes() {
        let watch = MoveWatch::default();
        let (c1, p1) = pair();
        let (c2, p2) = pair();
        watch.watch(&c1, &p1);
        watch.watch(&c1, &p1);
        watch.watch(&c2, &p2);
        assert_eq!(watch.live().len(), 2, "each consumer waker is held once");
        drop((c2, p2));
        assert_eq!(watch.live().len(), 1, "a pair whose wakers have gone is not live");
        let (c3, p3) = pair();
        watch.watch(&c3, &p3);
        assert_eq!(watch.pairs.load().len(), 2, "registering drops the dead pair");
    }
}
