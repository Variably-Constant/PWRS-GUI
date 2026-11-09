//! A renderer's link to its session, the same for every renderer: messages
//! arrive on [`Link::inbox`], and everything the renderer reports leaves
//! through the [`Sink`] the caller supplies, which sends to SubEtha rings in
//! a helper process and to in-memory channels on Windows' in-process path.

use crate::{FromHelper, ToHelper};
use std::any::Any;
use std::sync::Arc;
use std::sync::mpsc::Receiver;

/// Where a renderer's messages go. Every method runs on the renderer's
/// thread and must not wait on the session.
pub trait Sink {
    /// Something the session attached to a message, for the ring the
    /// message names: the in-process path's channels.
    fn attach(&mut self, ring: &str, attach: Box<dyn Any + Send>);
    /// `Hello` or `StartFailed`, to the start ring.
    fn hello(&mut self, msg: FromHelper);
    /// An event of the window whose ring is `ring`.
    fn window(&mut self, ring: &str, msg: FromHelper);
    /// A reply to a request, to the reply ring `ring`.
    fn reply(&mut self, ring: &str, msg: FromHelper);
    /// The ring `ring` is gone.
    fn forget(&mut self, ring: &str);
    /// `Log`, or an `Error` naming no window.
    fn log(&mut self, msg: FromHelper);
}

/// A message to a renderer, with what the session attached to it.
pub struct Inbound {
    pub msg: ToHelper,
    pub attach: Option<Box<dyn Any + Send>>,
}

impl Inbound {
    pub fn plain(msg: ToHelper) -> Inbound {
        Inbound { msg, attach: None }
    }
}

/// Wakes a renderer's event loop so it reads its inbox; callable from any
/// thread.
pub type WakeLoop = Arc<dyn Fn() + Send + Sync>;

/// A renderer's ends of its link to the session.
pub struct Link {
    pub inbox: Receiver<Inbound>,
    pub sink: Box<dyn Sink + Send>,
    /// Called once, when the loop can be woken, with the function that wakes
    /// it, so whoever feeds `inbox` can wake the loop after each message.
    pub on_ready: Box<dyn FnOnce(WakeLoop) + Send>,
}
