//! Windows of HTML: `Show-GuiWindow -Html` or `-Path`, and
//! `Send-GuiMessage`. A page talks to the session through `window.pwrs`:
//! `send(value)` returns a Promise of the -OnMessage handler's output,
//! `on(handler)` takes what Send-GuiMessage sends, `close(value)` ends the
//! window with a result, and `save(value)` stores a value `restore()` gives
//! back to the page loaded again after its engine ended. Values cross as
//! JSON, converted on this side by PowerShell's own ConvertFrom-Json and
//! ConvertTo-Json.

use crate::options::GuiRenderer;
use crate::session::{self, gui_error};
use crate::window::{self, GuiHandlerOutput, ShowArgs};
use pwrs::prelude::*;
use pwrs_gui_client::proto::{Fit, Kind, Node, Value, WindowOptions};
use pwrs_gui_client::{Session, Window, WindowEvent};
use std::path::PathBuf;

/// A value from JSON, as ConvertFrom-Json makes it, an array left whole.
const FROM_JSON: &str = "param($j) if ($PSVersionTable.PSVersion.Major -ge 7) { ConvertFrom-Json -InputObject $j -NoEnumerate } else { ConvertFrom-Json -InputObject $j }";

/// The PowerShell that turns a handler's output into JSON: nothing, or one
/// null, is null; one object is itself; several are an array. Each value is
/// first made plain data, to a depth of 100: a string or number as its base
/// value, without the note properties a provider attaches; a date as ISO
/// 8601; a list, dictionary or custom object walked. Windows PowerShell
/// 5.1's ConvertTo-Json would otherwise write an array it is given as one
/// object as `{"value":[...],"Count":n}`, and a file name from
/// Get-ChildItem -Name with its provider and drive to the full depth.
/// `$o` is the output, and the JSON is left in `$r`.
macro_rules! answer_json {
    () => {
        r#"function __plain($x, [int] $depth) {
    if ($null -eq $x) { return $null }
    $b = $x.psobject.BaseObject
    if ($b -is [datetime] -or $b -is [datetimeoffset]) { return $b.ToString('o') }
    if ($b -is [string] -or $b -is [ValueType]) { return $b }
    if ($depth -ge 100) { return $x }
    if ($b -is [System.Collections.IDictionary]) {
        $d = [ordered]@{}
        foreach ($k in $b.Keys) { $d[[string]$k] = __plain $b[$k] ($depth + 1) }
        return $d
    }
    if ($b -is [System.Collections.IList]) {
        $l = [System.Collections.Generic.List[object]]::new()
        foreach ($e in $b) { $l.Add((__plain $e ($depth + 1))) }
        return , $l.ToArray()
    }
    if ($b -is [System.Management.Automation.PSCustomObject]) {
        $d = [ordered]@{}
        foreach ($p in $x.psobject.Properties) { $d[$p.Name] = __plain $p.Value ($depth + 1) }
        return $d
    }
    $x
}
$items = [System.Collections.Generic.List[object]]::new()
foreach ($x in @($o)) { $items.Add((__plain $x 0)) }
$r = if ($items.Count -eq 0 -or ($items.Count -eq 1 -and $null -eq $items[0])) { 'null' } elseif ($items.Count -eq 1) { ConvertTo-Json -InputObject $items[0] -Compress -Depth 100 } else { ConvertTo-Json -InputObject $items.ToArray() -Compress -Depth 100 }
"#
    };
}

/// A handler's output as JSON, as [`answer_json`] writes it.
const TO_JSON: &str = concat!("param($o)\n", answer_json!(), "$r");

/// What a -NoWait window's handler runs as, on the pipeline thread once
/// the engine is idle: the message converted, the handler run, and its
/// output sent back as the send's reply, as [`answer_json`] writes it.
const ACTION: &str = concat!(
    "$d = $Event.MessageData\n",
    "$m = if ($PSVersionTable.PSVersion.Major -ge 7) { ConvertFrom-Json -InputObject $d.Json -NoEnumerate } else { ConvertFrom-Json -InputObject $d.Json }\n",
    "$o = @(& $d.Handler $m)\n",
    answer_json!(),
    "$d.Window.Reply($d.Call, $r)"
);

fn script(text: &str) -> PsResult<PsScriptBlock> {
    Ok(PsScriptBlock(PsType::from_name("scriptblock").call_static("Create", &[text.into_ps()?])?))
}

/// The PowerShell value in `json`; `$null` for `null`.
fn from_json(ps: &Pipeline<'_>, json: &str) -> PsResult<PsObject> {
    let mut out = script(FROM_JSON)?.call(ps, &[json.into_ps()?])?;
    match out.len() {
        0 => ().into_ps(),
        1 => Ok(out.remove(0)),
        _ => out.into_ps(),
    }
}

/// Output as JSON, as [`TO_JSON`] writes it.
fn to_json(ps: &Pipeline<'_>, output: Vec<PsObject>) -> PsResult<String> {
    let answered = script(TO_JSON)?.call(ps, &[output.into_ps()?])?;
    match answered.first() {
        Some(text) => String::from_ps(text),
        None => Err(PsError::new(ErrorCategory::InvalidResult, "GuiJson", "ConvertTo-Json wrote nothing").terminating()),
    }
}

/// A window of HTML that `Show-GuiWindow -NoWait` opened.
#[psclass(name = "PwrsGui.PageWindow", mode = proxy)]
pub struct PageWindow {
    /// The window's number, which Send-GuiMessage takes as -Window.
    pub id: u64,
    /// The window's title.
    pub title: String,
    /// The engine event each message raises for -OnMessage's action; null
    /// without -OnMessage. Unregister-Event removes the action.
    pub source_identifier: Option<String>,
    #[psfield(skip)]
    session: Session,
}

#[psmethods]
impl PageWindow {
    /// Replies to the page's `window.pwrs.send` numbered `call` with a value
    /// given as JSON; the action -OnMessage runs calls it.
    pub fn reply(&self, call: u32, json: String) -> PsResult<()> {
        self.session.reply(self.id, call, json).map_err(gui_error)
    }
}

/// What a message's engine event gives -OnMessage's action.
#[psclass(name = "PwrsGui.PageMessage")]
#[derive(Clone, Default)]
pub struct PageMessage {
    /// The window the message came from.
    pub window: PsObject,
    /// The page's number for its send, which the reply is for.
    pub call: u32,
    /// The value the page sent, as JSON.
    pub json: String,
    /// The -OnMessage script block.
    pub handler: PsObject,
}

/// Shows a window: one described in PowerShell with -Content, or a page of
/// HTML given as text with -Html, or as a file or a folder with -Path.
///
/// -Content is a block that writes the window's widgets. While it runs,
/// each widget's short name, such as Column, Text, Button and ListBox, is
/// an alias to its New-Gui<Widget> command, so nothing outside the block is
/// shadowed; inside it, Group-Object is spelled out. The command runs the
/// widgets' handlers on this thread as their events arrive, each with the
/// event as its argument: Window, Name, Kind, Value, and State, every named
/// input's current value. What a handler writes is dropped unless
/// -HandlerOutput says otherwise. When the window closes, the command
/// writes an object whose Result is what a -Result button or
/// Close-GuiWindow gave, or 'Closed', and whose State is the inputs' last
/// values. Without -Width or -Height the window fits its widgets on that
/// side, and its lists and tables scroll once it would pass nine tenths of
/// the screen's work area across or eight tenths down.
///
/// -WatchPath names files the window watches while it is open. A change to
/// one, such as a line added to a log, raises an event whose Kind is
/// FileChange, whose Name is the file's full path and whose Value is its
/// length in bytes, null while the file is gone; -OnFileChange runs with
/// it, and with -NoWait Wait-GuiEvent takes it. The system's change
/// notifications tell of each change, and on Windows a look at each file
/// once a second as well, since NTFS reports no write that a writer leaves
/// in the system's cache. -WatchInterval looks at that pace instead, on any
/// system, for a file system that gives no notifications, such as a network
/// share. A file has one change waiting at a time; the next comes once that
/// one is taken.
///
/// A page loads through pwrs://, with no server; it may not leave its own
/// origin or load remote content unless -AllowNavigation, and opens
/// developer tools only with -DevTools.
///
/// The page gets window.pwrs: send(value) gives -OnMessage the value, and
/// returns a Promise of what the handler returns; on(handler) takes what
/// Send-GuiMessage sends; close(value) closes the window; save(value) stores
/// a value that restore() returns a Promise of, so a page loaded again
/// after its engine ended picks up from it. Values cross as JSON. A page
/// whose engine ends is loaded again; one that ends again before it draws
/// closes its window, and the command fails saying why. The command waits
/// until the window closes and writes an object
/// whose Result is the value the page gave close, or 'Closed' when the
/// person or the session closed it. With -NoWait it returns the window once
/// it has opened, with its first frame drawn, and -OnMessage runs as an
/// engine-event action when the engine is idle.
///
/// HTML windows need a webview engine: WebView2 on Windows, WebKit on
/// macOS, WebKitGTK on Linux and FreeBSD. When none can start, the command
/// fails naming what to install.
///
/// # Examples
/// Show-GuiWindow { Text 'Deploy to' -Style Instruction; ListBox 'staging', 'production' -Name Target; ButtonBar { Button 'OK' -Default -Result Ok; Button 'Cancel' -Cancel -Result Cancel } }
/// Show-GuiWindow -Html '<button onclick="pwrs.close(42)">Done</button>'
/// Show-GuiWindow -Path .\site -OnMessage { param($m) "got $m" }
#[cmdlet(verb = "Show", noun = "GuiWindow", default_parameter_set = "Content", output = ["PwrsGui.WindowResult", "PwrsGui.PageWindow", "PwrsGui.Window"])]
#[derive(Default)]
pub struct ShowGuiWindow {
    /// The block that writes the window's widgets, in which each widget's
    /// short name stands for its New-Gui<Widget> command.
    #[param(mandatory, position = 0, set = "Content")]
    pub content: Option<PsScriptBlock>,
    /// The page's HTML. Text that does not begin with a doctype loads behind
    /// <!DOCTYPE html>, so the page is laid out by the current standards
    /// rather than in the webview's quirks mode.
    #[param(mandatory, position = 0, set = "Html")]
    pub html: Option<String>,
    /// A page's file, or a folder whose index.html is the page.
    #[param(mandatory, set = "Path")]
    pub path: Option<String>,
    /// The window's title.
    #[param]
    pub title: Option<String>,
    /// The window's inside width, in points. Without it, a window of widgets
    /// is as wide as its widgets, from 320 points to nine tenths of the
    /// screen's work area, and a page's window is 800 points wide.
    #[param(validate_range(100, 10000))]
    pub width: Option<f64>,
    /// The window's inside height, in points. Without it, a window of
    /// widgets is as high as its widgets, up to eight tenths of the screen's
    /// work area, and a page's window is 600 points high.
    #[param(validate_range(100, 10000))]
    pub height: Option<f64>,
    /// Runs for each value the page sends with window.pwrs.send, with the
    /// value as its argument; what it outputs is the send's reply.
    #[param(set = ["Html", "Path"])]
    pub on_message: Option<PsScriptBlock>,
    /// Lets the page leave its own origin and load remote content.
    #[param(set = ["Html", "Path"])]
    pub allow_navigation: bool,
    /// Lets the page open the webview's developer tools.
    #[param(set = ["Html", "Path"])]
    pub dev_tools: bool,
    /// The renderer that draws a window of widgets, Egui or Html; the one
    /// Set-GuiOption -Renderer names when not given. A window requested in
    /// HTML draws with egui when no webview engine can start, and the
    /// Verbose stream says why.
    #[param(set = "Content")]
    pub renderer: Option<GuiRenderer>,
    /// What happens to what the handlers write: Discard drops it, Pipeline
    /// writes it as the command's output before the result, Console sends
    /// it to Out-Default. Discard when not given.
    #[param(set = "Content")]
    pub handler_output: GuiHandlerOutput,
    /// Runs once the window has closed, with an event whose Kind is Close,
    /// whose Value is the window's result and whose State is the inputs'
    /// last values.
    #[param(set = "Content")]
    pub on_close: Option<PsScriptBlock>,
    /// The window's own keyboard shortcuts: a hashtable from each, as
    /// Ctrl+S, to the script block that runs when it is pressed, with an
    /// event whose Kind is Shortcut and whose Value is the shortcut.
    #[param(set = "Content")]
    pub on_shortcut: Option<PsObject>,
    /// Runs when files or text are dropped on the window where no widget
    /// under the pointer takes them, with the event as its argument; its
    /// Value is the paths dropped, or the text.
    #[param(set = "Content")]
    pub on_drop: Option<PsScriptBlock>,
    /// Files the window watches while it is open, each a path from the
    /// current location; a file need not exist yet, but its folder must. A
    /// change to one raises a FileChange event, whose Name is the file's
    /// full path and whose Value is its length in bytes.
    #[param(set = "Content")]
    pub watch_path: Option<Vec<String>>,
    /// The seconds, or a [TimeSpan], between looks at the watched files,
    /// above 0, in place of the system's change notifications; for a file
    /// system that gives none, such as a network share.
    #[param(set = "Content")]
    pub watch_interval: Option<PsObject>,
    /// Runs when a watched file changes, with the event as its argument and
    /// as $_; its Name is the file's full path and its Value its length in
    /// bytes, null while the file is gone.
    #[param(set = "Content")]
    pub on_file_change: Option<PsScriptBlock>,
    /// Returns the window once it has opened, with its first frame drawn,
    /// instead of waiting for it to close.
    #[param]
    pub no_wait: bool,
}

impl ShowGuiWindow {
    /// The page's node: its text, or its folder and first file.
    fn page_node(&self, ps: &Pipeline<'_>) -> PsResult<(Node, String)> {
        let mut node = Node::new(1, Kind::Html).prop("allow_navigation", Value::Bool(self.allow_navigation)).prop("dev_tools", Value::Bool(self.dev_tools));
        let title = match (&self.html, &self.path) {
            (Some(html), _) => {
                node = node.prop("html", Value::Str(html.clone()));
                String::from("PWRS-GUI")
            }
            (None, Some(path)) => {
                let full = match ps.resolve_path(path, true)?.into_iter().next() {
                    Some(p) => PathBuf::from(p),
                    None => return Err(PsError::new(ErrorCategory::ObjectNotFound, "GuiNoPage", format!("{path} resolves to nothing")).terminating()),
                };
                if !full.exists() {
                    return Err(PsError::new(ErrorCategory::ObjectNotFound, "GuiNoPage", format!("{path} names nothing that exists")).terminating());
                }
                let (folder, first) = if full.is_dir() {
                    (full.clone(), String::from("index.html"))
                } else {
                    let folder = full.parent().map(PathBuf::from).ok_or_else(|| PsError::new(ErrorCategory::InvalidArgument, "GuiNoPage", format!("{} is in no folder", full.display())).terminating())?;
                    let name = full.file_name().and_then(|n| n.to_str()).map(String::from).ok_or_else(|| PsError::new(ErrorCategory::InvalidArgument, "GuiNoPage", format!("{} names no file", full.display())).terminating())?;
                    (folder, name)
                };
                if !folder.join(&first).is_file() {
                    let text = format!("{} has no {first} to open", folder.display());
                    return Err(PsError::new(ErrorCategory::ObjectNotFound, "GuiNoPage", text).terminating());
                }
                node = node.prop("folder", Value::Str(folder.display().to_string())).prop("page", Value::Str(first.clone()));
                first
            }
            (None, None) => return Err(PsError::new(ErrorCategory::InvalidArgument, "GuiNoPage", "give the page with -Html or -Path").terminating()),
        };
        Ok((node, title))
    }
}

impl Cmdlet for ShowGuiWindow {
    fn process(&mut self, ps: &Pipeline<'_>) -> PsResult<()> {
        if let Some(content) = &self.content {
            let args = ShowArgs {
                content,
                title: self.title.clone(),
                width: self.width,
                height: self.height,
                renderer: self.renderer,
                output: self.handler_output,
                on_close: self.on_close.as_ref(),
                on_shortcut: self.on_shortcut.as_ref(),
                on_drop: self.on_drop.as_ref(),
                watch_path: self.watch_path.as_deref(),
                watch_interval: self.watch_interval.as_ref(),
                on_file_change: self.on_file_change.as_ref(),
                no_wait: self.no_wait,
            };
            return window::show(ps, args);
        }
        let (tree, default_title) = self.page_node(ps)?;
        let title = self.title.clone().unwrap_or(default_title);
        let options = WindowOptions {
            title: title.clone(),
            width: self.width.map(|w| w as f32),
            height: self.height.map(|h| h as f32),
            resizable: true,
            always_on_top: false,
            close_on_escape: false,
            dialog: false,
            fit: Fit::WINDOW,
            theme: crate::options::theme(),
            hidden: false,
        };
        let session = session::html_session(ps)?;
        let window = session.open_window(options, tree).map_err(gui_error)?;
        if self.no_wait { open_without_waiting(ps, session, window, title, self.on_message.take()) } else { wait(ps, window, self.on_message.as_ref()) }
    }
}

/// Replies to the page's sends with the handler's output, until the window
/// closes, then writes its result. Ctrl+C closes the window and writes
/// nothing.
fn wait(ps: &Pipeline<'_>, mut window: Window, handler: Option<&PsScriptBlock>) -> PsResult<()> {
    let mut problem = None;
    let result = loop {
        let Some(event) = session::next_event(ps, &mut window)? else {
            return Ok(());
        };
        match event {
            WindowEvent::Message { call, json } => {
                let reply = match handler {
                    Some(h) => {
                        let message = from_json(ps, &json)?;
                        let output = h.call(ps, &[message])?;
                        to_json(ps, output)?
                    }
                    None => String::from("null"),
                };
                window.reply(call, reply).map_err(gui_error)?;
            }
            WindowEvent::PageClosed(json) => break from_json(ps, &json)?,
            WindowEvent::Closed(reason) => match session::closed_error(reason, problem.as_deref()) {
                Some(e) => return Err(e),
                None => break "Closed".into_ps()?,
            },
            WindowEvent::Problem(p) => {
                pwrs::warning!(ps, "the HTML window reported: {p}")?;
                problem = Some(p);
            }
            WindowEvent::Opened | WindowEvent::Event { .. } | WindowEvent::PickerWanted { .. } | WindowEvent::FileChanged { .. } => {}
        }
    };
    let out = pwrs::object::new_psobject("PwrsGui.WindowResult");
    pwrs::object::add_note(&out, "Result", result)?;
    ps.write_object(&out)
}

/// Waits for the window's first frame, then writes the window and passes it
/// to a thread that replies to the page's sends: through an engine event
/// whose action runs -OnMessage, or with null when there is no handler.
/// Ctrl+C during the wait closes the window and writes nothing.
fn open_without_waiting(ps: &Pipeline<'_>, session: Session, mut window: Window, title: String, handler: Option<PsScriptBlock>) -> PsResult<()> {
    let Some(before) = session::first_frame(ps, &mut window)? else {
        return Ok(());
    };
    let id = window.id();
    let source = match &handler {
        Some(_) => {
            let source = format!("PwrsGui.Message.{}.{id}", session.id());
            let action = script(ACTION)?;
            ps.invoke("Register-EngineEvent", &[("SourceIdentifier", source.clone().into_ps()?), ("Action", action.0)])?;
            Some(source)
        }
        None => None,
    };
    let page = PageWindow { id, title, source_identifier: source.clone(), session }.into_ps()?;
    let raised = match (&handler, source) {
        (Some(h), Some(s)) => Some((ps.events()?, s, h.0.clone())),
        _ => None,
    };
    let held = page.clone();
    std::thread::Builder::new()
        .name(format!("pwrs-gui-page-{id}"))
        .spawn(move || answer_sends(window, before, held, raised))
        .map_err(|e| PsError::new(ErrorCategory::ResourceUnavailable, "GuiThread", format!("the HTML window's reader thread cannot start: {e}")).terminating())?;
    ps.write_object(&page)
}

/// Reads a -NoWait window's events until it closes, `before`, those that
/// came before its first frame, first: each send raises the handler's
/// engine event, or gets null as its reply when there is no handler.
fn answer_sends(mut window: Window, before: Vec<WindowEvent>, page: PsObject, raised: Option<(PsEvents, String, PsObject)>) {
    let mut before = before.into_iter();
    loop {
        let next = match before.next() {
            Some(event) => Ok(event),
            None => window.next_event(),
        };
        match next {
            Ok(WindowEvent::Message { call, json }) => match &raised {
                Some((events, source, handler)) => {
                    let data = PageMessage { window: page.clone(), call, json, handler: handler.clone() };
                    if let Err(e) = events.raise(source, data) {
                        eprintln!("PWRS-GUI: the HTML window could not raise {source} for its page's send {call}: {e}");
                    }
                }
                None => {
                    if let Err(e) = window.reply(call, String::from("null")) {
                        eprintln!("PWRS-GUI: the HTML window could not reply to its page's send {call}: {e}");
                    }
                }
            },
            Ok(WindowEvent::Closed(reason)) => {
                if let Some(e) = session::closed_error(reason, None) {
                    eprintln!("PWRS-GUI: the HTML window {}: {}", window.id(), e.message);
                }
                return;
            }
            Ok(WindowEvent::PageClosed(_)) => return,
            Ok(WindowEvent::Problem(p)) => eprintln!("PWRS-GUI: the HTML window reported: {p}"),
            Ok(WindowEvent::Opened | WindowEvent::Event { .. } | WindowEvent::PickerWanted { .. } | WindowEvent::FileChanged { .. }) => {}
            Err(e) => {
                eprintln!("PWRS-GUI: the HTML window stopped reading its events: {e}");
                return;
            }
        }
    }
}

/// Sends a value to the page of an HTML window, as JSON, for the handlers
/// it registered with window.pwrs.on. A window that is not open, or shows
/// widgets rather than a page of its own, is refused.
///
/// # Examples
/// Send-GuiMessage -Window $w.Id -Message @{ Progress = 50 }
/// Get-GuiWindow -Title Report | Send-GuiMessage -Message 'refresh'
#[cmdlet(verb = "Send", noun = "GuiMessage")]
#[derive(Default)]
pub struct SendGuiMessage {
    /// The window's number, as Get-GuiWindow or Show-GuiWindow -NoWait
    /// writes it.
    #[param(mandatory, position = 0, value_from_pipeline_by_property_name, alias = ["Id"])]
    pub window: u64,
    /// The value to send; it crosses as JSON.
    #[param(mandatory, position = 1)]
    pub message: Option<PsObject>,
}

impl Cmdlet for SendGuiMessage {
    fn process(&mut self, ps: &Pipeline<'_>) -> PsResult<()> {
        let message = match &self.message {
            Some(m) => m.clone(),
            None => ().into_ps()?,
        };
        let json = to_json(ps, vec![message])?;
        session::of_window(self.window)?.bridge(self.window, json, session::arm(ps)).map_err(gui_error)
    }
}
