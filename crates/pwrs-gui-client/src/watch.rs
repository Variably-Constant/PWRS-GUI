//! Ending waits from other threads: the canceler a caller ties to Ctrl+C,
//! and the registry a session wakes when its renderer dies.

use arc_swap::ArcSwap;
use pwrs_gui_proto::transport::Ring;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Weak};

/// Something that ends one wait when woken.
pub(crate) trait Wake: Send + Sync {
    fn wake(&self);
}

/// Ends a wait on a ring by closing the waiter's handle on it.
pub(crate) struct RingWake(pub(crate) Arc<Ring>);

impl Wake for RingWake {
    fn wake(&self) {
        self.0.close();
    }
}

/// Ends a wait in which the waiting thread is parked: waking marks the
/// wait woken and unparks the thread.
pub(crate) struct ParkWake {
    thread: std::thread::Thread,
    woken: AtomicBool,
}

impl ParkWake {
    /// A wake for a wait on the calling thread.
    pub(crate) fn here() -> ParkWake {
        ParkWake { thread: std::thread::current(), woken: AtomicBool::new(false) }
    }

    /// Parks the calling thread, which must be the one [`here`](Self::here)
    /// named, until the wait is woken.
    pub(crate) fn wait(&self) {
        while !self.woken.load(Ordering::Acquire) {
            std::thread::park();
        }
    }
}

impl Wake for ParkWake {
    fn wake(&self) {
        self.woken.store(true, Ordering::Release);
        self.thread.unpark();
    }
}

/// The waits a session ends when its renderer dies. Adding prunes the
/// waits already gone; neither adding nor waking takes a lock.
pub(crate) struct Watches {
    list: ArcSwap<Vec<Weak<dyn Wake>>>,
}

impl Watches {
    pub(crate) fn new() -> Watches {
        Watches { list: ArcSwap::from_pointee(Vec::new()) }
    }

    pub(crate) fn add(&self, wake: &Arc<dyn Wake>) {
        let weak = Arc::downgrade(wake);
        self.list.rcu(|current| {
            let mut next: Vec<Weak<dyn Wake>> = current.iter().filter(|w| w.strong_count() > 0).cloned().collect();
            next.push(weak.clone());
            next
        });
    }

    pub(crate) fn wake_all(&self) {
        for w in self.list.load().iter() {
            if let Some(live) = w.upgrade() {
                live.wake();
            }
        }
    }
}

/// Ends a wait from any thread, as a Ctrl+C handler does: the wait
/// returns [`crate::GuiError::Canceled`]. Calling it again does nothing.
#[derive(Clone)]
pub struct Canceler {
    canceled: Arc<AtomicBool>,
    wake: Arc<dyn Wake>,
}

impl Canceler {
    pub(crate) fn new(wake: Arc<dyn Wake>) -> Canceler {
        Canceler { canceled: Arc::new(AtomicBool::new(false)), wake }
    }

    pub fn cancel(&self) {
        self.canceled.store(true, Ordering::SeqCst);
        self.wake.wake();
    }

    pub fn is_canceled(&self) -> bool {
        self.canceled.load(Ordering::SeqCst)
    }

    pub(crate) fn wake(&self) -> &Arc<dyn Wake> {
        &self.wake
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicUsize;

    struct Count(AtomicUsize);

    impl Wake for Count {
        fn wake(&self) {
            self.0.fetch_add(1, Ordering::SeqCst);
        }
    }

    #[test]
    fn waking_reaches_every_live_wait_and_skips_the_gone() {
        let watches = Watches::new();
        let kept = Arc::new(Count(AtomicUsize::new(0)));
        let late = Arc::new(Count(AtomicUsize::new(0)));
        let kept_dyn: Arc<dyn Wake> = kept.clone();
        watches.add(&kept_dyn);
        let gone: Arc<dyn Wake> = Arc::new(Count(AtomicUsize::new(0)));
        watches.add(&gone);
        drop(gone);
        let late_dyn: Arc<dyn Wake> = late.clone();
        watches.add(&late_dyn);
        assert_eq!(watches.list.load().len(), 2, "adding pruned the wait that was gone");
        watches.wake_all();
        assert_eq!(kept.0.load(Ordering::SeqCst), 1);
        assert_eq!(late.0.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn a_parked_wait_ends_when_woken_from_another_thread_or_before_it_began() {
        let early = ParkWake::here();
        early.wake();
        early.wait();

        let parked = Arc::new(ParkWake::here());
        let waking = Arc::clone(&parked);
        let waker = std::thread::spawn(move || waking.wake());
        parked.wait();
        waker.join().expect("the waking thread");
    }

    #[test]
    fn a_canceler_marks_and_wakes() {
        let count = Arc::new(Count(AtomicUsize::new(0)));
        let wake: Arc<dyn Wake> = count.clone();
        let c = Canceler::new(wake);
        assert!(!c.is_canceled());
        c.clone().cancel();
        assert!(c.is_canceled());
        assert_eq!(count.0.load(Ordering::SeqCst), 1);
    }
}
