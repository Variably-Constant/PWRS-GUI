//! Dialogs on Windows, each run on a thread of its own: a message box
//! MessageBoxW draws, with each of its icons, or rfd's file and folder
//! dialogs, whose blocking calls run on the calling thread. A dialog closes
//! by posting its buttons' commands to its thread's dialog windows once they
//! show: until then a file dialog passes over Cancel and takes the OK posted
//! after it, which chooses the folder it opened in or navigates. A close
//! sets a flag as well, which the thread reads before it shows the dialog
//! and, through a hook on the messages its windows are sent, as a dialog
//! window shows, so a close that comes before then still ends it.

use crate::Answer;
use pwrs_gui_proto::{DialogAnswer, DialogFailure, DialogKind, MessageButton, MessageButtons, MessageIcon};
use std::cell::RefCell;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows_sys::Win32::System::Threading::GetCurrentThreadId;
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CWPSTRUCT, CallNextHookEx, EnumThreadWindows, GetClassNameW, IDCANCEL, IDNO, IDOK, IDYES, IsWindowVisible, MB_ICONERROR, MB_ICONINFORMATION, MB_ICONQUESTION, MB_ICONWARNING, MB_OK,
    MB_OKCANCEL, MB_SETFOREGROUND, MB_YESNO, MB_YESNOCANCEL, MessageBoxW, PostMessageW, SWP_SHOWWINDOW, SetWindowsHookExW, UnhookWindowsHookEx, WH_CALLWNDPROC, WINDOWPOS, WM_COMMAND,
    WM_WINDOWPOSCHANGED,
};
use windows_sys::core::BOOL;

/// The class of a dialog window, the message box's and the file dialogs'.
const DIALOG_CLASS: &str = "#32770";

/// What closes a dialog: the thread it runs on, and the flag a close sets.
pub struct Closer {
    thread: u32,
    closing: Arc<AtomicBool>,
}

impl Closer {
    /// Ends each shown dialog window of the dialog's thread as its Cancel
    /// button would; a message box with no Cancel button takes No, then OK.
    /// The flag is set first, so a window not shown yet when the enumeration
    /// passes it is shown after the flag was set, and its thread ends it
    /// then.
    pub fn close(&self) {
        self.closing.store(true, Ordering::SeqCst);
        // SAFETY: post_close only reads its window's class and state and
        // posts to it.
        unsafe { EnumThreadWindows(self.thread, Some(post_close), 0) };
    }

    /// Whether the dialog shows, which is when it is listed: once its
    /// thread has a dialog window shown, for a close to end.
    pub fn showing(&self) -> bool {
        let mut found = false;
        // SAFETY: find_dialog only reads its window's class and state and
        // writes the bool the argument points at, which outlives the
        // enumeration.
        unsafe { EnumThreadWindows(self.thread, Some(find_dialog), &raw mut found as LPARAM) };
        found
    }
}

/// Whether `window` is a dialog window, by its class.
fn is_dialog(window: HWND) -> bool {
    let mut class = [0u16; 16];
    // SAFETY: the buffer has room for `class.len()` characters.
    let n = unsafe { GetClassNameW(window, class.as_mut_ptr(), class.len() as i32) };
    n > 0 && String::from_utf16_lossy(&class[..n as usize]) == DIALOG_CLASS
}

/// Whether `window` is a dialog window that is shown.
fn shown_dialog(window: HWND) -> bool {
    // SAFETY: IsWindowVisible only reads the window's state.
    is_dialog(window) && unsafe { IsWindowVisible(window) } != 0
}

/// Posts Cancel, No and OK to `window`; the first of them its buttons take
/// ends it.
fn post_buttons(window: HWND) {
    for id in [IDCANCEL, IDNO, IDOK] {
        // SAFETY: posting to a window of the dialog's thread.
        unsafe { PostMessageW(window, WM_COMMAND, id as WPARAM, 0) };
    }
}

/// Posts the close to `window` when it is a shown dialog window.
unsafe extern "system" fn post_close(window: HWND, _: LPARAM) -> BOOL {
    if shown_dialog(window) {
        post_buttons(window);
    }
    1
}

/// Sets the bool `found` points at, and ends the enumeration, at the first
/// shown dialog window.
unsafe extern "system" fn find_dialog(window: HWND, found: LPARAM) -> BOOL {
    if !shown_dialog(window) {
        return 1;
    }
    // SAFETY: `found` points at the bool Closer::showing passed.
    unsafe { *(found as *mut bool) = true };
    0
}

thread_local! {
    /// The close flag of the dialog this thread shows, which the hook reads.
    static CLOSING: RefCell<Option<Arc<AtomicBool>>> = const { RefCell::new(None) };
}

/// Ends a dialog window of the thread as it is shown, when its dialog's
/// close flag is already set. The window counts as shown when it is told its
/// position changed with it shown, since it is then visible to a close
/// enumerating from another thread too.
unsafe extern "system" fn on_sent(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code >= 0 {
        // SAFETY: a WH_CALLWNDPROC hook's lparam points at the message sent.
        let sent = unsafe { &*(lparam as *const CWPSTRUCT) };
        // SAFETY: WM_WINDOWPOSCHANGED's lparam points at the window's new
        // position.
        if sent.message == WM_WINDOWPOSCHANGED
            && unsafe { &*(sent.lParam as *const WINDOWPOS) }.flags & SWP_SHOWWINDOW != 0
            && is_dialog(sent.hwnd)
            && CLOSING.with(|c| c.borrow().as_ref().is_some_and(|c| c.load(Ordering::SeqCst)))
        {
            post_buttons(sent.hwnd);
        }
    }
    // SAFETY: passes the hook's own arguments on.
    unsafe { CallNextHookEx(std::ptr::null_mut(), code, wparam, lparam) }
}

/// Runs the dialog on a thread of its own, passes `shown` what closes it,
/// and gives `done` its answer when it closes. A message box requests the
/// foreground when `focus` is true; the file dialogs take what Windows
/// gives them.
pub fn show(title: String, kind: DialogKind, focus: bool, shown: Box<dyn FnOnce(Closer) + Send>, done: Answer) {
    let started = std::thread::Builder::new().name(String::from("pwrs-gui-dialog")).spawn(move || {
        // SAFETY: GetCurrentThreadId has no preconditions.
        let thread = unsafe { GetCurrentThreadId() };
        let closing = Arc::new(AtomicBool::new(false));
        CLOSING.with(|c| *c.borrow_mut() = Some(Arc::clone(&closing)));
        // SAFETY: a hook on this thread's own messages, with no module, as
        // a hook on one thread of the process takes; removed below.
        let hook = unsafe { SetWindowsHookExW(WH_CALLWNDPROC, Some(on_sent), std::ptr::null_mut(), thread) };
        if hook.is_null() {
            eprintln!("PWRS-GUI: a dialog's thread could not hook its windows' messages, so a close that comes before the dialog shows does not end it: {}", std::io::Error::last_os_error());
        }
        shown(Closer { thread, closing: Arc::clone(&closing) });
        let outcome = if closing.load(Ordering::SeqCst) { Ok(DialogAnswer::Canceled) } else { run(&title, kind, focus).map_err(DialogFailure::Failed) };
        // SAFETY: the hook this thread set.
        if !hook.is_null() && unsafe { UnhookWindowsHookEx(hook) } == 0 {
            eprintln!("PWRS-GUI: a dialog's thread could not remove the hook on its windows' messages: {}", std::io::Error::last_os_error());
        }
        done.give(outcome);
    });
    if let Err(e) = started {
        eprintln!("PWRS-GUI: a dialog's thread could not start: {e}");
    }
}

fn run(title: &str, kind: DialogKind, focus: bool) -> Result<DialogAnswer, String> {
    match kind {
        DialogKind::Message { text, icon, buttons } => message(title, &text, icon, buttons, focus),
        DialogKind::Open { multiple, filters, directory } => {
            let d = file_dialog(title, &filters, directory.as_deref());
            if multiple { paths(d.pick_files()) } else { paths(d.pick_file().map(|p| vec![p])) }
        }
        DialogKind::Save { filters, directory, file_name } => {
            let mut d = file_dialog(title, &filters, directory.as_deref());
            if let Some(name) = file_name {
                d = d.set_file_name(name);
            }
            paths(d.save_file().map(|p| vec![p]))
        }
        DialogKind::Folder { multiple, directory } => {
            let d = file_dialog(title, &[], directory.as_deref());
            if multiple { paths(d.pick_folders()) } else { paths(d.pick_folder().map(|p| vec![p])) }
        }
    }
}

/// rfd's dialog titled `title`, or with the system's own title when it is
/// empty, with `filters` and starting in `directory`.
fn file_dialog(title: &str, filters: &[(String, Vec<String>)], directory: Option<&str>) -> rfd::FileDialog {
    let mut d = rfd::FileDialog::new();
    if !title.is_empty() {
        d = d.set_title(title);
    }
    for (name, extensions) in filters {
        d = d.add_filter(name, extensions);
    }
    if let Some(dir) = directory {
        d = d.set_directory(dir);
    }
    d
}

/// The paths chosen, or a cancel when none were.
fn paths(chosen: Option<Vec<PathBuf>>) -> Result<DialogAnswer, String> {
    let Some(chosen) = chosen else {
        return Ok(DialogAnswer::Canceled);
    };
    let mut out = Vec::new();
    for p in chosen {
        out.push(p.into_os_string().into_string().map_err(|p| format!("the path {p:?} the dialog chose is not Unicode"))?);
    }
    Ok(DialogAnswer::Paths(out))
}

/// A message box of `text` titled `title`, with `icon` and `buttons`,
/// requesting the foreground when `focus` is true, and the button pressed.
fn message(title: &str, text: &str, icon: MessageIcon, buttons: MessageButtons, focus: bool) -> Result<DialogAnswer, String> {
    let shown = match buttons {
        MessageButtons::Ok => MB_OK,
        MessageButtons::OkCancel => MB_OKCANCEL,
        MessageButtons::YesNo => MB_YESNO,
        MessageButtons::YesNoCancel => MB_YESNOCANCEL,
    };
    let drawn = match icon {
        MessageIcon::Info => MB_ICONINFORMATION,
        MessageIcon::Warning => MB_ICONWARNING,
        MessageIcon::Error => MB_ICONERROR,
        MessageIcon::Question => MB_ICONQUESTION,
    };
    let foreground = if focus { MB_SETFOREGROUND } else { 0 };
    let wide = |s: &str| s.encode_utf16().chain(std::iter::once(0)).collect::<Vec<u16>>();
    let (text, title) = (wide(text), wide(title));
    // SAFETY: both strings end in a nul and outlive the call.
    let pressed = unsafe { MessageBoxW(std::ptr::null_mut(), text.as_ptr(), title.as_ptr(), shown | drawn | foreground) };
    match pressed {
        IDOK => Ok(DialogAnswer::Button(MessageButton::Ok)),
        IDCANCEL => Ok(DialogAnswer::Button(MessageButton::Cancel)),
        IDYES => Ok(DialogAnswer::Button(MessageButton::Yes)),
        IDNO => Ok(DialogAnswer::Button(MessageButton::No)),
        0 => Err(format!("the message box could not show: {}", std::io::Error::last_os_error())),
        other => Err(format!("the message box returned {other}, which none of its buttons gives")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;
    use windows_sys::Win32::UI::WindowsAndMessaging::{CWPRETSTRUCT, WH_CALLWNDPROCRET, WM_CREATE, WM_INITDIALOG};

    thread_local! {
        /// What closes the dialog this thread shows, until the hook below
        /// uses it.
        static STARTING: RefCell<Option<Closer>> = const { RefCell::new(None) };
        /// Whether a window of this thread is inside its WM_INITDIALOG.
        static IN_INIT: RefCell<bool> = const { RefCell::new(false) };
    }

    /// Closes the thread's dialog once, as a window is created inside a
    /// dialog window's WM_INITDIALOG, before the dialog shows.
    unsafe extern "system" fn on_sent_starting(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
        if code >= 0 {
            // SAFETY: a WH_CALLWNDPROC hook's lparam points at the message sent.
            let sent = unsafe { &*(lparam as *const CWPSTRUCT) };
            if sent.message == WM_INITDIALOG {
                IN_INIT.with(|i| *i.borrow_mut() = true);
            } else if sent.message == WM_CREATE
                && IN_INIT.with(|i| *i.borrow())
                && let Some(closer) = STARTING.with(|s| s.borrow_mut().take())
            {
                closer.close();
            }
        }
        // SAFETY: passes the hook's own arguments on.
        unsafe { CallNextHookEx(std::ptr::null_mut(), code, wparam, lparam) }
    }

    /// Notes that a window's WM_INITDIALOG has returned.
    unsafe extern "system" fn on_returned(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
        if code >= 0 {
            // SAFETY: a WH_CALLWNDPROCRET hook's lparam points at the message
            // returned and its result.
            let returned = unsafe { &*(lparam as *const CWPRETSTRUCT) };
            if returned.message == WM_INITDIALOG {
                IN_INIT.with(|i| *i.borrow_mut() = false);
            }
        }
        // SAFETY: passes the hook's own arguments on.
        unsafe { CallNextHookEx(std::ptr::null_mut(), code, wparam, lparam) }
    }

    /// A close that comes while a folder dialog starts, before it shows, as
    /// an answer to a dialog just listed can, cancels it once it shows. Before
    /// then the dialog passes over Cancel and takes the OK posted after it,
    /// choosing the folder it opened in.
    #[test]
    fn a_close_while_a_folder_dialog_starts_cancels_it() {
        let (sent, answered) = mpsc::channel();
        let shown: Box<dyn FnOnce(Closer) + Send> = Box::new(|closer| {
            STARTING.with(|s| *s.borrow_mut() = Some(closer));
            // SAFETY: hooks on this thread's own messages, with no module, as
            // a hook on one thread takes; they end with the thread.
            let hooks = unsafe {
                [
                    SetWindowsHookExW(WH_CALLWNDPROC, Some(on_sent_starting), std::ptr::null_mut(), GetCurrentThreadId()),
                    SetWindowsHookExW(WH_CALLWNDPROCRET, Some(on_returned), std::ptr::null_mut(), GetCurrentThreadId()),
                ]
            };
            assert!(hooks.iter().all(|h| !h.is_null()), "the dialog's thread is hooked: {}", std::io::Error::last_os_error());
        });
        let done = Answer(Some(Box::new(move |outcome| sent.send(outcome).expect("the test waits for the dialog's answer"))));
        let directory = std::env::temp_dir().into_os_string().into_string().expect("the temporary folder's path is Unicode");
        show(String::from("PWRS-GUI test starting folder"), DialogKind::Folder { multiple: false, directory: Some(directory) }, false, shown, done);
        let outcome = answered.recv_timeout(std::time::Duration::from_secs(20)).expect("the folder dialog ended within 20 s");
        assert_eq!(outcome, Ok(DialogAnswer::Canceled));
    }
}
