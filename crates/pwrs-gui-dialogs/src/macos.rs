//! Dialogs on macOS. A message is a CFUserNotification alert, which the
//! system's notification agent draws, waited on by a thread of its own and
//! closed by CFUserNotificationCancel. A file or folder dialog is an
//! NSOpenPanel or NSSavePanel shown modeless on the main thread, where the
//! helper's event loop runs, so no dialog blocks the loop; it closes
//! through the panel's own cancel action.
//!
//! An alert's levels are Note, Caution and Stop; a question shows the Note
//! level, since macOS alerts have no question style. A panel takes the
//! filters' extensions together, as a panel without an accessory view
//! does, and every file when a filter takes every file.

use crate::Answer;
use arc_swap::ArcSwapOption;
use block2::RcBlock;
use dispatch2::DispatchQueue;
use objc2::MainThreadMarker;
use objc2::rc::Retained;
use objc2_app_kit::{NSApplication, NSModalResponse, NSModalResponseCancel, NSModalResponseOK, NSOpenPanel, NSSavePanel};
use objc2_core_foundation::{
    CFDictionary, CFOptionFlags, CFRetained, CFString, CFUserNotification, kCFUserNotificationAlertHeaderKey, kCFUserNotificationAlertMessageKey,
    kCFUserNotificationAlternateButtonTitleKey, kCFUserNotificationCancelResponse, kCFUserNotificationCautionAlertLevel, kCFUserNotificationDefaultButtonTitleKey,
    kCFUserNotificationNoteAlertLevel, kCFUserNotificationOtherButtonTitleKey, kCFUserNotificationStopAlertLevel,
};
use objc2_foundation::{NSArray, NSString, NSURL};
use pwrs_gui_proto::{DialogAnswer, DialogFailure, DialogKind, MessageButton, MessageButtons, MessageIcon};
use std::cell::Cell;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

/// What closes a dialog.
pub struct Closer {
    /// Set once the dialog is to close; a dialog not yet shown reads it and
    /// does not show.
    cancel: Arc<AtomicBool>,
    way: Way,
}

enum Way {
    /// A message's alert, stored while it shows.
    Alert(Arc<ArcSwapOption<Alert>>),
    /// A panel, by the number the main thread stores it under, and whether
    /// it has begun to show.
    Panel(u64, Arc<AtomicBool>),
}

impl Closer {
    /// Closes the dialog: an alert through CFUserNotificationCancel, a panel
    /// through its cancel action on the main thread.
    pub fn close(&self) {
        self.cancel.store(true, Ordering::Release);
        match &self.way {
            Way::Alert(alert) => {
                if let Some(alert) = alert.load_full() {
                    alert.0.cancel();
                }
            }
            Way::Panel(key, _) => {
                let key = *key;
                DispatchQueue::main().exec_async(move || cancel_panel(key));
            }
        }
    }

    /// Whether the dialog shows, which is when it is listed: an alert once
    /// it is made, a panel once it has begun.
    pub fn showing(&self) -> bool {
        match &self.way {
            Way::Alert(alert) => alert.load().is_some(),
            Way::Panel(_, begun) => begun.load(Ordering::Acquire),
        }
    }
}

/// A shown alert, which a [`Closer`] cancels from another thread while the
/// alert's own thread waits for its answer.
struct Alert(CFRetained<CFUserNotification>);

// SAFETY: off its own thread the alert is only canceled, and
// CFUserNotificationCancel reads the alert's reply port and token to send
// the notification server a cancel, whose reply to that port is what ends
// the wait in CFUserNotificationReceiveResponse.
unsafe impl Send for Alert {}
// SAFETY: as for Send; the cancel takes a shared reference and changes
// nothing in the alert.
unsafe impl Sync for Alert {}

/// The number the next panel is stored under.
static NEXT_PANEL: AtomicU64 = AtomicU64::new(1);

thread_local! {
    /// The panels showing, by number. Only the main thread, which shows
    /// them, reads it.
    static PANELS: Cell<Vec<(u64, Retained<NSSavePanel>)>> = const { Cell::new(Vec::new()) };
}

/// Shows the dialog, taking the focus when `focus` is true, passes `shown`
/// what closes it, and gives `done` its answer when it closes. An alert
/// floats above every window whether or not it has the focus.
pub fn show(title: String, kind: DialogKind, focus: bool, shown: Box<dyn FnOnce(Closer) + Send>, done: Answer) {
    let cancel = Arc::new(AtomicBool::new(false));
    if let DialogKind::Message { text, icon, buttons } = kind {
        let alert = Arc::new(ArcSwapOption::empty());
        let closer = Closer { cancel: Arc::clone(&cancel), way: Way::Alert(Arc::clone(&alert)) };
        let started = std::thread::Builder::new().name(String::from("pwrs-gui-dialog")).spawn(move || {
            shown(closer);
            done.give(message(&title, &text, icon, buttons, &cancel, &alert));
        });
        if let Err(e) = started {
            eprintln!("PWRS-GUI: a dialog's thread could not start: {e}");
        }
        return;
    }
    let key = NEXT_PANEL.fetch_add(1, Ordering::Relaxed);
    let begun = Arc::new(AtomicBool::new(false));
    shown(Closer { cancel: Arc::clone(&cancel), way: Way::Panel(key, Arc::clone(&begun)) });
    DispatchQueue::main().exec_async(move || show_panel(key, &title, kind, focus, &cancel, &begun, done));
}

/// An alert of `text` headed `title`, with `icon`'s level and `buttons`,
/// and the button pressed.
fn message(title: &str, text: &str, icon: MessageIcon, buttons: MessageButtons, cancel: &AtomicBool, shown: &ArcSwapOption<Alert>) -> Result<DialogAnswer, DialogFailure> {
    if cancel.load(Ordering::Acquire) {
        return Ok(DialogAnswer::Canceled);
    }
    let labels: &[(&str, MessageButton)] = match buttons {
        MessageButtons::Ok => &[("OK", MessageButton::Ok)],
        MessageButtons::OkCancel => &[("OK", MessageButton::Ok), ("Cancel", MessageButton::Cancel)],
        MessageButtons::YesNo => &[("Yes", MessageButton::Yes), ("No", MessageButton::No)],
        MessageButtons::YesNoCancel => &[("Yes", MessageButton::Yes), ("No", MessageButton::No), ("Cancel", MessageButton::Cancel)],
    };
    // SAFETY: these are CoreFoundation's constant keys, which nothing writes.
    let (header, body, button_keys) = unsafe {
        (
            kCFUserNotificationAlertHeaderKey,
            kCFUserNotificationAlertMessageKey,
            [kCFUserNotificationDefaultButtonTitleKey, kCFUserNotificationAlternateButtonTitleKey, kCFUserNotificationOtherButtonTitleKey],
        )
    };
    let mut entries = vec![(header, title), (body, text)];
    entries.extend(button_keys.into_iter().zip(labels.iter().map(|(label, _)| *label)));
    let mut keys = Vec::new();
    let mut values = Vec::new();
    for (key, value) in entries {
        keys.push(key.ok_or_else(|| DialogFailure::Failed(String::from("CoreFoundation gives no key for an alert's text")))?);
        values.push(CFString::from_str(value));
    }
    let values: Vec<&CFString> = values.iter().map(|v| &**v).collect();
    let dictionary = CFDictionary::<CFString, CFString>::from_slices(&keys, &values);
    let level = match icon {
        MessageIcon::Info | MessageIcon::Question => kCFUserNotificationNoteAlertLevel,
        MessageIcon::Warning => kCFUserNotificationCautionAlertLevel,
        MessageIcon::Error => kCFUserNotificationStopAlertLevel,
    };
    let mut error = 0i32;
    // SAFETY: `error` is a place for the error, and the dictionary maps the
    // alert's keys to strings, as CFUserNotificationCreate takes them.
    let created = unsafe { CFUserNotification::new(None, 0.0, level, &mut error, Some(dictionary.as_opaque())) };
    let Some(created) = created else {
        return Err(DialogFailure::Failed(format!("the alert could not show: CFUserNotificationCreate gave {error}")));
    };
    let alert = Arc::new(Alert(created));
    shown.store(Some(Arc::clone(&alert)));
    if cancel.load(Ordering::Acquire) {
        alert.0.cancel();
    }
    let mut response: CFOptionFlags = 0;
    // SAFETY: `response` is a place for the answer.
    let got = unsafe { alert.0.receive_response(0.0, &mut response) };
    shown.store(None);
    if got != 0 {
        return Err(DialogFailure::Failed(format!("the alert's answer could not be read: CFUserNotificationReceiveResponse gave {got}")));
    }
    let pressed = response & 0x3;
    if pressed == kCFUserNotificationCancelResponse {
        return Ok(DialogAnswer::Canceled);
    }
    match labels.get(pressed as usize) {
        Some((_, button)) => Ok(DialogAnswer::Button(*button)),
        None => Err(DialogFailure::Failed(format!("the alert returned button {pressed}, which it does not show"))),
    }
}

/// The panel stored under `key`, while it shows.
fn kept(key: u64) -> Option<Retained<NSSavePanel>> {
    PANELS.with(|panels| {
        let list = panels.take();
        let found = list.iter().find(|(k, _)| *k == key).map(|(_, panel)| panel.clone());
        panels.set(list);
        found
    })
}

fn keep(key: u64, panel: Retained<NSSavePanel>) {
    PANELS.with(|panels| {
        let mut list = panels.take();
        list.push((key, panel));
        panels.set(list);
    });
}

fn forget(key: u64) {
    PANELS.with(|panels| {
        let mut list = panels.take();
        list.retain(|(k, _)| *k != key);
        panels.set(list);
    });
}

/// Closes the panel stored under `key` as its Cancel button would; one that
/// has closed, or not yet shown, is not stored.
fn cancel_panel(key: u64) {
    if let Some(panel) = kept(key) {
        // SAFETY: cancel: is an action, and an action takes any sender,
        // none included.
        unsafe { panel.cancel(None) };
    }
}

/// Shows the panel for `kind` on the main thread, which the main queue runs
/// this on, unless it was closed first, and sets `begun` once it shows.
fn show_panel(key: u64, title: &str, kind: DialogKind, focus: bool, cancel: &AtomicBool, begun: &AtomicBool, done: Answer) {
    let Some(mtm) = MainThreadMarker::new() else {
        done.give(Err(DialogFailure::Failed(String::from("the panel was not shown on the main thread"))));
        return;
    };
    if cancel.load(Ordering::Acquire) {
        done.give(Ok(DialogAnswer::Canceled));
        return;
    }
    let (panel, open, filters, directory) = match kind {
        DialogKind::Open { multiple, filters, directory } => {
            let open = NSOpenPanel::openPanel(mtm);
            open.setCanChooseFiles(true);
            open.setCanChooseDirectories(false);
            open.setAllowsMultipleSelection(multiple);
            (Retained::into_super(open.clone()), Some(open), filters, directory)
        }
        DialogKind::Folder { multiple, directory } => {
            let open = NSOpenPanel::openPanel(mtm);
            open.setCanChooseFiles(false);
            open.setCanChooseDirectories(true);
            open.setAllowsMultipleSelection(multiple);
            open.setCanCreateDirectories(true);
            (Retained::into_super(open.clone()), Some(open), Vec::new(), directory)
        }
        DialogKind::Save { filters, directory, file_name } => {
            let save = NSSavePanel::savePanel(mtm);
            save.setCanCreateDirectories(true);
            if let Some(name) = file_name {
                save.setNameFieldStringValue(&NSString::from_str(&name));
            }
            (save, None, filters, directory)
        }
        DialogKind::Message { .. } => {
            done.give(Err(DialogFailure::Failed(String::from("a message is no panel"))));
            return;
        }
    };
    if !title.is_empty() {
        let title = NSString::from_str(title);
        panel.setTitle(Some(&title));
        panel.setMessage(Some(&title));
    }
    let every_file = filters.iter().any(|(_, extensions)| extensions.iter().any(|e| e == "*"));
    let types: Vec<Retained<NSString>> = filters.iter().flat_map(|(_, extensions)| extensions.iter()).map(|e| NSString::from_str(e)).collect();
    if !every_file && !types.is_empty() {
        // allowedContentTypes takes uniform types, and an extension a filter
        // names may have none.
        #[allow(deprecated)]
        panel.setAllowedFileTypes(Some(&NSArray::from_retained_slice(&types)));
    }
    if let Some(directory) = directory {
        panel.setDirectoryURL(Some(&NSURL::fileURLWithPath_isDirectory(&NSString::from_str(&directory), true)));
    }
    keep(key, panel.clone());
    let done = Cell::new(Some(done));
    let answered = panel.clone();
    let handler = RcBlock::new(move |response: NSModalResponse| {
        forget(key);
        if let Some(done) = done.take() {
            done.give(match response {
                r if r == NSModalResponseOK => chosen(&answered, open.as_deref()),
                r if r == NSModalResponseCancel => Ok(DialogAnswer::Canceled),
                other => Err(DialogFailure::Failed(format!("the panel could not show: it returned {other}"))),
            });
        }
    });
    if focus {
        // activate() is new in macOS 14; this older call does the same on
        // every release.
        #[allow(deprecated)]
        NSApplication::sharedApplication(mtm).activateIgnoringOtherApps(true);
    }
    panel.beginWithCompletionHandler(&handler);
    begun.store(true, Ordering::Release);
}

/// The paths chosen in a panel closed with OK: an open panel's URLs, a save
/// panel's URL.
fn chosen(panel: &NSSavePanel, open: Option<&NSOpenPanel>) -> Result<DialogAnswer, DialogFailure> {
    let urls: Vec<Retained<NSURL>> = match open {
        Some(open) => open.URLs().to_vec(),
        None => panel.URL().into_iter().collect(),
    };
    let mut paths = Vec::new();
    for url in urls {
        let Some(path) = url.path() else {
            return Err(DialogFailure::Failed(format!("the panel chose {url:?}, which names no path")));
        };
        paths.push(path.to_string());
    }
    if paths.is_empty() { Ok(DialogAnswer::Canceled) } else { Ok(DialogAnswer::Paths(paths)) }
}
