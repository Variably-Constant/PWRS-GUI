//! A work-stealing deque every process can wait on, whichever surface it
//! reaches SubEtha through.
//!
//! [`BlockingDeque`] keeps a [`RawDeque`] together with the two wakers its
//! waiters park on. A push through it wakes one thief waiting for an item
//! and a steal wakes the owner waiting for room, so a waiter in C, Python,
//! PowerShell or Rust wakes on a push or steal made through any of them. A
//! push or steal made through the [`RawDeque`] itself wakes no one. The
//! owner's pop wakes no one either: the owner is the deque's only
//! producer, and it is not waiting while it pops.
//!
//! The wakers sit in the files `<path>.cwaker.bin` and `<path>.pwaker.bin`
//! beside the deque at `<path>`, where the C ABI keeps them.

use std::convert::Infallible;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

pub use crate::blocking_wait::{BlockingWaitError, MAX_WAITERS_DEFAULT};
use crate::blocking_wait::{deadline, file_wakers, WaitState, WAKE_ANY};
use crate::cross_process_waker::CrossProcessWaker;
use crate::raw_deque::RawDeque;
use crate::shared_deque::DequeError;

/// A [`RawDeque`] with the wakers its waiters share across processes.
pub struct BlockingDeque {
    deque: RawDeque,
    consumer_waker: Arc<CrossProcessWaker>,
    producer_waker: Arc<CrossProcessWaker>,
    state: WaitState,
}

impl BlockingDeque {
    /// `deque` with its wakers in this process alone; each waker has
    /// `max_waiters` slots.
    pub fn anon(deque: RawDeque, max_waiters: usize) -> Result<Self, BlockingWaitError<DequeError>> {
        let consumer_waker = CrossProcessWaker::create_anon(max_waiters)?;
        let producer_waker = CrossProcessWaker::create_anon(max_waiters)?;
        Ok(Self::from_parts(deque, consumer_waker, producer_waker))
    }

    /// `deque` with its wakers in the files `<path>.cwaker.bin` and
    /// `<path>.pwaker.bin`, made when they do not exist and attached to
    /// when they do. `path` is the deque's own file.
    pub fn beside_file(deque: RawDeque, path: impl AsRef<Path>, max_waiters: usize) -> Result<Self, BlockingWaitError<DequeError>> {
        let (consumer_waker, producer_waker) = file_wakers(path.as_ref(), max_waiters)?;
        Ok(Self::from_parts(deque, consumer_waker, producer_waker))
    }

    /// `deque` with wakers its caller made. Waiters in other processes
    /// wake only when these are the ones beside the deque under the names
    /// above.
    pub fn from_parts(deque: RawDeque, consumer_waker: CrossProcessWaker, producer_waker: CrossProcessWaker) -> Self {
        Self {
            deque,
            consumer_waker: Arc::new(consumer_waker),
            producer_waker: Arc::new(producer_waker),
            state: WaitState::default(),
        }
    }

    /// The deque itself, for everything that neither pushes nor steals.
    pub fn deque(&self) -> &RawDeque {
        &self.deque
    }

    /// The waker thieves park on while the deque is empty.
    pub fn consumer_waker(&self) -> &Arc<CrossProcessWaker> {
        &self.consumer_waker
    }

    /// The waker the owner parks on while the deque is full.
    pub fn producer_waker(&self) -> &Arc<CrossProcessWaker> {
        &self.producer_waker
    }

    /// Waits through this handle that found every waiter slot in use and
    /// returned [`BlockingWaitError::WakerFull`].
    pub fn waiters_full(&self) -> u64 {
        self.state.waiters_full()
    }

    /// Whether [`close`](Self::close) has been called on this handle.
    pub fn is_closed(&self) -> bool {
        self.state.is_closed()
    }

    /// End every wait on this handle, now and from here on: each returns
    /// [`BlockingWaitError::Closed`]. Waits through other handles on the
    /// same deque wake, find the deque as it was and wait again.
    pub fn close(&self) {
        self.state.close();
        self.consumer_waker.wake_all();
        self.producer_waker.wake_all();
    }

    /// [`RawDeque::push`], then wake one waiting thief.
    pub fn try_push(&self, payload: &[u8]) -> Result<(), DequeError> {
        self.deque.push(payload)?;
        self.consumer_waker.wake_one_up_to(WAKE_ANY);
        Ok(())
    }

    /// [`RawDeque::pop`], the owner's take from its own end.
    pub fn try_pop(&self, out: &mut [u8]) -> Option<usize> {
        self.deque.pop(out)
    }

    /// [`RawDeque::steal`], then wake the owner if it waits for room.
    pub fn try_steal(&self, out: &mut [u8]) -> Option<usize> {
        let n = self.deque.steal(out)?;
        self.producer_waker.wake_one_up_to(WAKE_ANY);
        Some(n)
    }

    /// Push, parking until there is room or `timeout` has passed; `None`
    /// waits as long as it takes.
    pub fn push_blocking(&self, payload: &[u8], timeout: Option<Duration>) -> Result<(), BlockingWaitError<DequeError>> {
        self.state.wait(&self.producer_waker, deadline(timeout), |e| matches!(e, DequeError::Full), || {
            self.try_push(payload)
        })
    }

    /// Steal, parking until there is an item or `timeout` has passed;
    /// `None` waits as long as it takes. A steal another thief wins is
    /// tried again rather than reported.
    pub fn steal_blocking(&self, out: &mut [u8], timeout: Option<Duration>) -> Result<usize, BlockingWaitError<Infallible>> {
        self.state.wait_for(&self.consumer_waker, deadline(timeout), || self.try_steal(out))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::raw_treiber_stack::ElementLayout;
    use std::path::PathBuf;
    use std::time::Instant;

    /// A folder of its own under the temporary directory, removed when the
    /// test ends; declared before the deques so it drops after them.
    struct Folder(PathBuf);

    impl Folder {
        fn new(name: &str) -> Self {
            let dir = std::env::temp_dir().join(format!(
                "subetha-blocking-deque-{name}-{}-{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .expect("the wall clock is after the epoch")
                    .as_nanos()
            ));
            std::fs::create_dir_all(&dir).expect("make the test's folder");
            Self(dir)
        }

        fn path(&self) -> PathBuf {
            self.0.join("deque")
        }
    }

    impl Drop for Folder {
        fn drop(&mut self) {
            if let Err(e) = std::fs::remove_dir_all(&self.0) {
                eprintln!("could not remove {}: {e}", self.0.display());
            }
        }
    }

    const LAYOUT: ElementLayout = ElementLayout { slot_size: 8, alignment: 1, tag: 0 };

    /// The owner's handle on a new deque of four slots at `path`.
    fn owner(path: &Path) -> BlockingDeque {
        let deque = RawDeque::create(path, 4, LAYOUT).expect("make the deque");
        BlockingDeque::beside_file(deque, path, MAX_WAITERS_DEFAULT).expect("its wakers")
    }

    /// A thief's handle on the deque at `path`, as another process opens it.
    fn thief(path: &Path) -> BlockingDeque {
        let deque = RawDeque::open_as_thief(path, LAYOUT).expect("attach to the deque");
        BlockingDeque::beside_file(deque, path, MAX_WAITERS_DEFAULT).expect("its wakers")
    }

    /// Push until the deque refuses, so the next push must wait for room.
    fn fill(deque: &BlockingDeque) {
        loop {
            match deque.try_push(b"x") {
                Ok(()) => {}
                Err(DequeError::Full) => break,
                Err(e) => panic!("filling the deque: {e}"),
            }
        }
    }

    /// A thread pushing into a full deque through `deque`, which reports
    /// what its push came to and how long it took, returned once its wait
    /// has begun.
    #[allow(clippy::type_complexity)]
    fn waiting_owner(
        deque: &Arc<BlockingDeque>,
        timeout: Duration,
    ) -> std::thread::JoinHandle<(Result<(), BlockingWaitError<DequeError>>, Duration)> {
        let waiting = Arc::clone(deque);
        let returned = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let owner_returned = Arc::clone(&returned);
        let owner = std::thread::spawn(move || {
            let began = Instant::now();
            let pushed = waiting.push_blocking(b"late", Some(timeout));
            owner_returned.store(true, std::sync::atomic::Ordering::Release);
            (pushed, began.elapsed())
        });
        deque.producer_waker().until_a_wait_begins(&returned);
        owner
    }

    #[test]
    fn a_wait_for_an_item_times_out_and_takes_one_once_one_is_there() {
        let folder = Folder::new("timeout");
        let deque = RawDeque::create(folder.path(), 4, LAYOUT).expect("make the deque");
        let blocking = BlockingDeque::anon(deque, MAX_WAITERS_DEFAULT).expect("its wakers");
        let mut out = [0u8; 8];
        let began = Instant::now();
        assert!(matches!(
            blocking.steal_blocking(&mut out, Some(Duration::from_millis(50))),
            Err(BlockingWaitError::Timeout)
        ));
        assert!(began.elapsed() >= Duration::from_millis(40), "the wait lasted its timeout");
        blocking.try_push(b"one").expect("push");
        blocking.steal_blocking(&mut out, Some(Duration::from_secs(5))).expect("steal");
        assert_eq!(&out[..3], b"one");
    }

    #[test]
    fn a_push_through_another_handle_on_the_file_wakes_a_waiting_thief() {
        let folder = Folder::new("wake");
        let path = folder.path();
        let pushing = owner(&path);
        let stealing = Arc::new(thief(&path));
        let waiting = Arc::clone(&stealing);
        let waiter = std::thread::spawn(move || {
            let mut out = [0u8; 8];
            let began = Instant::now();
            let taken = waiting.steal_blocking(&mut out, Some(Duration::from_secs(20)));
            (taken, out, began.elapsed())
        });
        std::thread::sleep(Duration::from_millis(100));
        pushing.try_push(b"pushed").expect("push");
        let (taken, item, waited) = waiter.join().expect("the waiter thread");
        taken.expect("the woken steal takes the item");
        assert_eq!(&item[..6], b"pushed");
        assert!(waited < Duration::from_secs(10), "woken by the push, not by the timeout: {waited:?}");
    }

    #[test]
    fn a_steal_through_another_handle_wakes_an_owner_waiting_for_room() {
        let folder = Folder::new("room");
        let path = folder.path();
        let pushing = Arc::new(owner(&path));
        let stealing = thief(&path);
        fill(&pushing);
        let waiter = waiting_owner(&pushing, Duration::from_secs(20));
        let mut out = [0u8; 8];
        stealing.try_steal(&mut out).expect("make room");
        let (pushed, waited) = waiter.join().expect("the owner thread");
        pushed.expect("the woken push lands in the freed slot");
        assert!(waited < Duration::from_secs(10), "woken by the steal, not by the timeout: {waited:?}");
    }

    #[test]
    fn a_steal_through_the_deque_alone_leaves_that_owner_asleep() {
        let folder = Folder::new("alone");
        let path = folder.path();
        let pushing = Arc::new(owner(&path));
        let stealing = thief(&path);
        fill(&pushing);
        let timeout = Duration::from_millis(600);
        let waiter = waiting_owner(&pushing, timeout);
        let mut out = [0u8; 8];
        stealing.deque().steal(&mut out).expect("make room without waking anyone");
        let (pushed, waited) = waiter.join().expect("the owner thread");
        // Its wait ends at the timeout, and the try that follows finds the
        // room the silent steal made.
        pushed.expect("the push lands once its wait ends");
        assert!(waited >= timeout - Duration::from_millis(50), "slept until its timeout: {waited:?}");
    }

    #[test]
    fn closing_ends_a_wait_on_that_handle() {
        let folder = Folder::new("close");
        let deque = RawDeque::create(folder.path(), 4, LAYOUT).expect("make the deque");
        let blocking = Arc::new(BlockingDeque::anon(deque, MAX_WAITERS_DEFAULT).expect("its wakers"));
        let waiting = Arc::clone(&blocking);
        let waiter = std::thread::spawn(move || {
            let mut out = [0u8; 8];
            waiting.steal_blocking(&mut out, None)
        });
        std::thread::sleep(Duration::from_millis(50));
        blocking.close();
        assert!(matches!(waiter.join().expect("the waiter thread"), Err(BlockingWaitError::Closed)));
    }
}
