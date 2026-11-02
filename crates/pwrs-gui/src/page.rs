//! Windows of HTML: `Show-GuiWindow -Html` or `-Path`, and
//! `Send-GuiMessage`. A page talks to the session through `window.pwrs`:
//! `send(value)` returns a Promise of the -OnMessage handler's output,
//! `call(name, args)` one of the output of what -Call allows under that
//! name, `on(handler)` takes what Send-GuiMessage sends, `close(value)` ends
//! the window with a result, and `save(value)` stores a value `restore()`
//! gives back to the page loaded again after its engine ended. Values cross
//! as JSON, converted on this side by PowerShell's own ConvertFrom-Json and
//! ConvertTo-Json. `window.open(options)` opens another window of the
//! page's app, whose page talks to the session as the first one's does; the
//! app's command waits for its last window, or with -EndWithFirstWindow for
//! its first.

use crate::inspect::GuiEventKind;
use crate::options::GuiRenderer;
use crate::session::{self, gui_error};
use crate::values::from_value;
use crate::window::{self, GuiHandlerOutput, GuiWidgetEvent, ShowArgs};
use arc_swap::ArcSwap;
use pwrs::prelude::*;
use pwrs_gui_client::proto::{EventKind, Fit, Kind, Node, Value, WindowId, WindowOptions, policy};
use pwrs_gui_client::{Canceler, GuiError, Session, Window, WindowEvent};
use std::collections::{BTreeSet, HashMap};
use std::ops::RangeInclusive;
use std::path::{Path, PathBuf};
use std::sync::{Arc, mpsc};

/// The points a page window's inside may be across and down: -Width's and
/// -Height's range, which the size a page opens a window at keeps to too.
const SIDE: RangeInclusive<f64> = 100.0..=10000.0;

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

/// The PowerShell that runs a page's call of `$Target`, a command or a
/// script block, with the arguments `$Json` gives: an object's properties
/// as named arguments, an array's elements as positional ones, null as
/// none, and any other value as one argument. Its output and its errors
/// are read in the order it writes them. With `$Errors` Any, the first
/// error record, written or thrown, fails the call with the output written
/// before it. With Terminating only a thrown one does, and the others are
/// left in Errors for the window's command to write. `$outcome` is left
/// holding Resolved, Json, the output as [`answer_json`] writes it, the
/// failing error's Message, ErrorId and Category, and Errors.
macro_rules! call_body {
    () => {
        concat!(
            r#"$a = if ($PSVersionTable.PSVersion.Major -ge 7) { ConvertFrom-Json -InputObject $Json -NoEnumerate } else { ConvertFrom-Json -InputObject $Json }
$named = @{}
$listed = @()
if ($a -is [System.Management.Automation.PSCustomObject]) { foreach ($p in $a.psobject.Properties) { $named[$p.Name] = $p.Value } }
elseif ($a -is [array]) { $listed = $a }
elseif ($null -ne $a) { $listed = @(, $a) }
$written = [System.Collections.Generic.List[object]]::new()
$failed = [System.Collections.Generic.List[object]]::new()
$before = -1
$thrown = $null
try {
    & $Target @listed @named 2>&1 | ForEach-Object {
        if ($_ -is [System.Management.Automation.ErrorRecord]) {
            if ($before -lt 0) { $before = $written.Count }
            $failed.Add($_)
        } else {
            $written.Add($_)
        }
    }
} catch {
    $inner = $_.Exception.InnerException
    $thrown = if ($inner -is [System.Management.Automation.IContainsErrorRecord]) { $inner.ErrorRecord } else { $_ }
}
if ($Errors -eq 'Any' -and $failed.Count -gt 0) {
    $e = $failed[0]
    $o = $written.GetRange(0, $before).ToArray()
    $failed.Clear()
} else {
    $e = $thrown
    $o = $written.ToArray()
}
"#,
            answer_json!(),
            r#"$outcome = [pscustomobject]@{
    Resolved = $null -eq $e
    Json = $r
    Message = if ($null -ne $e) { $e.ToString() } else { $null }
    ErrorId = if ($null -ne $e) { $e.FullyQualifiedErrorId } else { $null }
    Category = if ($null -ne $e) { [string] $e.CategoryInfo.Category } else { $null }
    Errors = $failed.ToArray()
}
"#
        )
    };
}

/// A page's call in a window whose command waits, run as [`call_body`]
/// runs one, writing its outcome.
const CALL: &str = concat!("param($Target, [string] $Json, [string] $Errors)\n", call_body!(), "$outcome");

/// What a -NoWait window's page events run as, on the pipeline thread once
/// the engine is idle. A call runs as [`call_body`] runs one: the errors it
/// leaves are written into the action's event job, and its outcome answers
/// the page. A drop runs -OnDrop with its event, the paths dropped its
/// Value, as its argument and as $_. A send's message is converted, the
/// handler run with it and the window that sent it, and the handler's
/// output sent back as the send's reply, as [`answer_json`] writes it.
const ACTION: &str = concat!(
    "$d = $Event.MessageData\n",
    "if ($d.Kind -eq 'Call') {\n",
    "$Target = $d.Handler\n",
    "$Json = $d.Json\n",
    "$Errors = $d.CallErrors\n",
    call_body!(),
    "foreach ($x in $outcome.Errors) { Write-Error -ErrorRecord $x }\n",
    "$d.Window.Answer($d.Call, $outcome)\n",
    "return\n",
    "}\n",
    "if ($d.Kind -eq 'Drop') {\n",
    "$paths = if ($PSVersionTable.PSVersion.Major -ge 7) { ConvertFrom-Json -InputObject $d.Json -NoEnumerate } else { ConvertFrom-Json -InputObject $d.Json }\n",
    "$e = [pscustomobject]@{ PSTypeName = 'PwrsGui.WidgetEvent'; Window = $d.Window.Id; Name = $null; Kind = [PwrsGui.EventKind]::Drop; Value = $paths; State = [pscustomobject]@{ PSTypeName = 'PwrsGui.WindowState' } }\n",
    "$variables = [System.Collections.Generic.List[psvariable]]::new()\n",
    "$variables.Add([psvariable]::new('_', $e))\n",
    "$null = $d.Handler.InvokeWithContext($null, $variables, $e)\n",
    "return\n",
    "}\n",
    "$m = if ($PSVersionTable.PSVersion.Major -ge 7) { ConvertFrom-Json -InputObject $d.Json -NoEnumerate } else { ConvertFrom-Json -InputObject $d.Json }\n",
    "$o = @(& $d.Handler $m $d.Window)\n",
    answer_json!(),
    "$d.Window.Reply($d.Call, $r)"
);

/// What each of -Call's values allows, as one object for each name the
/// page may call: its Name, its Target, a command as Get-Command finds it
/// now or a script block, and the Problem that refuses it, null when there
/// is none. A string names a command, called by its own name, and a
/// hashtable gives the names the page calls and the script block each
/// runs.
const ALLOWED: &str = r#"param($Given)
foreach ($g in @($Given)) {
    $b = if ($null -ne $g) { $g.psobject.BaseObject }
    if ($b -is [string]) {
        if ($b -eq '') {
            [pscustomobject]@{ Name = $b; Target = $null; Problem = 'a command''s name is empty' }
        } elseif ([System.Management.Automation.WildcardPattern]::ContainsWildcardCharacters($b)) {
            [pscustomobject]@{ Name = $b; Target = $null; Problem = "$b holds wildcard characters, and each command allowed is named in full" }
        } elseif ($found = Get-Command -Name $b -ErrorAction Ignore | Select-Object -First 1) {
            [pscustomobject]@{ Name = $b; Target = $found; Problem = $null }
        } else {
            [pscustomobject]@{ Name = $b; Target = $null; Problem = "no command is named $b" }
        }
    } elseif ($b -is [System.Collections.IDictionary]) {
        foreach ($k in $b.Keys) {
            $name = [string] $k
            $v = $b[$k]
            $block = if ($null -ne $v) { $v.psobject.BaseObject }
            if ($name -eq '') {
                [pscustomobject]@{ Name = $name; Target = $null; Problem = 'a name the page calls is empty' }
            } elseif ($block -is [scriptblock]) {
                [pscustomobject]@{ Name = $name; Target = $block; Problem = $null }
            } else {
                $what = if ($null -eq $v) { 'nothing' } else { 'a ' + $v.GetType().FullName }
                [pscustomobject]@{ Name = $name; Target = $null; Problem = "$name is given $what where a script block runs" }
            }
        }
    } else {
        $what = if ($null -eq $g) { 'null' } else { 'a ' + $g.GetType().FullName }
        [pscustomobject]@{ Name = ''; Target = $null; Problem = "$what is neither a command's name nor a hashtable from names to script blocks" }
    }
}
"#;

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

/// Which errors of a page's call reject the Promise `window.pwrs.call`
/// returned.
#[psenum(name = "PwrsGui.CallErrors")]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum GuiCallErrors {
    /// Every error record: the first, written or thrown, rejects the call,
    /// with the output written before it.
    #[default]
    Any,
    /// Only an error that ends the call. The others are written as the
    /// window's command's own errors, and the call resolves with its
    /// output.
    Terminating,
}

impl GuiCallErrors {
    fn name(self) -> &'static str {
        match self {
            GuiCallErrors::Any => "Any",
            GuiCallErrors::Terminating => "Terminating",
        }
    }
}

/// What a page's `window.pwrs.call` may run, by the name the page calls it
/// by, matched without regard to case: a command, as Get-Command found it
/// when the window opened, or a script block; and which errors reject a
/// call.
#[derive(Clone, Default)]
struct Calls {
    allowed: Vec<(String, PsObject)>,
    errors: GuiCallErrors,
}

impl Calls {
    /// What the page's call of `name` runs.
    fn find(&self, name: &str) -> Option<&PsObject> {
        let wanted = name.to_lowercase();
        self.allowed.iter().find(|(n, _)| n.to_lowercase() == wanted).map(|(_, target)| target)
    }

    /// The refusal, as JSON, of the page's call of `name`, under which the
    /// window allows nothing: its message names the call.
    fn refused(&self, name: &str) -> String {
        let why = if self.allowed.is_empty() { format!("this window allows nothing to be called, so {name} does not run") } else { format!("this window allows no call named {name}") };
        serde_json::json!({ "message": why }).to_string()
    }
}

/// How a call's outcome, as [`call_body`] leaves it, answers the page: its
/// output as JSON, or a rejection, an object whose message, errorId and
/// category describe the error and whose output is the output written
/// before it.
fn call_answer(outcome: &PsObject) -> PsResult<Result<String, String>> {
    let output = String::from_ps(&pwrs::object::property(outcome, "Json")?)?;
    if bool::from_ps(&pwrs::object::property(outcome, "Resolved")?)? {
        return Ok(Ok(output));
    }
    let text = |key: &str| String::from_ps(&pwrs::object::property(outcome, key)?);
    let before: serde_json::Value = serde_json::from_str(&output).map_err(|e| PsError::new(ErrorCategory::InvalidResult, "GuiJson", format!("a call's output is not JSON ({e}): {output}")).terminating())?;
    Ok(Err(serde_json::json!({ "message": text("Message")?, "errorId": text("ErrorId")?, "category": text("Category")?, "output": before }).to_string()))
}

/// Runs a page's call of `target` with the arguments `json` gives, in a
/// window whose command waits, as [`call_body`] runs one: the errors it
/// leaves become the command's own, and the call's answer is returned.
fn run_call(ps: &Pipeline<'_>, target: &PsObject, json: &str, errors: GuiCallErrors) -> PsResult<Result<String, String>> {
    let ran = script(CALL)?.call(ps, &[target.clone(), json.into_ps()?, errors.name().into_ps()?])?;
    let Some(outcome) = ran.first() else {
        return Err(PsError::new(ErrorCategory::InvalidResult, "GuiJson", "a call ran and gave no outcome").terminating());
    };
    for record in Vec::<PsObject>::from_ps(&pwrs::object::property(outcome, "Errors")?)? {
        ps.invoke("Write-Error", &[("ErrorRecord", record)])?;
    }
    call_answer(outcome)
}

/// A page window's handlers: -OnMessage for its sends, and -OnDrop for the
/// files dropped where the page takes none.
struct PageHandlers {
    on_message: Option<PsScriptBlock>,
    on_drop: Option<PsScriptBlock>,
}

/// Runs a page window's handler with the event as its argument and as $_,
/// and writes one object, Errors, the error records it raised and the one
/// that ended it; what it writes is dropped. The handler runs through
/// ScriptBlock.InvokeWithContext, whose errors would reach the host rather
/// than the command.
const PAGE_HANDLER: &str = r#"param($Handler, $PageEvent)
$variables = [System.Collections.Generic.List[psvariable]]::new()
$variables.Add([psvariable]::new('_', $PageEvent))
$failed = [System.Collections.Generic.List[object]]::new()
try {
    & { $Handler.InvokeWithContext($null, $variables, $PageEvent) } 2>&1 | ForEach-Object {
        if ($_ -is [System.Management.Automation.ErrorRecord]) { $failed.Add($_) }
    }
} catch {
    $inner = $_.Exception.InnerException
    if ($inner -is [System.Management.Automation.IContainsErrorRecord]) { $failed.Add($inner.ErrorRecord) } else { $failed.Add($_) }
}
[pscustomobject]@{ Errors = $failed.ToArray() }
"#;

/// Runs `handler`, a page window's -OnDrop, with `event`, as
/// [`PAGE_HANDLER`] runs one: its errors become the command's own, and the
/// window goes on.
fn run_page_handler(ps: &Pipeline<'_>, handler: &PsScriptBlock, event: PsObject) -> PsResult<()> {
    let ran = script(PAGE_HANDLER)?.call(ps, &[handler.0.clone(), event])?;
    let Some(outcome) = ran.first() else {
        return Err(PsError::new(ErrorCategory::InvalidResult, "GuiBadState", "a page window's handler ran and gave no outcome").terminating());
    };
    for record in Vec::<PsObject>::from_ps(&pwrs::object::property(outcome, "Errors")?)? {
        ps.invoke("Write-Error", &[("ErrorRecord", record)])?;
    }
    Ok(())
}

/// The event -OnDrop is given for files dropped on page window `window`:
/// its Kind Drop and its Value the paths `value` lists.
fn drop_event(window: u64, value: &Value) -> PsResult<PsObject> {
    let state = pwrs::object::new_psobject("PwrsGui.WindowState");
    GuiWidgetEvent { window, name: None, kind: GuiEventKind::Drop, value: from_value(value)?, state }.into_ps()
}

/// The paths a drop's `value` lists, as JSON, for the action a -NoWait
/// window's -OnDrop runs as.
fn drop_json(value: &Value) -> String {
    let paths: Vec<&str> = match value {
        Value::List(items) => items.iter().filter_map(Value::as_str).collect(),
        _not_a_list => Vec::new(),
    };
    serde_json::json!(paths).to_string()
}

/// A window of HTML that `Show-GuiWindow -NoWait` opened, or a window of a
/// page app as -OnMessage is given the one that sent.
#[psclass(name = "PwrsGui.PageWindow", mode = proxy)]
pub struct PageWindow {
    /// The window's number, which Send-GuiMessage takes as -Window.
    pub id: u64,
    /// The window's title as it opened.
    pub title: String,
    /// The engine event each send raises for -OnMessage's action, and each
    /// call of what -Call allows for the call's; null without either.
    /// Unregister-Event removes the action.
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

    /// Answers the page's `window.pwrs.call` numbered `call` with the
    /// outcome of running it; the action a call runs as calls it.
    pub fn answer(&self, call: u32, outcome: PsObject) -> PsResult<()> {
        let answer = call_answer(&outcome)?;
        self.session.answer_call(self.id, call, answer).map_err(gui_error)
    }
}

/// What a page's engine event gives the action it runs: a send's, for
/// -OnMessage, or a call's, for what -Call allows under its name.
#[psclass(name = "PwrsGui.PageMessage")]
#[derive(Clone, Default)]
pub struct PageMessage {
    /// What the page did: Message for a send, Call for a call, Drop for
    /// files dropped that it left to -OnDrop.
    pub kind: String,
    /// The window the message came from.
    pub window: PsObject,
    /// The page's number for its send or call, which the answer is for; 0
    /// for a drop.
    pub call: u32,
    /// The value the page sent, the call's arguments, or the paths dropped,
    /// as JSON.
    pub json: String,
    /// The -OnMessage script block, what the call runs, a command or a
    /// script block, or the -OnDrop script block.
    pub handler: PsObject,
    /// Which errors reject a call: Any or Terminating, as -CallErrors says.
    pub call_errors: String,
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
/// developer tools only with -DevTools. Its files carry a content-security
/// policy that lets it reach only its own origin: -Connect adds the origins
/// it may fetch from and open sockets to, and -ContentSecurityPolicy gives
/// a policy of the caller's own in its place.
///
/// The page gets window.pwrs: send(value) gives -OnMessage the value and
/// the window that sent it, a PwrsGui.PageWindow, and returns a Promise of
/// what the handler returns; call(name, args) runs
/// what -Call allows under that name, on this thread, and returns a Promise
/// of its output, rejected with an Error when it fails; ondrop(handler) is
/// given the full paths of files dropped on the page, which -OnDrop is
/// given too where the page does not take the drop with preventDefault;
/// on(handler) takes what Send-GuiMessage sends; channel(name) opens a
/// channel to its backend that -Channel gives, as New-GuiChannel describes;
/// close(value) closes the window; save(value) stores
/// a value that restore() returns a Promise of, so a page loaded again
/// after its engine ended picks up from it. Values cross as JSON. A page
/// whose engine ends is loaded again; one that ends again before it draws
/// closes its window, and the command fails saying why.
///
/// window.pwrs.window.open(options) opens another window of the page's
/// app: a file of a -Path app's folder by its path, HTML text, or with
/// -AllowNavigation and -TrustNavigation an address. The options give its
/// title, size and frame, and the rest comes from the command's window.
/// Each later window's page talks to the session as the first one's does.
/// The command waits until the app's last window closes, or with
/// -EndWithFirstWindow until its own does, which closes the rest. It writes
/// an object whose Result is the value its own window's page gave close,
/// or 'Closed' when the person or the session closed it. With -NoWait it
/// returns its window once it has opened, with its first frame drawn;
/// -OnMessage runs as an engine-event action when the engine is idle, and
/// Get-GuiWindow lists the app's later windows.
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
    #[param(mandatory, position = 0, set = ["Html", "HtmlConnect", "HtmlPolicy"])]
    pub html: Option<String>,
    /// A page's file, or a folder whose index.html is the page.
    #[param(mandatory, set = ["Path", "PathConnect", "PathPolicy"])]
    pub path: Option<String>,
    /// The window's title. Without it, a page's window takes its document's
    /// title whenever the page sets one.
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
    /// Runs for each value a page of the app sends with window.pwrs.send,
    /// with the value and the window that sent it, a PwrsGui.PageWindow, as
    /// its arguments; what it outputs is the send's reply.
    #[param(set = ["Html", "HtmlConnect", "HtmlPolicy", "Path", "PathConnect", "PathPolicy"])]
    pub on_message: Option<PsScriptBlock>,
    /// Lets the page leave its own origin and load remote content.
    #[param(set = ["Html", "HtmlConnect", "HtmlPolicy", "Path", "PathConnect", "PathPolicy"])]
    pub allow_navigation: bool,
    /// Lets the page open the webview's developer tools.
    #[param(set = ["Html", "HtmlConnect", "HtmlPolicy", "Path", "PathConnect", "PathPolicy"])]
    pub dev_tools: bool,
    /// Origins the page may fetch from and open sockets and event streams
    /// to, beside its own, written as a content-security policy's
    /// connect-src writes them: https://api.example.com,
    /// ws://localhost:8080, https://*.example.com, or a scheme alone, as
    /// wss:. The rest of the page's policy stays as it is. It cannot be
    /// given with -ContentSecurityPolicy, which writes a connect-src of its
    /// own.
    #[param(mandatory, set = ["HtmlConnect", "PathConnect"])]
    pub connect: Option<Vec<String>>,
    /// The content-security policy the page's own files carry, whole, in
    /// place of the one they carry by default. With -AllowNavigation they
    /// still carry it.
    #[param(mandatory, set = ["HtmlPolicy", "PathPolicy"])]
    pub content_security_policy: Option<String>,
    /// Opens the window without the system's title bar and border, so the
    /// page draws its own: window.pwrs.window minimizes, maximizes,
    /// restores and closes it, and its startDrag and startResize move and
    /// resize it while the pointer's button is held. On macOS the window's
    /// three buttons stay over the page's top left unless
    /// -HideWindowButtons.
    #[param(set = ["Html", "HtmlConnect", "HtmlPolicy", "Path", "PathConnect", "PathPolicy"])]
    pub frameless: bool,
    /// With -Frameless on macOS, hides the window's close, minimize and
    /// zoom buttons too, so the page draws all three as it does elsewhere.
    #[param(set = ["Html", "HtmlConnect", "HtmlPolicy", "Path", "PathConnect", "PathPolicy"])]
    pub hide_window_buttons: bool,
    /// What the page's window.pwrs.call may run, on this thread: names of
    /// commands, each called by its own name, and hashtables from a name
    /// the page calls to the script block it runs. An object the page
    /// passes is splatted as named arguments, an array as positional ones,
    /// and any other value is one argument.
    #[param(set = ["Html", "HtmlConnect", "HtmlPolicy", "Path", "PathConnect", "PathPolicy"])]
    pub call: Option<Vec<PsObject>>,
    /// Which errors of a call reject its Promise: Any, every error record,
    /// the first rejecting it with the output written before it; or
    /// Terminating, only one that ends the call, the others written as this
    /// command's own errors. Any when not given.
    #[param(set = ["Html", "HtmlConnect", "HtmlPolicy", "Path", "PathConnect", "PathPolicy"])]
    pub call_errors: GuiCallErrors,
    /// Lets the origins -AllowNavigation admits call, move the page's
    /// window and be given the paths of files dropped on the page, as the
    /// page's own origin does.
    #[param(set = ["Html", "HtmlConnect", "HtmlPolicy", "Path", "PathConnect", "PathPolicy"])]
    pub trust_navigation: bool,
    /// Ends the app when the window the command opened closes, closing the
    /// windows its pages opened; without it the command waits for the app's
    /// last window. Either way the result is that first window's.
    #[param(set = ["Html", "HtmlConnect", "HtmlPolicy", "Path", "PathConnect", "PathPolicy"])]
    pub end_with_first_window: bool,
    /// The page's channels to its backend, which it opens with
    /// window.pwrs.channel(name): PwrsGui.Channel objects New-GuiChannel
    /// wrote, or hashtables from a channel's name to a hashtable of its
    /// ToPage and FromPage SubEtha.Ring objects, with Text, Stream,
    /// MaxFrames, MaxBytes and Process as New-GuiChannel takes them.
    #[param(set = ["Html", "HtmlConnect", "HtmlPolicy", "Path", "PathConnect", "PathPolicy"])]
    pub channel: Option<Vec<PsObject>>,
    /// The renderer that draws a window of widgets, Egui or Html; the one
    /// Set-GuiOption -Renderer names when not given. A window requested in
    /// HTML draws with egui when no webview engine can start, and the
    /// Verbose stream says why.
    #[param(set = "Content")]
    pub renderer: Option<GuiRenderer>,
    /// CSS for the window's widgets, applied after the HTML renderer's own
    /// styles, so its rules restyle them, and after -StylePath's. A window
    /// egui draws keeps egui's look.
    #[param(set = "Content")]
    pub style: Option<String>,
    /// A .css file for the window's widgets, read as UTF-8, or UTF-16 by
    /// its byte order mark, and applied as -Style is, before -Style's CSS.
    #[param(set = "Content")]
    pub style_path: Option<String>,
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
    /// under the pointer takes them, with the event as its argument and as
    /// $_; its Value is the paths dropped, or the text. On a page of your
    /// own it runs for files the page's handlers leave, not taking the drop
    /// with preventDefault.
    #[param]
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
        if self.hide_window_buttons && !self.frameless {
            let text = "-HideWindowButtons hides the buttons of a window without the system's frame; give -Frameless too";
            return Err(PsError::new(ErrorCategory::InvalidArgument, "GuiBadFrame", text).terminating());
        }
        if self.trust_navigation && !self.allow_navigation {
            let text = "-TrustNavigation trusts the origins -AllowNavigation admits, and without it the page reaches none; give -AllowNavigation too";
            return Err(PsError::new(ErrorCategory::InvalidArgument, "GuiBadNavigation", text).terminating());
        }
        let mut node = Node::new(1, Kind::Html)
            .prop("allow_navigation", Value::Bool(self.allow_navigation))
            .prop("trust_navigation", Value::Bool(self.trust_navigation))
            .prop("dev_tools", Value::Bool(self.dev_tools))
            .prop("frameless", Value::Bool(self.frameless))
            .prop("hide_window_buttons", Value::Bool(self.hide_window_buttons))
            .prop("drop", Value::Bool(self.on_drop.is_some()))
            .prop("follow_title", Value::Bool(self.title.is_none()));
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
        if let Some(policy) = self.policy()? {
            node = node.prop("policy", Value::Str(policy));
        }
        Ok((node, title))
    }

    /// What the page may call, from -Call, and which errors reject a call,
    /// from -CallErrors: nothing when -Call is not given. A value that names
    /// no command, or gives no script block, and a name given twice, are
    /// refused.
    fn calls(&self, ps: &Pipeline<'_>) -> PsResult<Calls> {
        let Some(given) = &self.call else {
            return Ok(Calls::default());
        };
        let refuse = |text: String| PsError::new(ErrorCategory::InvalidArgument, "GuiBadCall", text).terminating();
        let found = script(ALLOWED)?.call(ps, &[given.clone().into_ps()?])?;
        let mut allowed: Vec<(String, PsObject)> = Vec::with_capacity(found.len());
        for entry in &found {
            let name = String::from_ps(&pwrs::object::property(entry, "Name")?)?;
            if let Some(problem) = Option::<String>::from_ps(&pwrs::object::property(entry, "Problem")?)? {
                return Err(refuse(format!("-Call: {problem}")));
            }
            let lower = name.to_lowercase();
            if allowed.iter().any(|(n, _)| n.to_lowercase() == lower) {
                return Err(refuse(format!("-Call names {name} twice; a name the page calls runs one thing")));
            }
            allowed.push((name, pwrs::object::property(entry, "Target")?));
        }
        Ok(Calls { allowed, errors: self.call_errors })
    }

    /// A window of widgets' own CSS: -StylePath's file, then -Style's text;
    /// none when neither is given.
    fn stylesheet(&self, ps: &Pipeline<'_>) -> PsResult<Option<String>> {
        let file = match &self.style_path {
            Some(path) => match crate::widgets::document_file(ps, path) {
                Ok((text, _full)) => Some(text),
                Err(why) => return Err(PsError::new(ErrorCategory::ObjectNotFound, "GuiNoFile", format!("-StylePath: {why}")).terminating()),
            },
            None => None,
        };
        Ok(match (file, &self.style) {
            (None, None) => None,
            (Some(file), None) => Some(file),
            (None, Some(text)) => Some(text.clone()),
            (Some(file), Some(text)) => Some(format!("{file}\n{text}")),
        })
    }

    /// The content-security policy -Connect or -ContentSecurityPolicy gives
    /// the page's own files; none when neither is given, so they carry the
    /// default.
    fn policy(&self) -> PsResult<Option<String>> {
        let refuse = |text: String| PsError::new(ErrorCategory::InvalidArgument, "GuiBadPolicy", text).terminating();
        match (&self.connect, &self.content_security_policy) {
            (None, None) => Ok(None),
            (Some(sources), None) => policy::with_connect(sources).map(Some).map_err(|why| refuse(format!("-Connect: {why}"))),
            (None, Some(whole)) => policy::whole_policy(whole).map(|()| Some(whole.clone())).map_err(|why| refuse(format!("-ContentSecurityPolicy: {why}"))),
            (Some(_), Some(_)) => Err(refuse(String::from("give -Connect or -ContentSecurityPolicy, not both: a policy given whole writes its own connect-src"))),
        }
    }
}

impl Cmdlet for ShowGuiWindow {
    fn process(&mut self, ps: &Pipeline<'_>) -> PsResult<()> {
        if let Some(content) = &self.content {
            let css = self.stylesheet(ps)?;
            let args = ShowArgs {
                content,
                title: self.title.clone(),
                width: self.width,
                height: self.height,
                renderer: self.renderer,
                style: css.as_deref(),
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
        let channels = match &self.channel {
            Some(values) => crate::channel::given(values)?,
            None => Vec::new(),
        };
        let tree = if channels.is_empty() { tree } else { tree.prop("channels", crate::channel::property(&channels)) };
        let calls = self.calls(ps)?;
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
        };
        let session = session::html_session(ps)?;
        let window = session.open_window(options, tree.clone()).map_err(gui_error)?;
        let app = App {
            first: window.id(),
            node: tree,
            end_with_first: self.end_with_first_window,
            windows: Arc::new(ArcSwap::from_pointee(BTreeSet::from([window.id()]))),
            folders: channels.iter().filter_map(|c| c.folder.clone()).collect(),
        };
        crate::channel::watch_processes(&session, &channels, &app.windows)?;
        let handlers = PageHandlers { on_message: self.on_message.take(), on_drop: self.on_drop.take() };
        if self.no_wait { open_without_waiting(ps, window, title, app, handlers, calls) } else { wait(ps, window, title, &app, &handlers, &calls) }
    }
}

/// A page app: the number of the window its command opened, which every
/// later window's node names as its `app`; that window's node, whose
/// settings later windows take, its policy, its origins, its developer
/// tools, its frame, its drops, its channels and a -Path app's folder among
/// them; whether that window's close ends the app; the app's windows open
/// now, whose channels a channel's process ends as it exits; and the
/// folders of the channels New-GuiChannel made for it, which go once the
/// app has ended.
#[derive(Clone)]
struct App {
    first: WindowId,
    node: Node,
    end_with_first: bool,
    windows: Arc<ArcSwap<BTreeSet<WindowId>>>,
    folders: Vec<PathBuf>,
}

impl App {
    /// Window `id` of the app has opened.
    fn opened(&self, id: WindowId) {
        self.windows.rcu(|open| {
            let mut open = (**open).clone();
            open.insert(id);
            open
        });
    }

    /// Window `id` of the app has closed; true when it was the app's last,
    /// when the folders of its channels go.
    fn closed(&self, id: WindowId) -> bool {
        let mut last = false;
        self.windows.rcu(|open| {
            let mut open = (**open).clone();
            open.remove(&id);
            last = open.is_empty();
            open
        });
        if last {
            for folder in &self.folders {
                crate::channel::remove_folder(folder);
            }
        }
        last
    }

    /// The window a page of the app asked for with the options `json`: its
    /// node, its options and its title; or why it does not open, for the
    /// page. A path is a file of a -Path app's folder, and HTML text opened
    /// in such an app has the folder's files beside it. An address opens only
    /// with -AllowNavigation and -TrustNavigation. What the options leave
    /// out comes from the first window: its frame, and with it whether
    /// macOS's buttons are hidden. A window opens at a page window's size,
    /// and without a title takes its document's.
    fn later(&self, json: &str) -> Result<(Node, WindowOptions, String), String> {
        let opening = Opening::of(json)?;
        let flag = |key: &str| matches!(self.node.props.get(key), Some(Value::Bool(true)));
        let folder = match self.node.props.get("folder") {
            Some(Value::Str(f)) => Some(PathBuf::from(f)),
            Some(_) | None => None,
        };
        let mut node = self.node.clone();
        for key in ["html", "page", "url"] {
            node.props.remove(key);
        }
        let named = match &opening.page {
            OpenedPage::Path(path) => {
                let Some(folder) = &folder else {
                    return Err(format!("{path} is a path, and this app's page is HTML text, with no folder to find it in"));
                };
                let file = app_file(folder, path)?;
                let name = match file.rsplit('/').next() {
                    Some(last) => last.to_string(),
                    None => file.clone(),
                };
                node = node.prop("page", Value::Str(file));
                name
            }
            OpenedPage::Html(html) => {
                node = node.prop("html", Value::Str(html.clone()));
                String::from("PWRS-GUI")
            }
            OpenedPage::Url(url) => {
                if !(flag("allow_navigation") && flag("trust_navigation")) {
                    return Err(format!("{url} is at another origin, which an app's window opens only with -AllowNavigation and -TrustNavigation"));
                }
                let lower = url.to_ascii_lowercase();
                if !(lower.starts_with("https://") || lower.starts_with("http://")) {
                    return Err(format!("{url} is no http or https address"));
                }
                node.props.remove("folder");
                node = node.prop("url", Value::Str(url.clone()));
                String::from("PWRS-GUI")
            }
        };
        let frameless = match opening.frameless {
            Some(given) => given,
            None => flag("frameless"),
        };
        let app = i64::try_from(self.first).map_err(|e| format!("window {} cannot name an app: {e}", self.first))?;
        let node = node
            .prop("frameless", Value::Bool(frameless))
            .prop("hide_window_buttons", Value::Bool(frameless && flag("hide_window_buttons")))
            .prop("follow_title", Value::Bool(opening.title.is_none()))
            .prop("app", Value::Int(app));
        let title = match opening.title {
            Some(given) => given,
            None => named,
        };
        let options = WindowOptions {
            title: title.clone(),
            width: opening.width.map(|w| w as f32),
            height: opening.height.map(|h| h as f32),
            resizable: true,
            always_on_top: false,
            close_on_escape: false,
            dialog: false,
            fit: Fit::WINDOW,
            theme: crate::options::theme(),
        };
        Ok((node, options, title))
    }
}

/// What a page's `window.pwrs.window.open(options)` asked for: the page,
/// and the window's title, size and frame where it gave them.
#[derive(Clone, Debug, PartialEq)]
struct Opening {
    page: OpenedPage,
    title: Option<String>,
    width: Option<f64>,
    height: Option<f64>,
    frameless: Option<bool>,
}

/// The page a later window of an app shows.
#[derive(Clone, Debug, PartialEq)]
enum OpenedPage {
    /// A file of the app's folder, by its path from the folder.
    Path(String),
    /// HTML text.
    Html(String),
    /// An address at another origin.
    Url(String),
}

/// The options `window.pwrs.window.open` takes.
const OPEN_KEYS: [&str; 7] = ["path", "html", "url", "title", "width", "height", "frameless"];

impl Opening {
    /// What the options a page gave `open`, as JSON, ask for. An option it
    /// does not take, a value of the wrong kind, a size outside [`SIDE`],
    /// and options that name no page, or more than one, are refused.
    fn of(json: &str) -> Result<Opening, String> {
        let given: serde_json::Value = serde_json::from_str(json).map_err(|e| format!("open's options are no JSON ({e}): {json}"))?;
        let Some(options) = given.as_object() else {
            return Err(format!("open takes an object of path, html or url, with title, width, height and frameless, not {given}"));
        };
        if let Some(other) = options.keys().find(|k| !OPEN_KEYS.contains(&k.as_str())) {
            return Err(format!("open takes path, html, url, title, width, height and frameless, not {other}"));
        }
        let text = |key: &str| match options.get(key) {
            None | Some(serde_json::Value::Null) => Ok(None),
            Some(serde_json::Value::String(s)) => Ok(Some(s.clone())),
            Some(other) => Err(format!("open's {key} is text, not {other}")),
        };
        let side = |key: &str| match options.get(key) {
            None | Some(serde_json::Value::Null) => Ok(None),
            Some(v) => match v.as_f64() {
                Some(n) if SIDE.contains(&n) => Ok(Some(n)),
                Some(n) => Err(format!("open's {key} is from {} to {} points, not {n}", SIDE.start(), SIDE.end())),
                None => Err(format!("open's {key} is a number of points, not {v}")),
            },
        };
        let frameless = match options.get("frameless") {
            None | Some(serde_json::Value::Null) => None,
            Some(serde_json::Value::Bool(b)) => Some(*b),
            Some(other) => return Err(format!("open's frameless is true or false, not {other}")),
        };
        let page = match (text("path")?, text("html")?, text("url")?) {
            (Some(path), None, None) => OpenedPage::Path(path),
            (None, Some(html), None) => OpenedPage::Html(html),
            (None, None, Some(url)) => OpenedPage::Url(url),
            (None, None, None) => return Err(String::from("open names no page: give it path, html or url")),
            (Some(_), Some(_), _) | (Some(_), _, Some(_)) | (_, Some(_), Some(_)) => return Err(String::from("open takes one page, path, html or url, and was given two")),
        };
        Ok(Opening { page, title: text("title")?, width: side("width")?, height: side("height")?, frameless })
    }
}

/// The path from `folder` of the file of it a page named as `path`, its
/// parts joined with `/`. A rooted path, one naming a drive, one leaving the
/// folder and one naming no file in it are refused.
fn app_file(folder: &Path, path: &str) -> Result<String, String> {
    let written = path.replace('\\', "/");
    if written.starts_with('/') || written.contains(':') {
        return Err(format!("{path} is no path from the app's folder"));
    }
    let mut parts = Vec::new();
    for part in written.split('/') {
        match part {
            "" | "." => {}
            ".." => return Err(format!("{path} leaves the app's folder")),
            p => parts.push(p),
        }
    }
    if parts.is_empty() {
        return Err(format!("{path} names no file of the app's folder"));
    }
    let full = parts.iter().fold(folder.to_path_buf(), |f, p| f.join(p));
    if !full.is_file() {
        return Err(format!("{path} is no file of the app's folder {}", folder.display()));
    }
    Ok(parts.join("/"))
}

/// What a page's `open` resolves with: the window's number, as text, since
/// a JavaScript number cannot hold every window's number exactly, and its
/// title.
fn opened_json(id: WindowId, title: &str) -> String {
    serde_json::json!({ "id": id.to_string(), "title": title }).to_string()
}

/// What a page app's window readers send its command's thread: the window,
/// and its next event or the error that ended its reading.
type Read = (WindowId, Result<WindowEvent, GuiError>);

/// The readers of a page app's windows while its command waits: a thread
/// for each window, which sends the window's events to the command's
/// thread, and each window's canceler. Ctrl+C ends every reader, and so
/// does the end of the command; a reader ended so closes its window as it
/// goes, as the session's close, never held by a page's onclose handlers.
struct Readers {
    sent: mpsc::Sender<Read>,
    cancelers: Arc<ArcSwap<Vec<Canceler>>>,
    threads: Vec<std::thread::JoinHandle<()>>,
}

impl Readers {
    fn new(sent: mpsc::Sender<Read>) -> Readers {
        Readers { sent, cancelers: Arc::new(ArcSwap::from_pointee(Vec::new())), threads: Vec::new() }
    }

    /// Starts reading `window`'s events.
    fn start(&mut self, window: Window) -> PsResult<()> {
        let canceler = window.canceler();
        self.cancelers.rcu(|held| {
            let mut held = (**held).clone();
            held.push(canceler.clone());
            held
        });
        let (id, sent) = (window.id(), self.sent.clone());
        let thread = std::thread::Builder::new()
            .name(format!("pwrs-gui-page-{id}"))
            .spawn(move || read_window(window, &sent))
            .map_err(|e| PsError::new(ErrorCategory::ResourceUnavailable, "GuiThread", format!("the reader of window {id} cannot start: {e}")).terminating())?;
        self.threads.push(thread);
        Ok(())
    }

    /// What Ctrl+C runs: it ends every reader's wait.
    fn stopper(&self) -> impl FnOnce() + Send + 'static {
        let cancelers = Arc::clone(&self.cancelers);
        move || cancel_every(&cancelers)
    }
}

impl Drop for Readers {
    /// Ends every reader still reading, which closes its window, and waits
    /// for each to end.
    fn drop(&mut self) {
        cancel_every(&self.cancelers);
        for thread in self.threads.drain(..) {
            if let Err(panic) = thread.join() {
                let why = match (panic.downcast_ref::<&str>(), panic.downcast_ref::<String>()) {
                    (Some(text), _) => text.to_string(),
                    (None, Some(text)) => text.clone(),
                    (None, None) => String::from("a panic with no message"),
                };
                eprintln!("PWRS-GUI: the reader of a page app's window panicked: {why}");
            }
        }
    }
}

/// A waiting app's end: once its command stops waiting, with windows of
/// the app still open, as at Ctrl+C or with -EndWithFirstWindow, the
/// folders of its channels go, as they do when its last window closes.
struct Ending<'a>(&'a App);

impl Drop for Ending<'_> {
    fn drop(&mut self) {
        if self.0.windows.load().is_empty() {
            return;
        }
        for folder in &self.0.folders {
            crate::channel::remove_folder(folder);
        }
    }
}

fn cancel_every(cancelers: &ArcSwap<Vec<Canceler>>) {
    for c in cancelers.load().iter() {
        c.cancel();
    }
}

/// Sends `window`'s events to `sent` until its last, or until its reader is
/// ended, with the error that ended it; a window whose reader is ended
/// closes as it is dropped.
fn read_window(mut window: Window, sent: &mpsc::Sender<Read>) {
    let id = window.id();
    loop {
        let next = window.next_event();
        let last = match &next {
            Ok(event) => matches!(event, WindowEvent::Closed(_) | WindowEvent::PageClosed(_)),
            Err(ended) => {
                trace_ended(id, ended);
                true
            }
        };
        let read: Read = (id, next);
        match sent.send(read) {
            Ok(()) if last => return,
            Ok(()) => {}
            Err(unsent) => {
                eprintln!("PWRS-GUI: window {id}'s reading went on after its command stopped reading: {:?}", unsent.0.1);
                return;
            }
        }
    }
}

/// Traces why the reading of window `id` ended.
fn trace_ended(id: WindowId, ended: &GuiError) {
    pwrs_gui_client::proto::trace::line(|| format!("module: window {id}'s reading ended: {ended}"));
}

/// Window `id` of a waiting app as -OnMessage's second argument, made the
/// first time it sends.
fn page_window(pages: &mut HashMap<WindowId, PsObject>, session: &Session, id: WindowId, titles: &HashMap<WindowId, String>) -> PsResult<PsObject> {
    if let Some(page) = pages.get(&id) {
        return Ok(page.clone());
    }
    let title = match titles.get(&id) {
        Some(t) => t.clone(),
        None => String::new(),
    };
    let page = PageWindow { id, title, source_identifier: None, session: session.clone() }.into_ps()?;
    pages.insert(id, page.clone());
    Ok(page)
}

/// Runs a page app until its last window closes, or with
/// -EndWithFirstWindow until `first`, the window its command opened,
/// closes, which closes the rest; then writes the result: the value
/// `first`'s page gave close, or Closed. On this thread, one at a time,
/// each window's sends get -OnMessage's output, with the window as the
/// handler's second argument; its calls run what `calls` allows; its opens
/// open the app's later windows; and -OnDrop runs for the files dropped
/// where its page takes none. A later window whose page cannot be shown is
/// an error the command writes, and the app goes on. Ctrl+C closes every
/// window of the app and writes nothing.
fn wait(ps: &Pipeline<'_>, first: Window, title: String, app: &App, handlers: &PageHandlers, calls: &Calls) -> PsResult<()> {
    let session = first.session().clone();
    let ending = Ending(app);
    let (sent, events) = mpsc::channel::<Read>();
    let mut readers = Readers::new(sent);
    let mut open = BTreeSet::from([first.id()]);
    let mut titles = HashMap::from([(first.id(), title)]);
    let mut pages = HashMap::new();
    let mut problems: HashMap<WindowId, String> = HashMap::new();
    let mut result = None;
    readers.start(first)?;
    let stop = ps.on_stop(readers.stopper())?;
    while !open.is_empty() && !(app.end_with_first && result.is_some()) {
        let (id, next) = match events.recv() {
            Ok(read) => read,
            Err(gone) => return Err(PsError::new(ErrorCategory::InvalidOperation, "GuiBadState", format!("every reader of the app's windows ended: {gone}")).terminating()),
        };
        let event = match next {
            Ok(event) => event,
            Err(GuiError::Canceled) => return Ok(()),
            Err(failed) => return Err(gui_error(failed)),
        };
        match event {
            WindowEvent::Message { call, json } => {
                let reply = match &handlers.on_message {
                    Some(h) => {
                        let message = from_json(ps, &json)?;
                        let sender = page_window(&mut pages, &session, id, &titles)?;
                        let output = h.call(ps, &[message, sender])?;
                        to_json(ps, output)?
                    }
                    None => String::from("null"),
                };
                session.reply(id, call, reply).map_err(gui_error)?;
            }
            WindowEvent::Call { call, name, json } => {
                let answer = match calls.find(&name) {
                    Some(target) => run_call(ps, target, &json, calls.errors)?,
                    None => Err(calls.refused(&name)),
                };
                session.answer_call(id, call, answer).map_err(gui_error)?;
            }
            WindowEvent::Open { call, json } => {
                let refused = |why: String| serde_json::json!({ "message": why }).to_string();
                let answer = match app.later(&json) {
                    Ok((node, options, later_title)) => match session.open_window(options, node) {
                        Ok(window) => {
                            let later = window.id();
                            readers.start(window)?;
                            open.insert(later);
                            app.opened(later);
                            let answer = opened_json(later, &later_title);
                            titles.insert(later, later_title);
                            Ok(answer)
                        }
                        Err(not_opened) => Err(refused(format!("the window did not open: {not_opened}"))),
                    },
                    Err(why) => Err(refused(why)),
                };
                session.answer_call(id, call, answer).map_err(gui_error)?;
            }
            WindowEvent::Event { kind: EventKind::Drop, value, .. } => {
                if let Some(h) = &handlers.on_drop {
                    run_page_handler(ps, h, drop_event(id, &value)?)?;
                }
            }
            WindowEvent::PageClosed(json) => {
                open.remove(&id);
                app.closed(id);
                if id == app.first {
                    result = Some(from_json(ps, &json)?);
                }
            }
            WindowEvent::Closed(reason) => {
                open.remove(&id);
                app.closed(id);
                match session::closed_error(reason, problems.get(&id).map(String::as_str)) {
                    Some(e) if id == app.first => return Err(e),
                    Some(mut e) => {
                        e.terminating = false;
                        e.message = format!("window {id} of the app: {}", e.message);
                        ps.write_error(&e)?;
                    }
                    None if id == app.first => result = Some("Closed".into_ps()?),
                    None => {}
                }
            }
            WindowEvent::Problem(p) => {
                if id == app.first {
                    pwrs::warning!(ps, "the HTML window reported: {p}")?;
                } else {
                    pwrs::warning!(ps, "window {id} of the app reported: {p}")?;
                }
                problems.insert(id, p);
            }
            WindowEvent::Opened | WindowEvent::Event { .. } | WindowEvent::PickerWanted { .. } | WindowEvent::FileChanged { .. } => {}
        }
    }
    drop(stop);
    drop(readers);
    drop(ending);
    let result = match result {
        Some(r) => r,
        None => "Closed".into_ps()?,
    };
    let out = pwrs::object::new_psobject("PwrsGui.WindowResult");
    pwrs::object::add_note(&out, "Result", result)?;
    ps.write_object(&out)
}

/// What the readers of a -NoWait page app's windows share: the session,
/// the app, the engine event its pages' sends, calls and drops raise, with
/// the handlers its action runs, and what its pages may call.
struct Answering {
    session: Session,
    app: App,
    raised: Option<(PsEvents, String)>,
    handler: Option<PsObject>,
    on_drop: Option<PsObject>,
    calls: Calls,
}

impl Answering {
    /// Window `id` of the app has closed: it leaves the app's open windows,
    /// the folders of the app's channels going with its last, and with
    /// -EndWithFirstWindow the first one's close closes the others still
    /// open, as the session's close.
    fn closed(&self, id: WindowId) {
        self.app.closed(id);
        if id != self.app.first || !self.app.end_with_first {
            return;
        }
        for later in self.app.windows.load().iter() {
            if let Err(e) = self.session.close_window(*later) {
                eprintln!("PWRS-GUI: window {later} of the app could not be closed with the app's first window: {e}");
            }
        }
    }
}

/// Waits for the window's first frame, then writes the window and passes it
/// to a thread that answers the page: its sends, its calls of what `calls`
/// allows, its opens of the app's later windows, which threads of their own
/// answer alike, and its drops of files it does not take, through an engine
/// event whose action runs -OnMessage, the call or -OnDrop; a send with
/// null when there is no handler, and any other call with a refusal. Ctrl+C
/// during the wait closes the window and writes nothing.
fn open_without_waiting(ps: &Pipeline<'_>, mut window: Window, title: String, app: App, handlers: PageHandlers, calls: Calls) -> PsResult<()> {
    let session = window.session().clone();
    let Some(before) = session::first_frame(ps, &mut window)? else {
        return Ok(());
    };
    let id = window.id();
    let source = if handlers.on_message.is_some() || handlers.on_drop.is_some() || !calls.allowed.is_empty() {
        let source = format!("PwrsGui.Message.{}.{id}", session.id());
        let action = script(ACTION)?;
        ps.invoke("Register-EngineEvent", &[("SourceIdentifier", source.clone().into_ps()?), ("Action", action.0)])?;
        Some(source)
    } else {
        None
    };
    let page = PageWindow { id, title, source_identifier: source.clone(), session: session.clone() }.into_ps()?;
    let raised = match source {
        Some(s) => Some((ps.events()?, s)),
        None => None,
    };
    let answering = Arc::new(Answering {
        session,
        app,
        raised,
        handler: handlers.on_message.map(|h| h.0),
        on_drop: handlers.on_drop.map(|h| h.0),
        calls,
    });
    answer_window(window, before, page.clone(), answering).map_err(|why| PsError::new(ErrorCategory::ResourceUnavailable, "GuiThread", why).terminating())?;
    ps.write_object(&page)
}

/// Starts the thread that answers the page of `window`, a window of a
/// -NoWait app, `before` the events that came before its first frame
/// first; the thread runs on its own until the window closes.
fn answer_window(window: Window, before: Vec<WindowEvent>, page: PsObject, answering: Arc<Answering>) -> Result<(), String> {
    let id = window.id();
    std::thread::Builder::new()
        .name(format!("pwrs-gui-page-{id}"))
        .spawn(move || answer_sends(window, before, page, &answering))
        .map_err(|e| format!("the reader of window {id} cannot start: {e}"))?;
    Ok(())
}

/// Opens the later window of a -NoWait app that a page asked for with the
/// options `json`, and starts answering its page: the window's number and
/// title as JSON, or why it did not open, as JSON.
fn open_later(answering: &Arc<Answering>, json: &str) -> Result<String, String> {
    let refused = |why: String| serde_json::json!({ "message": why }).to_string();
    let (node, options, title) = answering.app.later(json).map_err(refused)?;
    let window = answering.session.open_window(options, node).map_err(|e| refused(format!("the window did not open: {e}")))?;
    let id = window.id();
    let source_identifier = answering.raised.as_ref().map(|(_events, source)| source.clone());
    let attached = pwrs::attach_current_thread();
    let made = PageWindow { id, title: title.clone(), source_identifier, session: answering.session.clone() }.into_ps();
    drop(attached);
    let page = made.map_err(|e| refused(format!("window {id} opened, and its page cannot be answered: {}", e.message)))?;
    answering.app.opened(id);
    if let Err(why) = answer_window(window, Vec::new(), page, Arc::clone(answering)) {
        answering.app.closed(id);
        return Err(refused(why));
    }
    Ok(opened_json(id, &title))
}

/// Answers the page of `window`, a window of a -NoWait app, until the
/// window closes, `before`, the events that came before its first frame,
/// first. Each send raises the app's engine event for -OnMessage, or gets
/// null as its reply when there is no handler; each call of what the app
/// allows raises it for the call, and any other call is refused; each open
/// opens a later window of the app; each drop of files the page leaves
/// raises it for -OnDrop.
fn answer_sends(mut window: Window, before: Vec<WindowEvent>, page: PsObject, answering: &Arc<Answering>) {
    let id = window.id();
    let mut before = before.into_iter();
    loop {
        let next = match before.next() {
            Some(event) => Ok(event),
            None => window.next_event(),
        };
        match next {
            Ok(WindowEvent::Message { call, json }) => match (&answering.raised, &answering.handler) {
                (Some((events, source)), Some(h)) => {
                    let data = PageMessage { kind: String::from("Message"), window: page.clone(), call, json, handler: h.clone(), call_errors: String::new() };
                    if let Err(e) = events.raise(source, data) {
                        eprintln!("PWRS-GUI: the HTML window {id} could not raise {source} for its page's send {call}: {e}");
                    }
                }
                (None, _) | (Some(_), None) => {
                    if let Err(e) = window.reply(call, String::from("null")) {
                        eprintln!("PWRS-GUI: the HTML window {id} could not reply to its page's send {call}: {e}");
                    }
                }
            },
            Ok(WindowEvent::Call { call, name, json }) => match (&answering.raised, answering.calls.find(&name)) {
                (Some((events, source)), Some(target)) => {
                    let data = PageMessage { kind: String::from("Call"), window: page.clone(), call, json, handler: target.clone(), call_errors: String::from(answering.calls.errors.name()) };
                    if let Err(e) = events.raise(source, data) {
                        let why = format!("the session could not be asked to run {name}: {e}");
                        if let Err(e) = window.answer_call(call, Err(serde_json::json!({ "message": why }).to_string())) {
                            eprintln!("PWRS-GUI: the HTML window {id} could not answer its page's call {call}, which {source} could not be raised for: {e}");
                        }
                    }
                }
                (None, _) | (Some(_), None) => {
                    if let Err(e) = window.answer_call(call, Err(answering.calls.refused(&name))) {
                        eprintln!("PWRS-GUI: the HTML window {id} could not answer its page's call {call}: {e}");
                    }
                }
            },
            Ok(WindowEvent::Open { call, json }) => {
                let answer = open_later(answering, &json);
                if let Err(e) = window.answer_call(call, answer) {
                    eprintln!("PWRS-GUI: the HTML window {id} could not answer its page's open of a window: {e}");
                }
            }
            Ok(WindowEvent::Event { kind: EventKind::Drop, value, .. }) => {
                if let (Some((events, source)), Some(h)) = (&answering.raised, &answering.on_drop) {
                    let data = PageMessage { kind: String::from("Drop"), window: page.clone(), call: 0, json: drop_json(&value), handler: h.clone(), call_errors: String::new() };
                    if let Err(e) = events.raise(source, data) {
                        eprintln!("PWRS-GUI: the HTML window {id} could not raise {source} for files dropped on its page: {e}");
                    }
                }
            }
            Ok(WindowEvent::Closed(reason)) => {
                if let Some(e) = session::closed_error(reason, None) {
                    eprintln!("PWRS-GUI: the HTML window {id}: {}", e.message);
                }
                answering.closed(id);
                return;
            }
            Ok(WindowEvent::PageClosed(_)) => {
                answering.closed(id);
                return;
            }
            Ok(WindowEvent::Problem(p)) => eprintln!("PWRS-GUI: the HTML window {id} reported: {p}"),
            Ok(WindowEvent::Opened | WindowEvent::Event { .. } | WindowEvent::PickerWanted { .. } | WindowEvent::FileChanged { .. }) => {}
            Err(e) => {
                eprintln!("PWRS-GUI: the HTML window {id} stopped reading its events: {e}");
                answering.closed(id);
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

#[cfg(test)]
mod tests {
    use super::*;

    fn app(node: Node) -> App {
        App { first: 7, node, end_with_first: false, windows: Arc::new(ArcSwap::from_pointee(BTreeSet::from([7]))), folders: Vec::new() }
    }

    /// A page app's first node: HTML text, its developer tools open, without
    /// the system's frame or macOS's buttons, and a policy of its own.
    fn first_node() -> Node {
        Node::new(1, Kind::Html)
            .prop("html", Value::Str(String::from("<p>one</p>")))
            .prop("dev_tools", Value::Bool(true))
            .prop("frameless", Value::Bool(true))
            .prop("hide_window_buttons", Value::Bool(true))
            .prop("follow_title", Value::Bool(false))
            .prop("policy", Value::Str(String::from("default-src 'self'")))
    }

    #[test]
    fn open_takes_one_page_with_a_title_a_size_and_a_frame() {
        let opening = Opening::of(r#"{"path":"settings.html","title":"Settings","width":400,"height":300,"frameless":true}"#).expect("an open of a file");
        let expected = Opening { page: OpenedPage::Path(String::from("settings.html")), title: Some(String::from("Settings")), width: Some(400.0), height: Some(300.0), frameless: Some(true) };
        assert_eq!(opening, expected);
        assert_eq!(Opening::of(r#"{"html":"<p/>"}"#).expect("an open of text").page, OpenedPage::Html(String::from("<p/>")));
        assert_eq!(Opening::of(r#"{"url":"https://example.com/"}"#).expect("an open of an address").page, OpenedPage::Url(String::from("https://example.com/")));
        for (json, why) in [
            ("{}", "names no page"),
            (r#"{"path":"a.html","html":"<p/>"}"#, "given two"),
            (r#"{"html":"<p/>","heigth":300}"#, "not heigth"),
            (r#"{"html":"<p/>","width":50}"#, "from 100 to 10000"),
            (r#"{"html":"<p/>","height":"tall"}"#, "a number of points"),
            (r#"{"html":"<p/>","frameless":"yes"}"#, "true or false"),
            (r#"{"html":42}"#, "is text"),
            ("[1]", "takes an object"),
        ] {
            let refused = Opening::of(json);
            assert!(matches!(refused, Err(ref e) if e.contains(why)), "{json}: {refused:?}");
        }
    }

    #[test]
    fn a_path_names_a_file_of_the_apps_folder_and_nothing_outside_it() {
        let folder = std::env::temp_dir().join(format!("pwrs-gui-app-file-{}", std::process::id()));
        std::fs::create_dir_all(folder.join("pages")).expect("the app's folder");
        std::fs::write(folder.join("pages").join("about.html"), "<p/>").expect("a page of the app");
        assert_eq!(app_file(&folder, "pages\\about.html"), Ok(String::from("pages/about.html")));
        assert_eq!(app_file(&folder, "./pages//about.html"), Ok(String::from("pages/about.html")));
        for (path, why) in [("../about.html", "leaves"), ("/etc/passwd", "no path from"), ("C:/about.html", "no path from"), ("pages/none.html", "no file of"), ("", "names no file")] {
            let refused = app_file(&folder, path);
            assert!(matches!(refused, Err(ref e) if e.contains(why)), "{path}: {refused:?}");
        }
        std::fs::remove_dir_all(&folder).expect("the app's folder removed");
    }

    #[test]
    fn a_later_window_takes_the_first_ones_settings_and_names_its_app() {
        let (node, options, title) = app(first_node()).later(r#"{"html":"<p>two</p>"}"#).expect("a later window");
        assert_eq!(node.props.get("html"), Some(&Value::Str(String::from("<p>two</p>"))));
        assert_eq!(node.props.get("app"), Some(&Value::Int(7)));
        assert_eq!(node.props.get("dev_tools"), Some(&Value::Bool(true)), "the developer tools come from the first window");
        assert_eq!(node.props.get("policy"), first_node().props.get("policy"), "the policy comes from the first window");
        assert_eq!(node.props.get("frameless"), Some(&Value::Bool(true)), "the frame comes from the first window");
        assert_eq!(node.props.get("hide_window_buttons"), Some(&Value::Bool(true)));
        assert_eq!(node.props.get("follow_title"), Some(&Value::Bool(true)), "a window opened without a title takes its document's");
        assert_eq!((title.as_str(), options.width, options.height), ("PWRS-GUI", None, None));
        let framed = app(first_node()).later(r#"{"html":"<p/>","frameless":false,"title":"Two","width":420}"#).expect("a framed window");
        assert_eq!(framed.0.props.get("frameless"), Some(&Value::Bool(false)));
        assert_eq!(framed.0.props.get("hide_window_buttons"), Some(&Value::Bool(false)), "a window with the system's frame keeps its buttons");
        assert_eq!(framed.0.props.get("follow_title"), Some(&Value::Bool(false)));
        assert_eq!((framed.1.title.as_str(), framed.1.width), ("Two", Some(420.0)));
    }

    #[test]
    fn a_later_window_opens_a_path_only_in_a_folder_and_an_address_only_when_trusted() {
        let path = app(first_node()).later(r#"{"path":"a.html"}"#);
        assert!(matches!(path, Err(ref e) if e.contains("no folder")), "{path:?}");
        let remote = app(first_node()).later(r#"{"url":"https://example.com/"}"#);
        assert!(matches!(remote, Err(ref e) if e.contains("-TrustNavigation")), "{remote:?}");
        let trusted = first_node().prop("allow_navigation", Value::Bool(true)).prop("trust_navigation", Value::Bool(true));
        let (node, _options, _title) = app(trusted.clone()).later(r#"{"url":"https://example.com/"}"#).expect("an address the app trusts");
        assert_eq!((node.props.get("url"), node.props.get("html")), (Some(&Value::Str(String::from("https://example.com/"))), None));
        let other = app(trusted).later(r#"{"url":"file:///etc/passwd"}"#);
        assert!(matches!(other, Err(ref e) if e.contains("no http or https")), "{other:?}");
    }
}
