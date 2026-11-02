//! A FilePicker's dialog: the system's own through pwrs-gui-dialogs, opened
//! when its page's Browse button is pressed. Its answer reaches the loop as
//! an event, so nothing waits on it, and it stays open only while the
//! window is: dropping it closes the dialog. Where the system has no
//! dialog of the kind, the window's command has the egui renderer draw it.

use super::{UserEvent, post};
use pwrs_gui_proto::{DialogKind, NodeId, WindowId};
use tao::event_loop::EventLoopProxy;

/// A file chooser's dialog while it is open: the system's, or, once the
/// system has none, none here while the window's command has it drawn; and
/// what it requests, which the command is given.
pub(super) struct Picking {
    pub dialog: Option<pwrs_gui_dialogs::Owned>,
    pub title: String,
    pub kind: DialogKind,
}

/// Opens the system's dialog of `title` and `kind` for file chooser `node`
/// of window `window`, without taking the focus when `focus` is false; its
/// answer comes back as [`UserEvent::Chosen`].
pub(super) fn open(proxy: &EventLoopProxy<UserEvent>, window: WindowId, node: NodeId, title: String, kind: DialogKind, focus: bool) -> Picking {
    let proxy = proxy.clone();
    let dialog = pwrs_gui_dialogs::open(title.clone(), kind.clone(), focus, None, Box::new(move |outcome| post(&proxy, UserEvent::Chosen(window, node, outcome))));
    Picking { dialog: Some(dialog), title, kind }
}
