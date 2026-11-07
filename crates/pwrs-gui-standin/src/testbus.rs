//! A test bus for PWRS-GUI's tray icon and notification suites on Linux and
//! the BSDs: on the session bus the PowerShell process runs on, which a
//! gate gives each suite of its own through dbus-run-session, it owns
//! `org.kde.StatusNotifierWatcher` as a watcher with a host, and serves
//! `org.freedesktop.Notifications` as a notification server. It records
//! every notification the renderer sends and every one it takes away, reads
//! each tray item and its menu as a panel does, and acts as the person
//! would: a click on an item, a choice in its menu, and a notification's
//! action, reply or close. On Windows and macOS, where neither goes over
//! D-Bus, its commands refuse to start.

use pwrs::prelude::*;

/// A tray item the watcher holds, as the panel reads it.
#[psclass(name = "PwrsGuiStandin.BusItem")]
#[derive(Clone, Default)]
pub struct BusItem {
    /// The bus name the item registered under.
    pub service: String,
    /// The item's Id, which PWRS-GUI's tray icons give as their number.
    pub id: String,
    /// The item's title.
    pub title: String,
    /// The text of its tooltip.
    pub tooltip: String,
    /// Its status: Active, Passive or NeedsAttention.
    pub status: String,
    /// The sizes of its pictures, as 32x32.
    pub icon_sizes: Vec<String>,
    /// Its menu's entries in order: an entry's label, a separator as -,
    /// and an entry inside a submenu as the submenu's label, a slash and
    /// its own.
    pub menu: Vec<String>,
    /// The entries of `menu` that are disabled.
    pub disabled: Vec<String>,
}

/// A call the notification server took.
#[psclass(name = "PwrsGuiStandin.BusCall")]
#[derive(Clone, Default)]
pub struct BusCall {
    /// Notify or CloseNotification.
    pub call: String,
    /// The notification's number, as the server gave it.
    pub id: u32,
    /// For Notify, the number of the notification it replaces; 0 for none.
    pub replaces_id: u32,
    /// For Notify, the app's name.
    pub app: String,
    /// For Notify, the app's icon.
    pub icon: String,
    /// For Notify, the summary.
    pub summary: String,
    /// For Notify, the body.
    pub body: String,
    /// For Notify, each action's key.
    pub action_keys: Vec<String>,
    /// For Notify, each action's label, in the order of `action_keys`.
    pub action_labels: Vec<String>,
    /// For Notify, each hint as its name, an equals sign and its value, in
    /// the order of their names.
    pub hints: Vec<String>,
    /// For Notify, the timeout in milliseconds; -1 for the server's own.
    pub timeout: i32,
}

/// Starts the test bus on the session bus: the tray watcher and host, and
/// the notification server. Fails when the session bus cannot be reached
/// or another process owns either name. Starting it again leaves the
/// running one as it is.
///
/// # Examples
/// Start-GuiTestBus
#[cmdlet(verb = "Start", noun = "GuiTestBus")]
#[derive(Default)]
pub struct StartGuiTestBus {}

impl Cmdlet for StartGuiTestBus {
    fn process(&mut self, _ps: &Pipeline<'_>) -> PsResult<()> {
        served::start().map_err(refused)
    }
}

/// Stops the test bus, releasing its names; nothing when it is not running.
///
/// # Examples
/// Stop-GuiTestBus
#[cmdlet(verb = "Stop", noun = "GuiTestBus")]
#[derive(Default)]
pub struct StopGuiTestBus {}

impl Cmdlet for StopGuiTestBus {
    fn process(&mut self, _ps: &Pipeline<'_>) -> PsResult<()> {
        served::stop();
        Ok(())
    }
}

/// Writes each tray item registered with the test bus as a panel reads it
/// now: its Id, tooltip, status, pictures and menu. An item whose process
/// has let its bus name go is left out.
///
/// # Examples
/// Get-GuiTestBusItem
/// Get-GuiTestBusItem -Item 3
#[cmdlet(verb = "Get", noun = "GuiTestBusItem", output = ["PwrsGuiStandin.BusItem"])]
#[derive(Default)]
pub struct GetGuiTestBusItem {
    /// Only the item of this Id.
    #[param(position = 0)]
    pub item: Option<String>,
}

impl Cmdlet for GetGuiTestBusItem {
    fn process(&mut self, ps: &Pipeline<'_>) -> PsResult<()> {
        for item in served::items().map_err(refused)? {
            if self.item.as_ref().is_none_or(|wanted| *wanted == item.id) {
                ps.write(item)?;
            }
        }
        Ok(())
    }
}

/// Acts on a tray item as a person would: without -MenuItem a left click,
/// which the item hears as Activate, and with it a click on the menu entry
/// of that label, as `Get-GuiTestBusItem` writes it.
///
/// # Examples
/// Invoke-GuiTestBusItem -Item 3
/// Invoke-GuiTestBusItem -Item 3 -MenuItem 'More/Quit'
#[cmdlet(verb = "Invoke", noun = "GuiTestBusItem")]
#[derive(Default)]
pub struct InvokeGuiTestBusItem {
    /// The item's Id; the only item registered when not given.
    #[param(position = 0)]
    pub item: Option<String>,
    /// The menu entry to choose.
    #[param]
    pub menu_item: Option<String>,
}

impl Cmdlet for InvokeGuiTestBusItem {
    fn process(&mut self, _ps: &Pipeline<'_>) -> PsResult<()> {
        served::act_on_item(self.item.as_deref(), self.menu_item.as_deref()).map_err(refused)
    }
}

/// Writes every call the test bus's notification server took, oldest
/// first: each Notify, an update among them, and each CloseNotification.
///
/// # Examples
/// Get-GuiTestBusCall | Where-Object Summary -EQ 'Backup done'
#[cmdlet(verb = "Get", noun = "GuiTestBusCall", output = ["PwrsGuiStandin.BusCall"])]
#[derive(Default)]
pub struct GetGuiTestBusCall {}

impl Cmdlet for GetGuiTestBusCall {
    fn process(&mut self, ps: &Pipeline<'_>) -> PsResult<()> {
        for call in served::calls().map_err(refused)? {
            ps.write(call)?;
        }
        Ok(())
    }
}

/// Answers a notification the test bus shows as a person would, through
/// the server's signals: a click on the action of a key, a reply typed into
/// its box, or a close for a reason of the specification's: 1 expired, 2
/// dismissed by the person, 3 closed by the app.
///
/// # Examples
/// Invoke-GuiTestBusNotification -Notification 4 -Action default
/// Invoke-GuiTestBusNotification -Notification 4 -Reply 'On my way'
/// Invoke-GuiTestBusNotification -Notification 4 -Closed 2
#[cmdlet(verb = "Invoke", noun = "GuiTestBusNotification", default_parameter_set = "Action")]
#[derive(Default)]
pub struct InvokeGuiTestBusNotification {
    /// The notification's number, as the server gave it.
    #[param(mandatory, position = 0)]
    pub notification: u32,
    /// The key of the action clicked.
    #[param(mandatory, set = "Action")]
    pub action: Option<String>,
    /// The text of a reply.
    #[param(mandatory, set = "Reply")]
    pub reply: Option<String>,
    /// The reason of a close.
    #[param(mandatory, set = "Closed")]
    pub closed: Option<u32>,
}

impl Cmdlet for InvokeGuiTestBusNotification {
    fn process(&mut self, _ps: &Pipeline<'_>) -> PsResult<()> {
        let answer = match (&self.action, &self.reply, self.closed) {
            (Some(key), _, _) => served::Answer::Action(key.clone()),
            (None, Some(text), _) => served::Answer::Reply(text.clone()),
            (None, None, Some(reason)) => served::Answer::Closed(reason),
            (None, None, None) => return Err(PsError::new(ErrorCategory::InvalidArgument, "GuiTestBusAnswer", "give -Action, -Reply or -Closed").terminating()),
        };
        served::answer(self.notification, answer).map_err(refused)
    }
}

/// The error of a test bus that could not do what it was asked.
fn refused(why: String) -> PsError {
    PsError::new(ErrorCategory::InvalidOperation, "GuiTestBus", why).terminating()
}

#[cfg(all(unix, not(target_vendor = "apple")))]
mod served {
    //! The test bus itself, over zbus on the session bus.

    use super::{BusCall, BusItem};
    use arc_swap::{ArcSwap, ArcSwapOption};
    use std::collections::HashMap;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicU32, Ordering};
    use zbus::blocking::fdo::DBusProxy;
    use zbus::blocking::{Connection, Proxy};
    use zbus::message::Header;
    use zbus::names::BusName;
    use zbus::object_server::SignalEmitter;
    use zbus::zvariant::{OwnedValue, Value};

    /// The notification server's object path.
    const NOTIFICATIONS_PATH: &str = "/org/freedesktop/Notifications";

    /// The interface of a tray item.
    const ITEM: &str = "org.kde.StatusNotifierItem";

    /// What a person does to a notification.
    pub(crate) enum Answer {
        Action(String),
        Reply(String),
        Closed(u32),
    }

    /// What the bus has heard: each item registered, as its bus name and
    /// object path, every call the server took, and the last notification
    /// number it gave.
    #[derive(Default)]
    struct State {
        items: ArcSwap<Vec<(String, String)>>,
        calls: ArcSwap<Vec<BusCall>>,
        last: AtomicU32,
    }

    /// The bus while it runs: its connection, which holds its names, and
    /// what it has heard.
    struct Running {
        conn: Connection,
        state: Arc<State>,
    }

    static RUNNING: ArcSwapOption<Running> = ArcSwapOption::const_empty();

    /// The watcher of tray items, which is their host as well.
    struct Watcher {
        state: Arc<State>,
    }

    #[zbus::interface(name = "org.kde.StatusNotifierWatcher")]
    impl Watcher {
        /// Takes an item, named by its bus name, whose object is at
        /// /StatusNotifierItem, or by an object path on the caller's
        /// connection.
        async fn register_status_notifier_item(&self, service: &str, #[zbus(header)] header: Header<'_>, #[zbus(signal_emitter)] emitter: SignalEmitter<'_>) -> zbus::fdo::Result<()> {
            let (name, path) = if service.starts_with('/') {
                match header.sender() {
                    Some(sender) => (sender.to_string(), service.to_string()),
                    None => return Err(zbus::fdo::Error::InvalidArgs(format!("{service} is an object path, and the call names no sender"))),
                }
            } else {
                (service.to_string(), String::from("/StatusNotifierItem"))
            };
            let entry = (name.clone(), path.clone());
            self.state.items.rcu(|items| {
                let mut items = (**items).clone();
                if !items.contains(&entry) {
                    items.push(entry.clone());
                }
                items
            });
            Self::status_notifier_item_registered(&emitter, &format!("{name}{path}")).await?;
            Ok(())
        }

        /// Takes a host; the bus is its own.
        fn register_status_notifier_host(&self, _service: &str) {}

        #[zbus(property)]
        fn registered_status_notifier_items(&self) -> Vec<String> {
            self.state.items.load().iter().map(|(name, path)| format!("{name}{path}")).collect()
        }

        #[zbus(property)]
        fn is_status_notifier_host_registered(&self) -> bool {
            true
        }

        #[zbus(property)]
        fn protocol_version(&self) -> i32 {
            0
        }

        #[zbus(signal)]
        async fn status_notifier_item_registered(emitter: &SignalEmitter<'_>, service: &str) -> zbus::Result<()>;
    }

    /// The notification server.
    struct Server {
        state: Arc<State>,
    }

    #[zbus::interface(name = "org.freedesktop.Notifications")]
    impl Server {
        /// Actions, a body with markup, a reply box, sounds, and
        /// notifications that stay until answered.
        fn get_capabilities(&self) -> Vec<String> {
            ["actions", "body", "body-markup", "inline-reply", "persistence", "sound"].iter().map(|c| c.to_string()).collect()
        }

        /// Records a notification, numbering a new one and keeping the
        /// number of one it replaces.
        #[allow(clippy::too_many_arguments)]
        fn notify(&self, app_name: String, replaces_id: u32, app_icon: String, summary: String, body: String, actions: Vec<String>, hints: HashMap<String, OwnedValue>, expire_timeout: i32) -> u32 {
            let id = if replaces_id == 0 { self.state.last.fetch_add(1, Ordering::AcqRel) + 1 } else { replaces_id };
            let action_keys = actions.iter().step_by(2).cloned().collect();
            let action_labels = actions.iter().skip(1).step_by(2).cloned().collect();
            let mut named: Vec<String> = hints.iter().map(|(name, value)| format!("{name}={}", hint_text(value))).collect();
            named.sort();
            let call = BusCall {
                call: String::from("Notify"),
                id,
                replaces_id,
                app: app_name,
                icon: app_icon,
                summary,
                body,
                action_keys,
                action_labels,
                hints: named,
                timeout: expire_timeout,
            };
            record(&self.state, call);
            id
        }

        /// Records the close, and tells the app it was closed by the app.
        async fn close_notification(&self, id: u32, #[zbus(signal_emitter)] emitter: SignalEmitter<'_>) -> zbus::fdo::Result<()> {
            record(&self.state, BusCall { call: String::from("CloseNotification"), id, ..BusCall::default() });
            Self::notification_closed(&emitter, id, 3).await?;
            Ok(())
        }

        fn get_server_information(&self) -> (String, String, String, String) {
            (String::from("PWRS-GUI test bus"), String::from("PWRS-GUI"), String::from("1"), String::from("1.2"))
        }

        #[zbus(signal)]
        async fn notification_closed(emitter: &SignalEmitter<'_>, id: u32, reason: u32) -> zbus::Result<()>;

        #[zbus(signal)]
        async fn action_invoked(emitter: &SignalEmitter<'_>, id: u32, action_key: &str) -> zbus::Result<()>;

        #[zbus(signal)]
        async fn notification_replied(emitter: &SignalEmitter<'_>, id: u32, text: &str) -> zbus::Result<()>;
    }

    /// Adds `call` to what the server has taken.
    fn record(state: &State, call: BusCall) {
        state.calls.rcu(|calls| {
            let mut calls = (**calls).clone();
            calls.push(call.clone());
            calls
        });
    }

    /// A hint's value as text: a number, a flag or text as it is, and any
    /// other value as GVariant text.
    fn hint_text(value: &Value<'_>) -> String {
        match value {
            Value::U8(n) => n.to_string(),
            Value::Bool(b) => b.to_string(),
            Value::I16(n) => n.to_string(),
            Value::U16(n) => n.to_string(),
            Value::I32(n) => n.to_string(),
            Value::U32(n) => n.to_string(),
            Value::I64(n) => n.to_string(),
            Value::U64(n) => n.to_string(),
            Value::Str(s) => s.to_string(),
            Value::Value(inner) => hint_text(inner),
            other => other.to_string(),
        }
    }

    /// Starts the bus, unless it runs.
    pub(crate) fn start() -> Result<(), String> {
        if RUNNING.load().is_some() {
            return Ok(());
        }
        let state = Arc::new(State::default());
        let conn = zbus::blocking::connection::Builder::session()
            .and_then(|b| b.name("org.kde.StatusNotifierWatcher"))
            .and_then(|b| b.name("org.freedesktop.Notifications"))
            .and_then(|b| b.serve_at("/StatusNotifierWatcher", Watcher { state: Arc::clone(&state) }))
            .and_then(|b| b.serve_at(NOTIFICATIONS_PATH, Server { state: Arc::clone(&state) }))
            .and_then(|b| b.build())
            .map_err(|e| format!("the test bus cannot start on the session bus: {e}"))?;
        RUNNING.store(Some(Arc::new(Running { conn, state })));
        Ok(())
    }

    /// Stops the bus, its connection closing as the last reference goes.
    pub(crate) fn stop() {
        RUNNING.store(None);
    }

    /// The running bus.
    fn running() -> Result<Arc<Running>, String> {
        RUNNING.load_full().ok_or_else(|| String::from("the test bus is not running; Start-GuiTestBus starts it"))
    }

    /// Whether some process owns bus name `name` now.
    fn owned(conn: &Connection, name: &str) -> Result<bool, String> {
        let bus = DBusProxy::new(conn).map_err(|e| format!("the session bus cannot be asked about {name}: {e}"))?;
        let named = BusName::try_from(name).map_err(|e| format!("{name} is no bus name: {e}"))?;
        bus.name_has_owner(named).map_err(|e| format!("the session bus does not say whether {name} has an owner: {e}"))
    }

    /// Property `property` of interface `iface` of the object at `path`
    /// of `name`, read as it is now.
    fn property(conn: &Connection, name: &str, path: &str, iface: &str, property: &str) -> Result<OwnedValue, String> {
        let props = Proxy::new(conn, name.to_string(), path.to_string(), "org.freedesktop.DBus.Properties").map_err(|e| format!("{name}{path} cannot be reached: {e}"))?;
        props.call("Get", &(iface, property)).map_err(|e| format!("{name}{path} gives no {property}: {e}"))
    }

    /// Text within `value`, through any variant around it.
    fn text_of(value: &Value<'_>) -> Option<String> {
        match value {
            Value::Str(s) => Some(s.to_string()),
            Value::ObjectPath(p) => Some(p.to_string()),
            Value::Value(inner) => text_of(inner),
            _ => None,
        }
    }

    /// Text property `property` of a tray item: refused when it is no text.
    fn text_property(conn: &Connection, name: &str, path: &str, property_name: &str) -> Result<String, String> {
        let value = property(conn, name, path, ITEM, property_name)?;
        text_of(&value).ok_or_else(|| format!("{name}{path}'s {property_name} is not text: {value:?}"))
    }

    /// A flag within `value`, through any variant around it.
    fn flag_of(value: &Value<'_>) -> Option<bool> {
        match value {
            Value::Bool(b) => Some(*b),
            Value::Value(inner) => flag_of(inner),
            _ => None,
        }
    }

    /// The fields of the structure within `value`.
    fn fields_of<'v>(value: &'v Value<'v>) -> Option<&'v [Value<'v>]> {
        match value {
            Value::Structure(s) => Some(s.fields()),
            Value::Value(inner) => fields_of(inner),
            _ => None,
        }
    }

    /// The sizes of the pictures in an `a(iiay)` value.
    fn sizes_of(value: &Value<'_>) -> Vec<String> {
        let list = match value {
            Value::Array(a) => a.inner(),
            Value::Value(inner) => return sizes_of(inner),
            _ => return Vec::new(),
        };
        list.iter()
            .filter_map(|p| match fields_of(p) {
                Some([Value::I32(w), Value::I32(h), ..]) => Some(format!("{w}x{h}")),
                _ => None,
            })
            .collect()
    }

    /// One menu entry: its dbusmenu id, its label as `BusItem::menu` shows
    /// it, and whether it is enabled.
    type Entry = (i32, String, bool);

    /// A dbusmenu node as GetLayout gives it: its id, its properties and
    /// its children, each a node in a variant.
    type Layout = (i32, HashMap<String, OwnedValue>, Vec<OwnedValue>);

    /// An item as a panel reads it, with what acting on it takes: the
    /// object path of the item, its menu's entries, and the menu's path.
    struct Seen {
        path: String,
        item: BusItem,
        entries: Vec<Entry>,
        menu: String,
    }

    /// The entries under a dbusmenu node's `children`, each a `(ia{sv}av)`
    /// structure, appended to `out` with `prefix` before each label.
    fn entries(children: &[Value<'_>], prefix: &str, out: &mut Vec<Entry>) -> Result<(), String> {
        for child in children {
            let Some([Value::I32(id), Value::Dict(props), Value::Array(grand)]) = fields_of(child) else {
                return Err(format!("a menu entry under {prefix:?} is not a dbusmenu node: {child:?}"));
            };
            let mut label = String::new();
            let mut enabled = true;
            let mut separator = false;
            for (key, value) in props.iter() {
                match text_of(key).as_deref() {
                    Some("label") => label = text_of(value).ok_or_else(|| format!("entry {id}'s label is not text: {value:?}"))?,
                    Some("enabled") => enabled = flag_of(value).ok_or_else(|| format!("entry {id}'s enabled is not a flag: {value:?}"))?,
                    Some("type") => separator = text_of(value).as_deref() == Some("separator"),
                    Some(_other) => {}
                    None => return Err(format!("entry {id} has a property whose name is not text: {key:?}")),
                }
            }
            if separator {
                out.push((*id, String::from("-"), enabled));
                continue;
            }
            let shown = format!("{prefix}{label}");
            out.push((*id, shown.clone(), enabled));
            entries(grand.inner(), &format!("{shown}/"), out)?;
        }
        Ok(())
    }

    /// The menu at `menu` of `name`, read with dbusmenu's GetLayout.
    fn menu_of(conn: &Connection, name: &str, menu: &str) -> Result<Vec<Entry>, String> {
        let proxy = Proxy::new(conn, name.to_string(), menu.to_string(), "com.canonical.dbusmenu").map_err(|e| format!("{name}{menu} cannot be reached: {e}"))?;
        let (_revision, layout): (u32, Layout) = proxy.call("GetLayout", &(0i32, -1i32, Vec::<String>::new())).map_err(|e| format!("{name}{menu} gives no menu: {e}"))?;
        let children: Vec<Value<'_>> = layout.2.iter().map(|c| Value::from(c.clone())).collect();
        let mut out = Vec::new();
        entries(&children, "", &mut out)?;
        Ok(out)
    }

    /// The item at `path` of `name` as a panel reads it, with its menu's
    /// entries and the menu's object path.
    fn item_of(conn: &Connection, name: &str, path: &str) -> Result<Seen, String> {
        let id = text_property(conn, name, path, "Id")?;
        let title = text_property(conn, name, path, "Title")?;
        let status = text_property(conn, name, path, "Status")?;
        let tip = property(conn, name, path, ITEM, "ToolTip")?;
        let tooltip = match fields_of(&tip) {
            Some([_, _, _, description]) => text_of(description).ok_or_else(|| format!("{name}{path}'s tooltip text is not text: {description:?}"))?,
            _ => return Err(format!("{name}{path}'s ToolTip is not a tooltip: {tip:?}")),
        };
        let pixmaps = property(conn, name, path, ITEM, "IconPixmap")?;
        let icon_sizes = sizes_of(&pixmaps);
        let menu_path = text_property(conn, name, path, "Menu")?;
        let entries = if menu_path == "/" { Vec::new() } else { menu_of(conn, name, &menu_path)? };
        let menu = entries.iter().map(|(_, label, _)| label.clone()).collect();
        let disabled = entries.iter().filter(|(_, _, enabled)| !enabled).map(|(_, label, _)| label.clone()).collect();
        let item = BusItem { service: name.to_string(), id, title, tooltip, status, icon_sizes, menu, disabled };
        Ok(Seen { path: path.to_string(), item, entries, menu: menu_path })
    }

    /// Each item registered whose process still owns its bus name, as a
    /// panel reads it now.
    fn live_items(r: &Running) -> Result<Vec<Seen>, String> {
        let mut out = Vec::new();
        for (name, path) in r.state.items.load().iter() {
            if !owned(&r.conn, name)? {
                continue;
            }
            out.push(item_of(&r.conn, name, path)?);
        }
        Ok(out)
    }

    /// Each item registered whose process still owns its bus name, as a
    /// panel reads it now.
    pub(crate) fn items() -> Result<Vec<BusItem>, String> {
        let r = running()?;
        Ok(live_items(&r)?.into_iter().map(|seen| seen.item).collect())
    }

    /// Clicks the item of Id `wanted`, the only one when none is named, or
    /// with `entry` chooses its menu entry of that label.
    pub(crate) fn act_on_item(wanted: Option<&str>, entry: Option<&str>) -> Result<(), String> {
        let r = running()?;
        let mut found: Vec<Seen> = live_items(&r)?.into_iter().filter(|seen| wanted.is_none_or(|w| w == seen.item.id)).collect();
        let Seen { path, item, entries, menu } = match found.len() {
            1 => found.remove(0),
            0 => {
                return Err(match wanted {
                    Some(w) => format!("no tray item {w} is registered"),
                    None => String::from("no tray item is registered"),
                });
            }
            n => return Err(format!("{n} tray items are registered; name one with -Item")),
        };
        let id = &item.id;
        match entry {
            None => {
                let target = Proxy::new(&r.conn, item.service.clone(), path, ITEM).map_err(|e| format!("tray item {id} cannot be reached: {e}"))?;
                target.call::<_, _, ()>("Activate", &(0i32, 0i32)).map_err(|e| format!("tray item {id} took no click: {e}"))
            }
            Some(label) => {
                let Some((entry_id, _, _)) = entries.iter().find(|(_, l, _)| l == label) else {
                    let labels: Vec<&str> = entries.iter().map(|(_, l, _)| l.as_str()).collect();
                    return Err(format!("tray item {id}'s menu has no entry {label}; its entries are {}", labels.join(", ")));
                };
                let target = Proxy::new(&r.conn, item.service.clone(), menu, "com.canonical.dbusmenu").map_err(|e| format!("tray item {id}'s menu cannot be reached: {e}"))?;
                target.call::<_, _, ()>("Event", &(*entry_id, "clicked", Value::from(0i32), 0u32)).map_err(|e| format!("tray item {id}'s menu took no click on {label}: {e}"))
            }
        }
    }

    /// Every call the server took, oldest first.
    pub(crate) fn calls() -> Result<Vec<BusCall>, String> {
        Ok(running()?.state.calls.load().iter().cloned().collect())
    }

    /// Sends the signal of `answer` to notification `id`.
    pub(crate) fn answer(id: u32, answer: Answer) -> Result<(), String> {
        let r = running()?;
        let emitter = SignalEmitter::new(r.conn.inner(), NOTIFICATIONS_PATH).map_err(|e| format!("the server's signals cannot be sent: {e}"))?;
        let sent = match &answer {
            Answer::Action(key) => zbus::block_on(Server::action_invoked(&emitter, id, key)),
            Answer::Reply(text) => zbus::block_on(Server::notification_replied(&emitter, id, text)),
            Answer::Closed(reason) => zbus::block_on(Server::notification_closed(&emitter, id, *reason)),
        };
        sent.map_err(|e| format!("notification {id}'s answer could not be sent: {e}"))
    }
}

#[cfg(not(all(unix, not(target_vendor = "apple"))))]
mod served {
    //! Neither tray icons nor notifications go over D-Bus here, so there
    //! is no test bus to start.

    use super::{BusCall, BusItem};

    /// What a person does to a notification.
    pub(crate) enum Answer {
        Action(String),
        Reply(String),
        Closed(u32),
    }

    /// Why there is no test bus here.
    fn absent() -> String {
        String::from("the test bus runs on Linux and the BSDs, where tray icons and notifications go over D-Bus")
    }

    pub(crate) fn start() -> Result<(), String> {
        Err(absent())
    }

    pub(crate) fn stop() {}

    pub(crate) fn items() -> Result<Vec<BusItem>, String> {
        Err(absent())
    }

    pub(crate) fn act_on_item(_wanted: Option<&str>, _entry: Option<&str>) -> Result<(), String> {
        Err(absent())
    }

    pub(crate) fn calls() -> Result<Vec<BusCall>, String> {
        Err(absent())
    }

    pub(crate) fn answer(id: u32, answer: Answer) -> Result<(), String> {
        let what = match answer {
            Answer::Action(key) => format!("action {key}"),
            Answer::Reply(text) => format!("reply {text:?}"),
            Answer::Closed(reason) => format!("close for reason {reason}"),
        };
        Err(format!("notification {id}'s {what} has no bus to go over: {}", absent()))
    }
}
