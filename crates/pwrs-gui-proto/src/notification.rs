//! A notification as the egui renderer reads it from its node
//! ([`Kind::Notification`]): the parts each platform maps to what it shows
//! ([`Content`]), and the toast document Windows shows it as
//! ([`Content::toast_xml`]), whose buttons and body name their nodes in
//! their activation arguments ([`Activation`]).

use crate::{Kind, Node, NodeId, Value, markdown};
use std::collections::BTreeMap;

/// The identity notifications are shown under unless they name another:
/// PWRS-GUI's own, on Windows the AppUserModelID it registers for the user.
pub const OWN_APP: &str = "PWRS-GUI";

/// The most buttons a Windows toast shows, as its schema allows.
pub const MOST_BUTTONS: usize = 5;

/// The most inputs a Windows toast shows, as its schema allows.
pub const MOST_INPUTS: usize = 5;

/// The context of the `Error` a renderer sends before it closes a
/// notification whose input its platform has no box for, which the
/// session refuses as an input, apart from other failures.
pub const INPUT_REFUSED: &str = "NotifyInput";

/// The context of the `Error` a renderer sends before it closes a
/// notification the person did not allow, as macOS asks the first time an
/// app shows one, which the session refuses apart from other failures.
pub const DENIED: &str = "NotifyDenied";

/// The activation argument of a click on a toast's body.
pub const BODY_ARGUMENT: &str = "pwrs-gui:body";

/// The activation argument of a click on a toast's header.
pub const HEADER_ARGUMENT: &str = "pwrs-gui:header";

/// What the activation argument of a toast's button begins with, before
/// its node's number.
pub const BUTTON_ARGUMENT: &str = "pwrs-gui:button:";

/// What a notification shows, read from its node's properties and
/// children by [`Content::of`].
#[derive(Clone, Debug, PartialEq)]
pub struct Content {
    /// The identity it is shown under, from `app`: on Windows the
    /// AppUserModelID of an app registered for the user. `None` shows it
    /// under PWRS-GUI's own.
    pub app: Option<String>,
    /// Its first line, from `title`.
    pub title: Option<String>,
    /// The text below it, from `body`.
    pub body: Option<String>,
    /// The full path of a picture shown in it, from `image`.
    pub image: Option<String>,
    /// The full path of a picture shown across its top, from `heroImage`.
    pub hero: Option<String>,
    /// The full path of a picture shown in place of the app's icon, from
    /// `appLogo`.
    pub logo: Option<String>,
    /// Its buttons, its `Button` children, in order.
    pub buttons: Vec<Button>,
    /// Its inputs, its `TextBox` and `ComboBox` children, in order.
    pub inputs: Vec<Input>,
    /// A progress bar, from `progress`.
    pub progress: Option<Progress>,
    /// The sound it makes, from `audio`; `None` for the platform's own.
    pub audio: Option<Audio>,
    /// What it is for, from `scenario`, which decides how long it stays.
    pub scenario: Scenario,
    /// The title of the group it is shown under, from `header`.
    pub header: Option<String>,
    /// A toast document of the caller's own, from `xml`, which Windows
    /// shows in place of every part above.
    pub xml: Option<String>,
    /// Shown without a banner, from `quiet`: on Windows in the notification
    /// center alone, on Linux and FreeBSD at low urgency, which most
    /// servers keep out of the way.
    pub quiet: bool,
}

/// One of a notification's buttons.
#[derive(Clone, Debug, PartialEq)]
pub struct Button {
    pub node: NodeId,
    pub text: String,
    /// The input the button stands beside, as a send button beside a reply
    /// box, from its `input`, that input's node.
    pub input: Option<NodeId>,
}

/// One of a notification's inputs: a text box, or a choice among `choices`
/// when it has any.
#[derive(Clone, Debug, PartialEq)]
pub struct Input {
    pub node: NodeId,
    /// The text above it, from `title`.
    pub title: Option<String>,
    /// The text shown in an empty text box, from `placeholder`.
    pub placeholder: Option<String>,
    /// The choices of a `ComboBox`, from its `items`; none for a text box.
    pub choices: Vec<String>,
    /// The text a text box opens with, from `text`, or the row of the
    /// choice a `ComboBox` opens with, from `selected`.
    pub opens_with: Option<Opening>,
}

/// What an input opens with.
#[derive(Clone, Debug, PartialEq)]
pub enum Opening {
    Text(String),
    Choice(usize),
}

/// A notification's progress bar.
#[derive(Clone, Debug, PartialEq)]
pub struct Progress {
    /// How far along, from 0 to 1; `None` for a bar that moves without
    /// saying how far.
    pub value: Option<f64>,
    /// The text above the bar.
    pub title: Option<String>,
    /// The text below the bar at its left, as Downloading.
    pub status: Option<String>,
    /// The text below the bar at its right, in place of the percentage.
    pub value_text: Option<String>,
}

/// A notification's sound.
#[derive(Clone, Debug, PartialEq)]
pub struct Audio {
    /// The sound's name, as Windows names its notification sounds
    /// ([`SOUNDS`]); `None` for the platform's own.
    pub sound: Option<String>,
    /// No sound at all.
    pub silent: bool,
    /// The sound plays until the notification is answered or dismissed,
    /// which Windows does for alarms and calls alone.
    pub repeat: bool,
}

/// What a notification is for.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Scenario {
    /// A notification that times out as any other does.
    #[default]
    Default,
    /// A reminder, which stays on the screen until it is answered.
    Reminder,
    /// An alarm, which stays on the screen and repeats its sound.
    Alarm,
    /// An incoming call, which stays on the screen and repeats its sound.
    IncomingCall,
    /// An urgent notification, which breaks through Do Not Disturb where the
    /// person allows it.
    Urgent,
}

impl Scenario {
    /// The scenario named `name`, as a node's `scenario` writes it.
    pub fn named(name: &str) -> Option<Scenario> {
        [Scenario::Default, Scenario::Reminder, Scenario::Alarm, Scenario::IncomingCall, Scenario::Urgent].into_iter().find(|s| s.name() == name)
    }

    /// Its name, as a node's `scenario` writes it.
    pub fn name(self) -> &'static str {
        match self {
            Scenario::Default => "default",
            Scenario::Reminder => "reminder",
            Scenario::Alarm => "alarm",
            Scenario::IncomingCall => "incomingCall",
            Scenario::Urgent => "urgent",
        }
    }
}

/// The sounds Windows names for notifications, each played from
/// `ms-winsoundevent:Notification.<name>`, and while it repeats from
/// `Notification.Looping.<name>`.
pub const SOUNDS: &[&str] = &[
    "Default", "IM", "Mail", "Reminder", "SMS", "Alarm", "Alarm2", "Alarm3", "Alarm4", "Alarm5", "Alarm6", "Alarm7", "Alarm8", "Alarm9", "Alarm10", "Call", "Call2", "Call3", "Call4", "Call5",
    "Call6", "Call7", "Call8", "Call9", "Call10",
];

/// What a click on a toast acted on, read from its activation argument.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Activation {
    /// Its body.
    Body,
    /// The button whose node this is.
    Button(NodeId),
    /// Its header.
    Header,
    /// An argument of a toast document of the caller's own.
    Other(String),
}

impl Activation {
    /// What the activation argument `arguments` names. An argument that
    /// opens as a button's does, [`BUTTON_ARGUMENT`], and names no node
    /// number is refused, naming it.
    pub fn of(arguments: &str) -> Result<Activation, String> {
        if arguments == BODY_ARGUMENT {
            return Ok(Activation::Body);
        }
        if arguments == HEADER_ARGUMENT {
            return Ok(Activation::Header);
        }
        match arguments.strip_prefix(BUTTON_ARGUMENT) {
            Some(number) => number.parse::<NodeId>().map(Activation::Button).map_err(|e| format!("the activation argument {arguments} names no button's node: {e}")),
            None => Ok(Activation::Other(arguments.to_string())),
        }
    }
}

/// A node's text property `key`: none when it is absent or null.
fn text(node: &Node, key: &str) -> Result<Option<String>, String> {
    text_in(&node.props, key)
}

/// The text under `key` in `map`: none when it is absent or null.
fn text_in(map: &BTreeMap<String, Value>, key: &str) -> Result<Option<String>, String> {
    match map.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Str(s)) => Ok(Some(s.clone())),
        Some(other) => Err(format!("{key} is text, not {other:?}")),
    }
}

/// The flag under `key` in `map`: false when it is absent or null.
fn flag(map: &BTreeMap<String, Value>, key: &str) -> Result<bool, String> {
    match map.get(key) {
        None | Some(Value::Null) => Ok(false),
        Some(Value::Bool(b)) => Ok(*b),
        Some(other) => Err(format!("{key} is true or false, not {other:?}")),
    }
}

/// A picture's full path from the property `key`: none when absent. A
/// path that names a share is refused, since opening it would send the
/// user's credentials to the share's host.
fn picture_path(node: &Node, key: &str) -> Result<Option<String>, String> {
    let Some(path) = text(node, key)? else {
        return Ok(None);
    };
    if markdown::names_a_share(&path) {
        return Err(format!("{key} is {path}, and {}", markdown::SHARED));
    }
    Ok(Some(path))
}

/// A button from a notification's `Button` child.
fn button_of(c: &Node) -> Result<Button, String> {
    let input = match c.props.get("input") {
        None | Some(Value::Null) => None,
        Some(Value::Int(n)) => Some(NodeId::try_from(*n).map_err(|e| format!("button {}'s input {n}: {e}", c.id))?),
        Some(other) => return Err(format!("button {}'s input is the node of an input, not {other:?}", c.id)),
    };
    let text = text(c, "text")?.ok_or_else(|| format!("button {} has no text", c.id))?;
    Ok(Button { node: c.id, text, input })
}

/// An input from a notification's `ComboBox` child, a choice among its
/// items.
fn choice_of(c: &Node) -> Result<Input, String> {
    let choices = match c.props.get("items") {
        Some(Value::List(items)) => items.iter().map(|i| i.as_str().map(str::to_string).ok_or_else(|| format!("input {}'s choices are text, and one is {i:?}", c.id))).collect::<Result<Vec<String>, String>>()?,
        None | Some(Value::Null) => Vec::new(),
        Some(other) => return Err(format!("input {}'s items are a list of text, not {other:?}", c.id)),
    };
    if choices.is_empty() {
        return Err(format!("input {} is a choice with no choices", c.id));
    }
    let opens_with = match c.props.get("selected") {
        None | Some(Value::Null) => None,
        Some(Value::Int(r)) => {
            let row = usize::try_from(*r).map_err(|e| format!("input {}'s selected row {r}: {e}", c.id))?;
            if row >= choices.len() {
                return Err(format!("input {}'s selected row {row} is beyond its {} choices", c.id, choices.len()));
            }
            Some(Opening::Choice(row))
        }
        Some(other) => return Err(format!("input {}'s selected is a row number, not {other:?}", c.id)),
    };
    Ok(Input { node: c.id, title: text(c, "title")?, placeholder: None, choices, opens_with })
}

impl Content {
    /// What the notification `node` shows, a [`Kind::Notification`]: its
    /// properties as [`Content`] names them, and its `Button`, `TextBox`
    /// and `ComboBox` children. One with neither a title nor a body, and no
    /// `xml`, is refused, as are a button standing beside a node that is no
    /// input and any other child.
    pub fn of(node: &Node) -> Result<Content, String> {
        if node.kind != Kind::Notification {
            return Err(format!("a notification's root is a Notification, not a {:?}", node.kind));
        }
        let progress = match node.props.get("progress") {
            None | Some(Value::Null) => None,
            Some(Value::Map(m)) => Some(progress_of(m)?),
            Some(other) => return Err(format!("progress is a map of its value, title, status and valueText, not {other:?}")),
        };
        let audio = match node.props.get("audio") {
            None | Some(Value::Null) => None,
            Some(Value::Map(m)) => Some(audio_of(m)?),
            Some(other) => return Err(format!("audio is a map of its sound, silent and loop, not {other:?}")),
        };
        let scenario = match text(node, "scenario")? {
            None => Scenario::Default,
            Some(name) => Scenario::named(&name).ok_or_else(|| format!("scenario is default, reminder, alarm, incomingCall or urgent, not {name}"))?,
        };
        let mut buttons = Vec::new();
        let mut inputs = Vec::new();
        for c in &node.children {
            match c.kind {
                Kind::Button => buttons.push(button_of(c)?),
                Kind::TextBox => inputs.push(Input { node: c.id, title: text(c, "title")?, placeholder: text(c, "placeholder")?, choices: Vec::new(), opens_with: text(c, "text")?.map(Opening::Text) }),
                Kind::ComboBox => inputs.push(choice_of(c)?),
                other => return Err(format!("a notification holds buttons and inputs, and node {} is a {other:?}", c.id)),
            }
        }
        let stray = buttons.iter().find_map(|b| b.input.filter(|i| !inputs.iter().any(|n| n.node == *i)).map(|i| (b.node, i)));
        if let Some((button, input)) = stray {
            return Err(format!("button {button} stands beside node {input}, which is not one of the notification's inputs"));
        }
        let content = Content {
            app: text(node, "app")?,
            title: text(node, "title")?,
            body: text(node, "body")?,
            image: picture_path(node, "image")?,
            hero: picture_path(node, "heroImage")?,
            logo: picture_path(node, "appLogo")?,
            buttons,
            inputs,
            progress,
            audio,
            scenario,
            header: text(node, "header")?,
            xml: text(node, "xml")?,
            quiet: flag(&node.props, "quiet")?,
        };
        if content.xml.is_none() && content.title.is_none() && content.body.is_none() {
            return Err(String::from("a notification shows a title, a body or both"));
        }
        Ok(content)
    }

    /// The input whose node is `node`.
    pub fn input(&self, node: NodeId) -> Option<&Input> {
        self.inputs.iter().find(|i| i.node == node)
    }

    /// The toast document Windows shows this as: the caller's own `xml`
    /// when it has one, else a generic toast of its parts. The title, the
    /// body and the progress bar's parts given are bound to the data
    /// [`Content::toast_data`] gives, so an update changes them in place.
    /// A body click and each button carry the arguments [`Activation`]
    /// reads, and each input's id is its node's number. Refused past
    /// [`MOST_BUTTONS`] or [`MOST_INPUTS`], for a sound Windows does not
    /// name, and for text XML cannot carry.
    pub fn toast_xml(&self) -> Result<String, String> {
        if let Some(xml) = &self.xml {
            return Ok(xml.clone());
        }
        if self.buttons.len() > MOST_BUTTONS {
            return Err(format!("a Windows notification shows {MOST_BUTTONS} buttons at most, and this one has {}", self.buttons.len()));
        }
        if self.inputs.len() > MOST_INPUTS {
            return Err(format!("a Windows notification shows {MOST_INPUTS} inputs at most, and this one has {}", self.inputs.len()));
        }
        let mut x = String::from("<toast");
        attribute(&mut x, "launch", BODY_ARGUMENT)?;
        if self.scenario != Scenario::Default {
            attribute(&mut x, "scenario", self.scenario.name())?;
        }
        x.push_str("><visual><binding template=\"ToastGeneric\">");
        if self.title.is_some() {
            x.push_str("<text>{title}</text>");
        }
        if self.body.is_some() {
            x.push_str("<text>{body}</text>");
        }
        for (path, placement) in [(&self.logo, Some("appLogoOverride")), (&self.hero, Some("hero")), (&self.image, None)] {
            if let Some(p) = path {
                x.push_str("<image");
                if let Some(at) = placement {
                    attribute(&mut x, "placement", at)?;
                }
                attribute(&mut x, "src", &file_uri(p))?;
                x.push_str("/>");
            }
        }
        if let Some(p) = &self.progress {
            progress_xml(&mut x, p);
        }
        x.push_str("</binding></visual>");
        if !self.inputs.is_empty() || !self.buttons.is_empty() {
            x.push_str("<actions>");
            for i in &self.inputs {
                input_xml(&mut x, i)?;
            }
            for b in &self.buttons {
                x.push_str("<action");
                attribute(&mut x, "content", &b.text)?;
                attribute(&mut x, "arguments", &format!("{BUTTON_ARGUMENT}{}", b.node))?;
                if let Some(i) = b.input {
                    attribute(&mut x, "hint-inputId", &i.to_string())?;
                }
                x.push_str("/>");
            }
            x.push_str("</actions>");
        }
        if let Some(a) = &self.audio {
            audio_xml(&mut x, a)?;
        }
        if let Some(h) = &self.header {
            x.push_str("<header");
            attribute(&mut x, "id", h)?;
            attribute(&mut x, "title", h)?;
            attribute(&mut x, "arguments", HEADER_ARGUMENT)?;
            x.push_str("/>");
        }
        x.push_str("</toast>");
        Ok(x)
    }

    /// The values the bound parts of [`Content::toast_xml`]'s document
    /// show, by the names it binds: `title`, `body`, and the progress bar's
    /// `progressValue`, a number from 0 to 1 or `indeterminate`, and the
    /// `progressTitle`, `progressValueString` and `progressStatus` given.
    pub fn toast_data(&self) -> Vec<(&'static str, String)> {
        let mut data = Vec::new();
        if let Some(t) = &self.title {
            data.push(("title", t.clone()));
        }
        if let Some(b) = &self.body {
            data.push(("body", b.clone()));
        }
        if let Some(p) = &self.progress {
            let value = match p.value {
                Some(v) => v.to_string(),
                None => String::from("indeterminate"),
            };
            data.push(("progressValue", value));
            for (name, given) in [("progressStatus", &p.status), ("progressTitle", &p.title), ("progressValueString", &p.value_text)] {
                if let Some(t) = given {
                    data.push((name, t.clone()));
                }
            }
        }
        data
    }
}

/// Appends a progress bar's element to `x`: its value bound, each of its
/// texts given bound, and a status not given written empty, since the
/// toast schema requires one.
fn progress_xml(x: &mut String, p: &Progress) {
    x.push_str("<progress");
    if p.title.is_some() {
        x.push_str(" title=\"{progressTitle}\"");
    }
    x.push_str(" value=\"{progressValue}\"");
    if p.value_text.is_some() {
        x.push_str(" valueStringOverride=\"{progressValueString}\"");
    }
    match &p.status {
        Some(_) => x.push_str(" status=\"{progressStatus}\"/>"),
        None => x.push_str(" status=\"\"/>"),
    }
}

/// Appends an input's element to `x`: a text box, or a selection of its
/// choices by row.
fn input_xml(x: &mut String, i: &Input) -> Result<(), String> {
    x.push_str("<input");
    attribute(x, "id", &i.node.to_string())?;
    attribute(x, "type", if i.choices.is_empty() { "text" } else { "selection" })?;
    if let Some(t) = &i.title {
        attribute(x, "title", t)?;
    }
    if let Some(p) = &i.placeholder {
        attribute(x, "placeHolderContent", p)?;
    }
    match &i.opens_with {
        Some(Opening::Text(t)) => attribute(x, "defaultInput", t)?,
        Some(Opening::Choice(row)) => attribute(x, "defaultInput", &row.to_string())?,
        None => {}
    }
    if i.choices.is_empty() {
        x.push_str("/>");
        return Ok(());
    }
    x.push('>');
    for (row, choice) in i.choices.iter().enumerate() {
        x.push_str("<selection");
        attribute(x, "id", &row.to_string())?;
        attribute(x, "content", choice)?;
        x.push_str("/>");
    }
    x.push_str("</input>");
    Ok(())
}

/// Appends a sound's element to `x`, refusing a sound Windows does not
/// name.
fn audio_xml(x: &mut String, a: &Audio) -> Result<(), String> {
    x.push_str("<audio");
    if let Some(name) = &a.sound {
        let Some(sound) = SOUNDS.iter().find(|s| s.eq_ignore_ascii_case(name)) else {
            return Err(format!("a Windows notification plays one of the sounds {}, not {name}", SOUNDS.join(", ")));
        };
        let looping = if a.repeat { "Looping." } else { "" };
        attribute(x, "src", &format!("ms-winsoundevent:Notification.{looping}{sound}"))?;
    }
    if a.repeat {
        attribute(x, "loop", "true")?;
    }
    if a.silent {
        attribute(x, "silent", "true")?;
    }
    x.push_str("/>");
    Ok(())
}

/// A progress bar from a node's `progress` map.
fn progress_of(m: &BTreeMap<String, Value>) -> Result<Progress, String> {
    if let Some(key) = m.keys().find(|k| !matches!(k.as_str(), "value" | "title" | "status" | "valueText")) {
        return Err(format!("progress takes value, title, status and valueText, and was given {key}"));
    }
    let value = match m.get("value") {
        None | Some(Value::Null) => None,
        Some(v) => match v.as_number() {
            Some(n) if (0.0..=1.0).contains(&n) => Some(n),
            Some(n) => return Err(format!("progress's value is {n}; it runs from 0 to 1")),
            None => return Err(format!("progress's value is a number from 0 to 1, not {v:?}")),
        },
    };
    Ok(Progress { value, title: text_in(m, "title")?, status: text_in(m, "status")?, value_text: text_in(m, "valueText")? })
}

/// A sound from a node's `audio` map.
fn audio_of(m: &BTreeMap<String, Value>) -> Result<Audio, String> {
    if let Some(key) = m.keys().find(|k| !matches!(k.as_str(), "sound" | "silent" | "loop")) {
        return Err(format!("audio takes sound, silent and loop, and was given {key}"));
    }
    Ok(Audio { sound: text_in(m, "sound")?, silent: flag(m, "silent")?, repeat: flag(m, "loop")? })
}

/// Appends ` name="value"` to `x`, the value escaped as XML takes it.
fn attribute(x: &mut String, name: &str, value: &str) -> Result<(), String> {
    x.push(' ');
    x.push_str(name);
    x.push_str("=\"");
    x.push_str(&escaped(value)?);
    x.push('"');
    Ok(())
}

/// `text` as XML takes it in an attribute or between tags: `&`, `<`, `>`,
/// `"` and `'` written as entities. A character XML 1.0 cannot carry, a
/// control character other than a tab, a line feed or a carriage return,
/// is refused, naming it.
pub fn escaped(text: &str) -> Result<String, String> {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            '\t' | '\n' | '\r' => out.push(c),
            c if (c as u32) < 0x20 || c == '\u{FFFE}' || c == '\u{FFFF}' => return Err(format!("{text:?} holds U+{:04X}, which a notification's XML cannot carry", c as u32)),
            c => out.push(c),
        }
    }
    Ok(out)
}

/// A full path as the file URI a toast's image takes: `file:///`, then the
/// path with each backslash a slash and each byte outside the letters,
/// digits, `-._~` and the separators `/` and `:` written as `%` and its
/// two hex digits.
pub fn file_uri(path: &str) -> String {
    let mut out = String::from("file:///");
    for b in path.trim_start_matches(['/', '\\']).bytes() {
        match b {
            b'\\' => out.push('/'),
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' | b'/' | b':' => out.push(b as char),
            other => out.push_str(&format!("%{other:02X}")),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text_node(id: NodeId, kind: Kind, props: &[(&str, Value)]) -> Node {
        let mut n = Node::new(id, kind);
        for (k, v) in props {
            n.props.insert(k.to_string(), v.clone());
        }
        n
    }

    fn s(text: &str) -> Value {
        Value::Str(text.into())
    }

    fn map(entries: &[(&str, Value)]) -> Value {
        Value::Map(entries.iter().map(|(k, v)| (k.to_string(), v.clone())).collect())
    }

    #[test]
    fn a_notification_reads_its_parts_buttons_and_inputs() {
        let node = text_node(1, Kind::Notification, &[("title", s("Backup done")), ("body", s("12 files")), ("scenario", s("reminder")), ("header", s("Backups"))])
            .child(text_node(2, Kind::TextBox, &[("placeholder", s("Type a reply")), ("title", s("Reply"))]))
            .child(text_node(3, Kind::Button, &[("text", s("Send")), ("input", Value::Int(2))]))
            .child(text_node(4, Kind::ComboBox, &[("items", Value::List(vec![s("Now"), s("Later")])), ("selected", Value::Int(1))]));
        let c = Content::of(&node).expect("a notification");
        assert_eq!(c.title.as_deref(), Some("Backup done"));
        assert_eq!(c.scenario, Scenario::Reminder);
        assert_eq!(c.buttons, vec![Button { node: 3, text: String::from("Send"), input: Some(2) }]);
        assert_eq!(c.inputs[0].placeholder.as_deref(), Some("Type a reply"));
        assert_eq!(c.inputs[1].choices, vec![String::from("Now"), String::from("Later")]);
        assert_eq!(c.inputs[1].opens_with, Some(Opening::Choice(1)));
        assert_eq!(c.input(4).map(|i| i.node), Some(4));
    }

    #[test]
    fn a_notification_needs_a_title_or_a_body_and_knows_its_children() {
        let empty = Node::new(1, Kind::Notification);
        assert!(Content::of(&empty).expect_err("nothing to show").contains("title, a body or both"));
        assert!(Content::of(&text_node(1, Kind::Notification, &[("xml", s("<toast/>"))])).is_ok(), "a document of the caller's own needs neither");
        let stray = text_node(1, Kind::Notification, &[("title", s("t"))]).child(Node::new(2, Kind::Slider));
        assert!(Content::of(&stray).expect_err("a slider").contains("Slider"));
        let beside = text_node(1, Kind::Notification, &[("title", s("t"))]).child(text_node(2, Kind::Button, &[("text", s("Go")), ("input", Value::Int(9))]));
        assert!(Content::of(&beside).expect_err("no such input").contains("node 9"));
        assert!(Content::of(&text_node(1, Kind::Notification, &[("title", s("t")), ("scenario", s("loud"))])).is_err());
    }

    #[test]
    fn a_picture_on_a_share_is_refused() {
        let shared = text_node(1, Kind::Notification, &[("title", s("t")), ("image", s(r"\\server\pictures\a.png"))]);
        assert!(Content::of(&shared).expect_err("a share").contains("credentials"));
    }

    #[test]
    fn a_progress_bar_runs_from_0_to_1_or_moves_without_a_value() {
        let with = |value: Value| text_node(1, Kind::Notification, &[("title", s("t")), ("progress", map(&[("value", value), ("status", s("Copying"))]))]);
        let c = Content::of(&with(Value::Float(0.25))).expect("a progress bar");
        assert_eq!(c.progress.as_ref().and_then(|p| p.value), Some(0.25));
        assert_eq!(Content::of(&with(Value::Null)).expect("an indeterminate bar").progress.and_then(|p| p.value), None);
        assert!(Content::of(&with(Value::Float(1.5))).is_err());
        let data = c.toast_data();
        assert!(data.contains(&("progressValue", String::from("0.25"))));
        assert!(data.contains(&("progressStatus", String::from("Copying"))));
        assert!(!data.iter().any(|(k, _)| *k == "progressTitle"), "a title not given is not bound");
        let xml = c.toast_xml().expect("its document");
        assert!(xml.contains("<progress value=\"{progressValue}\" status=\"{progressStatus}\"/>"), "{xml}");
        let unstated = text_node(1, Kind::Notification, &[("title", s("t")), ("progress", map(&[("value", Value::Float(0.5))]))]);
        let xml = Content::of(&unstated).expect("a bar with no status").toast_xml().expect("its document");
        assert!(xml.contains("status=\"\"/>"), "a status not given is written empty, as the schema requires one: {xml}");
    }

    #[test]
    fn the_toast_binds_its_text_and_names_each_buttons_node_in_its_arguments() {
        let node = text_node(1, Kind::Notification, &[("title", s("Deploy <web01>")), ("body", s("Ready")), ("image", s(r"C:\Pictures\a b.png")), ("audio", map(&[("silent", Value::Bool(true))]))])
            .child(text_node(2, Kind::TextBox, &[("placeholder", s("Why?"))]))
            .child(text_node(3, Kind::Button, &[("text", s("Go & see")), ("input", Value::Int(2))]));
        let xml = Content::of(&node).expect("a notification").toast_xml().expect("its document");
        assert!(xml.starts_with("<toast launch=\"pwrs-gui:body\"><visual><binding template=\"ToastGeneric\"><text>{title}</text><text>{body}</text>"), "{xml}");
        assert!(xml.contains("<image src=\"file:///C:/Pictures/a%20b.png\"/>"), "{xml}");
        assert!(xml.contains("<input id=\"2\" type=\"text\" placeHolderContent=\"Why?\"/>"), "{xml}");
        assert!(xml.contains("<action content=\"Go &amp; see\" arguments=\"pwrs-gui:button:3\" hint-inputId=\"2\"/>"), "{xml}");
        assert!(xml.contains("<audio silent=\"true\"/>"), "{xml}");
        assert!(!xml.contains("web01"), "the title is bound, not written into the document");
    }

    #[test]
    fn a_choice_input_lists_its_choices_by_row_and_a_sound_takes_windows_names() {
        let node = text_node(1, Kind::Notification, &[("title", s("t")), ("audio", map(&[("sound", s("alarm2")), ("loop", Value::Bool(true))]))])
            .child(text_node(2, Kind::ComboBox, &[("items", Value::List(vec![s("5 minutes"), s("1 hour")])), ("selected", Value::Int(0)), ("title", s("Snooze"))]));
        let xml = Content::of(&node).expect("a notification").toast_xml().expect("its document");
        assert!(xml.contains("<input id=\"2\" type=\"selection\" title=\"Snooze\" defaultInput=\"0\"><selection id=\"0\" content=\"5 minutes\"/><selection id=\"1\" content=\"1 hour\"/></input>"), "{xml}");
        assert!(xml.contains("<audio src=\"ms-winsoundevent:Notification.Looping.Alarm2\" loop=\"true\"/>"), "{xml}");
        let unknown = text_node(1, Kind::Notification, &[("title", s("t")), ("audio", map(&[("sound", s("Chime"))]))]);
        assert!(Content::of(&unknown).expect("read").toast_xml().expect_err("no such sound").contains("Chime"));
    }

    #[test]
    fn windows_limits_buttons_and_inputs_to_five_each() {
        let mut node = text_node(1, Kind::Notification, &[("title", s("t"))]);
        for id in 2..8 {
            node.children.push(text_node(id, Kind::Button, &[("text", s("b"))]));
        }
        let refused = Content::of(&node).expect("read").toast_xml().expect_err("six buttons");
        assert!(refused.contains("5 buttons"), "{refused}");
    }

    #[test]
    fn the_callers_own_document_is_shown_as_it_is() {
        let own = "<toast><visual><binding template=\"ToastGeneric\"><text>Hi</text></binding></visual></toast>";
        let c = Content::of(&text_node(1, Kind::Notification, &[("xml", s(own))])).expect("a document");
        assert_eq!(c.toast_xml().expect("its document"), own);
    }

    #[test]
    fn activation_arguments_name_the_body_a_button_or_the_header() {
        let read = |a: &str| Activation::of(a).expect("an argument");
        assert_eq!(read(BODY_ARGUMENT), Activation::Body);
        assert_eq!(read("pwrs-gui:button:12"), Activation::Button(12));
        assert_eq!(read(HEADER_ARGUMENT), Activation::Header);
        assert_eq!(read("action=open&id=4"), Activation::Other(String::from("action=open&id=4")));
        assert!(Activation::of("pwrs-gui:button:x").expect_err("no node").contains("pwrs-gui:button:x"));
    }

    #[test]
    fn xml_text_is_escaped_and_a_control_character_refused() {
        assert_eq!(escaped("a<b>&\"c'").expect("escaped"), "a&lt;b&gt;&amp;&quot;c&apos;");
        assert_eq!(escaped("two\nlines").expect("escaped"), "two\nlines");
        assert!(escaped("bell\u{7}").expect_err("a bell").contains("U+0007"));
    }

    #[test]
    fn a_path_becomes_a_file_uri_with_its_unsafe_bytes_encoded() {
        assert_eq!(file_uri(r"C:\Users\Ann\My Pictures\50%.png"), "file:///C:/Users/Ann/My%20Pictures/50%25.png");
        assert_eq!(file_uri("/home/ann/pic#1.png"), "file:///home/ann/pic%231.png");
        assert_eq!(file_uri("C:\\Bilder\\Gr\u{fc}\u{df}e.png"), "file:///C:/Bilder/Gr%C3%BC%C3%9Fe.png");
    }
}
