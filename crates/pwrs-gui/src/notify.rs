//! Notifications: Show-GuiNotification shows one through the egui renderer,
//! Update-GuiNotification changes its title, its body and its progress bar,
//! Remove-GuiNotification takes it away, and Get-GuiNotification lists
//! those shown. A notification's number is one of the numbers windows take,
//! so its answers queue for Wait-GuiEvent and run its handlers as a -NoWait
//! window's events do, and Invoke-GuiTestEvent -Notification answers it.
//! On Windows, Register-GuiNotificationApp and
//! Unregister-GuiNotificationApp manage the identities notifications are
//! shown under.

use crate::session::{self, Shown, arm, gui_error};
use crate::values::to_value;
use crate::widgets::{self, Built};
use crate::window::{self, GuiHandlerOutput, Handling, Open};
use pwrs::prelude::*;
use pwrs_gui_client::Renderer;
use pwrs_gui_client::proto::notification::SOUNDS;
use pwrs_gui_client::proto::{EventKind, Kind, Node, NodeId, PatchOp, Value};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::sync::Arc;

/// A notification the egui renderer shows.
#[psclass(name = "PwrsGui.Notification")]
#[derive(Clone, Default)]
pub struct GuiNotification {
    /// Its number, one of the numbers windows take, which Wait-GuiEvent
    /// takes as -Window, and Update-GuiNotification,
    /// Remove-GuiNotification and Invoke-GuiTestEvent as -Notification.
    pub id: u64,
    /// Its title; null for one with none.
    pub title: Option<String>,
    /// The names of its inputs and buttons, inputs first.
    pub names: Vec<String>,
    /// The engine event its handlers run on, which Unregister-Event
    /// removes; null for one with no handlers, and in a listing.
    pub source_identifier: Option<String>,
}

/// What a notification is for, which decides how long it stays.
#[psenum(name = "PwrsGui.NotificationScenario")]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum GuiNotificationScenario {
    /// It times out as any other does.
    #[default]
    Default,
    /// It stays on the screen until it is answered.
    Reminder,
    /// It stays on the screen and repeats its sound.
    Alarm,
    /// It stays on the screen and repeats its sound, as a call does.
    IncomingCall,
    /// It breaks through Do Not Disturb where the person allows it.
    Urgent,
}

impl GuiNotificationScenario {
    /// Its name as the renderer reads it; none for the default.
    fn wire(self) -> Option<&'static str> {
        match self {
            GuiNotificationScenario::Default => None,
            GuiNotificationScenario::Reminder => Some("reminder"),
            GuiNotificationScenario::Alarm => Some("alarm"),
            GuiNotificationScenario::IncomingCall => Some("incomingCall"),
            GuiNotificationScenario::Urgent => Some("urgent"),
        }
    }
}

/// A terminating error for a parameter given what it does not take.
fn refused(what: &str, text: String) -> PsError {
    PsError::new(ErrorCategory::InvalidArgument, "GuiBadNotification", format!("{what}: {text}")).terminating()
}

/// Whether `given` is a hashtable, as `@{}` or `[ordered]@{}` makes.
fn is_table(given: &PsObject) -> PsResult<bool> {
    let name = given.type_name()?;
    Ok(name == "System.Collections.Hashtable" || name == "System.Collections.Specialized.OrderedDictionary")
}

/// Every key the hashtables these commands read take, as they are named.
const KEYS: &[&str] = &["Name", "Text", "Input", "OnClick", "Title", "Placeholder", "Choices", "Selected", "Value", "Status", "ValueText", "Sound", "Silent", "Loop"];

/// The entries of the hashtable `given`, each under the one of `keys` its
/// key names without regard to case; a key not among them is refused,
/// naming `what` and the keys it takes.
fn entries(given: &PsObject, what: &str, keys: &[&str]) -> PsResult<Vec<(&'static str, PsObject)>> {
    let table = PsReadOnlyTable::over(given)?;
    let mut out = Vec::new();
    for key in table.keys()? {
        let Some(known) = KEYS.iter().find(|k| k.eq_ignore_ascii_case(&key) && keys.contains(k)) else {
            return Err(refused(what, format!("{key} is not one of {}", keys.join(", "))));
        };
        out.push((*known, table.get(&key)?));
    }
    Ok(out)
}

/// The full path of the file `path` names, resolved from the current
/// location; an error naming `what` unless it names one existing file.
fn full_path(ps: &Pipeline<'_>, what: &str, path: &str) -> PsResult<String> {
    let resolved = ps.resolve_path(path, true).map_err(|e| PsError::new(ErrorCategory::ObjectNotFound, "GuiNoFile", format!("{what} {path}: {}", e.message)).terminating())?;
    match resolved.as_slice() {
        [one] => Ok(one.clone()),
        [] => Err(PsError::new(ErrorCategory::ObjectNotFound, "GuiNoFile", format!("{what} {path} names no file")).terminating()),
        many => Err(refused(what, format!("{path} names {} files; it must name one", many.len()))),
    }
}

/// Text as the renderer takes it; null stays null.
fn text_value(v: &PsObject) -> PsResult<Value> {
    if v.is_null() { Ok(Value::Null) } else { Ok(Value::Str(String::from_ps(v)?)) }
}

/// A progress bar as the renderer reads it: a number from 0 to 1, or a
/// hashtable of Value, Title, Status and ValueText, a null Value making a
/// bar that moves without saying how far.
fn progress_of(given: &PsObject) -> PsResult<Value> {
    let mut map = BTreeMap::new();
    if is_table(given)? {
        for (key, v) in entries(given, "-Progress", &["Value", "Title", "Status", "ValueText"])? {
            let (wire, shown) = match key {
                "Value" => ("value", to_value(&v)?),
                "Title" => ("title", text_value(&v)?),
                "Status" => ("status", text_value(&v)?),
                _value_text => ("valueText", text_value(&v)?),
            };
            map.insert(String::from(wire), shown);
        }
    } else {
        map.insert(String::from("value"), to_value(given)?);
    }
    if let Some(v) = map.get("value")
        && !matches!(v, Value::Null)
    {
        match v.as_number() {
            Some(n) if (0.0..=1.0).contains(&n) => {}
            Some(n) => return Err(refused("-Progress", format!("the value is {n}; it runs from 0 to 1"))),
            None => return Err(refused("-Progress", format!("the value is a number from 0 to 1, not {v:?}"))),
        }
    }
    Ok(Value::Map(map))
}

/// A sound as the renderer reads it: a sound's name, or a hashtable of
/// Sound, Silent and Loop.
fn audio_of(given: &PsObject) -> PsResult<Value> {
    let mut map = BTreeMap::new();
    if is_table(given)? {
        for (key, v) in entries(given, "-Audio", &["Sound", "Silent", "Loop"])? {
            let (wire, shown) = match key {
                "Sound" => ("sound", text_value(&v)?),
                "Silent" => ("silent", Value::Bool(bool::from_ps(&v)?)),
                _loop => ("loop", Value::Bool(bool::from_ps(&v)?)),
            };
            map.insert(String::from(wire), shown);
        }
    } else {
        map.insert(String::from("sound"), Value::Str(String::from_ps(given)?));
    }
    if let Some(Value::Str(sound)) = map.get("sound")
        && !SOUNDS.iter().any(|s| s.eq_ignore_ascii_case(sound))
    {
        return Err(refused("-Audio", format!("the sound is one of {}, not {sound}", SOUNDS.join(", "))));
    }
    Ok(Value::Map(map))
}

/// An input of -Input: a name, which is a text box's name and its
/// placeholder, or a hashtable of Name, Title, Placeholder and Text for a
/// text box, or of Name, Title, Choices and Selected for a choice.
fn input_of(given: &PsObject, id: NodeId) -> PsResult<Node> {
    if !is_table(given)? {
        let text = String::from_ps(given)?;
        return Ok(Node::new(id, Kind::TextBox).named(&text).prop("placeholder", Value::Str(text.clone())));
    }
    let mut props = BTreeMap::new();
    let mut input_name = None;
    let mut choices: Option<Vec<String>> = None;
    let mut selected = None;
    for (key, v) in entries(given, "-Input", &["Name", "Title", "Placeholder", "Text", "Choices", "Selected"])? {
        match key {
            "Name" => input_name = Some(String::from_ps(&v)?),
            "Title" => {
                props.insert(String::from("title"), text_value(&v)?);
            }
            "Placeholder" => {
                props.insert(String::from("placeholder"), text_value(&v)?);
            }
            "Text" => {
                props.insert(String::from("text"), text_value(&v)?);
            }
            "Choices" => choices = Some(Vec::<String>::from_ps(&v)?),
            _selected => selected = Some(v),
        }
    }
    let Some(input_name) = input_name else {
        return Err(refused("-Input", String::from("each hashtable gives the input's Name, under which the event's State has its value")));
    };
    let Some(choices) = choices else {
        if selected.is_some() {
            return Err(refused("-Input", format!("{input_name} has Selected and no Choices")));
        }
        let mut node = Node::new(id, Kind::TextBox).named(&input_name);
        node.props.extend(props);
        return Ok(node);
    };
    if props.contains_key("placeholder") || props.contains_key("text") {
        return Err(refused("-Input", format!("{input_name} is a choice, which takes no Placeholder or Text")));
    }
    let row = match selected {
        None => None,
        Some(v) => Some(choice_row(&input_name, &choices, &v)?),
    };
    let mut node = Node::new(id, Kind::ComboBox).named(&input_name).prop("items", Value::List(choices.into_iter().map(Value::Str).collect()));
    node.props.extend(props);
    if let Some(r) = row {
        node = node.prop("selected", Value::Int(r));
    }
    Ok(node)
}

/// The row of the choice `given` names, by its row from 0 or its text.
fn choice_row(input: &str, choices: &[String], given: &PsObject) -> PsResult<i64> {
    let row = match to_value(given)? {
        Value::Int(n) => match usize::try_from(n) {
            Ok(r) => (r < choices.len()).then_some(r),
            Err(e) => return Err(refused("-Input", format!("{input}'s Selected row {n}: {e}"))),
        },
        Value::Str(s) => choices.iter().position(|c| c.eq_ignore_ascii_case(&s)),
        other => return Err(refused("-Input", format!("{input}'s Selected is a choice's row or text, not {other:?}"))),
    };
    match row {
        Some(r) => i64::try_from(r).map_err(|e| refused("-Input", format!("{input}'s Selected row: {e}"))),
        None => Err(refused("-Input", format!("{input}'s Selected names none of its choices, {}", choices.join(", ")))),
    }
}

/// A button of -Button, with the handler of its own a hashtable gives: a
/// text, which is its name as well, or a hashtable of Text, Name, Input,
/// the name of the input it stands beside, and OnClick.
fn button_of(given: &PsObject, id: NodeId, inputs: &[Node]) -> PsResult<(Node, Option<PsScriptBlock>)> {
    if !is_table(given)? {
        let text = String::from_ps(given)?;
        return Ok((Node::new(id, Kind::Button).named(&text).prop("text", Value::Str(text.clone())), None));
    }
    let (mut text, mut button_name, mut input, mut handler) = (None, None, None, None);
    for (key, v) in entries(given, "-Button", &["Text", "Name", "Input", "OnClick"])? {
        match key {
            "Text" => text = Some(String::from_ps(&v)?),
            "Name" => button_name = Some(String::from_ps(&v)?),
            "Input" => input = Some(String::from_ps(&v)?),
            _on_click => handler = Some(PsScriptBlock::from_ps(&v)?),
        }
    }
    let Some(text) = text else {
        return Err(refused("-Button", String::from("each hashtable gives the button's Text")));
    };
    let named = match button_name {
        Some(n) => n,
        None => text.clone(),
    };
    let mut node = Node::new(id, Kind::Button).named(&named).prop("text", Value::Str(text));
    if let Some(i) = input {
        let Some(beside) = inputs.iter().find(|n| n.name.as_deref().is_some_and(|m| m.eq_ignore_ascii_case(&i))) else {
            return Err(refused("-Button", format!("{named} stands beside the input {i}, which -Input does not give")));
        };
        node = node.prop("input", Value::Int(i64::from(beside.id)));
    }
    Ok((node, handler))
}

/// A tree whose nodes come from parameters rather than widgets, with its
/// handlers, as the reader of a -NoWait window takes it.
fn built_of(tree: Node, handlers: HashMap<(NodeId, EventKind), PsScriptBlock>, next: NodeId) -> Built {
    Built {
        tree,
        handlers,
        results: BTreeMap::new(),
        rows: BTreeMap::new(),
        tables: BTreeMap::new(),
        trees: BTreeMap::new(),
        items: widgets::TreeItems::default(),
        canvases: BTreeMap::new(),
        next,
    }
}

/// Shows a notification: a title, a body, pictures, buttons, inputs, a
/// progress bar and a sound, under PWRS-GUI's own identity or the one -App
/// names. It returns once the notification shows, or with -Wait once it
/// is answered. On Windows it is a toast, on Linux and FreeBSD it goes to
/// the desktop's notification service over D-Bus, and on macOS the
/// notification center shows it under PWRS-GUI.app. A part the platform
/// cannot show is named in a warning, and the rest shows. On macOS the
/// first notification asks the person's permission, waiting for the answer;
/// a refusal is GuiNotificationDenied.
///
/// A click on a button raises Click under the button's name, and a click on
/// the notification itself raises Activate. Each input's answer comes first
/// as a Change under its name, and every event's State has the inputs'
/// values. A dismissal raises Dismiss, its value User, Timeout or Hidden. A
/// timed-out notification stays in the system's list and can still be
/// answered. An answer or a dismissal by the person ends the notification,
/// and Close comes last. The events wait for Wait-GuiEvent, and -OnClick,
/// -OnActivate and -OnDismiss run as a -NoWait window's handlers do.
///
/// On Windows, a notification shown under PWRS-GUI's own identity
/// registers it for the user the first time, under
/// HKCU\Software\Classes\AppUserModelId\PWRS-GUI, with its icon copied to
/// %LOCALAPPDATA%\PWRS-GUI\Notifications. Register-GuiNotificationApp
/// registers an identity of the caller's own for -App.
///
/// # Examples
/// Show-GuiNotification 'Backup done' '12 files copied'
/// Show-GuiNotification 'Deploy?' 'web01 is ready' -Button 'Deploy', 'Later' -Wait
/// Show-GuiNotification 'New message' 'Ann: lunch?' -Input @{ Name = 'Reply'; Placeholder = 'Type a reply' } -Button @{ Text = 'Send'; Input = 'Reply' } -OnClick { $args[0].State.Reply }
/// $n = Show-GuiNotification 'Copying' -Progress @{ Value = 0; Status = 'Starting' }
#[cmdlet(verb = "Show", noun = "GuiNotification", output = ["PwrsGui.Notification", "PwrsGui.WidgetEvent"])]
#[derive(Default)]
pub struct ShowGuiNotification {
    /// The first line, in bold.
    #[param(position = 0)]
    pub title: Option<String>,
    /// The text below the title.
    #[param(position = 1)]
    pub body: Option<String>,
    /// A picture shown in the notification: a file's path, from the current
    /// location.
    #[param]
    pub image: Option<String>,
    /// A picture shown across the top, on Windows: a file's path, from the
    /// current location.
    #[param]
    pub hero_image: Option<String>,
    /// A picture shown in place of the app's icon: a file's path, from the
    /// current location.
    #[param]
    pub app_logo: Option<String>,
    /// The buttons, at most five on Windows: each a text, which is its name
    /// as well, or a hashtable of Text, Name, Input, the name of the input
    /// it sends, and OnClick, which runs in place of -OnClick.
    #[param]
    pub button: Vec<PsObject>,
    /// The inputs, at most five on Windows: each a name, a text box with
    /// that placeholder, or a hashtable of Name, Title, Placeholder and Text
    /// for a text box, or of Name, Title, Choices and Selected for a choice.
    #[param]
    pub input: Vec<PsObject>,
    /// A progress bar: a number from 0 to 1, or a hashtable of Value, Title,
    /// Status and ValueText, a null Value making a bar that moves without
    /// saying how far. Update-GuiNotification moves it.
    #[param]
    pub progress: Option<PsObject>,
    /// The sound: one of Windows' notification sounds, as Mail or Alarm2,
    /// or a hashtable of Sound, Silent and Loop.
    #[param]
    pub audio: Option<PsObject>,
    /// What it is for, which decides how long it stays: Default, Reminder,
    /// Alarm, IncomingCall or Urgent.
    #[param]
    pub scenario: GuiNotificationScenario,
    /// The title of the group it is shown under in the system's list, on
    /// Windows.
    #[param]
    pub header: Option<String>,
    /// The identity it is shown under: on Windows an AppUserModelID, as
    /// Register-GuiNotificationApp registers, and elsewhere the app's name.
    /// PWRS-GUI's own when not given.
    #[param]
    pub app: Option<String>,
    /// Shows it without a banner: on Windows in the notification center
    /// alone, on Linux and FreeBSD at low urgency, and on macOS passively,
    /// with provisional permission when the person has not answered yet.
    #[param]
    pub quiet: bool,
    /// A toast's XML of the caller's own, which Windows shows in place of
    /// every other part given.
    #[param]
    pub xml: Option<String>,
    /// Runs when the notification itself is clicked, with the event as its
    /// argument.
    #[param]
    pub on_activate: Option<PsScriptBlock>,
    /// Runs when a button with no OnClick of its own is clicked, with the
    /// event as its argument, its Name the button's.
    #[param]
    pub on_click: Option<PsScriptBlock>,
    /// Runs when the notification is dismissed, with the event as its
    /// argument, its Value User, Timeout or Hidden.
    #[param]
    pub on_dismiss: Option<PsScriptBlock>,
    /// Waits until the notification ends, and writes the event that ended
    /// it: the button's Click, the Activate of a click on it, the Dismiss
    /// of a person's dismissal, or else its Close. Ctrl+C ends the wait.
    #[param]
    pub wait: bool,
    /// What happens to what the handlers write: Discard drops it, Pipeline
    /// writes it into the handler's event job, Console sends it to
    /// Out-Default. Discard when not given.
    #[param]
    pub handler_output: GuiHandlerOutput,
}

impl ShowGuiNotification {
    /// The notification's tree and its handlers.
    fn built(&self, ps: &Pipeline<'_>) -> PsResult<Built> {
        let mut root = Node::new(1, Kind::Notification);
        for (key, given) in [("title", &self.title), ("body", &self.body), ("header", &self.header), ("app", &self.app), ("xml", &self.xml)] {
            if let Some(t) = given {
                root.props.insert(String::from(key), Value::Str(t.clone()));
            }
        }
        for (key, what, given) in [("image", "-Image", &self.image), ("heroImage", "-HeroImage", &self.hero_image), ("appLogo", "-AppLogo", &self.app_logo)] {
            if let Some(p) = given {
                root.props.insert(String::from(key), Value::Str(full_path(ps, what, p)?));
            }
        }
        if let Some(p) = &self.progress {
            root.props.insert(String::from("progress"), progress_of(p)?);
        }
        if let Some(a) = &self.audio {
            root.props.insert(String::from("audio"), audio_of(a)?);
        }
        if let Some(s) = self.scenario.wire() {
            root.props.insert(String::from("scenario"), Value::Str(String::from(s)));
        }
        if self.quiet {
            root.props.insert(String::from("quiet"), Value::Bool(true));
        }
        let mut handlers = HashMap::new();
        for (kind, handler) in [(EventKind::Activate, &self.on_activate), (EventKind::Dismiss, &self.on_dismiss)] {
            if let Some(h) = handler {
                handlers.insert((1, kind), PsScriptBlock(h.0.clone()));
            }
        }
        let mut next: NodeId = 2;
        let mut inputs = Vec::new();
        for i in &self.input {
            inputs.push(input_of(i, next)?);
            next += 1;
        }
        let mut buttons = Vec::new();
        for b in &self.button {
            let (node, own) = button_of(b, next, &inputs)?;
            if let Some(h) = own.as_ref().or(self.on_click.as_ref()) {
                handlers.insert((node.id, EventKind::Click), PsScriptBlock(h.0.clone()));
            }
            buttons.push(node);
            next += 1;
        }
        root.children = inputs.into_iter().chain(buttons).collect();
        window::names_once(&root, &mut BTreeSet::new())?;
        Ok(built_of(root, handlers, next))
    }
}

impl Cmdlet for ShowGuiNotification {
    fn process(&mut self, ps: &Pipeline<'_>) -> PsResult<()> {
        if self.title.is_none() && self.body.is_none() && self.xml.is_none() {
            return Err(refused("Show-GuiNotification", String::from("give -Title, -Body or both, or -Xml")));
        }
        let built = self.built(ps)?;
        if self.app.is_none() {
            identity::own(ps)?;
        }
        let session = session::egui_session(ps)?;
        let mut shown = session.notify(built.tree.clone()).map_err(gui_error)?;
        let Some(before) = session::first_shown(ps, &mut shown, Shown::Notification)? else {
            return Ok(());
        };
        let handling = Handling { on_close: None, output: self.handler_output, files: Vec::new(), on_file_change: None, subscribe: false };
        let title = self.title.clone();
        let listed = match &title {
            Some(t) => t.clone(),
            None => String::new(),
        };
        let info = window::read_without_waiting(ps, session, shown, before, built, listed, HashMap::new(), handling)?;
        if self.wait {
            return wait_for_answer(ps, info.id);
        }
        ps.write(GuiNotification { id: info.id, title, names: info.names, source_identifier: info.source_identifier })
    }
}

/// Waits until notification `id` ends, and writes the event that ended it:
/// the last Click, Activate, Fail or Dismiss other than a timeout's, else
/// its Close. Writes nothing when Ctrl+C ended the wait.
fn wait_for_answer(ps: &Pipeline<'_>, id: u64) -> PsResult<()> {
    let mut answer: Option<PsObject> = None;
    loop {
        let taken = ps.invoke("PWRS-GUI\\Wait-GuiEvent", &[("Window", id.into_ps()?)])?;
        let Some(event) = taken.into_iter().next() else {
            return Ok(());
        };
        let kind = String::from_ps(&event.get("Kind")?)?;
        match kind.as_str() {
            "Close" => {
                return match answer {
                    Some(a) => ps.write_object(&a),
                    None => ps.write_object(&event),
                };
            }
            "Click" | "Activate" | "Fail" => answer = Some(event),
            "Dismiss" => {
                let value = event.get("Value")?;
                if value.is_null() || String::from_ps(&value)? != "Timeout" {
                    answer = Some(event);
                }
            }
            _change => {}
        }
    }
}

/// The notification numbered `id`, as the module stores it, while it shows.
fn shown_notification(id: u64) -> Option<Arc<Open>> {
    window::open_of_kind(id, Kind::Notification).filter(|_| window::is_open(id))
}

/// The error for a number no notification shown has; non-terminating, for
/// a command given several.
fn no_notification(id: u64) -> PsError {
    PsError::new(ErrorCategory::ObjectNotFound, "GuiNoNotification", format!("no notification is shown with the number {id}"))
}

/// Changes a notification's title, its body or its progress bar where it
/// shows: on Windows in place, and on Linux and FreeBSD by replacing it.
/// A notification of -Xml keeps the parts its XML gives.
///
/// # Examples
/// Update-GuiNotification -Notification $n.Id -Progress @{ Value = 0.5; Status = 'Copying' }
/// $n | Update-GuiNotification -Title 'Copied' -Progress 1
#[cmdlet(verb = "Update", noun = "GuiNotification")]
#[derive(Default)]
pub struct UpdateGuiNotification {
    /// The notification's number, as Show-GuiNotification writes it.
    #[param(mandatory, position = 0, value_from_pipeline_by_property_name, alias = ["Id"])]
    pub notification: u64,
    /// Its new title.
    #[param]
    pub title: Option<String>,
    /// Its new body.
    #[param]
    pub body: Option<String>,
    /// Its progress bar now: a number from 0 to 1, or a hashtable of Value,
    /// Title, Status and ValueText.
    #[param]
    pub progress: Option<PsObject>,
}

impl Cmdlet for UpdateGuiNotification {
    fn process(&mut self, _ps: &Pipeline<'_>) -> PsResult<()> {
        let id = self.notification;
        let Some(open) = shown_notification(id) else {
            return Err(no_notification(id).terminating());
        };
        let mut props = BTreeMap::new();
        for (key, given) in [("title", &self.title), ("body", &self.body)] {
            if let Some(t) = given {
                props.insert(String::from(key), Value::Str(t.clone()));
            }
        }
        if let Some(p) = &self.progress {
            props.insert(String::from("progress"), progress_of(p)?);
        }
        if props.is_empty() {
            return Err(refused("Update-GuiNotification", String::from("give -Title, -Body or -Progress")));
        }
        let mut root = (*open.tree()).clone();
        root.props.extend(props.clone());
        open.replace_tree(root, HashMap::new());
        session::of_window(id)?.patch(id, vec![PatchOp::SetProps { node: 1, props }]).map_err(gui_error)
    }
}

/// Takes a notification away, from the screen and from the system's list,
/// and returns once it has gone. Its last event, Close, waits for
/// Wait-GuiEvent. A number no notification shown has is a non-terminating
/// GuiNoNotification error.
///
/// # Examples
/// Remove-GuiNotification -Notification $n.Id
/// Get-GuiNotification | Remove-GuiNotification
#[cmdlet(verb = "Remove", noun = "GuiNotification")]
#[derive(Default)]
pub struct RemoveGuiNotification {
    /// The notification's number, as Show-GuiNotification writes it.
    #[param(mandatory, position = 0, value_from_pipeline_by_property_name, alias = ["Id"])]
    pub notification: u64,
}

impl Cmdlet for RemoveGuiNotification {
    fn process(&mut self, ps: &Pipeline<'_>) -> PsResult<()> {
        let id = self.notification;
        if shown_notification(id).is_none() {
            return Err(no_notification(id));
        }
        window::close(id)?;
        window::wait_closed(ps, id)
    }
}

/// Gets the notifications the egui renderer shows and has not yet seen
/// end, in any runspace of this process. Writes nothing when it shows none
/// or is not running.
///
/// # Examples
/// Get-GuiNotification
/// Get-GuiNotification | Remove-GuiNotification
#[cmdlet(verb = "Get", noun = "GuiNotification", output = ["PwrsGui.Notification"])]
#[derive(Default)]
pub struct GetGuiNotification {}

impl Cmdlet for GetGuiNotification {
    fn process(&mut self, ps: &Pipeline<'_>) -> PsResult<()> {
        let Some(live) = session::running_of(Renderer::Egui) else {
            return Ok(());
        };
        for w in live.windows(arm(ps)).map_err(gui_error)? {
            if w.kind != Kind::Notification {
                continue;
            }
            let title = if w.title.is_empty() { None } else { Some(w.title) };
            ps.write(GuiNotification { id: w.window, title, names: w.names, source_identifier: None })?;
        }
        Ok(())
    }
}

/// Registers an identity of the caller's own for Show-GuiNotification -App,
/// on Windows: its AppUserModelID under
/// HKCU\Software\Classes\AppUserModelId, with the name and the icon its
/// notifications show. The icon stays where -Icon names it, and the
/// identity is kept until Unregister-GuiNotificationApp removes it.
///
/// # Examples
/// Register-GuiNotificationApp -Id 'Contoso.Backup' -Name 'Contoso Backup' -Icon .\backup.png
#[cmdlet(verb = "Register", noun = "GuiNotificationApp", supports_should_process)]
#[derive(Default)]
pub struct RegisterGuiNotificationApp {
    /// The AppUserModelID, as Company.Product: at most 128 characters, with
    /// no spaces or backslashes.
    #[param(mandatory, position = 0)]
    pub id: String,
    /// The name its notifications show.
    #[param(mandatory, position = 1)]
    pub name: String,
    /// The icon its notifications show: a picture's path, from the current
    /// location, which is written as its full path.
    #[param]
    pub icon: Option<String>,
}

impl Cmdlet for RegisterGuiNotificationApp {
    fn process(&mut self, ps: &Pipeline<'_>) -> PsResult<()> {
        identity::checked(&self.id)?;
        let icon = match &self.icon {
            Some(p) => Some(full_path(ps, "-Icon", p)?),
            None => None,
        };
        if !ps.should_process(&self.id, "Register the notification identity")? {
            return Ok(());
        }
        identity::register(ps, &self.id, &self.name, icon.as_deref())
    }
}

/// Removes a notification identity, on Windows: its notifications from the
/// system's list, its key under HKCU\Software\Classes\AppUserModelId, and
/// its entry in Settings. For PWRS-GUI's own identity, the icon copy it
/// keeps goes too. An identity with neither key is a non-terminating
/// GuiNoNotificationApp error.
///
/// # Examples
/// Unregister-GuiNotificationApp -Id 'Contoso.Backup'
#[cmdlet(verb = "Unregister", noun = "GuiNotificationApp", supports_should_process)]
#[derive(Default)]
pub struct UnregisterGuiNotificationApp {
    /// The AppUserModelID, as Register-GuiNotificationApp registered it.
    #[param(mandatory, position = 0)]
    pub id: String,
}

impl Cmdlet for UnregisterGuiNotificationApp {
    fn process(&mut self, ps: &Pipeline<'_>) -> PsResult<()> {
        identity::checked(&self.id)?;
        if !ps.should_process(&self.id, "Remove the notification identity and its notifications")? {
            return Ok(());
        }
        identity::unregister(ps, &self.id)
    }
}

#[cfg(windows)]
mod identity {
    //! Windows' notification identities: an AppUserModelID registered for
    //! the user under HKCU\Software\Classes\AppUserModelId, with the name
    //! and icon its toasts show.

    use crate::window::script;
    use pwrs::prelude::*;
    use pwrs_gui_client::proto::notification::OWN_APP;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicBool, Ordering};

    /// PWRS-GUI's icon, which its own identity's toasts show from a copy.
    const OWN_ICON: &[u8] = include_bytes!("../assets/PWRS-GUI-256.png");

    /// PWRS-GUI's own identity is registered in this process.
    static REGISTERED: AtomicBool = AtomicBool::new(false);

    /// Writes an identity's key, its display name and its icon, or with no
    /// icon none.
    const REGISTER: &str = r#"
param([string] $Id, [string] $Name, [string] $Icon)
$key = [Microsoft.Win32.Registry]::CurrentUser.CreateSubKey("Software\Classes\AppUserModelId\$Id")
try {
    $key.SetValue('DisplayName', $Name)
    if ($Icon) { $key.SetValue('IconUri', $Icon) } else { $key.DeleteValue('IconUri', $false) }
} finally {
    $key.Dispose()
}
"#;

    /// Removes an identity's key and its entry in Settings, writing whether
    /// either was there.
    const UNREGISTER: &str = r#"
param([string] $Id)
$user = [Microsoft.Win32.Registry]::CurrentUser
$found = $false
foreach ($path in "Software\Classes\AppUserModelId\$Id", "Software\Microsoft\Windows\CurrentVersion\Notifications\Settings\$Id") {
    $key = $user.OpenSubKey($path)
    if ($null -ne $key) {
        $key.Dispose()
        $user.DeleteSubKeyTree($path)
        $found = $true
    }
}
$found
"#;

    /// Fails unless `id` is an AppUserModelID a registry key can be named
    /// after: 1 to 128 characters, none a space, a backslash or a control
    /// character.
    pub(super) fn checked(id: &str) -> PsResult<()> {
        let fits = !id.is_empty() && id.chars().count() <= 128 && !id.chars().any(|c| c == ' ' || c == '\\' || c.is_control());
        if fits {
            return Ok(());
        }
        let text = format!("-Id is {id:?}; an AppUserModelID is 1 to 128 characters, with no spaces, backslashes or control characters");
        Err(PsError::new(ErrorCategory::InvalidArgument, "GuiBadAppId", text).terminating())
    }

    /// Writes identity `id`'s key with `name` and `icon`.
    pub(super) fn register(ps: &Pipeline<'_>, id: &str, name: &str, icon: Option<&str>) -> PsResult<()> {
        let icon = match icon {
            Some(i) => i.into_ps()?,
            None => PsObject::null(),
        };
        script(REGISTER)?.call(ps, &[id.into_ps()?, name.into_ps()?, icon])?;
        Ok(())
    }

    /// Takes identity `id`'s notifications from the system's list, then
    /// removes its keys and, for PWRS-GUI's own, its icon copy.
    pub(super) fn unregister(ps: &Pipeline<'_>, id: &str) -> PsResult<()> {
        pwrs_gui_egui::forget_app(id).map_err(|e| PsError::new(ErrorCategory::ResourceUnavailable, "GuiNotificationsKept", e).terminating())?;
        let answer = script(UNREGISTER)?.call(ps, &[id.into_ps()?])?;
        let had_keys = match answer.first() {
            Some(found) => bool::from_ps(found)?,
            None => return Err(PsError::new(ErrorCategory::InvalidResult, "GuiNotificationIdentity", format!("the registry gave no answer when {id}'s keys were removed")).terminating()),
        };
        let had_icon = id == OWN_APP && remove_own_icon()?;
        if id == OWN_APP {
            REGISTERED.store(false, Ordering::Release);
        }
        if had_keys || had_icon {
            return Ok(());
        }
        Err(PsError::new(ErrorCategory::ObjectNotFound, "GuiNoNotificationApp", format!("no notification identity {id} is registered for this user")))
    }

    /// Where PWRS-GUI's own identity keeps the copy of its icon its toasts
    /// show.
    fn own_icon_path() -> PsResult<PathBuf> {
        match std::env::var_os("LOCALAPPDATA") {
            Some(base) => Ok(PathBuf::from(base).join("PWRS-GUI").join("Notifications").join("PWRS-GUI.png")),
            None => Err(PsError::new(ErrorCategory::ResourceUnavailable, "GuiNotificationIdentity", "LOCALAPPDATA is not set, so PWRS-GUI's notification icon has no folder").terminating()),
        }
    }

    /// Registers PWRS-GUI's own identity for the user, once a process,
    /// copying its icon where its toasts read it from when the copy there
    /// is missing or differs.
    pub(super) fn own(ps: &Pipeline<'_>) -> PsResult<()> {
        if REGISTERED.load(Ordering::Acquire) {
            return Ok(());
        }
        let path = own_icon_path()?;
        let failed = |e: std::io::Error| PsError::new(ErrorCategory::WriteError, "GuiNotificationIdentity", format!("PWRS-GUI's notification icon cannot be copied to {}: {e}", path.display())).terminating();
        let current = match std::fs::read(&path) {
            Ok(bytes) => bytes == OWN_ICON,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => false,
            Err(e) => return Err(failed(e)),
        };
        if !current {
            if let Some(folder) = path.parent() {
                std::fs::create_dir_all(folder).map_err(failed)?;
            }
            std::fs::write(&path, OWN_ICON).map_err(failed)?;
        }
        register(ps, OWN_APP, OWN_APP, Some(&path.to_string_lossy()))?;
        REGISTERED.store(true, Ordering::Release);
        Ok(())
    }

    /// Removes the copy of PWRS-GUI's icon, and its folder once empty;
    /// whether the copy was there.
    fn remove_own_icon() -> PsResult<bool> {
        let path = own_icon_path()?;
        let failed = |e: std::io::Error| PsError::new(ErrorCategory::WriteError, "GuiNotificationIdentity", format!("{} cannot be removed: {e}", path.display())).terminating();
        match std::fs::remove_file(&path) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(false),
            Err(e) => return Err(failed(e)),
        }
        if let Some(folder) = path.parent() {
            match std::fs::remove_dir(folder) {
                Ok(()) => {}
                Err(e) if e.kind() == std::io::ErrorKind::DirectoryNotEmpty => {}
                Err(e) => return Err(failed(e)),
            }
        }
        Ok(true)
    }
}

#[cfg(not(windows))]
mod identity {
    //! Notification identities are Windows' own: elsewhere a notification
    //! shows under the app name -App gives, and there is nothing to
    //! register.

    use pwrs::prelude::*;

    /// The error of a command that registers identities, where there are
    /// none to register.
    fn windows_only() -> PsError {
        let text = "notification identities are registered on Windows alone; on this system Show-GuiNotification -App names the app its notifications show";
        PsError::new(ErrorCategory::NotImplemented, "GuiNotWindows", text).terminating()
    }

    pub(super) fn checked(_id: &str) -> PsResult<()> {
        Err(windows_only())
    }

    pub(super) fn register(_ps: &Pipeline<'_>, _id: &str, _name: &str, _icon: Option<&str>) -> PsResult<()> {
        Err(windows_only())
    }

    pub(super) fn unregister(_ps: &Pipeline<'_>, _id: &str) -> PsResult<()> {
        Err(windows_only())
    }

    /// No identity to register for PWRS-GUI's own notifications here.
    pub(super) fn own(_ps: &Pipeline<'_>) -> PsResult<()> {
        Ok(())
    }
}
