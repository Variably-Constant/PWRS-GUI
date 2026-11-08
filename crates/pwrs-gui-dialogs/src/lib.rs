//! PWRS-GUI's native dialogs: a message, files to open, a file to save or a
//! folder. Each dialog is stored under the request that opened it until it
//! closes, so a command that stops, or a test that answers it, can close it
//! the platform's own way; its command is then given the answer the close
//! gave rather than the dialog's own. Where the system has no dialog of the
//! kind, a renderer that can draws its own in its place, listed under the
//! same number ([`StandIn`]).

use arc_swap::{ArcSwap, ArcSwapOption};
use pwrs_gui_proto::{DialogAnswer, DialogFailure, DialogKind, DialogSummary, MessageButtons, RequestId};
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, LazyLock};

#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "macos")]
use macos as platform;
#[cfg(all(unix, not(target_vendor = "apple")))]
mod unix;
#[cfg(all(unix, not(target_vendor = "apple")))]
use unix as platform;
#[cfg(windows)]
mod windows;
#[cfg(windows)]
use windows as platform;

/// Where a dialog's answer goes once it closes, called on the thread the
/// dialog ran on: how it was answered, or why it gave none.
pub type Answered = Box<dyn FnOnce(Result<DialogAnswer, DialogFailure>) + Send>;

/// A dialog's answer, given once. Dropped without being given, as when the
/// dialog's thread cannot start or ends in a panic, it reports that the
/// dialog ended without an answer, so no command waits on it for ever.
pub(crate) struct Answer(Option<Answered>);

impl Answer {
    pub(crate) fn give(mut self, outcome: Result<DialogAnswer, DialogFailure>) {
        if let Some(answered) = self.0.take() {
            answered(outcome);
        }
    }
}

impl Drop for Answer {
    fn drop(&mut self) {
        if let Some(answered) = self.0.take() {
            answered(Err(DialogFailure::Failed(String::from("the dialog ended without an answer"))));
        }
    }
}

/// A dialog while it is open.
struct Open {
    kind: &'static str,
    title: String,
    /// A message's buttons; `None` for a file or folder dialog.
    buttons: Option<MessageButtons>,
    /// Whether a file dialog takes more than one path.
    multiple: bool,
    /// What closes the dialog, once its platform has started it or a
    /// renderer draws it.
    closer: ArcSwapOption<Closing>,
    /// The answer a close or a test gave, which the dialog's command is
    /// given in place of the dialog's own.
    answer: ArcSwapOption<DialogAnswer>,
}

/// What closes an open dialog: the platform's own way, or the way of the
/// renderer that draws it in the system's place.
enum Closing {
    Platform(platform::Closer),
    Drawn(Box<dyn Fn() + Send + Sync>),
}

impl Closing {
    fn close(&self) {
        match self {
            Closing::Platform(closer) => closer.close(),
            Closing::Drawn(close) => close(),
        }
    }

    /// Whether the dialog shows, which is when [`list`] lists it: a drawn
    /// one from when its renderer draws it.
    fn showing(&self) -> bool {
        match self {
            Closing::Platform(closer) => closer.showing(),
            Closing::Drawn(_) => true,
        }
    }
}

/// The dialogs open now, by the request that opened each.
static OPEN: LazyLock<ArcSwap<BTreeMap<RequestId, Arc<Open>>>> = LazyLock::new(|| ArcSwap::from_pointee(BTreeMap::new()));

/// What draws a dialog the system has none of, as the egui renderer does:
/// it is given the dialog on the thread that found the system lacks one,
/// and draws it where it draws.
pub type StandIn = Box<dyn FnOnce(Drawn) + Send>;

/// A dialog a renderer draws itself, where the system has none of its kind.
/// It stays listed and answerable under its number: [`Drawn::shown`] gives
/// the registry what closes it, and [`Drawn::give`] its answer. Dropped
/// without an answer, it reports that the dialog ended without one.
pub struct Drawn {
    request: RequestId,
    title: String,
    kind: DialogKind,
    why: String,
    open: Arc<Open>,
    done: Answer,
}

impl Drawn {
    /// The dialog's number.
    pub fn request(&self) -> RequestId {
        self.request
    }

    pub fn title(&self) -> &str {
        &self.title
    }

    pub fn kind(&self) -> &DialogKind {
        &self.kind
    }

    /// Why the system has no dialog of the kind.
    pub fn why(&self) -> &str {
        &self.why
    }

    /// Gives the registry what closes the drawn dialog at a close or a
    /// test's answer; when one came already, it closes the dialog at once.
    pub fn shown(&self, close: Box<dyn Fn() + Send + Sync>) {
        let closing = Arc::new(Closing::Drawn(close));
        self.open.closer.store(Some(Arc::clone(&closing)));
        if self.open.answer.load().is_some() {
            closing.close();
        }
    }

    /// Gives the drawn dialog's answer; an answer a close or a test gave is
    /// given in its place.
    pub fn give(self, outcome: Result<DialogAnswer, DialogFailure>) {
        self.done.give(outcome);
    }
}

/// Shows a dialog titled `title` for request `request`, and passes
/// `answered` how it was answered once it closes. With `focus` false the
/// dialog should open without taking the focus, as a renderer's windows do
/// under `PWRS_GUI_NO_FOCUS`; Windows and macOS honor that, and zenity and
/// the XDG portal take the focus as the desktop gives it. Where the system
/// has no dialog of the kind, `stand_in` is given the dialog to draw, and
/// without one the outcome says the system has none.
pub fn show(request: RequestId, title: String, kind: DialogKind, focus: bool, stand_in: Option<StandIn>, answered: Answered) {
    let (buttons, multiple) = match &kind {
        DialogKind::Message { buttons, .. } => (Some(*buttons), false),
        DialogKind::Open { multiple, .. } | DialogKind::Folder { multiple, .. } => (None, *multiple),
        DialogKind::Save { .. } => (None, false),
    };
    let open = Arc::new(Open { kind: kind.name(), title: title.clone(), buttons, multiple, closer: ArcSwapOption::empty(), answer: ArcSwapOption::empty() });
    OPEN.rcu(|m| {
        let mut m = (**m).clone();
        m.insert(request, open.clone());
        m
    });
    let shown = open.clone();
    let finished = open.clone();
    let done = Answer(Some(Box::new(move |outcome| {
        let outcome = match finished.answer.load_full() {
            Some(given) => Ok((*given).clone()),
            None => outcome,
        };
        OPEN.rcu(|m| {
            let mut m = (**m).clone();
            m.remove(&request);
            m
        });
        answered(outcome);
    })));
    let first = match stand_in {
        None => done,
        Some(stand_in) => {
            let (drawn_title, drawn_kind) = (title.clone(), kind.clone());
            Answer(Some(Box::new(move |outcome| match outcome {
                Err(DialogFailure::NoNative(why)) if open.answer.load().is_none() => stand_in(Drawn { request, title: drawn_title, kind: drawn_kind, why, open, done }),
                outcome => done.give(outcome),
            })))
        }
    };
    platform::show(
        title,
        kind,
        focus,
        Box::new(move |closer: platform::Closer| {
            let closer = Arc::new(Closing::Platform(closer));
            shown.closer.store(Some(closer.clone()));
            if shown.answer.load().is_some() {
                closer.close();
            }
        }),
        first,
    );
}

/// The bit set in the numbers of the dialogs a renderer opens for itself:
/// bit 63, above every session's request numbers, which give a taking
/// module's index from bit 40 and an HTML session's mark at bit 62.
const OWN: RequestId = 1 << 63;

/// The number below [`OWN`] of the next dialog a renderer opens for itself.
static NEXT_OWN: AtomicU64 = AtomicU64::new(1);

/// A dialog a renderer opened for itself, as a FilePicker's Browse button
/// opens one. It is listed and answered as any other, and dropping this
/// closes it while it is open, as when its window closes.
pub struct Owned(RequestId);

impl Owned {
    /// The dialog's number, which Get-GuiDialog lists.
    pub fn id(&self) -> RequestId {
        self.0
    }
}

impl Drop for Owned {
    fn drop(&mut self) {
        if OPEN.load().contains_key(&self.0)
            && let Err(why) = close(self.0)
        {
            eprintln!("PWRS-GUI: dialog {} was let go of and could not be closed: {why}", self.0);
        }
    }
}

/// Shows a dialog the renderer opens for itself, as [`show`] shows one for
/// a request, under a number of its own.
pub fn open(title: String, kind: DialogKind, focus: bool, stand_in: Option<StandIn>, answered: Answered) -> Owned {
    let id = OWN | NEXT_OWN.fetch_add(1, Ordering::Relaxed);
    show(id, title, kind, focus, stand_in, answered);
    Owned(id)
}

/// Closes the dialog `dialog`, whose command stopped; it answers
/// [`DialogAnswer::Canceled`].
pub fn close(dialog: RequestId) -> Result<(), String> {
    answer(dialog, DialogAnswer::Canceled)
}

/// Closes the dialog `dialog` as a person answering it with `given` would:
/// a button the message shows, the paths a file or folder dialog takes, or
/// [`DialogAnswer::Canceled`].
pub fn answer(dialog: RequestId, given: DialogAnswer) -> Result<(), String> {
    let open = match OPEN.load().get(&dialog) {
        Some(open) => open.clone(),
        None => return Err(format!("no dialog {dialog} is open")),
    };
    fits(&open, &given).map_err(|why| format!("dialog {dialog}, {} {:?}: {why}", open.kind, open.title))?;
    open.answer.store(Some(Arc::new(given)));
    if let Some(closer) = open.closer.load_full() {
        closer.close();
    }
    Ok(())
}

/// Whether `given` is an answer the dialog could have given.
fn fits(open: &Open, given: &DialogAnswer) -> Result<(), String> {
    match (given, open.buttons) {
        (DialogAnswer::Canceled, _) => Ok(()),
        (DialogAnswer::Button(b), Some(buttons)) if buttons.buttons().contains(b) => Ok(()),
        (DialogAnswer::Button(b), Some(buttons)) => Err(format!("it shows {:?}, not {b:?}", buttons.buttons())),
        (DialogAnswer::Button(b), None) => Err(format!("it takes paths, not the button {b:?}")),
        (DialogAnswer::Paths(_), Some(_)) => Err(String::from("a message is answered with a button, not paths")),
        (DialogAnswer::Paths(paths), None) if paths.is_empty() => Err(String::from("it takes a path; cancel it to give none")),
        (DialogAnswer::Paths(paths), None) if paths.len() > 1 && !open.multiple => Err(format!("it takes one path, not {}", paths.len())),
        (DialogAnswer::Paths(_), None) => Ok(()),
    }
}

/// The dialogs showing now: the request that opened each, its kind and its
/// title. One not yet showing is left out, since where the system cannot
/// show it, it ends without showing or a renderer draws it in its place,
/// perhaps under another number.
pub fn list() -> Vec<DialogSummary> {
    OPEN.load()
        .iter()
        .filter(|(_, open)| open.closer.load().as_ref().is_some_and(|closer| closer.showing()))
        .map(|(request, open)| DialogSummary { dialog: *request, kind: open.kind.to_string(), title: open.title.clone() })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use pwrs_gui_proto::MessageButton;

    fn dialog(buttons: Option<MessageButtons>, multiple: bool) -> Open {
        Open { kind: "Test", title: String::new(), buttons, multiple, closer: ArcSwapOption::empty(), answer: ArcSwapOption::empty() }
    }

    fn paths(n: usize) -> DialogAnswer {
        DialogAnswer::Paths((0..n).map(|i| format!("/tmp/{i}.log")).collect())
    }

    #[test]
    fn a_message_takes_a_button_it_shows_and_nothing_else() {
        let message = dialog(Some(MessageButtons::YesNo), false);
        assert_eq!(fits(&message, &DialogAnswer::Button(MessageButton::No)), Ok(()));
        let refused = fits(&message, &DialogAnswer::Button(MessageButton::Cancel)).expect_err("a YesNo message shows no Cancel");
        assert!(refused.contains("Cancel"), "{refused}");
        assert!(fits(&message, &paths(1)).is_err(), "a message takes no paths");
    }

    #[test]
    fn a_file_dialog_takes_one_path_unless_it_takes_many() {
        let one = dialog(None, false);
        assert_eq!(fits(&one, &paths(1)), Ok(()));
        let refused = fits(&one, &paths(2)).expect_err("a dialog of one file takes one path");
        assert!(refused.contains("one path"), "{refused}");
        assert_eq!(fits(&dialog(None, true), &paths(3)), Ok(()));
        assert!(fits(&one, &paths(0)).is_err(), "no paths is a cancel, not an answer of paths");
        assert!(fits(&one, &DialogAnswer::Button(MessageButton::Ok)).is_err(), "a file dialog shows no buttons");
    }

    #[test]
    fn every_dialog_can_be_canceled() {
        for open in [dialog(Some(MessageButtons::Ok), false), dialog(None, false), dialog(None, true)] {
            assert_eq!(fits(&open, &DialogAnswer::Canceled), Ok(()));
        }
    }

    #[test]
    fn a_dialog_not_open_cannot_be_answered() {
        let refused = answer(OWN - 1, DialogAnswer::Canceled).expect_err("no dialog has that number");
        assert!(refused.contains("no dialog"), "{refused}");
    }

    #[test]
    fn a_dialog_is_listed_once_it_shows_and_answered_before_it_does() {
        let id = OWN - 2;
        let open = Arc::new(dialog(Some(MessageButtons::YesNo), false));
        OPEN.rcu(|m| {
            let mut m = (**m).clone();
            m.insert(id, Arc::clone(&open));
            m
        });
        let listed = || list().iter().any(|d| d.dialog == id);
        assert!(!listed(), "a dialog nothing has started is not listed");
        answer(id, DialogAnswer::Button(MessageButton::No)).expect("an open dialog takes an answer before it shows");
        let closed = Arc::new(AtomicU64::new(0));
        let count = Arc::clone(&closed);
        open.closer.store(Some(Arc::new(Closing::Drawn(Box::new(move || {
            count.fetch_add(1, Ordering::Relaxed);
        })))));
        assert!(listed(), "a drawn dialog is listed");
        answer(id, DialogAnswer::Canceled).expect("a dialog that shows takes an answer");
        assert_eq!(closed.load(Ordering::Relaxed), 1, "an answer closes a dialog that shows");
        OPEN.rcu(|m| {
            let mut m = (**m).clone();
            m.remove(&id);
            m
        });
        assert!(!listed());
    }
}
