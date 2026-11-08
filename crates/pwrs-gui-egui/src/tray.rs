//! Tray icons: each a [`Kind::TrayIcon`] tree a session shows with
//! [`ToHelper::OpenTray`] through tray-icon, in the notification area on
//! Windows, the menu bar on macOS, and on Linux and the BSDs as a
//! StatusNotifierItem, whose clicks and menu choices come back as the tray
//! icon's events on its ring, as a window's do.
//!
//! tray-icon takes one handler for its clicks and one for its menus for the
//! life of the process, so both push what they hear onto [`HEARD`], a
//! queue of the process's own, and wake whichever renderer's loop runs now,
//! through [`WAKE`]; that loop's next frame takes them.
//!
//! [`ToHelper::OpenTray`]: proto::ToHelper::OpenTray

use crate::egui;
use arc_swap::ArcSwapOption;
use crossbeam_queue::SegQueue;
use proto::renderer::WakeLoop;
use proto::{EventKind, FromHelper, Kind, Node, NodeId, PatchOp, RingName, Value, WindowId, WindowSummary};
use pwrs_gui_proto as proto;
use std::collections::BTreeMap;
use std::sync::Arc;

/// What the platform heard a person do to a tray icon.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Heard {
    /// A left click, or with `double` a double click, on the icon.
    Clicked { tray: WindowId, double: bool },
    /// A menu item chosen.
    Chosen { tray: WindowId, node: NodeId },
}

/// What the platform heard, for the loop's next frame to take.
static HEARD: SegQueue<Heard> = SegQueue::new();

/// What wakes the loop of the renderer running now.
static WAKE: ArcSwapOption<WakeLoop> = ArcSwapOption::const_empty();

/// Records what the platform heard and wakes the renderer's loop.
fn hear(heard: Heard) {
    HEARD.push(heard);
    match WAKE.load().as_ref() {
        Some(wake) => wake(),
        None => proto::trace::line(|| String::from("egui: a tray icon was used while no renderer's loop could be woken")),
    }
}

/// A menu item's id as the platform carries it: the tray icon's number and
/// the item's node.
fn item_id(tray: WindowId, node: NodeId) -> String {
    format!("{tray}:{node}")
}

/// The tray icon and node an item id names, as [`item_id`] writes it, or
/// why it names none.
fn item_of(id: &str) -> Result<(WindowId, NodeId), String> {
    let (tray, node) = id.split_once(':').ok_or_else(|| format!("the menu item id {id} has no colon between a tray icon and a node"))?;
    let tray = tray.parse().map_err(|e| format!("the menu item id {id} names no tray icon: {e}"))?;
    let node = node.parse().map_err(|e| format!("the menu item id {id} names no node: {e}"))?;
    Ok((tray, node))
}

/// The tray icons a renderer shows, by number.
#[derive(Default)]
pub(crate) struct Trays {
    shown: BTreeMap<WindowId, Tray>,
}

/// A tray icon shown: the ring its events go to, none once detached, its
/// tree as last set, and the platform's icon.
pub(crate) struct Tray {
    pub(crate) ring: Option<RingName>,
    pub(crate) tree: Node,
    icon: platform::Icon,
}

impl Trays {
    /// Shows tray icon `tray` from `tree`, its loop woken through `wake` when
    /// a person uses it. Returns what the person should hear of where it
    /// shows, or why it cannot be shown: the tree is no tray icon, or the
    /// platform refuses it.
    pub(crate) fn open(&mut self, tray: WindowId, ring: RingName, tree: Node, wake: Option<&WakeLoop>) -> Result<Vec<String>, String> {
        check(&tree)?;
        if let Some(w) = wake {
            WAKE.store(Some(Arc::new(Arc::clone(w))));
        }
        let icon = platform::show(tray, &tree)?;
        self.shown.insert(tray, Tray { ring: Some(ring), tree, icon });
        Ok(platform::where_shown().into_iter().collect())
    }

    /// Whether `tray` is one of these.
    pub(crate) fn has(&self, tray: WindowId) -> bool {
        self.shown.contains_key(&tray)
    }

    /// The ring tray icon `tray`'s events go to.
    pub(crate) fn ring(&self, tray: WindowId) -> Option<RingName> {
        self.shown.get(&tray).and_then(|t| t.ring.clone())
    }

    /// Stops sending tray icon `tray`'s events, returning its ring.
    pub(crate) fn detach(&mut self, tray: WindowId) -> Option<RingName> {
        self.shown.get_mut(&tray).and_then(|t| t.ring.take())
    }

    /// Takes tray icon `tray` away, the platform's icon with it.
    pub(crate) fn remove(&mut self, tray: WindowId) -> Option<Tray> {
        self.shown.remove(&tray)
    }

    /// Takes every tray icon away.
    pub(crate) fn take_all(&mut self) -> Vec<(WindowId, Tray)> {
        std::mem::take(&mut self.shown).into_iter().collect()
    }

    /// Changes tray icon `tray`'s tree by `ops` and shows the change: its
    /// icon, its tooltip and its menu. Returns what could not be changed.
    pub(crate) fn patch(&mut self, tray: WindowId, ops: Vec<PatchOp>) -> Vec<String> {
        let Some(t) = self.shown.get_mut(&tray) else {
            return vec![format!("no tray icon {tray} is shown")];
        };
        let mut problems: Vec<String> = Vec::new();
        for op in ops {
            if let Err(why) = apply(&mut t.tree, op) {
                problems.push(why);
            }
        }
        if let Err(why) = check(&t.tree).and_then(|()| platform::update(&t.icon, tray, &t.tree)) {
            problems.push(why);
        }
        problems
    }

    /// The events of what the platform heard since the last call, each with
    /// the ring it goes to, for tray icons still shown and attached.
    pub(crate) fn heard(&self) -> Vec<(RingName, FromHelper)> {
        let mut out = Vec::new();
        while let Some(h) = HEARD.pop() {
            let (tray, node, kind) = match h {
                Heard::Clicked { tray, double } => (tray, None, if double { EventKind::DoubleClick } else { EventKind::Click }),
                Heard::Chosen { tray, node } => (tray, Some(node), EventKind::Click),
            };
            let Some(t) = self.shown.get(&tray) else {
                proto::trace::line(|| format!("egui: tray icon {tray} was used after it was taken away"));
                continue;
            };
            let Some(ring) = &t.ring else {
                continue;
            };
            let node = node.unwrap_or(t.tree.id);
            if t.tree.find(node).is_some_and(Node::disabled) {
                continue;
            }
            out.push((ring.clone(), FromHelper::Event { window: tray, node, kind, value: Value::Null, at: proto::clock::now_ns() }));
        }
        out
    }

    /// Acts on tray icon `tray` as a person would: with `name` empty on the
    /// icon itself, a Click or a DoubleClick, and otherwise a Click on the
    /// enabled menu item of that name. Returns the node and event to raise.
    pub(crate) fn test_event(&self, tray: WindowId, name: &str, kind: EventKind) -> Result<(NodeId, EventKind), String> {
        let t = self.shown.get(&tray).ok_or_else(|| format!("no tray icon {tray} is shown"))?;
        if name.is_empty() {
            return match kind {
                EventKind::Click | EventKind::DoubleClick => Ok((t.tree.id, kind)),
                other => Err(format!("a tray icon raises Click and DoubleClick, not {other:?}")),
            };
        }
        let item = t.tree.find_named(name).ok_or_else(|| format!("tray icon {tray} has no menu item named {name}; its items are {}", t.tree.names().join(", ")))?;
        if item.kind != Kind::MenuItem {
            return Err(format!("{name} in tray icon {tray} is a {:?}, which raises nothing", item.kind));
        }
        if kind != EventKind::Click {
            return Err(format!("a tray icon's menu item raises Click, not {kind:?}"));
        }
        if item.disabled() {
            return Err(format!("the menu item {name} is disabled"));
        }
        Ok((item.id, EventKind::Click))
    }

    /// Each tray icon shown, as a list of windows names it: its tooltip as
    /// its title, empty for one with none, and its menu items' names.
    pub(crate) fn summaries(&self) -> Vec<WindowSummary> {
        let titled = |t: &Tray| match t.tree.props.get("tooltip") {
            Some(Value::Str(tip)) => tip.clone(),
            _ => String::new(),
        };
        self.shown.iter().map(|(id, t)| WindowSummary { window: *id, title: titled(t), names: t.tree.names(), kind: Kind::TrayIcon }).collect()
    }

    /// Tray icon `tray`'s tree, as a snapshot of a window gives a window's.
    pub(crate) fn snapshot(&self, tray: WindowId) -> Option<proto::Snapshot> {
        let t = self.shown.get(&tray)?;
        Some(proto::Snapshot { tree: t.tree.clone(), state: BTreeMap::new(), pixels: None, bounds: BTreeMap::new(), engine: None, focused: None, copied: None, sound: BTreeMap::new() })
    }
}

/// Fails, saying why, unless `tree` is a tray icon whose children make a
/// menu: MenuItem, Menu and Separator widgets, a Menu holding the same,
/// each item and menu with its text.
fn check(tree: &Node) -> Result<(), String> {
    if tree.kind != Kind::TrayIcon {
        return Err(format!("a tray icon's root is a TrayIcon, not a {:?}", tree.kind));
    }
    fn menu(items: &[Node]) -> Result<(), String> {
        for item in items {
            match item.kind {
                Kind::Separator => {}
                Kind::MenuItem => {
                    shown_text(item)?;
                }
                Kind::Menu => {
                    shown_text(item)?;
                    menu(&item.children)?;
                }
                other => return Err(format!("a tray icon's menu holds MenuItem, Menu and Separator widgets, and node {} is a {other:?}", item.id)),
            }
        }
        Ok(())
    }
    menu(&tree.children)
}

/// Changes `tree` as `op` says, as a window's tree changes.
fn apply(tree: &mut Node, op: PatchOp) -> Result<(), String> {
    match op {
        PatchOp::SetProps { node, props } => {
            let n = tree.find_mut(node).ok_or_else(|| format!("the tray icon has no node {node}"))?;
            n.props.extend(props);
            Ok(())
        }
        PatchOp::Insert { parent, index, node } => {
            let at = usize::try_from(index).map_err(|e| format!("position {index} is beyond this platform's address space: {e}"))?;
            let p = tree.find_mut(parent).ok_or_else(|| format!("the tray icon has no node {parent}"))?;
            let at = at.min(p.children.len());
            p.children.insert(at, node);
            Ok(())
        }
        PatchOp::Remove { node } => match crate::remove_node(tree, node) {
            Some(_) => Ok(()),
            None => Err(format!("the tray icon has no node {node} to remove")),
        },
        PatchOp::Replace { node } => {
            let n = tree.find_mut(node.id).ok_or_else(|| format!("the tray icon has no node {} to replace", node.id))?;
            *n = node;
            Ok(())
        }
        other => Err(format!("a tray icon takes no change {other:?}")),
    }
}

/// The text of a menu item, or a menu's title, as the platform's menus show
/// it: each `&`, which would mark the next letter as the item's key,
/// doubled, so it shows as written. Refused for an entry with none.
fn shown_text(node: &Node) -> Result<String, String> {
    let key = if node.kind == Kind::Menu { "title" } else { "text" };
    match node.props.get(key) {
        Some(Value::Str(text)) => Ok(text.replace('&', "&&")),
        other => Err(format!("{:?} {} of the tray icon's menu has no {key} to show, but {other:?}", node.kind, node.id)),
    }
}

/// A tray icon's picture as straight RGBA pixels and their size: a raster
/// picture as it is, one larger than 256 pixels on a side shrunk to fit
/// 256, and an SVG drawn 64 pixels on its longer side.
fn icon_pixels(picture: &Value) -> Result<(Vec<u8>, u32, u32), String> {
    let (bytes, format) = crate::picture_parts(picture)?;
    if format == "svg" {
        let options = resvg::usvg::Options::default();
        let hint = egui::load::SizeHint::Size { width: 64, height: 64, maintain_aspect_ratio: true };
        let drawn = crate::draw_svg(&bytes, hint, &options)?;
        let rgba = drawn.pixels.iter().flat_map(|p| p.to_srgba_unmultiplied()).collect();
        let [w, h] = drawn.size;
        return Ok((rgba, u32::try_from(w).map_err(|e| e.to_string())?, u32::try_from(h).map_err(|e| e.to_string())?));
    }
    let decoded = image::load_from_memory(&bytes).map_err(|e| format!("the tray icon's {format} picture cannot be read: {e}"))?;
    let decoded = if decoded.width() > 256 || decoded.height() > 256 { decoded.resize(256, 256, image::imageops::FilterType::Lanczos3) } else { decoded };
    let rgba = decoded.into_rgba8();
    let (w, h) = rgba.dimensions();
    Ok((rgba.into_raw(), w, h))
}

mod platform {
    //! The system's tray through tray-icon and its menus, muda: Windows'
    //! notification area, macOS's menu bar, and on Linux and the BSDs a
    //! StatusNotifierItem over D-Bus, which tray-icon serves through ksni.

    use super::{Heard, hear, icon_pixels, item_id, item_of, shown_text};
    use proto::{Kind, Node, Value, WindowId};
    use pwrs_gui_proto as proto;
    use std::sync::Once;
    use tray_icon::menu::{IsMenuItem, Menu, MenuEvent, MenuItem, PredefinedMenuItem, Submenu};
    use tray_icon::{MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent};

    /// The platform's tray icon.
    pub(super) struct Icon(TrayIcon);

    /// What a click tray-icon reports is to PowerShell: a left click, a
    /// double click, or nothing, as a right click opens the menu.
    fn clicked(e: &TrayIconEvent) -> Option<bool> {
        match e {
            TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. } => Some(false),
            TrayIconEvent::DoubleClick { button: MouseButton::Left, .. } => Some(true),
            TrayIconEvent::Click { .. } | TrayIconEvent::DoubleClick { .. } | TrayIconEvent::Enter { .. } | TrayIconEvent::Move { .. } | TrayIconEvent::Leave { .. } => None,
            _ => None,
        }
    }

    /// Installs, once for the process, the handlers that pass what tray-icon
    /// hears on to [`hear`].
    fn listen() {
        static LISTENING: Once = Once::new();
        LISTENING.call_once(|| {
            TrayIconEvent::set_event_handler(Some(|e: TrayIconEvent| {
                let Some(double) = clicked(&e) else {
                    return;
                };
                match e.id().as_ref().parse::<WindowId>() {
                    Ok(tray) => hear(Heard::Clicked { tray, double }),
                    Err(why) => proto::trace::line(|| format!("egui: a tray icon with no number of ours was clicked: {why}")),
                }
            }));
            MenuEvent::set_event_handler(Some(|e: MenuEvent| match item_of(e.id().as_ref()) {
                Ok((tray, node)) => hear(Heard::Chosen { tray, node }),
                Err(why) => proto::trace::line(|| format!("egui: a menu item that is not ours was chosen: {why}")),
            }));
        });
    }

    /// The menu `tree`'s children make, each item's id naming its node.
    fn menu_of(tray: WindowId, tree: &Node) -> Result<Menu, String> {
        let menu = Menu::new();
        for item in &tree.children {
            let made = entry(tray, item)?;
            menu.append(made.as_ref()).map_err(|e| format!("the tray icon's menu could not take node {}: {e}", item.id))?;
        }
        Ok(menu)
    }

    /// One menu entry of `node`: an item, a separator, or a submenu of its
    /// own entries.
    fn entry(tray: WindowId, node: &Node) -> Result<Box<dyn IsMenuItem>, String> {
        let enabled = !node.disabled();
        Ok(match node.kind {
            Kind::Separator => Box::new(PredefinedMenuItem::separator()),
            Kind::Menu => {
                let sub = Submenu::with_id(item_id(tray, node.id), shown_text(node)?, enabled);
                for item in &node.children {
                    let made = entry(tray, item)?;
                    sub.append(made.as_ref()).map_err(|e| format!("the menu {} could not take node {}: {e}", node.id, item.id))?;
                }
                Box::new(sub)
            }
            _ => Box::new(MenuItem::with_id(item_id(tray, node.id), shown_text(node)?, enabled, None)),
        })
    }

    /// The platform's icon of `tree`'s picture.
    fn picture(tree: &Node) -> Result<tray_icon::Icon, String> {
        let picture = tree.props.get("icon").ok_or_else(|| String::from("a tray icon needs its icon"))?;
        let (rgba, w, h) = icon_pixels(picture)?;
        tray_icon::Icon::from_rgba(rgba, w, h).map_err(|e| format!("the tray icon's picture cannot be made an icon: {e}"))
    }

    /// Whether a left click opens the menu rather than raising Click.
    fn menu_on_click(tree: &Node) -> bool {
        tree.props.get("menuOnClick") == Some(&Value::Bool(true))
    }

    /// Shows tray icon `tray` of `tree`.
    pub(super) fn show(tray: WindowId, tree: &Node) -> Result<Icon, String> {
        listen();
        let mut built = TrayIconBuilder::new().with_id(tray.to_string()).with_icon(picture(tree)?).with_menu(Box::new(menu_of(tray, tree)?)).with_menu_on_left_click(menu_on_click(tree));
        if let Some(t) = tree.props.get("tooltip").and_then(Value::as_str) {
            built = built.with_tooltip(t);
        }
        built.build().map(Icon).map_err(|e| format!("the tray icon could not be shown: {e}"))
    }

    /// Shows `tree` as tray icon `tray` is now: its picture, its tooltip,
    /// its menu, and whether a left click opens the menu.
    pub(super) fn update(icon: &Icon, tray: WindowId, tree: &Node) -> Result<(), String> {
        icon.0.set_icon(Some(picture(tree)?)).map_err(|e| format!("the tray icon's picture could not be changed: {e}"))?;
        let tooltip = tree.props.get("tooltip").and_then(Value::as_str);
        icon.0.set_tooltip(tooltip).map_err(|e| format!("the tray icon's tooltip could not be changed: {e}"))?;
        icon.0.set_menu(Some(Box::new(menu_of(tray, tree)?)));
        icon.0.set_show_menu_on_left_click(menu_on_click(tree));
        Ok(())
    }

    /// Windows and macOS always show tray icons, so there is nothing to say
    /// of where one shows.
    #[cfg(any(windows, target_os = "macos"))]
    pub(super) fn where_shown() -> Option<String> {
        None
    }

    /// Whether a tray shows StatusNotifierItem icons on this desktop, said
    /// once for the renderer's life: none when the watcher on the session
    /// bus has a host registered, else why the icon does not show yet.
    /// The icon shows once a host starts, as GNOME's AppIndicator
    /// extension or a panel does.
    #[cfg(not(any(windows, target_os = "macos")))]
    pub(super) fn where_shown() -> Option<String> {
        static SAID: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
        let missing = host_missing()?;
        if SAID.swap(true, std::sync::atomic::Ordering::Relaxed) {
            return None;
        }
        Some(format!("{missing}, so the tray icon shows once one starts, as GNOME's AppIndicator extension or a desktop's panel does"))
    }

    /// Why no tray shows StatusNotifierItem icons now; none when one does.
    #[cfg(not(any(windows, target_os = "macos")))]
    fn host_missing() -> Option<String> {
        let bus = match zbus::blocking::Connection::session() {
            Ok(b) => b,
            Err(e) => return Some(format!("the session bus, which tray icons show through here, cannot be reached ({e})")),
        };
        let watcher = zbus::blocking::Proxy::new(&bus, "org.kde.StatusNotifierWatcher", "/StatusNotifierWatcher", "org.kde.StatusNotifierWatcher");
        match watcher.and_then(|w| w.get_property::<bool>("IsStatusNotifierHostRegistered")) {
            Ok(true) => None,
            Ok(false) => Some(String::from("no tray on this desktop shows StatusNotifierItem icons")),
            Err(e) => Some(format!("no StatusNotifierItem watcher answers on the session bus ({e})")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tray_tree() -> Node {
        Node::new(1, Kind::TrayIcon)
            .prop("tooltip", Value::Str("Backups".into()))
            .child(Node::new(2, Kind::MenuItem).named("Open").prop("text", Value::Str("Open".into())))
            .child(Node::new(3, Kind::Separator))
            .child(Node::new(4, Kind::Menu).prop("title", Value::Str("More".into())).child(Node::new(5, Kind::MenuItem).named("Quit").prop("text", Value::Str("Quit".into())).prop("enabled", Value::Bool(false))))
    }

    #[test]
    fn a_menu_item_id_names_its_tray_icon_and_node() {
        assert_eq!(item_of(&item_id(12, 5)).expect("an item"), (12, 5));
        assert!(item_of("12").expect_err("no colon").contains("colon"));
        assert!(item_of("x:5").expect_err("no tray icon").contains("tray icon"));
    }

    #[test]
    fn a_tray_icon_menu_holds_items_separators_and_menus_alone() {
        check(&tray_tree()).expect("a tray icon's menu");
        let stray = Node::new(1, Kind::TrayIcon).child(Node::new(2, Kind::Button));
        assert!(check(&stray).expect_err("a button").contains("Button"));
        assert!(check(&Node::new(1, Kind::Column)).is_err());
        let untitled = Node::new(1, Kind::TrayIcon).child(Node::new(2, Kind::MenuItem));
        assert!(check(&untitled).expect_err("no text").contains("no text"));
    }

    #[test]
    fn a_menu_items_ampersand_shows_as_written() {
        let item = Node::new(2, Kind::MenuItem).prop("text", Value::Str("Save & quit".into()));
        assert_eq!(shown_text(&item).expect("its text"), "Save && quit");
    }

    #[test]
    fn a_tray_icons_tree_changes_as_a_windows_does() {
        let mut tree = tray_tree();
        apply(&mut tree, PatchOp::SetProps { node: 2, props: BTreeMap::from([(String::from("text"), Value::Str("Show".into()))]) }).expect("a property set");
        assert_eq!(tree.find(2).and_then(|n| n.props.get("text")), Some(&Value::Str("Show".into())));
        apply(&mut tree, PatchOp::Remove { node: 3 }).expect("a separator taken away");
        assert!(tree.find(3).is_none());
        assert!(apply(&mut tree, PatchOp::Remove { node: 9 }).is_err());
    }

    #[test]
    fn what_a_person_does_reaches_only_the_tray_icons_still_shown() {
        HEARD.push(Heard::Clicked { tray: 41, double: false });
        let trays = Trays::default();
        assert!(trays.heard().is_empty(), "no tray icon 41 is shown, so its click goes nowhere");
    }

    #[test]
    fn a_picture_larger_than_an_icon_is_shrunk_to_fit() {
        let mut png = Vec::new();
        image::DynamicImage::new_rgba8(600, 300).write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).expect("a PNG");
        let picture = proto::picture::value(png).expect("a picture");
        let (rgba, w, h) = icon_pixels(&picture).expect("its pixels");
        assert_eq!((w, h), (256, 128));
        assert_eq!(rgba.len(), 256 * 128 * 4);
    }
}
