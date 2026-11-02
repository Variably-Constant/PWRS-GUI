//! The SubEtha rings that move messages between a session and a helper.
//!
//! A ring is a [`BlockingAdaptiveRing`] in named shared memory, created by
//! the session and opened by the helper under the same name; whichever of
//! the two lets go of it last removes its names. A message is
//! postcard-encoded and sent as frames, each a flag byte, the sender's
//! random 32-bit id, the frame's number in that sender's stream from 0,
//! and a piece of the message no larger than a region block less that
//! header. The flag's low bit says more pieces follow. The receiver puts
//! each sender's frames back in the order their numbers give before
//! joining them, since the ring can deliver one of a sender's frames after
//! its later ones when producers register or leave while it is in flight,
//! so a message of any size crosses whole and in order.
//!
//! On Windows the rings are created in the session's object namespace
//! with a security descriptor granting the current user alone; on Unix
//! SubEtha creates them with mode 0600.

use std::cell::Cell;
use std::collections::{BTreeMap, HashMap, VecDeque};
use std::marker::PhantomData;
use std::sync::Arc;
use std::time::{Duration, Instant};

use serde::Serialize;
use serde::de::DeserializeOwned;
use subetha_cxc::adaptive_ring::AdaptiveRing;
use subetha_cxc::blocking_adaptive_ring::{BlockingAdaptiveRing, MAX_WAITERS_DEFAULT};
use subetha_cxc::blocking_spsc_ring::BlockingError;
pub use subetha_cxc::cross_process_notifier::Notifier;
use subetha_cxc::frame_ring::LayoutHint;
use subetha_cxc::ring_holders::LastHolder;
use subetha_cxc::shared_ring::RingError;
use subetha_cxc::shm_file::ShmNamespace;

/// Bytes ahead of each piece: the flag, the sender's id and the frame's
/// number in the sender's stream.
const HEADER: usize = 9;
/// The flag bit saying more pieces of the message follow.
const MORE: u8 = 1;
/// Handles on a ring other than the inbound one: its creator's and the
/// renderer's.
const HOLDERS: usize = 2;
/// The regions [`BlockingAdaptiveRing::beside_shm`] makes beside a ring,
/// removed with the ring's own.
const BESIDE: [&str; 3] = ["_cwaker", "_pwaker", "_notify"];

/// What a ring is for, which sets its size.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Role {
    /// The helper's `Hello`, or why it could not start.
    Start,
    /// Every message to the helper, from every sending thread.
    Inbound,
    /// One window's events.
    Window,
    /// Replies to one thread's requests.
    Reply,
}

/// A ring's queue slots, its producer hint, its region of blocks for
/// pieces too large for a slot, and how many handles may have it open at
/// once.
struct Geometry {
    capacity: usize,
    producers: usize,
    block_bytes: usize,
    blocks: usize,
    holders: usize,
}

/// Handles on one session's inbound ring: the session's, the helper's, and
/// one for each module that takes the session through PWRS-GUI's native
/// entry.
const INBOUND_HOLDERS: usize = 32;

impl Role {
    fn geometry(self) -> Geometry {
        match self {
            Role::Start => Geometry { capacity: 8, producers: 1, block_bytes: 8 << 10, blocks: 16, holders: HOLDERS },
            Role::Inbound => Geometry { capacity: 64, producers: 4, block_bytes: 64 << 10, blocks: 128, holders: INBOUND_HOLDERS },
            Role::Window => Geometry { capacity: 256, producers: 1, block_bytes: 8 << 10, blocks: 64, holders: HOLDERS },
            Role::Reply => Geometry { capacity: 64, producers: 1, block_bytes: 64 << 10, blocks: 128, holders: HOLDERS },
        }
    }
}

/// Why a ring could not move a message.
#[derive(Debug)]
pub enum TransportError {
    /// The ring could not be created or opened.
    Open { ring: String, text: String },
    /// This handle on the ring was closed, which ends its waits.
    Closed { ring: String },
    /// A message could not be encoded or decoded.
    Codec { ring: String, text: String },
    /// The ring refused for a reason of its own.
    Ring { ring: String, text: String },
}

impl std::fmt::Display for TransportError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TransportError::Open { ring, text } => write!(f, "the ring {ring} could not be opened: {text}"),
            TransportError::Closed { ring } => write!(f, "the handle on the ring {ring} was closed"),
            TransportError::Codec { ring, text } => write!(f, "a message on the ring {ring} could not be encoded or decoded: {text}"),
            TransportError::Ring { ring, text } => write!(f, "the ring {ring} refused: {text}"),
        }
    }
}

impl std::error::Error for TransportError {}

/// A name for a session's rings that no other session shares: the
/// process id and 64 random bits.
pub fn session_id() -> Result<String, TransportError> {
    let mut bits = [0u8; 8];
    getrandom::fill(&mut bits)
        .map_err(|e| TransportError::Open { ring: String::from("(any)"), text: format!("no random bits for the session's name: {e}") })?;
    Ok(format!("pwrsgui{}_{:016x}", std::process::id(), u64::from_le_bytes(bits)))
}

/// The start ring of session `session`: the helper's `Hello`.
pub fn start_ring(session: &str) -> String {
    format!("{session}_start")
}

/// The inbound ring of session `session`: every message to the helper.
pub fn inbound_ring(session: &str) -> String {
    format!("{session}_in")
}

/// The ring of window `window` in session `session`.
pub fn window_ring(session: &str, window: u64) -> String {
    format!("{session}_w{window}")
}

/// Reply ring number `n` of session `session`.
pub fn reply_ring(session: &str, n: u64) -> String {
    format!("{session}_r{n}")
}

/// One handle on a ring.
pub struct Ring {
    inner: BlockingAdaptiveRing,
    name: String,
    role: Role,
}

impl Ring {
    /// Creates the ring `name`, as the session does.
    pub fn create(name: &str, role: Role) -> Result<Arc<Ring>, TransportError> {
        Ring::make(name, role, true)
    }

    /// Opens the ring `name` another process created, as the helper does.
    pub fn open(name: &str, role: Role) -> Result<Arc<Ring>, TransportError> {
        Ring::make(name, role, false)
    }

    fn make(name: &str, role: Role, create: bool) -> Result<Arc<Ring>, TransportError> {
        let g = role.geometry();
        let open_failed = |text: String| TransportError::Open { ring: name.to_string(), text };
        let sddl = security::owner_only().map_err(open_failed)?;
        let sddl = sddl.as_deref();
        let ring = if create {
            AdaptiveRing::create_shmfs_secured(name, g.producers, 1, g.capacity, ShmNamespace::Session, sddl)
        } else {
            AdaptiveRing::open_shmfs_secured(name, g.producers, 1, g.capacity, ShmNamespace::Session, sddl)
        }
        .map_err(|e| open_failed(e.to_string()))?
        .with_frames(g.block_bytes, g.blocks)
        .with_last_holder(g.holders, LastHolder::Unlink, &BESIDE)
        .map_err(|e| open_failed(e.to_string()))?;
        let inner = BlockingAdaptiveRing::beside_shm(Arc::new(ring), name, ShmNamespace::Session, create, sddl, MAX_WAITERS_DEFAULT)
            .map_err(|e| open_failed(e.to_string()))?;
        Ok(Arc::new(Ring { inner, name: name.to_string(), role }))
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn role(&self) -> Role {
        self.role
    }

    /// Ends every wait through this handle, now and from here on. Handles
    /// on the same ring in this process or another are unaffected.
    pub fn close(&self) {
        self.inner.close();
    }

    pub fn is_closed(&self) -> bool {
        self.inner.is_closed()
    }

    /// A sender registered on this ring, with a random id of its own in
    /// each of its frames. Each thread that sends has its own.
    pub fn sender(self: &Arc<Self>) -> Result<Sender, TransportError> {
        let mut bits = [0u8; 4];
        getrandom::fill(&mut bits).map_err(|e| self.refused(format!("no random bits for a sender's id: {e}")))?;
        let id = self.inner.ring().register_producer().map_err(|e| self.refused(e.to_string()))?;
        Ok(Sender { ring: Arc::clone(self), id, tag: u32::from_le_bytes(bits), seq: Cell::new(0), _not_sync: PhantomData })
    }

    /// A receiver registered on this ring.
    pub fn receiver(self: &Arc<Self>) -> Result<Receiver, TransportError> {
        let id = self.inner.ring().register_consumer().map_err(|e| self.refused(e.to_string()))?;
        Ok(Receiver { ring: Arc::clone(self), id, frame: Vec::new(), joins: Joins::default(), _not_sync: PhantomData })
    }

    /// A notifier every send through any handle on this ring signals, for
    /// a reader that waits on it beside other handles. Its native object is
    /// an event handle on Windows and a file descriptor on Unix; after a
    /// wake, drain the notifier first and then the ring.
    pub fn notifier(&self) -> Result<Notifier, TransportError> {
        self.inner.notifiers().attach().map_err(|e| self.refused(e.to_string()))
    }

    fn refused(&self, text: String) -> TransportError {
        TransportError::Ring { ring: self.name.clone(), text }
    }

    fn blocking_failed(&self, e: BlockingError) -> TransportError {
        match e {
            BlockingError::Closed => TransportError::Closed { ring: self.name.clone() },
            other => self.refused(other.to_string()),
        }
    }

    /// The bytes in one piece of a message.
    fn piece_bytes(&self) -> usize {
        self.role.geometry().block_bytes - HEADER
    }
}

/// The encoded form of a message, for a sender that may queue it before
/// sending.
pub fn encode<T: Serialize>(ring: &Ring, msg: &T) -> Result<Vec<u8>, TransportError> {
    postcard::to_stdvec(msg).map_err(|e| TransportError::Codec { ring: ring.name.clone(), text: e.to_string() })
}

/// A message as the bytes in a ring frame, for a crossing that is not
/// a ring: PWRS-GUI's native entry.
pub fn to_bytes<T: Serialize>(msg: &T) -> Result<Vec<u8>, String> {
    postcard::to_stdvec(msg).map_err(|e| e.to_string())
}

/// The message [`to_bytes`] encoded.
pub fn from_bytes<T: DeserializeOwned>(bytes: &[u8]) -> Result<T, String> {
    postcard::from_bytes(bytes).map_err(|e| e.to_string())
}

/// A sender on one ring. It moves between threads but is not shared by
/// reference, since its producer lane takes one writer at a time.
pub struct Sender {
    ring: Arc<Ring>,
    id: usize,
    tag: u32,
    /// The number of the next frame.
    seq: Cell<u32>,
    _not_sync: PhantomData<Cell<()>>,
}

impl Sender {
    pub fn ring(&self) -> &Arc<Ring> {
        &self.ring
    }

    /// Sends `msg`, waiting while the ring is full, with no time limit.
    pub fn send<T: Serialize>(&self, msg: &T) -> Result<(), TransportError> {
        let bytes = encode(&self.ring, msg)?;
        self.send_encoded(&bytes)
    }

    /// Sends an encoded message, waiting while the ring is full.
    pub fn send_encoded(&self, bytes: &[u8]) -> Result<(), TransportError> {
        let piece = self.ring.piece_bytes();
        let mut frame = Vec::with_capacity(HEADER + piece.min(bytes.len()));
        let mut chunks = bytes.chunks(piece).peekable();
        if chunks.peek().is_none() {
            return self.send_piece(&mut frame, &[], false);
        }
        while let Some(chunk) = chunks.next() {
            let more = chunks.peek().is_some();
            self.send_piece(&mut frame, chunk, more)?;
        }
        Ok(())
    }

    /// Sends an encoded message only if it is one piece and the ring has
    /// room for it now. False means nothing was sent: the ring was full,
    /// or the message needs several pieces, which only
    /// [`send_encoded`](Self::send_encoded) sends.
    pub fn try_send_encoded(&self, bytes: &[u8]) -> Result<bool, TransportError> {
        if bytes.len() > self.ring.piece_bytes() {
            return Ok(false);
        }
        if self.ring.is_closed() {
            return Err(TransportError::Closed { ring: self.ring.name.clone() });
        }
        let mut frame = Vec::with_capacity(HEADER + bytes.len());
        frame_into(&mut frame, self.tag, self.seq.get(), bytes, false);
        match self.ring.inner.send_frame_as(self.id, &frame, LayoutHint::Auto) {
            Ok(_) => {
                self.seq.set(self.seq.get().wrapping_add(1));
                Ok(true)
            }
            Err(RingError::Full) => Ok(false),
            Err(e) => Err(self.ring.refused(e.to_string())),
        }
    }

    fn send_piece(&self, frame: &mut Vec<u8>, piece: &[u8], more: bool) -> Result<(), TransportError> {
        frame_into(frame, self.tag, self.seq.get(), piece, more);
        match self.ring.inner.send_frame_blocking(self.id, frame, LayoutHint::Auto, None) {
            Ok(_) => {
                self.seq.set(self.seq.get().wrapping_add(1));
                Ok(())
            }
            Err(e) => Err(self.ring.blocking_failed(e)),
        }
    }
}

impl Drop for Sender {
    fn drop(&mut self) {
        self.ring.inner.ring().unregister_producer(self.id);
    }
}

fn frame_into(frame: &mut Vec<u8>, tag: u32, seq: u32, piece: &[u8], more: bool) {
    frame.clear();
    frame.push(if more { MORE } else { 0 });
    frame.extend_from_slice(&tag.to_le_bytes());
    frame.extend_from_slice(&seq.to_le_bytes());
    frame.extend_from_slice(piece);
}

/// A receiver on one ring.
pub struct Receiver {
    ring: Arc<Ring>,
    id: usize,
    frame: Vec<u8>,
    joins: Joins,
    _not_sync: PhantomData<Cell<()>>,
}

impl Receiver {
    pub fn ring(&self) -> &Arc<Ring> {
        &self.ring
    }

    /// The next whole message, waiting with no time limit.
    /// [`TransportError::Closed`] once the handle is closed.
    pub fn recv<T: DeserializeOwned>(&mut self) -> Result<T, TransportError> {
        loop {
            if let Some(bytes) = self.joins.ready.pop_front() {
                return self.decode(&bytes);
            }
            if let Err(e) = self.ring.inner.recv_frame_blocking(self.id, &mut self.frame, None) {
                return Err(self.ring.blocking_failed(e));
            }
            self.joins.take(&self.frame).map_err(|e| self.ring.refused(e))?;
        }
    }

    /// The next whole message if all of it arrives within `time`, `None`
    /// once `time` has passed without it; a message already here is taken
    /// even when `time` is zero. A time past the end of this clock is no
    /// limit. [`TransportError::Closed`] once the handle is closed.
    pub fn recv_within<T: DeserializeOwned>(&mut self, time: Duration) -> Result<Option<T>, TransportError> {
        let due = Instant::now().checked_add(time);
        loop {
            if let Some(bytes) = self.joins.ready.pop_front() {
                return self.decode(&bytes).map(Some);
            }
            let left = due.map(|d| d.saturating_duration_since(Instant::now()));
            match self.ring.inner.recv_frame_blocking(self.id, &mut self.frame, left) {
                Ok(_) => {}
                Err(BlockingError::Timeout) => return Ok(None),
                Err(e) => return Err(self.ring.blocking_failed(e)),
            }
            self.joins.take(&self.frame).map_err(|e| self.ring.refused(e))?;
        }
    }

    /// The next whole message if all of it has arrived, without waiting.
    pub fn try_recv<T: DeserializeOwned>(&mut self) -> Result<Option<T>, TransportError> {
        loop {
            if let Some(bytes) = self.joins.ready.pop_front() {
                return self.decode(&bytes).map(Some);
            }
            match self.ring.inner.recv_frame(self.id, &mut self.frame) {
                Ok(_) => {}
                Err(RingError::Empty) => return Ok(None),
                Err(e) => return Err(self.ring.refused(e.to_string())),
            }
            self.joins.take(&self.frame).map_err(|e| self.ring.refused(e))?;
        }
    }

    fn decode<T: DeserializeOwned>(&self, bytes: &[u8]) -> Result<T, TransportError> {
        postcard::from_bytes(bytes).map_err(|e| TransportError::Codec { ring: self.ring.name.clone(), text: e.to_string() })
    }
}

impl Drop for Receiver {
    fn drop(&mut self) {
        self.ring.inner.ring().unregister_consumer(self.id);
    }
}

/// Each sender's frames, put back in the order their numbers give and
/// joined into whole messages, which wait in `ready` in the order their
/// last pieces were put in place.
#[derive(Default)]
struct Joins {
    senders: HashMap<u32, Stream>,
    ready: VecDeque<Vec<u8>>,
}

/// One sender's stream: the number it is owed next, the frames that came
/// before that one, and its message in progress.
#[derive(Default)]
struct Stream {
    next: u32,
    early: BTreeMap<u32, (bool, Vec<u8>)>,
    message: Vec<u8>,
}

impl Joins {
    /// Takes one frame: in place when it is the one its sender owes next,
    /// with every early frame that follows it; set aside until then
    /// otherwise.
    fn take(&mut self, frame: &[u8]) -> Result<(), String> {
        if frame.len() < HEADER {
            return Err(format!("a {}-byte frame is shorter than its {HEADER}-byte header", frame.len()));
        }
        let more = frame[0] & MORE != 0;
        let tag = u32::from_le_bytes([frame[1], frame[2], frame[3], frame[4]]);
        let seq = u32::from_le_bytes([frame[5], frame[6], frame[7], frame[8]]);
        let piece = &frame[HEADER..];
        let stream = self.senders.entry(tag).or_default();
        if seq == stream.next {
            stream.put(more, piece, &mut self.ready);
            while let Some((more, piece)) = stream.early.remove(&stream.next) {
                stream.put(more, &piece, &mut self.ready);
            }
            Ok(())
        } else if seq.wrapping_sub(stream.next) < 1 << 31 {
            match stream.early.insert(seq, (more, piece.to_vec())) {
                None => Ok(()),
                Some(_) => Err(format!("sender {tag:08x} sent frame {seq} twice")),
            }
        } else {
            Err(format!("sender {tag:08x}'s frame {seq} arrived after frame {} was owed", stream.next))
        }
    }
}

impl Stream {
    fn put(&mut self, more: bool, piece: &[u8], ready: &mut VecDeque<Vec<u8>>) {
        self.message.extend_from_slice(piece);
        self.next = self.next.wrapping_add(1);
        if !more {
            ready.push_back(std::mem::take(&mut self.message));
        }
    }
}

#[cfg(windows)]
mod security {
    use std::io;
    use std::ptr::null_mut;
    use std::sync::OnceLock;
    use windows_sys::Win32::Foundation::{CloseHandle, HANDLE, LocalFree};
    use windows_sys::Win32::Security::Authorization::ConvertSidToStringSidW;
    use windows_sys::Win32::Security::{GetTokenInformation, TOKEN_QUERY, TOKEN_USER, TokenUser};
    use windows_sys::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};

    /// A security descriptor granting the current user, alone, every right:
    /// a protected DACL with one entry for the user's SID.
    pub fn owner_only() -> Result<Option<String>, String> {
        static SDDL: OnceLock<Result<String, String>> = OnceLock::new();
        SDDL.get_or_init(user_sddl).clone().map(Some)
    }

    fn user_sddl() -> Result<String, String> {
        let mut token: HANDLE = null_mut();
        if unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) } == 0 {
            return Err(format!("the process token cannot be opened: {}", io::Error::last_os_error()));
        }
        let sid = user_sid(token);
        if unsafe { CloseHandle(token) } == 0 {
            eprintln!("PWRS-GUI: the process token could not be closed: {}", io::Error::last_os_error());
        }
        Ok(format!("D:P(A;;GA;;;{})", sid?))
    }

    fn user_sid(token: HANDLE) -> Result<String, String> {
        let mut needed = 0u32;
        let sized = unsafe { GetTokenInformation(token, TokenUser, null_mut(), 0, &mut needed) };
        if sized != 0 || needed == 0 {
            return Err(format!("the token's user cannot be sized: {}", io::Error::last_os_error()));
        }
        let words = (needed as usize).div_ceil(size_of::<u64>());
        let mut buf = vec![0u64; words];
        if unsafe { GetTokenInformation(token, TokenUser, buf.as_mut_ptr().cast(), needed, &mut needed) } == 0 {
            return Err(format!("the token's user cannot be read: {}", io::Error::last_os_error()));
        }
        let user = unsafe { &*(buf.as_ptr() as *const TOKEN_USER) };
        let mut wide: *mut u16 = null_mut();
        if unsafe { ConvertSidToStringSidW(user.User.Sid, &mut wide) } == 0 {
            return Err(format!("the user's SID cannot be written as text: {}", io::Error::last_os_error()));
        }
        let mut len = 0usize;
        while unsafe { *wide.add(len) } != 0 {
            len += 1;
        }
        let text = String::from_utf16(unsafe { std::slice::from_raw_parts(wide, len) });
        if !unsafe { LocalFree(wide.cast()) }.is_null() {
            eprintln!("PWRS-GUI: the SID text could not be freed: {}", io::Error::last_os_error());
        }
        text.map_err(|e| format!("the user's SID is not UTF-16: {e}"))
    }
}

#[cfg(unix)]
mod security {
    /// SubEtha creates named shared memory with mode 0600 on Unix, so no
    /// descriptor is needed.
    pub fn owner_only() -> Result<Option<String>, String> {
        Ok(None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    fn pair(role: Role) -> (Arc<Ring>, Arc<Ring>) {
        let name = format!("{}_t", session_id().expect("a session name"));
        let made = Ring::create(&name, role).expect("create a ring");
        let opened = Ring::open(&name, role).expect("open the ring");
        (made, opened)
    }

    #[test]
    fn a_message_crosses_between_two_handles() {
        let (made, opened) = pair(Role::Window);
        let tx = opened.sender().expect("a sender");
        let mut rx = made.receiver().expect("a receiver");
        tx.send(&(7u64, String::from("seven"))).expect("send");
        assert_eq!(rx.recv::<(u64, String)>().expect("receive"), (7, String::from("seven")));
        assert!(rx.try_recv::<(u64, String)>().expect("an empty ring").is_none());
    }

    #[test]
    fn a_receive_within_a_time_gives_up_once_it_passes_and_takes_what_is_there() {
        let (made, opened) = pair(Role::Reply);
        let tx = opened.sender().expect("a sender");
        let mut rx = made.receiver().expect("a receiver");
        let start = Instant::now();
        assert!(rx.recv_within::<u64>(Duration::from_millis(50)).expect("an empty ring").is_none());
        assert!(start.elapsed() >= Duration::from_millis(50), "gave up after {:?}", start.elapsed());
        tx.send(&9u64).expect("send");
        assert_eq!(rx.recv_within::<u64>(Duration::ZERO).expect("receive"), Some(9), "a message already here is taken at zero");
    }

    #[test]
    fn a_message_larger_than_a_block_crosses_whole() {
        let (made, opened) = pair(Role::Reply);
        let tx = made.sender().expect("a sender");
        let mut rx = opened.receiver().expect("a receiver");
        let big: Vec<u8> = (0..(1usize << 20) + 12_345).map(|i| (i % 251) as u8).collect();
        let sending = std::thread::spawn(move || tx.send(&big).map(|()| big));
        let got: Vec<u8> = rx.recv().expect("the whole message");
        let sent = sending.join().expect("the sending thread").expect("send");
        assert_eq!(got.len(), sent.len());
        assert!(got == sent, "the bytes changed crossing");
    }

    #[test]
    fn the_pieces_of_two_senders_are_kept_apart() {
        let (made, opened) = pair(Role::Inbound);
        let mut rx = opened.receiver().expect("a receiver");
        let senders: Vec<_> = (0..2u8)
            .map(|k| {
                let ring = Arc::clone(&made);
                std::thread::spawn(move || {
                    let tx = ring.sender().expect("a sender");
                    for round in 0..8u8 {
                        let msg = vec![k ^ round; 200_000];
                        tx.send(&(k, round, msg)).expect("send");
                    }
                })
            })
            .collect();
        let mut seen = [0u8; 2];
        for _ in 0..16 {
            let (k, round, msg): (u8, u8, Vec<u8>) = rx.recv().expect("receive");
            assert_eq!(round, seen[k as usize], "sender {k}'s messages arrived out of order");
            assert!(msg.iter().all(|&b| b == k ^ round), "sender {k}'s message {round} was mixed with another's");
            seen[k as usize] += 1;
        }
        for s in senders {
            s.join().expect("a sending thread");
        }
    }

    fn frame(tag: u32, seq: u32, piece: &[u8], more: bool) -> Vec<u8> {
        let mut f = Vec::new();
        frame_into(&mut f, tag, seq, piece, more);
        f
    }

    #[test]
    fn a_senders_frames_handed_over_out_of_order_are_joined_in_the_order_sent() {
        let mut joins = Joins::default();
        for f in [frame(7, 0, b"a", true), frame(7, 2, b"c", false), frame(9, 0, b"x", false), frame(7, 3, b"d", false), frame(7, 1, b"b", true)] {
            joins.take(&f).expect("a frame in its sender's stream");
        }
        let ready: Vec<&[u8]> = joins.ready.iter().map(|m| m.as_slice()).collect();
        assert_eq!(ready, vec![&b"x"[..], &b"abc"[..], &b"d"[..]]);
        assert!(joins.take(&frame(7, 2, b"c", false)).is_err(), "a frame from before the one owed is refused");
        assert!(joins.take(&frame(9, 5, b"y", false)).is_ok(), "a frame ahead of the one owed is set aside");
        assert!(joins.take(&frame(9, 5, b"y", false)).is_err(), "a frame set aside already is refused the second time");
    }

    #[test]
    fn closing_a_handle_ends_a_wait_on_it() {
        let (made, _opened) = pair(Role::Window);
        let waiting = Arc::clone(&made);
        let waiter = std::thread::spawn(move || {
            let mut rx = waiting.receiver().expect("a receiver");
            let began = Instant::now();
            (rx.recv::<u64>(), began.elapsed())
        });
        std::thread::sleep(Duration::from_millis(100));
        made.close();
        let (outcome, waited) = waiter.join().expect("the waiting thread");
        assert!(matches!(outcome, Err(TransportError::Closed { .. })), "{outcome:?}");
        assert!(waited < Duration::from_secs(10), "the close woke the wait: {waited:?}");
    }

    #[test]
    fn a_try_send_refuses_a_message_of_several_pieces_and_sends_one_that_fits() {
        let (made, opened) = pair(Role::Window);
        let tx = opened.sender().expect("a sender");
        let big = encode(&opened, &vec![1u8; 100_000]).expect("encode");
        assert!(!tx.try_send_encoded(&big).expect("a refusal is not an error"));
        let small = encode(&opened, &3u64).expect("encode");
        assert!(tx.try_send_encoded(&small).expect("send"));
        let mut rx = made.receiver().expect("a receiver");
        assert_eq!(rx.try_recv::<u64>().expect("receive"), Some(3));
    }

    #[test]
    fn a_notifier_wakes_for_a_send() {
        let (made, opened) = pair(Role::Inbound);
        let notifier = opened.notifier().expect("a notifier");
        let tx = made.sender().expect("a sender");
        tx.send(&1u8).expect("send");
        assert!(notifier.wait(5000), "the send signaled the notifier");
        notifier.drain();
        let mut rx = opened.receiver().expect("a receiver");
        assert_eq!(rx.try_recv::<u8>().expect("receive"), Some(1));
    }
}
