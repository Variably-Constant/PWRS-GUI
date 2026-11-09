//! Exclusive access to a value through one atomic word, for a value whose
//! methods take `&mut` behind a handle any thread may call. An uncontended
//! claim is one compare-and-swap to take and one to give back.
//!
//! A caller that finds the value held waits by the plan in force for a
//! local wait ([`plan_in_force`]): it spins, then monitor-waits, taking the
//! value whenever it sees it free, until the plan's park point, which wait
//! calibration measures as the break-even between waiting and parking.
//! Until calibration publishes, the plan is the fixed ladder; a claim never
//! starts the calibration, since a claim is taken where starting a thread is
//! unsafe, such as a thread-local destructor. A caller still waiting at the
//! park point joins a first-in, first-out queue, keeping its place on its
//! own stack and sleeping on its own word, so the queue advances by waking
//! one thread. Only the queue's head sleeps on the claim word. It marks the
//! word and every release clears the mark, so a head that finds the value
//! held with its mark gone was passed over; it then marks the word owed, and
//! the next release hands the value straight to it, the word staying claimed
//! so no other caller can take it. A caller therefore has the value within
//! the park point plus two holds and a wake for itself and for each caller
//! queued ahead of it.

use std::cell::UnsafeCell;
use std::fmt;
use std::ops::{Deref, DerefMut};
use std::ptr;
use std::sync::atomic::{AtomicPtr, AtomicU32, Ordering};

use crate::cross_process_waker::platform_wait;
use crate::ordering::read_tsc;
use crate::wait_active::plan_in_force;
use crate::wait_plan::{MonitorPhase, ParkKind, Plan};

const FREE: u32 = 0;
/// Held, or on its way to the queue's head while [`HANDOFF`] is set.
const CLAIMED: u32 = 1;
/// The queue's head watches the word; every release clears it.
const MARKED: u32 = 2;
/// The queue's head sleeps in the kernel on the word, so a release wakes it.
const PARKED: u32 = 4;
/// The head saw a release go to another caller; the next release is its.
const OWED: u32 = 8;
/// A release handed the value to the head, the one caller that may take it.
const HANDOFF: u32 = 16;

/// A queued caller's turn: waiting, waiting asleep in the kernel, or head.
const WAITING: u32 = 0;
const WAITING_PARKED: u32 = 1;
const HEAD: u32 = 2;

/// A value reached only through [`claim`](Claim::claim), by one caller at a
/// time.
pub struct Claim<T: ?Sized> {
    state: AtomicU32,
    /// The last caller in the queue, or null while it is empty.
    tail: AtomicPtr<Waiter>,
    value: UnsafeCell<T>,
}

/// A queued caller's place, on that caller's stack while it is queued.
struct Waiter {
    /// The caller queued behind this one, once it has linked itself.
    next: AtomicPtr<Waiter>,
    /// [`WAITING`], [`WAITING_PARKED`] or [`HEAD`].
    turn: AtomicU32,
}

/// Aborts the process when dropped. A queued caller holds one, and forgets
/// it on leaving the queue, so an unwind can never leave the queue naming a
/// place on a stack that is gone.
struct AbortIfUnwound;

impl Drop for AbortIfUnwound {
    fn drop(&mut self) {
        std::process::abort();
    }
}

// SAFETY: the value is reached only through a `Claimed`, and the word lets
// one exist at a time, so moving the claim moves only its value. The queue
// names places on the stacks of callers inside `claim`, each live until that
// caller leaves the queue.
unsafe impl<T: ?Sized + Send> Send for Claim<T> {}
// SAFETY: as above; a shared claim hands the value to one caller at a time.
unsafe impl<T: ?Sized + Send> Sync for Claim<T> {}

impl<T> Claim<T> {
    /// A claim holding `value`, free.
    pub const fn new(value: T) -> Self {
        Self { state: AtomicU32::new(FREE), tail: AtomicPtr::new(ptr::null_mut()), value: UnsafeCell::new(value) }
    }

    /// The value, taking the claim apart.
    pub fn into_inner(self) -> T {
        self.value.into_inner()
    }
}

impl<T: ?Sized> Claim<T> {
    /// The value, for this caller alone until the returned guard drops.
    /// Waits while another caller holds it.
    pub fn claim(&self) -> Claimed<'_, T> {
        if self.state.compare_exchange(FREE, CLAIMED, Ordering::Acquire, Ordering::Relaxed).is_err() {
            self.claim_contended();
        }
        Claimed { claim: self }
    }

    /// The value through a unique reference, which no other caller can hold.
    pub fn get_mut(&mut self) -> &mut T {
        self.value.get_mut()
    }

    #[cold]
    fn claim_contended(&self) {
        let plan = plan_in_force(ParkKind::Local).plan;
        if self.barge(&plan) {
            return;
        }
        let queued = AbortIfUnwound;
        let place = Waiter { next: AtomicPtr::new(ptr::null_mut()), turn: AtomicU32::new(WAITING) };
        let me = ptr::from_ref(&place).cast_mut();
        let ahead = self.tail.swap(me, Ordering::AcqRel);
        if !ahead.is_null() {
            // SAFETY: the caller ahead stays queued until it has handed on
            // the head's place, and handing it on waits for this link.
            unsafe { (*ahead).next.store(me, Ordering::Release) };
            place.wait_for_turn(&plan);
        }
        self.take_as_head(&plan);
        self.hand_on(&place, me, &plan);
        std::mem::forget(queued);
    }

    /// Takes the value whenever it is free while the plan's spin and monitor
    /// phases last; false once they are spent.
    fn barge(&self, plan: &Plan) -> bool {
        for _ in 0..plan.spin_rounds {
            if self.state.load(Ordering::Relaxed) == FREE
                && self.state.compare_exchange_weak(FREE, CLAIMED, Ordering::Acquire, Ordering::Relaxed).is_ok()
            {
                return true;
            }
            std::hint::spin_loop();
        }
        let Some(phase) = plan.monitor else {
            return false;
        };
        let deadline = read_tsc().wrapping_add(phase.budget_cycles);
        loop {
            let seen = self.state.load(Ordering::Relaxed);
            if seen == FREE {
                if self.state.compare_exchange_weak(FREE, CLAIMED, Ordering::Acquire, Ordering::Relaxed).is_ok() {
                    return true;
                }
                continue;
            }
            let left = deadline.wrapping_sub(read_tsc());
            if left == 0 || left > phase.budget_cycles {
                return false;
            }
            let rest = MonitorPhase { budget_cycles: left, ..phase };
            platform_wait::monitor_tier(&self.state, seen, Some(&rest));
        }
    }

    /// Takes the value as the queue's head: whenever it is free, or at the
    /// release that hands it over once a release has gone to another caller.
    fn take_as_head(&self, plan: &Plan) {
        let mut marked = false;
        loop {
            let seen = self.state.load(Ordering::Acquire);
            if seen == FREE {
                if self.state.compare_exchange(FREE, CLAIMED, Ordering::Acquire, Ordering::Relaxed).is_ok() {
                    return;
                }
                continue;
            }
            if seen & HANDOFF != 0 {
                if self.state.compare_exchange(seen, CLAIMED, Ordering::Acquire, Ordering::Relaxed).is_ok() {
                    return;
                }
                continue;
            }
            let want = if seen & MARKED != 0 {
                seen
            } else if marked {
                seen | MARKED | OWED
            } else {
                seen | MARKED
            };
            if want != seen {
                if self.state.compare_exchange(seen, want, Ordering::Relaxed, Ordering::Relaxed).is_err() {
                    continue;
                }
                marked = true;
            }
            self.wait_while(want, plan);
        }
    }

    /// Waits by `plan` until the claim word is no longer `seen`, sleeping in
    /// the kernel at the end with the word marked parked.
    fn wait_while(&self, seen: u32, plan: &Plan) {
        for _ in 0..plan.spin_rounds {
            if self.state.load(Ordering::Relaxed) != seen {
                return;
            }
            std::hint::spin_loop();
        }
        if platform_wait::monitor_tier(&self.state, seen, plan.monitor.as_ref()) {
            return;
        }
        let parked = seen | PARKED;
        if parked != seen && self.state.compare_exchange(seen, parked, Ordering::Relaxed, Ordering::Relaxed).is_err()
        {
            return;
        }
        platform_wait::wait_forever(&self.state, parked, false, None);
    }

    /// Hands the head's place to the caller queued behind `place`, or empties
    /// the queue. Called holding the value, which the caller behind waits
    /// for, so that caller and its place outlast this call.
    fn hand_on(&self, place: &Waiter, me: *mut Waiter, plan: &Plan) {
        let mut behind = place.next.load(Ordering::Acquire);
        if behind.is_null() {
            if self.tail.compare_exchange(me, ptr::null_mut(), Ordering::AcqRel, Ordering::Acquire).is_ok() {
                return;
            }
            // A caller has made itself the tail and is about to link.
            let mut spins = 0u32;
            loop {
                behind = place.next.load(Ordering::Acquire);
                if !behind.is_null() {
                    break;
                }
                if spins < plan.spin_rounds {
                    spins += 1;
                    std::hint::spin_loop();
                } else {
                    std::thread::yield_now();
                }
            }
        }
        // SAFETY: the caller behind is queued until it holds the value, which
        // this thread holds now.
        let turn = unsafe { &(*behind).turn };
        if turn.swap(HEAD, Ordering::Release) == WAITING_PARKED {
            platform_wait::wake_one(turn, false);
        }
    }

    fn release(&self) {
        let mut seen = CLAIMED;
        loop {
            let next = if seen & OWED != 0 { CLAIMED | HANDOFF } else { FREE };
            match self.state.compare_exchange(seen, next, Ordering::Release, Ordering::Relaxed) {
                Ok(_) => break,
                Err(actual) => seen = actual,
            }
        }
        if seen & PARKED != 0 {
            platform_wait::wake_one(&self.state, false);
        }
    }
}

impl Waiter {
    /// Waits by `plan` until the caller ahead hands this one the head's place.
    fn wait_for_turn(&self, plan: &Plan) {
        for _ in 0..plan.spin_rounds {
            if self.turn.load(Ordering::Acquire) == HEAD {
                return;
            }
            std::hint::spin_loop();
        }
        // The turn leaves WAITING only for HEAD, so a change is the turn.
        if platform_wait::monitor_tier(&self.turn, WAITING, plan.monitor.as_ref()) {
            return;
        }
        loop {
            let seen = self.turn.load(Ordering::Acquire);
            if seen == HEAD {
                return;
            }
            if seen == WAITING
                && self.turn.compare_exchange(WAITING, WAITING_PARKED, Ordering::Acquire, Ordering::Acquire).is_err()
            {
                continue;
            }
            platform_wait::wait_forever(&self.turn, WAITING_PARKED, false, None);
        }
    }
}

impl<T: Default> Default for Claim<T> {
    fn default() -> Self {
        Self::new(T::default())
    }
}

impl<T: ?Sized> fmt::Debug for Claim<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let held = self.state.load(Ordering::Relaxed) & CLAIMED != 0;
        f.debug_struct("Claim").field("held", &held).finish_non_exhaustive()
    }
}

/// One caller's hold on a [`Claim`]'s value, given back when it drops,
/// unwinding included.
pub struct Claimed<'a, T: ?Sized> {
    claim: &'a Claim<T>,
}

impl<T: ?Sized> Deref for Claimed<'_, T> {
    type Target = T;

    fn deref(&self) -> &T {
        // SAFETY: this guard is the one hold the word allows.
        unsafe { &*self.claim.value.get() }
    }
}

impl<T: ?Sized> DerefMut for Claimed<'_, T> {
    fn deref_mut(&mut self) -> &mut T {
        // SAFETY: this guard is the one hold the word allows.
        unsafe { &mut *self.claim.value.get() }
    }
}

impl<T: ?Sized> Drop for Claimed<'_, T> {
    fn drop(&mut self) {
        self.claim.release();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, AtomicU64};
    use std::sync::Arc;
    use std::time::{Duration, Instant};

    /// How long a test waits on another thread before calling it stuck.
    const STUCK: Duration = Duration::from_secs(30);

    /// Waits until `done` answers true, failing the test past [`STUCK`].
    fn wait_until(what: &str, done: impl Fn() -> bool) {
        let deadline = Instant::now() + STUCK;
        while !done() {
            assert!(Instant::now() < deadline, "{what} did not happen within {STUCK:?}");
            std::thread::yield_now();
        }
    }

    #[test]
    fn threads_counting_through_one_claim_lose_no_increment() {
        let counter = Arc::new(Claim::new(0u64));
        let threads: Vec<_> = (0..8)
            .map(|_| {
                let counter = Arc::clone(&counter);
                std::thread::spawn(move || {
                    for _ in 0..10_000 {
                        *counter.claim() += 1;
                    }
                })
            })
            .collect();
        for t in threads {
            t.join().expect("a counting thread finished");
        }
        assert_eq!(*counter.claim(), 80_000);
    }

    /// Sixteen threads holding the value for a moment each, so callers run
    /// past the park point, queue, and hand the head's place on.
    #[test]
    fn a_contended_claim_queues_and_loses_no_increment() {
        let counter = Arc::new(Claim::new(0u64));
        let threads: Vec<_> = (0..16)
            .map(|_| {
                let counter = Arc::clone(&counter);
                std::thread::spawn(move || {
                    for _ in 0..2_000 {
                        let mut held = counter.claim();
                        *held += 1;
                        for _ in 0..200 {
                            std::hint::spin_loop();
                        }
                    }
                })
            })
            .collect();
        for t in threads {
            t.join().expect("a counting thread finished");
        }
        assert_eq!(*counter.claim(), 32_000);
        assert!(counter.tail.load(Ordering::Acquire).is_null(), "the queue is empty once everyone has left");
    }

    /// A caller that waits past the park point sleeps as the queue's head,
    /// and the holder's release wakes it with the holder's write visible.
    #[test]
    fn a_parked_head_is_woken_by_the_release() {
        let value = Arc::new(Claim::new(0u32));
        let mut held = value.claim();
        let waiter = {
            let value = Arc::clone(&value);
            std::thread::spawn(move || (*value.claim(), Instant::now()))
        };
        wait_until("the waiter parking as head", || value.state.load(Ordering::Acquire) & PARKED != 0);
        *held = 7;
        let released = Instant::now();
        drop(held);
        wait_until("the woken waiter finishing", || waiter.is_finished());
        let (got, woke) = waiter.join().expect("the waiter finished");
        assert_eq!(got, 7, "the waiter sees what the holder wrote");
        assert!(woke >= released, "the waiter got the value only after the release");
    }

    /// Callers that queue one after another while the value is held take it
    /// in the order they queued.
    #[test]
    fn queued_callers_take_the_value_in_the_order_they_queued() {
        let order = Arc::new(Claim::new(Vec::new()));
        let held = order.claim();
        let mut callers = Vec::new();
        let mut tail = ptr::null_mut();
        for index in 0..6usize {
            let order_for = Arc::clone(&order);
            callers.push(std::thread::spawn(move || order_for.claim().push(index)));
            wait_until("the caller joining the queue", || {
                let now = order.tail.load(Ordering::Acquire);
                !now.is_null() && now != tail
            });
            tail = order.tail.load(Ordering::Acquire);
        }
        drop(held);
        for caller in callers {
            caller.join().expect("a queued caller finished");
        }
        assert_eq!(*order.claim(), vec![0, 1, 2, 3, 4, 5]);
    }

    /// A release while the head is owed hands the value over: the word stays
    /// claimed, so neither a newcomer nor the releaser can take it.
    #[test]
    fn a_release_owed_to_the_head_hands_the_value_over() {
        let value = Claim::new(0u32);
        let held = value.claim();
        value.state.fetch_or(MARKED | OWED, Ordering::AcqRel);
        drop(held);
        assert_eq!(value.state.load(Ordering::Acquire), CLAIMED | HANDOFF);
        assert!(
            value.state.compare_exchange(FREE, CLAIMED, Ordering::Acquire, Ordering::Relaxed).is_err(),
            "a newcomer's claim fails while the value is on its way to the head"
        );
    }

    /// The head is passed over once: a release and a newcomer's claim land
    /// as one swap, so the head cannot win that race. The head then marks
    /// itself owed, and the next release goes to it even though the releaser
    /// claims again at once.
    #[test]
    fn a_head_passed_over_once_takes_the_next_release() {
        let value = Arc::new(Claim::new(0u32));
        let held = value.claim();
        let head = {
            let value = Arc::clone(&value);
            std::thread::spawn(move || *value.claim() = 1)
        };
        wait_until("the head parking", || value.state.load(Ordering::Acquire) & PARKED != 0);
        let before = value.state.swap(CLAIMED, Ordering::AcqRel);
        assert_eq!(before & (CLAIMED | MARKED | PARKED), CLAIMED | MARKED | PARKED);
        platform_wait::wake_one(&value.state, false);
        wait_until("the head marking itself owed", || value.state.load(Ordering::Acquire) & OWED != 0);
        drop(held);
        let again = value.claim();
        assert_eq!(*again, 1, "the release went to the owed head before the releaser's next claim");
        drop(again);
        head.join().expect("the head finished");
    }

    /// One thread claims and releases in a loop; another claims two hundred
    /// times. Each of its claims is owed the second release after it queues,
    /// so it finishes however tight the loop is.
    #[test]
    fn a_thread_reclaiming_in_a_loop_cannot_starve_another() {
        let value = Arc::new(Claim::new(0u64));
        let stop = Arc::new(AtomicBool::new(false));
        let hammer = {
            let value = Arc::clone(&value);
            let stop = Arc::clone(&stop);
            std::thread::spawn(move || {
                while !stop.load(Ordering::Relaxed) {
                    let mut held = value.claim();
                    *held += 1;
                    for _ in 0..64 {
                        std::hint::spin_loop();
                    }
                }
            })
        };
        wait_until("the loop getting going", || *value.claim() > 1_000);
        let reads = Arc::new(AtomicU64::new(0));
        let reader = {
            let value = Arc::clone(&value);
            let reads = Arc::clone(&reads);
            std::thread::spawn(move || {
                for _ in 0..200 {
                    std::hint::black_box(*value.claim());
                    reads.fetch_add(1, Ordering::Relaxed);
                }
            })
        };
        let deadline = Instant::now() + STUCK;
        while !reader.is_finished() {
            if Instant::now() >= deadline {
                stop.store(true, Ordering::Relaxed);
                panic!("the reader made {} of 200 claims within {STUCK:?}", reads.load(Ordering::Relaxed));
            }
            std::thread::yield_now();
        }
        stop.store(true, Ordering::Relaxed);
        reader.join().expect("the reader finished");
        hammer.join().expect("the loop finished");
    }

    #[test]
    fn a_panic_while_claimed_gives_the_claim_back() {
        let value = Claim::new(1u32);
        let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let mut held = value.claim();
            *held = 5;
            panic!("inside the claim");
        }));
        assert!(outcome.is_err());
        assert_eq!(value.state.load(Ordering::Acquire), FREE, "the unwind gave the claim back");
        assert_eq!(*value.claim(), 5);
    }
}
