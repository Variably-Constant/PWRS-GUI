//! The system's own dialogs: Show-GuiMessage, Read-GuiFile, Read-GuiSavePath
//! and Read-GuiFolder, which wait for the person's answer; Get-GuiDialog,
//! which lists the ones showing; and the answer Invoke-GuiTestEvent -Dialog
//! gives one.

use crate::inspect::WildcardOptions;
use crate::session::{self, arm, gui_error};
use crate::widgets;
use pwrs::prelude::*;
use pwrs_gui_client::proto::{DialogAnswer, DialogFailure, DialogKind, MessageButton, MessageButtons, MessageIcon, Value};
use pwrs_gui_client::{Canceler, GuiError, Renderer};

/// The icon a message box shows.
#[psenum(name = "PwrsGui.MessageIcon")]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum GuiMessageIcon {
    /// Information.
    #[default]
    Info,
    /// A warning.
    Warning,
    /// An error.
    Error,
    /// A question: Windows' question icon, zenity's question dialog, and
    /// macOS's informational alert, as macOS has no question style.
    Question,
}

/// The buttons a message box is answered with.
#[psenum(name = "PwrsGui.MessageButtons")]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum GuiMessageButtons {
    /// OK.
    #[default]
    Ok,
    /// OK and Cancel.
    OkCancel,
    /// Yes and No.
    YesNo,
    /// Yes, No and Cancel.
    YesNoCancel,
}

/// The button a message box was answered with.
#[psenum(name = "PwrsGui.MessageResult")]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum GuiMessageResult {
    #[default]
    Ok,
    Cancel,
    Yes,
    No,
}

/// An open dialog, as Get-GuiDialog writes it.
#[psclass(name = "PwrsGui.Dialog")]
#[derive(Clone, Default)]
pub struct GuiDialogInfo {
    /// The dialog's number, which Invoke-GuiTestEvent -Dialog takes.
    pub id: u64,
    /// Message, Open, Save or Folder.
    pub kind: String,
    /// The dialog's title; empty where the system's own title shows.
    pub title: String,
}

/// Shows a message in the system's own message box and writes the button
/// pressed. A box closed without a button writes Cancel where it shows one,
/// No for Yes and No, and Ok for OK alone; Ctrl+C in the console closes the
/// box and writes nothing.
///
/// -Icon Question shows Windows' question icon, zenity's question dialog on
/// Linux and the BSDs, and the informational alert on macOS, whose alerts
/// have no question style.
///
/// # Examples
/// Show-GuiMessage 'The copy finished.'
/// if ((Show-GuiMessage 'Delete the old logs?' -Icon Question -Buttons YesNo) -eq 'Yes') { Remove-Item *.log }
#[cmdlet(verb = "Show", noun = "GuiMessage", output = ["PwrsGui.MessageResult"])]
#[derive(Default)]
pub struct ShowGuiMessage {
    /// The message.
    #[param(mandatory, position = 0)]
    pub message: String,
    /// The box's title; PWRS-GUI when none is given.
    #[param(position = 1)]
    pub title: Option<String>,
    /// The icon beside the message.
    #[param]
    pub icon: GuiMessageIcon,
    /// The buttons the box is answered with.
    #[param]
    pub buttons: GuiMessageButtons,
}

impl Cmdlet for ShowGuiMessage {
    fn process(&mut self, ps: &Pipeline<'_>) -> PsResult<()> {
        let buttons = match self.buttons {
            GuiMessageButtons::Ok => MessageButtons::Ok,
            GuiMessageButtons::OkCancel => MessageButtons::OkCancel,
            GuiMessageButtons::YesNo => MessageButtons::YesNo,
            GuiMessageButtons::YesNoCancel => MessageButtons::YesNoCancel,
        };
        let icon = match self.icon {
            GuiMessageIcon::Info => MessageIcon::Info,
            GuiMessageIcon::Warning => MessageIcon::Warning,
            GuiMessageIcon::Error => MessageIcon::Error,
            GuiMessageIcon::Question => MessageIcon::Question,
        };
        let title = match &self.title {
            Some(t) => t.clone(),
            None => String::from("PWRS-GUI"),
        };
        let Some(answer) = ask(ps, &title, DialogKind::Message { text: self.message.clone(), icon, buttons })? else {
            return Ok(());
        };
        let pressed = match answer {
            DialogAnswer::Button(b) => b,
            DialogAnswer::Canceled if buttons.buttons().contains(&MessageButton::Cancel) => MessageButton::Cancel,
            DialogAnswer::Canceled if buttons == MessageButtons::YesNo => MessageButton::No,
            DialogAnswer::Canceled => MessageButton::Ok,
            DialogAnswer::Paths(p) => return Err(unexpected(format!("the message box returned paths, {p:?}"))),
        };
        ps.write(match pressed {
            MessageButton::Ok => GuiMessageResult::Ok,
            MessageButton::Cancel => GuiMessageResult::Cancel,
            MessageButton::Yes => GuiMessageResult::Yes,
            MessageButton::No => GuiMessageResult::No,
        })
    }
}

/// Shows the system's own dialog for files to open and writes the path of
/// the file chosen, or with -Multiple of each file chosen; nothing when it
/// is canceled. Ctrl+C in the console closes the dialog and writes nothing.
///
/// # Examples
/// Read-GuiFile -Filter 'Logs|*.log', 'All files|*'
/// Read-GuiFile -Multiple -InitialDirectory ~/Documents | Get-Item
#[cmdlet(verb = "Read", noun = "GuiFile", output = ["System.String"])]
#[derive(Default)]
pub struct ReadGuiFile {
    /// The kinds of files offered, the first first: 'Name|*.ext;*.ext'
    /// strings, or an ordered dictionary from each name to its extensions,
    /// as a FilePicker's -Filter takes them.
    #[param(position = 0)]
    pub filter: Option<PsObject>,
    /// Choose any number of files.
    #[param]
    pub multiple: bool,
    /// The folder the dialog starts in, a relative one from the current
    /// location.
    #[param]
    pub initial_directory: Option<String>,
    /// The dialog's title; the system's own when none is given.
    #[param]
    pub title: Option<String>,
}

impl Cmdlet for ReadGuiFile {
    fn process(&mut self, ps: &Pipeline<'_>) -> PsResult<()> {
        let kind = DialogKind::Open { multiple: self.multiple, filters: filters_of(ps, self.filter.as_ref())?, directory: directory_of(ps, self.initial_directory.as_deref())? };
        write_paths(ps, ask(ps, title_of(&self.title), kind)?)
    }
}

/// Shows the system's own dialog for a file to save and writes its path;
/// nothing when it is canceled. Ctrl+C in the console closes the dialog and
/// writes nothing.
///
/// # Examples
/// $path = Read-GuiSavePath -Filter 'Spreadsheets|*.csv' -FileName 'report.csv'
/// Get-Process | Export-Csv -Path (Read-GuiSavePath -Filter 'Spreadsheets|*.csv')
#[cmdlet(verb = "Read", noun = "GuiSavePath", output = ["System.String"])]
#[derive(Default)]
pub struct ReadGuiSavePath {
    /// The kinds of files offered, the first first, as Read-GuiFile takes
    /// them.
    #[param(position = 0)]
    pub filter: Option<PsObject>,
    /// The file name the dialog opens with.
    #[param]
    pub file_name: Option<String>,
    /// The folder the dialog starts in, a relative one from the current
    /// location.
    #[param]
    pub initial_directory: Option<String>,
    /// The dialog's title; the system's own when none is given.
    #[param]
    pub title: Option<String>,
}

impl Cmdlet for ReadGuiSavePath {
    fn process(&mut self, ps: &Pipeline<'_>) -> PsResult<()> {
        let kind = DialogKind::Save { filters: filters_of(ps, self.filter.as_ref())?, directory: directory_of(ps, self.initial_directory.as_deref())?, file_name: self.file_name.clone() };
        write_paths(ps, ask(ps, title_of(&self.title), kind)?)
    }
}

/// Shows the system's own dialog for a folder and writes its path; nothing
/// when it is canceled. Ctrl+C in the console closes the dialog and writes
/// nothing.
///
/// # Examples
/// Set-Location (Read-GuiFolder)
/// Read-GuiFolder -InitialDirectory ~ -Title 'Where should the logs go?'
#[cmdlet(verb = "Read", noun = "GuiFolder", output = ["System.String"])]
#[derive(Default)]
pub struct ReadGuiFolder {
    /// The folder the dialog starts in, a relative one from the current
    /// location.
    #[param(position = 0)]
    pub initial_directory: Option<String>,
    /// The dialog's title; the system's own when none is given.
    #[param]
    pub title: Option<String>,
}

impl Cmdlet for ReadGuiFolder {
    fn process(&mut self, ps: &Pipeline<'_>) -> PsResult<()> {
        let kind = DialogKind::Folder { multiple: false, directory: directory_of(ps, self.initial_directory.as_deref())? };
        write_paths(ps, ask(ps, title_of(&self.title), kind)?)
    }
}

/// Gets the system's dialogs showing in the sessions, in any runspace of
/// this process and in either renderer, which is how a test finds the
/// dialog another runspace's command is waiting on. A dialog is listed once
/// it shows, so one the system cannot show is never listed.
///
/// # Examples
/// Get-GuiDialog
/// Get-GuiDialog -Title 'Delete*' | ForEach-Object { Invoke-GuiTestEvent -Dialog $_.Id -Value Yes }
#[cmdlet(verb = "Get", noun = "GuiDialog", output = ["PwrsGui.Dialog"])]
#[derive(Default)]
pub struct GetGuiDialog {
    /// Only dialogs whose title matches this wildcard pattern, without
    /// regard to case.
    #[param(position = 0)]
    pub title: Option<String>,
}

impl Cmdlet for GetGuiDialog {
    fn process(&mut self, ps: &Pipeline<'_>) -> PsResult<()> {
        let pattern = match &self.title {
            Some(t) => Some(PsType::from_name("System.Management.Automation.WildcardPattern").new(&[t.clone().into_ps()?, WildcardOptions::IgnoreCase.into_ps()?])?),
            None => None,
        };
        for session in session::running() {
            for d in session.dialogs(arm(ps)).map_err(gui_error)? {
                if let Some(p) = &pattern
                    && !bool::from_ps(&p.call("IsMatch", &[d.title.clone().into_ps()?])?)?
                {
                    continue;
                }
                ps.write(GuiDialogInfo { id: d.dialog, kind: d.kind, title: d.title })?;
            }
        }
        Ok(())
    }
}

/// Closes the dialog `dialog`, one Get-GuiDialog lists, with the answer
/// `value` gives, as a person would: a message's button by name, a file or
/// folder dialog's path or paths, or nothing to cancel it.
pub(crate) fn answer(ps: &Pipeline<'_>, dialog: u64, value: Option<&PsObject>) -> PsResult<()> {
    for session in session::running() {
        let open = session.dialogs(arm(ps)).map_err(gui_error)?;
        if let Some(found) = open.iter().find(|d| d.dialog == dialog) {
            let given = answer_of(ps, &found.kind, value)?;
            return session.answer_dialog(dialog, given, arm(ps)).map_err(gui_error);
        }
    }
    let text = format!("no dialog {dialog} is showing; Get-GuiDialog lists the ones showing");
    Err(PsError::new(ErrorCategory::ObjectNotFound, "GuiNoDialog", text).terminating())
}

/// The answer `value` gives a dialog of `kind`.
fn answer_of(ps: &Pipeline<'_>, kind: &str, value: Option<&PsObject>) -> PsResult<DialogAnswer> {
    let refuse = |text: String| PsError::new(ErrorCategory::InvalidArgument, "GuiBadAnswer", text).terminating();
    let given = match value {
        Some(v) if !v.is_null() => v,
        Some(_) | None => return Ok(DialogAnswer::Canceled),
    };
    if kind != "Message" {
        return Ok(DialogAnswer::Paths(widgets::strings(ps, given)?));
    }
    let named = String::from_ps(given)?;
    match named.to_ascii_lowercase().as_str() {
        "ok" => Ok(DialogAnswer::Button(MessageButton::Ok)),
        "cancel" => Ok(DialogAnswer::Button(MessageButton::Cancel)),
        "yes" => Ok(DialogAnswer::Button(MessageButton::Yes)),
        "no" => Ok(DialogAnswer::Button(MessageButton::No)),
        _ => Err(refuse(format!("a message is answered with Ok, Cancel, Yes or No, not {named}"))),
    }
}

/// Shows the dialog in the session that draws widget windows and returns
/// how it was answered; `None` when Ctrl+C ended the wait, which closes
/// the dialog. Where the system has no dialog of the kind, the egui
/// renderer draws its own, so an HTML session's dialog goes to the egui
/// session, which starts when it is not running.
fn ask(ps: &Pipeline<'_>, title: &str, kind: DialogKind) -> PsResult<Option<DialogAnswer>> {
    let session = session::widget_session(ps)?;
    let asked = match session.dialog(title, kind.clone(), arm(ps)) {
        Err(GuiError::NoNativeDialog(_)) if session.renderer() != Renderer::Egui => session::widget_session_for(ps, Renderer::Egui)?.dialog(title, kind, arm(ps)),
        other => other,
    };
    match asked {
        Ok(answer) => Ok(Some(answer)),
        Err(GuiError::Canceled) => Ok(None),
        Err(e) => Err(gui_error(e)),
    }
}

/// Draws the dialog an HTML window's file chooser passed its command, the
/// system having none of its kind, with the egui renderer's own, and
/// returns how it was answered; a wait Ctrl+C ended returns a cancel.
pub(crate) fn draw_for_chooser(ps: &Pipeline<'_>, title: &str, kind: DialogKind) -> PsResult<Result<DialogAnswer, DialogFailure>> {
    let session = session::egui_session(ps)?;
    Ok(match session.dialog(title, kind, arm(ps)) {
        Ok(answer) => Ok(answer),
        Err(GuiError::Canceled) => Ok(DialogAnswer::Canceled),
        Err(e) => Err(DialogFailure::Failed(e.to_string())),
    })
}

/// As [`draw_for_chooser`], for a thread with no pipeline, as a `-NoWait`
/// window's reader: the egui session is started, when none runs, as the
/// native entry starts one, and nothing ends the wait but an answer.
pub(crate) fn draw_for_chooser_unattended(title: &str, kind: DialogKind) -> Result<DialogAnswer, DialogFailure> {
    let session = session::egui_session_unattended().map_err(DialogFailure::Failed)?;
    session.dialog(title, kind, |_: Canceler| Ok::<(), GuiError>(())).map_err(|e| DialogFailure::Failed(e.to_string()))
}

/// A file or folder dialog's paths, written one by one; nothing for a
/// cancel or a stopped wait.
fn write_paths(ps: &Pipeline<'_>, answer: Option<DialogAnswer>) -> PsResult<()> {
    match answer {
        Some(DialogAnswer::Paths(paths)) => paths.into_iter().try_for_each(|p| ps.write(p)),
        Some(DialogAnswer::Canceled) | None => Ok(()),
        Some(DialogAnswer::Button(b)) => Err(unexpected(format!("the dialog returned the button {b:?}"))),
    }
}

/// -Title, or the empty title that leaves the system's own.
fn title_of(title: &Option<String>) -> &str {
    match title {
        Some(t) => t,
        None => "",
    }
}

/// -InitialDirectory as a full path.
fn directory_of(ps: &Pipeline<'_>, given: Option<&str>) -> PsResult<Option<String>> {
    match given {
        Some(d) => Ok(Some(widgets::full_path(ps, d)?)),
        None => Ok(None),
    }
}

/// -Filter as the dialog takes it: each kind's name and its extensions.
fn filters_of(ps: &Pipeline<'_>, given: Option<&PsObject>) -> PsResult<Vec<(String, Vec<String>)>> {
    let refuse = |text: String| PsError::new(ErrorCategory::InvalidArgument, "GuiBadFilter", format!("-Filter: {text}")).terminating();
    let Some(given) = given else {
        return Ok(Vec::new());
    };
    let items = match widgets::filter_of(ps, given).map_err(refuse)? {
        Value::List(items) => items,
        other => return Err(refuse(format!("the kinds of files read as {other:?}, not a list"))),
    };
    let mut out = Vec::new();
    for item in items {
        let entry = match &item {
            Value::Map(m) => (m.get("name"), m.get("extensions")),
            other => return Err(refuse(format!("a kind of file read as {other:?}, not a name and its extensions"))),
        };
        match entry {
            (Some(Value::Str(name)), Some(Value::List(extensions))) => {
                let mut kept = Vec::new();
                for e in extensions {
                    match e {
                        Value::Str(s) => kept.push(s.clone()),
                        other => return Err(refuse(format!("{name}'s extension read as {other:?}, not text"))),
                    }
                }
                out.push((name.clone(), kept));
            }
            other => return Err(refuse(format!("a kind of file read as {other:?}, not a name and its extensions"))),
        }
    }
    Ok(out)
}

fn unexpected(text: String) -> PsError {
    PsError::new(ErrorCategory::InvalidResult, "GuiBadAnswer", text).terminating()
}
