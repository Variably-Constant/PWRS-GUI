//! `Read-GuiInput`, a form of labeled fields with OK and Cancel, and
//! `Read-GuiCredential`, a user name and password form.

use crate::session::{self, gui_error};
use crate::tree::Builder;
use crate::{widgets, window};
use pwrs::prelude::*;
use pwrs_gui_client::WindowEvent;
use pwrs_gui_client::proto::{EventKind, Fit, Kind, Node, NodeId, PatchOp, Value, WindowOptions, date};
use std::collections::{BTreeMap, BTreeSet};

/// The names the forms' buttons go by, which no field may take.
const BUTTONS: [&str; 2] = ["Ok", "Cancel"];

/// The keys a field's hashtable may have.
const FIELD_KEYS: [&str; 5] = ["Name", "Label", "Type", "Default", "Choice"];

/// What a field takes, from its hashtable's Type.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum FieldType {
    Text,
    Password,
    Number,
    Date,
    Check,
    Choice,
}

impl FieldType {
    fn parse(given: &str) -> Result<FieldType, String> {
        match given.to_ascii_lowercase().as_str() {
            "text" => Ok(FieldType::Text),
            "password" => Ok(FieldType::Password),
            "number" => Ok(FieldType::Number),
            "date" => Ok(FieldType::Date),
            "check" => Ok(FieldType::Check),
            "choice" => Ok(FieldType::Choice),
            _ => Err(format!("Type {given} is none of Text, Password, Number, Date, Check and Choice")),
        }
    }
}

/// One field: the name its value goes under, the label beside it, what it
/// takes, the value it opens with as the renderer takes it, and a Choice
/// field's choices.
struct Field {
    name: String,
    label: String,
    kind: FieldType,
    value: Value,
    choices: Vec<String>,
}

/// A form's tree, its inputs in field order, and its buttons.
struct Form {
    tree: Node,
    inputs: Vec<NodeId>,
    ok: NodeId,
    cancel: NodeId,
}

/// Shows a form of fields with OK and Cancel and writes what was entered as
/// one object, a property per field under its name, in the order given: a
/// Text field's text, a Password field's as a SecureString, a Number
/// field's as a [double], a Date field's as a [datetime] or null when none
/// was picked, a Check field's as a [bool] and a Choice field's choice as
/// text, or null when none was picked. Writes nothing when the window is
/// canceled or closed.
///
/// Each -Field is a hashtable: Name, which its property takes; Label, shown
/// beside the field, the Name when none is given; Type, one of Text,
/// Password, Number, Date, Check and Choice, Text when none is given;
/// Default, the value the field opens with, which a Password field does not
/// take; and Choice, a Choice field's choices, of which Default names one.
/// The first field has the keyboard as the form opens, so a person types
/// into it at once. Enter in a text field takes the form as OK does. Escape
/// cancels, and so does Ctrl+C in the console, which also closes the window.
///
/// # Examples
/// Read-GuiInput -Prompt 'New user' -Field @{ Name = 'Login' }, @{ Name = 'Password'; Type = 'Password' }
/// Read-GuiInput -Field @{ Name = 'Count'; Type = 'Number'; Default = 3 }, @{ Name = 'Due'; Type = 'Date' }
/// Read-GuiInput -Field @{ Name = 'Env'; Label = 'Deploy to'; Type = 'Choice'; Choice = 'staging', 'production'; Default = 'staging' }
#[cmdlet(verb = "Read", noun = "GuiInput", output = ["System.Management.Automation.PSCustomObject"])]
#[derive(Default)]
pub struct ReadGuiInput {
    /// The text above the fields.
    #[param(position = 0)]
    pub prompt: Option<String>,
    /// The fields, each a hashtable of Name, Label, Type, Default and
    /// Choice.
    #[param(mandatory, position = 1)]
    pub field: Vec<PsObject>,
    /// The window's title; Input when none is given.
    #[param]
    pub title: Option<String>,
}

impl Cmdlet for ReadGuiInput {
    fn process(&mut self, ps: &Pipeline<'_>) -> PsResult<()> {
        let refuse = |text: String| PsError::new(ErrorCategory::InvalidArgument, "GuiBadField", text).terminating();
        if self.field.is_empty() {
            return Err(refuse(String::from("Read-GuiInput was given no field")));
        }
        let mut fields = Vec::new();
        let mut names = BTreeSet::new();
        for (i, table) in self.field.iter().enumerate() {
            let f = field_of(ps, table).map_err(|e| refuse(format!("-Field {}: {e}", i + 1)))?;
            if BUTTONS.iter().any(|b| b.eq_ignore_ascii_case(&f.name)) {
                return Err(refuse(format!("-Field {}: {} names one of the form's buttons, Ok and Cancel", i + 1, f.name)));
            }
            if !names.insert(f.name.to_lowercase()) {
                return Err(refuse(format!("-Field {}: two fields are named {}, and each value goes under its own name", i + 1, f.name)));
            }
            fields.push(f);
        }
        let form = input_form(self.prompt.as_deref(), &fields, true);
        let options = WindowOptions {
            title: match &self.title {
                Some(t) => t.clone(),
                None => String::from("Input"),
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
        let Some(values) = ask(ps, options, &form, form.inputs[0], |_| true)? else {
            return Ok(());
        };
        let out = pwrs::object::new_psobject("PwrsGui.Input");
        for (f, id) in fields.iter().zip(&form.inputs) {
            let node = form_node(&form, *id)?;
            let presented = widgets::presented_value(node, values.get(id)).map_err(|e| bad_value(format!("{}: {e}", f.name)))?;
            pwrs::object::add_note(&out, &f.name, window::input_object(node, &presented)?)?;
        }
        ps.write_object(&out)
    }
}

/// Prompts for a user name and password and writes them as a PSCredential;
/// nothing when the window is canceled or closed. The user name field has
/// the keyboard as the form opens, or the password field when -UserName
/// fills the name, as the system's own prompts do. OK is enabled once a
/// user name is entered, and Enter in either field then takes the form as
/// OK does. Escape cancels, and so does Ctrl+C in the console, which also
/// closes the window.
///
/// # Examples
/// Read-GuiCredential -Message 'Sign in to the build server'
/// $credential = Read-GuiCredential -UserName 'deploy' -Title 'Deploy'
#[cmdlet(verb = "Read", noun = "GuiCredential", output = ["System.Management.Automation.PSCredential"])]
#[derive(Default)]
pub struct ReadGuiCredential {
    /// The text above the fields.
    #[param(position = 0)]
    pub message: Option<String>,
    /// The user name the form opens with.
    #[param]
    pub user_name: Option<String>,
    /// The window's title; Credential when none is given.
    #[param]
    pub title: Option<String>,
}

impl Cmdlet for ReadGuiCredential {
    fn process(&mut self, ps: &Pipeline<'_>) -> PsResult<()> {
        let user_name = match &self.user_name {
            Some(u) => u.clone(),
            None => String::new(),
        };
        let filled = |text: &str| !text.trim().is_empty();
        let fields = [
            Field { name: String::from("UserName"), label: String::from("User name"), kind: FieldType::Text, value: Value::Str(user_name.clone()), choices: Vec::new() },
            Field { name: String::from("Password"), label: String::from("Password"), kind: FieldType::Password, value: Value::Str(String::new()), choices: Vec::new() },
        ];
        let form = input_form(self.message.as_deref(), &fields, filled(&user_name));
        let (user, password) = (form.inputs[0], form.inputs[1]);
        let options = WindowOptions {
            title: match &self.title {
                Some(t) => t.clone(),
                None => String::from("Credential"),
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
        let ready = |values: &BTreeMap<NodeId, Value>| matches!(values.get(&user), Some(Value::Str(text)) if filled(text));
        let focus = if filled(&user_name) { password } else { user };
        let Some(values) = ask(ps, options, &form, focus, ready)? else {
            return Ok(());
        };
        let name = match values.get(&user) {
            Some(Value::Str(text)) => text.clone(),
            other => return Err(bad_value(format!("the user name field's value is {other:?}, which is not text"))),
        };
        let secret = match values.get(&password) {
            Some(v) => window::input_object(form_node(&form, password)?, v)?,
            None => return Err(bad_value(String::from("the password field's value went unread"))),
        };
        let credential = PsType::from_name("System.Management.Automation.PSCredential").new(&[name.into_ps()?, secret])?;
        ps.write_object(&credential)
    }
}

/// The field a hashtable describes, its Default converted as the renderer
/// takes the field's value.
fn field_of(ps: &Pipeline<'_>, table: &PsObject) -> Result<Field, String> {
    let message = |e: PsError| e.message;
    let read = PsReadOnlyTable::over(table).map_err(|e| format!("a field is a hashtable of Name, Label, Type, Default and Choice: {}", e.message))?;
    for key in read.keys().map_err(message)? {
        if !FIELD_KEYS.iter().any(|k| k.eq_ignore_ascii_case(&key)) {
            return Err(format!("{key} is none of Name, Label, Type, Default and Choice"));
        }
    }
    let text = |key: &str| -> Result<Option<String>, String> {
        let v = read.get(key).map_err(message)?;
        if v.is_null() { Ok(None) } else { String::from_ps(&v).map(Some).map_err(|e| format!("{key}: {}", e.message)) }
    };
    let name = match text("Name")? {
        Some(n) if !n.trim().is_empty() => n,
        Some(_) | None => return Err(String::from("a field needs a Name, which its value goes under")),
    };
    let label = match text("Label")? {
        Some(l) => l,
        None => name.clone(),
    };
    let kind = match text("Type")? {
        Some(t) => FieldType::parse(&t)?,
        None => FieldType::Text,
    };
    let given = read.get("Default").map_err(message)?;
    let choices = widgets::strings(ps, &read.get("Choice").map_err(message)?).map_err(|e| format!("Choice: {}", e.message))?;
    if kind != FieldType::Choice && !choices.is_empty() {
        return Err(format!("{name} is a {kind:?} field, and only a Choice field takes Choice"));
    }
    let default = |e: PsError| format!("{name}'s Default: {}", e.message);
    let value = match kind {
        FieldType::Text if given.is_null() => Value::Str(String::new()),
        FieldType::Text => Value::Str(String::from_ps(&given).map_err(default)?),
        FieldType::Password if given.is_null() => Value::Str(String::new()),
        FieldType::Password => return Err(format!("{name} is a Password field, which takes no Default; the person types it")),
        FieldType::Number if given.is_null() => Value::Float(0.0),
        FieldType::Number => {
            let n = f64::from_ps(&given).map_err(default)?;
            if !n.is_finite() {
                return Err(format!("{name}'s Default is {n}; it must be a finite number"));
            }
            Value::Float(n)
        }
        FieldType::Date if given.is_null() => Value::Null,
        FieldType::Date => Value::Str(date::minutes(PsDateTime::from_ps(&given).map_err(default)?.ticks)),
        FieldType::Check if given.is_null() => Value::Bool(false),
        FieldType::Check => Value::Bool(bool::from_ps(&given).map_err(default)?),
        FieldType::Choice if choices.is_empty() => return Err(format!("{name} is a Choice field and needs Choice, its choices")),
        FieldType::Choice if given.is_null() => Value::Null,
        FieldType::Choice => {
            let want = String::from_ps(&given).map_err(default)?;
            let lowered = want.to_lowercase();
            match choices.iter().position(|c| c.to_lowercase() == lowered) {
                Some(row) => Value::Int(row as i64),
                None => return Err(format!("{name}'s Default '{want}' is not one of its choices")),
            }
        }
    };
    Ok(Field { name, label, kind, value, choices })
}

/// A form of `fields` in a grid of two columns, each field's label beside
/// its input, a check box's label as its own text with the label column
/// empty beside it, every input as wide as the column the dialog leaves
/// them; below `prompt`, the dialog's main line, and with OK, the default
/// button, enabled as `ok_enabled` says, and Cancel in the button bar.
fn input_form(prompt: Option<&str>, fields: &[Field], ok_enabled: bool) -> Form {
    let mut b = Builder::new();
    let mut root = b.node(Kind::Column).prop("align", Value::Str(String::from("stretch")));
    if let Some(p) = prompt {
        root = root.child(b.text(p).prop("style", Value::Str(String::from("instruction")))).child(b.node(Kind::Spacer));
    }
    let mut grid = b.node(Kind::Grid).prop("columns", Value::Int(2)).prop("align", Value::Str(String::from("stretch")));
    let mut inputs = Vec::new();
    for f in fields {
        let label = b.text(if f.kind == FieldType::Check { "" } else { &f.label });
        let input = field_node(&mut b, f);
        inputs.push(input.id);
        grid = grid.child(label).child(input);
    }
    let ok = b.button(BUTTONS[0], "OK").prop("enabled", Value::Bool(ok_enabled)).prop("default", Value::Bool(true));
    let cancel = b.button(BUTTONS[1], "Cancel").prop("cancel", Value::Bool(true));
    let (ok_id, cancel_id) = (ok.id, cancel.id);
    let buttons = b.node(Kind::ButtonBar).child(ok).child(cancel);
    Form { tree: root.child(grid).child(buttons), inputs, ok: ok_id, cancel: cancel_id }
}

/// The input widget a field is shown by, with the value it opens with;
/// a check box shows the field's label as its own text.
fn field_node(b: &mut Builder, f: &Field) -> Node {
    match f.kind {
        FieldType::Text => b.named(Kind::TextBox, &f.name).prop("text", f.value.clone()),
        FieldType::Password => b.named(Kind::TextBox, &f.name).prop("password", Value::Bool(true)).prop("text", f.value.clone()),
        FieldType::Number => b.named(Kind::NumberBox, &f.name).prop("value", f.value.clone()),
        FieldType::Date => b.named(Kind::DatePicker, &f.name).prop("value", f.value.clone()),
        FieldType::Check => b.named(Kind::CheckBox, &f.name).prop("text", Value::Str(f.label.clone())).prop("checked", f.value.clone()),
        FieldType::Choice => b
            .named(Kind::ComboBox, &f.name)
            .prop("items", Value::List(f.choices.iter().cloned().map(Value::Str).collect()))
            .prop("selected", f.value.clone()),
    }
}

/// Opens `form` with the keyboard on `focus`, one of its inputs, as a
/// system's dialog gives a field the focus so a person types without a
/// click first, and runs it until OK, which returns each input's value as
/// the renderer last gave it, or until Cancel or the window closes, which
/// return `None`. `ready` tells from the values whether OK may be taken;
/// OK is enabled and disabled as its result changes.
fn ask(ps: &Pipeline<'_>, options: WindowOptions, form: &Form, focus: NodeId, ready: impl Fn(&BTreeMap<NodeId, Value>) -> bool) -> PsResult<Option<BTreeMap<NodeId, Value>>> {
    let mut values = BTreeMap::new();
    for id in &form.inputs {
        let node = form_node(form, *id)?;
        let start = match widgets::value_prop(node).and_then(|p| node.props.get(p)) {
            Some(v) => v.clone(),
            None => Value::Null,
        };
        values.insert(*id, start);
    }
    let session = session::widget_session(ps)?;
    let mut window = session.open_window(options, form.tree.clone()).map_err(gui_error)?;
    if let Err(e) = session.focus_node(window.id(), focus, session::arm(ps)) {
        pwrs::warning!(ps, "the form's field could not take the keyboard focus: {e}")?;
    }
    let mut allowed = ready(&values);
    let mut problem = None;
    let taken = loop {
        let Some(event) = session::next_event(ps, &mut window)? else {
            return Ok(None);
        };
        match event {
            WindowEvent::Event { node, kind: kind @ (EventKind::Change | EventKind::Submit), value, .. } if form.inputs.contains(&node) => {
                values.insert(node, value);
                let now = ready(&values);
                if now != allowed {
                    allowed = now;
                    window.patch(vec![enabled(form.ok, now)]).map_err(gui_error)?;
                }
                if kind == EventKind::Submit && allowed {
                    break true;
                }
            }
            WindowEvent::Event { node, kind: EventKind::Click, .. } if node == form.ok && allowed => break true,
            WindowEvent::Event { node, kind: EventKind::Click, .. } if node == form.cancel => break false,
            WindowEvent::Closed(reason) => return session::closed_error(reason, problem.as_deref()).map_or(Ok(None), Err),
            WindowEvent::PageClosed(_) => return Ok(None),
            WindowEvent::Problem(p) => {
                pwrs::warning!(ps, "the form reported: {p}")?;
                problem = Some(p);
            }
            WindowEvent::Message { json, .. } => pwrs::warning!(ps, "the form, which shows no page, got a page's message: {json}")?,
            WindowEvent::Call { name, .. } => pwrs::warning!(ps, "the form, which shows no page, got a page's call of {name}")?,
            WindowEvent::Open { json, .. } => pwrs::warning!(ps, "the form, which shows no page, got a page's open of a window: {json}")?,
            WindowEvent::Opened | WindowEvent::Event { .. } | WindowEvent::PickerWanted { .. } | WindowEvent::FileChanged { .. } => {}
        }
    };
    window.close().map_err(gui_error)?;
    Ok(if taken { Some(values) } else { None })
}

fn form_node(form: &Form, id: NodeId) -> PsResult<&Node> {
    match form.tree.find(id) {
        Some(node) => Ok(node),
        None => Err(bad_value(format!("the form has no node {id}"))),
    }
}

fn enabled(node: NodeId, on: bool) -> PatchOp {
    PatchOp::SetProps { node, props: BTreeMap::from([(String::from("enabled"), Value::Bool(on))]) }
}

fn bad_value(text: String) -> PsError {
    PsError::new(ErrorCategory::InvalidData, "GuiBadInput", text).terminating()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn field(name: &str, kind: FieldType, value: Value, choices: &[&str]) -> Field {
        Field { name: name.to_string(), label: format!("{name}:"), kind, value, choices: choices.iter().map(|c| c.to_string()).collect() }
    }

    #[test]
    fn types_read_without_regard_to_case_and_anything_else_is_refused() {
        assert_eq!(FieldType::parse("password"), Ok(FieldType::Password));
        assert_eq!(FieldType::parse("CHOICE"), Ok(FieldType::Choice));
        let other = FieldType::parse("Slider");
        assert!(matches!(other, Err(ref why) if why.contains("Slider")), "{other:?}");
    }

    #[test]
    fn a_form_holds_each_field_as_its_input_beside_its_label_in_order() {
        let fields = [
            field("Login", FieldType::Text, Value::Str(String::from("ada")), &[]),
            field("Secret", FieldType::Password, Value::Str(String::new()), &[]),
            field("Count", FieldType::Number, Value::Float(3.0), &[]),
            field("Due", FieldType::Date, Value::Null, &[]),
            field("Agree", FieldType::Check, Value::Bool(true), &[]),
            field("Env", FieldType::Choice, Value::Int(1), &["staging", "production"]),
        ];
        let form = input_form(Some("Fill in"), &fields, true);
        let kinds: Vec<Kind> = form.inputs.iter().map(|id| form.tree.find(*id).expect("an input").kind).collect();
        assert_eq!(kinds, vec![Kind::TextBox, Kind::TextBox, Kind::NumberBox, Kind::DatePicker, Kind::CheckBox, Kind::ComboBox]);
        let secret = form.tree.find(form.inputs[1]).expect("the password field");
        assert_eq!(secret.props.get("password"), Some(&Value::Bool(true)));
        let env = form.tree.find(form.inputs[5]).expect("the choice field");
        assert_eq!(env.props.get("selected"), Some(&Value::Int(1)));
        assert_eq!(form.tree.names(), vec!["Login", "Secret", "Count", "Due", "Agree", "Env", "Ok", "Cancel"]);
    }

    #[test]
    fn ok_opens_enabled_as_asked() {
        let fields = [field("UserName", FieldType::Text, Value::Str(String::new()), &[])];
        let off = input_form(None, &fields, false);
        assert_eq!(off.tree.find(off.ok).expect("OK").props.get("enabled"), Some(&Value::Bool(false)));
        let on = input_form(None, &fields, true);
        assert_eq!(on.tree.find(on.ok).expect("OK").props.get("enabled"), Some(&Value::Bool(true)));
    }

    #[test]
    fn a_check_field_shows_its_label_as_its_own_text_and_the_buttons_stand_in_the_bar() {
        let fields = [field("Login", FieldType::Text, Value::Str(String::new()), &[]), field("Admin", FieldType::Check, Value::Bool(false), &[])];
        let form = input_form(Some("New user"), &fields, true);
        let check = form.tree.find(form.inputs[1]).expect("the check box");
        assert_eq!(check.props.get("text"), Some(&Value::Str(String::from("Admin:"))), "the label is the check box's text");
        let grid = form.tree.children.iter().find(|c| c.kind == Kind::Grid).expect("the grid");
        assert_eq!(grid.props.get("align"), Some(&Value::Str(String::from("stretch"))));
        assert_eq!(grid.children[2].props.get("text"), Some(&Value::Str(String::new())), "the label column is empty beside a check box");
        let bar = form.tree.children.last().expect("the button bar");
        assert_eq!(bar.kind, Kind::ButtonBar);
        assert_eq!(form.tree.find(form.ok).and_then(|n| n.props.get("default")), Some(&Value::Bool(true)));
        assert_eq!(form.tree.find(form.cancel).and_then(|n| n.props.get("cancel")), Some(&Value::Bool(true)));
    }
}
