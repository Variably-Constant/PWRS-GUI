//! The web helper's own HTTP server on 127.0.0.1, which serves a runtime
//! page's clips on WebKitGTK. Its player reads a clip through WebKit's
//! GStreamer source, which takes http, https and blob addresses but not the
//! pwrs scheme, so there each media player plays its clip from
//! `http://127.0.0.1:<port>/media/<token>` in place of `media/<node>`.
//!
//! The port is the one the system gives as the first clip is found, and the
//! server listens on it for as long as the helper runs. Each token is 128
//! bits from the system's random source, in hex, made for one clip of one
//! player. It ends when that player, its source or its window ends, and so
//! does each connection whose last request named it. A request whose token
//! is unknown or has ended gets 404, and its connection is closed, as it is
//! after a method other than GET or HEAD (405) or another path (404). Any
//! other connection stays open until WebKit closes it: there is no idle time
//! and no cap on connections, nor on a request's line and headers.
//!
//! A clip's parts are the pwrs scheme's own ([`media::serve_clip`]), so a
//! range left open gets at most 1 MiB. Each response lets pages of the
//! helper's origin read it, as they read a clip the pwrs scheme serves, and
//! is marked no-store, so no cache keeps it.
//!
//! Where no port can be bound, the server serves nothing. At an accept error
//! other than a peer's own abort, it closes its listener and traces why. In
//! either case each player after that fails for the network's cause, with
//! why in its Error's detail.

use crate::media::{self, Answer, Clip};
use arc_swap::ArcSwap;
use pwrs_gui_proto::media::PAGE_ORIGIN;
use pwrs_gui_proto::{NodeId, trace};
use serde_json::{Map, Value as Json};
use std::borrow::Cow;
use std::collections::{BTreeMap, HashMap};
use std::io::{BufRead, BufReader, ErrorKind, Write};
use std::net::{Ipv4Addr, Shutdown, TcpListener, TcpStream};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, LazyLock, OnceLock};
use wry::http::{Response, StatusCode};

/// A clip the server serves under a token, with its player: the webview id
/// of its window, and its node.
struct Player {
    window: String,
    node: NodeId,
    clip: Clip,
}

/// A connection to the server: the token its last request named, and its
/// socket, which the end of that token shuts.
#[derive(Clone)]
struct Connection {
    token: Option<String>,
    socket: Arc<TcpStream>,
}

/// What the server serves, and to whom: swapped whole as the helper's loop
/// follows its windows, and read by the threads that serve connections.
#[derive(Clone, Default)]
struct Served {
    /// The player each token serves.
    tokens: HashMap<String, Arc<Player>>,
    /// Each window's players' tokens, by its webview's id and then by node.
    windows: HashMap<String, BTreeMap<NodeId, String>>,
    /// The script last given to each window's page.
    given: HashMap<String, String>,
}

static SERVED: LazyLock<ArcSwap<Served>> = LazyLock::new(|| ArcSwap::from_pointee(Served::default()));

/// Each open connection, by its number, swapped whole by the threads that
/// open, serve and end connections.
static CONNECTIONS: LazyLock<ArcSwap<HashMap<u64, Connection>>> = LazyLock::new(|| ArcSwap::from_pointee(HashMap::new()));

/// The number of the next connection.
static NEXT: AtomicU64 = AtomicU64::new(0);

/// The server's port, or why none could be bound; bound once.
static PORT: OnceLock<Result<u16, String>> = OnceLock::new();

/// Why the server closed its listener, at an accept error other than a
/// peer's own abort.
static CLOSED: OnceLock<String> = OnceLock::new();

/// The script that gives a page no address, as none of its window's
/// players is served.
const NONE: &str = "pwrsRuntime.clips({}, null);";

/// The digits a token is written in.
const HEX: &[u8; 16] = b"0123456789abcdef";

/// Follows the players of the window whose webview's id is `window`, which
/// play the clips `found`. A player keeps its token while its clip stays the
/// same and gets a new one for a new clip; the token of a player gone ends,
/// with each connection whose last request named it. The server starts as
/// the first clip is found. Returns the script that gives the page its
/// players' addresses, or why it has none, where that changed.
pub(super) fn follow(window: &str, found: &HashMap<NodeId, Clip>) -> Option<String> {
    let port = (!found.is_empty()).then(listening);
    let mut outcome = (None, Vec::new());
    SERVED.rcu(|served| {
        let mut next = Served::clone(served);
        outcome = followed(&mut next, window, found, port.clone());
        next
    });
    let (text, ended) = outcome;
    shut(&ended);
    text
}

/// What following window `window`'s clips `found` makes of `served`, given
/// the port `port` the server listens on where a clip is found: the script
/// that gives the page their addresses, where it changed, and the tokens
/// that end.
fn followed(served: &mut Served, window: &str, found: &HashMap<NodeId, Clip>, port: Option<Result<u16, String>>) -> (Option<String>, Vec<String>) {
    let Served { tokens, windows, given } = served;
    let issued = windows.entry(window.to_string()).or_default();
    let changed: Vec<NodeId> = issued.iter().filter(|&(node, token)| tokens.get(token).map(|p| &p.clip) != found.get(node)).map(|(node, _)| *node).collect();
    let mut ended = Vec::new();
    for node in changed {
        if let Some(token) = issued.remove(&node) {
            tokens.remove(&token);
            ended.push(token);
        }
    }
    let addresses: Result<BTreeMap<NodeId, String>, String> = match port {
        None => Ok(BTreeMap::new()),
        Some(Err(why)) => Err(why),
        Some(Ok(port)) => found.iter().map(|(node, clip)| issue(tokens, issued, window, *node, clip).map(|token| (*node, format!("http://127.0.0.1:{port}/media/{token}")))).collect(),
    };
    if issued.is_empty() {
        windows.remove(window);
    }
    let text = script(addresses);
    if given.get(window).map_or(NONE, String::as_str) == text {
        return (None, ended);
    }
    given.insert(window.to_string(), text.clone());
    (Some(text), ended)
}

/// The script last given to the page of the window whose webview's id is
/// `window`, for the page loaded again; none while it was given no address.
pub(super) fn addresses(window: &str) -> Option<String> {
    SERVED.load().given.get(window).cloned()
}

/// Ends the tokens of the window whose webview's id is `window`, and shuts
/// each connection whose last request named one.
pub(super) fn forget(window: &str) {
    let mut ended = Vec::new();
    SERVED.rcu(|served| {
        let mut next = Served::clone(served);
        next.given.remove(window);
        ended = match next.windows.remove(window) {
            Some(issued) => issued.into_values().collect(),
            None => Vec::new(),
        };
        for token in &ended {
            next.tokens.remove(token);
        }
        next
    });
    shut(&ended);
}

/// Why the server serves no new clip: no port could be bound, or it closed
/// its listener.
pub(super) fn unserved() -> Option<String> {
    match (PORT.get(), CLOSED.get()) {
        (Some(Err(why)), _) | (_, Some(why)) => Some(why.clone()),
        (Some(Ok(_)) | None, None) => None,
    }
}

/// The port the server listens on, which it binds the first time: or why it
/// serves no new clip.
fn listening() -> Result<u16, String> {
    let port = PORT.get_or_init(start).clone()?;
    match CLOSED.get() {
        Some(why) => Err(why.clone()),
        None => Ok(port),
    }
}

/// The token of player `node` of window `window`, whose clip is `clip`: the
/// one it has, or a new one.
fn issue(tokens: &mut HashMap<String, Arc<Player>>, issued: &mut BTreeMap<NodeId, String>, window: &str, node: NodeId, clip: &Clip) -> Result<String, String> {
    if let Some(token) = issued.get(&node) {
        return Ok(token.clone());
    }
    let token = new_token()?;
    tokens.insert(token.clone(), Arc::new(Player { window: window.to_string(), node, clip: clip.clone() }));
    issued.insert(node, token.clone());
    Ok(token)
}

/// Shuts each connection whose last request named one of `ended`, tokens
/// that have ended: after the swap that drops them, so each is shut once.
fn shut(ended: &[String]) {
    if ended.is_empty() {
        return;
    }
    let mut closing = Vec::new();
    CONNECTIONS.rcu(|open| {
        closing.clear();
        let mut kept = HashMap::clone(open);
        kept.retain(|number, connection| {
            let ends = connection.token.as_ref().is_some_and(|t| ended.contains(t));
            if ends {
                closing.push((*number, Arc::clone(&connection.socket)));
            }
            !ends
        });
        kept
    });
    for (number, socket) in closing {
        if let Err(e) = socket.shutdown(Shutdown::Both) {
            trace::line(|| format!("web: clip server connection {number} could not be shut: {e}"));
        }
    }
}

/// The script that gives a page its players' `addresses`, by node, or why
/// it has none.
fn script(addresses: Result<BTreeMap<NodeId, String>, String>) -> String {
    let (given, why) = match addresses {
        Ok(found) => (found.into_iter().map(|(node, address)| (node.to_string(), Json::String(address))).collect::<Map<String, Json>>(), Json::Null),
        Err(why) => (Map::new(), Json::String(why)),
    };
    format!("pwrsRuntime.clips({}, {why});", Json::Object(given))
}

/// A new token: 128 bits from the system's random source, in hex.
fn new_token() -> Result<String, String> {
    let mut bytes = [0u8; 16];
    random(&mut bytes)?;
    let mut text = String::with_capacity(32);
    for b in bytes {
        text.push(char::from(HEX[usize::from(b >> 4)]));
        text.push(char::from(HEX[usize::from(b & 15)]));
    }
    Ok(text)
}

/// Fills `bytes` from the system's random source.
#[cfg(unix)]
fn random(bytes: &mut [u8]) -> Result<(), String> {
    use std::io::Read;
    std::fs::File::open("/dev/urandom").and_then(|mut source| source.read_exact(bytes)).map_err(|e| format!("the system's random source could not be read for a clip's token: {e}"))
}

/// Fills `bytes` for the tests of a system with no clip server, whose
/// tokens need only differ: from hashers the standard library keys from the
/// system's random source.
#[cfg(not(unix))]
fn random(bytes: &mut [u8]) -> Result<(), String> {
    use std::hash::{BuildHasher, RandomState};
    for (i, chunk) in bytes.chunks_mut(8).enumerate() {
        let word = RandomState::new().hash_one(i).to_le_bytes();
        chunk.copy_from_slice(&word[..chunk.len()]);
    }
    Ok(())
}

/// Binds a port on 127.0.0.1, the one the system gives, and serves it on a
/// thread of its own: the port, or why none could be bound.
fn start() -> Result<u16, String> {
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).map_err(|e| format!("no port on 127.0.0.1 could be bound to serve the clips: {e}"))?;
    let port = listener.local_addr().map_err(|e| format!("the port bound to serve the clips could not be read: {e}"))?.port();
    std::thread::Builder::new()
        .name(String::from("pwrs-gui-clip-server"))
        .spawn(move || listen(&listener))
        .map_err(|e| format!("no thread could be started to serve the clips: {e}"))?;
    trace::line(|| format!("web: the clip server listens on 127.0.0.1:{port}"));
    Ok(port)
}

/// Serves each connection the listener takes on a thread of its own, until
/// an accept error other than a peer's own abort, which closes the listener
/// as the thread ends.
fn listen(listener: &TcpListener) {
    for stream in listener.incoming() {
        match stream {
            Ok(stream) => open(stream),
            Err(e) if e.kind() == ErrorKind::ConnectionAborted => trace::line(|| format!("web: a connection to the clip server was aborted before it was taken: {e}")),
            Err(e) => {
                let why = format!("the clip server stopped taking connections: {e}");
                trace::line(|| format!("web: {why}; its listener closes"));
                if CLOSED.set(why).is_err() {
                    trace::line(|| String::from("web: the clip server's listener was closed already"));
                }
                return;
            }
        }
    }
}

/// Numbers a connection and serves it on a thread of its own.
fn open(stream: TcpStream) {
    let number = match register(&stream) {
        Ok(n) => n,
        Err(why) => {
            eprintln!("pwrs-gui-web: a connection to the clip server goes unserved: {why}");
            return;
        }
    };
    let spawned = std::thread::Builder::new().name(String::from("pwrs-gui-clip")).spawn(move || {
        serve(number, stream);
        unregister(number);
    });
    if let Err(e) = spawned {
        eprintln!("pwrs-gui-web: clip server connection {number} goes unserved: no thread could be started for it: {e}");
        unregister(number);
    }
}

/// The number of a new connection, kept with a handle on its socket.
fn register(stream: &TcpStream) -> Result<u64, String> {
    let socket = Arc::new(stream.try_clone().map_err(|e| format!("its socket could not be kept: {e}"))?);
    let number = NEXT.fetch_add(1, Ordering::Relaxed);
    CONNECTIONS.rcu(|open| {
        let mut open = HashMap::clone(open);
        open.insert(number, Connection { token: None, socket: Arc::clone(&socket) });
        open
    });
    Ok(number)
}

/// Forgets connection `number`, which has ended.
fn unregister(number: u64) {
    CONNECTIONS.rcu(|open| {
        let mut open = HashMap::clone(open);
        open.remove(&number);
        open
    });
}

/// Serves connection `number`'s requests in turn, until WebKit closes it,
/// its token ends, or a response closes it.
fn serve(number: u64, stream: TcpStream) {
    let mut reader = match stream.try_clone() {
        Ok(s) => BufReader::new(s),
        Err(e) => {
            eprintln!("pwrs-gui-web: clip server connection {number} could not be read: {e}");
            return;
        }
    };
    let mut writer = stream;
    loop {
        let request = match read_request(&mut reader) {
            Ok(Some(r)) => r,
            Ok(None) => return,
            Err(why) => {
                trace::line(|| format!("web: clip server connection {number} gets 400: {why}"));
                if let Err(e) = send(&mut writer, &media::refused(StatusCode::BAD_REQUEST, why), true, true) {
                    trace::line(|| format!("web: clip server connection {number} ends: its response could not be sent: {e}"));
                }
                return;
            }
        };
        let (response, whose) = response(number, &request);
        let close = request.close || whose.is_none();
        trace::line(|| {
            let clip = whose.as_deref().unwrap_or("no clip it serves");
            format!("web: clip server connection {number} gets {} for a {} of {clip}, range {:?}, origin {:?}", response.status(), request.method, request.range, request.origin)
        });
        if let Err(e) = send(&mut writer, &response, request.method != "HEAD", close) {
            trace::line(|| format!("web: clip server connection {number} ends: its response could not be sent: {e}"));
            return;
        }
        if close {
            return;
        }
    }
}

/// A request: its method and target, its Range and Origin headers, and
/// whether its connection closes after its response.
struct Request {
    method: String,
    target: String,
    range: Option<String>,
    origin: Option<String>,
    close: bool,
}

/// Reads a connection's next request: none once WebKit has closed it, and
/// why where it could not be read or is no HTTP request.
fn read_request(reader: &mut impl BufRead) -> Result<Option<Request>, String> {
    let mut line = String::new();
    if reader.read_line(&mut line).map_err(|e| format!("its request could not be read: {e}"))? == 0 {
        return Ok(None);
    }
    let mut words = line.split_whitespace();
    let (Some(method), Some(target), Some(version)) = (words.next(), words.next(), words.next()) else {
        return Err(format!("its request line {line:?} is no HTTP request line"));
    };
    let mut request = Request { method: method.to_string(), target: target.to_string(), range: None, origin: None, close: version == "HTTP/1.0" };
    loop {
        let mut header = String::new();
        if reader.read_line(&mut header).map_err(|e| format!("its request's headers could not be read: {e}"))? == 0 {
            return Err(String::from("it closed before its request's headers ended"));
        }
        let header = header.trim_end();
        if header.is_empty() {
            return Ok(Some(request));
        }
        let Some((name, value)) = header.split_once(':') else {
            return Err(format!("its request holds {header:?}, which is no header"));
        };
        let value = value.trim();
        if name.eq_ignore_ascii_case("range") {
            request.range = Some(value.to_string());
        } else if name.eq_ignore_ascii_case("origin") {
            request.origin = Some(value.to_string());
        } else if name.eq_ignore_ascii_case("connection") {
            let says = |word: &str| value.split(',').any(|w| w.trim().eq_ignore_ascii_case(word));
            if says("close") {
                request.close = true;
            } else if says("keep-alive") {
                request.close = false;
            }
        }
    }
}

/// The response to `request` on connection `number`, and whose clip it
/// gives: none where it gives no clip, which closes the connection.
fn response(number: u64, request: &Request) -> (Response<Cow<'static, [u8]>>, Option<String>) {
    if request.method != "GET" && request.method != "HEAD" {
        return (media::refused(StatusCode::METHOD_NOT_ALLOWED, format!("the clip server serves GET and HEAD, not {}", request.method)), None);
    }
    let path = request.target.split_once('?').map_or(request.target.as_str(), |(path, _)| path);
    let Some(token) = path.strip_prefix("/media/") else {
        return (media::refused(StatusCode::NOT_FOUND, format!("the clip server serves /media/<token>, not {path}")), None);
    };
    let unknown = || -> (Response<Cow<'static, [u8]>>, Option<String>) { (media::refused(StatusCode::NOT_FOUND, String::from("no clip is served under that token")), None) };
    if !SERVED.load().tokens.contains_key(token) {
        return unknown();
    }
    CONNECTIONS.rcu(|open| {
        let mut open = HashMap::clone(open);
        if let Some(connection) = open.get_mut(&number) {
            connection.token = Some(token.to_string());
        }
        open
    });
    // The token is read again once the connection names it, so a token that
    // ended in between either shuts this connection or is refused here.
    let Some(player) = SERVED.load().tokens.get(token).cloned() else {
        return unknown();
    };
    let whose = format!("window {}'s player {}", player.window, player.node);
    let response = match media::serve_clip(&player.clip, request.range.as_deref()) {
        Answer::Now(response) => response,
        Answer::Fetch(url, asked) => media::fetch_part(&url, asked.as_deref()),
    };
    (response, Some(whose))
}

/// Writes `response` to a connection, its body only where `body`: it lets
/// pages of the helper's origin read it, is marked no-store, and says
/// whether the connection closes after it.
fn send(out: &mut impl Write, response: &Response<Cow<'static, [u8]>>, body: bool, close: bool) -> std::io::Result<()> {
    let mut head = Vec::new();
    write!(head, "HTTP/1.1 {}\r\n", response.status())?;
    for (name, value) in response.headers() {
        write!(head, "{name}: ")?;
        head.extend_from_slice(value.as_bytes());
        head.extend_from_slice(b"\r\n");
    }
    let connection = if close { "close" } else { "keep-alive" };
    write!(head, "Content-Length: {}\r\nAccess-Control-Allow-Origin: {PAGE_ORIGIN}\r\nCache-Control: no-store\r\nConnection: {connection}\r\n\r\n", response.body().len())?;
    out.write_all(&head)?;
    if body {
        out.write_all(response.body())?;
    }
    out.flush()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read;
    use std::time::Duration;

    /// The addresses a script gives a page, by node.
    fn addresses_in(script: &str) -> Map<String, Json> {
        let given = script.strip_prefix("pwrsRuntime.clips(").and_then(|s| s.strip_suffix(", null);")).expect("a script giving addresses");
        match serde_json::from_str::<Json>(given).expect("the addresses as JSON") {
            Json::Object(map) => map,
            other => panic!("the addresses are {other}"),
        }
    }

    /// A connection to the server at `address`, and the token it names.
    fn connect(address: &str) -> (TcpStream, String) {
        let (port, token) = address.strip_prefix("http://127.0.0.1:").and_then(|rest| rest.split_once("/media/")).expect("an address of the clip server");
        let stream = TcpStream::connect((Ipv4Addr::LOCALHOST, port.parse::<u16>().expect("a port"))).expect("a connection to the clip server");
        stream.set_read_timeout(Some(Duration::from_secs(10))).expect("a read timeout");
        (stream, token.to_string())
    }

    /// Sends `request` on `stream` and reads its response: the head, and the
    /// body of the length the head gives, none for a HEAD.
    fn exchange(stream: &mut TcpStream, request: &str) -> (String, Vec<u8>) {
        stream.write_all(request.as_bytes()).expect("the request sent");
        let mut reader = BufReader::new(stream.try_clone().expect("the connection read"));
        let mut head = String::new();
        loop {
            let mut line = String::new();
            if reader.read_line(&mut line).expect("the head read") == 0 || line == "\r\n" {
                break;
            }
            head.push_str(&line);
        }
        let length = head.lines().find_map(|l| l.strip_prefix("Content-Length: ")).map_or(0, |n| n.parse::<usize>().expect("a length"));
        let mut body = vec![0; if request.starts_with("HEAD ") { 0 } else { length }];
        reader.read_exact(&mut body).expect("the body read");
        (head, body)
    }

    #[test]
    fn a_player_s_clip_is_served_by_its_token_until_the_token_ends() {
        let dir = std::env::temp_dir().join(format!("pwrs-gui-clip-server-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("a folder for the clips");
        let (first, second) = (dir.join("first.mp4"), dir.join("second.mp4"));
        std::fs::write(&first, b"0123456789").expect("the first clip");
        std::fs::write(&second, b"abcdefghij").expect("the second clip");
        let window = "clip-server-test";
        let mut found = HashMap::from([(3, Clip::File(first.clone()))]);
        let script = follow(window, &found).expect("the page is given its player's address");
        let address = addresses_in(&script)["3"].as_str().expect("player 3's address").to_string();
        let (mut stream, token) = connect(&address);
        assert!(token.len() == 32 && token.bytes().all(|b| HEX.contains(&b)), "128 bits in hex: {token}");
        assert_eq!(follow(window, &found), None, "a player keeps its token while its clip stays the same");
        assert_eq!(addresses(window), Some(script.clone()), "a page loaded again is given the same");

        let (head, body) = exchange(&mut stream, &format!("GET /media/{token}?1 HTTP/1.1\r\nRange: bytes=2-5\r\nOrigin: {PAGE_ORIGIN}\r\n\r\n"));
        assert!(head.starts_with("HTTP/1.1 206 Partial Content\r\n"), "{head}");
        let origin = format!("Access-Control-Allow-Origin: {PAGE_ORIGIN}");
        for line in ["content-range: bytes 2-5/10", "content-type: video/mp4", origin.as_str(), "Cache-Control: no-store", "Connection: keep-alive"] {
            assert!(head.lines().any(|l| l == line), "{line} in {head}");
        }
        assert_eq!(body, b"2345");
        let (head, body) = exchange(&mut stream, &format!("HEAD /media/{token}?1 HTTP/1.1\r\n\r\n"));
        assert!(head.starts_with("HTTP/1.1 200 OK\r\n") && head.contains("Content-Length: 10\r\n"), "{head}");
        assert!(body.is_empty(), "a HEAD gets no body");
        let (head, body) = exchange(&mut stream, &format!("GET /media/{token}?2 HTTP/1.1\r\nRange: bytes=8-\r\n\r\n"));
        assert!(head.contains("content-range: bytes 8-9/10"), "{head}");
        assert_eq!(body, b"89", "the same connection serves the next request");

        let (mut unknown, _) = connect(&address);
        let (head, _) = exchange(&mut unknown, "GET /media/00000000000000000000000000000000 HTTP/1.1\r\n\r\n");
        assert!(head.starts_with("HTTP/1.1 404 Not Found\r\n") && head.contains("Connection: close\r\n"), "{head}");
        assert_eq!(unknown.read(&mut [0; 1]).expect("the connection's end"), 0, "a connection is closed after an unknown token");
        let (mut posted, _) = connect(&address);
        let (head, _) = exchange(&mut posted, &format!("POST /media/{token} HTTP/1.1\r\nContent-Length: 0\r\n\r\n"));
        assert!(head.starts_with("HTTP/1.1 405 ") && head.contains("Connection: close\r\n"), "{head}");

        found.insert(3, Clip::File(second.clone()));
        let script = follow(window, &found).expect("a new clip gets a new address");
        let renewed = addresses_in(&script)["3"].as_str().expect("player 3's new address").to_string();
        assert_ne!(renewed, address);
        assert_eq!(stream.read(&mut [0; 1]).expect("the connection's end"), 0, "a connection whose token ended is shut");
        let (mut ended, _) = connect(&address);
        assert!(exchange(&mut ended, &format!("GET /media/{token} HTTP/1.1\r\n\r\n")).0.starts_with("HTTP/1.1 404 "), "an ended token is not served");

        let (mut fresh, renewed_token) = connect(&renewed);
        let (head, body) = exchange(&mut fresh, &format!("GET /media/{renewed_token} HTTP/1.1\r\nRange: bytes=0-1\r\n\r\n"));
        assert!(head.starts_with("HTTP/1.1 206 "), "{head}");
        assert_eq!(body, b"ab");
        forget(window);
        assert_eq!(fresh.read(&mut [0; 1]).expect("the connection's end"), 0, "a window's end shuts its connections");
        assert_eq!(addresses(window), None, "a window gone is given nothing");
        assert_eq!(unserved(), None, "the port was bound");
        std::fs::remove_file(&first).expect("the first clip removed");
        std::fs::remove_file(&second).expect("the second clip removed");
        std::fs::remove_dir(&dir).expect("their folder removed");
    }

    #[test]
    fn a_request_gives_its_method_target_range_origin_and_whether_its_connection_closes() {
        let read = |text: &str| read_request(&mut text.as_bytes()).expect("a request read").expect("a request");
        let request = read("GET /media/ab?3 HTTP/1.1\r\nRange: bytes=0-\r\norigin: pwrs://localhost\r\nConnection: keep-alive, Upgrade\r\n\r\n");
        assert_eq!(
            (request.method.as_str(), request.target.as_str(), request.range.as_deref(), request.origin.as_deref(), request.close),
            ("GET", "/media/ab?3", Some("bytes=0-"), Some("pwrs://localhost"), false)
        );
        assert!(read("GET / HTTP/1.0\r\n\r\n").close, "an HTTP/1.0 connection closes unless it says otherwise");
        assert!(!read("GET / HTTP/1.0\r\nConnection: Keep-Alive\r\n\r\n").close);
        assert!(read("HEAD / HTTP/1.1\r\nConnection: close\r\n\r\n").close);
        assert!(read_request(&mut "GET /\r\n\r\n".as_bytes()).is_err(), "a request line with no version");
        assert!(read_request(&mut "GET / HTTP/1.1\r\nnonsense\r\n\r\n".as_bytes()).is_err(), "a line that is no header");
        assert!(read_request(&mut "GET / HTTP/1.1\r\n".as_bytes()).is_err(), "headers that never end");
        assert!(read_request(&mut "".as_bytes()).expect("a closed connection").is_none());
    }

    #[test]
    fn a_page_is_given_its_players_addresses_or_why_it_has_none() {
        assert_eq!(script(Ok(BTreeMap::new())), NONE);
        assert_eq!(script(Ok(BTreeMap::from([(2, String::from("http://127.0.0.1:9/media/ab"))]))), r#"pwrsRuntime.clips({"2":"http://127.0.0.1:9/media/ab"}, null);"#);
        assert_eq!(script(Err(String::from("no port"))), r#"pwrsRuntime.clips({}, "no port");"#);
    }
}
