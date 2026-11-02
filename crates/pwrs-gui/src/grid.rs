//! `Out-GuiGridView`: the piped objects in a table that sorts, filters and
//! selects in the renderer, with Out-GridView's parameters.

use crate::session::{self, gui_error};
use crate::tree::Builder;
use pwrs::prelude::*;
use pwrs::sys::{
    PS_TYPE_DATETIME, PS_TYPE_DECIMAL, PS_TYPE_F32, PS_TYPE_F64, PS_TYPE_I16, PS_TYPE_I32, PS_TYPE_I64, PS_TYPE_I8, PS_TYPE_OBJECT, PS_TYPE_TIMESPAN,
    PS_TYPE_U16, PS_TYPE_U32, PS_TYPE_U64, PS_TYPE_U8,
};
use pwrs_gui_client::proto::{Cell, EventKind, Fit, Kind, Node, NodeId, PatchOp, Row, SortKey, Value, WindowOptions};
use pwrs_gui_client::{Window, WindowEvent};
use std::collections::{BTreeMap, BTreeSet};
use std::time::{Duration, Instant};

/// Rows converted and sent together while input is arriving quickly.
const BATCH: usize = 256;

/// How long rows wait for their batch to fill: input arriving more slowly
/// than this is sent object by object.
const BATCH_WAIT: Duration = Duration::from_millis(100);

/// The columns for the first object: one per entry of the table view
/// Get-FormatData gives the first of its type names that has one, else
/// one per property of its default display set, else one per property;
/// a string or a value type is one column of itself. Each column has
/// its label, its format string, its alignment as the table view's header
/// gives it (`left`, `center` or `right`, null where the view leaves it
/// undefined or there is no view), and the calculated property
/// Select-Object evaluates it with, so a script-block column gets `$_` as
/// the formatter does.
pub const COLUMNS: &str = r#"
param($o)
$cols = [System.Collections.Generic.List[object]]::new()
function Add-Column($label, $expression, $format, $align) {
    $cols.Add([pscustomobject]@{ Label = [string]$label; Format = $format; Align = $align; Calc = @{ Name = "c$($cols.Count)"; Expression = $expression } })
}
if ($o -is [string] -or $o -is [ValueType]) {
    Add-Column $o.GetType().Name { $_ } $null
    return $cols
}
foreach ($t in $o.PSObject.TypeNames) {
    foreach ($d in Get-FormatData -TypeName $t -PowerShellVersion $PSVersionTable.PSVersion) {
        foreach ($v in $d.FormatViewDefinition) {
            if ($v.Control -is [System.Management.Automation.TableControl]) {
                $c = $v.Control
                $row = $c.Rows[0]
                for ($i = 0; $i -lt $row.Columns.Count; $i++) {
                    $e = $row.Columns[$i].DisplayEntry
                    $label = if ($i -lt $c.Headers.Count) { $c.Headers[$i].Label } else { $null }
                    if (-not $label) { $label = $e.Value }
                    $align = if ($i -lt $c.Headers.Count -and "$($c.Headers[$i].Alignment)" -ne 'Undefined') { "$($c.Headers[$i].Alignment)".ToLowerInvariant() } else { $null }
                    $expression = if ("$($e.ValueType)" -eq 'ScriptBlock') { [scriptblock]::Create($e.Value) } else { $e.Value }
                    Add-Column $label $expression $row.Columns[$i].FormatString $align
                }
                return $cols
            }
        }
    }
}
$names = $o.PSStandardMembers.DefaultDisplayPropertySet.ReferencedPropertyNames
if (-not $names) { $names = $o.PSObject.Properties.Name }
foreach ($n in $names) { Add-Column $n $n $null }
$cols
"#;

/// What Out-GuiGridView writes.
#[psenum(name = "PwrsGui.OutputMode")]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum GridOutputMode {
    /// Nothing; the command returns when its input ends and the window
    /// stays open.
    #[default]
    None,
    /// The row chosen with OK or a double-click.
    Single,
    /// The rows chosen with OK.
    Multiple,
}

/// Shows the piped objects in a window as a table that sorts by any
/// column, filters as you type and selects rows. The window opens with the
/// objects piped in its first tenth of a second, up to 256, sized to fit
/// their rows within the screen, and takes the rest while the pipeline
/// runs. A column of numbers is right-aligned, as PowerShell's own tables
/// align it, unless the table view says otherwise.
///
/// The columns are those of the table view Get-FormatData gives the first
/// object's type, each property or script block evaluated as the console's
/// table would; without a table view, its default display properties or
/// all its properties. Without -PassThru, -OutputMode Single or Multiple,
/// or -Wait, the command returns once its input has ended and its window
/// has opened, and leaves the window open, as Out-GridView does. With them
/// it waits: OK writes the selected objects, the original ones, in input
/// order, and Cancel or closing the window writes nothing.
///
/// # Examples
/// Get-Process | Out-GuiGridView
/// Get-Service | Out-GuiGridView -Title 'Pick services' -PassThru | Restart-Service -WhatIf
/// Get-ChildItem | Out-GuiGridView -OutputMode Single
#[cmdlet(verb = "Out", noun = "GuiGridView", default_parameter_set = "PassThru", output = ["System.Object"])]
#[derive(Default)]
pub struct OutGuiGridView {
    /// The objects to show, one row each.
    #[param(value_from_pipeline)]
    pub input_object: PsObject,
    /// The window's title; the command line that ran Out-GuiGridView when
    /// none is given.
    #[param]
    pub title: Option<String>,
    /// Writes the rows chosen with OK, as -OutputMode Multiple does.
    #[param(set = "PassThru")]
    pub pass_thru: bool,
    /// Waits for the window to close and writes nothing.
    #[param(set = "Wait")]
    pub wait: bool,
    /// What is written: nothing, one row chosen, or any rows chosen.
    #[param(set = "OutputMode")]
    pub output_mode: Option<GridOutputMode>,
    starting: Option<Starting>,
    grid: Option<Grid>,
}

/// A grid before its window opens: the window's tree and options from the
/// first object's columns, and the objects piped so far, which the window
/// opens with as its first rows, so it fits them.
struct Starting {
    tree: Node,
    options: WindowOptions,
    table: NodeId,
    buttons: Option<(NodeId, NodeId)>,
    calc: PsObject,
    formats: Vec<Option<String>>,
    objects: Vec<PsObject>,
    began: Instant,
}

impl Starting {
    /// Opens the window with the objects piped so far as its rows.
    fn open(self, ps: &Pipeline<'_>) -> PsResult<Grid> {
        let rows = rows_of(ps, &self.calc, &self.formats, &self.objects)?;
        let session = session::widget_session(ps)?;
        let window = session.open_window_with_rows(self.options, self.tree, vec![(self.table, rows)]).map_err(gui_error)?;
        Ok(Grid {
            window,
            table: self.table,
            buttons: self.buttons,
            calc: self.calc,
            formats: self.formats,
            sent: self.objects.len(),
            objects: self.objects,
            last_flush: Instant::now(),
            selected: BTreeSet::new(),
            opened: false,
            closed: false,
            problem: None,
        })
    }
}

/// How the command ends once its input has.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Ending {
    /// Leave the window open and return, once its first frame is drawn.
    Detach,
    /// Wait for the window to close.
    Wait,
    /// Wait for OK, writing one row or any rows.
    Choose { multiple: bool },
}

impl OutGuiGridView {
    fn ending(&self) -> Ending {
        if self.pass_thru {
            return Ending::Choose { multiple: true };
        }
        match self.output_mode {
            Some(GridOutputMode::Single) => Ending::Choose { multiple: false },
            Some(GridOutputMode::Multiple) => Ending::Choose { multiple: true },
            Some(GridOutputMode::None) | None if self.wait => Ending::Wait,
            Some(GridOutputMode::None) | None => Ending::Detach,
        }
    }

    fn title(&self, ps: &Pipeline<'_>) -> PsResult<String> {
        if let Some(t) = &self.title {
            return Ok(t.clone());
        }
        let line = String::from_ps(&ps.invocation()?.get("Line")?)?;
        let line = line.trim();
        Ok(if line.is_empty() { String::from("Out-GuiGridView") } else { line.to_string() })
    }
}

/// The open window and every object shown in it, in input order.
struct Grid {
    window: Window,
    table: NodeId,
    buttons: Option<(NodeId, NodeId)>,
    /// The calculated properties Select-Object evaluates the columns with.
    calc: PsObject,
    formats: Vec<Option<String>>,
    objects: Vec<PsObject>,
    /// How many of `objects` the window has as rows.
    sent: usize,
    last_flush: Instant,
    selected: BTreeSet<usize>,
    /// The renderer has drawn the window's first frame, so Get-GuiWindow
    /// lists it.
    opened: bool,
    /// The person closed the window.
    closed: bool,
    /// What the renderer reported last, which a window closed because its
    /// page could not be shown again ends the command with.
    problem: Option<String>,
}

impl Cmdlet for OutGuiGridView {
    fn process(&mut self, ps: &Pipeline<'_>) -> PsResult<()> {
        if self.input_object.is_null() {
            return Ok(());
        }
        if self.grid.is_none() {
            if self.starting.is_none() {
                self.starting = Some(self.start(ps)?);
            }
            let Some(s) = self.starting.as_mut() else {
                return Ok(());
            };
            s.objects.push(self.input_object.clone());
            if (s.objects.len() >= BATCH || s.began.elapsed() >= BATCH_WAIT)
                && let Some(s) = self.starting.take()
            {
                self.grid = Some(s.open(ps)?);
            }
            return Ok(());
        }
        let Some(grid) = self.grid.as_mut() else {
            return Ok(());
        };
        grid.drain(ps)?;
        if grid.closed {
            return Ok(());
        }
        grid.objects.push(self.input_object.clone());
        if grid.objects.len() - grid.sent >= BATCH || grid.last_flush.elapsed() >= BATCH_WAIT {
            grid.flush(ps)?;
        }
        Ok(())
    }

    fn end(&mut self, ps: &Pipeline<'_>) -> PsResult<()> {
        let ending = self.ending();
        if let Some(s) = self.starting.take() {
            self.grid = Some(s.open(ps)?);
        }
        let Some(mut grid) = self.grid.take() else {
            return Ok(());
        };
        grid.drain(ps)?;
        if grid.closed {
            return Ok(());
        }
        grid.flush(ps)?;
        let chosen = match ending {
            Ending::Detach => {
                while !grid.opened && !grid.closed {
                    let Some(event) = session::next_event(ps, &mut grid.window)? else {
                        return Ok(());
                    };
                    grid.take(ps, event)?;
                }
                if !grid.closed {
                    grid.window.detach().map_err(gui_error)?;
                }
                return Ok(());
            }
            Ending::Wait => {
                while !grid.closed {
                    let Some(event) = session::next_event(ps, &mut grid.window)? else {
                        return Ok(());
                    };
                    grid.take(ps, event)?;
                }
                return Ok(());
            }
            Ending::Choose { multiple } => grid.choose(ps, multiple)?,
        };
        let Grid { window, objects, .. } = grid;
        window.close().map_err(gui_error)?;
        if let Some(rows) = chosen {
            for r in rows {
                match objects.get(r) {
                    Some(obj) => ps.write_object(obj)?,
                    None => {
                        let text = format!("the window chose row {r}, and there are {} rows", objects.len());
                        return Err(PsError::new(ErrorCategory::InvalidData, "GuiBadSelection", text).terminating());
                    }
                }
            }
        }
        Ok(())
    }
}

impl OutGuiGridView {
    /// The window's tree and options from the first object's columns, a
    /// column of numbers right-aligned as PowerShell's tables align it, with
    /// OK, the default button, and Cancel in a button bar when the command
    /// waits for a choice.
    fn start(&self, ps: &Pipeline<'_>) -> PsResult<Starting> {
        let script = PsScriptBlock(PsType::from_name("scriptblock").call_static("Create", &[COLUMNS.into_ps()?])?);
        let columns = script.call(ps, std::slice::from_ref(&self.input_object))?;
        if columns.is_empty() {
            return Err(PsError::new(ErrorCategory::InvalidData, "GuiNoColumns", "the first object has no properties to show").terminating());
        }
        let mut labels = Vec::with_capacity(columns.len());
        let mut formats = Vec::with_capacity(columns.len());
        let mut aligns: Vec<Option<String>> = Vec::with_capacity(columns.len());
        let mut calc = Vec::with_capacity(columns.len());
        for c in &columns {
            labels.push(String::from_ps(&pwrs::object::property(c, "Label")?)?);
            formats.push(Option::<String>::from_ps(&pwrs::object::property(c, "Format")?)?.filter(|f| !f.is_empty()));
            aligns.push(Option::<String>::from_ps(&pwrs::object::property(c, "Align")?)?.filter(|a| !a.is_empty()));
            calc.push(pwrs::object::property(c, "Calc")?);
        }
        let ending = self.ending();
        let selection = match ending {
            Ending::Choose { multiple: true } => "multiple",
            Ending::Choose { multiple: false } => "single",
            Ending::Detach | Ending::Wait => "none",
        };
        let mut b = Builder::new();
        let calc = calc.into_ps()?;
        let columns_prop = Value::List(labels.iter().zip(&aligns).map(|(l, a)| column_value(l, a.as_deref())).collect());
        let table = b.named(Kind::Table, "Grid").prop("columns", columns_prop).prop("selection", Value::Str(String::from(selection))).prop("filter", Value::Bool(true));
        let table_id = table.id;
        let mut root = b.node(Kind::Column).child(table);
        let mut buttons = None;
        if let Ending::Choose { .. } = ending {
            let ok = b.button("Ok", "OK").prop("enabled", Value::Bool(false)).prop("default", Value::Bool(true));
            let cancel = b.button("Cancel", "Cancel").prop("cancel", Value::Bool(true));
            buttons = Some((ok.id, cancel.id));
            root = root.child(b.node(Kind::ButtonBar).child(ok).child(cancel));
        }
        let options = WindowOptions {
            title: self.title(ps)?,
            width: None,
            height: None,
            resizable: true,
            always_on_top: false,
            close_on_escape: false,
            dialog: false,
            fit: Fit::WINDOW,
            theme: crate::options::theme(),
            hidden: false,
        };
        Ok(Starting { tree: root, options, table: table_id, buttons, calc, formats, objects: Vec::new(), began: Instant::now() })
    }
}

/// A table column as the renderer's `columns` property lists it: its label,
/// and the alignment its table view gives it, when it gives one.
pub fn column_value(label: &str, align: Option<&str>) -> Value {
    let mut column = BTreeMap::from([(String::from("label"), Value::Str(label.to_string()))]);
    if let Some(a) = align {
        column.insert(String::from("align"), Value::Str(a.to_string()));
    }
    Value::Map(column)
}

/// The rows `objects` make: each evaluated by the columns' calculated
/// properties, its cells written by each column's format.
fn rows_of(ps: &Pipeline<'_>, calc: &PsObject, formats: &[Option<String>], objects: &[PsObject]) -> PsResult<Vec<Row>> {
    if objects.is_empty() {
        return Ok(Vec::new());
    }
    let count = objects.len();
    let evaluated = ps.invoke_with_input("Select-Object", &[("Property", calc.clone())], Some(&objects.to_vec().into_ps()?))?;
    if evaluated.len() != count {
        let text = format!("Select-Object returned {} rows for {count} objects", evaluated.len());
        return Err(PsError::new(ErrorCategory::InvalidResult, "GuiRowCount", text).terminating());
    }
    let mut rows = Vec::with_capacity(count);
    for e in &evaluated {
        let mut cells = Vec::with_capacity(formats.len());
        for (i, format) in formats.iter().enumerate() {
            let value = pwrs::object::property(e, &format!("c{i}"))?;
            cells.push(cell(&value, format.as_deref())?);
        }
        rows.push(Row { cells });
    }
    Ok(rows)
}

impl Grid {
    /// Converts the objects not yet shown and sends them as rows.
    fn flush(&mut self, ps: &Pipeline<'_>) -> PsResult<()> {
        self.last_flush = Instant::now();
        if self.closed || self.sent == self.objects.len() {
            return Ok(());
        }
        let batch = &self.objects[self.sent..];
        let count = batch.len();
        let rows = rows_of(ps, &self.calc, &self.formats, batch)?;
        self.window.rows(self.table, self.sent as u64, rows, false).map_err(gui_error)?;
        self.sent += count;
        Ok(())
    }

    /// Takes every event that has arrived, without waiting.
    fn drain(&mut self, ps: &Pipeline<'_>) -> PsResult<()> {
        while !self.closed {
            match self.window.try_next_event().map_err(gui_error)? {
                Some(event) => self.take(ps, event)?,
                None => break,
            }
        }
        Ok(())
    }

    /// Records what an event says about the selection and the window.
    fn take(&mut self, ps: &Pipeline<'_>, event: WindowEvent) -> PsResult<()> {
        match event {
            WindowEvent::Event { node, kind: EventKind::Select, value, .. } if node == self.table => {
                self.selected = selection_of(&value).map_err(|e| PsError::new(ErrorCategory::InvalidData, "GuiBadSelection", e).terminating())?;
            }
            WindowEvent::Closed(reason) => {
                self.closed = true;
                if let Some(e) = session::closed_error(reason, self.problem.as_deref()) {
                    return Err(e);
                }
            }
            WindowEvent::PageClosed(_) => self.closed = true,
            WindowEvent::Problem(p) => {
                pwrs::warning!(ps, "the grid window reported: {p}")?;
                self.problem = Some(p);
            }
            WindowEvent::Message { json, .. } => pwrs::warning!(ps, "the grid window, which shows no page, got a page's message: {json}")?,
            WindowEvent::Opened => self.opened = true,
            WindowEvent::Event { .. } | WindowEvent::PickerWanted { .. } | WindowEvent::FileChanged { .. } => {}
        }
        Ok(())
    }

    /// Enables OK and waits for the person's choice: the rows chosen, or
    /// `None` for Cancel, a closed window or Ctrl+C.
    fn choose(&mut self, ps: &Pipeline<'_>, multiple: bool) -> PsResult<Option<BTreeSet<usize>>> {
        let Some((ok, cancel)) = self.buttons else {
            return Ok(None);
        };
        let props = BTreeMap::from([(String::from("enabled"), Value::Bool(true))]);
        self.window.patch(vec![PatchOp::SetProps { node: ok, props }]).map_err(gui_error)?;
        let chosen = loop {
            let Some(event) = session::next_event(ps, &mut self.window)? else {
                return Ok(None);
            };
            match event {
                WindowEvent::Event { node, kind: EventKind::Click, .. } if node == ok => break Some(std::mem::take(&mut self.selected)),
                WindowEvent::Event { node, kind: EventKind::Click, .. } if node == cancel => break None,
                WindowEvent::Event { node, kind: EventKind::Activate, value, .. } if node == self.table && !multiple => {
                    let row = row_of(&value).map_err(|e| PsError::new(ErrorCategory::InvalidData, "GuiBadSelection", e).terminating())?;
                    break Some(BTreeSet::from([row]));
                }
                other => {
                    self.take(ps, other)?;
                    if self.closed {
                        return Ok(None);
                    }
                }
            }
        };
        Ok(chosen)
    }
}

/// One cell: the text shown, as the console's table shows the value, and
/// the key it sorts by. Numbers, dates and spans sort by value, anything
/// else by its text, and an empty cell last.
pub fn cell(value: &PsObject, format: Option<&str>) -> PsResult<Cell> {
    if value.is_null() {
        return Ok(Cell { text: String::new(), sort: SortKey::Empty });
    }
    let tag = value.type_tag()?;
    let sort = match tag {
        PS_TYPE_I8 | PS_TYPE_I16 | PS_TYPE_I32 | PS_TYPE_I64 | PS_TYPE_U8 | PS_TYPE_U16 | PS_TYPE_U32 | PS_TYPE_U64 | PS_TYPE_F32 | PS_TYPE_F64 | PS_TYPE_DECIMAL => {
            Some(SortKey::Number(f64::from_ps(value)?))
        }
        PS_TYPE_DATETIME => Some(SortKey::Date(PsDateTime::from_ps(value)?.ticks as f64)),
        PS_TYPE_TIMESPAN => Some(SortKey::Number(PsTimeSpan::from_ps(value)?.ticks as f64)),
        _ => None,
    };
    let text = match format {
        Some(f) => String::from_ps(&PsType::from_name("string").call_static("Format", &[f.into_ps()?, value.clone()])?)?,
        None if tag == PS_TYPE_DATETIME || tag == PS_TYPE_TIMESPAN => String::from_ps(&value.call("ToString", &[])?)?,
        None if tag == PS_TYPE_OBJECT && value.type_name()?.ends_with("[]") => {
            let items = Vec::<PsObject>::from_ps(value)?.iter().map(String::from_ps).collect::<PsResult<Vec<String>>>()?;
            format!("{{{}}}", items.join(", "))
        }
        None => String::from_ps(value)?,
    };
    let sort = match sort {
        Some(s) => s,
        None if text.is_empty() => SortKey::Empty,
        None => SortKey::Text(text.clone()),
    };
    Ok(Cell { text, sort })
}

/// The row numbers in a table's `Select` event.
fn selection_of(value: &Value) -> Result<BTreeSet<usize>, String> {
    match value {
        Value::List(items) => items.iter().map(row_of).collect(),
        other => Err(format!("a selection is a list of row numbers, not {other:?}")),
    }
}

/// One row number, as an `Activate` event gives it.
fn row_of(value: &Value) -> Result<usize, String> {
    match value {
        Value::Int(i) => usize::try_from(*i).map_err(|e| format!("row {i} is not a row number: {e}")),
        other => Err(format!("a row number is an integer, not {other:?}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn grid_with(pass_thru: bool, wait: bool, output_mode: Option<GridOutputMode>) -> OutGuiGridView {
        OutGuiGridView { pass_thru, wait, output_mode, ..OutGuiGridView::default() }
    }

    #[test]
    fn the_parameters_choose_how_the_command_ends() {
        assert_eq!(grid_with(false, false, None).ending(), Ending::Detach);
        assert_eq!(grid_with(false, false, Some(GridOutputMode::None)).ending(), Ending::Detach);
        assert_eq!(grid_with(false, true, None).ending(), Ending::Wait);
        assert_eq!(grid_with(true, false, None).ending(), Ending::Choose { multiple: true });
        assert_eq!(grid_with(false, false, Some(GridOutputMode::Single)).ending(), Ending::Choose { multiple: false });
        assert_eq!(grid_with(false, false, Some(GridOutputMode::Multiple)).ending(), Ending::Choose { multiple: true });
    }

    #[test]
    fn selections_read_as_row_numbers_and_anything_else_is_refused() {
        assert_eq!(selection_of(&Value::List(vec![Value::Int(3), Value::Int(1)])), Ok(BTreeSet::from([1, 3])));
        assert!(selection_of(&Value::Str(String::from("1"))).is_err());
        assert!(row_of(&Value::Int(-2)).is_err());
    }
}
