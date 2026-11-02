//! An adaptive ring every process can wait on, whichever surface it
//! reaches SubEtha through.
//!
//! [`BlockingAdaptiveRing`] keeps an [`AdaptiveRing`] together with the
//! two wakers its waiters park on and the notify record its pollers
//! attach to. A push through it wakes one parked consumer and signals
//! every attached notifier; a pop through it wakes one parked producer.
//! So a consumer waiting in C, Python, PowerShell or Rust wakes on a push
//! made through any of them. A push or pop made through the
//! [`AdaptiveRing`] itself wakes no one; an ordered receiver made with
//! [`AdaptiveOrderedReceiver::over_blocking`](crate::reorder::AdaptiveOrderedReceiver::over_blocking)
//! pops through the ring and wakes as a pop through this handle does. A
//! shape morph of the ring wakes every waiter on both wakers, whether the
//! caller, a sidecar or a registration in this process made it, and each
//! looks again. A producer waiting for room waits for its own lane: one
//! whose items still sit in the shape the ring left keeps that lane until
//! the consumer has emptied it, so its items keep their order.
//!
//! The wakers and the record sit beside the ring under fixed names, which
//! is how every process finds the same ones: `<prefix>.cwaker.bin`,
//! `<prefix>.pwaker.bin` and `<prefix>.notify.bin` beside a file-backed
//! ring, the regions `{name}_cwaker`, `{name}_pwaker` and `{name}_notify`
//! beside a ring in named shared memory, and in this process alone for an
//! anonymous ring. Every process names the same number of waiter slots,
//! [`MAX_WAITERS_DEFAULT`] unless all of them agree on another; a waker
//! made with another number is a [`BlockingError::WakerLayout`].

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::adaptive_ring::{AdaptiveRing, RingItem};
use crate::blocking_spsc_ring::BlockingError;
use crate::cross_process_notifier::NotifierSet;
pub use crate::cross_process_waker::MAX_WAITERS_DEFAULT;
use crate::cross_process_waker::{waker_region_size, CrossProcessWaker, WakerError};
use crate::frame_ring::{FrameClass, LayoutHint};
use crate::layout::Mismatch;
use crate::shared_ring::RingError;
use crate::shm_file::{ShmFile, ShmNamespace};

/// The sequence every parked waiter registers and every wake reaches: any
/// item, and any freed slot, counts.
const WAKE_ANY: u64 = 0;

/// An [`AdaptiveRing`] with the wakers and notify record its waiters and
/// pollers share across processes.
pub struct BlockingAdaptiveRing {
    ring: Arc<AdaptiveRing>,
    consumer_waker: Arc<CrossProcessWaker>,
    producer_waker: Arc<CrossProcessWaker>,
    notifiers: NotifierSet,
    closed: AtomicBool,
    waiters_full: AtomicU64,
}

impl BlockingAdaptiveRing {
    /// `ring` with its wakers and notify record in this process alone,
    /// for an anonymous ring; each waker has `max_waiters` slots.
    pub fn anon(ring: Arc<AdaptiveRing>, max_waiters: usize) -> Result<Self, BlockingError> {
        let consumer_waker = CrossProcessWaker::create_anon(max_waiters)?;
        let producer_waker = CrossProcessWaker::create_anon(max_waiters)?;
        Ok(Self::from_parts(ring, consumer_waker, producer_waker, NotifierSet::anon()))
    }

    /// `ring` with its wakers and notify record in the files
    /// `<prefix>.cwaker.bin`, `<prefix>.pwaker.bin` and
    /// `<prefix>.notify.bin`, made when they do not exist and attached to
    /// when they do. `prefix` is the one the ring's own files were made
    /// under.
    pub fn beside_files(ring: Arc<AdaptiveRing>, prefix: impl AsRef<Path>, max_waiters: usize) -> Result<Self, BlockingError> {
        let prefix = prefix.as_ref();
        let consumer_waker = CrossProcessWaker::create(suffixed(prefix, ".cwaker.bin"), max_waiters)?;
        let producer_waker = CrossProcessWaker::create(suffixed(prefix, ".pwaker.bin"), max_waiters)?;
        let notifiers = NotifierSet::file(prefix).map_err(BlockingError::Notify)?;
        Ok(Self::from_parts(ring, consumer_waker, producer_waker, notifiers))
    }

    /// `ring` with its wakers and notify record in the named
    /// shared-memory regions `{name}_cwaker`, `{name}_pwaker` and
    /// `{name}_notify`, made with `sddl` as their security descriptor when
    /// `create` and opened otherwise. The regions' names outlive this
    /// handle, as the ring's own do, until the ring is unlinked.
    pub fn beside_shm(
        ring: Arc<AdaptiveRing>,
        name: &str,
        namespace: ShmNamespace,
        create: bool,
        sddl: Option<&str>,
        max_waiters: usize,
    ) -> Result<Self, BlockingError> {
        let consumer_waker = shm_waker(name, "_cwaker", namespace, max_waiters, create, sddl)?;
        let producer_waker = shm_waker(name, "_pwaker", namespace, max_waiters, create, sddl)?;
        let notifiers = NotifierSet::shm(name, namespace, sddl).map_err(BlockingError::Notify)?;
        Ok(Self::from_parts(ring, consumer_waker, producer_waker, notifiers))
    }

    /// `ring` with wakers and a notify record its caller made. Waiters in
    /// other processes wake only when these are the ones beside the ring
    /// under the names above. Every shape morph of `ring` from here on
    /// wakes every waiter on both wakers.
    pub fn from_parts(
        ring: Arc<AdaptiveRing>,
        consumer_waker: CrossProcessWaker,
        producer_waker: CrossProcessWaker,
        notifiers: NotifierSet,
    ) -> Self {
        let consumer_waker = Arc::new(consumer_waker);
        let producer_waker = Arc::new(producer_waker);
        ring.watch_moves(&consumer_waker, &producer_waker);
        Self {
            ring,
            consumer_waker,
            producer_waker,
            notifiers,
            closed: AtomicBool::new(false),
            waiters_full: AtomicU64::new(0),
        }
    }

    /// The ring itself, for everything that neither pushes nor pops.
    pub fn ring(&self) -> &Arc<AdaptiveRing> {
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

    /// The notify record every push signals.
    pub fn notifiers(&self) -> &NotifierSet {
        &self.notifiers
    }

    /// Waits through this handle that found every waiter slot in use and
    /// returned [`BlockingError::WakerFull`].
    pub fn waiters_full(&self) -> u64 {
        self.waiters_full.load(Ordering::Acquire)
    }

    /// Whether [`close`](Self::close) has been called on this handle.
    pub fn is_closed(&self) -> bool {
        self.closed.load(Ordering::Acquire)
    }

    /// End every wait on this handle, now and from here on: each returns
    /// [`BlockingError::Closed`]. Waits through other handles on the same
    /// ring, in this process or another, wake, find the ring as it was and
    /// wait again.
    pub fn close(&self) {
        self.closed.store(true, Ordering::Release);
        self.consumer_waker.wake_all();
        self.producer_waker.wake_all();
    }

    /// [`AdaptiveRing::try_send`], then wake one waiting consumer and
    /// signal every attached notifier.
    pub fn try_send(&self, producer_id: usize, payload: &[u8]) -> Result<(), RingError> {
        self.ring.try_send(producer_id, payload)?;
        self.pushed();
        Ok(())
    }

    /// [`AdaptiveRing::try_recv`], then wake one waiting producer.
    pub fn try_recv(&self, consumer_id: usize, out: &mut [u8]) -> Result<usize, RingError> {
        let n = self.ring.try_recv(consumer_id, out)?;
        self.popped();
        Ok(n)
    }

    /// [`AdaptiveRing::try_recv_with_stamp`], then wake one waiting
    /// producer.
    pub fn try_recv_with_stamp(&self, consumer_id: usize, out: &mut [u8]) -> Result<(usize, u64), RingError> {
        let taken = self.ring.try_recv_with_stamp(consumer_id, out)?;
        self.popped();
        Ok(taken)
    }

    /// [`AdaptiveRing::send_frame_as`], then wake one waiting consumer and
    /// signal every attached notifier.
    pub fn send_frame_as(&self, producer_id: usize, payload: &[u8], hint: LayoutHint) -> Result<FrameClass, RingError> {
        let class = self.ring.send_frame_as(producer_id, payload, hint)?;
        self.pushed();
        Ok(class)
    }

    /// [`AdaptiveRing::recv_frame`], then wake one waiting producer.
    pub fn recv_frame(&self, consumer_id: usize, out: &mut Vec<u8>) -> Result<FrameClass, RingError> {
        let class = self.ring.recv_frame(consumer_id, out)?;
        self.popped();
        Ok(class)
    }

    /// [`AdaptiveRing::recv_any`], then wake one waiting producer.
    pub fn recv_any(&self, consumer_id: usize, out: &mut Vec<u8>) -> Result<RingItem, RingError> {
        let item = self.ring.recv_any(consumer_id, out)?;
        self.popped();
        Ok(item)
    }

    /// Receive the next item, a plain item or a frame, parking until one
    /// arrives or `timeout` has passed; `None` waits as long as it takes.
    pub fn recv_any_blocking(
        &self,
        consumer_id: usize,
        out: &mut Vec<u8>,
        timeout: Option<Duration>,
    ) -> Result<RingItem, BlockingError> {
        let deadline = deadline(timeout);
        loop {
            match self.recv_any(consumer_id, out) {
                Ok(item) => return Ok(item),
                Err(RingError::Empty) => {}
                Err(e) => return Err(BlockingError::Ring(e)),
            }
            self.park(&self.consumer_waker, deadline, || !self.ring.is_empty())?;
        }
    }

    /// Send, parking until there is room or `timeout` has passed; `None`
    /// waits as long as it takes.
    pub fn send_blocking(&self, producer_id: usize, payload: &[u8], timeout: Option<Duration>) -> Result<(), BlockingError> {
        let deadline = deadline(timeout);
        loop {
            match self.try_send(producer_id, payload) {
                Ok(()) => return Ok(()),
                Err(RingError::Full) => {}
                Err(e) => return Err(BlockingError::Ring(e)),
            }
            self.park(&self.producer_waker, deadline, || self.ring.has_room_for(producer_id))?;
        }
    }

    /// Receive, parking until an item arrives or `timeout` has passed;
    /// `None` waits as long as it takes.
    pub fn recv_blocking(&self, consumer_id: usize, out: &mut [u8], timeout: Option<Duration>) -> Result<usize, BlockingError> {
        let deadline = deadline(timeout);
        loop {
            match self.try_recv(consumer_id, out) {
                Ok(n) => return Ok(n),
                Err(RingError::Empty) => {}
                Err(e) => return Err(BlockingError::Ring(e)),
            }
            self.park(&self.consumer_waker, deadline, || !self.ring.is_empty())?;
        }
    }

    /// [`recv_blocking`](Self::recv_blocking) with the item's stamp.
    pub fn recv_blocking_with_stamp(
        &self,
        consumer_id: usize,
        out: &mut [u8],
        timeout: Option<Duration>,
    ) -> Result<(usize, u64), BlockingError> {
        let deadline = deadline(timeout);
        loop {
            match self.try_recv_with_stamp(consumer_id, out) {
                Ok(taken) => return Ok(taken),
                Err(RingError::Empty) => {}
                Err(e) => return Err(BlockingError::Ring(e)),
            }
            self.park(&self.consumer_waker, deadline, || !self.ring.is_empty())?;
        }
    }

    /// Send a frame, parking until there is room or `timeout` has passed.
    pub fn send_frame_blocking(
        &self,
        producer_id: usize,
        payload: &[u8],
        hint: LayoutHint,
        timeout: Option<Duration>,
    ) -> Result<FrameClass, BlockingError> {
        let deadline = deadline(timeout);
        loop {
            match self.send_frame_as(producer_id, payload, hint) {
                Ok(class) => return Ok(class),
                Err(RingError::Full) => {}
                Err(e) => return Err(BlockingError::Ring(e)),
            }
            self.park(&self.producer_waker, deadline, || self.ring.has_room_for(producer_id))?;
        }
    }

    /// Receive a frame, parking until one arrives or `timeout` has passed.
    pub fn recv_frame_blocking(
        &self,
        consumer_id: usize,
        out: &mut Vec<u8>,
        timeout: Option<Duration>,
    ) -> Result<FrameClass, BlockingError> {
        let deadline = deadline(timeout);
        loop {
            match self.recv_frame(consumer_id, out) {
                Ok(class) => return Ok(class),
                Err(RingError::Empty) => {}
                Err(e) => return Err(BlockingError::Ring(e)),
            }
            self.park(&self.consumer_waker, deadline, || !self.ring.is_empty())?;
        }
    }

    fn pushed(&self) {
        self.consumer_waker.wake_one_up_to(WAKE_ANY);
        self.notifiers.signal();
    }

    /// Wake one producer parked for room: what every pop through this
    /// handle does, and what an ordered receiver made with
    /// [`AdaptiveOrderedReceiver::over_blocking`](crate::reorder::AdaptiveOrderedReceiver::over_blocking)
    /// does after each of its pops.
    pub(crate) fn popped(&self) {
        self.producer_waker.wake_one_up_to(WAKE_ANY);
    }

    /// Park on `waker` until a wake or `deadline`. `ready` is checked once
    /// the slot is held, so a wake that landed between the caller's failed
    /// try and the park returns at once without entering the kernel.
    fn park(&self, waker: &CrossProcessWaker, deadline: Option<Instant>, ready: impl Fn() -> bool) -> Result<(), BlockingError> {
        if self.is_closed() {
            return Err(BlockingError::Closed);
        }
        let token = match waker.try_park(WAKE_ANY) {
            Ok(token) => token,
            Err(WakerError::Full) => {
                self.waiters_full.fetch_add(1, Ordering::AcqRel);
                return Err(BlockingError::WakerFull);
            }
            Err(e) => return Err(e.into()),
        };
        if ready() {
            waker.release(token);
            return Ok(());
        }
        if self.is_closed() {
            waker.release(token);
            return Err(BlockingError::Closed);
        }
        let remaining = match deadline {
            None => None,
            Some(d) => match d.checked_duration_since(Instant::now()) {
                Some(left) if !left.is_zero() => Some(left),
                _ => {
                    waker.release(token);
                    return Err(BlockingError::Timeout);
                }
            },
        };
        waker.wait(token, remaining)?;
        Ok(())
    }
}

/// The instant `timeout` from now; `None` for no deadline.
fn deadline(timeout: Option<Duration>) -> Option<Instant> {
    timeout.map(|t| Instant::now() + t)
}

/// `prefix` with `suffix` appended to its last component.
fn suffixed(prefix: &Path, suffix: &str) -> PathBuf {
    let mut p = prefix.as_os_str().to_owned();
    p.push(suffix);
    PathBuf::from(p)
}

/// A waker in the region `{name}{suffix}`, made or opened, whose name
/// outlives this handle.
fn shm_waker(
    name: &str,
    suffix: &str,
    namespace: ShmNamespace,
    capacity: usize,
    create: bool,
    sddl: Option<&str>,
) -> Result<CrossProcessWaker, BlockingError> {
    let region_name = format!("{name}{suffix}");
    let size = waker_region_size(capacity);
    let region = if create {
        ShmFile::create_named_secured(&region_name, size, namespace, sddl)
    } else {
        ShmFile::open_named_secured(&region_name, size, namespace, sddl)
    };
    let mut region = region.map_err(|e| match Mismatch::carried_by(&e) {
        Some(m) => BlockingError::WakerLayout(m),
        None => BlockingError::Io(std::io::Error::new(e.kind(), format!("waker region {region_name}: {e}"))),
    })?;
    region.keep_name();
    let waker = if create {
        CrossProcessWaker::create_from_shm(region, capacity)
    } else {
        CrossProcessWaker::open_from_shm(region, capacity)
    };
    Ok(waker?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adaptive_ring::ADAPTIVE_SPSC_PAYLOAD_BYTES;

    /// A folder of its own under the temporary directory, removed when the
    /// test ends; declared before the rings so it drops after them.
    struct Folder(PathBuf);

    impl Folder {
        fn new(name: &str) -> Self {
            let dir = std::env::temp_dir().join(format!(
                "subetha-blocking-adaptive-{name}-{}-{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .expect("the wall clock is after the epoch")
                    .as_nanos()
            ));
            std::fs::create_dir_all(&dir).expect("make the test's folder");
            Self(dir)
        }

        fn prefix(&self) -> PathBuf {
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

    /// A handle on the file-backed ring at `prefix`, made or attached to
    /// as another process would.
    fn handle(prefix: &Path, create: bool) -> BlockingAdaptiveRing {
        let ring = if create {
            AdaptiveRing::create(prefix, 1, 1, 8).expect("make the ring")
        } else {
            AdaptiveRing::open(prefix, 1, 1, 8).expect("attach to the ring")
        };
        BlockingAdaptiveRing::beside_files(Arc::new(ring), prefix, MAX_WAITERS_DEFAULT).expect("its wakers")
    }

    #[test]
    fn a_wait_on_an_empty_ring_times_out_and_takes_an_item_once_one_is_there() {
        let ring = AdaptiveRing::create_anon(1, 1, 8).expect("an anonymous ring");
        let blocking = BlockingAdaptiveRing::anon(Arc::new(ring), MAX_WAITERS_DEFAULT).expect("its wakers");
        let mut out = [0u8; ADAPTIVE_SPSC_PAYLOAD_BYTES];
        let began = Instant::now();
        assert!(matches!(
            blocking.recv_blocking(0, &mut out, Some(Duration::from_millis(50))),
            Err(BlockingError::Timeout)
        ));
        assert!(began.elapsed() >= Duration::from_millis(40), "the wait lasted its timeout");
        blocking.try_send(0, b"one").expect("send");
        let n = blocking.recv_blocking(0, &mut out, Some(Duration::from_secs(5))).expect("receive");
        assert_eq!(n, ADAPTIVE_SPSC_PAYLOAD_BYTES, "an item comes back in a whole slot");
        assert_eq!(&out[..3], b"one");
    }

    #[test]
    fn a_push_through_another_handle_on_the_files_wakes_a_waiting_consumer() {
        let folder = Folder::new("wake");
        let prefix = folder.prefix();
        let consumer = Arc::new(handle(&prefix, true));
        let producer = handle(&prefix, false);
        let waiting = Arc::clone(&consumer);
        let waiter = std::thread::spawn(move || {
            let mut out = [0u8; ADAPTIVE_SPSC_PAYLOAD_BYTES];
            let began = Instant::now();
            waiting.recv_blocking(0, &mut out, Some(Duration::from_secs(20))).expect("receive");
            (out, began.elapsed())
        });
        std::thread::sleep(Duration::from_millis(100));
        let sent = b"from the other handle";
        producer.try_send(0, sent).expect("send");
        let (item, waited) = waiter.join().expect("the waiter thread");
        assert_eq!(&item[..sent.len()], sent);
        assert!(waited < Duration::from_secs(10), "woken by the push, not by the timeout: {waited:?}");
        drop(producer);
        drop(consumer);
    }

    #[test]
    fn a_pop_through_another_handle_wakes_a_producer_waiting_for_room() {
        let folder = Folder::new("room");
        let prefix = folder.prefix();
        let producer = Arc::new(handle(&prefix, true));
        let consumer = handle(&prefix, false);
        loop {
            match producer.try_send(0, b"x") {
                Ok(()) => {}
                Err(RingError::Full) => break,
                Err(e) => panic!("filling the ring: {e}"),
            }
        }
        let waiting = Arc::clone(&producer);
        let sender = std::thread::spawn(move || {
            let began = Instant::now();
            waiting.send_blocking(0, b"late", Some(Duration::from_secs(20))).expect("send");
            began.elapsed()
        });
        std::thread::sleep(Duration::from_millis(100));
        let mut out = [0u8; ADAPTIVE_SPSC_PAYLOAD_BYTES];
        consumer.try_recv(0, &mut out).expect("make room");
        let waited = sender.join().expect("the sender thread");
        assert!(waited < Duration::from_secs(10), "woken by the pop, not by the timeout: {waited:?}");
        drop(consumer);
        drop(producer);
    }

    #[test]
    fn closing_ends_a_wait_on_that_handle() {
        let ring = AdaptiveRing::create_anon(1, 1, 8).expect("an anonymous ring");
        let blocking = Arc::new(BlockingAdaptiveRing::anon(Arc::new(ring), MAX_WAITERS_DEFAULT).expect("its wakers"));
        let waiting = Arc::clone(&blocking);
        let waiter = std::thread::spawn(move || {
            let mut out = [0u8; ADAPTIVE_SPSC_PAYLOAD_BYTES];
            waiting.recv_blocking(0, &mut out, None)
        });
        std::thread::sleep(Duration::from_millis(50));
        blocking.close();
        assert!(matches!(waiter.join().expect("the waiter thread"), Err(BlockingError::Closed)));
        let mut out = [0u8; ADAPTIVE_SPSC_PAYLOAD_BYTES];
        assert!(matches!(blocking.recv_blocking(0, &mut out, None), Err(BlockingError::Closed)));
    }

    #[test]
    fn a_push_signals_the_notifiers_attached_to_the_ring() {
        let ring = AdaptiveRing::create_anon(1, 1, 8).expect("an anonymous ring");
        let blocking = BlockingAdaptiveRing::anon(Arc::new(ring), MAX_WAITERS_DEFAULT).expect("its wakers");
        let notifier = blocking.notifiers().attach().expect("attach a notifier");
        blocking.try_send(0, b"ping").expect("send");
        assert!(notifier.wait(5000), "the push signaled the notifier");
    }
}
