//! A locale-adaptive ring every process can wait on, whichever surface it
//! reaches SubEtha through.
//!
//! [`BlockingLocaleRing`] keeps a [`LocaleAdaptiveRing`] together with the
//! two wakers its waiters park on. A send through it wakes one parked
//! consumer and a receive wakes one parked producer, so a waiter in C,
//! Python, PowerShell or Rust wakes on a send or receive made through any
//! of them. A send or receive made through the [`LocaleAdaptiveRing`]
//! itself wakes no one.
//!
//! Each handle's anonymous backing is its own, so while the ring is in
//! the anonymous locale a waiter wakes on another handle's send but finds
//! nothing to take. A migration in this process wakes every waiter on both
//! wakers, whether the caller or the locale sidecar made it, because a
//! move into or out of the anonymous locale changes what a waiter in
//! another process can find; a backing's shape morph, which a
//! registration can make, wakes them the same way.
//!
//! The wakers sit in the files `<base>.cwaker.bin` and `<base>.pwaker.bin`
//! beside the ring made under `<base>`, where the C ABI keeps them,
//! whichever locale the ring is in.

use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

pub use crate::blocking_wait::{BlockingWaitError, MAX_WAITERS_DEFAULT};
use crate::blocking_wait::{deadline, file_wakers, WaitState, WAKE_ANY};
use crate::cross_process_waker::CrossProcessWaker;
use crate::locale_adaptive_ring::LocaleAdaptiveRing;
use crate::shared_ring::RingError;

/// A [`LocaleAdaptiveRing`] with the wakers its waiters share across
/// processes.
pub struct BlockingLocaleRing {
    ring: Arc<LocaleAdaptiveRing>,
    consumer_waker: Arc<CrossProcessWaker>,
    producer_waker: Arc<CrossProcessWaker>,
    state: WaitState,
}

impl BlockingLocaleRing {
    /// `ring` with its wakers in this process alone; each waker has
    /// `max_waiters` slots.
    pub fn anon(ring: Arc<LocaleAdaptiveRing>, max_waiters: usize) -> Result<Self, BlockingWaitError<RingError>> {
        let consumer_waker = CrossProcessWaker::create_anon(max_waiters)?;
        let producer_waker = CrossProcessWaker::create_anon(max_waiters)?;
        Ok(Self::from_parts(ring, consumer_waker, producer_waker))
    }

    /// `ring` with its wakers in the files `<base>.cwaker.bin` and
    /// `<base>.pwaker.bin`, made when they do not exist and attached to
    /// when they do. `base` is the one the ring was made under.
    pub fn beside_file(
        ring: Arc<LocaleAdaptiveRing>,
        base: impl AsRef<Path>,
        max_waiters: usize,
    ) -> Result<Self, BlockingWaitError<RingError>> {
        let (consumer_waker, producer_waker) = file_wakers(base.as_ref(), max_waiters)?;
        Ok(Self::from_parts(ring, consumer_waker, producer_waker))
    }

    /// `ring` with wakers its caller made. Waiters in other processes wake
    /// only when these are the ones beside the ring under the names above.
    /// Every migration of `ring` from here on wakes every waiter on both
    /// wakers.
    pub fn from_parts(ring: Arc<LocaleAdaptiveRing>, consumer_waker: CrossProcessWaker, producer_waker: CrossProcessWaker) -> Self {
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
    pub fn ring(&self) -> &Arc<LocaleAdaptiveRing> {
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

    /// [`LocaleAdaptiveRing::try_send`], then wake one parked consumer.
    pub fn try_send(&self, producer: usize, payload: &[u8]) -> Result<(), RingError> {
        self.ring.try_send(producer, payload)?;
        self.consumer_waker.wake_one_up_to(WAKE_ANY);
        Ok(())
    }

    /// [`LocaleAdaptiveRing::try_recv`], then wake one parked producer.
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
    use crate::locale_adaptive_ring::Locale;
    use std::path::PathBuf;
    use std::time::Instant;

    /// A folder of its own under the temporary directory, removed when the
    /// test ends; declared before the rings so it drops after them. The
    /// ring's base takes the folder's name, which no other test shares,
    /// because the ring names its shared-memory backing after the last
    /// component of its base.
    struct Folder(PathBuf);

    impl Folder {
        fn new(name: &str) -> Self {
            let dir = std::env::temp_dir().join(format!(
                "subetha_blocking_locale_{}_{}_{name}",
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
            self.0.join(self.0.file_name().expect("the folder has a name"))
        }
    }

    impl Drop for Folder {
        fn drop(&mut self) {
            if let Err(e) = std::fs::remove_dir_all(&self.0) {
                eprintln!("could not remove {}: {e}", self.0.display());
            }
        }
    }

    /// A handle on the ring at `base`, made or attached to as another
    /// process would.
    fn handle(base: &Path, create: bool) -> BlockingLocaleRing {
        let ring = if create {
            LocaleAdaptiveRing::create(base, 1, 1, 8).expect("make the ring")
        } else {
            LocaleAdaptiveRing::open(base, 1, 1, 8, false).expect("attach to the ring")
        };
        BlockingLocaleRing::beside_file(Arc::new(ring), base, MAX_WAITERS_DEFAULT).expect("its wakers")
    }

    /// Send until the ring refuses, so the next send must wait for room.
    fn fill(ring: &BlockingLocaleRing, producer: usize) {
        loop {
            match ring.try_send(producer, b"x") {
                Ok(()) => {}
                Err(RingError::Full) => break,
                Err(e) => panic!("filling the ring: {e}"),
            }
        }
    }

    #[test]
    fn a_wait_on_an_empty_ring_times_out_and_takes_an_item_once_one_is_there() {
        let folder = Folder::new("timeout");
        let ring = LocaleAdaptiveRing::create(folder.base(), 1, 1, 8).expect("make the ring");
        let blocking = BlockingLocaleRing::anon(Arc::new(ring), MAX_WAITERS_DEFAULT).expect("its wakers");
        let producer = blocking.ring().register_producer().expect("register a producer");
        let consumer = blocking.ring().register_consumer().expect("register a consumer");
        let mut out = [0u8; ADAPTIVE_SPSC_PAYLOAD_BYTES];
        let began = Instant::now();
        assert!(matches!(
            blocking.recv_blocking(consumer, &mut out, Some(Duration::from_millis(50))),
            Err(BlockingWaitError::Timeout)
        ));
        assert!(began.elapsed() >= Duration::from_millis(40), "the wait lasted its timeout");
        blocking.try_send(producer, b"one").expect("send");
        blocking.recv_blocking(consumer, &mut out, Some(Duration::from_secs(5))).expect("receive");
        assert_eq!(&out[..3], b"one");
    }

    #[test]
    fn a_send_through_another_handle_on_the_files_wakes_a_waiting_consumer() {
        let folder = Folder::new("wake");
        let base = folder.base();
        let receiving = Arc::new(handle(&base, true));
        let sending = handle(&base, false);
        receiving.ring().migrate_to(Locale::File).expect("move to a locale both handles share");
        let consumer = receiving.ring().register_consumer().expect("register a consumer");
        let producer = sending.ring().register_producer().expect("register a producer");
        let waiting = Arc::clone(&receiving);
        let waiter = std::thread::spawn(move || {
            let mut out = [0u8; ADAPTIVE_SPSC_PAYLOAD_BYTES];
            let began = Instant::now();
            let taken = waiting.recv_blocking(consumer, &mut out, Some(Duration::from_secs(20)));
            (taken, out, began.elapsed())
        });
        std::thread::sleep(Duration::from_millis(100));
        let sent = b"from the other handle";
        sending.try_send(producer, sent).expect("send");
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
        sending.ring().migrate_to(Locale::File).expect("move to a locale both handles share");
        let producer = sending.ring().register_producer().expect("register a producer");
        let consumer = receiving.ring().register_consumer().expect("register a consumer");
        fill(&sending, producer);
        let waiting = Arc::clone(&sending);
        let waiter = std::thread::spawn(move || {
            let began = Instant::now();
            let sent = waiting.send_blocking(producer, b"late", Some(Duration::from_secs(20)));
            (sent, began.elapsed())
        });
        std::thread::sleep(Duration::from_millis(100));
        let mut out = [0u8; ADAPTIVE_SPSC_PAYLOAD_BYTES];
        receiving.try_recv(consumer, &mut out).expect("make room");
        let (sent, waited) = waiter.join().expect("the producer thread");
        sent.expect("the woken send lands in the freed slot");
        assert!(waited < Duration::from_secs(10), "woken by the receive, not by the timeout: {waited:?}");
    }

    #[test]
    fn a_receive_through_the_ring_alone_leaves_that_producer_asleep() {
        let folder = Folder::new("alone");
        let base = folder.base();
        let sending = Arc::new(handle(&base, true));
        let producer = sending.ring().register_producer().expect("register a producer");
        let consumer = sending.ring().register_consumer().expect("register a consumer");
        fill(&sending, producer);
        let timeout = Duration::from_millis(600);
        let waiting = Arc::clone(&sending);
        let returned = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let waiter_returned = Arc::clone(&returned);
        let waiter = std::thread::spawn(move || {
            let began = Instant::now();
            let sent = waiting.send_blocking(producer, b"late", Some(timeout));
            waiter_returned.store(true, std::sync::atomic::Ordering::Release);
            (sent, began.elapsed())
        });
        sending.producer_waker().until_a_wait_begins(&returned);
        let mut out = [0u8; ADAPTIVE_SPSC_PAYLOAD_BYTES];
        sending.ring().try_recv(consumer, &mut out).expect("make room without waking anyone");
        let (sent, waited) = waiter.join().expect("the producer thread");
        // Its wait ends at the timeout, and the try that follows finds the
        // room the silent receive made.
        sent.expect("the send lands once its wait ends");
        assert!(waited >= timeout - Duration::from_millis(50), "slept until its timeout: {waited:?}");
    }

    /// What a waiting receive came to, the item and how long it waited.
    type Received = (Result<usize, BlockingWaitError<RingError>>, [u8; ADAPTIVE_SPSC_PAYLOAD_BYTES], Duration);

    /// Two handles on the ring at `base` in the anonymous locale, an item
    /// sent through the ring alone into the first handle's own anonymous
    /// backing, and a thread receiving through the second, which cannot
    /// see that backing and waits.
    fn an_item_the_other_handle_cannot_see(base: &Path) -> (BlockingLocaleRing, std::thread::JoinHandle<Received>) {
        let sending = handle(base, true);
        let receiving = Arc::new(handle(base, false));
        let producer = sending.ring().register_producer().expect("register a producer");
        let consumer = receiving.ring().register_consumer().expect("register a consumer");
        sending.ring().try_send(producer, b"carried").expect("send without waking anyone");
        let receiver = std::thread::spawn(move || {
            let mut out = [0u8; ADAPTIVE_SPSC_PAYLOAD_BYTES];
            let began = Instant::now();
            let taken = receiving.recv_blocking(consumer, &mut out, Some(Duration::from_secs(20)));
            (taken, out, began.elapsed())
        });
        std::thread::sleep(Duration::from_millis(100));
        (sending, receiver)
    }

    #[test]
    fn a_migration_through_the_ring_wakes_a_consumer_on_another_handle() {
        let folder = Folder::new("migrate");
        let (sending, receiver) = an_item_the_other_handle_cannot_see(&folder.base());
        sending.ring().migrate_to(Locale::File).expect("move to a locale both handles share");
        let (taken, item, waited) = receiver.join().expect("the receiver thread");
        taken.expect("the woken receive takes the carried item");
        assert_eq!(&item[..7], b"carried");
        assert!(waited < Duration::from_secs(10), "woken by the migration, not by the timeout: {waited:?}");
    }

    #[test]
    fn the_locale_sidecars_migration_wakes_a_consumer_on_another_handle() {
        use crate::locale_adaptive_ring::{DefaultLocalePolicy, LocaleAdaptiveRingSidecar};

        let folder = Folder::new("sidecar");
        let (sending, receiver) = an_item_the_other_handle_cannot_see(&folder.base());
        // The sidecar moves the ring where it is asked once its 250 ms
        // cooldown has passed.
        let sidecar = LocaleAdaptiveRingSidecar::spawn(
            Arc::clone(sending.ring()),
            DefaultLocalePolicy::default(),
            Duration::from_millis(1),
        );
        sidecar.request_locale(Locale::File);
        let (taken, item, waited) = receiver.join().expect("the receiver thread");
        sidecar.shutdown();
        taken.expect("the woken receive takes the carried item");
        assert_eq!(&item[..7], b"carried");
        assert!(waited < Duration::from_secs(10), "woken by the sidecar's migration, not by the timeout: {waited:?}");
    }

    #[test]
    fn closing_ends_a_wait_on_that_handle() {
        let folder = Folder::new("close");
        let ring = LocaleAdaptiveRing::create(folder.base(), 1, 1, 8).expect("make the ring");
        let blocking = Arc::new(BlockingLocaleRing::anon(Arc::new(ring), MAX_WAITERS_DEFAULT).expect("its wakers"));
        let consumer = blocking.ring().register_consumer().expect("register a consumer");
        let waiting = Arc::clone(&blocking);
        let waiter = std::thread::spawn(move || {
            let mut out = [0u8; ADAPTIVE_SPSC_PAYLOAD_BYTES];
            waiting.recv_blocking(consumer, &mut out, None)
        });
        std::thread::sleep(Duration::from_millis(50));
        blocking.close();
        assert!(matches!(waiter.join().expect("the waiter thread"), Err(BlockingWaitError::Closed)));
    }
}
