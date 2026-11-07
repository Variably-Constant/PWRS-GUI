//! Dialogs on Linux and the BSDs, each run on a thread of its own: a file or
//! folder dialog through the XDG desktop portal, by rfd, which closes it
//! through the portal's Request.Close; where the portal cannot show it, and
//! for every message, zenity when it is on PATH, closed by ending it. With
//! neither, the system has no dialog of the kind, and the egui renderer
//! draws its own. A dialog shows, and is listed, once the portal has taken
//! its request or zenity has started.
//!
//! A message box takes its icon through the flag zenity's version takes,
//! read once: --icon-name before 4, --icon from 4. zenity returns the same
//! exit code for closing a box of two or three buttons as for its second
//! button.

use crate::Answer;
use pwrs_gui_proto::{DialogAnswer, DialogFailure, DialogKind, MessageButton, MessageButtons, MessageIcon};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, OnceLock};

/// What closes a dialog: the flag the portal's wait reads, and zenity's
/// process while it runs, 0 when none does; and whether it shows.
pub struct Closer {
    cancel: Arc<AtomicBool>,
    zenity: Arc<AtomicU32>,
    showing: Arc<AtomicBool>,
}

impl Closer {
    /// Closes the dialog: the portal's request through its flag, zenity by
    /// ending its process. A dialog not yet shown reads the flag and does
    /// not show.
    pub fn close(&self) {
        self.cancel.store(true, Ordering::Release);
        end(self.zenity.load(Ordering::Acquire));
    }

    /// Whether the dialog shows: once the portal has taken its request or
    /// zenity has started.
    pub fn showing(&self) -> bool {
        self.showing.load(Ordering::Acquire)
    }
}

/// Ends zenity's process `pid`; 0 names none.
fn end(pid: u32) {
    if pid != 0 {
        // SAFETY: kill only sends a signal; for a process that has ended
        // it returns ESRCH.
        unsafe { libc::kill(pid as libc::pid_t, libc::SIGTERM) };
    }
}

/// Runs the dialog on a thread of its own, passes `shown` what closes it,
/// and gives `done` its answer when it closes. Neither the portal nor
/// zenity can be told to leave the focus alone, so `_focus` goes unread.
pub fn show(title: String, kind: DialogKind, _focus: bool, shown: Box<dyn FnOnce(Closer) + Send>, done: Answer) {
    let cancel = Arc::new(AtomicBool::new(false));
    let zenity = Arc::new(AtomicU32::new(0));
    let showing = Arc::new(AtomicBool::new(false));
    let closer = Closer { cancel: Arc::clone(&cancel), zenity: Arc::clone(&zenity), showing: Arc::clone(&showing) };
    let started = std::thread::Builder::new().name(String::from("pwrs-gui-dialog")).spawn(move || {
        shown(closer);
        done.give(run(&title, kind, &cancel, &zenity, &showing));
    });
    if let Err(e) = started {
        eprintln!("PWRS-GUI: a dialog's thread could not start: {e}");
    }
}

fn run(title: &str, kind: DialogKind, cancel: &Arc<AtomicBool>, zenity: &AtomicU32, showing: &Arc<AtomicBool>) -> Result<DialogAnswer, DialogFailure> {
    if let DialogKind::Message { text, icon, buttons } = &kind {
        return message(title, text, *icon, *buttons, cancel, zenity, showing);
    }
    let failed = Arc::new(AtomicBool::new(false));
    let chosen = portal(title, &kind, cancel, &failed, showing);
    if !failed.load(Ordering::Acquire) {
        return paths(chosen);
    }
    file_by_zenity(title, &kind, cancel, zenity, showing)
}

/// The file or folder dialog through the XDG portal: the paths chosen,
/// `None` for a cancel; `failed` is set where the portal cannot show it,
/// and `showing` once it has taken the request.
fn portal(title: &str, kind: &DialogKind, cancel: &Arc<AtomicBool>, failed: &Arc<AtomicBool>, showing: &Arc<AtomicBool>) -> Option<Vec<PathBuf>> {
    let mut d = rfd::FileDialog::new().set_cancel(Arc::clone(cancel)).set_failed(Arc::clone(failed)).set_shown(Arc::clone(showing));
    if !title.is_empty() {
        d = d.set_title(title);
    }
    let (filters, directory) = match kind {
        DialogKind::Open { filters, directory, .. } | DialogKind::Save { filters, directory, .. } => (filters.as_slice(), directory.as_deref()),
        DialogKind::Folder { directory, .. } => (&[][..], directory.as_deref()),
        DialogKind::Message { .. } => (&[][..], None),
    };
    for (name, extensions) in filters {
        d = d.add_filter(name, extensions);
    }
    if let Some(dir) = directory {
        d = d.set_directory(dir);
    }
    match kind {
        DialogKind::Open { multiple: true, .. } => d.pick_files(),
        DialogKind::Open { multiple: false, .. } => d.pick_file().map(|p| vec![p]),
        DialogKind::Save { file_name, .. } => {
            if let Some(name) = file_name {
                d = d.set_file_name(name);
            }
            d.save_file().map(|p| vec![p])
        }
        DialogKind::Folder { multiple: true, .. } => d.pick_folders(),
        DialogKind::Folder { multiple: false, .. } => d.pick_folder().map(|p| vec![p]),
        DialogKind::Message { .. } => None,
    }
}

/// The paths chosen, or a cancel when none were.
fn paths(chosen: Option<Vec<PathBuf>>) -> Result<DialogAnswer, DialogFailure> {
    let Some(chosen) = chosen else {
        return Ok(DialogAnswer::Canceled);
    };
    let mut out = Vec::new();
    for p in chosen {
        out.push(p.into_os_string().into_string().map_err(|p| DialogFailure::Failed(format!("the path {p:?} the dialog chose is not Unicode")))?);
    }
    Ok(DialogAnswer::Paths(out))
}

/// The file or folder dialog through zenity.
fn file_by_zenity(title: &str, kind: &DialogKind, cancel: &AtomicBool, zenity: &AtomicU32, showing: &AtomicBool) -> Result<DialogAnswer, DialogFailure> {
    let mut args = vec![String::from("--file-selection")];
    if !title.is_empty() {
        args.push(format!("--title={title}"));
    }
    let (filters, start) = match kind {
        DialogKind::Open { multiple, filters, directory } => {
            if *multiple {
                args.push(String::from("--multiple"));
                args.push(String::from("--separator=\n"));
            }
            (filters.as_slice(), directory.as_deref().map(|d| format!("{}/", d.trim_end_matches('/'))))
        }
        DialogKind::Save { filters, directory, file_name } => {
            args.push(String::from("--save"));
            let start = match (directory, file_name) {
                (Some(d), Some(n)) => Some(format!("{}/{n}", d.trim_end_matches('/'))),
                (Some(d), None) => Some(format!("{}/", d.trim_end_matches('/'))),
                (None, Some(n)) => Some(n.clone()),
                (None, None) => None,
            };
            (filters.as_slice(), start)
        }
        DialogKind::Folder { multiple, directory } => {
            args.push(String::from("--directory"));
            if *multiple {
                args.push(String::from("--multiple"));
                args.push(String::from("--separator=\n"));
            }
            (&[][..], directory.as_deref().map(|d| format!("{}/", d.trim_end_matches('/'))))
        }
        DialogKind::Message { .. } => return Err(DialogFailure::Failed(String::from("a message is no file dialog"))),
    };
    for (name, extensions) in filters {
        let patterns: Vec<String> = extensions.iter().map(|e| if e == "*" { String::from("*") } else { format!("*.{e}") }).collect();
        args.push(format!("--file-filter={name} | {}", patterns.join(" ")));
    }
    if let Some(s) = start {
        args.push(format!("--filename={s}"));
    }
    let (code, written, said) = run_zenity(&args, cancel, zenity, showing)?;
    match code {
        0 => Ok(DialogAnswer::Paths(written.lines().filter(|l| !l.is_empty()).map(String::from).collect())),
        1 => Ok(DialogAnswer::Canceled),
        other => Err(DialogFailure::Failed(format!("zenity ended with {other}: {said}"))),
    }
}

/// A message box through zenity, and the button pressed.
fn message(title: &str, text: &str, icon: MessageIcon, buttons: MessageButtons, cancel: &AtomicBool, zenity: &AtomicU32, showing: &AtomicBool) -> Result<DialogAnswer, DialogFailure> {
    let mut args = match (buttons, icon) {
        (MessageButtons::Ok, MessageIcon::Warning) => vec![String::from("--warning")],
        (MessageButtons::Ok, MessageIcon::Error) => vec![String::from("--error")],
        (MessageButtons::Ok, MessageIcon::Info | MessageIcon::Question) => vec![String::from("--info")],
        (MessageButtons::OkCancel, _) => vec![String::from("--question"), String::from("--ok-label=OK"), String::from("--cancel-label=Cancel")],
        (MessageButtons::YesNo, _) => vec![String::from("--question"), String::from("--ok-label=Yes"), String::from("--cancel-label=No")],
        (MessageButtons::YesNoCancel, _) => {
            vec![String::from("--question"), String::from("--ok-label=Yes"), String::from("--cancel-label=No"), String::from("--extra-button=Cancel")]
        }
    };
    let shown_by_kind = matches!((buttons, icon), (MessageButtons::Ok, MessageIcon::Info | MessageIcon::Warning | MessageIcon::Error));
    if !shown_by_kind && !cancel.load(Ordering::Acquire) {
        args.push(format!("{}={}", icon_flag()?, icon_name(icon)));
    }
    args.push(format!("--title={title}"));
    args.push(format!("--text={text}"));
    args.push(String::from("--no-markup"));
    let (code, written, said) = run_zenity(&args, cancel, zenity, showing)?;
    let pressed = match (code, buttons) {
        (0, MessageButtons::Ok | MessageButtons::OkCancel) => MessageButton::Ok,
        (0, MessageButtons::YesNo | MessageButtons::YesNoCancel) => MessageButton::Yes,
        (1, MessageButtons::YesNoCancel) if written.trim() == "Cancel" => MessageButton::Cancel,
        (1, MessageButtons::Ok) => return Ok(DialogAnswer::Canceled),
        (1, MessageButtons::OkCancel) => MessageButton::Cancel,
        (1, MessageButtons::YesNo | MessageButtons::YesNoCancel) => MessageButton::No,
        (other, _) => return Err(DialogFailure::Failed(format!("zenity ended with {other}: {said}"))),
    };
    Ok(DialogAnswer::Button(pressed))
}

/// The freedesktop icon a message box shows for `icon`.
fn icon_name(icon: MessageIcon) -> &'static str {
    match icon {
        MessageIcon::Info => "dialog-information",
        MessageIcon::Warning => "dialog-warning",
        MessageIcon::Error => "dialog-error",
        MessageIcon::Question => "dialog-question",
    }
}

/// The flag zenity takes for a box's icon, from its version, read once:
/// --icon-name before 4, --icon from 4.
fn icon_flag() -> Result<&'static str, DialogFailure> {
    static FLAG: OnceLock<Result<&'static str, DialogFailure>> = OnceLock::new();
    FLAG.get_or_init(read_icon_flag).clone()
}

fn read_icon_flag() -> Result<&'static str, DialogFailure> {
    let out = match Command::new("zenity").arg("--version").stdin(Stdio::null()).output() {
        Ok(out) => out,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Err(DialogFailure::NoNative(String::from("neither the XDG desktop portal nor zenity is available"))),
        Err(e) => return Err(DialogFailure::Failed(format!("zenity --version could not run: {e}"))),
    };
    let text = String::from_utf8_lossy(&out.stdout).trim().to_string();
    match text.split('.').next().and_then(|m| m.parse::<u32>().ok()) {
        Some(major) if major >= 4 => Ok("--icon"),
        Some(_) => Ok("--icon-name"),
        None => Err(DialogFailure::Failed(format!("zenity gave its version as {text:?}, which names no major version"))),
    }
}

/// Runs zenity with `args` unless the dialog was closed first, storing its
/// process where the Closer ends it and setting `showing` once it has
/// started: its exit code, what it wrote, and what it said on its error
/// stream. zenity missing from PATH is the system having no dialog of the
/// kind.
fn run_zenity(args: &[String], cancel: &AtomicBool, zenity: &AtomicU32, showing: &AtomicBool) -> Result<(i32, String, String), DialogFailure> {
    if cancel.load(Ordering::Acquire) {
        return Ok((1, String::new(), String::new()));
    }
    let child = match Command::new("zenity").args(args).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn() {
        Ok(child) => child,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Err(DialogFailure::NoNative(String::from("neither the XDG desktop portal nor zenity is available"))),
        Err(e) => return Err(DialogFailure::Failed(format!("zenity could not start: {e}"))),
    };
    zenity.store(child.id(), Ordering::Release);
    showing.store(true, Ordering::Release);
    if cancel.load(Ordering::Acquire) {
        end(child.id());
    }
    let out = child.wait_with_output();
    zenity.store(0, Ordering::Release);
    let out = out.map_err(|e| DialogFailure::Failed(format!("zenity's end could not be read: {e}")))?;
    let said = String::from_utf8_lossy(&out.stderr).trim().to_string();
    let Some(code) = out.status.code() else {
        return Err(DialogFailure::Failed(format!("zenity was ended by a signal: {said}")));
    };
    let written = String::from_utf8(out.stdout).map_err(|e| DialogFailure::Failed(format!("zenity wrote a path that is not Unicode: {e}")))?;
    Ok((code, written, said))
}
