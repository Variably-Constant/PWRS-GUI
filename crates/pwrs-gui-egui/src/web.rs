//! The pictures of a Markdown document's https images, which the helper
//! fetches itself through [`pwrs_gui_fetch`], the loader both helpers link:
//! over rustls on aws-lc-rs, the server's certificate verified against the
//! Mozilla roots and the system's own, redirects followed to https
//! addresses alone, and a body read whatever its size.

use crate::egui;
use arc_swap::ArcSwap;
use egui::load::{Bytes, BytesLoadResult, BytesLoader, BytesPoll, LoadError};
use std::collections::HashMap;
use std::sync::Arc;

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
/// never sent. The roots the loader passed over wait in its notes for the
/// renderer's log.
pub struct WebLoader {
    fetched: Arc<ArcSwap<HashMap<String, Fetched>>>,
}

impl WebLoader {
    const ID: &'static str = egui::generate_loader_id!(WebLoader);

    pub fn new() -> WebLoader {
        WebLoader { fetched: Arc::new(ArcSwap::from_pointee(HashMap::new())) }
    }

    /// The notes written since the last call, oldest first.
    pub fn take_notes(&self) -> Vec<String> {
        pwrs_gui_fetch::take_notes()
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
        let (fetched, ctx, owned) = (Arc::clone(&self.fetched), ctx.clone(), uri.to_string());
        let spawned = std::thread::Builder::new().name(String::from("pwrs-gui-picture")).spawn(move || {
            let result = pwrs_gui_fetch::fetch(address_of(&owned));
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
        if !pwrs_gui_fetch::is_web(uri) {
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

#[cfg(test)]
mod tests {
    use super::*;

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
