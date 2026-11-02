//! A stand-in for a module that draws through PWRS-GUI's session rather
//! than starting a renderer of its own: it takes the session through
//! `Get-GuiNativeEntry` and opens its windows with it, so the process
//! has one renderer whichever modules draw. Built for PWRS-GUI's tests,
//! and the smallest whole example of a module taking the session.

use pwrs::prelude::*;
use pwrs_gui_client::proto::{EventKind, Kind, Node, Value, WindowOptions};
use pwrs_gui_client::{Canceler, GuiError, NativeEntry, Renderer, Session, WindowEvent};

fn gui_error(e: GuiError) -> PsError {
    PsError::new(ErrorCategory::ResourceUnavailable, "StandinGui", e.to_string()).terminating()
}

/// One property of the entry Get-GuiNativeEntry wrote.
fn field(entry: &PsObject, name: &str) -> PsResult<u64> {
    u64::from_ps(&entry.get(name)?)
}

/// PWRS-GUI's session of `renderer`, taken through the native entry of the
/// PWRS-GUI module imported in this runspace, and started when none is
/// running; Ctrl+C ends a wait for it.
fn take(ps: &Pipeline<'_>, renderer: Renderer) -> PsResult<Session> {
    let answers = ps.invoke("Get-GuiNativeEntry", &[])?;
    let Some(entry) = answers.first() else {
        return Err(PsError::new(ErrorCategory::ObjectNotFound, "StandinNoEntry", "Get-GuiNativeEntry wrote nothing").terminating());
    };
    let narrow = |name: &str| -> PsResult<u32> {
        let n = field(entry, name)?;
        u32::try_from(n).map_err(|e| PsError::new(ErrorCategory::InvalidData, "StandinEntry", format!("{name} is {n}, which is not a 32-bit number: {e}")).terminating())
    };
    // SAFETY: the addresses are what PWRS-GUI's Get-GuiNativeEntry returned
    // in this process, and PWRS never unloads the library they are in.
    let native = unsafe {
        NativeEntry::from_addresses(narrow("AbiVersion")?, narrow("Protocol")?, field(entry, "StartV1")?, field(entry, "WatchV1")?, field(entry, "UnwatchV1")?, field(entry, "SendV1")?)
    }
    .map_err(gui_error)?;
    native.session(renderer, |c: Canceler| ps.on_stop(move || c.cancel()).map_err(|e| GuiError::Arm(e.message))).map_err(gui_error)
}

/// The renderer a command requests with -Html.
fn renderer(html: bool) -> Renderer {
    if html { Renderer::Html } else { Renderer::Egui }
}

/// Shows the choices in a window PWRS-GUI's session draws, and writes the
/// one selected when OK is pressed; nothing when the window is closed or
/// Ctrl+C ends the wait.
///
/// # Examples
/// Read-StandinChoice -Choice 'red', 'green'
/// Read-StandinChoice -Choice 'red', 'green' -Html
#[cmdlet(verb = "Read", noun = "StandinChoice", output = ["System.String"])]
#[derive(Default)]
pub struct ReadStandinChoice {
    /// The choices.
    #[param(mandatory, position = 0)]
    pub choice: Vec<String>,
    /// Draws through PWRS-GUI's HTML session instead of its egui session.
    #[param]
    pub html: bool,
}

impl Cmdlet for ReadStandinChoice {
    fn process(&mut self, ps: &Pipeline<'_>) -> PsResult<()> {
        let session = take(ps, renderer(self.html))?;
        pwrs::verbose!(ps, "drawing through PWRS-GUI's session {}, whose renderer is {}", session.id(), session.path())?;
        let items = Value::List(self.choice.iter().cloned().map(Value::Str).collect());
        let tree = Node::new(1, Kind::Column)
            .child(Node::new(2, Kind::ListBox).named("Choice").prop("items", items))
            .child(Node::new(3, Kind::Button).named("Ok").prop("text", Value::Str(String::from("OK"))));
        let options = WindowOptions { close_on_escape: true, ..WindowOptions::titled("PWRS-GUI stand-in") };
        let mut window = session.open_window(options, tree).map_err(gui_error)?;
        let mut selected = None;
        loop {
            let canceler = window.canceler();
            let waker = ps.on_stop(move || canceler.cancel())?;
            let event = window.next_event();
            drop(waker);
            match event {
                Ok(WindowEvent::Event { node: 2, kind: EventKind::Select, value: Value::List(rows), .. }) => {
                    selected = rows.first().and_then(Value::as_int);
                }
                Ok(WindowEvent::Event { node: 3, kind: EventKind::Click, .. }) => break,
                Ok(WindowEvent::Closed(_) | WindowEvent::PageClosed(_)) | Err(GuiError::Canceled) => return Ok(()),
                Ok(WindowEvent::Problem(p)) => pwrs::warning!(ps, "the stand-in's window reported: {p}")?,
                Ok(WindowEvent::Message { json, .. }) => pwrs::warning!(ps, "the stand-in's window, which shows no page, received a page's message: {json}")?,
                Ok(WindowEvent::Opened | WindowEvent::Event { .. } | WindowEvent::PickerWanted { .. } | WindowEvent::FileChanged { .. }) => {}
                Err(e) => return Err(gui_error(e)),
            }
        }
        window.close().map_err(gui_error)?;
        if let Some(row) = selected {
            let bad_row = |why: String| PsError::new(ErrorCategory::InvalidData, "StandinRow", why).terminating();
            let at = usize::try_from(row).map_err(|e| bad_row(format!("row {row} is not a row number: {e}")))?;
            match self.choice.get(at) {
                Some(chosen) => ps.write(chosen.clone())?,
                None => return Err(bad_row(format!("row {row} is not one of the {} choices", self.choice.len()))),
            }
        }
        Ok(())
    }
}

/// Writes the name of PWRS-GUI's session as this module took it, and where
/// its renderer draws, starting it when none is running.
///
/// # Examples
/// Get-StandinSession
/// Get-StandinSession -Html
#[cmdlet(verb = "Get", noun = "StandinSession", output = ["System.String"])]
#[derive(Default)]
pub struct GetStandinSession {
    /// Takes PWRS-GUI's HTML session instead of its egui session.
    #[param]
    pub html: bool,
}

impl Cmdlet for GetStandinSession {
    fn process(&mut self, ps: &Pipeline<'_>) -> PsResult<()> {
        let session = take(ps, renderer(self.html))?;
        ps.write(format!("{} {}", session.id(), session.path()))
    }
}

pwrs::export_module! {
    name: "PwrsGuiStandin",
    cmdlets: [ReadStandinChoice, GetStandinSession],
}
