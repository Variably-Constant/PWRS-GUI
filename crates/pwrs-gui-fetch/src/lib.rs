//! PWRS-GUI's https loader, which both helpers link: a Markdown document's
//! pictures fetched whole, and a media clip read by its byte ranges, for
//! its header and for the player that plays it. Every fetch goes over
//! rustls on aws-lc-rs, the server's certificate verified against the
//! Mozilla roots webpki-root-certs includes and the system's own: on
//! Windows the current user's Root store as SecureFetch reads it, elsewhere
//! the roots in OpenSSL's files or macOS's Keychain. Redirects are followed
//! to https addresses alone, a proxy the environment names is gone through,
//! and no Windows logon is offered to a server.

use arc_swap::ArcSwap;
use pwrs_gui_proto::media::probe;
use std::io::Read;
use std::sync::{Arc, LazyLock, OnceLock};

/// The agent every fetch goes through, made at the first fetch; or why it
/// cannot be.
static AGENT: OnceLock<Result<ureq::Agent, String>> = OnceLock::new();

/// The roots of the system's the agent passed over, until a helper takes
/// them for its log.
static NOTES: LazyLock<ArcSwap<Vec<String>>> = LazyLock::new(|| ArcSwap::from_pointee(Vec::new()));

/// Whether `address` is one the loader fetches: an https address.
pub fn is_web(address: &str) -> bool {
    address.starts_with("https://")
}

/// The notes written since the last call, oldest first.
pub fn take_notes() -> Vec<String> {
    if NOTES.load().is_empty() {
        return Vec::new();
    }
    Arc::unwrap_or_clone(NOTES.swap(Arc::new(Vec::new())))
}

/// The body at `url` and its media type, read whatever its size.
pub fn fetch(url: &str) -> Result<(Arc<[u8]>, Option<String>), String> {
    let mut response = agent()?.get(url).call().map_err(|e| failed(url, e))?;
    let mime = response.body().mime_type().map(str::to_string);
    let bytes = response.body_mut().with_config().read_to_vec().map_err(|e| format!("{url} could not be read: {e}"))?;
    Ok((bytes.into(), mime))
}

/// A part of a body fetched by its byte range: its bytes, and the whole
/// body's length where the server gives it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Part {
    pub bytes: Vec<u8>,
    pub total: Option<u64>,
}

/// The `count` bytes of the body at `url` from `start`, fewer at its end,
/// none past it. A server that answers with the whole body is read from its
/// start alone; asked for a later range, it is refused, as a clip it serves
/// cannot be sought in.
pub fn fetch_range(url: &str, start: u64, count: u64) -> Result<Part, String> {
    if count == 0 {
        return Ok(Part { bytes: Vec::new(), total: None });
    }
    let end = start.saturating_add(count - 1);
    let mut response = match agent()?.get(url).header("Range", format!("bytes={start}-{end}")).call() {
        Ok(r) => r,
        Err(ureq::Error::StatusCode(416)) => return Ok(Part { bytes: Vec::new(), total: None }),
        Err(e) => return Err(failed(url, e)),
    };
    let total = match response.status().as_u16() {
        206 => match header(&response, "content-range")? {
            Some(range) => total_of(range)?,
            None => None,
        },
        200 if start == 0 => match header(&response, "content-length")? {
            Some(length) => Some(length.parse::<u64>().map_err(|e| format!("{url} gave its length as {length:?}: {e}"))?),
            None => None,
        },
        200 => return Err(format!("{url} is served whole, with no byte ranges, so its clip cannot be read from {start}")),
        other => return Err(format!("{url} answered a range with status {other}")),
    };
    let mut bytes = Vec::new();
    response.body_mut().as_reader().take(count).read_to_end(&mut bytes).map_err(|e| format!("{url} could not be read from {start}: {e}"))?;
    Ok(Part { bytes, total })
}

/// The part a [`Remote`] fetches first, and again after a seek: 64 KiB, as
/// the header reader's first read.
pub const FIRST_PART: u64 = 64 * 1024;

/// The most a [`Remote`]'s part grows to while reads run on in order:
/// 1 MiB, as the header reader's widest read.
pub const MOST_PART: u64 = 1024 * 1024;

/// How a [`Remote`] fetches a part: the bytes from a start, as many as
/// asked for or fewer at the end.
type Fetcher = Box<dyn FnMut(u64, u64) -> Result<Part, String> + Send>;

/// A clip at an https address read as a file is read, by its byte ranges:
/// its length is read with its first part, and a read beyond the part
/// fetched last fetches the part from there. A part is [`FIRST_PART`]
/// bytes, or as many as the read asks for where it asks for more, and
/// doubles up to [`MOST_PART`] while each part follows the one before it;
/// a read elsewhere, as after a seek, starts again from the first size.
pub struct Remote {
    fetch: Fetcher,
    length: u64,
    at: u64,
    /// Where the part fetched last starts, and its bytes.
    start: u64,
    part: Vec<u8>,
    /// The size of the next part, where it follows the one fetched last.
    next: u64,
}

impl Remote {
    /// Opens the clip at `url`, fetching its first part and its length.
    pub fn open(url: &str) -> Result<Remote, String> {
        let address = url.to_string();
        Remote::with(Box::new(move |start, count| fetch_range(&address, start, count))).map_err(|why| format!("{url}: {why}"))
    }

    /// Opens a clip whose parts `fetch` fetches.
    fn with(mut fetch: Fetcher) -> Result<Remote, String> {
        let first = fetch(0, FIRST_PART)?;
        let length = first.total.ok_or_else(|| String::from("the server gives no length for it, so it cannot be read by its parts"))?;
        Ok(Remote { fetch, length, at: 0, start: 0, part: first.bytes, next: (FIRST_PART * 2).min(MOST_PART) })
    }

    /// The clip's length in bytes.
    pub fn length(&self) -> u64 {
        self.length
    }

    /// Where the next read starts.
    pub fn position(&self) -> u64 {
        self.at
    }

    /// Moves where the next read starts to `at`; past the end, a read gives
    /// nothing.
    pub fn seek_to(&mut self, at: u64) {
        self.at = at;
    }

    /// Reads into `into` from the position, as many bytes as it holds or
    /// fewer at the end, and moves the position past them; how many it read,
    /// 0 at the end.
    pub fn read_into(&mut self, into: &mut [u8]) -> Result<usize, String> {
        if into.is_empty() || self.at >= self.length {
            return Ok(0);
        }
        let end = self.start + self.part.len() as u64;
        if !(self.start..end).contains(&self.at) {
            let size = if self.at == end {
                let size = self.next;
                self.next = (size * 2).min(MOST_PART);
                size
            } else {
                self.next = (FIRST_PART * 2).min(MOST_PART);
                FIRST_PART
            };
            let count = (into.len() as u64).max(size).min(self.length - self.at);
            let part = (self.fetch)(self.at, count)?;
            if part.bytes.is_empty() {
                return Err(format!("the server gave nothing from byte {} of the {} it said the clip holds", self.at, self.length));
            }
            self.start = self.at;
            self.part = part.bytes;
        }
        let from = (self.at - self.start) as usize;
        let n = into.len().min(self.part.len() - from);
        into[..n].copy_from_slice(&self.part[from..from + n]);
        self.at += n as u64;
        Ok(n)
    }
}

impl std::io::Read for Remote {
    fn read(&mut self, into: &mut [u8]) -> std::io::Result<usize> {
        self.read_into(into).map_err(std::io::Error::other)
    }
}

impl std::io::Seek for Remote {
    fn seek(&mut self, to: std::io::SeekFrom) -> std::io::Result<u64> {
        let at = match to {
            std::io::SeekFrom::Start(n) => Some(n),
            std::io::SeekFrom::End(d) => self.length.checked_add_signed(d),
            std::io::SeekFrom::Current(d) => self.at.checked_add_signed(d),
        };
        let at = at.ok_or_else(|| std::io::Error::new(std::io::ErrorKind::InvalidInput, format!("{to:?} from byte {} of {} falls before the clip's start", self.at, self.length)))?;
        self.seek_to(at);
        Ok(at)
    }
}

/// Whether the server of `url` lets a page of `origin` read it, as a
/// browser asks under CORS: its answer to a request naming the origin
/// allows any origin, or that one, in its `Access-Control-Allow-Origin`.
pub fn allows(url: &str, origin: &str) -> Result<bool, String> {
    let response = agent()?.get(url).header("Origin", origin).header("Range", "bytes=0-0").call().map_err(|e| failed(url, e))?;
    Ok(match header(&response, "access-control-allow-origin")? {
        Some(allowed) => allowed.trim() == "*" || allowed.trim() == origin,
        None => false,
    })
}

/// The format of the clip at `address`, a file's path or an https address,
/// as its headers name it: a file's read from the file, an https clip's by
/// byte ranges, the first range giving the clip's length.
pub fn clip_format(address: &str) -> Result<probe::Format, String> {
    if !is_web(address) {
        return probe::read(std::path::Path::new(address));
    }
    let length = fetch_range(address, 0, 1)?.total;
    probe::read_ranges(length, &mut |at, count| fetch_range(address, at, count).map(|part| part.bytes))
}

/// The edit lists of the clip at `address`, a file's path or an https
/// address, read as [`clip_format`] reads its format.
pub fn clip_edits(address: &str) -> Result<probe::Edits, String> {
    if !is_web(address) {
        return probe::edits(std::path::Path::new(address));
    }
    let length = fetch_range(address, 0, 1)?.total;
    probe::edits_ranges(length, &mut |at, count| fetch_range(address, at, count).map(|part| part.bytes))
}

/// The text of `response`'s header `name`; none where the server sent none.
fn header<'a, B>(response: &'a ureq::http::Response<B>, name: &str) -> Result<Option<&'a str>, String> {
    match response.headers().get(name) {
        None => Ok(None),
        Some(v) => v.to_str().map(Some).map_err(|e| format!("the server's {name} header is not text: {e}")),
    }
}

/// The whole body's length a `Content-Range` header gives, as in
/// `bytes 0-99/1000`; none where the server writes `*` for it.
fn total_of(header: &str) -> Result<Option<u64>, String> {
    let (_, total) = header.rsplit_once('/').ok_or_else(|| format!("a Content-Range of {header:?} gives no length"))?;
    match total.trim() {
        "*" => Ok(None),
        n => n.parse::<u64>().map(Some).map_err(|e| format!("a Content-Range of {header:?} gives a length that is no number: {e}")),
    }
}

/// Why a fetch of `url` failed, in words.
fn failed(url: &str, e: ureq::Error) -> String {
    match e {
        ureq::Error::StatusCode(code) => format!("the server returned status {code} for {url}"),
        other => format!("{url} could not be fetched: {other}"),
    }
}

/// The agent every fetch goes through.
fn agent() -> Result<&'static ureq::Agent, String> {
    AGENT.get_or_init(make_agent).as_ref().map_err(String::clone)
}

/// An agent of https alone, redirects included, verifying servers against
/// [`roots`]; the roots it passed over go to the notes.
fn make_agent() -> Result<ureq::Agent, String> {
    let (roots, passed_over) = roots()?;
    if !passed_over.is_empty() {
        NOTES.rcu(|v| {
            let mut v = (**v).clone();
            v.extend(passed_over.iter().cloned());
            v
        });
    }
    let tls = ureq::tls::TlsConfig::builder()
        .provider(ureq::tls::TlsProvider::Rustls)
        .root_certs(ureq::tls::RootCerts::Specific(Arc::new(roots)))
        .unversioned_rustls_crypto_provider(Arc::new(rustls::crypto::aws_lc_rs::default_provider()))
        .build();
    Ok(ureq::Agent::config_builder().tls_config(tls).https_only(true).build().new_agent())
}

/// The roots a server's certificate is verified against: the Mozilla roots
/// webpki-root-certs includes, and those [`system_roots`] adds, with a note
/// for each root of the system's passed over.
fn roots() -> Result<(Vec<ureq::tls::Certificate<'static>>, Vec<String>), String> {
    let mut roots: Vec<ureq::tls::Certificate<'static>> = webpki_root_certs::TLS_SERVER_ROOT_CERTS.iter().map(|c| ureq::tls::Certificate::from_der(c.as_ref())).collect();
    let passed_over = system_roots(&mut roots)?;
    Ok((roots, passed_over))
}

/// The object identifier of server authentication, the extended key usage
/// a root must allow to be taken from the Windows store.
#[cfg(windows)]
const SERVER_AUTHENTICATION: &str = "1.3.6.1.5.5.7.3.1";

/// Adds to `roots` the certificates of the current user's Root store, which
/// also shows the machine's and Group Policy's, that Windows lets
/// authenticate servers and that are valid now. A store that cannot be
/// opened is an error; an entry whose uses or validity cannot be read, or
/// that rustls cannot take as a root, is passed over, and the notes
/// returned name each by its thumbprint.
#[cfg(windows)]
fn system_roots(roots: &mut Vec<ureq::tls::Certificate<'static>>) -> Result<Vec<String>, String> {
    use schannel::cert_context::{HashAlgorithm, ValidUses};
    let store = schannel::cert_store::CertStore::open_current_user("ROOT").map_err(|e| format!("the current user's Root certificate store cannot be opened: {e}"))?;
    let mut passed_over = Vec::new();
    for certificate in store.certs() {
        let name = match certificate.fingerprint(HashAlgorithm::sha1()) {
            Ok(sha1) => format!("with SHA-1 thumbprint {}", sha1.iter().map(|b| format!("{b:02X}")).collect::<String>()),
            Err(e) => format!("whose thumbprint cannot be read ({e})"),
        };
        let uses = match certificate.valid_uses() {
            Ok(uses) => uses,
            Err(e) => {
                passed_over.push(format!("https fetches pass over the Windows root {name}: its uses cannot be read: {e}"));
                continue;
            }
        };
        let current = match certificate.is_time_valid() {
            Ok(current) => current,
            Err(e) => {
                passed_over.push(format!("https fetches pass over the Windows root {name}: its validity cannot be read: {e}"));
                continue;
            }
        };
        let serves = match uses {
            ValidUses::All => true,
            ValidUses::Oids(oids) => oids.iter().any(|oid| oid == SERVER_AUTHENTICATION),
        };
        if !(current && serves) {
            continue;
        }
        let der = certificate.to_der();
        match rustls::RootCertStore::empty().add(rustls::pki_types::CertificateDer::from(der)) {
            Ok(()) => roots.push(ureq::tls::Certificate::from_der(der).to_owned()),
            Err(e) => passed_over.push(format!("https fetches pass over the Windows root {name}: rustls cannot take it as a root: {e}")),
        }
    }
    Ok(passed_over)
}

/// Adds to `roots` the system's own roots, as rustls-native-certs reads
/// them: the files `SSL_CERT_FILE` and `SSL_CERT_DIR` name, or else the
/// bundle OpenSSL reads, or macOS's Keychain. What cannot be read, and a
/// root rustls cannot take, is passed over, and the notes returned say so;
/// the bundled roots still verify servers then.
#[cfg(not(windows))]
fn system_roots(roots: &mut Vec<ureq::tls::Certificate<'static>>) -> Result<Vec<String>, String> {
    let found = rustls_native_certs::load_native_certs();
    let mut passed_over: Vec<String> = found.errors.iter().map(|e| format!("https fetches pass over the system's roots it could not read: {e}")).collect();
    for der in found.certs {
        match rustls::RootCertStore::empty().add(der.clone()) {
            Ok(()) => roots.push(ureq::tls::Certificate::from_der(der.as_ref()).to_owned()),
            Err(e) => passed_over.push(format!("https fetches pass over a root of the system's that rustls cannot take: {e}")),
        }
    }
    Ok(passed_over)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn servers_are_verified_against_the_bundled_roots_and_the_systems_own() {
        let (roots, passed_over) = roots().expect("the roots are read");
        let bundled = webpki_root_certs::TLS_SERVER_ROOT_CERTS.len();
        assert!(bundled >= 100, "{bundled} bundled roots");
        assert!(roots.len() > bundled, "{} roots taken, {bundled} of them bundled, passed over: {passed_over:?}", roots.len());
    }

    #[test]
    fn a_remote_clip_is_read_by_parts_that_grow_in_order_and_start_again_at_a_seek() {
        use std::io::{Read, Seek, SeekFrom};
        const K: u64 = 1024;
        let body: Arc<Vec<u8>> = Arc::new((0..MOST_PART * 4 + 10).map(|i| (i % 251) as u8).collect());
        let (tell, told) = std::sync::mpsc::channel();
        let served = Arc::clone(&body);
        let fetch: Fetcher = Box::new(move |start, count| {
            tell.send((start, count)).map_err(|e| format!("the test stopped hearing: {e}"))?;
            let from = (start as usize).min(served.len());
            let to = (from + count as usize).min(served.len());
            Ok(Part { bytes: served[from..to].to_vec(), total: Some(served.len() as u64) })
        });
        let mut clip = Remote::with(fetch).expect("the clip opens");
        assert_eq!(clip.length(), body.len() as u64);
        let (mut chunk, mut read) = ([0u8; 4096], Vec::new());
        while (read.len() as u64) < 3 * MOST_PART {
            let n = clip.read(&mut chunk).expect("a few kilobytes are read in order");
            read.extend_from_slice(&chunk[..n]);
        }
        assert_eq!(&read[..], &body[..read.len()]);
        let sizes: Vec<u64> = told.try_iter().map(|(_, count)| count).collect();
        assert_eq!(sizes, [64 * K, 128 * K, 256 * K, 512 * K, MOST_PART, MOST_PART, MOST_PART], "parts double up to a mebibyte while they follow each other");
        clip.seek(SeekFrom::Start(10)).expect("a seek");
        let mut head = [0u8; 50];
        clip.read_exact(&mut head).expect("the head is read again");
        assert_eq!(&head[..], &body[10..60]);
        clip.read_exact(&mut read[..(FIRST_PART as usize)]).expect("on in order");
        let asked: Vec<(u64, u64)> = told.try_iter().collect();
        assert_eq!(asked, [(10, FIRST_PART), (10 + FIRST_PART, 2 * FIRST_PART)], "a seek starts the parts again from the first size");
        clip.seek(SeekFrom::End(-4)).expect("a seek from the end");
        let mut tail = Vec::new();
        clip.read_to_end(&mut tail).expect("the tail is read");
        assert_eq!(&tail[..], &body[body.len() - 4..]);
        assert!(clip.seek(SeekFrom::Current(-(body.len() as i64) - 1)).is_err(), "a seek before the start is refused");
        let nothing: Fetcher = Box::new(|_, _| Ok(Part { bytes: vec![1, 2, 3], total: None }));
        assert!(Remote::with(nothing).is_err(), "a clip whose length is not given cannot be read by parts");
    }

    #[test]
    fn a_content_range_gives_the_whole_length_or_none() {
        assert_eq!(total_of("bytes 0-99/1000").expect("a length"), Some(1000));
        assert_eq!(total_of("bytes 0-99/*").expect("no length"), None);
        assert!(total_of("bytes 0-99").is_err());
        assert!(total_of("bytes 0-99/many").is_err());
        assert!(is_web("https://example.com/a.mp4") && !is_web("C:\\clips\\a.mp4") && !is_web("http://example.com/a.mp4"));
    }
}
