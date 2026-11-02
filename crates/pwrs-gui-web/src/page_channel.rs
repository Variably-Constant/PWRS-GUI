//! A page's channels to its backend ([`proto::channel`]), which the page's
//! `window.pwrs.channel` reaches through `POST` requests to
//! `/.pwrs/<name>/<what>` at the page's own origin, answered through wry's
//! asynchronous protocol. `open` opens the channel's rings and is answered
//! with its settings. `send` carries one ring frame and is answered once the
//! frame is in the ring. `recv` gives back the room the page has handled
//! since its last, and is answered with the backend's frames waiting, each
//! behind its length in four bytes, least significant first, or once one
//! comes. `close` ends the channel, and each request still waiting is
//! answered as gone.

use super::proto;
use proto::channel::{Channel, ChannelNews, ChannelSpec, FRAME_BYTES, Outlet};
use serde_json::{Value as Json, json};
use std::collections::{HashMap, VecDeque};
use wry::RequestAsyncResponder;
use wry::http::header::{ACCESS_CONTROL_ALLOW_ORIGIN, CONTENT_TYPE, HeaderValue};
use wry::http::{Response, StatusCode};

/// What a page asks of one of its channels.
#[derive(Debug, PartialEq)]
pub(super) enum Ask {
    Open,
    /// One ring frame for the backend.
    Send(Vec<u8>),
    /// The page has handled `frames` frames and `bytes` bytes of the
    /// backend's since its last ask, and waits for more.
    Recv { frames: usize, bytes: usize },
    Close,
}

impl Ask {
    /// The channel's name and the ask a request at `/.pwrs/<rest>` makes,
    /// `body` its body, the name written as a URL's path writes it.
    pub(super) fn of(rest: &str, body: Vec<u8>) -> Result<(String, Ask), String> {
        let Some((written, what)) = rest.rsplit_once('/') else {
            return Err(format!("a channel's request names its channel and what it asks, not {rest}"));
        };
        let name = match super::percent_decode(written) {
            Some(name) if !name.is_empty() => name,
            Some(_) | None => return Err(format!("a channel's request names no channel: {rest}")),
        };
        let ask = match what {
            "open" => Ask::Open,
            "send" => Ask::Send(body),
            "recv" => {
                let handled: Json = if body.is_empty() { json!({}) } else { serde_json::from_slice(&body).map_err(|e| format!("a channel's recv carries no JSON of what the page handled: {e}"))? };
                let count = |key: &str| match handled.get(key) {
                    None => Ok(0),
                    Some(n) => match n.as_u64().map(usize::try_from) {
                        Some(Ok(count)) => Ok(count),
                        Some(Err(too_large)) => Err(format!("a channel's recv says it handled {n} {key}, more than this machine counts: {too_large}")),
                        None => Err(format!("a channel's recv says it handled {n} {key}, which is no count")),
                    },
                };
                Ask::Recv { frames: count("frames")?, bytes: count("bytes")? }
            }
            "close" => Ask::Close,
            other => return Err(format!("a channel is asked open, send, recv or close, not {other}")),
        };
        Ok((name, ask))
    }
}

/// A request waiting for its answer, and the origin its answer allows when
/// the page asked from another one the window trusts.
pub(super) struct Answer {
    pub(super) responder: RequestAsyncResponder,
    pub(super) allow: Option<String>,
}

impl Answer {
    fn respond(self, status: StatusCode, body: Vec<u8>, content_type: &'static str) {
        let mut response = Response::new(body);
        *response.status_mut() = status;
        response.headers_mut().insert(CONTENT_TYPE, HeaderValue::from_static(content_type));
        if let Some(origin) = &self.allow {
            match HeaderValue::from_str(origin) {
                Ok(allowed) => {
                    response.headers_mut().insert(ACCESS_CONTROL_ALLOW_ORIGIN, allowed);
                }
                Err(e) => super::trace::line(|| format!("web: a channel's answer cannot allow the origin {origin}: {e}")),
            }
        }
        self.responder.respond(response);
    }

    /// Answers with `status` and `why` as text.
    pub(super) fn refuse(self, status: StatusCode, why: &str) {
        self.respond(status, why.as_bytes().to_vec(), "text/plain; charset=utf-8");
    }

    /// Answers that what was asked is done.
    pub(super) fn done(self) {
        self.respond(StatusCode::OK, Vec::new(), "application/octet-stream");
    }

    fn json(self, value: &Json) {
        self.respond(StatusCode::OK, value.to_string().into_bytes(), "application/json");
    }

    /// Answers with `frames`, each behind its length in four bytes, least
    /// significant first.
    fn frames(self, frames: impl IntoIterator<Item = Vec<u8>>) {
        match framed(frames) {
            Ok(body) => self.respond(StatusCode::OK, body, "application/octet-stream"),
            Err(why) => self.refuse(StatusCode::INTERNAL_SERVER_ERROR, &why),
        }
    }
}

/// `frames`, each behind its length in four bytes, least significant first;
/// or why one is longer than four bytes count.
fn framed(frames: impl IntoIterator<Item = Vec<u8>>) -> Result<Vec<u8>, String> {
    let mut body = Vec::new();
    for frame in frames {
        let length = u32::try_from(frame.len()).map_err(|e| format!("a frame of {} bytes is longer than four bytes count: {e}", frame.len()))?;
        body.extend_from_slice(&length.to_le_bytes());
        body.extend_from_slice(&frame);
    }
    Ok(body)
}

/// A channel a page has open: the bridge to its rings, its settings, the
/// backend's frames not yet given to the page, the page's request waiting
/// for them, and the page's sends waiting to go into the ring, by their
/// numbers.
pub(super) struct PageChannel {
    channel: Channel,
    spec: ChannelSpec,
    frames: VecDeque<Vec<u8>>,
    waiting: Option<Answer>,
    sends: HashMap<u64, Answer>,
}

impl PageChannel {
    /// Opens the rings `spec` names, the page starting with its room, its
    /// threads telling `out` what happens.
    pub(super) fn open(spec: ChannelSpec, out: Outlet) -> Result<PageChannel, String> {
        let channel = Channel::open(&spec.ends, spec.room, out)?;
        Ok(PageChannel { channel, spec, frames: VecDeque::new(), waiting: None, sends: HashMap::new() })
    }

    /// Answers `answer`, the `open` the channel was opened for, with its
    /// settings.
    pub(super) fn opened(&self, answer: Answer) {
        answer.json(&json!({ "text": self.spec.text, "stream": self.spec.stream, "maxFrames": self.spec.room.frames, "maxBytes": self.spec.room.bytes, "frameBytes": FRAME_BYTES }));
    }

    /// Hands the page's `frame` to the ring, `answer` waiting until it is in.
    pub(super) fn send(&mut self, frame: Vec<u8>, answer: Answer) {
        match self.channel.send(frame) {
            Ok(number) => {
                self.sends.insert(number, answer);
            }
            Err(why) => answer.refuse(StatusCode::BAD_REQUEST, &why),
        }
    }

    /// Gives back the room the page has handled, and answers `answer` with
    /// the backend's frames waiting, or keeps it until one comes. A request
    /// already waiting is answered with none, since this one takes its
    /// place.
    pub(super) fn recv(&mut self, frames: usize, bytes: usize, answer: Answer) {
        if frames > 0 || bytes > 0 {
            self.channel.give_back(frames, bytes);
        }
        if let Some(before) = self.waiting.take() {
            before.frames(Vec::new());
        }
        if self.frames.is_empty() {
            self.waiting = Some(answer);
        } else {
            answer.frames(self.frames.drain(..));
        }
    }

    /// Takes the channel's `news`; false once it has stopped reading the
    /// backend's frames, when the requests waiting have been answered as
    /// gone.
    pub(super) fn news(&mut self, news: ChannelNews) -> bool {
        match news {
            ChannelNews::Frame(frame) => {
                self.frames.push_back(frame);
                if let Some(waiting) = self.waiting.take() {
                    waiting.frames(self.frames.drain(..));
                }
                true
            }
            ChannelNews::Sent { frame, outcome } => {
                if let Some(answer) = self.sends.remove(&frame) {
                    match outcome {
                        Ok(()) => answer.done(),
                        Err(why) => answer.refuse(StatusCode::INTERNAL_SERVER_ERROR, &why),
                    }
                }
                true
            }
            ChannelNews::Problem(why) => {
                self.answer_gone(&why);
                false
            }
        }
    }

    /// Answers each request still waiting as gone, `why` saying why.
    fn answer_gone(&mut self, why: &str) {
        if let Some(waiting) = self.waiting.take() {
            waiting.refuse(StatusCode::GONE, why);
        }
        for (_number, answer) in self.sends.drain() {
            answer.refuse(StatusCode::GONE, why);
        }
    }

    /// Ends the channel. With `answering`, each request still waiting is
    /// answered as gone, `why` saying why; without, as when the page that
    /// made them is gone, they are dropped.
    pub(super) fn close(mut self, why: &str, answering: bool) {
        if answering {
            self.answer_gone(why);
        } else {
            self.waiting = None;
            self.sends.clear();
        }
        self.channel.close();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_channel_request_names_its_channel_and_what_it_asks() {
        let asked = |rest: &str, body: &[u8]| Ask::of(rest, body.to_vec()).expect(rest);
        assert_eq!(asked("Lsp/open", b""), (String::from("Lsp"), Ask::Open));
        assert_eq!(asked("my%20link/send", b"hi"), (String::from("my link"), Ask::Send(b"hi".to_vec())));
        assert_eq!(asked("Lsp/recv", br#"{"frames":2,"bytes":40}"#), (String::from("Lsp"), Ask::Recv { frames: 2, bytes: 40 }));
        assert_eq!(asked("Lsp/recv", b""), (String::from("Lsp"), Ask::Recv { frames: 0, bytes: 0 }), "a first recv has handled nothing");
        assert_eq!(asked("Lsp/close", b""), (String::from("Lsp"), Ask::Close));
        for (rest, body, why) in [("Lsp", &b""[..], "names its channel"), ("/open", &b""[..], "names no channel"), ("Lsp/shout", &b""[..], "not shout"), ("Lsp/recv", &br#"{"frames":-1}"#[..], "no count")] {
            let refused = Ask::of(rest, body.to_vec());
            assert!(matches!(refused, Err(ref e) if e.contains(why)), "{rest}: {refused:?}");
        }
    }

    #[test]
    fn the_backend_s_frames_reach_the_page_each_behind_its_length() {
        let body = framed([b"one".to_vec(), Vec::new(), vec![7u8; 300]]).expect("frames of four-byte lengths");
        assert_eq!(&body[..7], &[3, 0, 0, 0, b'o', b'n', b'e']);
        assert_eq!(&body[7..11], &[0, 0, 0, 0], "an empty frame is its length alone");
        assert_eq!(&body[11..15], &[44, 1, 0, 0], "300 bytes, least significant first");
        assert_eq!(body.len(), 15 + 300);
    }
}
