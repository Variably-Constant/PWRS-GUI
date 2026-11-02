//! A page's channel to its backend: two SubEtha adaptive rings that the
//! backend's process opens too, one carrying the backend's frames to the
//! page and one carrying the page's frames to the backend. The renderer
//! bridges the pair. Its reader thread takes the backend's frames only
//! while the page has room for more, so frames the page has not handled
//! stay in the ring, and the backend's own sends wait once it is full. Its
//! writer thread puts the page's frames into the other ring in the order
//! the page sent them, waiting while that ring is full, and tells the
//! renderer as each goes in. A frame holds at most [`FRAME_BYTES`].

use crate::Value;
use crate::source::{RingKind, RingSource};
use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicIsize, Ordering};
use std::sync::mpsc;
use std::thread::JoinHandle;
use subetha_cxc::adaptive_ring::AdaptiveRing;
use subetha_cxc::blocking_adaptive_ring::{BlockingAdaptiveRing, MAX_WAITERS_DEFAULT};
use subetha_cxc::blocking_spsc_ring::BlockingError;
use subetha_cxc::frame_ring::LayoutHint;

/// The most a channel's frame holds: the block size of the frame region
/// SubEtha's adaptive ring makes for a frame too large for a slot, the one
/// every binding of SubEtha uses.
pub const FRAME_BYTES: usize = AdaptiveRing::FRAME_DEFAULT_BLOCK_SIZE;

/// A channel's two rings, as a page window's node names them: `toPage`,
/// which the backend writes and the renderer reads, and `fromPage`, which
/// the renderer writes and the backend reads, each a map of a ring's kind,
/// path and sizes as [`RingSource::value`] writes it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChannelEnds {
    pub to_page: RingSource,
    pub from_page: RingSource,
}

impl ChannelEnds {
    /// The channel as a node's property holds it.
    pub fn value(&self) -> Value {
        Value::Map(BTreeMap::from([(String::from("toPage"), self.to_page.value()), (String::from("fromPage"), self.from_page.value())]))
    }

    /// The channel a node's property names, or why it names none rightly.
    pub fn from_value(v: &Value) -> Result<ChannelEnds, String> {
        let Value::Map(m) = v else {
            return Err(format!("a channel is {v:?}, not a map of its toPage and fromPage rings"));
        };
        let end = |key: &str| match m.get(key) {
            Some(ring) => RingSource::from_value(ring).map_err(|why| format!("the channel's {key} ring: {why}")),
            None => Err(format!("the channel names no {key} ring")),
        };
        Ok(ChannelEnds { to_page: end("toPage")?, from_page: end("fromPage")? })
    }
}

/// The room a channel's page has for the backend's traffic, and the most of
/// the page's own traffic in flight at once, unless the channel says
/// otherwise: 64 frames and 1 MiB each way.
pub const DEFAULT_ROOM: Window = Window { frames: 64, bytes: 1 << 20 };

/// A channel a page window's node offers under its name: its rings,
/// whether its messages are text, whether it is a plain byte stream rather
/// than messages sent as parts, and the most in flight each way.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChannelSpec {
    pub ends: ChannelEnds,
    pub text: bool,
    pub stream: bool,
    pub room: Window,
}

impl ChannelSpec {
    /// The channel as the node's `channels` property holds it under its
    /// name: its `toPage` and `fromPage` rings, `text`, `stream`,
    /// `maxFrames` and `maxBytes`.
    pub fn value(&self) -> Value {
        let count = |n: usize| match i64::try_from(n) {
            Ok(n) => Value::Int(n),
            Err(_past_the_largest) => Value::Int(i64::MAX),
        };
        Value::Map(BTreeMap::from([
            (String::from("toPage"), self.ends.to_page.value()),
            (String::from("fromPage"), self.ends.from_page.value()),
            (String::from("text"), Value::Bool(self.text)),
            (String::from("stream"), Value::Bool(self.stream)),
            (String::from("maxFrames"), count(self.room.frames)),
            (String::from("maxBytes"), count(self.room.bytes)),
        ]))
    }

    /// The channel the node's property names, or why it names none rightly.
    pub fn from_value(v: &Value) -> Result<ChannelSpec, String> {
        let ends = ChannelEnds::from_value(v)?;
        let Value::Map(m) = v else {
            return Err(format!("a channel is {v:?}, not a map of its rings and settings"));
        };
        let flag = |key: &str| match m.get(key) {
            None => Ok(false),
            Some(Value::Bool(b)) => Ok(*b),
            Some(other) => Err(format!("a channel's {key} is true or false, not {other:?}")),
        };
        let cap = |key: &str, default: usize| match m.get(key) {
            None => Ok(default),
            Some(Value::Int(n)) if *n > 0 => usize::try_from(*n).map_err(|e| format!("a channel's {key}, {n}, is no count this machine takes: {e}")),
            Some(other) => Err(format!("a channel's {key} is a count above 0, not {other:?}")),
        };
        let (text, stream) = (flag("text")?, flag("stream")?);
        if text && stream {
            return Err(String::from("a channel is a stream of bytes or carries messages, and only messages are text"));
        }
        Ok(ChannelSpec { ends, text, stream, room: Window { frames: cap("maxFrames", DEFAULT_ROOM.frames)?, bytes: cap("maxBytes", DEFAULT_ROOM.bytes)? } })
    }

    /// The channels the node's `channels` property offers, by name; none
    /// without the property.
    pub fn all_of(v: Option<&Value>) -> Result<BTreeMap<String, ChannelSpec>, String> {
        match v {
            None => Ok(BTreeMap::new()),
            Some(Value::Map(named)) => named.iter().map(|(name, spec)| ChannelSpec::from_value(spec).map(|s| (name.clone(), s)).map_err(|why| format!("channel {name}: {why}"))).collect(),
            Some(other) => Err(format!("a page's channels are a map of channels by name, not {other:?}")),
        }
    }
}

/// The slots of a channel's ring with room for `frames` frames at once:
/// the next power of two, two at least.
pub fn ring_slots(frames: usize) -> Result<usize, String> {
    frames.max(2).checked_next_power_of_two().ok_or_else(|| format!("a ring cannot hold {frames} frames at once"))
}

/// Makes at `path` a ring of a channel, which one process writes and
/// another reads, of `slots` slots, and the wakers beside it that every
/// binding of SubEtha opens. The ring's files stay once this returns, for
/// the renderer and the backend to open.
pub fn make_ring(path: &std::path::Path, slots: usize) -> Result<(), String> {
    let ring = AdaptiveRing::create(path, 1, 1, slots).map_err(|e| format!("the ring at {} cannot be made: {e}", path.display()))?;
    BlockingAdaptiveRing::beside_files(Arc::new(ring), path, MAX_WAITERS_DEFAULT).map_err(|e| format!("the wakers of the ring at {} cannot be made: {e}", path.display()))?;
    Ok(())
}

/// How much of the backend's traffic the page may hold at once, given to
/// it and not yet handed back: `frames` frames and `bytes` bytes. A frame
/// is given whenever some room of each is left, so one larger than the
/// bytes left is given too, and the next waits until the page has handed
/// enough back.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Window {
    pub frames: usize,
    pub bytes: usize,
}

/// What a channel's threads tell the renderer.
#[derive(Debug, PartialEq)]
pub enum ChannelNews {
    /// A frame the backend sent, for the page.
    Frame(Vec<u8>),
    /// The page's frame that [`Channel::send`] numbered `frame` is in the
    /// ring to the backend, or why it is not.
    Sent { frame: u64, outcome: Result<(), String> },
    /// Why the channel stopped reading the backend's frames.
    Problem(String),
}

/// Where a channel's threads send their news; false once the renderer has
/// stopped taking it, which ends the thread.
pub type Outlet = Arc<dyn Fn(ChannelNews) -> bool + Send + Sync>;

/// The room the page has left for the backend's frames, in frames and in
/// bytes, which the reader takes as it gives the page a frame and the page
/// gives back as it handles them. Either count goes below zero when a
/// frame is larger than the room left, since a frame's size is known only
/// once it is read.
struct Room {
    frames: AtomicIsize,
    bytes: AtomicIsize,
    closed: AtomicBool,
}

impl Room {
    /// Whether the page has room for another frame.
    fn any(&self) -> bool {
        self.frames.load(Ordering::Acquire) > 0 && self.bytes.load(Ordering::Acquire) > 0
    }

    fn closed(&self) -> bool {
        self.closed.load(Ordering::Acquire)
    }
}

/// `n` as a signed count, held at the largest one when it is larger.
fn signed(n: usize) -> isize {
    n.min(isize::MAX as usize) as isize
}

/// A channel the renderer has open: its rings, the room the page has left,
/// its two threads, and the number of the page's last frame.
pub struct Channel {
    to_page: Arc<BlockingAdaptiveRing>,
    from_page: Arc<BlockingAdaptiveRing>,
    room: Arc<Room>,
    queue: Option<mpsc::Sender<(u64, Vec<u8>)>>,
    sent: u64,
    reader: Option<JoinHandle<()>>,
    writer: Option<JoinHandle<()>>,
}

impl Channel {
    /// Opens `ends`, whose rings must exist with the sizes they name, takes
    /// the reader's place on the ring to the page and the writer's on the
    /// other, so every frame the backend sends from then on reaches the
    /// page, and starts the reader and the writer, which tell `out` what
    /// happens; the page starts with `window`'s room, which must hold a
    /// frame at least.
    pub fn open(ends: &ChannelEnds, window: Window, out: Outlet) -> Result<Channel, String> {
        if window.frames == 0 || window.bytes == 0 {
            return Err(format!("a channel's page must have room for a frame at least, and {} frames and {} bytes is none", window.frames, window.bytes));
        }
        let to_page = open_ring(&ends.to_page, "toPage")?;
        let from_page = open_ring(&ends.from_page, "fromPage")?;
        let consumer = to_page.ring().register_consumer().map_err(|e| format!("the ring to the page takes no more readers: {e}"))?;
        let producer = match from_page.ring().register_producer() {
            Ok(p) => p,
            Err(e) => {
                to_page.ring().unregister_consumer(consumer);
                return Err(format!("the ring from the page takes no more writers: {e}"));
            }
        };
        let room = Arc::new(Room { frames: AtomicIsize::new(signed(window.frames)), bytes: AtomicIsize::new(signed(window.bytes)), closed: AtomicBool::new(false) });
        let reader = {
            let (ring, room, out) = (Arc::clone(&to_page), Arc::clone(&room), Arc::clone(&out));
            std::thread::Builder::new().name(String::from("pwrs-gui-channel-in")).spawn(move || read(&ring, consumer, &room, &out))
        };
        let reader = match reader {
            Ok(r) => r,
            Err(e) => {
                to_page.ring().unregister_consumer(consumer);
                from_page.ring().unregister_producer(producer);
                return Err(format!("the channel's reader cannot start: {e}"));
            }
        };
        let (queue, queued) = mpsc::channel();
        let writer = {
            let ring = Arc::clone(&from_page);
            std::thread::Builder::new().name(String::from("pwrs-gui-channel-out")).spawn(move || write(&ring, producer, queued, &out))
        };
        let mut channel = Channel { to_page, from_page, room, queue: Some(queue), sent: 0, reader: Some(reader), writer: None };
        match writer {
            Ok(w) => {
                channel.writer = Some(w);
                Ok(channel)
            }
            Err(e) => {
                channel.from_page.ring().unregister_producer(producer);
                channel.end();
                Err(format!("the channel's writer cannot start: {e}"))
            }
        }
    }

    /// Hands the page's `frame` to the writer, to go into the ring to the
    /// backend after the frames sent before it, and returns the number that
    /// [`ChannelNews::Sent`] names once it is in. A frame larger than
    /// [`FRAME_BYTES`] is refused, and so is any frame once the writer has
    /// stopped.
    pub fn send(&mut self, frame: Vec<u8>) -> Result<u64, String> {
        if frame.len() > FRAME_BYTES {
            return Err(format!("a frame holds at most {FRAME_BYTES} bytes, and this one is {}", frame.len()));
        }
        let Some(queue) = &self.queue else {
            return Err(String::from("the channel is closed"));
        };
        self.sent += 1;
        let number = self.sent;
        queue.send((number, frame)).map_err(|e| format!("the channel's writer has stopped, so the frame was not sent: {e}"))?;
        Ok(number)
    }

    /// The page has handled `frames` frames and `bytes` bytes of the
    /// backend's traffic, room for that much more, which wakes the reader
    /// when it waits for room.
    pub fn give_back(&self, frames: usize, bytes: usize) {
        self.room.frames.fetch_add(signed(frames), Ordering::AcqRel);
        self.room.bytes.fetch_add(signed(bytes), Ordering::AcqRel);
        if let Some(r) = &self.reader {
            r.thread().unpark();
        }
    }

    /// Ends both threads, returning once each has, after which nothing in
    /// this process has the rings open. The reader stops waiting for a
    /// frame or for room. The writer stops waiting for room in its ring,
    /// and each of its frames not yet in is told as not sent, while the
    /// renderer takes the news.
    pub fn close(mut self) {
        self.end();
    }

    fn end(&mut self) {
        self.room.closed.store(true, Ordering::Release);
        self.to_page.close();
        self.from_page.close();
        self.queue = None;
        for (what, thread) in [("reader", self.reader.take()), ("writer", self.writer.take())] {
            let Some(t) = thread else { continue };
            t.thread().unpark();
            if let Err(panic) = t.join() {
                let why = match (panic.downcast_ref::<&str>(), panic.downcast_ref::<String>()) {
                    (Some(text), _) => text.to_string(),
                    (None, Some(text)) => text.clone(),
                    (None, None) => String::from("a panic with no message"),
                };
                eprintln!("PWRS-GUI: a channel's {what} panicked: {why}");
            }
        }
    }
}

impl Drop for Channel {
    fn drop(&mut self) {
        self.end();
    }
}

/// Opens the ring `source` names, one of a channel's, `which` it is.
fn open_ring(source: &RingSource, which: &str) -> Result<Arc<BlockingAdaptiveRing>, String> {
    if source.kind != RingKind::Ring {
        return Err(format!("the channel's {which} ring is a {}, and a channel takes a Ring, SubEtha's adaptive ring", source.kind.name()));
    }
    let size = |n: i64, what: &str| usize::try_from(n).map_err(|e| format!("the channel's {which} ring's {what}, {n}, is no size this machine takes: {e}"));
    let capacity = size(source.capacity, "capacity")?;
    if capacity < 2 || !capacity.is_power_of_two() || capacity > 1 << 31 {
        return Err(format!("the channel's {which} ring's capacity, {capacity}, is no ring's: a ring has a power of two of slots, from 2 to 2^31"));
    }
    let (producers, consumers) = (size(source.producers, "producer count")?, size(source.consumers, "consumer count")?);
    if producers == 0 || consumers == 0 {
        return Err(format!("the channel's {which} ring names no producer or no consumer; a ring has at least one of each"));
    }
    let path = &source.path;
    let ring = AdaptiveRing::open(path, producers, consumers, capacity).map_err(|e| format!("the channel's {which} ring at {path} cannot be opened: {e}"))?;
    let ring = BlockingAdaptiveRing::beside_files(Arc::new(ring), path, MAX_WAITERS_DEFAULT).map_err(|e| format!("the wakers of the channel's {which} ring at {path} cannot be opened: {e}"))?;
    Ok(Arc::new(ring))
}

/// Gives the page the backend's frames, read as `consumer`, until the
/// ring's handle is closed, each once the page has room for it, parking
/// while it has none.
fn read(ring: &BlockingAdaptiveRing, consumer: usize, room: &Room, out: &Outlet) {
    let mut frame = Vec::new();
    loop {
        while !room.any() && !room.closed() {
            std::thread::park();
        }
        if room.closed() {
            break;
        }
        match ring.recv_frame_blocking(consumer, &mut frame, None) {
            Ok(_class) => {
                room.frames.fetch_sub(1, Ordering::AcqRel);
                room.bytes.fetch_sub(signed(frame.len()), Ordering::AcqRel);
                if !out(ChannelNews::Frame(std::mem::take(&mut frame))) {
                    break;
                }
            }
            Err(BlockingError::Closed) => break,
            Err(e) => {
                out(ChannelNews::Problem(format!("reading the ring to the page stopped: {e}")));
                break;
            }
        }
    }
    ring.ring().unregister_consumer(consumer);
}

/// Puts the page's frames, as they are queued, into the ring to the
/// backend as `producer`, telling as each goes in, until the queue's sender
/// is gone.
fn write(ring: &BlockingAdaptiveRing, producer: usize, queued: mpsc::Receiver<(u64, Vec<u8>)>, out: &Outlet) {
    for (frame, bytes) in queued {
        let outcome = match ring.send_frame_blocking(producer, &bytes, LayoutHint::Auto, None) {
            Ok(_class) => Ok(()),
            Err(BlockingError::Closed) => Err(String::from("the channel closed before the frame went into the ring")),
            Err(e) => Err(format!("the frame could not go into the ring to the backend: {e}")),
        };
        if !out(ChannelNews::Sent { frame, outcome }) {
            break;
        }
    }
    ring.ring().unregister_producer(producer);
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;
    use std::time::Duration;

    /// A ring of 8 slots made at `dir`'s `name` as a backend makes one, and
    /// the source naming it.
    fn made(dir: &Path, name: &str) -> (RingSource, Arc<BlockingAdaptiveRing>) {
        let path = dir.join(name);
        let ring = AdaptiveRing::create(&path, 1, 1, 8).expect("a ring");
        let ring = Arc::new(BlockingAdaptiveRing::beside_files(Arc::new(ring), &path, MAX_WAITERS_DEFAULT).expect("its wakers"));
        (RingSource { kind: RingKind::Ring, path: path.display().to_string(), capacity: 8, producers: 1, consumers: 1 }, ring)
    }

    #[test]
    fn the_backend_s_frames_reach_the_page_as_its_room_allows_and_the_page_s_reach_the_backend() {
        let dir = std::env::temp_dir().join(format!("pwrs-gui-channel-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("a folder for the rings");
        let (to_page, backend_out) = made(&dir, "to-page");
        let (from_page, backend_in) = made(&dir, "from-page");
        let (tx, rx) = mpsc::channel();
        let out: Outlet = Arc::new(move |news| tx.send(news).is_ok());
        let mut channel = Channel::open(&ChannelEnds { to_page, from_page }, Window { frames: 2, bytes: 1 << 20 }, out).expect("the channel opens");
        let producer = backend_out.ring().register_producer().expect("the backend writes to the page");
        let large = vec![7u8; FRAME_BYTES];
        for frame in [b"one".to_vec(), large.clone(), b"three".to_vec()] {
            backend_out.send_frame_blocking(producer, &frame, LayoutHint::Auto, Some(Duration::from_secs(5))).expect("the backend's frame goes in");
        }
        let next = |what: &str| rx.recv_timeout(Duration::from_secs(5)).expect(what);
        assert_eq!(next("the first frame"), ChannelNews::Frame(b"one".to_vec()));
        assert_eq!(next("the second frame, a whole block"), ChannelNews::Frame(large.clone()));
        assert!(rx.recv_timeout(Duration::from_millis(300)).is_err(), "the third frame waits until the page hands room back");
        channel.give_back(1, 3);
        assert_eq!(next("the third frame"), ChannelNews::Frame(b"three".to_vec()));
        let number = channel.send(b"hello".to_vec()).expect("the page's frame goes to the writer");
        match next("the page's frame told sent") {
            ChannelNews::Sent { frame, outcome } => {
                assert_eq!(frame, number);
                if let Err(why) = outcome {
                    panic!("the page's frame did not go in: {why}");
                }
            }
            other => panic!("the page's frame is told sent, not {other:?}"),
        }
        let consumer = backend_in.ring().register_consumer().expect("the backend reads the page's frames");
        let mut got = Vec::new();
        backend_in.recv_frame_blocking(consumer, &mut got, Some(Duration::from_secs(5))).expect("the page's frame arrives");
        assert_eq!(got, b"hello");
        let refused = channel.send(vec![0; FRAME_BYTES + 1]).expect_err("a frame larger than a block");
        assert!(refused.contains("at most"), "{refused}");
        channel.close();
        drop(backend_out);
        drop(backend_in);
        std::fs::remove_dir_all(&dir).expect("the rings' folder removed");
    }

    #[test]
    fn a_channel_takes_two_adaptive_rings_that_exist_and_a_page_with_room() {
        let ring = |kind: RingKind, path: &str| RingSource { kind, path: path.to_string(), capacity: 8, producers: 1, consumers: 1 };
        let missing = std::env::temp_dir().join(format!("pwrs-gui-channel-missing-{}", std::process::id())).display().to_string();
        let out: Outlet = Arc::new(|_news| true);
        let window = Window { frames: 4, bytes: 4096 };
        let wrong = ChannelEnds { to_page: ring(RingKind::BroadcastRing, &missing), from_page: ring(RingKind::Ring, &missing) };
        let refused = Channel::open(&wrong, window, Arc::clone(&out)).err().map(|why| why.contains("takes a Ring"));
        assert_eq!(refused, Some(true), "a broadcast ring is no channel's");
        let absent = ChannelEnds { to_page: ring(RingKind::Ring, &missing), from_page: ring(RingKind::Ring, &missing) };
        assert!(Channel::open(&absent, window, Arc::clone(&out)).is_err(), "a ring that does not exist");
        assert!(Channel::open(&absent, Window { frames: 0, bytes: 4096 }, out).is_err(), "a page with no room");
    }

    #[test]
    fn a_channel_reads_back_from_the_property_it_is_written_to() {
        let ring = |path: &str| RingSource { kind: RingKind::Ring, path: path.to_string(), capacity: 64, producers: 1, consumers: 1 };
        let ends = ChannelEnds { to_page: ring("C:/rings/to"), from_page: ring("C:/rings/from") };
        assert_eq!(ChannelEnds::from_value(&ends.value()).expect("a channel written"), ends);
        let one = Value::Map(BTreeMap::from([(String::from("toPage"), ring("x").value())]));
        let why = ChannelEnds::from_value(&one).expect_err("no fromPage ring");
        assert!(why.contains("fromPage"), "{why}");
        assert!(ChannelEnds::from_value(&Value::Str(String::from("x"))).is_err());
    }

    #[test]
    fn a_channel_s_settings_read_back_and_default_to_64_frames_and_1_mib() {
        let ring = |path: &str| RingSource { kind: RingKind::Ring, path: path.to_string(), capacity: 64, producers: 1, consumers: 1 };
        let ends = ChannelEnds { to_page: ring("C:/rings/to"), from_page: ring("C:/rings/from") };
        let spec = ChannelSpec { ends: ends.clone(), text: true, stream: false, room: Window { frames: 8, bytes: 4096 } };
        assert_eq!(ChannelSpec::from_value(&spec.value()).expect("a channel written"), spec);
        let plain = ChannelSpec::from_value(&ends.value()).expect("a channel of rings alone");
        assert_eq!((plain.text, plain.stream, plain.room), (false, false, DEFAULT_ROOM));
        assert_eq!(DEFAULT_ROOM, Window { frames: 64, bytes: 1_048_576 });
        let named = Value::Map(BTreeMap::from([(String::from("Lsp"), spec.value())]));
        assert_eq!(ChannelSpec::all_of(Some(&named)).expect("channels by name").get("Lsp"), Some(&spec));
        assert!(ChannelSpec::all_of(None).expect("no channels").is_empty());
        let Value::Map(mut both) = spec.value() else { panic!("a channel is a map") };
        both.insert(String::from("stream"), Value::Bool(true));
        let why = ChannelSpec::from_value(&Value::Map(both)).expect_err("a text stream");
        assert!(why.contains("only messages are text"), "{why}");
        let Value::Map(mut none) = ends.value() else { panic!("a channel is a map") };
        none.insert(String::from("maxFrames"), Value::Int(0));
        assert!(ChannelSpec::from_value(&Value::Map(none)).is_err(), "no room for a frame");
    }
}
