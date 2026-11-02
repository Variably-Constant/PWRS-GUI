//! The clips a window's media players play, which a runtime page loads
//! from `media/<node>` beside the runtime's files: each read as the engine
//! asks for it, a range at a time, so a long clip streams and seeks without
//! the helper holding it. A file is read in place; an https clip is fetched
//! through the loader, on a thread of its own, so it is the page's own and
//! fails with the loader's words. A player whose engine fetches an https
//! clip itself is not served; its server is asked under CORS as the page is.

use arc_swap::ArcSwap;
use pwrs_gui_proto::media::{self, Cause, Remedy, probe};
use pwrs_gui_proto::{EventKind, Kind, Node, NodeId, Value, WindowId};
use std::borrow::Cow;
use std::collections::HashMap;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::{Arc, LazyLock, OnceLock};
use wry::http::header::{ACCEPT_RANGES, CONTENT_RANGE, CONTENT_TYPE, HeaderValue};
use wry::http::{Response, StatusCode};

/// A media player's clip as the scheme's handler serves it.
#[derive(Clone, Debug, PartialEq)]
pub(super) enum Clip {
    /// A file, by its full path, read in place.
    File(PathBuf),
    /// An https clip, by its address, fetched through the loader.
    Web(String),
}

/// The clips of each open window's media players, by its webview's id and
/// then by the player's node.
pub(super) type Clips = ArcSwap<HashMap<String, Arc<HashMap<NodeId, Clip>>>>;

/// Why the loader could not fetch a part of an https clip the engine asked
/// the scheme's handler for, by address: the player's Error then names the
/// network's cause and these words.
static UNSERVED: LazyLock<ArcSwap<HashMap<String, String>>> = LazyLock::new(|| ArcSwap::from_pointee(HashMap::new()));

/// Whether each https clip's server lets pages of the web helper's origin
/// read it, by address, each asked once, on a thread of its own, as a
/// window first shows a player whose engine fetches the clip itself.
type Allowed = HashMap<String, Arc<OnceLock<Result<bool, String>>>>;
static ALLOWED: LazyLock<ArcSwap<Allowed>> = LazyLock::new(|| ArcSwap::from_pointee(HashMap::new()));

/// The most bytes one answer to a range the engine leaves open gives; the
/// engine asks for the rest as it plays.
const CHUNK: u64 = 1 << 20;

/// Why an https clip's headers are not known.
#[derive(Clone, Debug)]
enum Unread {
    /// The loader could not fetch the clip's first byte.
    Unfetched(String),
    /// Its headers could not be read or named no format.
    Unnamed(String),
}

impl Unread {
    fn text(&self) -> String {
        match self {
            Unread::Unfetched(why) | Unread::Unnamed(why) => why.clone(),
        }
    }
}

/// The formats of https clips as their headers name them, by address, each
/// read once, on a thread of its own, as a window first shows its player.
type Headers = HashMap<String, Arc<OnceLock<Result<probe::Format, Unread>>>>;
static HEADERS: LazyLock<ArcSwap<Headers>> = LazyLock::new(|| ArcSwap::from_pointee(HashMap::new()));

/// The format of the clip at `source`: a file's read from it now, an https
/// clip's as its own thread read it, or why it is not known.
fn format_of(source: &str) -> Result<probe::Format, String> {
    if !pwrs_gui_fetch::is_web(source) {
        return probe::read(Path::new(source));
    }
    match HEADERS.load().get(source).and_then(|cell| cell.get()) {
        Some(read) => read.clone().map_err(|why| why.text()),
        None => Err(String::from("its headers are still being read")),
    }
}

/// Why the loader could not fetch the https clip at `source`: a part the
/// engine asked for, or its headers; none for a clip it fetched, or not
/// tried yet.
fn unfetched(source: &str) -> Option<String> {
    if let Some(why) = UNSERVED.load().get(source) {
        return Some(why.clone());
    }
    match HEADERS.load().get(source).and_then(|cell| cell.get()) {
        Some(Err(Unread::Unfetched(why))) => Some(why.clone()),
        Some(Err(Unread::Unnamed(_)) | Ok(_)) | None => None,
    }
}

/// Why the engine may not fetch the https clip at `source` itself: its
/// server lets no page of the web helper's origin read it, or could not be
/// asked; none where it lets them, or has not answered yet.
fn forbidden(source: &str) -> Option<String> {
    match ALLOWED.load().get(source).and_then(|cell| cell.get()) {
        Some(Ok(false)) => Some(media::refused_by_cors(source)),
        Some(Err(why)) => Some(format!("its server could not be asked whether it lets the player fetch it: {why}")),
        Some(Ok(true)) | None => None,
    }
}

/// Starts asking the server of an https clip whether it lets pages of the
/// web helper's origin read it, unless it was asked already.
fn ask_allowed(source: &str) {
    if ALLOWED.load().contains_key(source) {
        return;
    }
    let cell = Arc::new(OnceLock::new());
    ALLOWED.rcu(|a| {
        let mut a = HashMap::clone(a);
        a.entry(source.to_string()).or_insert_with(|| Arc::clone(&cell));
        a
    });
    let (address, ours) = (source.to_string(), Arc::clone(&cell));
    let spawned = std::thread::Builder::new().name(String::from("pwrs-gui-media-cors")).spawn(move || {
        // The first answer for an address sets its cell; another is ignored.
        if ours.set(pwrs_gui_fetch::allows(&address, media::PAGE_ORIGIN)).is_err() {
            eprintln!("pwrs-gui-web: {address}'s server was asked twice");
        }
    });
    if let Err(e) = spawned {
        let failed = cell.set(Err(format!("no thread could be started to ask it: {e}")));
        if failed.is_err() {
            eprintln!("pwrs-gui-web: {source}'s server was asked twice");
        }
    }
}

/// Starts reading the headers of an https clip not read yet: its first byte
/// fetched for its length, and then its headers.
fn read_headers(source: &str) {
    if HEADERS.load().contains_key(source) {
        return;
    }
    let cell = Arc::new(OnceLock::new());
    HEADERS.rcu(|h| {
        let mut h = HashMap::clone(h);
        h.entry(source.to_string()).or_insert_with(|| Arc::clone(&cell));
        h
    });
    let (address, ours) = (source.to_string(), Arc::clone(&cell));
    let spawned = std::thread::Builder::new().name(String::from("pwrs-gui-media-headers")).spawn(move || {
        let read = match pwrs_gui_fetch::fetch_range(&address, 0, 1) {
            Err(why) => Err(Unread::Unfetched(why)),
            Ok(first) => probe::read_ranges(first.total, &mut |at, count| pwrs_gui_fetch::fetch_range(&address, at, count).map(|part| part.bytes)).map_err(Unread::Unnamed),
        };
        // The first read of an address sets its cell; another is ignored.
        if ours.set(read).is_err() {
            eprintln!("pwrs-gui-web: {address}'s headers were read twice");
        }
    });
    if let Err(e) = spawned {
        let failed = cell.set(Err(Unread::Unnamed(format!("no thread could be started to read its headers: {e}"))));
        if failed.is_err() {
            eprintln!("pwrs-gui-web: {source}'s headers were read twice");
        }
    }
}

/// Gives the scheme's handler the clips of window `window`'s media players
/// in `tree`, in place of those it had, and starts reading the headers of
/// its https clips; for a player whose engine fetches one itself, the
/// handler is given none, and its server is asked whether it lets it.
pub(super) fn follow(clips: &Clips, window: WindowId, tree: &Node) {
    let mut found = HashMap::new();
    let mut stack = vec![tree];
    while let Some(n) = stack.pop() {
        stack.extend(n.children.iter());
        if n.kind == Kind::MediaPlayer
            && let Some(Value::Str(source)) = n.props.get("source")
        {
            if !pwrs_gui_fetch::is_web(source) {
                found.insert(n.id, Clip::File(PathBuf::from(source)));
                continue;
            }
            read_headers(source);
            if n.props.get("engine_fetch").and_then(Value::as_bool) == Some(true) {
                ask_allowed(source);
            } else {
                found.insert(n.id, Clip::Web(source.clone()));
            }
        }
    }
    let id = window.to_string();
    let found = Arc::new(found);
    clips.rcu(|t| {
        let mut t = HashMap::clone(t);
        t.insert(id.clone(), Arc::clone(&found));
        t
    });
}

/// Stops giving the clips of the webview whose id is `id`.
pub(super) fn forget(clips: &Clips, id: &str) {
    clips.rcu(|t| {
        let mut t = HashMap::clone(t);
        t.remove(id);
        t
    });
}

/// What the scheme's handler answers a request for a clip with.
pub(super) enum Answer {
    /// The answer, made now.
    Now(Response<Cow<'static, [u8]>>),
    /// An https clip's address and the range asked of it, for a thread of
    /// its own to fetch through the loader and answer with [`fetch_part`].
    Fetch(String, Option<String>),
}

/// The answer to a request for `media/<node>` from the webview whose id is
/// `id`, `range` its Range header: for a file, the part asked for, at most
/// [`CHUNK`] bytes where the range is left open, as partial content, and
/// the whole file when no range is asked for; for an https clip, the part
/// to fetch; not found for a player the window does not have.
pub(super) fn serve(clips: &Clips, id: &str, node: &str, range: Option<&HeaderValue>) -> Answer {
    let number = match node.parse::<NodeId>() {
        Ok(n) => n,
        Err(e) => return Answer::Now(refused(StatusCode::BAD_REQUEST, format!("media/{node} names no media player: {e}"))),
    };
    let table = clips.load();
    let Some(clip) = table.get(id).and_then(|c| c.get(&number)) else {
        return Answer::Now(refused(StatusCode::NOT_FOUND, format!("window {id} has no media player {number}")));
    };
    let asked = match range.map(HeaderValue::to_str) {
        None => None,
        Some(Ok(text)) => Some(text),
        Some(Err(e)) => return Answer::Now(refused(StatusCode::RANGE_NOT_SATISFIABLE, format!("the range asked for is not text: {e}"))),
    };
    match clip {
        Clip::File(path) => Answer::Now(serve_file(path, asked)),
        Clip::Web(url) => Answer::Fetch(url.clone(), asked.map(String::from)),
    }
}

/// The part of the file at `path` that `asked`, a Range header's text,
/// asks for, as [`serve`] answers it.
fn serve_file(path: &Path, asked: Option<&str>) -> Response<Cow<'static, [u8]>> {
    let Part { bytes, start, length, whole } = match read(path, asked) {
        Ok(part) => part,
        Err((status, text)) => return refused(status, text),
    };
    let given = bytes.len() as u64;
    let mut response = Response::new(Cow::Owned(bytes));
    response.headers_mut().insert(CONTENT_TYPE, HeaderValue::from_static(mime_of(path)));
    response.headers_mut().insert(ACCEPT_RANGES, HeaderValue::from_static("bytes"));
    if whole {
        return response;
    }
    partial(response, start, given, length)
}

/// `response` as partial content of `given` bytes from `start` of a clip of
/// `length` bytes.
fn partial(mut response: Response<Cow<'static, [u8]>>, start: u64, given: u64, length: u64) -> Response<Cow<'static, [u8]>> {
    *response.status_mut() = StatusCode::PARTIAL_CONTENT;
    let end = (start + given).saturating_sub(1);
    match HeaderValue::from_str(&format!("bytes {start}-{end}/{length}")) {
        Ok(v) => {
            response.headers_mut().insert(CONTENT_RANGE, v);
            response
        }
        Err(e) => refused(StatusCode::INTERNAL_SERVER_ERROR, format!("the range's header could not be written: {e}")),
    }
}

/// The part of the https clip at `url` that `asked`, a Range header's
/// text, asks for, fetched through the loader: at most [`CHUNK`] bytes
/// where the range is left open or none is asked, as partial content. A
/// part the loader cannot fetch is refused, and why is kept for the
/// player's Error, which then names the network's cause.
pub(super) fn fetch_part(url: &str, asked: Option<&str>) -> Response<Cow<'static, [u8]>> {
    let unserved = |why: String| {
        UNSERVED.rcu(|u| {
            let mut u = HashMap::clone(u);
            u.insert(url.to_string(), why.clone());
            u
        });
        refused(StatusCode::BAD_GATEWAY, why)
    };
    let length = || pwrs_gui_fetch::fetch_range(url, 0, 1).and_then(|p| p.total.ok_or_else(|| format!("{url}'s server gives no length for it")));
    let (start, count) = match asked {
        None => (0, CHUNK),
        Some(range) => match open_span(range) {
            Ok(Span::From(a, b)) => (a, b.map_or(CHUNK, |b| (b + 1).saturating_sub(a).min(CHUNK))),
            Ok(Span::Last(n)) => match length() {
                Ok(total) => (total.saturating_sub(n), n.min(total)),
                Err(why) => return unserved(why),
            },
            Err(text) => return refused(StatusCode::RANGE_NOT_SATISFIABLE, text),
        },
    };
    let fetched = match pwrs_gui_fetch::fetch_range(url, start, count) {
        Ok(p) => p,
        Err(why) => return unserved(why),
    };
    let total = match fetched.total {
        Some(t) => t,
        None => match length() {
            Ok(t) => t,
            Err(why) => return unserved(why),
        },
    };
    let given = fetched.bytes.len() as u64;
    let mut response = Response::new(Cow::Owned(fetched.bytes));
    response.headers_mut().insert(CONTENT_TYPE, HeaderValue::from_static(mime_of(Path::new(url.split(['?', '#']).next().unwrap_or(url)))));
    response.headers_mut().insert(ACCEPT_RANGES, HeaderValue::from_static("bytes"));
    partial(response, start, given, total)
}

/// What a Range header's text asks of a clip whose length is not known
/// yet: the bytes from a first one, to a last one where it names one, or
/// the last few.
enum Span {
    From(u64, Option<u64>),
    Last(u64),
}

/// The span `range`, a Range header's text, asks for; several parts, or
/// none, are refused.
fn open_span(range: &str) -> Result<Span, String> {
    let refuse = || format!("the range {range:?} asks for no part of a clip");
    let spec = range.trim().strip_prefix("bytes=").ok_or_else(refuse)?;
    if spec.contains(',') {
        return Err(format!("the range {range:?} asks for several parts; one is answered at a time"));
    }
    let (first, last) = spec.split_once('-').ok_or_else(refuse)?;
    let number = |text: &str| text.trim().parse::<u64>().map_err(|e| format!("the range {range:?} holds {text:?}, which is no byte position: {e}"));
    match (first.trim().is_empty(), last.trim().is_empty()) {
        (true, true) => Err(refuse()),
        (true, false) => Ok(Span::Last(number(last)?)),
        (false, true) => Ok(Span::From(number(first)?, None)),
        (false, false) => {
            let (a, b) = (number(first)?, number(last)?);
            if b < a { Err(refuse()) } else { Ok(Span::From(a, Some(b))) }
        }
    }
}

/// A part of a clip's file read for an answer: its bytes, where they start,
/// the file's length, and whether they are the whole file, asked for with
/// no range.
struct Part {
    bytes: Vec<u8>,
    start: u64,
    length: u64,
    whole: bool,
}

/// Reads the part of `path` that `range`, a Range header's text, asks for.
fn read(path: &Path, range: Option<&str>) -> Result<Part, (StatusCode, String)> {
    let unreadable = |e: std::io::Error| (StatusCode::INTERNAL_SERVER_ERROR, format!("{} cannot be read: {e}", path.display()));
    let mut file = std::fs::File::open(path).map_err(|e| match e.kind() {
        std::io::ErrorKind::NotFound => (StatusCode::NOT_FOUND, format!("{} is not there", path.display())),
        _other => unreadable(e),
    })?;
    let length = file.metadata().map_err(unreadable)?.len();
    let Some(range) = range else {
        let mut bytes = Vec::new();
        file.read_to_end(&mut bytes).map_err(unreadable)?;
        return Ok(Part { bytes, start: 0, length, whole: true });
    };
    let (start, end) = span(range, length).map_err(|text| (StatusCode::RANGE_NOT_SATISFIABLE, text))?;
    file.seek(SeekFrom::Start(start)).map_err(unreadable)?;
    let mut bytes = Vec::new();
    file.take(end - start + 1).read_to_end(&mut bytes).map_err(unreadable)?;
    Ok(Part { bytes, start, length, whole: false })
}

/// The first and last byte, counting from 0, that a Range header's text
/// asks for in a file of `length` bytes: `bytes=a-b`, `bytes=a-`, at most
/// [`CHUNK`] bytes from `a`, or `bytes=-n`, the last `n`. A range past the
/// file's end, or of several parts, is refused.
fn span(range: &str, length: u64) -> Result<(u64, u64), String> {
    let refuse = || format!("the range {range:?} asks for no part of a file of {length} bytes");
    let spec = range.trim().strip_prefix("bytes=").ok_or_else(refuse)?;
    if spec.contains(',') {
        return Err(format!("the range {range:?} asks for several parts; one is answered at a time"));
    }
    let (first, last) = spec.split_once('-').ok_or_else(refuse)?;
    let number = |text: &str| text.trim().parse::<u64>().map_err(|e| format!("the range {range:?} holds {text:?}, which is no byte position: {e}"));
    if length == 0 {
        return Err(refuse());
    }
    let (start, end) = match (first.trim().is_empty(), last.trim().is_empty()) {
        (true, true) => return Err(refuse()),
        (true, false) => {
            let n = number(last)?;
            if n == 0 {
                return Err(refuse());
            }
            (length.saturating_sub(n), length - 1)
        }
        (false, true) => {
            let a = number(first)?;
            (a, a.saturating_add(CHUNK - 1).min(length - 1))
        }
        (false, false) => {
            let (a, b) = (number(first)?, number(last)?);
            if b < a {
                return Err(refuse());
            }
            (a, b.min(length - 1))
        }
    };
    if start >= length {
        return Err(refuse());
    }
    Ok((start, end))
}

/// The media type a clip is sent as, by its file's extension; the engine
/// tells the format by its bytes where the extension is not one of these.
fn mime_of(path: &Path) -> &'static str {
    let ext = path.extension().and_then(|e| e.to_str()).map(str::to_ascii_lowercase);
    match ext.as_deref() {
        Some("mp4" | "m4v") => "video/mp4",
        Some("m4a") => "audio/mp4",
        Some("mov") => "video/quicktime",
        Some("webm") => "video/webm",
        Some("mkv") => "video/x-matroska",
        Some("ogv" | "ogg") => "video/ogg",
        Some("oga" | "opus") => "audio/ogg",
        Some("mp3") => "audio/mpeg",
        Some("wav") => "audio/wav",
        Some("flac") => "audio/flac",
        Some("aac") => "audio/aac",
        Some("ac3") => "audio/ac3",
        Some("aif" | "aiff") => "audio/aiff",
        Some("caf") => "audio/x-caf",
        Some("3gp") => "video/3gpp",
        Some("avi") => "video/x-msvideo",
        Some("wmv") => "video/x-ms-wmv",
        Some("wma") => "audio/x-ms-wma",
        Some("flv") => "video/x-flv",
        _ => "application/octet-stream",
    }
}

/// The remedy a page's Error names for a clip its engine cannot play.
const EGUI: &str = "-Renderer Egui";

/// The value of event `kind` that a page raised on node `node` of `tree`.
/// A media player's Error, which the page gives as MediaError's code and
/// the engine's text, is worded as the egui renderer words its errors; a
/// Loaded whose engine has decoded no sound yet has sound where the
/// clip's headers name a sound track.
pub(super) fn named(tree: &Node, node: NodeId, kind: EventKind, value: Value) -> Value {
    let Some(source) = source_of(tree, node) else {
        return value;
    };
    match kind {
        EventKind::Error => {
            let field = |key: &str| value.as_map().and_then(|m| m.get(key));
            let code = field("code").and_then(Value::as_int).unwrap_or(0);
            let detail = field("detail").and_then(Value::as_str).unwrap_or("it gave no reason");
            page_error(source, engine_fetches(tree, node), code, detail)
        }
        EventKind::Loaded => heard(source, value),
        _ => value,
    }
}

/// The clip of media player `node` of `tree`; none for another node.
fn source_of(tree: &Node, node: NodeId) -> Option<&str> {
    tree.find(node).filter(|n| n.kind == Kind::MediaPlayer).and_then(|n| n.props.get("source")).and_then(Value::as_str)
}

/// Whether the engine of media player `node` of `tree` fetches an https
/// clip itself.
fn engine_fetches(tree: &Node, node: NodeId) -> bool {
    tree.find(node).and_then(|n| n.props.get("engine_fetch")).and_then(Value::as_bool) == Some(true)
}

/// The Error value of the clip at `source`, which the page's engine could
/// not play for MediaError `code`, its own text `detail`. An https clip the
/// loader could not fetch either fails for the network's cause, as the egui
/// renderer's does, whatever the engine made of it; so does one the engine
/// fetches itself, with `engine_fetch`, whose server does not let it, its
/// detail the helper's words, as egui's.
fn page_error(source: &str, engine_fetch: bool, code: i64, detail: &str) -> Value {
    let engine = super::engine();
    if engine_fetch && let Some(why) = forbidden(source) {
        return media::clip_error(source, Cause::Network, format_of(source).map(|f| f.name()), Remedy::None, &why);
    }
    if !engine_fetch && let Some(why) = unfetched(source) {
        return media::clip_error(source, Cause::Network, Err(why), Remedy::None, &format!("{engine}: {detail}"));
    }
    let format = format_of(source).map(|f| f.name());
    match Cause::of_code(code) {
        Ok(cause) => {
            let remedy = if cause == Cause::FormatNotSupported { Remedy::Player(EGUI) } else { Remedy::None };
            media::clip_error(source, cause, format, remedy, &format!("{engine}: {detail}"))
        }
        Err(why) => media::clip_error(source, Cause::Decode, format, Remedy::None, &format!("{engine}: {detail} ({why})")),
    }
}

/// The message of player `node`'s Error, as the page's state reply gives
/// it: MediaError's `code` and the engine's own text `detail`.
pub(super) fn page_message(tree: &Node, node: NodeId, code: i64, detail: &str) -> String {
    let value = page_error(source_of(tree, node).unwrap_or("the clip"), engine_fetches(tree, node), code, detail);
    value.as_map().and_then(|m| m.get("message")).and_then(Value::as_str).map_or_else(|| detail.to_string(), String::from)
}

/// A Loaded value of the clip in the file at `source`, its `has_audio`
/// true where the clip's headers name a sound track the engine has not
/// decoded yet.
fn heard(source: &str, value: Value) -> Value {
    let Value::Map(mut map) = value else {
        return value;
    };
    if map.get("has_audio") == Some(&Value::Bool(false)) {
        match format_of(source) {
            Ok(format) if format.audio.is_some() => {
                map.insert(String::from("has_audio"), Value::Bool(true));
            }
            Ok(_) => {}
            Err(why) => eprintln!("pwrs-gui-web: {source}'s headers could not be read for its Loaded event: {why}"),
        }
    }
    Value::Map(map)
}

/// An answer refusing a request, with why as text.
fn refused(status: StatusCode, text: String) -> Response<Cow<'static, [u8]>> {
    let mut response = Response::new(Cow::Owned(text.into_bytes()));
    *response.status_mut() = status;
    response.headers_mut().insert(CONTENT_TYPE, HeaderValue::from_static("text/plain; charset=utf-8"));
    response
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_range_names_its_first_and_last_byte_an_open_one_at_most_a_chunk() {
        let read = |range: &str, length: u64| span(range, length).expect("a range of the file");
        assert_eq!(read("bytes=0-99", 1000), (0, 99));
        assert_eq!(read("bytes=900-2000", 1000), (900, 999), "a range past the end stops at it");
        assert_eq!(read("bytes=-100", 1000), (900, 999), "the last 100 bytes");
        assert_eq!(read("bytes=0-", 10 * CHUNK), (0, CHUNK - 1), "an open range gives a chunk");
        assert_eq!(read("bytes=5-", 10), (5, 9));
        assert!(span("bytes=1000-", 1000).is_err(), "a range starting past the end");
        assert!(span("bytes=0-1,5-6", 1000).is_err(), "several parts");
        assert!(span("items=0-1", 1000).is_err());
        assert!(span("bytes=9-3", 1000).is_err());
        let open = |range: &str| match open_span(range).expect("a range of a clip") {
            Span::From(a, b) => (Some(a), b),
            Span::Last(n) => (None, Some(n)),
        };
        assert_eq!(open("bytes=5-"), (Some(5), None), "an https clip's open range");
        assert_eq!(open("bytes=5-9"), (Some(5), Some(9)));
        assert_eq!(open("bytes=-100"), (None, Some(100)), "the last 100 bytes");
        assert!(open_span("bytes=9-3").is_err() && open_span("bytes=0-1,2-3").is_err() && open_span("bytes=-").is_err());
    }

    #[test]
    fn a_player_s_clip_is_served_a_range_at_a_time_and_another_window_s_is_not_found() {
        let dir = std::env::temp_dir().join(format!("pwrs-gui-media-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("a folder for the clip");
        let path = dir.join("clip.mp4");
        std::fs::write(&path, b"0123456789").expect("the clip");
        let web = "https://127.0.0.1:9/clip.mp4";
        let tree = Node::new(1, Kind::Column)
            .child(Node::new(2, Kind::MediaPlayer).prop("source", Value::Str(path.to_string_lossy().into_owned())))
            .child(Node::new(4, Kind::MediaPlayer).prop("source", Value::Str(web.to_string())))
            .child(Node::new(5, Kind::MediaPlayer).prop("source", Value::Str(web.to_string())).prop("engine_fetch", Value::Bool(true)));
        let clips = Clips::from_pointee(HashMap::new());
        follow(&clips, 7, &tree);
        let now = |answer: Answer| match answer {
            Answer::Now(response) => response,
            Answer::Fetch(url, range) => panic!("{url} would be fetched for {range:?}"),
        };
        let serve_now = |window: &str, node: &str, range: Option<&HeaderValue>| now(serve(&clips, window, node, range));
        let part = serve_now("7", "2", Some(&HeaderValue::from_static("bytes=2-5")));
        assert_eq!((part.status(), part.body().as_ref()), (StatusCode::PARTIAL_CONTENT, &b"2345"[..]));
        assert_eq!(part.headers()[CONTENT_RANGE], "bytes 2-5/10");
        assert_eq!(part.headers()[CONTENT_TYPE], "video/mp4");
        let whole = serve_now("7", "2", None);
        assert_eq!((whole.status(), whole.body().as_ref()), (StatusCode::OK, &b"0123456789"[..]));
        assert_eq!(serve_now("8", "2", None).status(), StatusCode::NOT_FOUND, "another window's webview");
        assert_eq!(serve_now("7", "3", None).status(), StatusCode::NOT_FOUND, "no such player");
        assert_eq!(serve_now("7", "two", None).status(), StatusCode::BAD_REQUEST, "no player's number");
        assert_eq!(serve_now("7", "2", Some(&HeaderValue::from_static("bytes=20-"))).status(), StatusCode::RANGE_NOT_SATISFIABLE);
        match serve(&clips, "7", "4", Some(&HeaderValue::from_static("bytes=0-"))) {
            Answer::Fetch(url, range) => assert_eq!((url.as_str(), range.as_deref()), (web, Some("bytes=0-")), "an https clip is fetched through the loader"),
            Answer::Now(response) => panic!("an https clip was answered at once with {}", response.status()),
        }
        assert_eq!(serve_now("7", "5", None).status(), StatusCode::NOT_FOUND, "a player whose engine fetches its clip is not served");
        forget(&clips, "7");
        assert_eq!(serve_now("7", "2", None).status(), StatusCode::NOT_FOUND, "a window gone");
        std::fs::remove_file(&path).expect("the clip removed");
        std::fs::remove_dir(&dir).expect("its folder removed");
    }

    #[test]
    fn the_page_raises_position_and_bounds_the_rate_as_the_protocol_says() {
        let interval = format!("var POSITION_INTERVAL = {};", media::POSITION_INTERVAL);
        let rates = format!("var RATES = [{}, {}];", media::RATES[0], media::RATES[1]);
        for line in [interval, rates] {
            assert!(crate::RUNTIME_JS.contains(&line), "the runtime holds {line}");
        }
    }

    #[test]
    fn a_page_s_error_is_worded_as_the_egui_renderer_words_one_naming_the_format() {
        let dir = std::env::temp_dir().join(format!("pwrs-gui-media-named-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("a folder for the clip");
        let path = dir.join("clip.flv");
        let mut flv = b"FLV\x01\x05\0\0\0\x09\0\0\0\0".to_vec();
        flv.extend_from_slice(&[9, 0, 0, 1, 0, 0, 0, 0, 0, 0, 0, 0x22, 0, 0, 0, 12, 8, 0, 0, 1, 0, 0, 0, 0, 0, 0, 0, 0x6A, 0, 0, 0, 12]);
        std::fs::write(&path, &flv).expect("the clip");
        let source = path.to_string_lossy().into_owned();
        let tree = Node::new(1, Kind::Column).child(Node::new(2, Kind::MediaPlayer).prop("source", Value::Str(source.clone())));
        let page = |code: i64, detail: &str| Value::Map(std::collections::BTreeMap::from([(String::from("code"), Value::Int(code)), (String::from("detail"), Value::Str(detail.to_string()))]));
        let field = |v: &Value, key: &str| v.as_map().and_then(|m| m.get(key)).cloned().unwrap_or(Value::Null);
        let sent = named(&tree, 2, EventKind::Error, page(4, "DEMUXER_ERROR_COULD_NOT_OPEN"));
        let message = format!("{source} could not be played: its format is not supported here; it holds Sorenson Spark (H.263) video and Nellymoser audio in Flash Video (FLV)");
        assert_eq!(field(&sent, "message"), Value::Str(message.clone()));
        assert_eq!(field(&sent, "remedy"), Value::Str(String::from(EGUI)));
        assert_eq!(field(&sent, "detail"), Value::Str(format!("{}: DEMUXER_ERROR_COULD_NOT_OPEN", crate::engine())));
        assert_eq!(page_message(&tree, 2, 4, "no"), message, "a state reply's error is the event's message");
        let read = named(&tree, 2, EventKind::Error, page(2, "the read failed"));
        assert_eq!(field(&read, "cause"), Value::Str(String::from("Network")));
        assert_eq!(field(&read, "remedy"), Value::Null, "only a format names another player");
        assert_eq!(named(&tree, 1, EventKind::Error, page(4, "")), page(4, ""), "no player");
        std::fs::remove_file(&path).expect("the clip removed");
        std::fs::remove_dir(&dir).expect("its folder removed");
    }
}
