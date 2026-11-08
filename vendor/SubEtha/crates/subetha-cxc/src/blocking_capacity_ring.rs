//! A capacity-adaptive ring every process can wait on, whichever surface
//! it reaches SubEtha through.
//!
//! [`BlockingCapacityRing`] keeps a [`CapacityAdaptiveRing`] together with
//! the two wakers its waiters park on. A send through it wakes one parked
//! consumer and a receive wakes one parked producer, so a waiter in C,
//! Python, PowerShell or Rust wakes on a send or receive made through any
//! of them. A send or receive made through the [`CapacityAdaptiveRing`]
//! itself wakes no one. A morph wakes every waiter on both wakers, whether
//! the caller, the capacity sidecar or a registration in this process made
//! it: a capacity morph starts an empty backing, and a backing's shape
//! morph sends producers into a backing that can have room.
//!
//! The wakers sit beside the ring under the names in [`crate::blocking_wait`]:
//! `<base>.cwaker.bin` and `<base>.pwaker.bin` beside a file-backed ring
//! made under `<base>`, `{prefix}_cwaker` and `{prefix}_pwaker` beside one
//! in named shared memory.

use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

pub use crate::blocking_wait::{BlockingWaitError, MAX_WAITERS_DEFAULT};
use crate::blocking_wait::{deadline, file_wakers, shm_wakers, WaitState, WAKE_ANY};
use crate::capacity_adaptive_ring::CapacityAdaptiveRing;
use crate::cross_process_waker::CrossProcessWaker;
use crate::shared_ring::RingError;
use crate::shm_file::ShmNamespace;

/// A [`CapacityAdaptiveRing`] with the wakers its waiters share across
/// processes.
pub struct BlockingCapacityRing {
    ring: Arc<CapacityAdaptiveRing>,
    consumer_waker: Arc<CrossProcessWaker>,
    producer_waker: Arc<CrossProcessWaker>,
    state: WaitState,
}

impl BlockingCapacityRing {
    /// `ring` with its wakers in this process alone, for an anonymous
    /// ring; each waker has `max_waiters` slots.
    pub fn anon(ring: Arc<CapacityAdaptiveRing>, max_waiters: usize) -> Result<Self, BlockingWaitError<RingError>> {
        let consumer_waker = CrossProcessWaker::create_anon(max_waiters)?;
        let producer_waker = CrossProcessWaker::create_anon(max_waiters)?;
        Ok(Self::from_parts(ring, consumer_waker, producer_waker))
    }

    /// `ring` with its wakers in the files `<base>.cwaker.bin` and
    /// `<base>.pwaker.bin`, made when they do not exist and attached to
    /// when they do. `base` is the one the ring was made under.
    pub fn beside_file(
        ring: Arc<CapacityAdaptiveRing>,
        base: impl AsRef<Path>,
        max_waiters: usize,
    ) -> Result<Self, BlockingWaitError<RingError>> {
        let (consumer_waker, producer_waker) = file_wakers(base.as_ref(), max_waiters)?;
        Ok(Self::from_parts(ring, consumer_waker, producer_waker))
    }

    /// `ring` with its wakers in the named shared-memory regions
    /// `{prefix}_cwaker` and `{prefix}_pwaker`, made with `sddl` as their
    /// security descriptor when `create` and opened otherwise. Made
    /// regions' names go with this handle.
    pub fn beside_shm(
        ring: Arc<CapacityAdaptiveRing>,
        prefix: &str,
        namespace: ShmNamespace,
        create: bool,
        sddl: Option<&str>,
        max_waiters: usize,
    ) -> Result<Self, BlockingWaitError<RingError>> {
        let (consumer_waker, producer_waker) = shm_wakers(prefix, namespace, create, sddl, max_waiters)?;
        Ok(Self::from_parts(ring, consumer_waker, producer_waker))
    }

    /// `ring` with wakers its caller made. Waiters in other processes wake
    /// only when these are the ones beside the ring under the names above.
    /// Every morph of `ring` from here on wakes every waiter on both
    /// wakers.
    pub fn from_parts(ring: Arc<CapacityAdaptiveRing>, consumer_waker: CrossProcessWaker, producer_waker: CrossProcessWaker) -> Self {
        let consumer_waker = Arc::new(consumer_waker);
        let producer_waker = Arc::new(producer_waker);
        ring.watch_moves(&consumer_waker, &producer_waker);
        Self {
            ring,
            consumer_waker,
            producer_waker,
            state: WaitState::default(),
        }
    }

    /// The ring itself, for everything that neither sends nor receives.
    pub fn ring(&self) -> &Arc<CapacityAdaptiveRing> {
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

    /// [`CapacityAdaptiveRing::try_send`], then wake one parked consumer.
    pub fn try_send(&self, producer: usize, payload: &[u8]) -> Result<(), RingError> {
        self.ring.try_send(producer, payload)?;
        self.consumer_waker.wake_one_up_to(WAKE_ANY);
        Ok(())
    }

    /// [`CapacityAdaptiveRing::try_recv`], then wake one parked producer.
    pub fn try_recv(&self, consumer: usize, out: &mut [u8]) -> Result<usize, RingError> {
        let n = self.ring.try_recv(consumer, out)?;
        self.producer_waker.wake_one_up_to(WAKE_ANY);
        Ok(n)
    }

    /// Send, parking until there is room or `timeout` has passed; `None`
    /// waits as long as it takes.
    pub fn send_blocking(&self, producer: usize, payload: &[u8], timeout: Option<Duration>) -> Result<(), BlockingWaitError<RingError>> {
        self.state.wait(&self.producer_waker, deadline(timeout), |e| matches!(e, RingError::Full), || {
            self.try_send(producer, payload)
        })
    }

    /// Receive, parking until an item arrives or `timeout` has passed;
    /// `None` waits as long as it takes.
    pub fn recv_blocking(
        &self,
        consumer: usize,
        out: &mut [u8],
        timeout: Option<Duration>,
    ) -> Result<usize, BlockingWaitError<RingError>> {
        self.state.wait(&self.consumer_waker, deadline(timeout), |e| matches!(e, RingError::Empty), || {
            self.try_recv(consumer, out)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adaptive_ring::ADAPTIVE_SPSC_PAYLOAD_BYTES;
    use std::path::PathBuf;
    use std::time::Instant;

    /// A folder of its own under the temporary directory, removed when the
    /// test ends; declared before the rings so it drops after them.
    struct Folder(PathBuf);

    impl Folder {
        fn new(name: &str) -> Self {
            let dir = std::env::temp_dir().join(format!(
                "subetha-blocking-capacity-{name}-{}-{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .expect("the wall clock is after the epoch")
                    .as_nanos()
            ));
            std::fs::create_dir_all(&dir).expect("make the test's folder");
            Self(dir)
        }

        fn base(&self) -> PathBuf {
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

    /// A handle on the file-backed ring at `base`, made or attached to as
    /// another process would.
    fn handle(base: &Path, create: bool) -> BlockingCapacityRing {
        let ring = if create {
            CapacityAdaptiveRing::create(base, 1, 1, 8).expect("make the ring")
        } else {
            CapacityAdaptiveRing::open(base, 1, 1, 8, false).expect("attach to the ring")
        };
        BlockingCapacityRing::beside_file(Arc::new(ring), base, MAX_WAITERS_DEFAULT).expect("its wakers")
    }

    /// Send until the ring refuses, so the next send must wait for room.
    fn fill(ring: &BlockingCapacityRing) {
        loop {
            match ring.try_send(0, b"x") {
                Ok(()) => {}
                Err(RingError::Full) => break,
                Err(e) => panic!("filling the ring: {e}"),
            }
        }
    }

    #[test]
    fn a_wait_on_an_empty_ring_times_out_and_takes_an_item_once_one_is_there() {
        let ring = CapacityAdaptiveRing::create_anon(1, 1, 8).expect("an anonymous ring");
        let blocking = BlockingCapacityRing::anon(Arc::new(ring), MAX_WAITERS_DEFAULT).expect("its wakers");
        let mut out = [0u8; ADAPTIVE_SPSC_PAYLOAD_BYTES];
        let began = Instant::now();
        assert!(matches!(
            blocking.recv_blocking(0, &mut out, Some(Duration::from_millis(50))),
            Err(BlockingWaitError::Timeout)
        ));
        assert!(began.elapsed() >= Duration::from_millis(40), "the wait lasted its timeout");
        blocking.try_send(0, b"one").expect("send");
        blocking.recv_blocking(0, &mut out, Some(Duration::from_secs(5))).expect("receive");
        assert_eq!(&out[..3], b"one");
    }

    #[test]
    fn a_send_through_another_handle_on_the_files_wakes_a_waiting_consumer() {
        let folder = Folder::new("wake");
        let base = folder.base();
        let receiving = Arc::new(handle(&base, true));
        let sending = handle(&base, false);
        let waiting = Arc::clone(&receiving);
        let waiter = std::thread::spawn(move || {
            let mut out = [0u8; ADAPTIVE_SPSC_PAYLOAD_BYTES];
            let began = Instant::now();
            let taken = waiting.recv_blocking(0, &mut out, Some(Duration::from_secs(20)));
            (taken, out, began.elapsed())
        });
        std::thread::sleep(Duration::from_millis(100));
        let sent = b"from the other handle";
        sending.try_send(0, sent).expect("send");
        let (taken, item, waited) = waiter.join().expect("the waiter thread");
        taken.expect("the woken receive takes the item");
        assert_eq!(&item[..sent.len()], sent);
        assert!(waited < Duration::from_secs(10), "woken by the send, not by the timeout: {waited:?}");
    }

    #[test]
    fn a_receive_through_another_handle_wakes_a_producer_waiting_for_room() {
        let folder = Folder::new("room");
        let base = folder.base();
        let sending = Arc::new(handle(&base, true));
        let receiving = handle(&base, false);
        fill(&sending);
        let waiting = Arc::clone(&sending);
        let producer = std::thread::spawn(move || {
            let began = Instant::now();
            let sent = waiting.send_blocking(0, b"late", Some(Duration::from_secs(20)));
            (sent, began.elapsed())
        });
        std::thread::sleep(Duration::from_millis(100));
        let mut out = [0u8; ADAPTIVE_SPSC_PAYLOAD_BYTES];
        receiving.try_recv(0, &mut out).expect("make room");
        let (sent, waited) = producer.join().expect("the producer thread");
        sent.expect("the woken send lands in the freed slot");
        assert!(waited < Duration::from_secs(10), "woken by the receive, not by the timeout: {waited:?}");
    }

    #[test]
    fn a_receive_through_the_ring_alone_leaves_that_producer_asleep() {
        let folder = Folder::new("alone");
        let base = folder.base();
        let sending = Arc::new(handle(&base, true));
        fill(&sending);
        let timeout = Duration::from_millis(600);
        let waiting = Arc::clone(&sending);
        let returned = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let producer_returned = Arc::clone(&returned);
        let producer = std::thread::spawn(move || {
            let began = Instant::now();
            let sent = waiting.send_blocking(0, b"late", Some(timeout));
            producer_returned.store(true, std::sync::atomic::Ordering::Release);
            (sent, began.elapsed())
        });
        sending.producer_waker().until_a_wait_begins(&returned);
        let mut out = [0u8; ADAPTIVE_SPSC_PAYLOAD_BYTES];
        sending.ring().try_recv(0, &mut out).expect("make room without waking anyone");
        let (sent, waited) = producer.join().expect("the producer thread");
        // Its wait ends at the timeout, and the try that follows finds the
        // room the silent receive made.
        sent.expect("the send lands once its wait ends");
        assert!(waited >= timeout - Duration::from_millis(50), "slept until its timeout: {waited:?}");
    }

    #[test]
    fn closing_ends_a_wait_on_that_handle() {
        let ring = CapacityAdaptiveRing::create_anon(1, 1, 8).expect("an anonymous ring");
        let blocking = Arc::new(BlockingCapacityRing::anon(Arc::new(ring), MAX_WAITERS_DEFAULT).expect("its wakers"));
        let waiting = Arc::clone(&blocking);
        let waiter = std::thread::spawn(move || {
            let mut out = [0u8; ADAPTIVE_SPSC_PAYLOAD_BYTES];
            waiting.recv_blocking(0, &mut out, None)
        });
        std::thread::sleep(Duration::from_millis(50));
        blocking.close();
        assert!(matches!(waiter.join().expect("the waiter thread"), Err(BlockingWaitError::Closed)));
    }

    /// A full anonymous ring and a thread sending into it, which reports
    /// what its send came to and how long it took.
    #[allow(clippy::type_complexity)]
    fn full_ring_with_a_waiting_sender() -> (
        Arc<BlockingCapacityRing>,
        std::thread::JoinHandle<(Result<(), BlockingWaitError<RingError>>, Duration)>,
    ) {
        let ring = CapacityAdaptiveRing::create_anon(1, 1, 8).expect("an anonymous ring");
        let blocking = Arc::new(BlockingCapacityRing::anon(Arc::new(ring), MAX_WAITERS_DEFAULT).expect("its wakers"));
        fill(&blocking);
        let waiting = Arc::clone(&blocking);
        let sender = std::thread::spawn(move || {
            let began = Instant::now();
            let sent = waiting.send_blocking(0, b"late", Some(Duration::from_secs(20)));
            (sent, began.elapsed())
        });
        std::thread::sleep(Duration::from_millis(100));
        (blocking, sender)
    }

    #[test]
    fn a_morph_through_the_ring_wakes_a_producer_waiting_for_room() {
        let (blocking, sender) = full_ring_with_a_waiting_sender();
        blocking.ring().morph_capacity_to(16).expect("grow the ring");
        let (sent, waited) = sender.join().expect("the sender thread");
        sent.expect("the woken send lands in the new backing");
        assert!(waited < Duration::from_secs(10), "woken by the morph, not by the timeout: {waited:?}");
    }

    #[test]
    fn the_capacity_sidecars_morph_wakes_a_producer_waiting_for_room() {
        use crate::capacity_adaptive_ring::{CapacityAdaptiveRingSidecar, DefaultCapacityPolicy};

        let (blocking, sender) = full_ring_with_a_waiting_sender();
        // A full ring is past the policy's grow threshold, so the sidecar
        // doubles it once its 100 ms cooldown has passed.
        let sidecar = CapacityAdaptiveRingSidecar::spawn(
            Arc::clone(blocking.ring()),
            DefaultCapacityPolicy::default(),
            Duration::from_millis(1),
        );
        let (sent, waited) = sender.join().expect("the sender thread");
        sidecar.shutdown();
        sent.expect("the woken send lands in the new backing");
        assert!(waited < Duration::from_secs(10), "woken by the sidecar's morph, not by the timeout: {waited:?}");
    }
}
