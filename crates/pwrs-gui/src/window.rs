//! Windows described in PowerShell while they are open: Show-GuiWindow
//! -Content's run of its block; the loop that runs each handler on the
//! pipeline thread and writes the window's result; the reader of a window
//! opened with -NoWait, which queues its events for Wait-GuiEvent and runs
//! its handlers as engine-event actions; what the module stores of each
//! such window, which the update commands read; and those commands.

use crate::canvas::TICKS_PER_SECOND;
use crate::inspect::{GuiEventKind, GuiWindowInfo};
use crate::options::GuiRenderer;
use crate::session::{self, gui_error};
use crate::values::{from_value, ordered, to_value};
use crate::widgets::{self, Built, TableRows, TreeItems, TreeSpec, WIDGETS, full_path};
use arc_swap::{ArcSwap, ArcSwapOption};
use pwrs::prelude::*;
use pwrs::sys::PS_TYPE_TIMESPAN;
use pwrs_gui_client::proto::canvas::Board;
use pwrs_gui_client::proto::{EventKind, Fit, Kind, Node, NodeId, PatchOp, Row, Shortcut, Value, WindowId, WindowOptions, date};
use pwrs_gui_client::{Session, Window, WindowEvent};
use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicUsize, Ordering};
use std::sync::{Arc, LazyLock};
use std::thread::Thread;
use std::time::{Duration, Instant};

/// Runs a -Content block with each widget's short name an alias to its
/// New-Gui<Widget> command, set in this script's own scope, which ends with
/// the block, and writes what the block writes. An alias is resolved before
/// a function, so Group wins over the built-in alias group. A name whose
/// built-in alias has AllScope, as Windows PowerShell's group, is set with
/// AllScope, which the engine requires; it still ends with this scope.
const CONTENT: &str = r#"
param($Content, [string[]] $Names)
foreach ($n in $Names) {
    $existing = Get-Alias -Name $n -ErrorAction Ignore
    $option = if ($existing -and ($existing.Options -band [System.Management.Automation.ScopedItemOptions]::AllScope)) { 'AllScope' } else { 'None' }
    Set-Alias -Name $n -Value "PWRS-GUI\New-Gui$n" -Scope Local -Force -Option $option
}
& $Content
"#;

/// Runs a handler with the event as its argument and as $_, and with
/// $PwrsGuiWindow set to its window's number, which the update commands
/// read, and writes one object: Written, what the handler wrote when
/// $Output is Pipeline, and Errors, the error records it raised and the one
/// that ended it, which [`write_records`] makes the command's own. Console
/// sends what it wrote to Out-Default here, and Discard drops it. The
/// script runs through ScriptBlock.Invoke, whose errors would reach the
/// host rather than the command, so none is written here. An array the
/// handler writes stays one object.
const HANDLER: &str = r#"
param($Handler, [uint64] $Window, $WidgetEvent, [string] $Output)
$variables = [System.Collections.Generic.List[psvariable]]::new()
$variables.Add([psvariable]::new('PwrsGuiWindow', $Window))
$variables.Add([psvariable]::new('_', $WidgetEvent))
$failed = [System.Collections.Generic.List[object]]::new()
try {
    $written = @(& { $Handler.InvokeWithContext($null, $variables, $WidgetEvent) } 2>&1 | ForEach-Object {
        if ($_ -is [System.Management.Automation.ErrorRecord]) { $failed.Add($_) } else { , $_ }
    })
} catch {
    $inner = $_.Exception.InnerException
    if ($inner -is [System.Management.Automation.IContainsErrorRecord]) { $failed.Add($inner.ErrorRecord) } else { $failed.Add($_) }
    $written = @()
}
if ($Output -eq 'Console') { $written | Out-Default }
if ($Output -ne 'Pipeline') { $written = @() }
[pscustomobject]@{ Written = $written; Errors = $failed.ToArray() }
"#;

/// Runs a tree's -OnExpand as [`HANDLER`] runs a handler, and writes one
/// object: Failed, whether an error ended it, Written, what it wrote, which
/// becomes the node's children rather than output, and Errors, the error
/// records it raised and the one that ended it.
const EXPAND: &str = r#"
param($Handler, [uint64] $Window, $WidgetEvent)
$variables = [System.Collections.Generic.List[psvariable]]::new()
$variables.Add([psvariable]::new('PwrsGuiWindow', $Window))
$variables.Add([psvariable]::new('_', $WidgetEvent))
$failed = [System.Collections.Generic.List[object]]::new()
try {
    $written = @(& { $Handler.InvokeWithContext($null, $variables, $WidgetEvent) } 2>&1 | ForEach-Object {
        if ($_ -is [System.Management.Automation.ErrorRecord]) { $failed.Add($_) } else { , $_ }
    })
} catch {
    $inner = $_.Exception.InnerException
    if ($inner -is [System.Management.Automation.IContainsErrorRecord]) { $failed.Add($inner.ErrorRecord) } else { $failed.Add($_) }
    [pscustomobject]@{ Failed = $true; Written = @(); Errors = $failed.ToArray() }
    return
}
[pscustomobject]@{ Failed = $false; Written = $written; Errors = $failed.ToArray() }
"#;

/// The engine-event action a -NoWait window's handlers run as, once the
/// engine is idle: the event made from the values the window's reader sent
/// as JSON, and the handler run with it as [`HANDLER`] runs one. The value
/// of a widget's event is the one a window that waits gives, through
/// [`GuiWidgetEventData::shown`]; each named input [`inputs_for_actions`]
/// lists takes its value in State as PowerShell is given it: a table's rows
/// and a tree's items as their objects, a number as a double, which Windows
/// PowerShell's ConvertFrom-Json would give as a decimal, and a password
/// field's text as a SecureString. What a handler writes with Pipeline goes
/// into the event's job, but for -OnExpand, whose output becomes the node's
/// children; when -OnExpand fails, the node closes again.
const ACTION: &str = r#"
$d = $Event.MessageData
$one = @('Activate', 'Expand') -contains [string] $d.Kind
if ($null -ne $d.Node) { $value = [PwrsGui.WidgetEventData]::Shown([uint64] $d.Window, [uint32] $d.Node, [string] $d.ValueJson, $one) }
elseif ($PSVersionTable.PSVersion.Major -ge 7) { $value = ConvertFrom-Json -InputObject $d.ValueJson -NoEnumerate }
else { $value = ConvertFrom-Json -InputObject $d.ValueJson }
$state = ConvertFrom-Json -InputObject $d.StateJson
$inputs = $d.Inputs
if ($inputs) {
    $shown = {
        param($i, $v)
        if ($null -eq $v) { $null }
        elseif ($i.Kind -eq 'Number') { [double] $v }
        elseif ($i.Kind -eq 'Secret') { [System.Net.NetworkCredential]::new('', [string] $v).SecurePassword }
        else { $o = [PwrsGui.WidgetEventData]::Shown([uint64] $i.Window, [uint32] $i.Node, (ConvertTo-Json -InputObject @($v) -Compress), $false); if ($i.Multiple) { , @($o) } else { $o } }
    }
    foreach ($n in @($inputs.Keys)) { if ($state.PSObject.Properties[$n]) { $state.$n = & $shown $inputs[$n] $state.$n } }
}
$e = [pscustomobject]@{ PSTypeName = 'PwrsGui.WidgetEvent'; Window = $d.Window; Name = $d.Name; Kind = $d.Kind; Value = $value; State = $state }
$variables = [System.Collections.Generic.List[psvariable]]::new()
$variables.Add([psvariable]::new('PwrsGuiWindow', [uint64] $d.Window))
$variables.Add([psvariable]::new('_', $e))
if ([string] $d.Kind -eq 'Expand') {
    $item = [uint32] (ConvertFrom-Json -InputObject $d.ValueJson)
    try {
        $written = $d.Handler.InvokeWithContext($null, $variables, $e)
    } catch {
        [PwrsGui.WidgetEventData]::Reopen([uint64] $d.Window, $item)
        $inner = $_.Exception.InnerException
        if ($inner -is [System.Management.Automation.IContainsErrorRecord]) { Write-Error -ErrorRecord $inner.ErrorRecord } else { Write-Error -ErrorRecord $_ }
        return
    }
    [PwrsGui.WidgetEventData]::Expanded([uint64] $d.Window, [uint32] $d.Node, $item, @($written))
    return
}
$written = $d.Handler.InvokeWithContext($null, $variables, $e)
switch ([string] $d.Output) {
    'Pipeline' { $written }
    'Console' { $written | Out-Default }
}
"#;

pub(crate) fn script(text: &str) -> PsResult<PsScriptBlock> {
    Ok(PsScriptBlock(PsType::from_name("scriptblock").call_static("Create", &[text.into_ps()?])?))
}

/// What happens to what a window's handlers write.
#[psenum(name = "PwrsGui.HandlerOutput")]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum GuiHandlerOutput {
    /// It is dropped.
    #[default]
    Discard,
    /// It is written as the command's output, before the window's result;
    /// in a -NoWait window, into the handler's event job.
    Pipeline,
    /// It goes to Out-Default, as if typed at the prompt.
    Console,
}

impl GuiHandlerOutput {
    fn name(self) -> &'static str {
        match self {
            GuiHandlerOutput::Discard => "Discard",
            GuiHandlerOutput::Pipeline => "Pipeline",
            GuiHandlerOutput::Console => "Console",
        }
    }
}

/// What a handler is given, and what Wait-GuiEvent writes: the event, and
/// the window's state.
#[psclass(name = "PwrsGui.WidgetEvent")]
pub struct GuiWidgetEvent {
    /// The window's number, which the update commands take as -Window.
    pub window: u64,
    /// The widget's name; null for a widget with none, and for the window's
    /// own Close.
    pub name: Option<String>,
    /// What happened: one of PwrsGui.EventKind's kinds, such as Click,
    /// Change, Select, PointerDown or DragMove, or the window's Close.
    pub kind: GuiEventKind,
    /// A list's selected item, or with -Multiple its items, for Select, and
    /// a table's or tree's objects; the item, row's object or node's object
    /// for Activate; the node's object for Expand; an input's new value for
    /// Change; a text field's text for Submit; the shortcut for Shortcut and
    /// the key for Key; the paths or text dropped for Drop; where a
    /// GpuSurface's or a Canvas's pointer is, with the canvas's item there,
    /// for their pointer and item events; a chart's point for its Select; a
    /// Markdown link's address for its Click; the window's result for Close;
    /// null for any other Click.
    pub value: PsObject,
    /// Every named input's current value, under its name.
    pub state: PsObject,
}

/// What a -NoWait window's event gives its handler's action.
#[psclass(name = "PwrsGui.WidgetEventData")]
#[derive(Clone)]
pub struct GuiWidgetEventData {
    /// The window's number.
    pub window: u64,
    /// The widget's node in its window; null for the window's own Close.
    pub node: Option<u32>,
    /// The widget's name; null for a widget with none, and for the window's
    /// own Close.
    pub name: Option<String>,
    /// What happened.
    pub kind: GuiEventKind,
    /// The event's value, as JSON.
    pub value_json: String,
    /// Every named input's value under its name, as JSON.
    pub state_json: String,
    /// The handler the action runs.
    pub handler: PsObject,
    /// Where what the handler writes goes.
    pub output: GuiHandlerOutput,
    /// What turns each named input's value in the JSON into the one
    /// PowerShell is given, by name, as [`inputs_for_actions`] makes it;
    /// null for a window with none that needs it.
    pub inputs: PsObject,
}

/// What the -NoWait windows' handler actions, which run on the pipeline
/// thread, call to give their events what a window this thread runs gives
/// its own.
#[psmethods]
impl GuiWidgetEventData {
    /// The value `value_json` of an event of node `node` in window `window`
    /// as PowerShell is given it, as a handler's event in a window that
    /// waits gives it: a table's rows and a tree's items as their objects,
    /// one when `one` is set, for the single row or item of an activation or
    /// an expansion; a password field's text as a SecureString; anything
    /// else as the JSON reads. A window no longer open gives the JSON's
    /// value.
    pub fn shown(window: u64, node: u32, value_json: String, one: bool) -> PsResult<PsObject> {
        let json: serde_json::Value = serde_json::from_str(&value_json).map_err(|e| bad_state(format!("an event's value is not JSON: {e}")))?;
        let value = value_of_json(&json);
        match OPEN.load().get(&window) {
            Some(open) => open.shown(Some(node), &value, one),
            None => from_value(&value),
        }
    }

    /// Makes `written` the children of the tree item `item` that the tree
    /// `tree` of window `window` opened, as Add-GuiItem -Parent makes nodes;
    /// when they cannot become nodes, closes the item again and fails.
    pub fn expanded(window: u64, tree: u32, item: u32, written: Vec<PsObject>) -> PsResult<()> {
        let open = open_window(window)?;
        if let Err(e) = add_tree_items(&open, window, tree, Some(item), &written) {
            reopen_item(&open, window, item)?;
            return Err(e);
        }
        Ok(())
    }

    /// Closes again the tree item `item` of window `window`, whose -OnExpand
    /// failed, so opening it once more runs -OnExpand again.
    pub fn reopen(window: u64, item: u32) -> PsResult<()> {
        let open = open_window(window)?;
        reopen_item(&open, window, item)
    }
}

/// A value from the JSON [`json_of`] writes: an integer as an `Int`, any
/// other number as a `Float`, an object as a map.
fn value_of_json(v: &serde_json::Value) -> Value {
    match v {
        serde_json::Value::Null => Value::Null,
        serde_json::Value::Bool(b) => Value::Bool(*b),
        serde_json::Value::Number(n) => match (n.as_i64(), n.as_f64()) {
            (Some(i), _) => Value::Int(i),
            (None, Some(f)) => Value::Float(f),
            (None, None) => Value::Null,
        },
        serde_json::Value::String(s) => Value::Str(s.clone()),
        serde_json::Value::Array(items) => Value::List(items.iter().map(value_of_json).collect()),
        serde_json::Value::Object(m) => Value::Map(m.iter().map(|(k, v)| (k.clone(), value_of_json(v))).collect()),
    }
}

/// An event of a window opened with -NoWait, as Wait-GuiEvent writes it:
/// its values as PowerShell is given them, taken when the event came.
#[derive(Clone, Debug)]
struct Queued {
    node: Option<NodeId>,
    name: Option<String>,
    kind: GuiEventKind,
    value: Value,
    state: Vec<(String, NodeId, Value)>,
}

/// How many events fit in one chunk of a window's event log.
const LOG_CHUNK: usize = 256;

/// The slots of one chunk of a window's event log.
type LogChunk = Arc<[ArcSwapOption<Queued>]>;

/// The events of a window opened with -NoWait that Wait-GuiEvent has not
/// taken, oldest first: a log the window's reader adds to and Wait-GuiEvent
/// takes from, each event once. Adding an event fills one slot and taking
/// one empties its slot, so neither copies the events waiting; a chunk
/// whose events are all taken is let go.
struct EventLog {
    /// The number of the first chunk stored, and the chunks from it on.
    chunks: ArcSwap<(usize, Vec<LogChunk>)>,
    /// How many events have been added. The window's reader alone adds.
    added: AtomicUsize,
    /// Every event before this one has been taken.
    oldest: AtomicUsize,
}

impl EventLog {
    fn new() -> EventLog {
        EventLog { chunks: ArcSwap::from_pointee((0, Vec::new())), added: AtomicUsize::new(0), oldest: AtomicUsize::new(0) }
    }

    /// The slot of event `n` among `chunks`; none when its chunk was let go.
    fn slot(chunks: &(usize, Vec<LogChunk>), n: usize) -> Option<&ArcSwapOption<Queued>> {
        let chunk = (n / LOG_CHUNK).checked_sub(chunks.0)?;
        chunks.1.get(chunk).and_then(|c| c.get(n % LOG_CHUNK))
    }

    /// Adds an event after those there; only the window's reader adds.
    fn push(&self, queued: Queued) -> Result<(), String> {
        let n = self.added.load(Ordering::Relaxed);
        if n.is_multiple_of(LOG_CHUNK) {
            let fresh: LogChunk = (0..LOG_CHUNK).map(|_| ArcSwapOption::empty()).collect();
            self.chunks.rcu(|c| {
                let mut kept = c.1.clone();
                kept.push(Arc::clone(&fresh));
                (c.0, kept)
            });
        }
        let chunks = self.chunks.load();
        let slot = Self::slot(&chunks, n).ok_or_else(|| format!("the event log has no slot for event {n}"))?;
        slot.store(Some(Arc::new(queued)));
        self.added.store(n + 1, Ordering::Release);
        Ok(())
    }

    /// Takes the oldest event of the widget `name`, or of any widget when it
    /// is `None`; the window's Close matches either.
    fn take(&self, name: Option<&str>) -> Option<Queued> {
        let added = self.added.load(Ordering::Acquire);
        let chunks = self.chunks.load();
        let mut n = self.oldest.load(Ordering::Acquire).max(chunks.0 * LOG_CHUNK);
        while n < added {
            let Some(slot) = Self::slot(&chunks, n) else {
                break;
            };
            let held = slot.load();
            let wanted = match held.as_ref() {
                Some(q) => match name {
                    None => true,
                    Some(w) => q.kind == GuiEventKind::Close || q.name.as_deref().is_some_and(|m| m.eq_ignore_ascii_case(w)),
                },
                None => false,
            };
            if !wanted {
                n += 1;
                continue;
            }
            let before = slot.compare_and_swap(&held, None);
            let won = match (before.as_ref(), held.as_ref()) {
                (Some(a), Some(b)) => Arc::ptr_eq(a, b),
                (None, None) | (Some(_), None) | (None, Some(_)) => false,
            };
            if won {
                let taken = held.as_ref().map(|q| (**q).clone());
                drop(chunks);
                self.trim();
                return taken;
            }
        }
        None
    }

    /// Moves `oldest` past the events taken, and lets go of the chunks
    /// before it.
    fn trim(&self) {
        let added = self.added.load(Ordering::Acquire);
        let chunks = self.chunks.load();
        let mut n = self.oldest.load(Ordering::Acquire).max(chunks.0 * LOG_CHUNK);
        while n < added && Self::slot(&chunks, n).is_some_and(|s| s.load().is_none()) {
            n += 1;
        }
        self.oldest.fetch_max(n, Ordering::AcqRel);
        let keep = n / LOG_CHUNK;
        if keep > chunks.0 {
            self.chunks.rcu(|c| {
                if keep > c.0 {
                    let from = (keep - c.0).min(c.1.len());
                    (keep, c.1[from..].to_vec())
                } else {
                    (c.0, c.1.clone())
                }
            });
        }
    }
}

/// What the module stores of an open window described in PowerShell: the
/// session drawing it, its tree as last set, but for the lines Add-GuiItem
/// adds to a log and the items of its trees, which it stores apart, the
/// current value of each input, and for a window opened with -NoWait its
/// events not yet taken.
pub struct Open {
    session: Session,
    tree: ArcSwap<Node>,
    state: ArcSwap<BTreeMap<NodeId, Value>>,
    /// The result Close-GuiWindow gave from outside the window's handlers,
    /// which the window's result has when it closes.
    closing: ArcSwapOption<Value>,
    /// The window was opened with -NoWait, so its events queue for
    /// Wait-GuiEvent.
    queues: bool,
    /// The events Wait-GuiEvent has not taken, oldest first; the window's
    /// Close last, once it has closed.
    events: EventLog,
    /// The threads waiting in Wait-GuiEvent, which a new event wakes.
    waiters: ArcSwap<Vec<Thread>>,
    /// Each table's rows and the objects they were made from, by its node:
    /// stored apart from `tree`, as the trees' items are, and behind `Arc`, so
    /// a change to one table copies none of the others.
    tables: ArcSwap<BTreeMap<NodeId, Arc<TableRows>>>,
    /// How each tree reads the nodes added to it, by the tree's node.
    trees: BTreeMap<NodeId, TreeSpec>,
    /// Every tree item, by its node and by its object's key: stored apart from
    /// `tree`, so a change to any widget copies the widgets alone, and
    /// behind `Arc`, so a copy of the items takes a count and no handle of
    /// the engine's.
    items: ArcSwap<TreeItems>,
    /// Each canvas's items for later changes, by its node: stored apart from
    /// `tree`, as the tables are, so a change to one canvas copies no widget
    /// and no other canvas.
    canvases: ArcSwap<BTreeMap<NodeId, Arc<Board>>>,
    /// The number the window's next node takes.
    next: AtomicU32,
    /// The window has closed; for a window opened with -NoWait, its reader
    /// sets it once the window's last event is read.
    closed: AtomicBool,
    /// The buffers and render pipelines each GpuSurface's draws name, by
    /// its node, stored until its draws change or the window closes.
    surfaces: ArcSwap<BTreeMap<NodeId, Arc<[PsObject]>>>,
    /// The files the window watches, in full as -WatchPath named them, each
    /// with whether a change to it waits in `events` untaken, so a file has
    /// one change waiting there at a time.
    files: Vec<(String, AtomicBool)>,
}

/// What a window stores of its trees when it opens.
struct Trees {
    specs: BTreeMap<NodeId, TreeSpec>,
    items: TreeItems,
    next: NodeId,
}

impl Open {
    fn new(session: Session, mut tree: Node, tables: BTreeMap<NodeId, TableRows>, trees: Trees, canvases: BTreeMap<NodeId, Board>, queues: bool, files: Vec<String>) -> Open {
        without_items(&mut tree);
        let mut state = BTreeMap::new();
        seeded(&tree, &mut state);
        Open {
            session,
            tree: ArcSwap::from_pointee(tree),
            state: ArcSwap::from_pointee(state),
            closing: ArcSwapOption::empty(),
            queues,
            events: EventLog::new(),
            waiters: ArcSwap::from_pointee(Vec::new()),
            tables: ArcSwap::from_pointee(tables.into_iter().map(|(node, rows)| (node, Arc::new(rows))).collect()),
            trees: trees.specs,
            items: ArcSwap::from_pointee(trees.items),
            canvases: ArcSwap::from_pointee(canvases.into_iter().map(|(node, board)| (node, Arc::new(board))).collect()),
            next: AtomicU32::new(trees.next),
            closed: AtomicBool::new(false),
            surfaces: ArcSwap::from_pointee(BTreeMap::new()),
            files: files.into_iter().map(|f| (f, AtomicBool::new(false))).collect(),
        }
    }

    /// Stores `objects`, the buffers and render pipelines the draws of the
    /// GpuSurface `node` name, in place of those it stored before.
    pub(crate) fn hold_surface(&self, node: NodeId, objects: Vec<PsObject>) {
        let held: Arc<[PsObject]> = objects.into();
        self.surfaces.rcu(|s| {
            let mut s = (**s).clone();
            s.insert(node, Arc::clone(&held));
            s
        });
    }

    /// A value as PowerShell is given it: a table's rows and a tree's items
    /// as the objects they were made from, a password field's text as a
    /// SecureString, a date picker's date as a [datetime], a chart's point
    /// with an X that is a date as a [datetime], any other value as it is.
    /// Runs on a thread the host called in on.
    fn shown(&self, node: Option<NodeId>, value: &Value, one: bool) -> PsResult<PsObject> {
        let Some(node) = node else {
            return from_value(value);
        };
        if let Some(table) = self.tables.load().get(&node) {
            return self.picked(node, table, value, one);
        }
        if self.trees.contains_key(&node) {
            return self.tree_objects(node, value, one);
        }
        let tree = self.tree.load();
        match tree.find(node) {
            Some(n) if n.kind == Kind::Chart => point_object(value),
            Some(n) => input_object(n, value),
            None => from_value(value),
        }
    }

    /// A table's rows as the objects they were made from: one, or with
    /// -Selection Multiple all of them unless `one` is set, for the single row
    /// of an activation.
    fn picked(&self, node: NodeId, table: &TableRows, value: &Value, one: bool) -> PsResult<PsObject> {
        let rows: Vec<&Value> = match value {
            Value::List(items) => items.iter().collect(),
            Value::Null => Vec::new(),
            one_row => vec![one_row],
        };
        let mut picked = Vec::with_capacity(rows.len());
        for r in rows {
            let i = r.as_int().ok_or_else(|| bad_state(format!("table {node} was given a row that is not a number: {r:?}")))?;
            let i = usize::try_from(i).map_err(|e| bad_state(format!("table {node} was given row {i}: {e}")))?;
            let o = table.object(i).ok_or_else(|| bad_state(format!("table {node} has {} rows and was given row {i}", table.count())))?;
            picked.push(o.clone());
        }
        let multiple = !one && matches!(self.tree.load().find(node).and_then(|n| n.props.get("selection")), Some(Value::Str(s)) if s == "multiple");
        if multiple {
            return picked.into_ps();
        }
        match picked.into_iter().next() {
            Some(o) => Ok(o),
            None => Ok(PsObject::null()),
        }
    }

    /// A tree's items as the objects they were made from: one, or with
    /// -Multiple all of them unless `one` is set, for the single item of an
    /// activation or an expansion. An item removed since the renderer named
    /// it has no object to give, so an event that came as it was removed
    /// gives the items that remain.
    fn tree_objects(&self, tree: NodeId, value: &Value, one: bool) -> PsResult<PsObject> {
        let ids: Vec<&Value> = match value {
            Value::List(items) => items.iter().collect(),
            Value::Null => Vec::new(),
            single => vec![single],
        };
        let items = self.items.load();
        let mut picked = Vec::with_capacity(ids.len());
        for v in ids {
            let i = v.as_int().ok_or_else(|| bad_state(format!("tree {tree} was given an item that is not a number: {v:?}")))?;
            let id = NodeId::try_from(i).map_err(|e| bad_state(format!("tree {tree} was given item {i}: {e}")))?;
            if let Some(item) = items.get(&id) {
                picked.push(item.object.clone());
            }
        }
        let multiple = !one && matches!(self.tree.load().find(tree).and_then(|n| n.props.get("multiple")), Some(Value::Bool(true)));
        if multiple {
            return picked.into_ps();
        }
        match picked.into_iter().next() {
            Some(o) => Ok(o),
            None => Ok(PsObject::null()),
        }
    }

    /// Takes what an event says about its input's value, as the renderer
    /// takes it: a list's, table's or tree's selection, the one row or item
    /// an activation selects, or an input's new value.
    fn record(&self, node: NodeId, kind: EventKind, value: &Value) {
        let taken = match (self.tree.load().find(node).map(|n| n.kind), kind) {
            (Some(Kind::ListBox | Kind::Table | Kind::TreeView), EventKind::Select) => Some(value.clone()),
            (Some(Kind::ListBox | Kind::Table | Kind::TreeView), EventKind::Activate) => Some(Value::List(vec![value.clone()])),
            (
                Some(Kind::CheckBox | Kind::TextBox | Kind::NumberBox | Kind::Slider | Kind::ComboBox | Kind::RadioGroup | Kind::Tabs | Kind::ColorPicker | Kind::DatePicker | Kind::FilePicker),
                EventKind::Change,
            ) => Some(value.clone()),
            (Some(Kind::TextBox), EventKind::Submit) => Some(value.clone()),
            _ => None,
        };
        if let Some(v) = taken {
            self.set_value(node, v);
        }
    }

    fn set_value(&self, node: NodeId, value: Value) {
        self.state.rcu(|s| {
            let mut s = (**s).clone();
            s.insert(node, value.clone());
            s
        });
    }

    /// Takes properties the session set on node `node` into the tree as
    /// stored, and the node's value when they set it, as the renderer takes
    /// it; what a person entered stays when they set others.
    fn apply(&self, node: NodeId, props: &BTreeMap<String, Value>) {
        let mut taken = None;
        self.tree.rcu(|t| {
            let mut t = (**t).clone();
            if let Some(n) = t.find_mut(node) {
                n.props.extend(props.iter().map(|(k, v)| (k.clone(), v.clone())));
                taken = widgets::value_prop(n).and_then(|p| props.get(p).cloned());
            }
            t
        });
        if let Some(value) = taken {
            self.set_value(node, value);
        }
    }

    /// Stores the tree items `added` along with those it has.
    fn keep_items(&self, added: &TreeItems) {
        self.items.rcu(|m| {
            let mut m = (**m).clone();
            m.extend(added);
            m
        });
    }

    /// Forgets the tree items `gone`.
    fn forget_items(&self, gone: &BTreeSet<NodeId>) {
        self.items.rcu(|m| {
            let mut m = (**m).clone();
            for id in gone {
                m.remove(*id);
            }
            m
        });
    }

    /// Every named input's current value as PowerShell gives it, under its
    /// name, in tree order.
    fn state_object(&self) -> PsResult<PsObject> {
        self.state_object_of(&self.presented_state().map_err(bad_state)?)
    }

    /// Inputs' values under their names, as an object PowerShell reads, a
    /// table's as its objects.
    fn state_object_of(&self, values: &[(String, NodeId, Value)]) -> PsResult<PsObject> {
        let obj = pwrs::object::new_psobject("PwrsGui.WindowState");
        for (name, node, v) in values {
            pwrs::object::add_note(&obj, name, self.shown(Some(*node), v, false)?)?;
        }
        Ok(obj)
    }

    /// Every named input's node and current value as PowerShell is given it,
    /// under its name, in tree order.
    fn presented_state(&self) -> Result<Vec<(String, NodeId, Value)>, String> {
        let mut out = Vec::new();
        presented_inputs(&self.tree.load(), &self.state.load(), &mut out)?;
        Ok(out)
    }

    /// An event as Wait-GuiEvent writes it, its values taken now.
    fn queued(&self, node: NodeId, kind: EventKind, value: &Value) -> Result<Queued, String> {
        let tree = self.tree.load();
        let (name, value) = match tree.find(node) {
            Some(n) => (n.name.clone(), event_value(n, kind, value)?),
            None => (None, value.clone()),
        };
        Ok(Queued { node: Some(node), name, kind: GuiEventKind::of(kind), value, state: self.presented_state()? })
    }

    /// Queues an event and wakes every waiting Wait-GuiEvent; only the
    /// window's reader calls it.
    fn push(&self, queued: Queued) {
        if let Err(why) = self.events.push(queued) {
            eprintln!("PWRS-GUI: an event of a window could not be queued: {why}");
        }
        for w in self.waiters.load().iter() {
            w.unpark();
        }
    }

    /// The oldest queued event of the widget `name`, or of any widget when
    /// `name` is `None`; the window's Close matches either. A watched file's
    /// change taken lets the next change to that file be queued.
    fn take(&self, name: Option<&str>) -> Option<Queued> {
        let taken = self.events.take(name)?;
        if taken.kind == GuiEventKind::FileChange
            && let Some((_, waiting)) = self.files.iter().find(|(path, _)| taken.name.as_deref() == Some(path.as_str()))
        {
            waiting.store(false, Ordering::Release);
        }
        Some(taken)
    }

    fn wait_here(&self, me: &Thread) {
        let me = me.clone();
        self.waiters.rcu(|w| {
            let mut w = (**w).clone();
            w.push(me.clone());
            w
        });
    }

    fn stop_waiting(&self, me: &Thread) {
        let id = me.id();
        self.waiters.rcu(|w| {
            let mut w = (**w).clone();
            w.retain(|t| t.id() != id);
            w
        });
    }
}

fn bad_state(e: String) -> PsError {
    PsError::new(ErrorCategory::InvalidData, "GuiBadState", e).terminating()
}

/// Whether `node` is a password field, whose text PowerShell is given as a
/// SecureString.
fn is_secret(node: &Node) -> bool {
    node.is_password()
}

/// An input's value as PowerShell is given it: a password field's text as
/// a SecureString, a date picker's date as a [datetime], any other value as
/// it is.
pub(crate) fn input_object(node: &Node, value: &Value) -> PsResult<PsObject> {
    if is_secret(node) {
        secure(value)
    } else if node.kind == Kind::DatePicker {
        date_object(value)
    } else {
        from_value(value)
    }
}

/// A password field's text as a SecureString; null as null.
fn secure(value: &Value) -> PsResult<PsObject> {
    match value {
        Value::Null => Ok(PsObject::null()),
        Value::Str(text) => {
            let credential = PsType::from_name("System.Net.NetworkCredential").new(&[String::new().into_ps()?, text.clone().into_ps()?])?;
            pwrs::object::property(&credential, "SecurePassword")
        }
        other => Err(bad_state(format!("a password field's value is {other:?}, which is not text"))),
    }
}

fn presented_inputs(node: &Node, state: &BTreeMap<NodeId, Value>, out: &mut Vec<(String, NodeId, Value)>) -> Result<(), String> {
    if let (Some(name), Some(_)) = (&node.name, widgets::value_prop(node)) {
        let v = widgets::presented_value(node, state.get(&node.id)).map_err(|e| format!("{name}: {e}"))?;
        out.push((name.clone(), node.id, v));
    }
    node.children.iter().try_for_each(|c| presented_inputs(c, state, out))
}

/// Takes the items out of every tree and canvas in `node`, which the
/// window stores apart from its widgets.
fn without_items(node: &mut Node) {
    match node.kind {
        Kind::TreeView => node.children.clear(),
        Kind::Canvas => {
            node.props.remove("items");
        }
        _ => node.children.iter_mut().for_each(without_items),
    }
}

/// The current value of each input in `tree`, from its properties.
fn seeded(node: &Node, state: &mut BTreeMap<NodeId, Value>) {
    if let Some(p) = widgets::value_prop(node) {
        let value = match node.props.get(p) {
            Some(v) => v.clone(),
            None => Value::Null,
        };
        state.insert(node.id, value);
    }
    for c in &node.children {
        seeded(c, state);
    }
}

/// A value as JSON, as ConvertFrom-Json reads it back: a map as an object
/// in its order, bytes as an array of numbers, a number that is not finite
/// as null.
fn json_of(v: &Value) -> serde_json::Value {
    match v {
        Value::Null => serde_json::Value::Null,
        Value::Bool(b) => serde_json::Value::Bool(*b),
        Value::Int(i) => serde_json::Value::from(*i),
        Value::Float(f) => match serde_json::Number::from_f64(*f) {
            Some(n) => serde_json::Value::Number(n),
            None => serde_json::Value::Null,
        },
        Value::Str(s) => serde_json::Value::String(s.clone()),
        Value::List(items) => serde_json::Value::Array(items.iter().map(json_of).collect()),
        Value::Map(m) => serde_json::Value::Object(m.iter().map(|(k, v)| (k.clone(), json_of(v))).collect()),
        Value::Bytes(b) => serde_json::Value::Array(b.iter().map(|x| serde_json::Value::from(*x)).collect()),
    }
}

/// The open windows described in PowerShell, in every runspace, by number;
/// a window opened with -NoWait stays until Wait-GuiEvent takes its Close.
static OPEN: LazyLock<ArcSwap<BTreeMap<WindowId, Arc<Open>>>> = LazyLock::new(|| ArcSwap::from_pointee(BTreeMap::new()));

fn register(id: WindowId, open: &Arc<Open>) {
    OPEN.rcu(|m| {
        let mut m = (**m).clone();
        m.insert(id, Arc::clone(open));
        m
    });
}

fn forget(id: WindowId) {
    OPEN.rcu(|m| {
        let mut m = (**m).clone();
        m.remove(&id);
        m
    });
}

/// Forgets every window described in PowerShell, as the module does once no
/// runspace of the process has it imported.
pub fn forget_all() {
    OPEN.store(Arc::new(BTreeMap::new()));
}

/// Lists a window among the open ones for as long as it lives.
struct Registered(WindowId);

impl Registered {
    fn new(id: WindowId, open: &Arc<Open>) -> Registered {
        register(id, open);
        Registered(id)
    }
}

impl Drop for Registered {
    fn drop(&mut self) {
        forget(self.0);
        crate::gpu::release_window(self.0);
    }
}

/// The open window described in PowerShell numbered `id`.
fn open_window(id: WindowId) -> PsResult<Arc<Open>> {
    match OPEN.load().get(&id) {
        Some(o) => Ok(Arc::clone(o)),
        None => Err(PsError::new(ErrorCategory::ObjectNotFound, "GuiNoWindow", format!("no window described in PowerShell is open with the number {id}")).terminating()),
    }
}

thread_local! {
    /// The windows whose commands wait on this thread, each with the result
    /// Close-GuiWindow gave it from inside one of its handlers.
    static WAITING: RefCell<BTreeMap<WindowId, Option<PsObject>>> = const { RefCell::new(BTreeMap::new()) };
}

/// Marks a window as waited on by this thread for as long as it lives.
struct Waited(WindowId);

impl Waited {
    fn new(id: WindowId) -> Waited {
        WAITING.with(|w| w.borrow_mut().insert(id, None));
        Waited(id)
    }

    /// The result Close-GuiWindow gave from inside a handler, once.
    fn closing(&self) -> Option<PsObject> {
        WAITING.with(|w| w.borrow_mut().get_mut(&self.0).and_then(Option::take))
    }
}

impl Drop for Waited {
    fn drop(&mut self) {
        WAITING.with(|w| w.borrow_mut().remove(&self.0));
    }
}

/// What Show-GuiWindow -Content was given.
pub struct ShowArgs<'a> {
    pub content: &'a PsScriptBlock,
    pub title: Option<String>,
    pub width: Option<f64>,
    pub height: Option<f64>,
    pub renderer: Option<GuiRenderer>,
    /// The window's own CSS, for the HTML renderer's page.
    pub style: Option<&'a str>,
    pub output: GuiHandlerOutput,
    pub on_close: Option<&'a PsScriptBlock>,
    pub on_shortcut: Option<&'a PsObject>,
    pub on_drop: Option<&'a PsScriptBlock>,
    pub watch_path: Option<&'a [String]>,
    pub watch_interval: Option<&'a PsObject>,
    pub on_file_change: Option<&'a PsScriptBlock>,
    pub no_wait: bool,
}

/// The files a window watches, each in full and once, and the pace
/// -WatchInterval sets, none to take the system's notifications.
type Watch = (Vec<String>, Option<Duration>);

/// What -WatchPath and -WatchInterval give; none without -WatchPath, which
/// -WatchInterval and -OnFileChange need. A file need not exist yet, but
/// its folder must, for the system to tell of the file once it is made.
fn watch_of(ps: &Pipeline<'_>, args: &ShowArgs<'_>) -> PsResult<Option<Watch>> {
    let refuse = |text: String| PsError::new(ErrorCategory::InvalidArgument, "GuiBadWatch", text).terminating();
    let Some(given) = args.watch_path else {
        if args.watch_interval.is_some() {
            return Err(refuse(String::from("-WatchInterval sets how often the files -WatchPath names are looked at; give -WatchPath too")));
        }
        if args.on_file_change.is_some() {
            return Err(refuse(String::from("-OnFileChange runs when a file -WatchPath names changes; give -WatchPath too")));
        }
        return Ok(None);
    };
    if given.is_empty() {
        return Err(refuse(String::from("-WatchPath names no file")));
    }
    let mut files: Vec<String> = Vec::with_capacity(given.len());
    for path in given {
        let full = full_path(ps, path)?;
        if !Path::new(&full).parent().is_some_and(Path::is_dir) {
            let text = format!("{full} cannot be watched: its folder does not exist");
            return Err(PsError::new(ErrorCategory::ResourceUnavailable, "GuiNoWatch", text).terminating());
        }
        let same = |other: &String| if cfg!(any(windows, target_os = "macos")) { other.to_lowercase() == full.to_lowercase() } else { *other == full };
        if files.iter().any(same) {
            return Err(refuse(format!("-WatchPath names {full} twice")));
        }
        files.push(full);
    }
    let every = match args.watch_interval {
        Some(given) => Some(watch_every(given)?),
        None => None,
    };
    Ok(Some((files, every)))
}

/// -WatchInterval's seconds between looks at the watched files: a number,
/// or a [TimeSpan]'s length, above 0.
fn watch_every(given: &PsObject) -> PsResult<Duration> {
    let refuse = |text: String| PsError::new(ErrorCategory::InvalidArgument, "GuiBadWatch", text).terminating();
    let seconds = if given.type_tag()? == PS_TYPE_TIMESPAN {
        PsTimeSpan::from_ps(given)?.ticks as f64 / TICKS_PER_SECOND
    } else {
        f64::from_ps(given).map_err(|e| refuse(format!("-WatchInterval takes seconds or a [TimeSpan]: {}", e.message)))?
    };
    match Duration::try_from_secs_f64(seconds) {
        Ok(every) if !every.is_zero() => Ok(every),
        Ok(_) => Err(refuse(format!("-WatchInterval is {seconds}; it is the seconds between looks at the files, above 0"))),
        Err(e) => Err(refuse(format!("-WatchInterval is {seconds}, which is no length of time: {e}"))),
    }
}

/// The window's own shortcuts -OnShortcut gives, each written one way,
/// with the script block it runs.
fn shortcuts_of(table: Option<&PsObject>) -> PsResult<HashMap<String, PsScriptBlock>> {
    let mut out = HashMap::new();
    let Some(table) = table else {
        return Ok(out);
    };
    let refuse = |text: String| PsError::new(ErrorCategory::InvalidArgument, "GuiBadShortcut", text).terminating();
    let read = PsReadOnlyTable::over(table)?;
    for key in read.keys()? {
        let written = Shortcut::parse(&key).map_err(|e| refuse(format!("-OnShortcut: {e}")))?.written();
        let handler = read.get(&key)?;
        let type_name = handler.type_name()?;
        if type_name != "System.Management.Automation.ScriptBlock" {
            return Err(refuse(format!("-OnShortcut gives {written} a {type_name}; each shortcut takes a script block")));
        }
        if out.insert(written.clone(), PsScriptBlock(handler)).is_some() {
            return Err(refuse(format!("-OnShortcut names {written} twice")));
        }
    }
    Ok(out)
}

/// Runs the block and opens the window it describes; then runs the
/// window's handlers until it closes and writes its result, or with
/// -NoWait writes the window once its first frame is drawn.
pub fn show(ps: &Pipeline<'_>, args: ShowArgs<'_>) -> PsResult<()> {
    let names: Vec<String> = WIDGETS.iter().map(|n| n.to_string()).collect();
    let mut widgets = Vec::new();
    for o in script(CONTENT)?.call(ps, &[args.content.0.clone(), names.into_ps()?])? {
        if o.is_null() {
            continue;
        }
        let type_name = o.type_name()?;
        if type_name != "PwrsGui.Widget" {
            let text = format!("-Content wrote a {type_name}, which is not a widget; widgets come from the short names, such as Text and Button, or from New-Gui<Widget>");
            return Err(PsError::new(ErrorCategory::InvalidArgument, "GuiNotAWidget", text).terminating());
        }
        widgets.push(o);
    }
    if widgets.is_empty() {
        return Err(PsError::new(ErrorCategory::InvalidArgument, "GuiNoWidgets", "-Content wrote no widget").terminating());
    }
    let mut built = widgets::build(ps, &widgets)?;
    names_once(&built.tree, &mut BTreeSet::new())?;
    let shortcuts = shortcuts_of(args.on_shortcut)?;
    if !shortcuts.is_empty() {
        let mut written: Vec<&String> = shortcuts.keys().collect();
        written.sort();
        built.tree.props.insert(String::from("shortcuts"), Value::List(written.into_iter().map(|s| Value::Str(s.clone())).collect()));
    }
    if let Some(h) = args.on_drop {
        built.tree.props.insert(String::from("drop"), Value::Bool(true));
        built.handlers.insert((built.tree.id, EventKind::Drop), PsScriptBlock(h.0.clone()));
    }
    if let Some(css) = args.style {
        built.tree.props.insert(String::from("stylesheet"), Value::Str(css.to_string()));
    }
    let watch = watch_of(ps, &args)?;
    let title = args.title.clone().unwrap_or_else(|| String::from("PWRS-GUI"));
    let options = WindowOptions {
        title: title.clone(),
        width: args.width.map(|w| w as f32),
        height: args.height.map(|h| h as f32),
        resizable: true,
        always_on_top: false,
        close_on_escape: false,
        dialog: false,
        fit: Fit::WINDOW,
        theme: crate::options::theme(),
    };
    let session = match args.renderer {
        Some(r) => session::widget_session_for(ps, r.into())?,
        None => session::widget_session(ps)?,
    };
    let opened: Vec<(NodeId, Vec<Row>)> = built.rows.iter().filter(|(_, rows)| !rows.is_empty()).map(|(node, rows)| (*node, rows.clone())).collect();
    let mut window = session.open_window_with_rows(options, built.tree.clone(), opened).map_err(gui_error)?;
    let files = match watch {
        Some((files, every)) => {
            if let Err(e) = window.watch_files(files.iter().map(PathBuf::from).collect(), every) {
                let id = window.id();
                if let Err(closing) = window.close() {
                    pwrs::warning!(ps, "window {id} could not be closed once its files could not be watched: {closing}")?;
                }
                return Err(gui_error(e));
            }
            files
        }
        None => Vec::new(),
    };
    if args.no_wait {
        let handling = Handling { on_close: args.on_close, output: args.output, files, on_file_change: args.on_file_change };
        match open_without_waiting(ps, session, window, built, title, shortcuts, handling)? {
            Some(info) => ps.write(info),
            None => Ok(()),
        }
    } else {
        run(ps, session, window, built, shortcuts, files, &args)
    }
}

/// Opens the window `widgets` describe as Show-GuiWindow -NoWait does, with
/// no handlers, and returns what Get-GuiWindow writes of it once its first
/// frame is drawn; `None` when Ctrl+C ended that wait.
pub(crate) fn open_widgets(ps: &Pipeline<'_>, widgets: &[PsObject], options: WindowOptions) -> PsResult<Option<GuiWindowInfo>> {
    let built = widgets::build(ps, widgets)?;
    names_once(&built.tree, &mut BTreeSet::new())?;
    let title = options.title.clone();
    let session = session::widget_session(ps)?;
    let window = session.open_window(options, built.tree.clone()).map_err(gui_error)?;
    open_without_waiting(ps, session, window, built, title, HashMap::new(), Handling { on_close: None, output: GuiHandlerOutput::default(), files: Vec::new(), on_file_change: None })
}

/// Whether window `id` is open: the module has it, and it has not closed.
pub(crate) fn is_open(id: WindowId) -> bool {
    match OPEN.load().get(&id) {
        Some(open) => !open.closed.load(Ordering::Acquire),
        None => false,
    }
}

/// Closes window `id` from outside its handlers, its result Closed.
pub(crate) fn close(id: WindowId) -> PsResult<()> {
    if let Some(open) = OPEN.load().get(&id) {
        open.closing.store(Some(Arc::new(Value::Str(String::from("Closed")))));
    }
    session::of_window(id)?.close_window(id).map_err(gui_error)
}

/// Waits until window `id`, opened with -NoWait, has closed: until its
/// reader has read the window's last event, after which [`is_open`] returns
/// false. Ctrl+C ends the wait. A window the module does not have, or one
/// whose command reads its events itself, is not waited for.
pub(crate) fn wait_closed(ps: &Pipeline<'_>, id: WindowId) -> PsResult<()> {
    let Some(open) = OPEN.load().get(&id).cloned() else {
        return Ok(());
    };
    if !open.queues {
        return Ok(());
    }
    let me = std::thread::current();
    let woken = me.clone();
    let waker = ps.on_stop(move || woken.unpark())?;
    open.wait_here(&me);
    while !open.closed.load(Ordering::Acquire) && !ps.stopping() {
        std::thread::park();
    }
    open.stop_waiting(&me);
    drop(waker);
    Ok(())
}

/// Fails when two widgets share a name, compared without regard to case.
fn names_once(node: &Node, seen: &mut BTreeSet<String>) -> PsResult<()> {
    if let Some(n) = &node.name
        && !seen.insert(n.to_lowercase())
    {
        let text = format!("two widgets are named {n}; State and the update commands find a widget by its name, so each must be its own");
        return Err(PsError::new(ErrorCategory::InvalidArgument, "GuiNameTwice", text).terminating());
    }
    node.children.iter().try_for_each(|c| names_once(c, seen))
}

/// Runs the window's handlers on this thread until it closes, then
/// -OnClose, and writes the result: what a -Result button or
/// Close-GuiWindow gave, or Closed. Ctrl+C closes the window and writes
/// nothing.
fn run(ps: &Pipeline<'_>, session: Session, mut window: Window, built: Built, shortcuts: HashMap<String, PsScriptBlock>, files: Vec<String>, args: &ShowArgs<'_>) -> PsResult<()> {
    let id = window.id();
    let Built { tree, handlers, results, tables, trees, items, canvases, next, .. } = built;
    let open = Arc::new(Open::new(session, tree, tables, Trees { specs: trees, items, next }, canvases, false, files));
    let _registered = Registered::new(id, &open);
    let waited = Waited::new(id);
    let handler = script(HANDLER)?;
    let mut problem = None;
    let result = loop {
        let Some(event) = session::next_event(ps, &mut window)? else {
            return Ok(());
        };
        match event {
            WindowEvent::Event { node, kind, value, .. } => {
                open.record(node, kind, &value);
                let shortcut = match (&kind, &value) {
                    (EventKind::Shortcut, Value::Str(s)) => shortcuts.get(s),
                    _ => None,
                };
                if kind == EventKind::Expand {
                    expand(ps, &open, id, node, &value, handlers.get(&(node, kind)))?;
                } else if let Some(h) = handlers.get(&(node, kind)).or(shortcut) {
                    let e = event_object(&open, id, node, kind, &value)?;
                    run_handler(ps, &handler, h, id, e, args.output)?;
                }
                if let Some(r) = waited.closing() {
                    break r;
                }
                if kind == EventKind::Click
                    && let Some(r) = results.get(&node)
                {
                    break r.clone();
                }
            }
            WindowEvent::Closed(reason) => {
                if let Some(e) = session::closed_error(reason, problem.as_deref()) {
                    return Err(e);
                }
                break match open.closing.load_full() {
                    Some(v) => from_value(&v)?,
                    None => "Closed".into_ps()?,
                };
            }
            WindowEvent::Problem(p) => {
                pwrs::warning!(ps, "the window reported: {p}")?;
                problem = Some(p);
            }
            WindowEvent::Message { json, .. } => pwrs::warning!(ps, "the window, which shows no page, got a page's message: {json}")?,
            WindowEvent::Call { name, .. } => pwrs::warning!(ps, "the window, which shows no page, got a page's call of {name}")?,
            WindowEvent::Open { json, .. } => pwrs::warning!(ps, "the window, which shows no page, got a page's open of a window: {json}")?,
            WindowEvent::PageClosed(_) => break "Closed".into_ps()?,
            WindowEvent::Opened => {}
            WindowEvent::PickerWanted { node, title, kind } => {
                let outcome = crate::dialogs::draw_for_chooser(ps, &title, kind)?;
                session::of_window(id)?.answer_picker(id, node, outcome).map_err(gui_error)?;
            }
            WindowEvent::FileChanged { file, length, .. } => {
                if let Some(h) = args.on_file_change {
                    let name = open.files.get(file).map(|(path, _)| path.clone());
                    let e = GuiWidgetEvent { window: id, name, kind: GuiEventKind::FileChange, value: from_value(&length_value(length))?, state: open.state_object()? }.into_ps()?;
                    run_handler(ps, &handler, h, id, e, args.output)?;
                }
                if let Some(r) = waited.closing() {
                    break r;
                }
            }
        }
    };
    window.close().map_err(gui_error)?;
    let state = open.state_object()?;
    if let Some(h) = args.on_close {
        let e = GuiWidgetEvent { window: id, name: None, kind: GuiEventKind::Close, value: result.clone(), state: state.clone() }.into_ps()?;
        run_handler(ps, &handler, h, id, e, args.output)?;
    }
    let out = pwrs::object::new_psobject("PwrsGui.WindowResult");
    pwrs::object::add_note(&out, "Result", result)?;
    pwrs::object::add_note(&out, "State", state)?;
    ps.write_object(&out)
}

/// The event a handler is given, its value as PowerShell gives it.
fn event_object(open: &Open, id: WindowId, node: NodeId, kind: EventKind, value: &Value) -> PsResult<PsObject> {
    let tree = open.tree.load();
    let (name, shown) = match tree.find(node) {
        Some(n) if matches!(kind, EventKind::Drop | EventKind::Key) => (n.name.clone(), from_value(value)?),
        Some(n) => (n.name.clone(), open.shown(Some(node), &event_value(n, kind, value).map_err(bad_state)?, matches!(kind, EventKind::Activate | EventKind::Expand))?),
        None => (None, from_value(value)?),
    };
    GuiWidgetEvent { window: id, name, kind: GuiEventKind::of(kind), value: shown, state: open.state_object()? }.into_ps()
}

/// An event's value as PowerShell is given it: a list's selection or
/// activated item as text, a table's or tree's rows or items as the
/// renderer gives them, an input's new value as State gives it, a Markdown
/// link's address for its click, a chart's point as a map of its Series, X
/// and Y, a canvas's pointer and the item under it, and nothing for any
/// other click.
fn event_value(node: &Node, kind: EventKind, value: &Value) -> Result<Value, String> {
    match (node.kind, kind) {
        (
            Kind::Canvas,
            EventKind::Click
            | EventKind::DoubleClick
            | EventKind::PointerEnter
            | EventKind::PointerLeave
            | EventKind::DragStart
            | EventKind::DragMove
            | EventKind::DragEnd
            | EventKind::PointerDown
            | EventKind::PointerUp
            | EventKind::PointerMove
            | EventKind::Wheel,
        ) => canvas_pointer(kind, value),
        (Kind::Markdown, EventKind::Click) => Ok(value.clone()),
        (_, EventKind::Click) => Ok(Value::Null),
        (Kind::ListBox | Kind::TreeView, EventKind::Select) => widgets::presented_value(node, Some(value)),
        (Kind::Chart, EventKind::Select) => chart_point(value),
        (
            Kind::CheckBox | Kind::TextBox | Kind::NumberBox | Kind::Slider | Kind::ComboBox | Kind::RadioGroup | Kind::Tabs | Kind::ColorPicker | Kind::DatePicker | Kind::FilePicker,
            EventKind::Change,
        )
        | (Kind::TextBox, EventKind::Submit) => widgets::presented_value(node, Some(value)),
        (Kind::ListBox, EventKind::Activate) => {
            let mut single = node.clone();
            single.props.insert(String::from("multiple"), Value::Bool(false));
            widgets::presented_value(&single, Some(value))
        }
        (Kind::GpuSurface, EventKind::PointerDown | EventKind::PointerUp | EventKind::PointerMove | EventKind::Wheel) => surface_pointer(kind, value),
        _ => Ok(value.clone()),
    }
}

/// A GpuSurface's pointer event as the renderer reports it, under the
/// names PowerShell reads: X, Y, U, V, PixelsPerPoint, Button, Shift, Ctrl
/// and Alt, and Notches for a wheel turn.
fn surface_pointer(kind: EventKind, value: &Value) -> Result<Value, String> {
    const WHERE: [(&str, &str); 9] = [("x", "X"), ("y", "Y"), ("u", "U"), ("v", "V"), ("ppp", "PixelsPerPoint"), ("button", "Button"), ("shift", "Shift"), ("ctrl", "Ctrl"), ("alt", "Alt")];
    const TURNED: [(&str, &str); 1] = [("notches", "Notches")];
    let given = value.as_map().ok_or_else(|| format!("a surface's pointer event is a map of where and what, not {value:?}"))?;
    let wheel: &[(&str, &str)] = if kind == EventKind::Wheel { &TURNED } else { &[] };
    let mut out = BTreeMap::new();
    for (wire, shown) in WHERE.iter().chain(wheel) {
        let v = given.get(*wire).ok_or_else(|| format!("a surface's {kind:?} has no {wire}: {value:?}"))?;
        out.insert((*shown).to_string(), v.clone());
    }
    Ok(Value::Map(out))
}

/// A canvas's event as the renderer reports it: a GpuSurface's pointer
/// event under the names PowerShell reads, with the Item under the
/// pointer, the topmost named one, or null where none is, and SceneX and
/// SceneY, the pointer in the items' coordinates under the canvas's view.
fn canvas_pointer(kind: EventKind, value: &Value) -> Result<Value, String> {
    let given = |key: &str| value.as_map().and_then(|m| m.get(key)).cloned().ok_or_else(|| format!("a canvas's {kind:?} has no {key}: {value:?}"));
    let (item, scene_x, scene_y) = (given("item")?, given("scenex")?, given("sceney")?);
    let Value::Map(mut out) = surface_pointer(kind, value)? else {
        return Err(format!("a canvas's {kind:?} is a map of where and what, not {value:?}"));
    };
    out.insert(String::from("Item"), item);
    out.insert(String::from("SceneX"), scene_x);
    out.insert(String::from("SceneY"), scene_y);
    Ok(Value::Map(out))
}

/// A chart's point the renderer reports, a map of its `series`, `x` and
/// `y`, under the names PowerShell reads: Series, X and Y.
fn chart_point(value: &Value) -> Result<Value, String> {
    let point = value.as_map().ok_or_else(|| format!("a chart's point is a map of its series, x and y, not {value:?}"))?;
    let mut out = BTreeMap::new();
    for (wire, shown) in [("series", "Series"), ("x", "X"), ("y", "Y")] {
        let v = point.get(wire).ok_or_else(|| format!("a chart's point has no {wire}: {value:?}"))?;
        out.insert(shown.to_string(), v.clone());
    }
    Ok(Value::Map(out))
}

/// A date written as [`date`] writes one, as a [datetime] of no zone, as
/// PowerShell reads such text; null for none.
fn date_object(value: &Value) -> PsResult<PsObject> {
    match value {
        Value::Null => Ok(PsObject::null()),
        Value::Str(text) => PsDateTime::new(date::parse(text).map_err(bad_state)?, DateTimeKind::Unspecified).into_ps(),
        other => Err(bad_state(format!("a date is written as text, not {other:?}"))),
    }
}

/// A chart's point as PowerShell is given it: an ordered dictionary of its
/// Series, X and Y, an X written as a date given as a [datetime].
fn point_object(value: &Value) -> PsResult<PsObject> {
    let Some(point) = value.as_map() else {
        return from_value(value);
    };
    let mut entries = Vec::with_capacity(point.len());
    for (key, v) in point {
        let shown = if key == "X" && matches!(v, Value::Str(_)) { date_object(v)? } else { from_value(v)? };
        entries.push((key.clone(), shown));
    }
    ordered(entries)
}

/// Runs one handler through [`HANDLER`], writing what it writes when the
/// output goes to the pipeline, then the errors it raised as the command's
/// own.
fn run_handler(ps: &Pipeline<'_>, runner: &PsScriptBlock, handler: &PsScriptBlock, window: WindowId, event: PsObject, output: GuiHandlerOutput) -> PsResult<()> {
    let ran = runner.call(ps, &[handler.0.clone(), window.into_ps()?, event, output.name().into_ps()?])?;
    let Some(outcome) = ran.first() else {
        return Err(bad_state(String::from("the runner of a handler wrote nothing")));
    };
    for o in Vec::<PsObject>::from_ps(&pwrs::object::property(outcome, "Written")?)? {
        ps.write_object(&o)?;
    }
    write_records(ps, outcome)
}

/// Adds tree items made from `objects` to the tree `tree` of window `id`,
/// under the item `parent`, or among the tree's top items when it is
/// `None`: stores them with their objects, then sends them to the renderer.
/// An item given its children loses `more`, so one given none is a leaf.
/// Every way a tree grows comes through here: Add-GuiItem, and -OnExpand's
/// output in either kind of window.
fn add_tree_items(open: &Open, id: WindowId, tree: NodeId, parent: Option<NodeId>, objects: &[PsObject]) -> PsResult<()> {
    let spec = open.trees.get(&tree).ok_or_else(|| bad_state(format!("window {id} has no tree {tree}")))?;
    let mut number = || -> PsResult<NodeId> {
        let n = open.next.fetch_add(1, Ordering::Relaxed);
        if n == NodeId::MAX {
            return Err(PsError::new(ErrorCategory::LimitsExceeded, "GuiNoNodeNumber", format!("window {id} has used every node number")).terminating());
        }
        Ok(n)
    };
    let mut made = TreeItems::default();
    let nodes = widgets::tree_items(spec, tree, parent, objects, &mut number, &mut made)?;
    let at = match parent {
        Some(p) => p,
        None => tree,
    };
    let mut ops: Vec<PatchOp> = nodes.into_iter().map(|node| PatchOp::Insert { parent: at, index: u32::MAX, node }).collect();
    if let Some(p) = parent {
        ops.push(PatchOp::SetProps { node: p, props: BTreeMap::from([(String::from("more"), Value::Bool(false))]) });
    }
    if ops.is_empty() {
        return Ok(());
    }
    open.keep_items(&made);
    open.session.patch(id, ops).map_err(gui_error)
}

/// Closes again the tree item `item` of window `id`, whose -OnExpand
/// failed, so that opening it once more runs -OnExpand again.
fn reopen_item(open: &Open, id: WindowId, item: NodeId) -> PsResult<()> {
    let props = BTreeMap::from([(String::from("more"), Value::Bool(true))]);
    open.session.patch(id, vec![PatchOp::SetProps { node: item, props }]).map_err(gui_error)
}

/// The tree item an Expand or Activate event names.
fn item_named(value: &Value) -> PsResult<NodeId> {
    let i = value.as_int().ok_or_else(|| bad_state(format!("a tree's event named {value:?}, which is not an item")))?;
    NodeId::try_from(i).map_err(|e| bad_state(format!("a tree's event named item {i}: {e}")))
}

/// Handles a tree's Expand in a window this thread runs: runs -OnExpand for
/// the item the event names and makes what it writes the item's children,
/// then writes the errors it raised as the command's own. When an error
/// ends the handler, the item closes again and that error is written; when
/// what it wrote cannot become nodes, that error is written and the item
/// closes again. A tree with no -OnExpand makes the item a leaf.
fn expand(ps: &Pipeline<'_>, open: &Open, id: WindowId, tree: NodeId, value: &Value, handler: Option<&PsScriptBlock>) -> PsResult<()> {
    let item = item_named(value)?;
    let Some(handler) = handler else {
        return add_tree_items(open, id, tree, Some(item), &[]);
    };
    let event = event_object(open, id, tree, EventKind::Expand, value)?;
    let ran = match script(EXPAND)?.call(ps, &[handler.0.clone(), id.into_ps()?, event]) {
        Ok(ran) => ran,
        Err(e) => {
            reopen_item(open, id, item)?;
            return Err(e);
        }
    };
    let Some(outcome) = ran.first() else {
        reopen_item(open, id, item)?;
        return Err(bad_state(String::from("the runner of -OnExpand wrote nothing")));
    };
    if bool::from_ps(&pwrs::object::property(outcome, "Failed")?)? {
        reopen_item(open, id, item)?;
        return write_records(ps, outcome);
    }
    let written = Vec::<PsObject>::from_ps(&pwrs::object::property(outcome, "Written")?)?;
    if let Err(mut e) = add_tree_items(open, id, tree, Some(item), &written) {
        reopen_item(open, id, item)?;
        e.terminating = false;
        ps.write_error(&e)?;
    }
    write_records(ps, outcome)
}

/// Writes the error records a handler raised, which `outcome` has as its
/// Errors, as this command's own errors, each as it was raised: a
/// Write-Error -ErrorRecord the command runs passes its error to the
/// command's error stream, where -ErrorVariable, -ErrorAction and a
/// redirection find it.
fn write_records(ps: &Pipeline<'_>, outcome: &PsObject) -> PsResult<()> {
    for record in Vec::<PsObject>::from_ps(&pwrs::object::property(outcome, "Errors")?)? {
        ps.invoke("Write-Error", &[("ErrorRecord", record)])?;
    }
    Ok(())
}

/// What runs a -NoWait window's handlers: -OnClose, where the handlers'
/// output goes, and the files the window watches, with -OnFileChange.
struct Handling<'a> {
    on_close: Option<&'a PsScriptBlock>,
    output: GuiHandlerOutput,
    files: Vec<String>,
    on_file_change: Option<&'a PsScriptBlock>,
}

/// A watched file's length as an event's value: its bytes, or null while
/// the file is gone.
fn length_value(length: Option<u64>) -> Value {
    match length {
        Some(n) => Value::Int(n as i64),
        None => Value::Null,
    }
}

/// Waits for the window's first frame, then passes the window to a reader
/// thread and returns what Get-GuiWindow writes of it: its events queue for
/// Wait-GuiEvent, and those with a handler, and the window's Close with
/// -OnClose, raise the engine event whose action runs the handler. `None`
/// when Ctrl+C ended the wait, which closes the window.
fn open_without_waiting(ps: &Pipeline<'_>, session: Session, mut window: Window, built: Built, title: String, shortcuts: HashMap<String, PsScriptBlock>, handling: Handling<'_>) -> PsResult<Option<GuiWindowInfo>> {
    let Some(before) = session::first_frame(ps, &mut window)? else {
        return Ok(None);
    };
    let id = window.id();
    let names = built.tree.names();
    let mut results = BTreeMap::new();
    for (node, r) in &built.results {
        results.insert(*node, to_value(r)?);
    }
    let handlers: HashMap<(NodeId, EventKind), PsObject> = built.handlers.into_iter().map(|(k, h)| (k, h.0)).collect();
    let shortcuts: HashMap<String, PsObject> = shortcuts.into_iter().map(|(s, h)| (s, h.0)).collect();
    let on_close = handling.on_close.map(|h| h.0.clone());
    let on_file_change = handling.on_file_change.map(|h| h.0.clone());
    let raised = if handlers.is_empty() && shortcuts.is_empty() && on_close.is_none() && on_file_change.is_none() {
        None
    } else {
        let source = format!("PwrsGui.Widget.{}.{id}", session.id());
        ps.invoke("Register-EngineEvent", &[("SourceIdentifier", source.clone().into_ps()?), ("Action", script(ACTION)?.0)])?;
        Some((ps.events()?, source))
    };
    let source_identifier = raised.as_ref().map(|(_, s)| s.clone());
    let inputs = inputs_for_actions(id, &built.tree)?;
    let trees = Trees { specs: built.trees, items: built.items, next: built.next };
    let open = Arc::new(Open::new(session, built.tree, built.tables, trees, built.canvases, true, handling.files));
    register(id, &open);
    let reader = Reader { id, window: Some(window), before, open, handlers, shortcuts, results, on_close, on_file_change, output: handling.output, raised, inputs };
    std::thread::Builder::new()
        .name(format!("pwrs-gui-widgets-{id}"))
        .spawn(move || reader.run())
        .map_err(|e| PsError::new(ErrorCategory::ResourceUnavailable, "GuiThread", format!("the window's reader thread cannot start: {e}")).terminating())?;
    Ok(Some(GuiWindowInfo { id, title, names, source_identifier }))
}

/// A named input whose value in the JSON a -NoWait window's handler action
/// turns into the one PowerShell is given.
enum Typed {
    /// A table, whose rows become its objects.
    Table { node: NodeId, multiple: bool },
    /// A tree, whose items become the objects they were made from, which
    /// the window stores as its tree grows.
    Tree { node: NodeId, multiple: bool },
    /// A number field or slider, whose number becomes a double.
    Number,
    /// A password field, whose text becomes a SecureString.
    Secret,
}

/// For a -NoWait window's handler actions, which run on the pipeline
/// thread, what turns each named input's value in the JSON into the one
/// PowerShell is given, as a hashtable by name: `@{ Kind = 'Table'; Window;
/// Node; Multiple }` for a table and `@{ Kind = 'Tree'; Window; Node;
/// Multiple }` for a tree of window `window`, whose rows and items the
/// window gives as it has them then, `@{ Kind = 'Number' }` for a number
/// field or slider, `@{ Kind = 'Secret' }` for a password field; null when
/// the window has none of them. Made here, on the pipeline thread.
fn inputs_for_actions(window: WindowId, tree: &Node) -> PsResult<PsObject> {
    let mut named = Vec::new();
    collect_typed(tree, &mut named);
    if named.is_empty() {
        return Ok(PsObject::null());
    }
    let table = PsType::from_name("System.Collections.Hashtable").new(&[])?;
    for (name, typed) in named {
        let entry = match typed {
            Typed::Table { node, multiple } => vec![
                (String::from("Kind"), "Table".into_ps()?),
                (String::from("Window"), window.into_ps()?),
                (String::from("Node"), node.into_ps()?),
                (String::from("Multiple"), multiple.into_ps()?),
            ],
            Typed::Tree { node, multiple } => vec![
                (String::from("Kind"), "Tree".into_ps()?),
                (String::from("Window"), window.into_ps()?),
                (String::from("Node"), node.into_ps()?),
                (String::from("Multiple"), multiple.into_ps()?),
            ],
            Typed::Number => vec![(String::from("Kind"), "Number".into_ps()?)],
            Typed::Secret => vec![(String::from("Kind"), "Secret".into_ps()?)],
        };
        table.call("Add", &[name.into_ps()?, crate::values::ordered(entry)?])?;
    }
    Ok(table)
}

fn collect_typed(node: &Node, out: &mut Vec<(String, Typed)>) {
    if let Some(name) = &node.name {
        let typed = match node.kind {
            Kind::Table => Some(Typed::Table { node: node.id, multiple: matches!(node.props.get("selection"), Some(Value::Str(s)) if s == "multiple") }),
            Kind::TreeView => Some(Typed::Tree { node: node.id, multiple: matches!(node.props.get("multiple"), Some(Value::Bool(true))) }),
            Kind::NumberBox | Kind::Slider => Some(Typed::Number),
            Kind::TextBox if is_secret(node) => Some(Typed::Secret),
            _ => None,
        };
        if let Some(t) = typed {
            out.push((name.clone(), t));
        }
    }
    for c in &node.children {
        collect_typed(c, out);
    }
}

/// The reader of a window opened with -NoWait.
struct Reader {
    id: WindowId,
    window: Option<Window>,
    /// The events that came before the window's first frame, which it
    /// reads first.
    before: Vec<WindowEvent>,
    open: Arc<Open>,
    handlers: HashMap<(NodeId, EventKind), PsObject>,
    /// The handler of each of the window's own shortcuts, by the shortcut.
    shortcuts: HashMap<String, PsObject>,
    results: BTreeMap<NodeId, Value>,
    on_close: Option<PsObject>,
    on_file_change: Option<PsObject>,
    output: GuiHandlerOutput,
    raised: Option<(PsEvents, String)>,
    /// What [`inputs_for_actions`] made, which each raised event has.
    inputs: PsObject,
}

impl Reader {
    /// Reads the window's events until it closes, then queues its Close.
    fn run(mut self) {
        let result = match self.window.take() {
            Some(mut w) => {
                let before = std::mem::take(&mut self.before);
                let result = self.read(&mut w, before);
                if let Err(e) = w.close() {
                    eprintln!("PWRS-GUI: window {} could not be closed: {e}", self.id);
                }
                result
            }
            None => Value::Str(String::from("Closed")),
        };
        self.open.closed.store(true, Ordering::Release);
        self.open.surfaces.store(Arc::new(BTreeMap::new()));
        crate::gpu::release_window(self.id);
        let state = match self.open.presented_state() {
            Ok(s) => s,
            Err(e) => {
                eprintln!("PWRS-GUI: window {}'s last state cannot be read: {e}", self.id);
                Vec::new()
            }
        };
        let close = Queued { node: None, name: None, kind: GuiEventKind::Close, value: result, state };
        self.open.push(close.clone());
        if let Some(h) = self.on_close.clone() {
            self.raise(&h, &close);
        }
    }

    /// The window's result, once it has closed, read from `before`, the
    /// events that came before its first frame, then from the window.
    fn read(&self, window: &mut Window, before: Vec<WindowEvent>) -> Value {
        let mut before = before.into_iter();
        loop {
            let next = match before.next() {
                Some(event) => Ok(event),
                None => window.next_event(),
            };
            match next {
                Ok(WindowEvent::Event { node, kind, value, .. }) => {
                    self.open.record(node, kind, &value);
                    match self.open.queued(node, kind, &value) {
                        Ok(q) => {
                            self.open.push(q.clone());
                            let shortcut = match (&kind, &value) {
                                (EventKind::Shortcut, Value::Str(s)) => self.shortcuts.get(s),
                                _ => None,
                            };
                            if let Some(h) = self.handlers.get(&(node, kind)).or(shortcut).cloned() {
                                self.raise(&h, &q);
                            }
                        }
                        Err(e) => eprintln!("PWRS-GUI: window {}'s event from node {node} cannot be read: {e}", self.id),
                    }
                    if kind == EventKind::Click
                        && let Some(r) = self.results.get(&node)
                    {
                        return r.clone();
                    }
                }
                Ok(WindowEvent::Closed(_) | WindowEvent::PageClosed(_)) => {
                    return match self.open.closing.load_full() {
                        Some(v) => (*v).clone(),
                        None => Value::Str(String::from("Closed")),
                    };
                }
                Ok(WindowEvent::Problem(p)) => eprintln!("PWRS-GUI: window {} reported: {p}", self.id),
                Ok(WindowEvent::Message { .. } | WindowEvent::Opened) => {}
                Ok(WindowEvent::Call { name, .. }) => eprintln!("PWRS-GUI: window {}, which shows no page, got a page's call of {name}", self.id),
                Ok(WindowEvent::Open { json, .. }) => eprintln!("PWRS-GUI: window {}, which shows no page, got a page's open of a window: {json}", self.id),
                Ok(WindowEvent::PickerWanted { node, title, kind }) => {
                    let id = self.id;
                    let drawing = std::thread::Builder::new().name(String::from("pwrs-gui-chooser")).spawn(move || {
                        let outcome = crate::dialogs::draw_for_chooser_unattended(&title, kind);
                        match session::running_of(pwrs_gui_client::Renderer::of_number(id)) {
                            Some(s) => {
                                if let Err(e) = s.answer_picker(id, node, outcome) {
                                    eprintln!("PWRS-GUI: window {id}'s file chooser could not be given its dialog's answer: {e}");
                                }
                            }
                            None => eprintln!("PWRS-GUI: window {id}'s renderer ended before its file chooser's dialog was answered"),
                        }
                    });
                    if let Err(e) = drawing {
                        eprintln!("PWRS-GUI: window {}'s file chooser's dialog could not be drawn: {e}", self.id);
                    }
                }
                Ok(WindowEvent::FileChanged { file, length, .. }) => self.file_changed(file, length),
                Err(e) => {
                    eprintln!("PWRS-GUI: window {} stopped reading its events: {e}", self.id);
                    return Value::Str(String::from("Closed"));
                }
            }
        }
    }

    /// Queues a change to watched file `file`, unless one waits there
    /// untaken, and raises -OnFileChange with it.
    fn file_changed(&self, file: usize, length: Option<u64>) {
        let Some((path, waiting)) = self.open.files.get(file) else {
            eprintln!("PWRS-GUI: window {} was told of a change to file {file}, which it does not watch", self.id);
            return;
        };
        let state = match self.open.presented_state() {
            Ok(s) => s,
            Err(e) => {
                eprintln!("PWRS-GUI: window {}'s state cannot be read for a change to {path}: {e}", self.id);
                Vec::new()
            }
        };
        let changed = Queued { node: None, name: Some(path.clone()), kind: GuiEventKind::FileChange, value: length_value(length), state };
        if !waiting.swap(true, Ordering::AcqRel) {
            self.open.push(changed.clone());
        }
        if let Some(h) = self.on_file_change.clone() {
            self.raise(&h, &changed);
        }
    }

    /// Raises the engine event that runs `handler` with `event`.
    fn raise(&self, handler: &PsObject, event: &Queued) {
        let Some((events, source)) = &self.raised else {
            return;
        };
        let state: serde_json::Map<String, serde_json::Value> = event.state.iter().map(|(k, _, v)| (k.clone(), json_of(v))).collect();
        let data = GuiWidgetEventData {
            window: self.id,
            node: event.node,
            name: event.name.clone(),
            kind: event.kind,
            value_json: json_of(&event.value).to_string(),
            state_json: serde_json::Value::Object(state).to_string(),
            handler: handler.clone(),
            output: self.output,
            inputs: self.inputs.clone(),
        };
        if let Err(e) = events.raise(source, data) {
            eprintln!("PWRS-GUI: window {} could not raise {source}: {e}", self.id);
        }
    }
}

/// The window an update command acts on: the one -Window names, else the
/// window of the handler it runs in.
pub(crate) fn window_of(ps: &Pipeline<'_>, given: Option<u64>) -> PsResult<WindowId> {
    if let Some(w) = given {
        return Ok(w);
    }
    let inside = ps.variable("PwrsGuiWindow")?;
    if inside.is_null() {
        let text = "give -Window: this runs outside a window's handler, where it would act on the handler's window";
        return Err(PsError::new(ErrorCategory::InvalidArgument, "GuiNoWindow", text).terminating());
    }
    u64::from_ps(&inside)
}

/// The widget named `name` in the window, or an error naming the window's
/// widgets.
fn widget_named(tree: &Node, id: WindowId, name: &str) -> PsResult<Node> {
    match tree.find_named(name) {
        Some(n) => Ok(n.clone()),
        None => {
            let text = format!("window {id} has no widget named {name}; its widgets are {}", tree.names().join(", "));
            Err(PsError::new(ErrorCategory::ObjectNotFound, "GuiNoWidget", text).terminating())
        }
    }
}

/// The GpuSurface named `name` in window `id`: the window as the module
/// stores it, and the surface's node.
pub(crate) fn surface_named(id: WindowId, name: &str) -> PsResult<(Arc<Open>, NodeId)> {
    let open = open_window(id)?;
    let node = widget_named(&open.tree.load(), id, name)?;
    if node.kind != Kind::GpuSurface {
        let text = format!("{name} in window {id} is a {}, not a GpuSurface", widgets::widget_name(node.kind));
        return Err(PsError::new(ErrorCategory::InvalidArgument, "GuiNotASurface", text).terminating());
    }
    Ok((open, node.id))
}

/// The node of the Canvas named `name` in window `id`, whose view a
/// command acts on.
pub(crate) fn view_canvas(id: WindowId, name: &str) -> PsResult<NodeId> {
    let open = open_window(id)?;
    let node = widget_named(&open.tree.load(), id, name)?;
    if node.kind != Kind::Canvas {
        let text = format!("{name} in window {id} is a {}, not a Canvas", widgets::widget_name(node.kind));
        return Err(PsError::new(ErrorCategory::InvalidArgument, "GuiNotACanvas", text).terminating());
    }
    Ok(node.id)
}

/// Changes properties of a widget in an open window described in
/// PowerShell, found by its name. The properties are those of the widget's
/// New-Gui<Widget> command, under its parameters' names. With -Item it
/// changes one item of a canvas instead, found by its name at any depth:
/// the properties are the item's keys, a null taking one away, and an
/// Animate among them starts its animations. Inside a handler it acts on
/// the handler's window; elsewhere -Window names the window.
///
/// # Examples
/// Update-GuiWidget Status @{ Text = 'Done' }
/// Update-GuiWidget -Window 3 -Name Ok -Property @{ Enabled = $true }
/// Update-GuiWidget Board -Item Box -Property @{ X = 120; Fill = 'Gold' }
#[cmdlet(verb = "Update", noun = "GuiWidget")]
#[derive(Default)]
pub struct UpdateGuiWidget {
    /// The widget's name, compared without regard to case.
    #[param(mandatory, position = 0)]
    pub name: String,
    /// The properties to set, by the names the widget's New-Gui<Widget>
    /// parameters give them; with -Item, the item's keys.
    #[param(mandatory, position = 1)]
    pub property: PsObject,
    /// For a canvas, the name of the item to change, compared without
    /// regard to case.
    #[param]
    pub item: Option<String>,
    /// The window's number; inside a handler, the handler's window when not
    /// given.
    #[param(value_from_pipeline_by_property_name, alias = ["Id"])]
    pub window: Option<u64>,
}

impl Cmdlet for UpdateGuiWidget {
    fn process(&mut self, ps: &Pipeline<'_>) -> PsResult<()> {
        let id = window_of(ps, self.window)?;
        match &self.item {
            Some(item) => set_canvas_item(ps, id, &self.name, item, &self.property),
            None => set_widget(ps, id, &self.name, &self.property),
        }
    }
}

/// The canvas named `name` in window `id`, for a command that changes its
/// items: the window as the module stores it, the canvas's node and its
/// items as the module stores them.
fn canvas_named(id: WindowId, name: &str) -> PsResult<(Arc<Open>, Node, Arc<Board>)> {
    let open = open_window(id)?;
    let node = widget_named(&open.tree.load(), id, name)?;
    if node.kind != Kind::Canvas {
        let text = format!("-Item names an item of a canvas, and {name} is a {}", widgets::widget_name(node.kind));
        return Err(PsError::new(ErrorCategory::InvalidArgument, "GuiNotACanvas", text).terminating());
    }
    let board = open.canvases.load().get(&node.id).cloned().ok_or_else(|| bad_state(format!("canvas {name} in window {id} has no items")))?;
    Ok((open, node, board))
}

/// Applies `change` to the board of the canvas `node` and stores the board
/// it leaves, unless it refuses; returns what it returned. The change is made
/// to the board as it stands when it is stored, so two changes at once both
/// take.
fn change_board<T>(open: &Open, node: NodeId, change: impl Fn(&mut Board) -> Result<T, String>) -> Result<T, String> {
    let mut outcome = None;
    open.canvases.rcu(|boards| {
        let mut boards = (**boards).clone();
        let result = match boards.get(&node) {
            Some(held) => {
                let mut board = (**held).clone();
                let result = change(&mut board);
                if result.is_ok() {
                    boards.insert(node, Arc::new(board));
                }
                result
            }
            None => Err(format!("canvas {node} has no items")),
        };
        outcome = Some(result);
        boards
    });
    match outcome {
        Some(result) => result,
        None => Err(format!("canvas {node}'s items were not reached")),
    }
}

/// A canvas's refusal of a change to its items, naming the canvas: an
/// item it does not have is not found, and any other is a bad argument.
fn item_refused(canvas: &str, why: String) -> PsError {
    let (category, id) = if why.starts_with("no item is named") { (ErrorCategory::ObjectNotFound, "GuiNoItem") } else { (ErrorCategory::InvalidArgument, "GuiBadItem") };
    PsError::new(category, id, format!("{canvas}: {why}")).terminating()
}

/// Changes the item named `item` of the canvas named `name` in window
/// `id`, as Update-GuiWidget -Item does: the canvas as the module stores it
/// takes the change, which must leave items the canvas reads, and the
/// renderer is sent it.
fn set_canvas_item(ps: &Pipeline<'_>, id: WindowId, name: &str, item: &str, property: &PsObject) -> PsResult<()> {
    let (open, node, board) = canvas_named(id, name)?;
    let props = crate::canvas::changes(ps, &board, item, property).map_err(|e| item_refused(name, e))?;
    change_board(&open, node.id, |b| b.set(item, &props)).map_err(|e| item_refused(name, e))?;
    open.session.patch(id, vec![PatchOp::SetItem { node: node.id, item: item.to_string(), props }]).map_err(gui_error)
}

/// Adds the items `given` makes to the canvas `canvas`, named `name`, of
/// window `id`, above its own or with `group` above that Group's, as
/// Add-GuiItem does: the canvas as the module stores it takes them, which
/// must leave items it reads, and the renderer is sent them.
fn add_canvas_items(ps: &Pipeline<'_>, open: &Open, id: WindowId, canvas: &Node, name: &str, group: Option<&str>, given: &[PsObject]) -> PsResult<()> {
    let added = crate::canvas::added(ps, given).map_err(|e| item_refused(name, e))?;
    change_board(open, canvas.id, |b| b.add(group, &added)).map_err(|e| item_refused(name, e))?;
    open.session.patch(id, vec![PatchOp::AddItems { node: canvas.id, group: group.map(str::to_string), items: added }]).map_err(gui_error)
}

/// Takes the items `names` name, at any depth, out of the canvas `canvas`,
/// named `name`, of window `id`, as Remove-GuiItem does: a name no item
/// has takes none away.
fn remove_canvas_items(open: &Open, id: WindowId, canvas: &Node, name: &str, names: Vec<String>) -> PsResult<()> {
    change_board(open, canvas.id, |b| b.remove(&names)).map_err(|e| item_refused(name, e))?;
    open.session.patch(id, vec![PatchOp::RemoveItems { node: canvas.id, names }]).map_err(gui_error)
}

/// Sets properties of the widget named `name` in window `id`, under the
/// names its New-Gui<Widget> parameters give them, as Update-GuiWidget
/// does.
pub(crate) fn set_widget(ps: &Pipeline<'_>, id: WindowId, name: &str, property: &PsObject) -> PsResult<()> {
    let open = open_window(id)?;
    let node = widget_named(&open.tree.load(), id, name)?;
    let items = widgets::choices_of(&node)?;
    let mut props = widgets::wire_props(ps, node.kind, property, &items, false)?;
    if let Some(Value::List(given)) = props.get("items")
        && matches!(node.kind, Kind::ListBox | Kind::ComboBox | Kind::RadioGroup)
        && !props.contains_key("selected")
    {
        let selected = reselected(&open, &node, &items, given)?;
        props.insert(String::from("selected"), selected);
    }
    widgets::settle(node.kind, &node.props, open.state.load().get(&node.id), &mut props)?;
    let board = match props.get("items") {
        Some(given) if node.kind == Kind::Canvas => Some(Arc::new(Board::of_items(given).map_err(|e| item_refused(name, e))?)),
        _ => None,
    };
    open.session.patch(id, vec![PatchOp::SetProps { node: node.id, props: props.clone() }]).map_err(gui_error)?;
    if let Some(board) = board {
        open.canvases.rcu(|boards| {
            let mut boards = (**boards).clone();
            boards.insert(node.id, Arc::clone(&board));
            boards
        });
        props.remove("items");
    }
    open.apply(node.id, &props);
    Ok(())
}

/// The rows a list, drop-down or option group has chosen, from its value:
/// a list of rows, one row, or null for none.
fn selected_rows(open: &Open, node: &Node) -> PsResult<BTreeSet<usize>> {
    let row = |r: &Value| -> PsResult<usize> {
        let i = r.as_int().ok_or_else(|| bad_state(format!("{} has a row that is not a number: {r:?}", node.id)))?;
        usize::try_from(i).map_err(|e| bad_state(format!("{} has row {i}: {e}", node.id)))
    };
    match open.state.load().get(&node.id) {
        Some(Value::List(rows)) => rows.iter().map(row).collect(),
        Some(Value::Null) | None => Ok(BTreeSet::new()),
        Some(one) => Ok(BTreeSet::from([row(one)?])),
    }
}

/// Rows as a widget of `kind` lists them in `selected`: a list of them for
/// a ListBox, the one row, or null for none, for a drop-down or option
/// group.
fn selection_value(kind: Kind, rows: &[usize]) -> PsResult<Value> {
    let int = |r: usize| i64::try_from(r).map(Value::Int).map_err(|e| bad_state(format!("row {r} does not fit the protocol: {e}")));
    if kind == Kind::ListBox {
        return Ok(Value::List(rows.iter().map(|r| int(*r)).collect::<PsResult<Vec<Value>>>()?));
    }
    match rows.first() {
        Some(r) => int(*r),
        None => Ok(Value::Null),
    }
}

/// What a list, drop-down or option group has chosen once Update-GuiWidget
/// gives it `new` items in place of `old`: the chosen items it still has,
/// found by their text without regard to case.
fn reselected(open: &Open, node: &Node, old: &[String], new: &[Value]) -> PsResult<Value> {
    let mut rows = Vec::new();
    let mut taken = BTreeSet::new();
    for r in selected_rows(open, node)? {
        let text = old.get(r).ok_or_else(|| bad_state(format!("{} has row {r} of {} items", node.id, old.len())))?;
        let at = (0..new.len()).find(|i| !taken.contains(i) && new[*i].as_str().is_some_and(|s| s.eq_ignore_ascii_case(text)));
        if let Some(at) = at {
            taken.insert(at);
            rows.push(at);
        }
    }
    rows.sort_unstable();
    selection_value(node.kind, &rows)
}

/// A list's, drop-down's or option group's items and what it has chosen
/// after a change to its items: `keep` says which of the current items
/// stay, and `added` follow them. A chosen item that stays stays chosen.
fn relisted(open: &Open, node: &Node, keep: impl Fn(usize, &str) -> bool, added: &[String]) -> PsResult<BTreeMap<String, Value>> {
    let items = widgets::list_items(node)?;
    let selected = selected_rows(open, node)?;
    let mut kept = Vec::new();
    let mut rows = Vec::new();
    for (i, item) in items.iter().enumerate() {
        if keep(i, item) {
            if selected.contains(&i) {
                rows.push(kept.len());
            }
            kept.push(Value::Str(item.clone()));
        }
    }
    kept.extend(added.iter().cloned().map(Value::Str));
    Ok(BTreeMap::from([(String::from("items"), Value::List(kept)), (String::from("selected"), selection_value(node.kind, &rows)?)]))
}

/// The widget named `name` in the window that lists items, for an item
/// command.
fn list_named(ps: &Pipeline<'_>, window: Option<u64>, name: &str, verb: &str) -> PsResult<(WindowId, Arc<Open>, Node)> {
    let id = window_of(ps, window)?;
    let open = open_window(id)?;
    let node = widget_named(&open.tree.load(), id, name)?;
    if !widgets::lists_items(node.kind) {
        let text = format!("{name} is a {}, which has no items to {verb}; a ListBox, ComboBox, RadioGroup, LogView, Chart, Table, TreeView or Canvas has", widgets::widget_name(node.kind));
        return Err(PsError::new(ErrorCategory::InvalidArgument, "GuiNoItems", text).terminating());
    }
    Ok((id, open, node))
}

/// The items of the tree `tree`, named `name`, made from each of `wanted`,
/// as Add-GuiItem -Parent and Remove-GuiItem -Item find a node: the same
/// object, or text or a number of the same value. An object no node was
/// made from, and one two or more were, is an error naming the tree and
/// the nodes.
fn tree_items_made_from(open: &Open, tree: &Node, name: &str, wanted: &[PsObject]) -> PsResult<Vec<NodeId>> {
    let items = open.items.load();
    let found = widgets::tree_items_of(&items, tree.id, wanted)?;
    let shown = |id: &NodeId| match items.get(id) {
        Some(item) => match item.parent.and_then(|p| items.get(&p)) {
            Some(parent) => format!("{} under {}", item.text, parent.text),
            None => item.text.clone(),
        },
        None => id.to_string(),
    };
    let mut out = Vec::with_capacity(found.len());
    for (w, ids) in wanted.iter().zip(found) {
        match ids.as_slice() {
            [one] => out.push(*one),
            [] => {
                let text = format!("{name} has no node made from {}", String::from_ps(w)?);
                return Err(PsError::new(ErrorCategory::ObjectNotFound, "GuiNoNode", text).terminating());
            }
            many => {
                let nodes: Vec<String> = many.iter().map(shown).collect();
                let text = format!("{} nodes of {name} are made from {}: {}; name one by the object it was made from", many.len(), String::from_ps(w)?, nodes.join(", "));
                return Err(PsError::new(ErrorCategory::InvalidArgument, "GuiTwoNodes", text).terminating());
            }
        }
    }
    Ok(out)
}

/// Adds items to a widget that lists them, after the items it has; a
/// ListBox, ComboBox or RadioGroup leaves its choice as it was, and a LogView
/// takes them as lines, which reach it without the lines it has. A Chart
/// takes points, each a hashtable of the Series it joins, or starts, its Y,
/// and its X, a number or a date; a point without X takes its series' count
/// of points, and on a chart of dates each point gives its date. A Table
/// takes objects, a row each, their cells from the table's columns. A
/// TreeView takes nodes in the forms New-GuiTreeView's -Node takes, under
/// the node -Parent names, or at its top. A Canvas takes items, hashtables
/// as New-GuiCanvas's -Item takes, drawn above those it has, or with
/// -Parent, the name of a Group, above that group's own; a name an item
/// already has is refused. Inside a handler it acts on the handler's
/// window; elsewhere -Window names the window.
///
/// # Examples
/// Add-GuiItem Servers 'web03', 'web04'
/// Add-GuiItem -Window 3 -Name Log -Item 'started'
/// Add-GuiItem Load @{ Series = 'cpu'; X = (Get-Date); Y = 42 }
/// Add-GuiItem Food 'Kiwi' -Parent $fruit
/// Add-GuiItem Board @{ Kind = 'Ellipse'; Name = 'Dot'; X = 10; Y = 10; Width = 8; Height = 8 } -Parent Layer
#[cmdlet(verb = "Add", noun = "GuiItem")]
#[derive(Default)]
pub struct AddGuiItem {
    /// The widget's name, compared without regard to case.
    #[param(mandatory, position = 0)]
    pub name: String,
    /// The items to add: text for a list, a drop-down, an option group or a
    /// log, hashtables of Series, X and Y for a chart's points, the objects
    /// of a table's rows, for a tree its nodes, in the forms
    /// New-GuiTreeView's -Node takes, and for a canvas hashtables of each
    /// item's Kind, Name and keys.
    #[param(mandatory, position = 1, allow_empty_collection)]
    pub item: Vec<PsObject>,
    /// For a tree, the node the items go under, named by the object it was
    /// made from: the same object, or text or a number of the same value;
    /// the top of the tree when not given. For a canvas, the name of the
    /// Group the items go into; the canvas's own items when not given.
    #[param]
    pub parent: Option<PsObject>,
    /// The window's number; inside a handler, the handler's window when not
    /// given.
    #[param(value_from_pipeline_by_property_name, alias = ["Id"])]
    pub window: Option<u64>,
}

impl Cmdlet for AddGuiItem {
    fn process(&mut self, ps: &Pipeline<'_>) -> PsResult<()> {
        let (id, open, node) = list_named(ps, self.window, &self.name, "add")?;
        let parent = self.parent.as_ref().filter(|p| !p.is_null());
        if node.kind == Kind::TreeView {
            let under = match parent {
                Some(p) => tree_items_made_from(&open, &node, &self.name, std::slice::from_ref(p))?.first().copied(),
                None => None,
            };
            return add_tree_items(&open, id, node.id, under, &self.item);
        }
        if node.kind == Kind::Canvas {
            let group = parent.map(String::from_ps).transpose()?;
            return add_canvas_items(ps, &open, id, &node, &self.name, group.as_deref(), &self.item);
        }
        if parent.is_some() {
            let text = format!("-Parent names a node of a tree or a Group of a canvas, and {} is a {}, whose items have none", self.name, widgets::widget_name(node.kind));
            return Err(PsError::new(ErrorCategory::InvalidArgument, "GuiNoParent", text).terminating());
        }
        if node.kind == Kind::Table {
            return add_table_rows(ps, &open, id, &node, &self.item);
        }
        if node.kind == Kind::Chart {
            let had = node.props.get("dates").and_then(Value::as_bool);
            let mut dates = had;
            let values = widgets::chart_points(&self.item, &mut dates).map_err(|e| PsError::new(ErrorCategory::InvalidArgument, "GuiBadPoint", format!("{}: {e}", self.name)).terminating())?;
            let append = PatchOp::Append { node: node.id, prop: String::from("points"), values };
            let Some(settled) = dates.filter(|_| had.is_none()) else {
                return open.session.patch(id, vec![append]).map_err(gui_error);
            };
            let props = BTreeMap::from([(String::from("dates"), Value::Bool(settled))]);
            open.session.patch(id, vec![PatchOp::SetProps { node: node.id, props: props.clone() }, append]).map_err(gui_error)?;
            open.apply(node.id, &props);
            return Ok(());
        }
        let texts = self.item.iter().map(String::from_ps).collect::<PsResult<Vec<String>>>()?;
        if node.kind == Kind::LogView {
            let values = texts.into_iter().map(Value::Str).collect();
            return open.session.patch(id, vec![PatchOp::Append { node: node.id, prop: String::from("lines"), values }]).map_err(gui_error);
        }
        let props = relisted(&open, &node, |_, _| true, &texts)?;
        open.session.patch(id, vec![PatchOp::SetProps { node: node.id, props: props.clone() }]).map_err(gui_error)?;
        open.apply(node.id, &props);
        Ok(())
    }
}

/// Removes items from a widget that lists them, each named by its text
/// without regard to case; a ListBox, ComboBox or RadioGroup leaves chosen
/// the items that stay. A Table's rows and a TreeView's nodes are named by
/// the objects they were made from; a table leaves selected the rows that
/// stay, and a node goes with the nodes
/// under it. A Canvas's items are named by their names, at any depth, a
/// Group going with its items.
/// An item the widget does not have is an error naming it. Inside a handler
/// it acts on the handler's window; elsewhere -Window names the window.
///
/// # Examples
/// Remove-GuiItem Servers 'web03'
/// Remove-GuiItem Folders $e.Value
/// Remove-GuiItem Board 'Dot', 'Layer'
#[cmdlet(verb = "Remove", noun = "GuiItem")]
#[derive(Default)]
pub struct RemoveGuiItem {
    /// The widget's name, compared without regard to case.
    #[param(mandatory, position = 0)]
    pub name: String,
    /// The items to remove, by their text; a table's rows and a tree's
    /// nodes by the objects they were made from: the same object, or text
    /// or a number of the same value; a canvas's items by their names.
    #[param(mandatory, position = 1)]
    pub item: Vec<PsObject>,
    /// The window's number; inside a handler, the handler's window when not
    /// given.
    #[param(value_from_pipeline_by_property_name, alias = ["Id"])]
    pub window: Option<u64>,
}

impl Cmdlet for RemoveGuiItem {
    fn process(&mut self, ps: &Pipeline<'_>) -> PsResult<()> {
        let (id, open, node) = list_named(ps, self.window, &self.name, "remove")?;
        if node.kind == Kind::LogView {
            let text = format!("{} is a LogView, whose lines Add-GuiItem adds and Update-GuiWidget -Property @{{ Line = @() }} clears; it takes none away one by one", self.name);
            return Err(PsError::new(ErrorCategory::InvalidArgument, "GuiNoItems", text).terminating());
        }
        if node.kind == Kind::Chart {
            let text = format!("{} is a Chart, whose points Add-GuiItem adds and Update-GuiWidget -Property @{{ Series = ... }} replaces; it takes none away one by one", self.name);
            return Err(PsError::new(ErrorCategory::InvalidArgument, "GuiNoItems", text).terminating());
        }
        if node.kind == Kind::Table {
            return remove_table_rows(&open, id, &node, &self.name, &self.item);
        }
        if node.kind == Kind::TreeView {
            return remove_tree_items(&open, id, &node, &self.name, &self.item);
        }
        let wanted = self.item.iter().map(String::from_ps).collect::<PsResult<Vec<String>>>()?;
        if node.kind == Kind::Canvas {
            return remove_canvas_items(&open, id, &node, &self.name, wanted);
        }
        let items = widgets::list_items(&node)?;
        let mut gone = BTreeSet::new();
        for wanted in &wanted {
            match items.iter().enumerate().position(|(i, it)| !gone.contains(&i) && it.eq_ignore_ascii_case(wanted)) {
                Some(i) => {
                    gone.insert(i);
                }
                None => {
                    let text = format!("{} has no item {wanted}; its items are {}", self.name, items.join(", "));
                    return Err(PsError::new(ErrorCategory::ObjectNotFound, "GuiNoItem", text).terminating());
                }
            }
        }
        let props = relisted(&open, &node, |i, _| !gone.contains(&i), &[])?;
        open.session.patch(id, vec![PatchOp::SetProps { node: node.id, props: props.clone() }]).map_err(gui_error)?;
        open.apply(node.id, &props);
        Ok(())
    }
}

/// Adds rows made from `objects`, nulls passed over, after the rows of the
/// table `table` of window `id`: their cells from the columns it opened
/// with, or, for a table that opened with no object and no -Property, from
/// the first object's, which it shows from then on. Every way a table grows
/// comes through here.
fn add_table_rows(ps: &Pipeline<'_>, open: &Open, id: WindowId, table: &Node, objects: &[PsObject]) -> PsResult<()> {
    let objects: Vec<PsObject> = objects.iter().filter(|o| !o.is_null()).cloned().collect();
    if objects.is_empty() {
        return Ok(());
    }
    let had = open.tables.load().get(&table.id).cloned().ok_or_else(|| bad_state(format!("window {id} has no table {}", table.id)))?;
    let fresh = had.columns.to_come();
    let columns = if fresh { Arc::new(widgets::table_columns(ps, objects.first(), None)?) } else { Arc::clone(&had.columns) };
    let cells = widgets::table_cells(ps, &objects, &columns)?;
    let keys = widgets::item_keys(&objects)?;
    let mut start = had.count();
    open.tables.rcu(|tables| {
        let mut tables = (**tables).clone();
        if let Some(now) = tables.get(&table.id) {
            let mut grown = (**now).clone();
            start = grown.count();
            if fresh {
                grown.columns = Arc::clone(&columns);
            }
            grown.push(objects.clone(), keys.clone());
            tables.insert(table.id, Arc::new(grown));
        }
        tables
    });
    if fresh {
        let props = BTreeMap::from([(String::from("columns"), columns.value())]);
        open.session.patch(id, vec![PatchOp::SetProps { node: table.id, props: props.clone() }]).map_err(gui_error)?;
        open.apply(table.id, &props);
    }
    let start = u64::try_from(start).map_err(|e| bad_state(format!("row {start}: {e}")))?;
    open.session.rows(id, table.id, start, cells, true).map_err(gui_error)
}

/// Removes from the table `table`, named `name`, of window `id` the rows
/// made from each of `wanted`, found as a tree's nodes are: the same
/// object, or text or a number of the same value. An object no row was
/// made from, and one two or more were, is an error naming the table and
/// the rows. The table leaves selected the rows that stay.
fn remove_table_rows(open: &Open, id: WindowId, table: &Node, name: &str, wanted: &[PsObject]) -> PsResult<()> {
    let rows = open.tables.load().get(&table.id).cloned().ok_or_else(|| bad_state(format!("window {id} has no table {}", table.id)))?;
    let mut gone = BTreeSet::new();
    for (w, numbers) in wanted.iter().zip(rows.made_from(wanted)?) {
        match numbers.as_slice() {
            [one] => {
                gone.insert(*one);
            }
            [] => {
                let text = format!("{name} has no row made from {}", String::from_ps(w)?);
                return Err(PsError::new(ErrorCategory::ObjectNotFound, "GuiNoRow", text).terminating());
            }
            many => {
                let places: Vec<String> = rows.places(&many.iter().copied().collect()).iter().map(u64::to_string).collect();
                let text = format!("{} rows of {name} are made from {}: rows {}; name one by the object it was made from", many.len(), String::from_ps(w)?, places.join(", "));
                return Err(PsError::new(ErrorCategory::InvalidArgument, "GuiTwoRows", text).terminating());
            }
        }
    }
    let mut places = Vec::new();
    let mut before = 0;
    open.tables.rcu(|tables| {
        let mut tables = (**tables).clone();
        if let Some(now) = tables.get(&table.id) {
            places = now.places(&gone);
            before = now.count();
            let kept = now.without(&gone);
            tables.insert(table.id, Arc::new(kept));
        }
        tables
    });
    if places.is_empty() {
        return Ok(());
    }
    open.session.patch(id, vec![PatchOp::RemoveRows { node: table.id, rows: places.clone() }]).map_err(gui_error)?;
    if let Some(Value::List(selected)) = open.state.load().get(&table.id) {
        let mut all: Vec<usize> = (0..before).collect();
        let moved = pwrs_gui_client::proto::remove_rows(&mut all, &places);
        let mut kept = Vec::with_capacity(selected.len());
        for v in selected {
            let r = v.as_int().ok_or_else(|| bad_state(format!("table {} has a selection that is not a row: {v:?}", table.id)))?;
            let r = usize::try_from(r).map_err(|e| bad_state(format!("table {} has row {r}: {e}", table.id)))?;
            if let Some(Some(to)) = moved.get(r) {
                kept.push(Value::Int(i64::try_from(*to).map_err(|e| bad_state(format!("row {to}: {e}")))?));
            }
        }
        open.set_value(table.id, Value::List(kept));
    }
    Ok(())
}

/// Removes from the tree `tree`, named `name`, the nodes made from each of
/// `wanted` and the nodes under them: a node under another being removed
/// goes with it. The tree leaves selected the nodes that stay.
fn remove_tree_items(open: &Open, id: WindowId, tree: &Node, name: &str, wanted: &[PsObject]) -> PsResult<()> {
    let named: BTreeSet<NodeId> = tree_items_made_from(open, tree, name, wanted)?.into_iter().collect();
    let items = open.items.load();
    let gone = items.under(&named);
    let mut ops = Vec::new();
    for root in &named {
        let mut above = items.get(root).and_then(|i| i.parent);
        let mut within = false;
        while let Some(a) = above {
            if named.contains(&a) {
                within = true;
                break;
            }
            above = items.get(&a).and_then(|i| i.parent);
        }
        if !within {
            ops.push(PatchOp::Remove { node: *root });
        }
    }
    let selected = match open.state.load().get(&tree.id) {
        Some(Value::List(ids)) => ids.clone(),
        Some(Value::Null) | None => Vec::new(),
        Some(one) => vec![one.clone()],
    };
    let mut kept = Vec::with_capacity(selected.len());
    for v in &selected {
        let i = v.as_int().ok_or_else(|| bad_state(format!("tree {} has a selection that is not an item: {v:?}", tree.id)))?;
        let item = NodeId::try_from(i).map_err(|e| bad_state(format!("tree {} has item {i}: {e}", tree.id)))?;
        if !gone.contains(&item) {
            kept.push(v.clone());
        }
    }
    let reselect = kept.len() != selected.len();
    let props = BTreeMap::from([(String::from("selected"), Value::List(kept))]);
    if reselect {
        ops.push(PatchOp::SetProps { node: tree.id, props: props.clone() });
    }
    open.session.patch(id, ops).map_err(gui_error)?;
    if reselect {
        open.apply(tree.id, &props);
    }
    open.forget_items(&gone);
    Ok(())
}

/// Gives a widget of an open window described in PowerShell the keyboard
/// focus, found by its name, as a person's click or Tab gives it, so a
/// field takes the next keys typed. A Table's focus goes to its filter
/// field, and a RadioGroup's to the option chosen, or the first. Inside a
/// handler it acts on the handler's window; elsewhere -Window names the
/// window. A widget that takes no focus is an error that stops the
/// command; the renderer refuses a disabled widget, one on a tab that is
/// not shown, and a Canvas taking no keys with an error that does not.
///
/// # Examples
/// Set-GuiFocus Filter
/// Set-GuiFocus -Window 3 -Name Ok
#[cmdlet(verb = "Set", noun = "GuiFocus")]
#[derive(Default)]
pub struct SetGuiFocus {
    /// The widget's name, compared without regard to case.
    #[param(mandatory, position = 0)]
    pub name: String,
    /// The window's number; inside a handler, the handler's window when not
    /// given.
    #[param(value_from_pipeline_by_property_name, alias = ["Id"])]
    pub window: Option<u64>,
}

impl Cmdlet for SetGuiFocus {
    fn process(&mut self, ps: &Pipeline<'_>) -> PsResult<()> {
        let id = window_of(ps, self.window)?;
        let open = open_window(id)?;
        let node = widget_named(&open.tree.load(), id, &self.name)?;
        let name = &self.name;
        if !node.takes_focus() {
            let text = match node.kind {
                Kind::Text => format!("{name} in window {id} is a Text, which takes the focus only with -Style Link"),
                Kind::GpuSurface => format!("{name} in window {id} is a GpuSurface, which takes the focus only with -OnKey"),
                other => format!("{name} in window {id} is a {}, which takes no focus", widgets::widget_name(other)),
            };
            return Err(PsError::new(ErrorCategory::InvalidArgument, "GuiNotFocusable", text).terminating());
        }
        match open.session.focus_node(id, node.id, session::arm(ps)) {
            Ok(()) => Ok(()),
            Err(pwrs_gui_client::GuiError::Refused(why)) => Err(PsError::new(ErrorCategory::InvalidArgument, "GuiRefused", format!("{name} in window {id} cannot take the focus: {why}"))),
            Err(e) => Err(gui_error(e)),
        }
    }
}

/// Closes a window. Inside a handler it closes the handler's window, and
/// the command waiting on it writes -Result as its result's Result;
/// elsewhere -Window names the window, and a window described in
/// PowerShell takes -Result as its result as well. A number no open window
/// has is a non-terminating error; a -NoWait window counts as open until
/// its Close is taken.
///
/// # Examples
/// Close-GuiWindow -Result Saved
/// Get-GuiWindow -Title Report | Close-GuiWindow
#[cmdlet(verb = "Close", noun = "GuiWindow")]
#[derive(Default)]
pub struct CloseGuiWindow {
    /// The window's number; inside a handler, the handler's window when not
    /// given.
    #[param(position = 0, value_from_pipeline_by_property_name, alias = ["Id"])]
    pub window: Option<u64>,
    /// The window's result; Closed when not given.
    #[param]
    pub result: Option<PsObject>,
}

impl Cmdlet for CloseGuiWindow {
    fn process(&mut self, ps: &Pipeline<'_>) -> PsResult<()> {
        let id = window_of(ps, self.window)?;
        let result = match &self.result {
            Some(r) => r.clone(),
            None => "Closed".into_ps()?,
        };
        let waited_here = WAITING.with(|w| match w.borrow_mut().get_mut(&id) {
            Some(slot) => {
                *slot = Some(result.clone());
                true
            }
            None => false,
        });
        if waited_here {
            return Ok(());
        }
        if let Some(open) = OPEN.load().get(&id) {
            open.closing.store(Some(Arc::new(to_value(&result)?)));
            return session::of_window(id)?.close_window(id).map_err(gui_error);
        }
        let Some(live) = session::running_of(pwrs_gui_client::Renderer::of_number(id)) else {
            return Err(no_window_to_close(id));
        };
        match live.close_open_window(id, session::arm(ps)) {
            Ok(()) => Ok(()),
            Err(pwrs_gui_client::GuiError::Refused(_)) => Err(no_window_to_close(id)),
            Err(e) => Err(gui_error(e)),
        }
    }
}

/// Close-GuiWindow's non-terminating error for a number no open window has.
fn no_window_to_close(id: WindowId) -> PsError {
    PsError::new(ErrorCategory::ObjectNotFound, "GuiNoWindow", format!("no window is open with the number {id}"))
}

/// Waits for the next event of a window Show-GuiWindow -NoWait opened, and
/// writes it as a handler is given it: Window, Name, Kind, Value and State.
/// With -Name, only that widget's events, and the window's Close. A
/// window's events wait for this command whether or not a handler ran for
/// them, the window's Close last, after which the window is forgotten.
/// With -Timeout, writes nothing when no event comes in that many seconds.
/// Ctrl+C ends the wait.
///
/// # Examples
/// $w = Show-GuiWindow -NoWait { Button 'Go' -Name Go }; Wait-GuiEvent $w.Id
/// Wait-GuiEvent -Window 3 -Name Servers -Timeout 10
#[cmdlet(verb = "Wait", noun = "GuiEvent", output = ["PwrsGui.WidgetEvent"])]
#[derive(Default)]
pub struct WaitGuiEvent {
    /// The window's number, as Show-GuiWindow -NoWait writes it.
    #[param(mandatory, position = 0, value_from_pipeline_by_property_name, alias = ["Id"])]
    pub window: u64,
    /// Only this widget's events, and the window's Close.
    #[param(position = 1)]
    pub name: Option<String>,
    /// How many seconds to wait, zero or more; with no event by then the
    /// command writes nothing. Without it, the wait has no limit.
    #[param]
    pub timeout: Option<f64>,
}

impl Cmdlet for WaitGuiEvent {
    fn process(&mut self, ps: &Pipeline<'_>) -> PsResult<()> {
        let open = open_window(self.window)?;
        if !open.queues {
            let text = format!("window {} was not opened with -NoWait; the command that opened it reads its events", self.window);
            return Err(PsError::new(ErrorCategory::InvalidOperation, "GuiNotQueued", text).terminating());
        }
        let deadline = match self.timeout {
            None => None,
            Some(t) if t.is_finite() && t >= 0.0 => Some(Instant::now() + Duration::from_secs_f64(t)),
            Some(t) => return Err(PsError::new(ErrorCategory::InvalidArgument, "GuiBadTimeout", format!("-Timeout is {t}; it must be zero or more seconds")).terminating()),
        };
        let me = std::thread::current();
        let woken = me.clone();
        let waker = ps.on_stop(move || woken.unpark())?;
        open.wait_here(&me);
        let taken = loop {
            if let Some(q) = open.take(self.name.as_deref()) {
                break Some(q);
            }
            if ps.stopping() {
                break None;
            }
            match deadline {
                None => std::thread::park(),
                Some(d) => {
                    let now = Instant::now();
                    if now >= d {
                        break None;
                    }
                    std::thread::park_timeout(d - now);
                }
            }
        };
        open.stop_waiting(&me);
        drop(waker);
        let Some(q) = taken else {
            return Ok(());
        };
        if q.kind == GuiEventKind::Close {
            forget(self.window);
        }
        let value = open.shown(q.node, &q.value, matches!(q.kind, GuiEventKind::Activate | GuiEventKind::Expand))?;
        let state = open.state_object_of(&q.state)?;
        ps.write(GuiWidgetEvent { window: self.window, name: q.name, kind: q.kind, value, state })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicBool;

    fn event(name: &str, kind: GuiEventKind, n: i64) -> Queued {
        Queued { node: Some(1), name: Some(name.to_string()), kind, value: Value::Int(n), state: Vec::new() }
    }

    fn number(q: &Queued) -> i64 {
        q.value.as_int().expect("an event's number")
    }

    #[test]
    fn the_event_log_gives_events_oldest_first_and_lets_go_of_the_chunks_taken() {
        let log = EventLog::new();
        for n in 0..1000 {
            log.push(event("a", GuiEventKind::Click, n)).expect("an event added");
        }
        for n in 0..1000 {
            assert_eq!(log.take(None).map(|q| number(&q)), Some(n));
        }
        assert!(log.take(None).is_none());
        let chunks = log.chunks.load();
        assert!(chunks.1.len() <= 1, "{} chunks left after every event was taken", chunks.1.len());
        assert_eq!(chunks.0, 1000 / LOG_CHUNK);
    }

    #[test]
    fn the_event_log_takes_one_widgets_oldest_event_and_the_close_for_any_name() {
        let log = EventLog::new();
        for (name, n) in [("a", 0), ("b", 1), ("a", 2)] {
            log.push(event(name, GuiEventKind::Click, n)).expect("an event added");
        }
        assert_eq!(log.take(Some("B")).map(|q| number(&q)), Some(1));
        assert_eq!(log.take(Some("a")).map(|q| number(&q)), Some(0));
        log.push(Queued { node: None, name: None, kind: GuiEventKind::Close, value: Value::Int(3), state: Vec::new() }).expect("the close added");
        assert_eq!(log.take(Some("b")).map(|q| number(&q)), Some(3), "the window's Close matches any name");
        assert_eq!(log.take(None).map(|q| number(&q)), Some(2));
        assert!(log.take(None).is_none());
    }

    #[test]
    fn the_event_log_gives_each_event_once_to_takers_on_many_threads() {
        let log = Arc::new(EventLog::new());
        let done = Arc::new(AtomicBool::new(false));
        let total = 20_000;
        let takers: Vec<_> = (0..4)
            .map(|_| {
                let log = Arc::clone(&log);
                let done = Arc::clone(&done);
                std::thread::spawn(move || {
                    let mut got = Vec::new();
                    loop {
                        match log.take(None) {
                            Some(q) => got.push(number(&q)),
                            None if done.load(Ordering::Acquire) => match log.take(None) {
                                Some(q) => got.push(number(&q)),
                                None => break,
                            },
                            None => std::thread::yield_now(),
                        }
                    }
                    got
                })
            })
            .collect();
        for n in 0..total {
            log.push(event("a", GuiEventKind::Click, n)).expect("an event added");
        }
        done.store(true, Ordering::Release);
        let mut all: Vec<i64> = takers.into_iter().flat_map(|t| t.join().expect("a taker's thread")).collect();
        all.sort_unstable();
        assert_eq!(all, (0..total).collect::<Vec<i64>>(), "every event taken once");
    }
}
