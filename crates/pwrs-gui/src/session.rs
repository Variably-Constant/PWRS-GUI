//! The module's renderer sessions, one per renderer, shared by every
//! runspace in the process, and the waits every command makes on them.

use crate::options::{self, Env};
use pwrs::StopWaker;
use pwrs::prelude::*;
use pwrs_gui_client::proto::{CloseReason, trace};
use pwrs_gui_client::{Canceler, GuiError, Renderer, Session, SessionSlot, StartOptions, Window, WindowEvent};
use std::ffi::OsString;
use std::path::PathBuf;

static EGUI: SessionSlot = SessionSlot::new();
static HTML: SessionSlot = SessionSlot::new();

/// The slot the session of `renderer` lives in, which the native entry
/// offers.
pub fn slot(renderer: Renderer) -> &'static SessionSlot {
    match renderer {
        Renderer::Egui => &EGUI,
        Renderer::Html => &HTML,
    }
}

/// Every renderer's slot.
pub fn slots() -> [&'static SessionSlot; 2] {
    [&EGUI, &HTML]
}

/// The live sessions, without starting one.
pub fn running() -> Vec<Session> {
    slots().iter().filter_map(|s| s.current()).collect()
}

/// The live session of `renderer`, without starting one.
pub fn running_of(renderer: Renderer) -> Option<Session> {
    slot(renderer).current()
}

/// What a session of `renderer` that another module starts through the
/// native entry starts with: as `Set-GuiOption` says, with `PWRS_GUI_MODE`
/// and `PWRS_GUI_WEBVIEW` read from the process's environment, since the
/// entry has no session to read.
pub fn start_options(renderer: Renderer) -> Result<StartOptions, String> {
    options_for(renderer, &Env::Process)
}

/// What a session of `renderer` starts with now, as `Set-GuiOption`,
/// `PWRS_GUI_MODE` and `PWRS_GUI_WEBVIEW` say, the variables and the
/// temporary folder read from `env`.
fn options_for(renderer: Renderer, env: &Env<'_, '_>) -> Result<StartOptions, String> {
    let o = options::current(env).map_err(|e| e.message)?;
    Ok(StartOptions {
        mode: o.mode.into(),
        renderer,
        helper: pwrs::helper_path(renderer.helper_name()).map_err(|e| e.message),
        adapter: o.adapter,
        low_power: o.low_power,
        log_dir: temp_dir(env)?.join("PWRS-GUI"),
        web_view: o.web_view.into(),
        web_view_data_folder: o.web_view_data_folder,
        fonts: o.font.into(),
        environment: None,
    })
}

/// The temporary folder: `TMPDIR` as `env` has it on Linux, FreeBSD and
/// macOS when it is set, else the one Rust gives for the process.
fn temp_dir(env: &Env<'_, '_>) -> Result<PathBuf, String> {
    if cfg!(unix) {
        match env.var("TMPDIR").map_err(|e| format!("TMPDIR cannot be read: {e}"))? {
            Some(dir) if !dir.is_empty() => return Ok(PathBuf::from(dir)),
            _unset_or_empty => {}
        }
    }
    Ok(std::env::temp_dir())
}

/// The session's environment, which a helper starts with on Linux, FreeBSD
/// and macOS, where what `$env:` sets stays out of the process's own
/// environment; `None` on Windows, where the two are one.
fn environment(ps: &Pipeline<'_>) -> PsResult<Option<Vec<(OsString, OsString)>>> {
    if cfg!(windows) {
        return Ok(None);
    }
    let mut vars = Vec::new();
    for entry in ps.invoke("Microsoft.PowerShell.Management\\Get-ChildItem", &[("Path", "Env:".into_ps()?)])? {
        let name = String::from_ps(&entry.get("Key")?)?;
        let value = entry.get("Value")?;
        let value = if value.is_null() { String::new() } else { String::from_ps(&value)? };
        vars.push((OsString::from(name), OsString::from(value)));
    }
    Ok(Some(vars))
}

/// The live egui session, or one started from the process's environment as
/// the native entry starts one, for a thread with no pipeline, such as a
/// `-NoWait` window's reader; nothing ends the wait for it but its start.
pub fn egui_session_unattended() -> Result<Session, String> {
    let slot = slot(Renderer::Egui);
    if let Some(live) = slot.current() {
        return Ok(live);
    }
    let start = start_options(Renderer::Egui)?;
    match slot.get_or_start(&start, |_: Canceler| Ok::<(), GuiError>(())) {
        Ok((session, fell_back)) => {
            if let Some(why) = fell_back {
                eprintln!("PWRS-GUI: the egui helper could not start, so its windows are drawn in this process: {why}");
            }
            Ok(session)
        }
        Err(e) => Err(format!("the egui renderer could not start: {e}")),
    }
}

/// The live egui session, or one started as `Set-GuiOption` and
/// `PWRS_GUI_MODE` say; see [`session_of`].
pub fn egui_session(ps: &Pipeline<'_>) -> PsResult<Session> {
    session_of(ps, Renderer::Egui)
}

/// The session a widget window draws in: that of the renderer
/// `Set-GuiOption -Renderer` names. When that is HTML and its renderer
/// cannot start, the window draws with egui, and the Verbose stream says
/// why.
pub fn widget_session(ps: &Pipeline<'_>) -> PsResult<Session> {
    let renderer: Renderer = options::current(&Env::Session(ps))?.renderer.into();
    widget_session_for(ps, renderer)
}

/// The session a widget window requested in `renderer` draws in: when that
/// is HTML and its renderer cannot start, the window draws with egui, and
/// the Verbose stream says why.
pub fn widget_session_for(ps: &Pipeline<'_>, renderer: Renderer) -> PsResult<Session> {
    if renderer == Renderer::Egui {
        return egui_session(ps);
    }
    match get_or_start(ps, Renderer::Html)? {
        Ok(session) => Ok(session),
        Err(GuiError::Start(why)) => {
            pwrs::verbose!(ps, "the HTML renderer could not start, so this window is drawn with egui: {why}")?;
            egui_session(ps)
        }
        Err(e) => Err(gui_error(e)),
    }
}

/// The HTML session, for a window of raw HTML, which only a webview draws.
/// When the renderer cannot start, the error gives its reason and names
/// what to install when the engine is missing.
pub fn html_session(ps: &Pipeline<'_>) -> PsResult<Session> {
    match get_or_start(ps, Renderer::Html)? {
        Ok(session) => Ok(session),
        Err(GuiError::Start(why)) => Err(PsError::new(
            ErrorCategory::ResourceUnavailable,
            "GuiNoWebview",
            format!("the HTML renderer could not start, so an HTML window cannot open: {why}. {}", install_hint()),
        )
        .terminating()),
        Err(e) => Err(gui_error(e)),
    }
}

/// What to install when this platform's webview engine is missing.
fn install_hint() -> String {
    if cfg!(windows) {
        String::from("When WebView2 is not installed, install Microsoft's WebView2 Runtime: https://developer.microsoft.com/microsoft-edge/webview2/")
    } else if cfg!(target_vendor = "apple") {
        String::from("WebKit is part of macOS, and the HTML helper could not use it")
    } else {
        let rid = if cfg!(target_os = "freebsd") { "freebsd-x64" } else { "linux-x64" };
        format!("When WebKitGTK 4.1 is not installed, install the system's libwebkit2gtk-4.1, or run Install-Module PWRS-GUI.WebKit.{rid}")
    }
}

/// The live session of `renderer`, or one started as `Set-GuiOption` and
/// `PWRS_GUI_MODE` say. A start waits for the renderer with no time limit,
/// and Ctrl+C ends the wait; a start another command has under way is
/// waited on, not repeated.
pub fn session_of(ps: &Pipeline<'_>, renderer: Renderer) -> PsResult<Session> {
    get_or_start(ps, renderer)?.map_err(gui_error)
}

/// As [`session_of`], with the session's own error, so a caller can
/// tell a renderer that cannot start from the rest.
fn get_or_start(ps: &Pipeline<'_>, renderer: Renderer) -> PsResult<Result<Session, GuiError>> {
    let slot = slot(renderer);
    if let Some(live) = slot.current() {
        return Ok(Ok(live));
    }
    let mut start = options_for(renderer, &Env::Session(ps)).map_err(|e| PsError::new(ErrorCategory::InvalidData, "GuiOptions", e).terminating())?;
    start.environment = environment(ps)?;
    trace::decide(start.var("PWRS_GUI_TRACE").is_some());
    pwrs::verbose!(ps, "starting the GUI's {} renderer in {:?} mode and waiting for it with no time limit; Ctrl+C ends the wait", renderer.name(), start.mode)?;
    let (session, fell_back) = match slot.get_or_start(&start, arm(ps)) {
        Ok(started) => started,
        Err(e) => return Ok(Err(e)),
    };
    if let Some(why) = fell_back {
        pwrs::verbose!(ps, "the GUI's {} helper could not start, so its windows are drawn in this process: {why}", renderer.name())?;
    }
    if let Some(hello) = session.hello() {
        for note in &hello.notes {
            pwrs::warning!(ps, "the GUI's {} renderer started, and {note}", renderer.name())?;
        }
    }
    Ok(Ok(session))
}

/// The live session that numbered window `window`, or an error saying it
/// is not running, for a command that acts on a window already open.
pub fn of_window(window: u64) -> PsResult<Session> {
    let renderer = Renderer::of_number(window);
    match slot(renderer).current() {
        Some(live) => Ok(live),
        None => Err(PsError::new(
            ErrorCategory::ObjectNotFound,
            "GuiNoSession",
            format!("window {window} would be drawn by the {} renderer, and none is running in this process, so there is no renderer to send it to", renderer.name()),
        )),
    }
}

/// Ends every session, closing every window.
pub fn end() {
    for slot in slots() {
        if let Some(s) = slot.take() {
            s.shut_down();
        }
    }
}

/// The arm each wait takes: it passes the wait's canceler to the pipeline's
/// stop, so Ctrl+C ends the wait, and unregisters when the wait ends.
pub fn arm<'a, 'ps>(ps: &'a Pipeline<'ps>) -> impl FnMut(Canceler) -> Result<StopWaker<'ps>, GuiError> + 'a {
    move |c: Canceler| ps.on_stop(move || c.cancel()).map_err(|e| GuiError::Arm(e.message))
}

/// The window's next event, waiting with no time limit; `None` when
/// Ctrl+C ended the wait.
pub fn next_event(ps: &Pipeline<'_>, window: &mut Window) -> PsResult<Option<WindowEvent>> {
    let canceler = window.canceler();
    let waker = ps.on_stop(move || canceler.cancel())?;
    let event = window.next_event();
    drop(waker);
    match event {
        Ok(e) => Ok(Some(e)),
        Err(GuiError::Canceled) => Ok(None),
        Err(e) => Err(gui_error(e)),
    }
}

/// Waits until the renderer has drawn `window`'s first frame, which is when
/// Get-GuiWindow lists it, so a command that leaves its window open writes
/// a window its title already finds. Gives back the events that came
/// first, in order, for the window's reader; a close among them ends the
/// wait, and a page that could not be shown ends the command as
/// [`closed_error`] says. `None` when Ctrl+C ended the wait.
pub fn first_frame(ps: &Pipeline<'_>, window: &mut Window) -> PsResult<Option<Vec<WindowEvent>>> {
    let mut before = Vec::new();
    let mut problem = None;
    loop {
        let Some(event) = next_event(ps, window)? else {
            return Ok(None);
        };
        match event {
            WindowEvent::Opened => return Ok(Some(before)),
            WindowEvent::Closed(reason) => {
                if let Some(e) = closed_error(reason, problem.as_deref()) {
                    return Err(e);
                }
                before.push(WindowEvent::Closed(reason));
                return Ok(Some(before));
            }
            WindowEvent::PageClosed(json) => {
                before.push(WindowEvent::PageClosed(json));
                return Ok(Some(before));
            }
            WindowEvent::Problem(p) => {
                problem = Some(p.clone());
                before.push(WindowEvent::Problem(p));
            }
            other => before.push(other),
        }
    }
}

/// What a window that closed for `reason` ends its command with: nothing
/// for a close the person or the session made, and for a page that could
/// not be shown a terminating error saying why, as a dead helper's windows
/// end theirs. `problem` is what the renderer reported last before the
/// close.
pub fn closed_error(reason: CloseReason, problem: Option<&str>) -> Option<PsError> {
    match reason {
        CloseReason::Failed => {
            let why = problem.unwrap_or("the engine drawing its page ended");
            Some(PsError::new(ErrorCategory::ResourceUnavailable, "GuiPageFailed", format!("the window closed because its page could not be shown: {why}")).terminating())
        }
        CloseReason::User | CloseReason::Program | CloseReason::Shutdown => None,
    }
}

/// A session error as PowerShell reports it. A failure that leaves the
/// command nothing to do is terminating; a refusal is not, so a command
/// given several targets goes on with the rest.
pub fn gui_error(e: GuiError) -> PsError {
    let (category, id, terminating) = match &e {
        GuiError::NoDisplay(_) => (ErrorCategory::ResourceUnavailable, "GuiNoDisplay", true),
        GuiError::Start(_) => (ErrorCategory::ResourceUnavailable, "GuiStartFailed", true),
        GuiError::Mismatch { .. } => (ErrorCategory::InvalidOperation, "GuiBuildMismatch", true),
        GuiError::Canceled => (ErrorCategory::OperationStopped, "GuiCanceled", true),
        GuiError::Gone(_) => (ErrorCategory::ResourceUnavailable, "GuiRendererGone", true),
        GuiError::Refused(_) => (ErrorCategory::InvalidArgument, "GuiRefused", false),
        GuiError::Late(_) => (ErrorCategory::OperationTimeout, "GuiTimeout", false),
        GuiError::Transport(_) => (ErrorCategory::ConnectionError, "GuiTransport", true),
        GuiError::Arm(_) => (ErrorCategory::InvalidOperation, "GuiWaitNotArmed", true),
        GuiError::NoNativeDialog(_) => (ErrorCategory::ResourceUnavailable, "GuiNoNativeDialog", true),
        GuiError::Watch(_) => (ErrorCategory::ResourceUnavailable, "GuiNoWatch", true),
    };
    let error = PsError::new(category, id, e.to_string());
    if terminating { error.terminating() } else { error }
}
