//! Input the window layer gives a window as a whole, which the renderer
//! routes to one of its widgets. Files dropped go to the innermost widget
//! under the pointer whose `drop` is true, or else to the window's root
//! when its `drop` is true. A key pressed goes to the widget with the
//! focus when its `keys` is true and it does not use the key itself
//! ([`Shortcut::widget_keeps`]), and Enter and Escape to the window's
//! default and cancel buttons where that widget does not use them.
//!
//! winit gives a drop without where it landed, and no pointer moves
//! reach a window while something is dragged over it, so at a drop the
//! pointer's place is read from the system: from Windows, macOS and X11.
//! Where the system cannot say, the last place egui recorded for the
//! pointer stands in. winit
//! gives a Wayland window no drops, and no window dropped text.

use super::{clipboard_key, egui, event};
use pwrs_gui_proto::button_bar::{self, Role};
use pwrs_gui_proto::{EventKind, FromHelper, Kind, Node, NodeId, Shortcut, Value, WindowId};
use std::collections::HashMap;

/// Raises Drop with the paths of the files dropped on the window this
/// frame, on the widget they were dropped on.
pub(super) fn route_drops(ctx: &egui::Context, tree: &Node, rects: &HashMap<NodeId, egui::Rect>, window: WindowId, out: &mut Vec<FromHelper>) {
    let dropped: Vec<Value> = ctx.input(|i| i.raw.dropped_files.iter().map(|f| Value::Str(f.path().display().to_string())).collect());
    if dropped.is_empty() {
        return;
    }
    let at = match pointer(ctx) {
        Some(at) => Some(at),
        None => ctx.input(|i| i.pointer.latest_pos()),
    };
    match drop_target(tree, rects, at) {
        Some(node) => event(out, window, node, EventKind::Drop, Value::List(dropped)),
        None => pwrs_gui_proto::trace::line(|| format!("egui: {} file(s) dropped on window {window} at {at:?}, where no widget takes them", dropped.len())),
    }
}

/// The widget a drop at `at` lands on: the innermost one whose `drop` is
/// true and whose place in the last frame contains `at`, or else the root when
/// its `drop` is true, wherever the drop landed.
pub(super) fn drop_target(tree: &Node, rects: &HashMap<NodeId, egui::Rect>, at: Option<egui::Pos2>) -> Option<NodeId> {
    let mut best: Option<(NodeId, usize)> = None;
    if let Some(at) = at {
        innermost(tree, 0, &|n| takes(n, "drop") && rects.get(&n.id).is_some_and(|r| r.contains(at)), &mut best);
    }
    match best {
        Some((id, _)) => Some(id),
        None => takes(tree, "drop").then_some(tree.id),
    }
}

/// Raises Key for each key pressed this frame, once a press, that the
/// widget with the focus does not use itself, when its `keys` is true:
/// the presses the window's shortcuts left, and egui's clipboard events as
/// the keys they stand for. A widget uses a key itself as
/// [`Shortcut::node_keeps`] says, or as `kept` says of its own state, as
/// a canvas's view keeps the keys its controls bind. `focused` is where the
/// focused widget was drawn; a key a shortcut cannot name raises nothing.
pub(super) fn route_keys(ctx: &egui::Context, tree: &Node, rects: &HashMap<NodeId, egui::Rect>, focused: Option<egui::Rect>, kept: &dyn Fn(&Node, &Shortcut) -> bool, window: WindowId, out: &mut Vec<FromHelper>) {
    let Some(focused) = focused else {
        return;
    };
    let mut best: Option<(NodeId, usize)> = None;
    innermost(tree, 0, &|n| takes(n, "keys") && rects.get(&n.id).is_some_and(|r| r.contains(focused.center())), &mut best);
    let Some(node) = best.and_then(|(id, _)| tree.find(id)) else {
        return;
    };
    let typed = ctx.text_edit_focused() || node.kind == Kind::NumberBox;
    let pressed: Vec<Shortcut> = ctx.input(|i| i.events.iter().filter_map(|e| pressed(e, i.modifiers)).collect());
    for s in pressed {
        if !s.node_keeps(node, typed) && !kept(node, &s) {
            event(out, window, node.id, EventKind::Key, Value::Str(s.written()));
        }
    }
}

/// Presses the window's default button for an Enter, and its cancel button
/// for an Escape, pressed this frame without a modifier, raising the
/// button's Click, unless the widget with the focus uses the key itself
/// ([`button_bar::keeps`]) or a popup was open as the frame began, which the
/// key closes. A disabled button is not pressed. `focused` is where the
/// focused widget was drawn, in the frame `rects` describes.
pub(super) fn route_roles(ctx: &egui::Context, tree: &Node, rects: &HashMap<NodeId, egui::Rect>, focused: Option<egui::Rect>, popup_open: bool, window: WindowId, out: &mut Vec<FromHelper>) {
    if popup_open {
        return;
    }
    let pressed = |key: egui::Key| ctx.input(|i| i.events.iter().any(|e| matches!(e, egui::Event::Key { key: k, pressed: true, repeat: false, modifiers, .. } if *k == key && modifiers.is_none())));
    let holder = focused.and_then(|f| {
        let mut best: Option<(NodeId, usize)> = None;
        innermost(tree, 0, &|n| rects.get(&n.id).is_some_and(|r| r.contains(f.center())), &mut best);
        best.and_then(|(id, _)| tree.find(id))
    });
    for (key, role, name) in [(egui::Key::Enter, Role::Default, "Enter"), (egui::Key::Escape, Role::Cancel, "Escape")] {
        if !pressed(key) {
            continue;
        }
        let Some(button) = button_bar::role_button(tree, role) else {
            continue;
        };
        if matches!(button.props.get("enabled"), Some(Value::Bool(false))) || holder.is_some_and(|n| button_bar::keeps(n, name)) {
            continue;
        }
        event(out, window, button.id, EventKind::Click, Value::Null);
    }
}

/// The widget with the focus: the innermost one that takes the focus
/// ([`Node::takes_focus`]) whose place in `rects` contains the middle of
/// `focused`, where the focused widget was drawn; none while nothing has
/// the focus.
pub(super) fn focus_holder(tree: &Node, rects: &HashMap<NodeId, egui::Rect>, focused: Option<egui::Rect>) -> Option<NodeId> {
    let focused = focused?;
    let mut best: Option<(NodeId, usize)> = None;
    innermost(tree, 0, &|n| n.takes_focus() && rects.get(&n.id).is_some_and(|r| r.contains(focused.center())), &mut best);
    best.map(|(id, _)| id)
}

/// The key a frame's event presses, as a shortcut writes it: a key's first
/// press, not its repeats, or a clipboard event standing for one. A key
/// whose character a shortcut has no name for, as Shift and 1 types ! on
/// many keyboards, is the key at its place, as [`named`] says. Ctrl is the
/// Command key on macOS. A key a shortcut cannot name is traced.
fn pressed(e: &egui::Event, held: egui::Modifiers) -> Option<Shortcut> {
    let (key, place, modifiers) = match e {
        egui::Event::Key { key, physical_key, pressed: true, repeat: false, modifiers } => (*key, *physical_key, *modifiers),
        other => (clipboard_key(other, held)?, None, held),
    };
    let mut s = match named(key, place) {
        Ok(s) => s,
        Err(why) => {
            pwrs_gui_proto::trace::line(|| format!("egui: the key {} raises no Key: {why}", key.name()));
            return None;
        }
    };
    s.ctrl = modifiers.command;
    s.shift = modifiers.shift;
    s.alt = modifiers.alt;
    Some(s)
}

/// The shortcut key a press of `key`, at the place of `place`, names, with
/// no modifiers: `key` where a shortcut names it, or else `place`, so a
/// digit or letter typed as a symbol is the key at its place; otherwise
/// why a shortcut names neither.
pub(super) fn named(key: egui::Key, place: Option<egui::Key>) -> Result<Shortcut, String> {
    let by_key = match Shortcut::parse(shortcut_name(key)) {
        Ok(s) => return Ok(s),
        Err(why) => why,
    };
    match place {
        Some(p) => Shortcut::parse(shortcut_name(p)).map_err(|at_place| format!("{by_key}; and at its place, {at_place}")),
        None => Err(by_key),
    }
}

/// The key a press of `key`, at the place of `place`, counts as for a
/// shortcut, as [`named`] chooses: `key` where a shortcut names it, and
/// otherwise the key at its place.
pub(super) fn counted(key: egui::Key, place: Option<egui::Key>) -> egui::Key {
    match place {
        Some(p) if Shortcut::parse(shortcut_name(key)).is_err() => p,
        _ => key,
    }
}

/// `key`'s name as a shortcut writes it: egui names the arrows Left,
/// Right, Up and Down, and every other key a shortcut names as a shortcut
/// does.
pub(super) fn shortcut_name(key: egui::Key) -> &'static str {
    match key {
        egui::Key::ArrowLeft => "ArrowLeft",
        egui::Key::ArrowRight => "ArrowRight",
        egui::Key::ArrowUp => "ArrowUp",
        egui::Key::ArrowDown => "ArrowDown",
        other => other.name(),
    }
}

/// Makes the widget at `response`, which takes keys, take the focus when it
/// is pressed and, while it has the focus, keep the arrows and Escape,
/// which egui would otherwise move the focus with or take it away for.
/// Returns whether a ring shows the focus: while it has a focus the
/// keyboard gave it, as a browser shows such an element. `from_press`
/// records whether a press gave it the focus it has.
pub(super) fn focus_keys(ui: &egui::Ui, response: &egui::Response, from_press: &mut bool) -> bool {
    if response.is_pointer_button_down_on() {
        response.request_focus();
        *from_press = true;
    }
    if response.lost_focus() {
        *from_press = false;
    }
    if !response.has_focus() {
        return false;
    }
    ui.memory_mut(|m| m.set_focus_lock_filter(response.id, egui::EventFilter { tab: false, horizontal_arrows: true, vertical_arrows: true, escape: true }));
    super::stay_focused_on_arrows(ui);
    !*from_press
}

/// The ring about `rect` that shows a widget has the focus, in egui's
/// selection stroke.
pub(super) fn focus_ring(ui: &egui::Ui, rect: egui::Rect) {
    ui.painter().rect_stroke(rect, 0.0, ui.visuals().selection.stroke, egui::StrokeKind::Inside);
}

/// Whether the node's flag `key` is true.
fn takes(node: &Node, key: &str) -> bool {
    matches!(node.props.get(key), Some(Value::Bool(true)))
}

/// Stores in `best` the deepest node of `node`'s subtree that `fits`, with
/// its depth; the first of two at one depth.
fn innermost(node: &Node, depth: usize, fits: &dyn Fn(&Node) -> bool, best: &mut Option<(NodeId, usize)>) {
    if fits(node) && best.is_none_or(|(_, d)| depth > d) {
        *best = Some((node.id, depth));
    }
    for c in &node.children {
        innermost(c, depth + 1, fits, best);
    }
}

/// Where the pointer is over the inside of the window `ctx` draws, in its
/// points, as the system says; none where it cannot say.
fn pointer(ctx: &egui::Context) -> Option<egui::Pos2> {
    let (inner, native) = ctx.input(|i| (i.viewport().inner_rect, i.viewport().native_pixels_per_point));
    let (inner, native) = (inner?, native?);
    let screen = cursor(native)?;
    let ppp = ctx.pixels_per_point();
    Some(egui::pos2(screen.x / ppp - inner.min.x, screen.y / ppp - inner.min.y))
}

/// The pointer's place on the desktop in physical pixels, as winit places
/// windows.
#[cfg(windows)]
fn cursor(_native: f32) -> Option<egui::Pos2> {
    use windows_sys::Win32::Foundation::POINT;
    use windows_sys::Win32::UI::WindowsAndMessaging::GetCursorPos;
    let mut at = POINT { x: 0, y: 0 };
    let found = unsafe { GetCursorPos(&mut at) } != 0;
    found.then(|| egui::pos2(at.x as f32, at.y as f32))
}

/// The pointer's place on the desktop in physical pixels, as winit places
/// windows: AppKit counts points up from the bottom of the screen with the
/// menu bar, and winit counts down from its top, `native` pixels a point.
#[cfg(target_os = "macos")]
fn cursor(native: f32) -> Option<egui::Pos2> {
    let mtm = objc2::MainThreadMarker::new()?;
    let primary = objc2_app_kit::NSScreen::screens(mtm).firstObject()?;
    let height = primary.frame().size.height;
    let at = objc2_app_kit::NSEvent::mouseLocation();
    let scale = f64::from(native);
    Some(egui::pos2((at.x * scale) as f32, ((height - at.y) * scale) as f32))
}

/// The pointer's place on the X server's root window, in pixels, read
/// over a connection of its own.
#[cfg(all(unix, not(target_vendor = "apple")))]
fn cursor(_native: f32) -> Option<egui::Pos2> {
    let xlib = match x11_dl::xlib::Xlib::open() {
        Ok(x) => x,
        Err(e) => {
            pwrs_gui_proto::trace::line(|| format!("egui: libX11 could not be loaded to find the pointer: {e}"));
            return None;
        }
    };
    let display = unsafe { (xlib.XOpenDisplay)(std::ptr::null()) };
    if display.is_null() {
        pwrs_gui_proto::trace::line(|| String::from("egui: no X display could be opened to find the pointer"));
        return None;
    }
    let (mut root, mut child, mut mask) = (0, 0, 0);
    let (mut x, mut y, mut window_x, mut window_y) = (0, 0, 0, 0);
    let found = unsafe {
        let screen_root = (xlib.XDefaultRootWindow)(display);
        let found = (xlib.XQueryPointer)(display, screen_root, &mut root, &mut child, &mut x, &mut y, &mut window_x, &mut window_y, &mut mask);
        (xlib.XCloseDisplay)(display);
        found
    };
    (found != 0).then(|| egui::pos2(x as f32, y as f32))
}

/// No system to read the pointer's place from.
#[cfg(not(any(windows, target_os = "macos", all(unix, not(target_vendor = "apple")))))]
fn cursor(_native: f32) -> Option<egui::Pos2> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect(x0: f32, y0: f32, x1: f32, y1: f32) -> egui::Rect {
        egui::Rect::from_min_max(egui::pos2(x0, y0), egui::pos2(x1, y1))
    }

    #[test]
    fn a_drop_lands_on_the_innermost_widget_taking_it_or_else_the_root() {
        let tree = Node::new(1, Kind::Column)
            .prop("drop", Value::Bool(true))
            .child(Node::new(2, Kind::Group).prop("drop", Value::Bool(true)).child(Node::new(3, Kind::ListBox).prop("drop", Value::Bool(true))).child(Node::new(4, Kind::Text)))
            .child(Node::new(5, Kind::Image));
        let rects = HashMap::from([(1, rect(0.0, 0.0, 100.0, 100.0)), (2, rect(0.0, 0.0, 50.0, 50.0)), (3, rect(0.0, 0.0, 20.0, 20.0)), (4, rect(20.0, 20.0, 50.0, 50.0)), (5, rect(50.0, 50.0, 100.0, 100.0))]);
        assert_eq!(drop_target(&tree, &rects, Some(egui::pos2(10.0, 10.0))), Some(3));
        assert_eq!(drop_target(&tree, &rects, Some(egui::pos2(30.0, 30.0))), Some(2), "a Text takes no drop, so its Group does");
        assert_eq!(drop_target(&tree, &rects, Some(egui::pos2(70.0, 70.0))), Some(1));
        assert_eq!(drop_target(&tree, &rects, Some(egui::pos2(500.0, 500.0))), Some(1), "the root takes a drop anywhere in the window");
        assert_eq!(drop_target(&tree, &rects, None), Some(1));
        let no_root = Node::new(1, Kind::Column).child(Node::new(2, Kind::ListBox).prop("drop", Value::Bool(true)));
        assert_eq!(drop_target(&no_root, &rects, Some(egui::pos2(70.0, 70.0))), None);
    }

    #[test]
    fn the_focus_is_held_by_the_innermost_widget_taking_it_around_the_focused_place() {
        let tree = Node::new(1, Kind::Column).child(Node::new(2, Kind::Row).child(Node::new(3, Kind::Button)).child(Node::new(4, Kind::Text))).child(Node::new(5, Kind::Table));
        let rects = HashMap::from([(1, rect(0.0, 0.0, 100.0, 100.0)), (2, rect(0.0, 0.0, 100.0, 20.0)), (3, rect(0.0, 0.0, 40.0, 20.0)), (4, rect(40.0, 0.0, 100.0, 20.0)), (5, rect(0.0, 20.0, 100.0, 100.0))]);
        assert_eq!(focus_holder(&tree, &rects, Some(rect(2.0, 2.0, 38.0, 18.0))), Some(3));
        assert_eq!(focus_holder(&tree, &rects, Some(rect(10.0, 22.0, 90.0, 40.0))), Some(5), "the table's filter field is inside the table");
        assert_eq!(focus_holder(&tree, &rects, Some(rect(50.0, 2.0, 90.0, 18.0))), None, "a Text that is not a link takes no focus");
        assert_eq!(focus_holder(&tree, &rects, None), None);
    }

    #[test]
    fn a_press_is_written_as_a_shortcut_and_a_repeat_is_not() {
        let ctrl = egui::Modifiers { command: true, ctrl: true, ..egui::Modifiers::NONE };
        let press = |key, repeat, modifiers| egui::Event::Key { key, physical_key: None, pressed: true, repeat, modifiers };
        assert_eq!(pressed(&press(egui::Key::K, false, ctrl), ctrl).map(|s| s.written()), Some(String::from("Ctrl+K")));
        assert_eq!(pressed(&press(egui::Key::F5, false, egui::Modifiers::NONE), egui::Modifiers::NONE).map(|s| s.written()), Some(String::from("F5")));
        assert!(pressed(&press(egui::Key::K, true, ctrl), ctrl).is_none(), "a key's repeats raise no more Key events");
        assert_eq!(pressed(&egui::Event::Copy, ctrl).map(|s| s.written()), Some(String::from("Ctrl+C")));
        assert_eq!(pressed(&press(egui::Key::ArrowLeft, false, egui::Modifiers::NONE), egui::Modifiers::NONE).map(|s| s.written()), Some(String::from("ArrowLeft")), "egui's Left is a shortcut's ArrowLeft");
        let at = |key, place, modifiers| egui::Event::Key { key, physical_key: Some(place), pressed: true, repeat: false, modifiers };
        let shift = egui::Modifiers::SHIFT;
        assert_eq!(pressed(&at(egui::Key::Exclamationmark, egui::Key::Num1, shift), shift).map(|s| s.written()), Some(String::from("Shift+1")), "Shift and 1 typing ! is the 1 key");
        assert_eq!(pressed(&at(egui::Key::Plus, egui::Key::Equals, shift), shift).map(|s| s.written()), Some(String::from("Shift+Plus")), "+ typed with Shift is Plus");
        assert!(pressed(&at(egui::Key::Questionmark, egui::Key::Slash, shift), shift).is_none(), "a symbol at a symbol's place names no key");
    }
}
