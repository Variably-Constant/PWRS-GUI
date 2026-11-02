//! The pictures of a Markdown document's https images, which the helper
//! fetches itself: over rustls on aws-lc-rs, the server's certificate
//! verified against the Mozilla roots webpki-root-certs includes and the
//! system's own, on Windows the current user's Root store as SecureFetch
//! reads it, elsewhere the roots in OpenSSL's files or macOS's Keychain.
//! Redirects are followed to https addresses alone, a proxy the environment
//! names is gone through, and a body is read whatever its size.

use crate::egui;
use arc_swap::ArcSwap;
use egui::load::{Bytes, BytesLoadResult, BytesLoader, BytesPoll, LoadError};
use std::collections::HashMap;
use std::sync::{Arc, OnceLock};

/// What an address's fetch has come to.
#[derive(Clone)]
enum Fetched {
    /// Still being fetched.
    Pending,
    /// The body, and the media type the server gave it.
    Ready { bytes: Arc<[u8]>, mime: Option<String> },
    /// Why it could not be fetched.
    Failed(String),
}

/// A fetch's body and media type, or why there is none.
type Fetch = Result<(Arc<[u8]>, Option<String>), String>;

/// Fetches the pictures egui requests by an `https://` address, each on a
/// thread of its own; one that cannot be fetched is a loading error, which
/// the image showing it reports. The address's fragment names that image,
/// so two images of one picture are fetched and forgotten apart; it is
/// never sent. The roots passed over wait in its notes for the renderer's
/// log.
pub struct WebLoader {
    fetched: Arc<ArcSwap<HashMap<String, Fetched>>>,
    agent: Arc<OnceLock<Result<ureq::Agent, String>>>,
    notes: Arc<ArcSwap<Vec<String>>>,
}

impl WebLoader {
    const ID: &'static str = egui::generate_loader_id!(WebLoader);

    pub fn new() -> WebLoader {
        WebLoader { fetched: Arc::new(ArcSwap::from_pointee(HashMap::new())), agent: Arc::new(OnceLock::new()), notes: Arc::new(ArcSwap::from_pointee(Vec::new())) }
    }

    /// The notes written since the last call, oldest first.
    pub fn take_notes(&self) -> Vec<String> {
        if self.notes.load().is_empty() {
            return Vec::new();
        }
        Arc::unwrap_or_clone(self.notes.swap(Arc::new(Vec::new())))
    }

    /// Enters `uri` as being fetched; returns whether it was absent, so
    /// that only the first request starts its fetch.
    fn begin(&self, uri: &str) -> bool {
        let mut absent = false;
        self.fetched.rcu(|m| {
            absent = !m.contains_key(uri);
            if !absent {
                return Arc::clone(m);
            }
            let mut m = (**m).clone();
            m.insert(uri.to_string(), Fetched::Pending);
            Arc::new(m)
        });
        absent
    }

    /// Fetches `uri` on a thread of its own, which wakes egui when done.
    fn start(&self, ctx: &egui::Context, uri: &str) {
        let (fetched, agent, notes, ctx, owned) = (Arc::clone(&self.fetched), Arc::clone(&self.agent), Arc::clone(&self.notes), ctx.clone(), uri.to_string());
        let spawned = std::thread::Builder::new().name(String::from("pwrs-gui-picture")).spawn(move || {
            let result = match agent.get_or_init(|| make_agent(&notes)) {
                Ok(agent) => fetch(agent, address_of(&owned)),
                Err(why) => Err(why.clone()),
            };
            if settle(&fetched, &owned, result) {
                ctx.request_repaint();
            }
        });
        match spawned {
            Ok(_running) => {}
            Err(e) => {
                settle(&self.fetched, uri, Err(format!("no thread could be started to fetch it: {e}")));
            }
        }
    }
}

impl BytesLoader for WebLoader {
    fn id(&self) -> &str {
        Self::ID
    }

    fn load(&self, ctx: &egui::Context, uri: &str) -> BytesLoadResult {
        if !uri.starts_with("https://") {
            return Err(LoadError::NotSupported);
        }
        if let Some(found) = self.fetched.load().get(uri) {
            return match found {
                Fetched::Pending => Ok(BytesPoll::Pending { size: None }),
                Fetched::Ready { bytes, mime } => Ok(BytesPoll::Ready { size: None, bytes: Bytes::Shared(Arc::clone(bytes)), mime: mime.clone() }),
                Fetched::Failed(why) => Err(LoadError::Loading(why.clone())),
            };
        }
        if self.begin(uri) {
            self.start(ctx, uri);
        }
        Ok(BytesPoll::Pending { size: None })
    }

    fn forget(&self, uri: &str) {
        if !self.fetched.load().contains_key(uri) {
            return;
        }
        self.fetched.rcu(|m| {
            let mut m = (**m).clone();
            m.remove(uri);
            m
        });
    }

    fn forget_all(&self) {
        self.fetched.store(Arc::new(HashMap::new()));
    }

    fn byte_size(&self) -> usize {
        let held = |f: &Fetched| match f {
            Fetched::Pending => 0,
            Fetched::Ready { bytes, mime } => bytes.len() + mime.as_ref().map_or(0, String::len),
            Fetched::Failed(why) => why.len(),
        };
        self.fetched.load().values().map(held).sum()
    }

    fn has_pending(&self) -> bool {
        self.fetched.load().values().any(|f| matches!(f, Fetched::Pending))
    }
}

/// The address `uri` fetches: itself without its fragment.
fn address_of(uri: &str) -> &str {
    match uri.split_once('#') {
        Some((address, _image)) => address,
        None => uri,
    }
}

/// Enters `uri`'s fetch as it came to, unless egui forgot the address while
/// it was being fetched; returns whether it was entered.
fn settle(fetched: &ArcSwap<HashMap<String, Fetched>>, uri: &str, result: Fetch) -> bool {
    let entry = match result {
        Ok((bytes, mime)) => Fetched::Ready { bytes, mime },
        Err(why) => Fetched::Failed(why),
    };
    let mut kept = false;
    fetched.rcu(|m| {
        kept = m.contains_key(uri);
        if !kept {
            return Arc::clone(m);
        }
        let mut m = (**m).clone();
        m.insert(uri.to_string(), entry.clone());
        Arc::new(m)
    });
    kept
}

/// The body at `url` and its media type, read whatever its size.
fn fetch(agent: &ureq::Agent, url: &str) -> Fetch {
    let mut response = match agent.get(url).call() {
        Ok(response) => response,
        Err(ureq::Error::StatusCode(code)) => return Err(format!("the server returned status {code}")),
        Err(e) => return Err(e.to_string()),
    };
    let mime = response.body().mime_type().map(str::to_string);
    let bytes = response.body_mut().with_config().read_to_vec().map_err(|e| e.to_string())?;
    Ok((bytes.into(), mime))
}

/// The agent every fetch goes through: https alone, redirects included,
/// verifying servers against [`roots`]; the roots it passed over go to
/// `notes`.
fn make_agent(notes: &ArcSwap<Vec<String>>) -> Result<ureq::Agent, String> {
    let (roots, passed_over) = roots()?;
    if !passed_over.is_empty() {
        notes.rcu(|v| {
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
                passed_over.push(format!("https pictures pass over the Windows root {name}: its uses cannot be read: {e}"));
                continue;
            }
        };
        let current = match certificate.is_time_valid() {
            Ok(current) => current,
            Err(e) => {
                passed_over.push(format!("https pictures pass over the Windows root {name}: its validity cannot be read: {e}"));
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
            Err(e) => passed_over.push(format!("https pictures pass over the Windows root {name}: rustls cannot take it as a root: {e}")),
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
    let mut passed_over: Vec<String> = found.errors.iter().map(|e| format!("https pictures pass over the system's roots it could not read: {e}")).collect();
    for der in found.certs {
        match rustls::RootCertStore::empty().add(der.clone()) {
            Ok(()) => roots.push(ureq::tls::Certificate::from_der(der.as_ref()).to_owned()),
            Err(e) => passed_over.push(format!("https pictures pass over a root of the system's that rustls cannot take: {e}")),
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
    fn a_fetch_is_entered_once_and_dropped_when_its_address_was_forgotten_meanwhile() {
        let loader = WebLoader::new();
        let ctx = egui::Context::default();
        let uri = "https://example.invalid/a.png#pwrs-gui-picture-1";
        assert!(loader.begin(uri));
        assert!(!loader.begin(uri), "a second request starts no second fetch");
        match loader.load(&ctx, uri) {
            Ok(BytesPoll::Pending { .. }) => {}
            Ok(BytesPoll::Ready { .. }) => panic!("a picture still being fetched is not ready"),
            Err(e) => panic!("a picture still being fetched is pending, not an error: {e}"),
        }
        let png: Fetch = Ok((Arc::from(&b"png"[..]), Some(String::from("image/png"))));
        assert!(settle(&loader.fetched, uri, png), "a fetch of an address still requested is entered");
        match loader.load(&ctx, uri) {
            Ok(BytesPoll::Ready { bytes, mime, .. }) => {
                assert_eq!(&*bytes, b"png");
                assert_eq!(mime.as_deref(), Some("image/png"));
            }
            Ok(BytesPoll::Pending { .. }) => panic!("the fetched picture is ready, not pending"),
            Err(e) => panic!("the fetched picture is ready, not an error: {e}"),
        }
        loader.forget(uri);
        assert!(!settle(&loader.fetched, uri, Err(String::from("too late"))), "a fetch of a forgotten address is dropped");
        assert!(!loader.fetched.load().contains_key(uri));
        assert_eq!(address_of(uri), "https://example.invalid/a.png");
    }
}
