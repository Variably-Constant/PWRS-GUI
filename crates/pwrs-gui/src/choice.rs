//! `Read-GuiChoice`: a list of choices with OK and Cancel.

use crate::session::{self, gui_error};
use crate::tree::Builder;
use pwrs::prelude::*;
use pwrs_gui_client::WindowEvent;
use pwrs_gui_client::proto::{EventKind, Fit, Kind, Node, NodeId, PatchOp, Value, WindowOptions};
use std::collections::{BTreeMap, BTreeSet};

/// The text each object is shown by, one string per object in order.
/// With `$Property`, a property name or a script block run with `$_` as the
/// object, as Format-Wide's -Property takes. Without it, what Format-Wide
/// shows: a string or a value type as itself; else the entry of the first
/// wide view Get-FormatData gives one of its type names, the entry selected
/// by one of its type names before the entry selected by none; else its
/// type's DefaultDisplayProperty; else its first property named like name,
/// id, key, *key, *name or *id, in that order; else its ToString(). Wide
/// views are looked up once per type name.
const LABELS: &str = r#"
param($Objects, $Property)
$wide = @{}
function Find-Wide($o) {
    foreach ($t in $o.PSObject.TypeNames) {
        if (-not $wide.ContainsKey($t)) {
            $wide[$t] = $null
            foreach ($d in Get-FormatData -TypeName $t -PowerShellVersion $PSVersionTable.PSVersion) {
                foreach ($v in $d.FormatViewDefinition) {
                    if ($v.Control -is [System.Management.Automation.WideControl] -and $null -eq $wide[$t]) { $wide[$t] = $v.Control.Entries }
                }
            }
        }
        $entries = $wide[$t]
        if (-not $entries) { continue }
        $pick = $null
        foreach ($e in $entries) {
            if ($e.EntrySelectedBy -and $e.EntrySelectedBy.TypeNames) {
                foreach ($n in $e.EntrySelectedBy.TypeNames) {
                    if (-not $pick -and $o.PSObject.TypeNames -contains $n) { $pick = $e }
                }
            }
        }
        if (-not $pick) {
            foreach ($e in $entries) {
                if (-not $pick -and -not ($e.EntrySelectedBy -and $e.EntrySelectedBy.TypeNames)) { $pick = $e }
            }
        }
        if (-not $pick) { $pick = $entries[0] }
        $entry = $pick.DisplayEntry
        if ("$($entry.ValueType)" -eq 'ScriptBlock') { return [scriptblock]::Create($entry.Value) }
        return [string]$entry.Value
    }
    $null
}
function Find-Expression($o) {
    $found = Find-Wide $o
    if ($null -ne $found) { return $found }
    $standard = $o.PSStandardMembers
    if ($standard -and $standard.DefaultDisplayProperty) { return [string]$standard.DefaultDisplayProperty }
    foreach ($pattern in 'name', 'id', 'key', '*key', '*name', '*id') {
        foreach ($p in $o.PSObject.Properties) {
            if ($p.Name -like $pattern) { return $p.Name }
        }
    }
    $null
}
foreach ($o in $Objects) {
    if ($null -eq $o) { ''; continue }
    $expression = $Property
    if ($null -eq $expression) {
        if ($o -is [string] -or $o -is [ValueType]) { [string]$o; continue }
        $expression = Find-Expression $o
    }
    if ($null -eq $expression) { [string]$o; continue }
    if ($expression -is [scriptblock]) {
        $variables = [System.Collections.Generic.List[psvariable]]::new()
        $variables.Add([psvariable]::new('_', $o))
        (@($expression.InvokeWithContext($null, $variables)) | ForEach-Object { [string]$_ }) -join ' '
        continue
    }
    $p = $o.PSObject.Properties[[string]$expression]
    if ($null -eq $p) { ''; continue }
    [string]$p.Value
}
"#;

/// Shows a list of choices and writes the one chosen, or with -Multiple
/// every one checked, as the objects given and in the order given; nothing
/// when the window is canceled or closed.
///
/// Each choice is shown by what Format-Wide shows for it, or by the
/// property or script block -Property names. Without -Multiple, OK is
/// enabled once a choice is selected, and double-clicking a choice or
/// pressing Enter on it takes it. Escape cancels, and so does Ctrl+C in
/// the console, which also closes the window.
///
/// # Examples
/// Read-GuiChoice -Prompt 'Deploy to' -Choice 'staging', 'production'
/// Get-Service | Read-GuiChoice -Prompt 'Restart which?' -Multiple
/// Get-Service | Read-GuiChoice -Property DisplayName
#[cmdlet(verb = "Read", noun = "GuiChoice", output = ["System.Object"])]
#[derive(Default)]
pub struct ReadGuiChoice {
    /// The text above the choices.
    #[param(position = 0)]
    pub prompt: Option<String>,
    /// The choices; piped objects are added to them in the order piped.
    #[param(mandatory, position = 1, value_from_pipeline)]
    pub choice: Vec<PsObject>,
    /// The choice, or with -Multiple the choices, selected when the window
    /// opens, matched to the text the choices are shown by without regard
    /// to case. An object given here is shown by the same rule first.
    #[param]
    pub default: Option<Vec<PsObject>>,
    /// Check any number of choices rather than select one.
    #[param]
    pub multiple: bool,
    /// The window's title; Choose when none is given.
    #[param]
    pub title: Option<String>,
    /// What each choice is shown by: a property name, or a script block
    /// run with $_ as the choice, as Format-Wide's -Property takes. Without
    /// it, a choice is shown by what Format-Wide shows for it.
    #[param]
    pub property: Option<PsObject>,
    gathered: Vec<PsObject>,
}

impl Cmdlet for ReadGuiChoice {
    fn process(&mut self, _ps: &Pipeline<'_>) -> PsResult<()> {
        self.gathered.append(&mut self.choice);
        Ok(())
    }

    fn end(&mut self, ps: &Pipeline<'_>) -> PsResult<()> {
        if self.gathered.is_empty() {
            return Err(PsError::new(ErrorCategory::InvalidArgument, "GuiNoChoices", "Read-GuiChoice was given no choices").terminating());
        }
        let property = match &self.property {
            Some(p) if !p.is_null() => {
                let kind = p.type_name()?;
                if kind != "System.String" && kind != "System.Management.Automation.ScriptBlock" {
                    let text = format!("-Property takes a property name or a script block, and was given a {kind}");
                    return Err(PsError::new(ErrorCategory::InvalidArgument, "GuiPropertyInvalid", text).terminating());
                }
                p.clone()
            }
            Some(_) | None => PsObject::null(),
        };
        let script = PsScriptBlock(PsType::from_name("scriptblock").call_static("Create", &[LABELS.into_ps()?])?);
        let labels = shown_as(ps, &script, &self.gathered, &property)?;
        let defaults = match &self.default {
            Some(d) => shown_as(ps, &script, d, &property)?,
            None => Vec::new(),
        };
        let preset = preset_rows(&labels, &defaults).map_err(|e| PsError::new(ErrorCategory::InvalidArgument, "GuiDefaultNotAChoice", e).terminating())?;
        if !self.multiple && preset.len() > 1 {
            let text = format!("{} defaults were given for one choice; -Multiple takes several", preset.len());
            return Err(PsError::new(ErrorCategory::InvalidArgument, "GuiTooManyDefaults", text).terminating());
        }
        let dialog = dialog(self.prompt.as_deref(), &labels, &preset, self.multiple);
        let options = WindowOptions {
            title: match &self.title {
                Some(t) => t.clone(),
                None => String::from("Choose"),
            },
            width: None,
            height: None,
            resizable: true,
            always_on_top: false,
            close_on_escape: true,
            dialog: true,
            fit: Fit::DIALOG,
            theme: crate::options::theme(),
        };
        let session = session::widget_session(ps)?;
        let mut window = session.open_window(options, dialog.tree).map_err(gui_error)?;
        // The list takes the keyboard as the dialog opens, as a system's
        // dialog gives its first control the focus, so the arrows and Enter
        // choose without a click first.
        if let Err(e) = session.focus_node(window.id(), dialog.list, session::arm(ps)) {
            pwrs::warning!(ps, "the choice list could not take the keyboard focus: {e}")?;
        }
        let mut selected = preset;
        let mut problem = None;
        let chosen = loop {
            let Some(event) = session::next_event(ps, &mut window)? else {
                return Ok(());
            };
            match event {
                WindowEvent::Event { node, kind: EventKind::Select, value, .. } if node == dialog.list => {
                    selected = rows_of(&value).map_err(|e| PsError::new(ErrorCategory::InvalidData, "GuiBadSelection", e).terminating())?;
                    if !self.multiple {
                        window.patch(vec![enabled(dialog.ok, !selected.is_empty())]).map_err(gui_error)?;
                    }
                }
                WindowEvent::Event { node, kind: EventKind::Activate, value, .. } if node == dialog.list && !self.multiple => {
                    let row = row_of(&value).map_err(|e| PsError::new(ErrorCategory::InvalidData, "GuiBadSelection", e).terminating())?;
                    break Some(BTreeSet::from([row]));
                }
                WindowEvent::Event { node, kind: EventKind::Click, .. } if node == dialog.ok => break Some(selected),
                WindowEvent::Event { node, kind: EventKind::Click, .. } if node == dialog.cancel => break None,
                WindowEvent::Closed(reason) => return session::closed_error(reason, problem.as_deref()).map_or(Ok(()), Err),
                WindowEvent::PageClosed(_) => return Ok(()),
                WindowEvent::Problem(p) => {
                    pwrs::warning!(ps, "the choice window reported: {p}")?;
                    problem = Some(p);
                }
                WindowEvent::Message { json, .. } => pwrs::warning!(ps, "the choice window, which shows no page, got a page's message: {json}")?,
                WindowEvent::Call { name, .. } => pwrs::warning!(ps, "the choice window, which shows no page, got a page's call of {name}")?,
                WindowEvent::Open { json, .. } => pwrs::warning!(ps, "the choice window, which shows no page, got a page's open of a window: {json}")?,
                WindowEvent::Opened | WindowEvent::Event { .. } | WindowEvent::PickerWanted { .. } | WindowEvent::FileChanged { .. } => {}
            }
        };
        window.close().map_err(gui_error)?;
        if let Some(rows) = chosen {
            for r in rows {
                match self.gathered.get(r) {
                    Some(obj) => ps.write_object(obj)?,
                    None => {
                        let text = format!("the window chose row {r}, and there are {} choices", self.gathered.len());
                        return Err(PsError::new(ErrorCategory::InvalidData, "GuiBadSelection", text).terminating());
                    }
                }
            }
        }
        Ok(())
    }
}

/// The choice window's tree and the nodes its events come from.
struct Dialog {
    tree: Node,
    list: NodeId,
    ok: NodeId,
    cancel: NodeId,
}

/// The prompt as the dialog's main line, then the list filling the room
/// left, as wide as the dialog, and OK, the default button, and Cancel in
/// the dialog's button bar.
fn dialog(prompt: Option<&str>, labels: &[String], preset: &BTreeSet<usize>, multiple: bool) -> Dialog {
    let mut b = Builder::new();
    let mut root = b.node(Kind::Column).prop("align", Value::Str(String::from("stretch")));
    if let Some(p) = prompt {
        root = root.child(b.text(p).prop("style", Value::Str(String::from("instruction")))).child(b.node(Kind::Spacer));
    }
    let list = b
        .named(Kind::ListBox, "Choice")
        .prop("items", Value::List(labels.iter().cloned().map(Value::Str).collect()))
        .prop("multiple", Value::Bool(multiple))
        .prop("selected", Value::List(preset.iter().map(|&r| Value::Int(r as i64)).collect()));
    let ok = b.button("Ok", "OK").prop("enabled", Value::Bool(multiple || !preset.is_empty())).prop("default", Value::Bool(true));
    let cancel = b.button("Cancel", "Cancel").prop("cancel", Value::Bool(true));
    let (list_id, ok_id, cancel_id) = (list.id, ok.id, cancel.id);
    let buttons = b.node(Kind::ButtonBar).child(ok).child(cancel);
    Dialog { tree: root.child(list).child(buttons), list: list_id, ok: ok_id, cancel: cancel_id }
}

fn enabled(node: NodeId, on: bool) -> PatchOp {
    PatchOp::SetProps { node, props: BTreeMap::from([(String::from("enabled"), Value::Bool(on))]) }
}

/// The text each of `objects` is shown by, as [`LABELS`] gives it with
/// `property`, one per object.
fn shown_as(ps: &Pipeline<'_>, script: &PsScriptBlock, objects: &[PsObject], property: &PsObject) -> PsResult<Vec<String>> {
    let shown = script.call(ps, &[objects.to_vec().into_ps()?, property.clone()])?;
    if shown.len() != objects.len() {
        let text = format!("the labels came back as {} for {} objects", shown.len(), objects.len());
        return Err(PsError::new(ErrorCategory::InvalidResult, "GuiLabelCount", text).terminating());
    }
    shown.iter().map(String::from_ps).collect()
}

/// The rows of `labels` the defaults name, matched without regard to case.
fn preset_rows(labels: &[String], defaults: &[String]) -> Result<BTreeSet<usize>, String> {
    let lowered: Vec<String> = labels.iter().map(|l| l.to_lowercase()).collect();
    defaults
        .iter()
        .map(|d| {
            let want = d.to_lowercase();
            lowered.iter().position(|l| *l == want).ok_or_else(|| format!("the default '{d}' is not one of the choices"))
        })
        .collect()
}

/// The row numbers in a list's `Select` event.
fn rows_of(value: &Value) -> Result<BTreeSet<usize>, String> {
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

    fn labels() -> Vec<String> {
        vec![String::from("Staging"), String::from("Production"), String::from("Lab")]
    }

    #[test]
    fn defaults_match_choices_without_regard_to_case() {
        let rows = preset_rows(&labels(), &[String::from("production"), String::from("LAB")]).expect("both are choices");
        assert_eq!(rows, BTreeSet::from([1, 2]));
        let missing = preset_rows(&labels(), &[String::from("Test")]);
        assert!(matches!(missing, Err(ref why) if why.contains("'Test'")), "{missing:?}");
    }

    #[test]
    fn selections_read_as_row_numbers_and_anything_else_is_refused() {
        assert_eq!(rows_of(&Value::List(vec![Value::Int(2), Value::Int(0)])), Ok(BTreeSet::from([0, 2])));
        assert!(rows_of(&Value::Int(1)).is_err());
        assert!(row_of(&Value::Int(-1)).is_err());
        assert_eq!(row_of(&Value::Int(1)), Ok(1));
    }

    #[test]
    fn ok_starts_disabled_for_one_choice_with_no_default() {
        let none = dialog(Some("Pick"), &labels(), &BTreeSet::new(), false);
        let ok = none.tree.find(none.ok).expect("the OK button");
        assert_eq!(ok.props.get("enabled"), Some(&Value::Bool(false)));
        let preset = dialog(None, &labels(), &BTreeSet::from([1]), false);
        let ok = preset.tree.find(preset.ok).expect("the OK button");
        assert_eq!(ok.props.get("enabled"), Some(&Value::Bool(true)));
        let many = dialog(None, &labels(), &BTreeSet::new(), true);
        let ok = many.tree.find(many.ok).expect("the OK button");
        assert_eq!(ok.props.get("enabled"), Some(&Value::Bool(true)));
        assert_eq!(many.tree.names(), vec![String::from("Choice"), String::from("Ok"), String::from("Cancel")]);
    }

    #[test]
    fn the_prompt_leads_the_list_and_ok_and_cancel_stand_in_the_button_bar_with_their_roles() {
        let d = dialog(Some("Pick"), &labels(), &BTreeSet::new(), false);
        let kinds: Vec<Kind> = d.tree.children.iter().map(|c| c.kind).collect();
        assert_eq!(kinds, [Kind::Text, Kind::Spacer, Kind::ListBox, Kind::ButtonBar]);
        assert_eq!(d.tree.children[0].props.get("style"), Some(&Value::Str(String::from("instruction"))));
        let ok = d.tree.find(d.ok).expect("the OK button");
        let cancel = d.tree.find(d.cancel).expect("the Cancel button");
        assert_eq!(ok.props.get("default"), Some(&Value::Bool(true)));
        assert_eq!(cancel.props.get("cancel"), Some(&Value::Bool(true)));
    }
}
