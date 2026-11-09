//! Notifications: each a [`Kind::Notification`] tree a session shows with
//! [`ToHelper::Notify`], as a toast on Windows, whose clicks, buttons,
//! inputs, dismissals and failures come back as the notification's events
//! on its ring, as a window's do. An answer raises a `Change` for each
//! input given a value, then the `Click` of its button or the `Activate`
//! of its body, and ends the notification with `Closed`.
//!
//! The platform calls back on threads of its own, so what it reports goes
//! onto [`HEARD`], a queue of the process's own, and wakes whichever
//! renderer's loop runs now, through [`WAKE`]; that loop's next frame takes
//! it.
//!
//! [`ToHelper::Notify`]: proto::ToHelper::Notify

use arc_swap::ArcSwapOption;
use crossbeam_queue::SegQueue;
use proto::notification::{Activation, Content};
use proto::renderer::WakeLoop;
use proto::{CloseReason, EventKind, FromHelper, Kind, Node, NodeId, PatchOp, RingName, Value, WindowId};
use pwrs_gui_proto as proto;
use std::collections::BTreeMap;
use std::sync::Arc;

/// Why a notification left the screen unanswered.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Dismissal {
    /// The person dismissed it.
    User,
    /// It timed out into the system's list of notifications, where it can
    /// still be answered.
    Timeout,
    /// The session took it away.
    #[cfg_attr(target_os = "macos", allow(dead_code))]
    Hidden,
}

impl Dismissal {
    /// The value its Dismiss event gives.
    fn name(self) -> &'static str {
        match self {
            Dismissal::User => "User",
            Dismissal::Timeout => "Timeout",
            Dismissal::Hidden => "Hidden",
        }
    }
}

/// Why a notification cannot be shown: an input the platform has no box
/// for, which a script waits on and so is refused, or anything else.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Refused {
    Input(String),
    Other(String),
}

impl From<String> for Refused {
    fn from(text: String) -> Refused {
        Refused::Other(text)
    }
}

/// What the platform reported of a notification.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Heard {
    /// The person clicked its body, a button or its header: the
    /// activation's argument, and each input's value by its id, or why the
    /// values could not be read.
    Activated { notification: WindowId, arguments: String, inputs: Result<Vec<(String, String)>, String> },
    Dismissed { notification: WindowId, reason: Dismissal },
    /// It could not be shown, for the reason given.
    #[cfg_attr(target_os = "macos", allow(dead_code))]
    Failed { notification: WindowId, text: String },
    /// A notification the platform took after answering, as the macOS
    /// notification center does once the person's permission is known, now
    /// shows.
    #[cfg_attr(not(target_os = "macos"), allow(dead_code))]
    Shown { notification: WindowId },
    /// A notification the platform took after answering cannot be shown:
    /// with `denied`, because the person did not allow notifications.
    #[cfg_attr(not(target_os = "macos"), allow(dead_code))]
    Refused { notification: WindowId, denied: bool, text: String },
}

/// What the platform reported, for the loop's next frame to take.
static HEARD: SegQueue<Heard> = SegQueue::new();

/// What wakes the loop of the renderer running now.
static WAKE: ArcSwapOption<WakeLoop> = ArcSwapOption::const_empty();

/// Records what the platform reported and wakes the renderer's loop.
fn hear(heard: Heard) {
    HEARD.push(heard);
    match WAKE.load().as_ref() {
        Some(wake) => wake(),
        None => proto::trace::line(|| String::from("egui: a notification was answered while no renderer's loop could be woken")),
    }
}

/// The notifications a renderer shows, by number.
#[derive(Default)]
pub(crate) struct Notifications {
    shown: BTreeMap<WindowId, Shown>,
}

/// A notification shown: the ring its events go to, none once detached,
/// its tree and what it shows as last set, the platform's toast, and the
/// document the platform shows it from.
pub(crate) struct Shown {
    pub(crate) ring: Option<RingName>,
    pub(crate) tree: Node,
    content: Content,
    toast: platform::Toast,
    document: String,
}

impl Notifications {
    /// Shows notification `id` from `tree`, its loop woken through `wake`
    /// when the platform reports on it. Returns what the person should hear
    /// of parts the platform cannot show, and whether it shows now, or else
    /// once the platform has answered, which [`Notifications::heard`]
    /// reports; or why it cannot be shown.
    pub(crate) fn show(&mut self, id: WindowId, ring: RingName, tree: Node, wake: Option<&WakeLoop>) -> Result<(Vec<String>, bool), Refused> {
        let content = Content::of(&tree)?;
        if let Some(w) = wake {
            WAKE.store(Some(Arc::new(Arc::clone(w))));
        }
        let shown = platform::show(id, &content)?;
        self.shown.insert(id, Shown { ring: Some(ring), tree, content, toast: shown.toast, document: shown.document });
        Ok((shown.notes, !shown.pending))
    }

    /// Whether `id` is one of these.
    pub(crate) fn has(&self, id: WindowId) -> bool {
        self.shown.contains_key(&id)
    }

    /// The ring notification `id`'s events go to.
    pub(crate) fn ring(&self, id: WindowId) -> Option<RingName> {
        self.shown.get(&id).and_then(|s| s.ring.clone())
    }

    /// Stops sending notification `id`'s events, returning its ring.
    pub(crate) fn detach(&mut self, id: WindowId) -> Option<RingName> {
        self.shown.get_mut(&id).and_then(|s| s.ring.take())
    }

    /// Takes notification `id` away from the screen and the system's list
    /// of notifications. Returns it, and what the platform answered.
    pub(crate) fn remove(&mut self, id: WindowId) -> Option<(Shown, Result<(), String>)> {
        let shown = self.shown.remove(&id)?;
        let hidden = platform::hide(&shown.toast);
        Some((shown, hidden))
    }

    /// Forgets every notification, leaving each in the system's list,
    /// where the person can still read it.
    pub(crate) fn take_all(&mut self) -> Vec<(WindowId, Shown)> {
        std::mem::take(&mut self.shown).into_iter().collect()
    }

    /// Changes notification `id`'s root by `ops`, which set its
    /// properties, and shows what changed where the platform can: its
    /// title, its body and its progress bar. Returns what could not be
    /// changed.
    pub(crate) fn patch(&mut self, id: WindowId, ops: Vec<PatchOp>) -> Vec<String> {
        let Some(s) = self.shown.get_mut(&id) else {
            return vec![format!("no notification {id} is shown")];
        };
        let mut tree = s.tree.clone();
        for op in ops {
            match op {
                PatchOp::SetProps { node, props } if node == tree.id => tree.props.extend(props),
                other => return vec![format!("a notification takes changes to its own properties, not {other:?}")],
            }
        }
        let content = match Content::of(&tree) {
            Ok(c) => c,
            Err(why) => return vec![why],
        };
        if let Err(why) = platform::update(&s.toast, &content) {
            return vec![why];
        }
        s.tree = tree;
        s.content = content;
        Vec::new()
    }

    /// The events of what the platform reported since the last call, each
    /// with the ring it goes to; a notification that ended is forgotten.
    pub(crate) fn heard(&mut self) -> Vec<(RingName, FromHelper)> {
        let mut out = Vec::new();
        while let Some(h) = HEARD.pop() {
            let id = match &h {
                Heard::Activated { notification, .. }
                | Heard::Dismissed { notification, .. }
                | Heard::Failed { notification, .. }
                | Heard::Shown { notification }
                | Heard::Refused { notification, .. } => *notification,
            };
            let Some(s) = self.shown.get(&id) else {
                proto::trace::line(|| format!("egui: notification {id} was answered after it ended: {h:?}"));
                continue;
            };
            let (events, ends) = events_of(id, s.tree.id, &s.content, h);
            if let Some(ring) = &s.ring {
                out.extend(events.into_iter().map(|e| (ring.clone(), e)));
            }
            if ends {
                self.shown.remove(&id);
            }
        }
        out
    }

    /// Answers notification `id` as a person would, and takes it away from
    /// the screen as the person's answer does. With `name` empty, `kind` is
    /// Activate on its body or Dismiss, `value` then the reason; with a
    /// button's name it is Click. A map `value` of an Activate or a Click
    /// gives inputs' values by their names, a choice's by its text or its
    /// row. Returns the events the answer raises.
    pub(crate) fn test_event(&mut self, id: WindowId, name: &str, kind: EventKind, value: &Value) -> Result<Vec<FromHelper>, String> {
        let s = self.shown.get(&id).ok_or_else(|| format!("no notification {id} is shown"))?;
        let heard = match (name.is_empty(), kind) {
            (true, EventKind::Dismiss) => {
                let reason = match value.as_str() {
                    None | Some("User") => Dismissal::User,
                    Some("Timeout") => Dismissal::Timeout,
                    Some(other) => return Err(format!("a test dismisses a notification as User or Timeout, not {other}")),
                };
                Heard::Dismissed { notification: id, reason }
            }
            (true, EventKind::Activate) => Heard::Activated { notification: id, arguments: String::from(proto::notification::BODY_ARGUMENT), inputs: Ok(test_inputs(&s.tree, &s.content, value)?) },
            (true, other) => return Err(format!("a notification's body raises Activate, and the notification Dismiss, not {other:?}")),
            (false, EventKind::Click) => {
                let button = s.tree.find_named(name).ok_or_else(|| format!("notification {id} has no button named {name}; its widgets are {}", s.tree.names().join(", ")))?;
                if button.kind != Kind::Button {
                    return Err(format!("{name} in notification {id} is a {:?}; a test clicks its buttons", button.kind));
                }
                let arguments = format!("{}{}", proto::notification::BUTTON_ARGUMENT, button.id);
                Heard::Activated { notification: id, arguments, inputs: Ok(test_inputs(&s.tree, &s.content, value)?) }
            }
            (false, other) => return Err(format!("a notification's button raises Click, not {other:?}")),
        };
        let (events, ends) = events_of(id, s.tree.id, &s.content, heard);
        if ends && let Some((_, Err(problem))) = self.remove(id) {
            proto::trace::line(|| format!("egui: notification {id}, answered by a test, could not be taken away: {problem}"));
        }
        Ok(events)
    }

    /// Each notification shown, as a list of windows names it: its title,
    /// empty for one with none, and its buttons' and inputs' names.
    pub(crate) fn summaries(&self) -> Vec<proto::WindowSummary> {
        let titled = |s: &Shown| match &s.content.title {
            Some(t) => t.clone(),
            None => String::new(),
        };
        self.shown.iter().map(|(id, s)| proto::WindowSummary { window: *id, title: titled(s), names: s.tree.names(), kind: Kind::Notification }).collect()
    }

    /// Notification `id`'s tree, with the document the platform shows it
    /// from under `document` in its snapshot's `engine` settings, which on
    /// Windows is the toast's XML.
    pub(crate) fn snapshot(&self, id: WindowId) -> Option<proto::Snapshot> {
        let s = self.shown.get(&id)?;
        let settings = BTreeMap::from([(String::from("document"), Value::Str(s.document.clone()))]);
        let engine = proto::Engine { name: String::from(platform::NAME), version: String::new(), folder: None, settings };
        Some(proto::Snapshot { tree: s.tree.clone(), state: BTreeMap::new(), pixels: None, bounds: BTreeMap::new(), engine: Some(engine), focused: None, copied: None, sound: BTreeMap::new() })
    }
}

/// A choice's row `row`, a test's answer, when `choices` has it.
fn choice_row(name: &str, choices: &[String], row: i64) -> Result<usize, String> {
    match usize::try_from(row) {
        Ok(r) if r < choices.len() => Ok(r),
        Ok(r) => Err(format!("{name} has {} choices, and row {r} is none of them", choices.len())),
        Err(e) => Err(format!("{name}'s row {row} is no row: {e}")),
    }
}

/// The values a test gives the inputs of a notification of `tree` showing
/// `content`, by their names in `value`, a map, as the platform reports
/// them: by each input's node, a text box's text and a choice's row. Null
/// gives none.
fn test_inputs(tree: &Node, content: &Content, value: &Value) -> Result<Vec<(String, String)>, String> {
    let given = match value {
        Value::Null => return Ok(Vec::new()),
        Value::Map(m) => m,
        other => return Err(format!("a test gives a notification's inputs as a map of their names to their values, not {other:?}")),
    };
    let mut out = Vec::new();
    for (name, v) in given {
        let node = tree.find_named(name).ok_or_else(|| format!("the notification has no input named {name}; its widgets are {}", tree.names().join(", ")))?;
        let input = content.input(node.id).ok_or_else(|| format!("{name} is a {:?}, not one of the notification's inputs", node.kind))?;
        let reported = if input.choices.is_empty() {
            v.as_str().ok_or_else(|| format!("the text box {name} takes text, not {v:?}"))?.to_string()
        } else {
            let row = match v {
                Value::Str(text) => input.choices.iter().position(|c| c == text).ok_or_else(|| format!("{text} is not one of {name}'s choices, {}", input.choices.join(", ")))?,
                Value::Int(r) => choice_row(name, &input.choices, *r)?,
                other => return Err(format!("the choice {name} takes the text of a choice or its row, not {other:?}")),
            };
            row.to_string()
        };
        out.push((node.id.to_string(), reported));
    }
    Ok(out)
}

/// The events `heard` raises for notification `id`, whose root is `root`
/// and which shows `content`, and whether it ends the notification: for a
/// notification the platform took after answering, `Opened` once it shows,
/// or why it cannot be shown and `Closed` with reason `Failed`.
fn events_of(id: WindowId, root: NodeId, content: &Content, heard: Heard) -> (Vec<FromHelper>, bool) {
    let at = proto::clock::now_ns();
    let event = |node: NodeId, kind: EventKind, value: Value| FromHelper::Event { window: id, node, kind, value, at };
    match heard {
        Heard::Activated { arguments, inputs, .. } => {
            let mut events = Vec::new();
            let given = match inputs {
                Ok(given) => given,
                Err(why) => {
                    events.push(FromHelper::Error { window: Some(id), context: String::from("Notify"), text: format!("the answer's inputs could not be read: {why}") });
                    Vec::new()
                }
            };
            let activation = match Activation::of(&arguments) {
                Ok(a) => a,
                Err(why) => {
                    events.push(FromHelper::Error { window: Some(id), context: String::from("Notify"), text: why });
                    return (events, false);
                }
            };
            if let Activation::Other(own) = &activation {
                let inputs = given.iter().map(|(k, v)| (k.clone(), Value::Str(v.clone()))).collect();
                let value = Value::Map(BTreeMap::from([(String::from("arguments"), Value::Str(own.clone())), (String::from("inputs"), Value::Map(inputs))]));
                events.push(event(root, EventKind::Activate, value));
                events.push(FromHelper::Closed { window: id, reason: CloseReason::User });
                return (events, true);
            }
            events.extend(input_changes(content, &given).into_iter().map(|(node, value)| event(node, EventKind::Change, value)));
            match activation {
                Activation::Body => events.push(event(root, EventKind::Activate, Value::Null)),
                Activation::Button(node) => events.push(event(node, EventKind::Click, Value::Null)),
                Activation::Header => {
                    events.push(event(root, EventKind::Activate, Value::Str(String::from("Header"))));
                    return (events, false);
                }
                Activation::Other(_) => {}
            }
            events.push(FromHelper::Closed { window: id, reason: CloseReason::User });
            (events, true)
        }
        Heard::Dismissed { reason, .. } => {
            let mut events = vec![event(root, EventKind::Dismiss, Value::Str(reason.name().to_string()))];
            let closes = match reason {
                Dismissal::User => Some(CloseReason::User),
                Dismissal::Hidden => Some(CloseReason::Program),
                Dismissal::Timeout => None,
            };
            match closes {
                Some(r) => {
                    events.push(FromHelper::Closed { window: id, reason: r });
                    (events, true)
                }
                None => (events, false),
            }
        }
        Heard::Failed { text, .. } => (vec![event(root, EventKind::Fail, Value::Str(text)), FromHelper::Closed { window: id, reason: CloseReason::Failed }], true),
        Heard::Shown { .. } => (vec![FromHelper::Opened { window: id }], false),
        Heard::Refused { denied, text, .. } => {
            let context = if denied { proto::notification::DENIED } else { "Notify" };
            (vec![FromHelper::Error { window: Some(id), context: String::from(context), text }, FromHelper::Closed { window: id, reason: CloseReason::Failed }], true)
        }
    }
}

/// The value each input of `content` given in `given`, by its id, takes: a
/// text box's text, and a choice's row as a number, as a window's ComboBox
/// raises it. A row naming no choice is passed over with a trace line.
fn input_changes(content: &Content, given: &[(String, String)]) -> Vec<(NodeId, Value)> {
    let mut out = Vec::new();
    for input in &content.inputs {
        let key = input.node.to_string();
        let Some((_, v)) = given.iter().find(|(k, _)| *k == key) else {
            continue;
        };
        if input.choices.is_empty() {
            out.push((input.node, Value::Str(v.clone())));
            continue;
        }
        match v.parse::<usize>() {
            Ok(row) if row < input.choices.len() => out.push((input.node, Value::Int(row as i64))),
            Ok(row) => proto::trace::line(|| format!("egui: input {} was answered with row {row}, beyond its {} choices", input.node, input.choices.len())),
            Err(why) => proto::trace::line(|| format!("egui: input {} was answered with {v}, which names none of its choices: {why}", input.node)),
        }
    }
    out
}

/// What the platform made of a notification it shows: its toast, the
/// document it shows the toast from, what the person should hear of parts
/// it cannot show, and whether it shows once the platform has answered,
/// which [`Heard::Shown`] or [`Heard::Refused`] then reports.
struct Made {
    toast: platform::Toast,
    document: String,
    notes: Vec<String>,
    pending: bool,
}

#[cfg(windows)]
mod platform {
    //! Windows: a WinRT toast of the notification's document, shown under
    //! its identity, an AppUserModelID registered for the user.

    use super::{Dismissal, Heard, Made, Refused, hear};
    use proto::WindowId;
    use proto::notification::{Content, MOST_INPUTS, OWN_APP};
    use pwrs_gui_proto as proto;
    use windows::Data::Xml::Dom::XmlDocument;
    use windows::Foundation::{IPropertyValue, TypedEventHandler};
    use windows::UI::Notifications::{
        NotificationData, NotificationSetting, NotificationUpdateResult, ToastActivatedEventArgs, ToastDismissalReason, ToastDismissedEventArgs, ToastFailedEventArgs, ToastNotification, ToastNotificationManager,
        ToastNotifier,
    };
    use windows::core::{HSTRING, IInspectable, Interface, Ref};

    /// What a snapshot names the platform's notifications.
    pub(super) const NAME: &str = "WindowsToast";

    /// A toast shown: its notifier, the toast, the tag and group that name
    /// it in the system's list, and the identity it is shown under.
    pub(super) struct Toast {
        notifier: ToastNotifier,
        toast: ToastNotification,
        tag: HSTRING,
        group: HSTRING,
        app: HSTRING,
    }

    /// What a notifier's setting gives for an identity registered under
    /// HKCU\Software\Classes\AppUserModelId with no Start menu shortcut,
    /// for which Windows keeps no setting until Settings lists the app:
    /// Element not found. Its toasts show all the same.
    const NO_SETTING_YET: windows::core::HRESULT = windows::core::HRESULT(0x8007_0490_u32 as i32);

    /// A WinRT error as text: its message and its code.
    fn said(e: &windows::core::Error) -> String {
        format!("{} ({:#010x})", e.message(), e.code().0)
    }

    /// The data the bound parts of `content`'s document show, at
    /// `sequence`, which tells Windows a later update from an earlier one.
    fn data_of(content: &Content, sequence: u32) -> Result<NotificationData, String> {
        let data = NotificationData::new().map_err(|e| format!("the notification's data cannot be made: {}", said(&e)))?;
        let values = data.Values().map_err(|e| format!("the notification's data has no values: {}", said(&e)))?;
        for (name, value) in content.toast_data() {
            values.Insert(&HSTRING::from(name), &HSTRING::from(value)).map_err(|e| format!("the notification's {name} cannot be set: {}", said(&e)))?;
        }
        data.SetSequenceNumber(sequence).map_err(|e| format!("the notification's data cannot be numbered: {}", said(&e)))?;
        Ok(data)
    }

    /// Each input's value an activation carries, by its id.
    fn inputs_of(args: &ToastActivatedEventArgs) -> Result<Vec<(String, String)>, String> {
        let set = args.UserInput().map_err(|e| format!("Windows gave no inputs with the answer: {}", said(&e)))?;
        let mut out = Vec::new();
        for pair in &set {
            let key = pair.Key().map_err(|e| format!("an input's id cannot be read: {}", said(&e)))?;
            let value = pair.Value().map_err(|e| format!("input {key}'s value cannot be read: {}", said(&e)))?;
            let text = value.cast::<IPropertyValue>().and_then(|v| v.GetString()).map_err(|e| format!("input {key}'s value is not text: {}", said(&e)))?;
            out.push((key.to_string(), text.to_string()));
        }
        Ok(out)
    }

    /// What an activation of notification `id` reports.
    fn activated(id: WindowId, args: &Ref<IInspectable>) -> Heard {
        let Some(given) = args.as_ref() else {
            return Heard::Failed { notification: id, text: String::from("Windows reported the answer with nothing to read") };
        };
        let args = match given.cast::<ToastActivatedEventArgs>() {
            Ok(a) => a,
            Err(e) => return Heard::Failed { notification: id, text: format!("the answer could not be read: {}", said(&e)) },
        };
        match args.Arguments() {
            Ok(arguments) => Heard::Activated { notification: id, arguments: arguments.to_string(), inputs: inputs_of(&args) },
            Err(e) => Heard::Failed { notification: id, text: format!("the answer's arguments could not be read: {}", said(&e)) },
        }
    }

    /// What a dismissal of notification `id` reports.
    fn dismissed(id: WindowId, args: &Ref<ToastDismissedEventArgs>) -> Heard {
        let Some(given) = args.as_ref() else {
            return Heard::Failed { notification: id, text: String::from("Windows reported the dismissal with nothing to read") };
        };
        match given.Reason() {
            Ok(r) if r == ToastDismissalReason::UserCanceled => Heard::Dismissed { notification: id, reason: Dismissal::User },
            Ok(r) if r == ToastDismissalReason::TimedOut => Heard::Dismissed { notification: id, reason: Dismissal::Timeout },
            Ok(r) if r == ToastDismissalReason::ApplicationHidden => Heard::Dismissed { notification: id, reason: Dismissal::Hidden },
            Ok(r) => Heard::Failed { notification: id, text: format!("Windows dismissed it for a reason it numbers {}", r.0) },
            Err(e) => Heard::Failed { notification: id, text: format!("the dismissal could not be read: {}", said(&e)) },
        }
    }

    /// What a failure of notification `id` reports.
    fn failed(id: WindowId, args: &Ref<ToastFailedEventArgs>) -> Heard {
        let Some(given) = args.as_ref() else {
            return Heard::Failed { notification: id, text: String::from("Windows could not show it, and gave nothing to say why") };
        };
        let text = match given.ErrorCode() {
            Ok(code) => format!("Windows could not show it: {} ({:#010x})", windows::core::Error::from_hresult(code).message(), code.0),
            Err(e) => format!("Windows could not show it, and could not say why: {}", said(&e)),
        };
        Heard::Failed { notification: id, text }
    }

    /// Why notifications under an identity whose setting is `setting` do
    /// not show; none when they show.
    fn refused(setting: NotificationSetting, app: &str) -> Option<String> {
        if setting == NotificationSetting::Enabled {
            return None;
        }
        Some(if setting == NotificationSetting::DisabledForApplication {
            format!("notifications from {app} are turned off in Settings > System > Notifications")
        } else if setting == NotificationSetting::DisabledForUser {
            String::from("notifications are turned off for this user in Settings > System > Notifications")
        } else if setting == NotificationSetting::DisabledByGroupPolicy {
            String::from("Group Policy turns notifications off on this machine")
        } else {
            format!("Windows turns notifications from {app} off, its setting being {}", setting.0)
        })
    }

    /// Shows notification `id` of `content` as a toast, its events reported
    /// through [`hear`].
    pub(super) fn show(id: WindowId, content: &Content) -> Result<Made, Refused> {
        if content.xml.is_none() && content.inputs.len() > MOST_INPUTS {
            return Err(Refused::Input(format!("a Windows notification shows {MOST_INPUTS} inputs at most, and this one has {}", content.inputs.len())));
        }
        let xml = content.toast_xml()?;
        let doc = XmlDocument::new().map_err(|e| format!("an XML document cannot be made: {}", said(&e)))?;
        doc.LoadXml(&HSTRING::from(xml.as_str())).map_err(|e| format!("the toast's XML does not load: {}", said(&e)))?;
        let toast = ToastNotification::CreateToastNotification(&doc).map_err(|e| format!("Windows refuses the toast's XML: {}", said(&e)))?;
        let tag = HSTRING::from(id.to_string());
        let group = HSTRING::from(format!("pwrs-gui-{}", std::process::id()));
        toast.SetTag(&tag).map_err(|e| format!("the toast's tag cannot be set: {}", said(&e)))?;
        toast.SetGroup(&group).map_err(|e| format!("the toast's group cannot be set: {}", said(&e)))?;
        if content.quiet {
            toast.SetSuppressPopup(true).map_err(|e| format!("the toast cannot be kept to the notification center: {}", said(&e)))?;
        }
        if content.xml.is_none() {
            toast.SetData(&data_of(content, 1)?).map_err(|e| format!("the toast's data cannot be set: {}", said(&e)))?;
        }
        toast
            .Activated(&TypedEventHandler::new(move |_, args: Ref<IInspectable>| {
                hear(activated(id, &args));
                Ok(())
            }))
            .map_err(|e| format!("the toast's answers cannot be heard: {}", said(&e)))?;
        toast
            .Dismissed(&TypedEventHandler::new(move |_, args: Ref<ToastDismissedEventArgs>| {
                hear(dismissed(id, &args));
                Ok(())
            }))
            .map_err(|e| format!("the toast's dismissal cannot be heard: {}", said(&e)))?;
        toast
            .Failed(&TypedEventHandler::new(move |_, args: Ref<ToastFailedEventArgs>| {
                hear(failed(id, &args));
                Ok(())
            }))
            .map_err(|e| format!("the toast's failure cannot be heard: {}", said(&e)))?;
        let app_text = content.app.as_deref().unwrap_or(OWN_APP);
        let app = HSTRING::from(app_text);
        let notifier = ToastNotificationManager::CreateToastNotifierWithId(&app).map_err(|e| format!("no notifier for {app_text}: {}", said(&e)))?;
        match notifier.Setting() {
            Ok(setting) => {
                if let Some(why) = refused(setting, app_text) {
                    return Err(Refused::Other(why));
                }
            }
            Err(e) if e.code() == NO_SETTING_YET => {}
            Err(e) => return Err(Refused::Other(format!("Windows does not say whether {app_text} may show notifications: {}", said(&e)))),
        }
        notifier.Show(&toast).map_err(|e| format!("Windows did not show the toast: {}", said(&e)))?;
        Ok(Made { toast: Toast { notifier, toast, tag, group, app }, document: xml, notes: Vec::new(), pending: false })
    }

    /// Shows `content`'s bound parts in its toast now: its title, its body
    /// and its progress bar.
    pub(super) fn update(toast: &Toast, content: &Content) -> Result<(), String> {
        if content.xml.is_some() {
            return Err(String::from("a toast of the caller's own XML keeps the parts its XML gives"));
        }
        static SEQUENCE: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(2);
        let data = data_of(content, SEQUENCE.fetch_add(1, std::sync::atomic::Ordering::Relaxed))?;
        let result = toast.notifier.UpdateWithTagAndGroup(&data, &toast.tag, &toast.group).map_err(|e| format!("the toast could not be updated: {}", said(&e)))?;
        if result == NotificationUpdateResult::Succeeded {
            return Ok(());
        }
        Err(if result == NotificationUpdateResult::NotificationNotFound {
            String::from("the toast is no longer in the system's list of notifications, so it cannot be updated")
        } else {
            format!("Windows did not update the toast, answering {}", result.0)
        })
    }

    /// Takes the toast away from the screen and the system's list.
    pub(super) fn hide(toast: &Toast) -> Result<(), String> {
        toast.notifier.Hide(&toast.toast).map_err(|e| format!("the toast could not be hidden: {}", said(&e)))?;
        let history = ToastNotificationManager::History().map_err(|e| format!("the system's list of notifications cannot be reached: {}", said(&e)))?;
        history.RemoveGroupedTagWithId(&toast.tag, &toast.group, &toast.app).map_err(|e| format!("the toast could not be taken from the system's list: {}", said(&e)))
    }

    /// Takes every notification shown under identity `app` from the
    /// system's list, for an identity being unregistered.
    pub(super) fn forget(app: &str) -> Result<(), String> {
        let history = ToastNotificationManager::History().map_err(|e| format!("the system's list of notifications cannot be reached: {}", said(&e)))?;
        history.ClearWithId(&HSTRING::from(app)).map_err(|e| format!("the notifications of {app} could not be taken from the system's list: {}", said(&e)))
    }
}

/// Takes every notification shown under the Windows identity `app`, an
/// AppUserModelID, from the system's list, as unregistering the identity
/// does. Runs on a thread the process has initialized for COM, as every
/// thread .NET starts is.
#[cfg(windows)]
pub fn forget_app(app: &str) -> Result<(), String> {
    platform::forget(app)
}

#[cfg(target_os = "macos")]
mod platform {
    //! macOS: the UserNotifications framework's notification center, which
    //! shows an app bundle's notifications alone, so the helper shows them
    //! from inside PWRS-GUI.app. Each notification first asks the person's
    //! permission, which macOS asks once and then remembers; a quiet one
    //! asks for provisional permission, delivered quietly without asking.
    //! The center answers on a thread of its own, so a notification shows,
    //! or is refused, through [`hear`]: [`Heard::Shown`] or
    //! [`Heard::Refused`]. A click, a button, a reply and a dismissal come
    //! back through the center's delegate, a class of this module's own.

    use super::{Dismissal, Heard, Made, Refused, hear};
    use arc_swap::ArcSwap;
    use block2::{DynBlock, RcBlock};
    use objc2::rc::Retained;
    use objc2::runtime::{AnyObject, Bool, NSObject, NSObjectProtocol, ProtocolObject};
    use objc2::{AllocAnyThread, Message, define_class, msg_send};
    use objc2_foundation::{NSArray, NSBundle, NSError, NSSet, NSString, NSURL};
    use objc2_user_notifications::{
        UNAuthorizationOptions, UNMutableNotificationContent, UNNotification, UNNotificationAction, UNNotificationActionOptions, UNNotificationAttachment, UNNotificationCategory,
        UNNotificationCategoryOptions, UNNotificationDefaultActionIdentifier, UNNotificationDismissActionIdentifier, UNNotificationInterruptionLevel, UNNotificationPresentationOptions,
        UNNotificationRequest, UNNotificationResponse, UNNotificationSound, UNTextInputNotificationAction, UNTextInputNotificationResponse, UNUserNotificationCenter,
        UNUserNotificationCenterDelegate,
    };
    use proto::notification::{BODY_ARGUMENT, BUTTON_ARGUMENT, Content, Progress, Scenario};
    use proto::{NodeId, WindowId};
    use pwrs_gui_proto as proto;
    use std::cell::RefCell;
    use std::collections::BTreeMap;
    use std::sync::LazyLock;

    /// What a snapshot names the platform's notifications.
    pub(super) const NAME: &str = "UserNotifications";

    /// A notification the center shows, by its number.
    pub(super) struct Toast {
        id: WindowId,
    }

    /// The text input each notification's reply button sends, by the
    /// notification's number, which the delegate reads on the center's
    /// thread.
    static REPLIES: LazyLock<ArcSwap<BTreeMap<WindowId, NodeId>>> = LazyLock::new(|| ArcSwap::from_pointee(BTreeMap::new()));

    thread_local! {
        /// Each notification's category, its buttons and reply, by its
        /// number: the center takes every category of the app at once.
        static CATEGORIES: RefCell<BTreeMap<WindowId, Retained<UNNotificationCategory>>> = const { RefCell::new(BTreeMap::new()) };
        /// The center's delegate, which the center holds weakly.
        static DELEGATE: RefCell<Option<Retained<Delegate>>> = const { RefCell::new(None) };
    }

    define_class!(
        // SAFETY: NSObject has no subclassing requirements, and the class
        // has no Drop of its own.
        #[unsafe(super(NSObject))]
        #[name = "PwrsGuiNotificationDelegate"]
        struct Delegate;

        unsafe impl NSObjectProtocol for Delegate {}

        unsafe impl UNUserNotificationCenterDelegate for Delegate {
            /// Shows a notification as a banner while the helper is the
            /// active app, as when it is not; a quiet one in the list
            /// alone.
            #[unsafe(method(userNotificationCenter:willPresentNotification:withCompletionHandler:))]
            fn will_present(&self, _center: &UNUserNotificationCenter, notification: &UNNotification, handler: &DynBlock<dyn Fn(UNNotificationPresentationOptions)>) {
                let quiet = notification.request().content().interruptionLevel() == UNNotificationInterruptionLevel::Passive;
                let options = if quiet {
                    UNNotificationPresentationOptions::List
                } else {
                    UNNotificationPresentationOptions::Banner | UNNotificationPresentationOptions::List | UNNotificationPresentationOptions::Sound
                };
                handler.call((options,));
            }

            /// Reports the person's answer: a click, a button, a reply or a
            /// dismissal.
            #[unsafe(method(userNotificationCenter:didReceiveNotificationResponse:withCompletionHandler:))]
            fn did_receive(&self, _center: &UNUserNotificationCenter, response: &UNNotificationResponse, handler: &DynBlock<dyn Fn()>) {
                match heard_of(response) {
                    Ok(Some(heard)) => hear(heard),
                    Ok(None) => proto::trace::line(|| String::from("egui: another PWRS-GUI process's notification was answered here")),
                    Err(why) => proto::trace::line(|| format!("egui: a notification's answer cannot be read: {why}")),
                }
                handler.call(());
            }
        }
    );

    impl Delegate {
        fn new() -> Retained<Delegate> {
            let this = Delegate::alloc().set_ivars(());
            // SAFETY: NSObject's init takes nothing and returns the object.
            unsafe { msg_send![super(this), init] }
        }
    }

    /// The identifier of notification `id`'s request and category: this
    /// process's id and the number, so two helpers' notifications under
    /// the one bundle are told apart.
    fn identifier(id: WindowId) -> String {
        format!("pwrs-gui-{}-{id}", std::process::id())
    }

    /// The number of this process's notification `identifier` names; none
    /// for another process's.
    fn number_of(identifier: &str) -> Result<Option<WindowId>, String> {
        let rest = identifier.strip_prefix("pwrs-gui-").ok_or_else(|| format!("{identifier} is no notification of PWRS-GUI's"))?;
        let (pid, id) = rest.split_once('-').ok_or_else(|| format!("{identifier} names no process"))?;
        if pid != std::process::id().to_string() {
            return Ok(None);
        }
        id.parse().map(Some).map_err(|e| format!("{identifier} names no notification: {e}"))
    }

    /// What the person's answer `response` reports; none for another
    /// process's notification.
    fn heard_of(response: &UNNotificationResponse) -> Result<Option<Heard>, String> {
        let Some(id) = number_of(&response.notification().request().identifier().to_string())? else {
            return Ok(None);
        };
        let action = response.actionIdentifier();
        // SAFETY: the framework's constant strings, which live as long as
        // the process.
        let (default, dismiss) = unsafe { (UNNotificationDefaultActionIdentifier, UNNotificationDismissActionIdentifier) };
        if action.isEqualToString(dismiss) {
            return Ok(Some(Heard::Dismissed { notification: id, reason: Dismissal::User }));
        }
        let arguments = if action.isEqualToString(default) { String::from(BODY_ARGUMENT) } else { action.to_string() };
        let any: &AnyObject = response.as_ref();
        let inputs = match any.downcast_ref::<UNTextInputNotificationResponse>() {
            Some(reply) => match REPLIES.load().get(&id) {
                Some(node) => Ok(vec![(node.to_string(), reply.userText().to_string())]),
                None => Err(format!("notification {id} was answered with text, and it has no text input")),
            },
            None => Ok(Vec::new()),
        };
        Ok(Some(Heard::Activated { notification: id, arguments, inputs }))
    }

    /// The notification center, with this module's delegate; refused
    /// outside an app bundle, where macOS has no notification center for
    /// the process.
    fn center() -> Result<Retained<UNUserNotificationCenter>, String> {
        if NSBundle::mainBundle().bundleIdentifier().is_none() {
            return Err(String::from("macOS shows the notifications of an app bundle alone, and this renderer runs outside PWRS-GUI.app"));
        }
        let center = UNUserNotificationCenter::currentNotificationCenter();
        DELEGATE.with(|d| {
            let mut d = d.borrow_mut();
            if d.is_none() {
                let delegate = Delegate::new();
                center.setDelegate(Some(ProtocolObject::from_ref(&*delegate)));
                *d = Some(delegate);
            }
        });
        Ok(center)
    }

    /// Gives the center every category: notification `id`'s, or with none
    /// the rest without it.
    fn categories(center: &UNUserNotificationCenter, id: WindowId, category: Option<Retained<UNNotificationCategory>>) {
        CATEGORIES.with(|c| {
            let mut c = c.borrow_mut();
            match category {
                Some(made) => {
                    c.insert(id, made);
                }
                None => {
                    c.remove(&id);
                }
            }
            let all: Vec<Retained<UNNotificationCategory>> = c.values().cloned().collect();
            center.setNotificationCategories(&NSSet::from_retained_slice(&all));
        });
    }

    /// Notification `id`'s category: its buttons, one of them the reply
    /// button that sends its text input, and a dismissal reported as an
    /// answer. Refused for a choice, more than one text input, and a text
    /// input no button sends, which a macOS notification shows no box for.
    fn category_of(id: WindowId, content: &Content) -> Result<(Retained<UNNotificationCategory>, Option<NodeId>), Refused> {
        if let Some(choice) = content.inputs.iter().find(|i| !i.choices.is_empty()) {
            return Err(Refused::Input(format!("a macOS notification takes a text input alone, and input {} is a choice", choice.node)));
        }
        if content.inputs.len() > 1 {
            return Err(Refused::Input(format!("a macOS notification takes one text input, and this one has {}", content.inputs.len())));
        }
        let mut reply = None;
        let mut actions: Vec<Retained<UNNotificationAction>> = Vec::new();
        for b in &content.buttons {
            let identifier = NSString::from_str(&format!("{BUTTON_ARGUMENT}{}", b.node));
            let title = NSString::from_str(&b.text);
            let action = match b.input.and_then(|node| content.input(node)) {
                Some(input) => {
                    reply = Some(input.node);
                    let placeholder = match &input.placeholder {
                        Some(p) => NSString::from_str(p),
                        None => NSString::new(),
                    };
                    let sends = UNTextInputNotificationAction::actionWithIdentifier_title_options_textInputButtonTitle_textInputPlaceholder(
                        &identifier,
                        &title,
                        UNNotificationActionOptions::empty(),
                        &title,
                        &placeholder,
                    );
                    Retained::into_super(sends)
                }
                None => UNNotificationAction::actionWithIdentifier_title_options(&identifier, &title, UNNotificationActionOptions::empty()),
            };
            actions.push(action);
        }
        if let Some(input) = content.inputs.first()
            && reply.is_none()
        {
            return Err(Refused::Input(format!("on macOS a text input is answered through the button that sends it, and no button sends input {}", input.node)));
        }
        let category = UNNotificationCategory::categoryWithIdentifier_actions_intentIdentifiers_options(
            &NSString::from_str(&identifier(id)),
            &NSArray::from_retained_slice(&actions),
            &NSArray::new(),
            UNNotificationCategoryOptions::CustomDismissAction,
        );
        Ok((category, reply))
    }

    /// A progress bar as the line below the title, which is all a macOS
    /// notification has for one.
    fn progress_line(p: &Progress) -> String {
        let mut parts: Vec<String> = [&p.title, &p.status].into_iter().flatten().cloned().collect();
        match (&p.value_text, p.value) {
            (Some(t), _) => parts.push(t.clone()),
            (None, Some(v)) => parts.push(format!("{:.0}%", v * 100.0)),
            (None, None) => {}
        }
        parts.join(" \u{2014} ")
    }

    /// The picture at `path` as an attachment, from a copy, since the
    /// center moves the file it is given into its own store.
    fn attachment_of(id: WindowId, path: &str) -> Result<Retained<UNNotificationAttachment>, String> {
        let source = std::path::Path::new(path);
        let name = source.file_name().ok_or_else(|| format!("{path} names no file"))?.to_string_lossy().into_owned();
        let copy = std::env::temp_dir().join(format!("pwrs-gui-{}-{id}-{name}", std::process::id()));
        std::fs::copy(source, &copy).map_err(|e| format!("{path} cannot be copied for the notification: {e}"))?;
        let url = NSURL::fileURLWithPath(&NSString::from_str(&copy.to_string_lossy()));
        // SAFETY: the identifier and the URL are valid objects, and no
        // options are given.
        let made = unsafe { UNNotificationAttachment::attachmentWithIdentifier_URL_options_error(&NSString::from_str("image"), &url, None) };
        made.map_err(|e| format!("{path} cannot be attached: {}", e.localizedDescription()))
    }

    /// Notification `id`'s request, with notes on the parts a macOS
    /// notification cannot show.
    fn request_of(id: WindowId, content: &Content, notes: &mut Vec<String>) -> Result<Retained<UNNotificationRequest>, Refused> {
        if content.xml.is_some() {
            if content.title.is_none() && content.body.is_none() {
                return Err(Refused::Other(String::from("a macOS notification shows a title or a body, and -Xml is a Windows toast's own")));
            }
            notes.push(String::from("macOS shows no toast XML, so the notification shows its title and body"));
        }
        let shown = UNMutableNotificationContent::new();
        if let Some(t) = &content.title {
            shown.setTitle(&NSString::from_str(t));
        }
        if let Some(b) = &content.body {
            shown.setBody(&NSString::from_str(b));
        }
        if let Some(h) = &content.header {
            shown.setThreadIdentifier(&NSString::from_str(h));
        }
        if let Some(p) = &content.progress {
            shown.setSubtitle(&NSString::from_str(&progress_line(p)));
        }
        match &content.audio {
            Some(a) if a.silent => shown.setSound(None),
            Some(a) => {
                if let Some(name) = &a.sound {
                    notes.push(format!("macOS plays its own notification sound, not Windows' {name}"));
                }
                if a.repeat {
                    notes.push(String::from("macOS plays a notification's sound once"));
                }
                shown.setSound(Some(&UNNotificationSound::defaultSound()));
            }
            None => shown.setSound(Some(&UNNotificationSound::defaultSound())),
        }
        let level = match (content.quiet, content.scenario) {
            (true, _) => UNNotificationInterruptionLevel::Passive,
            (false, Scenario::Urgent) => UNNotificationInterruptionLevel::TimeSensitive,
            (false, Scenario::Default | Scenario::Reminder | Scenario::Alarm | Scenario::IncomingCall) => UNNotificationInterruptionLevel::Active,
        };
        shown.setInterruptionLevel(level);
        if matches!(content.scenario, Scenario::Reminder | Scenario::Alarm | Scenario::IncomingCall) {
            notes.push(String::from("macOS keeps a notification on the screen as the person's settings say, whatever its scenario"));
        }
        if content.hero.is_some() {
            notes.push(String::from("macOS shows no hero image"));
        }
        if content.logo.is_some() {
            notes.push(String::from("macOS shows the app's own icon, so -AppLogo is left out"));
        }
        if content.app.is_some() {
            notes.push(String::from("-App names a Windows identity; macOS shows PWRS-GUI.app's name"));
        }
        if let Some(path) = &content.image {
            match attachment_of(id, path) {
                Ok(attached) => shown.setAttachments(&NSArray::from_retained_slice(&[attached])),
                Err(why) => notes.push(format!("the image is left out: {why}")),
            }
        }
        shown.setCategoryIdentifier(&NSString::from_str(&identifier(id)));
        Ok(UNNotificationRequest::requestWithIdentifier_content_trigger(&NSString::from_str(&identifier(id)), &shown, None))
    }

    /// Hands the center `request` once the person's permission is known:
    /// asked the first time, provisional for a `quiet` one, and remembered
    /// by macOS after. What the center answers reaches [`hear`].
    fn deliver(center: &UNUserNotificationCenter, id: WindowId, request: Retained<UNNotificationRequest>, quiet: bool) {
        let mut options = UNAuthorizationOptions::Alert | UNAuthorizationOptions::Sound;
        if quiet {
            options |= UNAuthorizationOptions::Provisional;
        }
        let adder = center.retain();
        let answered = RcBlock::new(move |granted: Bool, error: *mut NSError| {
            if !granted.as_bool() {
                // SAFETY: the center passes a valid error or none.
                let text = match unsafe { error.as_ref() } {
                    Some(e) => format!("macOS did not allow PWRS-GUI's notifications: {}", e.localizedDescription()),
                    None => String::from("the person did not allow PWRS-GUI's notifications, which System Settings > Notifications > PWRS-GUI allows"),
                };
                hear(Heard::Refused { notification: id, denied: true, text });
                return;
            }
            let added = RcBlock::new(move |error: *mut NSError| {
                // SAFETY: the center passes a valid error or none.
                match unsafe { error.as_ref() } {
                    None => hear(Heard::Shown { notification: id }),
                    Some(e) => hear(Heard::Refused { notification: id, denied: false, text: format!("the notification center refused it: {}", e.localizedDescription()) }),
                }
            });
            adder.addNotificationRequest_withCompletionHandler(&request, Some(&added));
        });
        center.requestAuthorizationWithOptions_completionHandler(options, &answered);
    }

    /// Shows notification `id` of `content` once the person's permission
    /// is known, which [`hear`] reports.
    pub(super) fn show(id: WindowId, content: &Content) -> Result<Made, Refused> {
        let center = center().map_err(Refused::Other)?;
        let (category, reply) = category_of(id, content)?;
        let mut notes = Vec::new();
        let request = request_of(id, content, &mut notes)?;
        categories(&center, id, Some(category));
        if let Some(node) = reply {
            REPLIES.rcu(|r| {
                let mut r = (**r).clone();
                r.insert(id, node);
                r
            });
        }
        deliver(&center, id, request, content.quiet);
        Ok(Made { toast: Toast { id }, document: String::new(), notes, pending: true })
    }

    /// Shows `content` in place of the notification the center delivered:
    /// a request of the same identifier replaces it.
    pub(super) fn update(toast: &Toast, content: &Content) -> Result<(), String> {
        let center = center()?;
        let mut notes = Vec::new();
        let request = request_of(toast.id, content, &mut notes).map_err(|refused| match refused {
            Refused::Input(why) | Refused::Other(why) => why,
        })?;
        let id = toast.id;
        let added = RcBlock::new(move |error: *mut NSError| {
            // SAFETY: the center passes a valid error or none.
            if let Some(e) = unsafe { error.as_ref() } {
                let why = e.localizedDescription().to_string();
                proto::trace::line(|| format!("egui: notification {id} could not be replaced: {why}"));
            }
        });
        center.addNotificationRequest_withCompletionHandler(&request, Some(&added));
        Ok(())
    }

    /// Takes the notification from the screen and the center's list, with
    /// its category and its reply.
    pub(super) fn hide(toast: &Toast) -> Result<(), String> {
        let center = center()?;
        let ids = NSArray::from_retained_slice(&[NSString::from_str(&identifier(toast.id))]);
        center.removeDeliveredNotificationsWithIdentifiers(&ids);
        center.removePendingNotificationRequestsWithIdentifiers(&ids);
        categories(&center, toast.id, None);
        REPLIES.rcu(|r| {
            let mut r = (**r).clone();
            r.remove(&toast.id);
            r
        });
        Ok(())
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn a_notifications_identifier_names_this_process_and_its_number() {
            assert_eq!(number_of(&identifier(42)).expect("an identifier"), Some(42));
            assert_eq!(number_of("pwrs-gui-1-42").expect("another process's"), None);
            assert!(number_of("other-42").is_err());
        }

        #[test]
        fn a_progress_bar_reads_as_a_line() {
            let p = Progress { value: Some(0.5), title: None, status: Some(String::from("Copying")), value_text: None };
            assert_eq!(progress_line(&p), "Copying \u{2014} 50%");
        }
    }
}

#[cfg(all(unix, not(target_vendor = "apple")))]
mod platform {
    //! Linux and the BSDs: the desktop's notification service,
    //! `org.freedesktop.Notifications` on the session bus, through zbus. One
    //! thread, started with the first notification, hears the service's
    //! signals for every notification and reports them through [`hear`];
    //! the service's numbers map to the session's through [`NUMBERED`].

    use super::{Dismissal, Heard, Made, Refused, hear};
    use arc_swap::ArcSwap;
    use proto::notification::{BODY_ARGUMENT, BUTTON_ARGUMENT, Content, OWN_APP, Scenario, file_uri};
    use proto::{NodeId, WindowId};
    use pwrs_gui_proto as proto;
    use std::collections::{BTreeMap, HashMap};
    use std::sync::{LazyLock, OnceLock};
    use zbus::blocking::{Connection, Proxy};
    use zbus::zvariant::Value as Variant;

    /// What a snapshot names the platform's notifications.
    pub(super) const NAME: &str = "FreedesktopNotifications";

    /// The service's bus name, object path and interface.
    const SERVICE: (&str, &str, &str) = ("org.freedesktop.Notifications", "/org/freedesktop/Notifications", "org.freedesktop.Notifications");

    /// The action key of a click on a notification's body.
    const DEFAULT_ACTION: &str = "default";

    /// The action key KDE's service gives the box a reply is typed in.
    const REPLY_ACTION: &str = "inline-reply";

    /// A notification the service shows: the session's number for it, and
    /// the input a reply goes to with the button it stands beside.
    #[derive(Clone, Copy, Debug)]
    struct Numbered {
        id: WindowId,
        reply: Option<(NodeId, Option<NodeId>)>,
    }

    /// The notifications the service shows, by the service's number, which
    /// the listening thread reads and the renderer's thread writes.
    static NUMBERED: LazyLock<ArcSwap<BTreeMap<u32, Numbered>>> = LazyLock::new(|| ArcSwap::from_pointee(BTreeMap::new()));

    /// The connection to the session bus and what the service can show,
    /// reached once, or why it could not be.
    struct Reached {
        bus: Connection,
        capabilities: Vec<String>,
    }

    static REACHED: OnceLock<Result<Reached, String>> = OnceLock::new();

    /// A notification the service shows, by the number it gave it.
    pub(super) struct Toast {
        number: u32,
    }

    /// The service on `bus`.
    fn service(bus: &Connection) -> Result<Proxy<'static>, String> {
        Proxy::new(bus, SERVICE.0, SERVICE.1, SERVICE.2).map_err(|e| format!("the desktop's notification service cannot be reached: {e}"))
    }

    /// The session bus and the service's capabilities, reached on the first
    /// call, which also starts the thread that hears the service.
    fn reached() -> Result<&'static Reached, String> {
        let r = REACHED.get_or_init(|| {
            let bus = Connection::session().map_err(|e| format!("notifications are shown through the session bus, which cannot be reached: {e}"))?;
            let proxy = service(&bus)?;
            let capabilities: Vec<String> = proxy
                .call("GetCapabilities", &())
                .map_err(|e| format!("no notification service answers on the session bus ({e}); a desktop's notification service, or one such as dunst, shows them"))?;
            let signals = proxy.receive_all_signals().map_err(|e| format!("the notification service's answers cannot be heard: {e}"))?;
            std::thread::Builder::new().name(String::from("pwrs-gui-notify-bus")).spawn(move || listen(signals)).map_err(|e| format!("the thread that hears the notification service cannot start: {e}"))?;
            Ok(Reached { bus, capabilities })
        });
        r.as_ref().map_err(String::clone)
    }

    /// Hears the service's signals until the bus closes, reporting each on
    /// a notification the session shows.
    fn listen(signals: zbus::blocking::proxy::SignalIterator<'static>) {
        for msg in signals {
            let member = msg.header().member().map(|m| m.to_string());
            let heard = match member.as_deref() {
                Some("ActionInvoked") => msg.body().deserialize::<(u32, String)>().map(|(n, key)| action(n, &key)),
                Some("NotificationReplied") => msg.body().deserialize::<(u32, String)>().map(|(n, text)| replied(n, text)),
                Some("NotificationClosed") => msg.body().deserialize::<(u32, u32)>().map(|(n, reason)| closed(n, reason)),
                _ => continue,
            };
            match heard {
                Ok(Some(h)) => hear(h),
                Ok(None) => {}
                Err(e) => proto::trace::line(|| format!("egui: the notification service's {member:?} could not be read: {e}")),
            }
        }
        proto::trace::line(|| String::from("egui: the notification service's signals ended"));
    }

    /// The notification the service numbered `n`, when the session shows it.
    fn numbered(n: u32) -> Option<Numbered> {
        let found = NUMBERED.load().get(&n).copied();
        if found.is_none() {
            proto::trace::line(|| format!("egui: the notification service spoke of notification {n}, which the session does not show"));
        }
        found
    }

    /// What a click on action `key` of notification `n` reports.
    fn action(n: u32, key: &str) -> Option<Heard> {
        let shown = numbered(n)?;
        let arguments = if key == DEFAULT_ACTION { String::from(BODY_ARGUMENT) } else { format!("{BUTTON_ARGUMENT}{key}") };
        Some(Heard::Activated { notification: shown.id, arguments, inputs: Ok(Vec::new()) })
    }

    /// What a reply typed into notification `n` reports: its input's text,
    /// and a click on the button beside it, or on the body when it has none.
    fn replied(n: u32, text: String) -> Option<Heard> {
        let shown = numbered(n)?;
        let Some((input, button)) = shown.reply else {
            return Some(Heard::Failed { notification: shown.id, text: String::from("the notification service sent a reply to a notification with no input") });
        };
        let arguments = match button {
            Some(b) => format!("{BUTTON_ARGUMENT}{b}"),
            None => String::from(BODY_ARGUMENT),
        };
        Some(Heard::Activated { notification: shown.id, arguments, inputs: Ok(vec![(input.to_string(), text)]) })
    }

    /// What the close of notification `n` for the spec's `reason` reports:
    /// 1 expired, 2 dismissed by the person, 3 closed by the session.
    fn closed(n: u32, reason: u32) -> Option<Heard> {
        let shown = numbered(n)?;
        let reason = match reason {
            1 => Dismissal::Timeout,
            3 => Dismissal::Hidden,
            _ => Dismissal::User,
        };
        if reason != Dismissal::Timeout {
            NUMBERED.rcu(|m| {
                let mut m = (**m).clone();
                m.remove(&n);
                m
            });
        }
        Some(Heard::Dismissed { notification: shown.id, reason })
    }

    /// The freedesktop sound theme's name for the sound Windows calls
    /// `name`.
    fn sound_name(name: &str) -> &'static str {
        let lower = name.to_ascii_lowercase();
        if lower.starts_with("alarm") || lower == "reminder" {
            "alarm-clock-elapsed"
        } else if lower.starts_with("call") {
            "phone-incoming-call"
        } else if lower == "mail" {
            "message-new-email"
        } else {
            "message-new-instant"
        }
    }

    /// `text` as a body the service reads as markup takes it: `&`, `<` and
    /// `>` written as entities.
    fn as_markup(text: &str) -> String {
        text.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
    }

    /// A part `Notify` takes as a string: the part, or for one not given the
    /// empty string, which the specification reads as none.
    fn spec_text(part: Option<&str>) -> String {
        match part {
            Some(text) => text.to_string(),
            None => String::new(),
        }
    }

    /// The input a reply is typed into, with the button it stands beside;
    /// refused where the service has no box to type one in, for a choice,
    /// and for more than one input, which no service shows.
    fn reply_of(content: &Content, capabilities: &[String]) -> Result<Option<(NodeId, Option<NodeId>)>, String> {
        let Some(input) = content.inputs.first() else {
            return Ok(None);
        };
        if !capabilities.iter().any(|c| c == "inline-reply") {
            return Err(String::from("the desktop's notification service has no box to type a reply in, so a notification with an input is refused"));
        }
        if content.inputs.len() > 1 {
            return Err(format!("the desktop's notification service shows one reply box, and the notification has {} inputs", content.inputs.len()));
        }
        if !input.choices.is_empty() {
            return Err(String::from("the desktop's notification service shows a box to type a reply in, and no choices"));
        }
        let beside = content.buttons.iter().find(|b| b.input == Some(input.node)).map(|b| b.node);
        Ok(Some((input.node, beside)))
    }

    /// The `Notify` call that shows a notification's content.
    struct Call {
        app: String,
        icon: String,
        summary: String,
        body: String,
        actions: Vec<String>,
        hints: BTreeMap<&'static str, Variant<'static>>,
        timeout: i32,
    }

    impl Call {
        /// The call that shows `content` with its reply box `reply`, on a
        /// service of `capabilities`, adding to `notes` a line for each
        /// part the service cannot show.
        fn of(content: &Content, reply: Option<(NodeId, Option<NodeId>)>, capabilities: &[String], notes: &mut Vec<String>) -> Result<Call, String> {
            if content.xml.is_some() {
                return Err(String::from("a notification of the caller's own toast XML shows on Windows alone"));
            }
            let can = |c: &str| capabilities.iter().any(|x| x == c);
            let mut actions = vec![String::from(DEFAULT_ACTION), String::new()];
            let mut hints: BTreeMap<&'static str, Variant<'static>> = BTreeMap::new();
            if !content.buttons.is_empty() && !can("actions") {
                notes.push(String::from("the desktop's notification service shows no buttons, so the notification shows without them"));
            }
            for b in &content.buttons {
                if reply.is_some_and(|(_, beside)| beside == Some(b.node)) {
                    continue;
                }
                actions.push(b.node.to_string());
                actions.push(b.text.clone());
            }
            if let Some((input, beside)) = reply {
                let label = beside.and_then(|b| content.buttons.iter().find(|x| x.node == b)).map(|b| b.text.clone());
                let title = content.input(input).and_then(|i| i.title.clone());
                let shown = match (title, &label) {
                    (Some(t), _) => t,
                    (None, Some(l)) => l.clone(),
                    (None, None) => String::from("Reply"),
                };
                actions.push(String::from(REPLY_ACTION));
                actions.push(shown);
                if let Some(p) = content.input(input).and_then(|i| i.placeholder.clone()) {
                    hints.insert("x-kde-reply-placeholder-text", Variant::from(p));
                }
                if let Some(l) = label {
                    hints.insert("x-kde-reply-submit-button-text", Variant::from(l));
                }
            }
            if let Some(image) = &content.image {
                hints.insert("image-path", Variant::from(file_uri(image)));
            }
            if content.hero.is_some() {
                notes.push(String::from("the desktop's notification service shows no hero image, so the notification shows without it"));
            }
            if content.header.is_some() {
                notes.push(String::from("the desktop's notification service groups no notifications under a header, so the notification shows without one"));
            }
            if let Some(p) = &content.progress {
                match p.value {
                    Some(v) => {
                        hints.insert("value", Variant::from((v * 100.0).round() as i32));
                    }
                    None => notes.push(String::from("the desktop's notification service shows no progress bar that moves without a value, so the notification shows without it")),
                }
            }
            if let Some(a) = &content.audio {
                if a.silent {
                    hints.insert("suppress-sound", Variant::from(true));
                } else if let Some(name) = &a.sound {
                    if !can("sound") {
                        notes.push(String::from("the desktop's notification service plays no sounds, so the notification shows without its sound"));
                    }
                    hints.insert("sound-name", Variant::from(String::from(sound_name(name))));
                }
                if a.repeat {
                    notes.push(String::from("the desktop's notification service repeats no sound, so the notification's sound plays once"));
                }
            }
            let (urgency, timeout) = match content.scenario {
                Scenario::Default => (1u8, -1),
                Scenario::Reminder => (1u8, 0),
                Scenario::Alarm | Scenario::IncomingCall => (2u8, 0),
                Scenario::Urgent => (2u8, -1),
            };
            let urgency = if content.quiet { 0u8 } else { urgency };
            hints.insert("urgency", Variant::from(urgency));
            let body = spec_text(content.body.as_deref());
            let icon = match &content.logo {
                Some(path) => file_uri(path),
                None => String::new(),
            };
            Ok(Call {
                app: spec_text(Some(content.app.as_deref().unwrap_or(OWN_APP))),
                icon,
                summary: spec_text(content.title.as_deref()),
                body: if can("body-markup") { as_markup(&body) } else { body },
                actions,
                hints,
                timeout,
            })
        }

        /// The call as a line of text, for a snapshot's document.
        fn written(&self) -> String {
            let hints: Vec<String> = self.hints.iter().map(|(k, v)| format!("{k}={v}")).collect();
            format!(
                "Notify(app_name={:?}, app_icon={:?}, summary={:?}, body={:?}, actions={:?}, hints={{{}}}, expire_timeout={})",
                self.app,
                self.icon,
                self.summary,
                self.body,
                self.actions,
                hints.join(", "),
                self.timeout
            )
        }

        /// Sends the call, in place of the service's notification
        /// `replaces`, 0 for a new one; the service's number for it.
        fn send(&self, bus: &Connection, replaces: u32) -> Result<u32, String> {
            let actions: Vec<&str> = self.actions.iter().map(String::as_str).collect();
            let hints: HashMap<&str, &Variant<'_>> = self.hints.iter().map(|(k, v)| (*k, v)).collect();
            let body = (self.app.as_str(), replaces, self.icon.as_str(), self.summary.as_str(), self.body.as_str(), actions, hints, self.timeout);
            service(bus)?.call("Notify", &body).map_err(|e| format!("the desktop's notification service did not show the notification: {e}"))
        }
    }

    /// Shows notification `id` of `content` through the service, its
    /// answers reported through [`hear`].
    pub(super) fn show(id: WindowId, content: &Content) -> Result<Made, Refused> {
        let r = reached()?;
        let reply = reply_of(content, &r.capabilities).map_err(Refused::Input)?;
        let mut notes = Vec::new();
        let call = Call::of(content, reply, &r.capabilities, &mut notes)?;
        let number = call.send(&r.bus, 0)?;
        NUMBERED.rcu(|m| {
            let mut m = (**m).clone();
            m.insert(number, Numbered { id, reply });
            m
        });
        Ok(Made { toast: Toast { number }, document: call.written(), notes, pending: false })
    }

    /// Shows `content` in place of the notification the service shows now.
    pub(super) fn update(toast: &Toast, content: &Content) -> Result<(), String> {
        let r = reached()?;
        let reply = reply_of(content, &r.capabilities)?;
        let call = Call::of(content, reply, &r.capabilities, &mut Vec::new())?;
        let number = call.send(&r.bus, toast.number)?;
        if number != toast.number {
            return Err(format!("the notification service showed the update as a new notification, {number}, not in place of {}", toast.number));
        }
        Ok(())
    }

    /// Takes the notification away, from the screen and the service's list.
    pub(super) fn hide(toast: &Toast) -> Result<(), String> {
        let r = reached()?;
        NUMBERED.rcu(|m| {
            let mut m = (**m).clone();
            m.remove(&toast.number);
            m
        });
        service(&r.bus)?.call::<_, _, ()>("CloseNotification", &(toast.number,)).map_err(|e| format!("the notification service did not take notification {} away: {e}", toast.number))
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use proto::notification::{Audio, Button, Input, Progress};

        fn content() -> Content {
            Content {
                app: None,
                title: Some(String::from("Backup")),
                body: Some(String::from("1 < 2 & done")),
                image: Some(String::from("/home/ann/pic.png")),
                hero: Some(String::from("/home/ann/hero.png")),
                logo: None,
                buttons: vec![Button { node: 3, text: String::from("Open"), input: None }, Button { node: 4, text: String::from("Send"), input: Some(2) }],
                inputs: vec![Input { node: 2, title: None, placeholder: Some(String::from("Why?")), choices: Vec::new(), opens_with: None }],
                progress: Some(Progress { value: Some(0.42), title: None, status: None, value_text: None }),
                audio: Some(Audio { sound: None, silent: true, repeat: false }),
                scenario: Scenario::Urgent,
                header: None,
                xml: None,
                quiet: false,
            }
        }

        #[test]
        fn a_reply_needs_the_services_reply_box_and_goes_beside_its_button() {
            let c = content();
            assert!(reply_of(&c, &[String::from("actions")]).expect_err("no reply box").contains("no box"));
            assert_eq!(reply_of(&c, &[String::from("inline-reply")]).expect("a reply box"), Some((2, Some(4))));
        }

        #[test]
        fn a_call_names_each_button_by_its_node_and_says_what_it_leaves_out() {
            let caps = [String::from("actions"), String::from("body-markup"), String::from("inline-reply")];
            let mut notes = Vec::new();
            let call = Call::of(&content(), Some((2, Some(4))), &caps, &mut notes).expect("a call");
            assert_eq!(call.actions, vec!["default", "", "3", "Open", "inline-reply", "Send"]);
            assert_eq!(call.body, "1 &lt; 2 &amp; done");
            assert_eq!(call.hints.get("value"), Some(&Variant::from(42i32)));
            assert_eq!(call.hints.get("urgency"), Some(&Variant::from(2u8)));
            assert_eq!(call.hints.get("suppress-sound"), Some(&Variant::from(true)));
            assert_eq!(call.hints.get("image-path"), Some(&Variant::from(String::from("file:///home/ann/pic.png"))));
            assert!(notes.iter().any(|n| n.contains("hero image")), "{notes:?}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tree() -> Node {
        Node::new(1, Kind::Notification)
            .prop("title", Value::Str("Deploy?".into()))
            .child(Node::new(2, Kind::TextBox).named("Reason"))
            .child(Node::new(3, Kind::Button).named("Go").prop("text", Value::Str("Go".into())))
            .child(Node::new(4, Kind::ComboBox).named("When").prop("items", Value::List(vec![Value::Str("Now".into()), Value::Str("Later".into())])))
    }

    /// The node and kind of each widget event among `events`, in order.
    fn kinds(events: &[FromHelper]) -> Vec<(NodeId, EventKind)> {
        let mut out = Vec::new();
        for e in events {
            if let FromHelper::Event { node, kind, .. } = e {
                out.push((*node, *kind));
            }
        }
        out
    }

    #[test]
    fn an_answer_changes_its_inputs_then_clicks_its_button_and_ends_it() {
        let t = tree();
        let content = Content::of(&t).expect("a notification");
        let heard = Heard::Activated { notification: 9, arguments: String::from("pwrs-gui:button:3"), inputs: Ok(vec![(String::from("2"), String::from("busy")), (String::from("4"), String::from("1"))]) };
        let (events, ends) = events_of(9, t.id, &content, heard);
        assert!(ends);
        assert_eq!(kinds(&events), vec![(2, EventKind::Change), (4, EventKind::Change), (3, EventKind::Click)]);
        assert!(matches!(events.last(), Some(FromHelper::Closed { window: 9, reason: CloseReason::User })));
    }

    #[test]
    fn a_click_on_the_body_activates_its_root_and_a_header_click_ends_nothing() {
        let t = tree();
        let content = Content::of(&t).expect("a notification");
        let (events, ends) = events_of(9, t.id, &content, Heard::Activated { notification: 9, arguments: String::from(proto::notification::BODY_ARGUMENT), inputs: Ok(Vec::new()) });
        assert!(ends);
        assert_eq!(kinds(&events), vec![(1, EventKind::Activate)]);
        let (events, ends) = events_of(9, t.id, &content, Heard::Activated { notification: 9, arguments: String::from(proto::notification::HEADER_ARGUMENT), inputs: Ok(Vec::new()) });
        assert!(!ends, "a click on the header leaves the notification where it is");
        assert_eq!(kinds(&events), vec![(1, EventKind::Activate)]);
    }

    #[test]
    fn a_timeout_dismisses_without_ending_and_a_failure_ends() {
        let t = tree();
        let content = Content::of(&t).expect("a notification");
        let (events, ends) = events_of(9, t.id, &content, Heard::Dismissed { notification: 9, reason: Dismissal::Timeout });
        assert!(!ends, "a toast that timed out can still be answered from the system's list");
        assert!(matches!(&events[..], [FromHelper::Event { kind: EventKind::Dismiss, value: Value::Str(v), .. }] if v == "Timeout"));
        let (events, ends) = events_of(9, t.id, &content, Heard::Failed { notification: 9, text: String::from("no") });
        assert!(ends);
        assert!(matches!(events.last(), Some(FromHelper::Closed { reason: CloseReason::Failed, .. })));
    }

    #[test]
    fn an_answer_whose_inputs_cannot_be_read_still_clicks_and_says_why() {
        let t = tree();
        let content = Content::of(&t).expect("a notification");
        let heard = Heard::Activated { notification: 9, arguments: String::from("pwrs-gui:button:3"), inputs: Err(String::from("this Windows gives none")) };
        let (events, ends) = events_of(9, t.id, &content, heard);
        assert!(ends);
        assert!(matches!(&events[0], FromHelper::Error { text, .. } if text.contains("this Windows gives none")));
        assert_eq!(kinds(&events), vec![(3, EventKind::Click)]);
    }

    #[test]
    fn a_test_answers_inputs_by_name_and_a_choice_by_its_text_or_row() {
        let t = tree();
        let content = Content::of(&t).expect("a notification");
        let given = Value::Map(BTreeMap::from([(String::from("Reason"), Value::Str("busy".into())), (String::from("When"), Value::Str("Later".into()))]));
        assert_eq!(test_inputs(&t, &content, &given).expect("inputs"), vec![(String::from("2"), String::from("busy")), (String::from("4"), String::from("1"))]);
        let by_row = Value::Map(BTreeMap::from([(String::from("When"), Value::Int(0))]));
        assert_eq!(test_inputs(&t, &content, &by_row).expect("inputs"), vec![(String::from("4"), String::from("0"))]);
        let beyond = Value::Map(BTreeMap::from([(String::from("When"), Value::Int(5))]));
        assert!(test_inputs(&t, &content, &beyond).expect_err("no such row").contains("row 5"));
        let unknown = Value::Map(BTreeMap::from([(String::from("Nope"), Value::Str("x".into()))]));
        assert!(test_inputs(&t, &content, &unknown).is_err());
    }
}
