//! A Vyukov MPMC ring every process can wait on, whichever surface it
//! reaches SubEtha through.
//!
//! [`BlockingVyukovRing`] keeps a [`SharedRing`] together with the two
//! wakers its waiters park on. A push through it wakes one waiter for an
//! item and a pop wakes one waiter for room, so a waiter in C, Python,
//! PowerShell or Rust wakes on a push or pop made through any of them. A
//! push or pop made through the [`SharedRing`] itself wakes no one.
//!
//! The wakers sit beside the ring under the names in [`crate::blocking_wait`]:
//! `<path>.cwaker.bin` and `<path>.pwaker.bin` beside a file-backed ring,
//! `{name}_cwaker` and `{name}_pwaker` beside one in named shared memory.

use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

pub use crate::blocking_wait::{BlockingWaitError, MAX_WAITERS_DEFAULT};
use crate::blocking_wait::{deadline, file_wakers, shm_wakers, WaitState, WAKE_ANY};
use crate::cross_process_waker::CrossProcessWaker;
use crate::shared_ring::{RingError, SharedRing};
use crate::shm_file::ShmNamespace;

/// A [`SharedRing`] with the wakers its waiters share across processes.
pub struct BlockingVyukovRing {
    ring: Arc<SharedRing>,
    consumer_waker: Arc<CrossProcessWaker>,
    producer_waker: Arc<CrossProcessWaker>,
    state: WaitState,
}

impl BlockingVyukovRing {
    /// `ring` with its wakers in this process alone, for an anonymous
    /// ring; each waker has `max_waiters` slots.
    pub fn anon(ring: Arc<SharedRing>, max_waiters: usize) -> Result<Self, BlockingWaitError<RingError>> {
        let consumer_waker = CrossProcessWaker::create_anon(max_waiters)?;
        let producer_waker = CrossProcessWaker::create_anon(max_waiters)?;
        Ok(Self::from_parts(ring, consumer_waker, producer_waker))
    }

    /// `ring` with its wakers in the files `<path>.cwaker.bin` and
    /// `<path>.pwaker.bin`, made when they do not exist and attached to
    /// when they do. `path` is the ring's own file.
    pub fn beside_file(ring: Arc<SharedRing>, path: impl AsRef<Path>, max_waiters: usize) -> Result<Self, BlockingWaitError<RingError>> {
        let (consumer_waker, producer_waker) = file_wakers(path.as_ref(), max_waiters)?;
        Ok(Self::from_parts(ring, consumer_waker, producer_waker))
    }

    /// `ring` with its wakers in the named shared-memory regions
    /// `{name}_cwaker` and `{name}_pwaker`, made with `sddl` as their
    /// security descriptor when `create` and opened otherwise. Made
    /// regions' names go with this handle.
    pub fn beside_shm(
        ring: Arc<SharedRing>,
        name: &str,
        namespace: ShmNamespace,
        create: bool,
        sddl: Option<&str>,
        max_waiters: usize,
    ) -> Result<Self, BlockingWaitError<RingError>> {
        let (consumer_waker, producer_waker) = shm_wakers(name, namespace, create, sddl, max_waiters)?;
        Ok(Self::from_parts(ring, consumer_waker, producer_waker))
    }

    /// `ring` with wakers its caller made. Waiters in other processes wake
    /// only when these are the ones beside the ring under the names above.
    pub fn from_parts(ring: Arc<SharedRing>, consumer_waker: CrossProcessWaker, producer_waker: CrossProcessWaker) -> Self {
        Self {
            ring,
            consumer_waker: Arc::new(consumer_waker),
            producer_waker: Arc::new(producer_waker),
            state: WaitState::default(),
        }
    }

    /// The ring itself, for everything that neither pushes nor pops.
    pub fn ring(&self) -> &Arc<SharedRing> {
        &self.ring
    }

    /// The waker consumers park on while the ring is empty.
    pub fn consumer_waker(&self) -> &Arc<CrossProcessWaker> {
        &self.consumer_waker
    }

    /// The waker producers park on while the ring is full.
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
    /// same ring wake, find the ring as it was and wait again.
    pub fn close(&self) {
        self.state.close();
        self.consumer_waker.wake_all();
        self.producer_waker.wake_all();
    }

    /// [`SharedRing::try_push`], then wake one waiter for an item.
    pub fn try_push(&self, payload: &[u8]) -> Result<(), RingError> {
        self.ring.try_push(payload)?;
        self.consumer_waker.wake_one_up_to(WAKE_ANY);
        Ok(())
    }

    /// [`SharedRing::try_pop`], then wake one waiter for room.
    pub fn try_pop(&self, out: &mut [u8]) -> Result<usize, RingError> {
        let n = self.ring.try_pop(out)?;
        self.producer_waker.wake_one_up_to(WAKE_ANY);
        Ok(n)
    }

    /// Push, parking until a slot is free or `timeout` has passed; `None`
    /// waits as long as it takes.
    pub fn push_blocking(&self, payload: &[u8], timeout: Option<Duration>) -> Result<(), BlockingWaitError<RingError>> {
        self.state.wait(&self.producer_waker, deadline(timeout), |e| matches!(e, RingError::Full), || {
            self.try_push(payload)
        })
    }

    /// Pop, parking until there is an item or `timeout` has passed; `None`
    /// waits as long as it takes.
    pub fn pop_blocking(&self, out: &mut [u8], timeout: Option<Duration>) -> Result<usize, BlockingWaitError<RingError>> {
        self.state.wait(&self.consumer_waker, deadline(timeout), |e| matches!(e, RingError::Empty), || {
            self.try_pop(out)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shared_ring::PAYLOAD_BYTES;
    use std::path::PathBuf;
    use std::time::Instant;

    /// A folder of its own under the temporary directory, removed when the
    /// test ends; declared before the rings so it drops after them.
    struct Folder(PathBuf);

    impl Folder {
        fn new(name: &str) -> Self {
            let dir = std::env::temp_dir().join(format!(
                "subetha-blocking-vyukov-{name}-{}-{}",
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
            self.0.join("ring")
        }
    }

    impl Drop for Folder {
        fn drop(&mut self) {
            if let Err(e) = std::fs::remove_dir_all(&self.0) {
                eprintln!("could not remove {}: {e}", self.0.display());
            }
        }
    }

    /// A handle on the ring of four slots at `path`: the first call makes
    /// it, and later ones attach as another process would.
    fn handle(path: &Path) -> BlockingVyukovRing {
        let ring = if path.exists() { SharedRing::open(path, 4) } else { SharedRing::create(path, 4) };
        BlockingVyukovRing::beside_file(Arc::new(ring.expect("make or attach to the ring")), path, MAX_WAITERS_DEFAULT)
            .expect("its wakers")
    }

    /// Push until the ring refuses, so the next push must wait for room.
    fn fill(ring: &BlockingVyukovRing) {
        loop {
            match ring.try_push(b"x") {
                Ok(()) => {}
                Err(RingError::Full) => break,
                Err(e) => panic!("filling the ring: {e}"),
            }
        }
    }

    /// A thread pushing onto a full ring through `ring`, which reports what
    /// its push came to and how long it took, returned once its wait has
    /// begun.
    #[allow(clippy::type_complexity)]
    fn waiting_pusher(
        ring: &Arc<BlockingVyukovRing>,
        timeout: Duration,
    ) -> std::thread::JoinHandle<(Result<(), BlockingWaitError<RingError>>, Duration)> {
        let waiting = Arc::clone(ring);
        let returned = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let pusher_returned = Arc::clone(&returned);
        let pusher = std::thread::spawn(move || {
            let began = Instant::now();
            let pushed = waiting.push_blocking(b"late", Some(timeout));
            pusher_returned.store(true, std::sync::atomic::Ordering::Release);
            (pushed, began.elapsed())
        });
        ring.producer_waker().until_a_wait_begins(&returned);
        pusher
    }

    #[test]
    fn a_wait_for_an_item_times_out_and_takes_one_once_one_is_there() {
        let ring = Arc::new(SharedRing::create_anon(4).expect("make the ring"));
        let blocking = BlockingVyukovRing::anon(ring, MAX_WAITERS_DEFAULT).expect("its wakers");
        let mut out = [0u8; PAYLOAD_BYTES];
        let began = Instant::now();
        assert!(matches!(
            blocking.pop_blocking(&mut out, Some(Duration::from_millis(50))),
            Err(BlockingWaitError::Timeout)
        ));
        assert!(began.elapsed() >= Duration::from_millis(40), "the wait lasted its timeout");
        blocking.try_push(b"one").expect("push");
        blocking.pop_blocking(&mut out, Some(Duration::from_secs(5))).expect("pop");
        assert_eq!(&out[..3], b"one");
    }

    #[test]
    fn a_push_through_another_handle_on_the_file_wakes_a_waiting_popper() {
        let folder = Folder::new("wake");
        let path = folder.path();
        let popping = Arc::new(handle(&path));
        let pushing = handle(&path);
        let waiting = Arc::clone(&popping);
        let waiter = std::thread::spawn(move || {
            let mut out = [0u8; PAYLOAD_BYTES];
            let began = Instant::now();
            let taken = waiting.pop_blocking(&mut out, Some(Duration::from_secs(20)));
            (taken, out, began.elapsed())
        });
        std::thread::sleep(Duration::from_millis(100));
        pushing.try_push(b"pushed").expect("push");
        let (taken, item, waited) = waiter.join().expect("the waiter thread");
        taken.expect("the woken pop takes the item");
        assert_eq!(&item[..6], b"pushed");
        assert!(waited < Duration::from_secs(10), "woken by the push, not by the timeout: {waited:?}");
    }

    #[test]
    fn a_pop_through_another_handle_wakes_a_pusher_waiting_for_room() {
        let folder = Folder::new("room");
        let path = folder.path();
        let pushing = Arc::new(handle(&path));
        let popping = handle(&path);
        fill(&pushing);
        let waiter = waiting_pusher(&pushing, Duration::from_secs(20));
        let mut out = [0u8; PAYLOAD_BYTES];
        popping.try_pop(&mut out).expect("make room");
        let (pushed, waited) = waiter.join().expect("the pusher thread");
        pushed.expect("the woken push lands in the freed slot");
        assert!(waited < Duration::from_secs(10), "woken by the pop, not by the timeout: {waited:?}");
    }

    #[test]
    fn a_pop_through_the_ring_alone_leaves_that_pusher_asleep() {
        let folder = Folder::new("alone");
        let path = folder.path();
        let pushing = Arc::new(handle(&path));
        let popping = handle(&path);
        fill(&pushing);
        let timeout = Duration::from_millis(600);
        let waiter = waiting_pusher(&pushing, timeout);
        let mut out = [0u8; PAYLOAD_BYTES];
        popping.ring().try_pop(&mut out).expect("make room without waking anyone");
        let (pushed, waited) = waiter.join().expect("the pusher thread");
        // Its wait ends at the timeout, and the try that follows finds the
        // room the silent pop made.
        pushed.expect("the push lands once its wait ends");
        assert!(waited >= timeout - Duration::from_millis(50), "slept until its timeout: {waited:?}");
    }

    #[test]
    fn closing_ends_a_wait_on_that_handle() {
        let ring = Arc::new(SharedRing::create_anon(4).expect("make the ring"));
        let blocking = Arc::new(BlockingVyukovRing::anon(ring, MAX_WAITERS_DEFAULT).expect("its wakers"));
        let waiting = Arc::clone(&blocking);
        let waiter = std::thread::spawn(move || {
            let mut out = [0u8; PAYLOAD_BYTES];
            waiting.pop_blocking(&mut out, None)
        });
        std::thread::sleep(Duration::from_millis(50));
        blocking.close();
        assert!(matches!(waiter.join().expect("the waiter thread"), Err(BlockingWaitError::Closed)));
    }
}
