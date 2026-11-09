//! A broadcast ring every process can wait on, whichever surface it
//! reaches SubEtha through.
//!
//! [`BlockingBroadcastRing`] keeps a [`SharedBroadcastRing`] together with
//! the two wakers its waiters park on. A push through it wakes every
//! parked consumer, since every registered consumer wants each item; a
//! receive through it wakes one parked producer. So a waiter in C,
//! Python, PowerShell or Rust wakes on a push or receive made through any
//! of them. A push or receive made through the [`SharedBroadcastRing`]
//! itself wakes no one.
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
use crate::shared_broadcast_ring::{BroadcastError, SharedBroadcastRing};
use crate::shm_file::ShmNamespace;

/// A [`SharedBroadcastRing`] with the wakers its waiters share across
/// processes.
pub struct BlockingBroadcastRing {
    ring: Arc<SharedBroadcastRing>,
    consumer_waker: Arc<CrossProcessWaker>,
    producer_waker: Arc<CrossProcessWaker>,
    state: WaitState,
}

impl BlockingBroadcastRing {
    /// `ring` with its wakers in this process alone, for an anonymous
    /// ring; each waker has `max_waiters` slots.
    pub fn anon(ring: Arc<SharedBroadcastRing>, max_waiters: usize) -> Result<Self, BlockingWaitError<BroadcastError>> {
        let consumer_waker = CrossProcessWaker::create_anon(max_waiters)?;
        let producer_waker = CrossProcessWaker::create_anon(max_waiters)?;
        Ok(Self::from_parts(ring, consumer_waker, producer_waker))
    }

    /// `ring` with its wakers in the files `<path>.cwaker.bin` and
    /// `<path>.pwaker.bin`, made when they do not exist and attached to
    /// when they do. `path` is the ring's own file.
    pub fn beside_file(
        ring: Arc<SharedBroadcastRing>,
        path: impl AsRef<Path>,
        max_waiters: usize,
    ) -> Result<Self, BlockingWaitError<BroadcastError>> {
        let (consumer_waker, producer_waker) = file_wakers(path.as_ref(), max_waiters)?;
        Ok(Self::from_parts(ring, consumer_waker, producer_waker))
    }

    /// `ring` with its wakers in the named shared-memory regions
    /// `{name}_cwaker` and `{name}_pwaker`, made with `sddl` as their
    /// security descriptor when `create` and opened otherwise. Made
    /// regions' names go with this handle.
    pub fn beside_shm(
        ring: Arc<SharedBroadcastRing>,
        name: &str,
        namespace: ShmNamespace,
        create: bool,
        sddl: Option<&str>,
        max_waiters: usize,
    ) -> Result<Self, BlockingWaitError<BroadcastError>> {
        let (consumer_waker, producer_waker) = shm_wakers(name, namespace, create, sddl, max_waiters)?;
        Ok(Self::from_parts(ring, consumer_waker, producer_waker))
    }

    /// `ring` with wakers its caller made. Waiters in other processes wake
    /// only when these are the ones beside the ring under the names above.
    pub fn from_parts(ring: Arc<SharedBroadcastRing>, consumer_waker: CrossProcessWaker, producer_waker: CrossProcessWaker) -> Self {
        Self {
            ring,
            consumer_waker: Arc::new(consumer_waker),
            producer_waker: Arc::new(producer_waker),
            state: WaitState::default(),
        }
    }

    /// The ring itself, for everything that neither pushes nor receives.
    pub fn ring(&self) -> &Arc<SharedBroadcastRing> {
        &self.ring
    }

    /// The waker consumers park on while they have nothing to read.
    pub fn consumer_waker(&self) -> &Arc<CrossProcessWaker> {
        &self.consumer_waker
    }

    /// The waker the producer parks on while the slowest consumer is a lap
    /// behind.
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
    /// [`BlockingWaitError::Closed`]. Waits through other handles on the same ring
    /// wake, find the ring as it was and wait again.
    pub fn close(&self) {
        self.state.close();
        self.consumer_waker.wake_all();
        self.producer_waker.wake_all();
    }

    /// [`SharedBroadcastRing::try_push`], then wake every parked consumer.
    pub fn try_push(&self, payload: &[u8]) -> Result<(), BroadcastError> {
        self.ring.try_push(payload)?;
        self.consumer_waker.wake_all();
        Ok(())
    }

    /// [`SharedBroadcastRing::try_recv`] for `consumer`, then wake one
    /// parked producer.
    pub fn try_recv(&self, consumer: usize, out: &mut [u8]) -> Result<usize, BroadcastError> {
        let n = self.ring.try_recv(consumer, out)?;
        self.producer_waker.wake_one_up_to(WAKE_ANY);
        Ok(n)
    }

    /// [`SharedBroadcastRing::unregister_consumer`], then wake one parked
    /// producer: the consumer given back may be the one a full ring was
    /// holding its slot for.
    pub fn unregister_consumer(&self, consumer: usize) {
        self.ring.unregister_consumer(consumer);
        self.producer_waker.wake_one_up_to(WAKE_ANY);
    }

    /// Push, parking while the slowest registered consumer is a lap behind,
    /// until `timeout` has passed; `None` waits as long as it takes.
    pub fn push_blocking(&self, payload: &[u8], timeout: Option<Duration>) -> Result<(), BlockingWaitError<BroadcastError>> {
        self.state.wait(&self.producer_waker, deadline(timeout), |e| matches!(e, BroadcastError::Full), || {
            self.try_push(payload)
        })
    }

    /// Receive for `consumer`, parking until an item arrives or `timeout`
    /// has passed; `None` waits as long as it takes.
    pub fn recv_blocking(
        &self,
        consumer: usize,
        out: &mut [u8],
        timeout: Option<Duration>,
    ) -> Result<usize, BlockingWaitError<BroadcastError>> {
        self.state.wait(&self.consumer_waker, deadline(timeout), |e| matches!(e, BroadcastError::Empty), || {
            self.try_recv(consumer, out)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shared_broadcast_ring::BROADCAST_PAYLOAD_BYTES;
    use std::path::PathBuf;
    use std::time::Instant;

    /// A file path of its own under the temporary directory, whose ring and
    /// waker files are removed when the test ends; declared before the
    /// rings so it drops after them.
    struct Scratch(PathBuf);

    impl Scratch {
        fn new(name: &str) -> Self {
            let path = std::env::temp_dir().join(format!(
                "subetha-blocking-broadcast-{name}-{}-{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .expect("the wall clock is after the epoch")
                    .as_nanos()
            ));
            Self(path)
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            for suffix in ["", ".cwaker.bin", ".pwaker.bin"] {
                let file = crate::blocking_wait::suffixed(&self.0, suffix);
                if let Err(e) = std::fs::remove_file(&file)
                    && e.kind() != std::io::ErrorKind::NotFound
                {
                    eprintln!("could not remove {}: {e}", file.display());
                }
            }
        }
    }

    /// A handle on the file-backed ring at `path`, made or attached to as
    /// another process would.
    fn handle(path: &Path, create: bool) -> BlockingBroadcastRing {
        let ring = if create {
            SharedBroadcastRing::create(path, 4).expect("make the ring")
        } else {
            SharedBroadcastRing::open(path, 4).expect("attach to the ring")
        };
        BlockingBroadcastRing::beside_file(Arc::new(ring), path, MAX_WAITERS_DEFAULT).expect("its wakers")
    }

    /// Push until the ring refuses, so the next push must wait for room.
    fn fill(ring: &BlockingBroadcastRing) {
        loop {
            match ring.try_push(b"x") {
                Ok(()) => {}
                Err(BroadcastError::Full) => break,
                Err(e) => panic!("filling the ring: {e}"),
            }
        }
    }

    #[test]
    fn a_wait_with_nothing_to_read_times_out_and_takes_an_item_once_one_is_there() {
        let ring = SharedBroadcastRing::create_anon(4).expect("an anonymous ring");
        let blocking = BlockingBroadcastRing::anon(Arc::new(ring), MAX_WAITERS_DEFAULT).expect("its wakers");
        let consumer = blocking.ring().register_consumer().expect("a consumer");
        let mut out = [0u8; BROADCAST_PAYLOAD_BYTES];
        let began = Instant::now();
        assert!(matches!(
            blocking.recv_blocking(consumer, &mut out, Some(Duration::from_millis(50))),
            Err(BlockingWaitError::Timeout)
        ));
        assert!(began.elapsed() >= Duration::from_millis(40), "the wait lasted its timeout");
        blocking.try_push(b"one").expect("push");
        blocking.recv_blocking(consumer, &mut out, Some(Duration::from_secs(5))).expect("receive");
        assert_eq!(&out[..3], b"one");
    }

    #[test]
    fn a_push_through_another_handle_wakes_every_waiting_consumer() {
        let scratch = Scratch::new("wake");
        let receiving = Arc::new(handle(&scratch.0, true));
        let first = receiving.ring().register_consumer().expect("a consumer");
        let second = receiving.ring().register_consumer().expect("a second consumer");
        let waiters: Vec<_> = [first, second]
            .into_iter()
            .map(|consumer| {
                let waiting = Arc::clone(&receiving);
                std::thread::spawn(move || {
                    let mut out = [0u8; BROADCAST_PAYLOAD_BYTES];
                    let began = Instant::now();
                    waiting.recv_blocking(consumer, &mut out, Some(Duration::from_secs(20))).expect("receive");
                    (out, began.elapsed())
                })
            })
            .collect();
        std::thread::sleep(Duration::from_millis(100));
        let sending = handle(&scratch.0, false);
        let sent = b"from the other handle";
        sending.try_push(sent).expect("push");
        for waiter in waiters {
            let (item, waited) = waiter.join().expect("a waiter thread");
            assert_eq!(&item[..sent.len()], sent);
            assert!(waited < Duration::from_secs(10), "woken by the push, not by the timeout: {waited:?}");
        }
    }

    #[test]
    fn a_receive_through_another_handle_wakes_a_producer_waiting_for_room() {
        let scratch = Scratch::new("room");
        let sending = Arc::new(handle(&scratch.0, true));
        let receiving = handle(&scratch.0, false);
        let consumer = receiving.ring().register_consumer().expect("a consumer");
        fill(&sending);
        let waiting = Arc::clone(&sending);
        let producer = std::thread::spawn(move || {
            let began = Instant::now();
            let pushed = waiting.push_blocking(b"late", Some(Duration::from_secs(20)));
            (pushed, began.elapsed())
        });
        std::thread::sleep(Duration::from_millis(100));
        let mut out = [0u8; BROADCAST_PAYLOAD_BYTES];
        receiving.try_recv(consumer, &mut out).expect("make room");
        let (pushed, waited) = producer.join().expect("the producer thread");
        pushed.expect("the woken push lands in the freed slot");
        assert!(waited < Duration::from_secs(10), "woken by the receive, not by the timeout: {waited:?}");
    }

    #[test]
    fn giving_back_the_consumer_a_full_ring_waits_for_wakes_its_producer() {
        let scratch = Scratch::new("give-back");
        let sending = Arc::new(handle(&scratch.0, true));
        let stalled = handle(&scratch.0, false);
        let consumer = stalled.ring().register_consumer().expect("a consumer");
        fill(&sending);
        let waiting = Arc::clone(&sending);
        let producer = std::thread::spawn(move || {
            let began = Instant::now();
            let pushed = waiting.push_blocking(b"late", Some(Duration::from_secs(20)));
            (pushed, began.elapsed())
        });
        std::thread::sleep(Duration::from_millis(100));
        stalled.unregister_consumer(consumer);
        let (pushed, waited) = producer.join().expect("the producer thread");
        pushed.expect("with no consumer left the push lands");
        assert!(waited < Duration::from_secs(10), "woken by the give-back, not by the timeout: {waited:?}");
    }

    #[test]
    fn a_receive_through_the_ring_alone_leaves_that_producer_asleep() {
        let scratch = Scratch::new("alone");
        let sending = Arc::new(handle(&scratch.0, true));
        let consumer = sending.ring().register_consumer().expect("a consumer");
        fill(&sending);
        let timeout = Duration::from_millis(600);
        let waiting = Arc::clone(&sending);
        let returned = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let producer_returned = Arc::clone(&returned);
        let producer = std::thread::spawn(move || {
            let began = Instant::now();
            let pushed = waiting.push_blocking(b"late", Some(timeout));
            producer_returned.store(true, std::sync::atomic::Ordering::Release);
            (pushed, began.elapsed())
        });
        sending.producer_waker().until_a_wait_begins(&returned);
        let mut out = [0u8; BROADCAST_PAYLOAD_BYTES];
        sending.ring().try_recv(consumer, &mut out).expect("make room without waking anyone");
        let (pushed, waited) = producer.join().expect("the producer thread");
        // Its wait ends at the timeout, and the try that follows finds the
        // room the silent receive made.
        pushed.expect("the push lands once its wait ends");
        assert!(waited >= timeout - Duration::from_millis(50), "slept until its timeout: {waited:?}");
    }

    #[test]
    fn closing_ends_a_wait_on_that_handle() {
        let ring = SharedBroadcastRing::create_anon(4).expect("an anonymous ring");
        let blocking = Arc::new(BlockingBroadcastRing::anon(Arc::new(ring), MAX_WAITERS_DEFAULT).expect("its wakers"));
        let consumer = blocking.ring().register_consumer().expect("a consumer");
        let waiting = Arc::clone(&blocking);
        let waiter = std::thread::spawn(move || {
            let mut out = [0u8; BROADCAST_PAYLOAD_BYTES];
            waiting.recv_blocking(consumer, &mut out, None)
        });
        std::thread::sleep(Duration::from_millis(50));
        blocking.close();
        assert!(matches!(waiter.join().expect("the waiter thread"), Err(BlockingWaitError::Closed)));
        let mut out = [0u8; BROADCAST_PAYLOAD_BYTES];
        assert!(matches!(blocking.recv_blocking(consumer, &mut out, None), Err(BlockingWaitError::Closed)));
    }
}
