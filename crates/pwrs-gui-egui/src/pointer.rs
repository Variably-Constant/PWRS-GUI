//! A GpuSurface's pointer events: a press over it and that button's
//! release wherever it happens, the pointer's moves over it or while a
//! button pressed over it is still down, once a frame at most, and the
//! wheel turned over it. Each gives where, in physical pixels from the
//! surface's top left and as fractions of its width and height, the
//! display's pixels per point, the button, and the modifier keys pressed.

use crate::{camera, egui, event, flag};
use pwrs_gui_proto::{EventKind, FromHelper, Node, NodeId, Value, WindowId};
use std::collections::BTreeMap;

/// The three buttons a pointer event names, as it names them.
const BUTTONS: [(egui::PointerButton, &str); 3] = [(egui::PointerButton::Primary, "Left"), (egui::PointerButton::Secondary, "Right"), (egui::PointerButton::Middle, "Middle")];

/// Which of its pointer events a surface's node turns on.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Wanted {
    pub down: bool,
    pub up: bool,
    pub moves: bool,
    pub wheel: bool,
}

impl Wanted {
    /// What `node`'s `pointerDown`, `pointerUp`, `pointerMove` and `wheel`
    /// turn on.
    pub(crate) fn of(node: &Node) -> Result<Wanted, String> {
        Ok(Wanted { down: flag(node, "pointerDown", false)?, up: flag(node, "pointerUp", false)?, moves: flag(node, "pointerMove", false)?, wheel: flag(node, "wheel", false)? })
    }

    pub(crate) fn any(self) -> bool {
        self.down || self.up || self.moves || self.wheel
    }

    /// Whether events of `kind` are turned on.
    pub(crate) fn has(self, kind: EventKind) -> bool {
        match kind {
            EventKind::PointerDown => self.down,
            EventKind::PointerUp => self.up,
            EventKind::PointerMove => self.moves,
            EventKind::Wheel => self.wheel,
            _ => false,
        }
    }
}

/// What a surface's pointer events store between frames: the buttons
/// pressed over it, whose release is reported wherever it happens, and
/// where the last move reported was.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Tracked {
    pressed: [bool; 3],
    last: Option<egui::Pos2>,
}

/// Where a surface was drawn: its top left and its size in physical
/// pixels, and the display's pixels per point.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Place {
    pub left: f32,
    pub top: f32,
    pub across: f32,
    pub down: f32,
    pub ppp: f32,
}

impl Place {
    /// `at`, in points, as physical pixels from the surface's top left.
    pub(crate) fn pixels(&self, at: egui::Pos2) -> (f32, f32) {
        (at.x * self.ppp - self.left, at.y * self.ppp - self.top)
    }

    /// The map an event gives for a point `x` and `y` physical pixels
    /// from the surface's top left, `u` and `v` of its width and height,
    /// with `button` and `keys`.
    fn value(&self, (x, y): (f32, f32), (u, v): (f64, f64), button: Option<&str>, keys: egui::Modifiers) -> BTreeMap<String, Value> {
        let mut out = BTreeMap::new();
        out.insert(String::from("x"), Value::Float(f64::from(x)));
        out.insert(String::from("y"), Value::Float(f64::from(y)));
        out.insert(String::from("u"), Value::Float(u));
        out.insert(String::from("v"), Value::Float(v));
        out.insert(String::from("ppp"), Value::Float(f64::from(self.ppp)));
        out.insert(String::from("button"), button.map_or(Value::Null, |b| Value::Str(b.to_string())));
        out.insert(String::from("shift"), Value::Bool(keys.shift));
        out.insert(String::from("ctrl"), Value::Bool(keys.ctrl || keys.mac_cmd));
        out.insert(String::from("alt"), Value::Bool(keys.alt));
        out
    }

    /// The map an event at `at`, in points, gives.
    pub(crate) fn value_at(&self, at: egui::Pos2, button: Option<&str>, keys: egui::Modifiers) -> BTreeMap<String, Value> {
        let (x, y) = self.pixels(at);
        self.value((x, y), (f64::from(x / self.across), f64::from(y / self.down)), button, keys)
    }
}

/// A surface as a frame drew it: its rect in points, its place in
/// physical pixels, and whether the pointer is over it with nothing on top
/// of it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Drawn {
    pub rect: egui::Rect,
    pub place: Place,
    pub over: bool,
}

impl Tracked {
    /// Raises the pointer events `wanted` turns on of surface `node` of
    /// window `window`, drawn as `drawn` says, from this frame's input. A
    /// press over it is tracked whether or not its event is on, so its
    /// release is reported wherever the pointer is.
    pub(crate) fn report(&mut self, ctx: &egui::Context, drawn: &Drawn, wanted: Wanted, (window, node): (WindowId, NodeId), out: &mut Vec<FromHelper>) {
        let (events, latest, keys) = ctx.input(|i| (i.events.clone(), i.pointer.latest_pos(), i.modifiers));
        let place = drawn.place;
        for e in &events {
            match e {
                egui::Event::PointerButton { pos, button, pressed, modifiers } => {
                    let Some(slot) = BUTTONS.iter().position(|(b, _)| b == button) else {
                        continue;
                    };
                    let name = BUTTONS[slot].1;
                    if *pressed && drawn.over && drawn.rect.contains(*pos) {
                        self.pressed[slot] = true;
                        if wanted.down {
                            event(out, window, node, EventKind::PointerDown, Value::Map(place.value_at(*pos, Some(name), *modifiers)));
                        }
                    } else if !*pressed && self.pressed[slot] {
                        self.pressed[slot] = false;
                        if wanted.up {
                            event(out, window, node, EventKind::PointerUp, Value::Map(place.value_at(*pos, Some(name), *modifiers)));
                        }
                    }
                }
                egui::Event::MouseWheel { unit, delta, modifiers, .. } if wanted.wheel && drawn.over => {
                    if let Some(at) = latest {
                        let mut value = place.value_at(at, None, *modifiers);
                        value.insert(String::from("notches"), Value::Float(camera::notches(*unit, delta.y)));
                        event(out, window, node, EventKind::Wheel, Value::Map(value));
                    }
                }
                _ => {}
            }
        }
        let held = BUTTONS.iter().zip(self.pressed).find(|(_, down)| *down).map(|((_, name), _)| *name);
        if wanted.moves
            && let Some(at) = latest
            && (drawn.over || held.is_some())
            && self.last != Some(at)
        {
            self.last = Some(at);
            event(out, window, node, EventKind::PointerMove, Value::Map(place.value_at(at, held, keys)));
        }
    }
}

/// The value a test event of `kind` raises on a surface drawn at `place`.
/// `given` is a map, its keys compared without regard to case, of `u` and
/// `v`, fractions of the surface's width and height, with any of `button`
/// (`Left` when a press or release names none), `shift`, `ctrl` and `alt`,
/// and for a wheel turn its `notches`, 1 when absent.
pub(crate) fn test_value(place: Place, kind: EventKind, given: &Value) -> Result<Value, String> {
    let map = given.as_map().ok_or_else(|| format!("a pointer test event takes a map of u and v, fractions of the surface's width and height, not {given:?}"))?;
    let get = |key: &str| map.iter().find(|(k, _)| k.eq_ignore_ascii_case(key)).map(|(_, v)| v);
    let fraction = |key: &str| get(key).and_then(Value::as_number).ok_or_else(|| format!("a pointer test event takes {key}, a fraction of the surface"));
    let held = |key: &str| match get(key) {
        None => Ok(false),
        Some(Value::Bool(on)) => Ok(*on),
        Some(other) => Err(format!("{key} is true or false, not {other:?}")),
    };
    let (u, v) = (fraction("u")?, fraction("v")?);
    let keys = egui::Modifiers { shift: held("shift")?, ctrl: held("ctrl")?, alt: held("alt")?, ..egui::Modifiers::NONE };
    let button = match (kind, get("button")) {
        (EventKind::Wheel, _) | (EventKind::PointerMove, None | Some(Value::Null)) => None,
        (_, None | Some(Value::Null)) => Some("Left"),
        (_, Some(Value::Str(named))) => {
            let found = BUTTONS.iter().find(|(_, name)| name.eq_ignore_ascii_case(named));
            Some(found.map(|(_, name)| *name).ok_or_else(|| format!("{named} is not a pointer button: Left, Right or Middle"))?)
        }
        (_, Some(other)) => return Err(format!("button is Left, Right or Middle, not {other:?}")),
    };
    let (x, y) = ((u * f64::from(place.across)) as f32, (v * f64::from(place.down)) as f32);
    let mut value = place.value((x, y), (u, v), button, keys);
    if kind == EventKind::Wheel {
        let notches = match get("notches") {
            None => 1.0,
            Some(n) => n.as_number().ok_or_else(|| format!("notches is a number, not {n:?}"))?,
        };
        value.insert(String::from("notches"), Value::Float(notches));
    }
    Ok(Value::Map(value))
}

#[cfg(test)]
mod tests {
    use super::*;

    const PLACE: Place = Place { left: 10.0, top: 20.0, across: 200.0, down: 100.0, ppp: 2.0 };

    /// Runs one frame of a surface drawn at [`PLACE`], 100 by 50 points
    /// from (5, 10) points, given `events`, and returns the events raised.
    fn frame(ctx: &egui::Context, events: Vec<egui::Event>, wanted: Wanted, tracked: &mut Tracked) -> Vec<FromHelper> {
        let rect = egui::Rect::from_min_size(egui::pos2(5.0, 10.0), egui::vec2(100.0, 50.0));
        let input = egui::RawInput { events, screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(400.0, 300.0))), ..egui::RawInput::default() };
        let mut out = Vec::new();
        ctx.run_ui(input, |ui| {
            let response = ui.allocate_rect(rect, egui::Sense::hover());
            let drawn = Drawn { rect, place: PLACE, over: response.contains_pointer() };
            tracked.report(ui.ctx(), &drawn, wanted, (7, 3), &mut out);
        })
        .drop_without_applying_deltas();
        out
    }

    /// The kind and the value of each event in `out`.
    fn raised(out: &[FromHelper]) -> Vec<(EventKind, BTreeMap<String, Value>)> {
        out.iter()
            .filter_map(|m| match m {
                FromHelper::Event { kind, value: Value::Map(v), .. } => Some((*kind, v.clone())),
                _ => None,
            })
            .collect()
    }

    fn number(value: &BTreeMap<String, Value>, key: &str) -> f64 {
        value.get(key).and_then(Value::as_number).expect("a number")
    }

    #[test]
    fn a_press_reports_where_in_pixels_fractions_and_scale_and_its_release_wherever_it_happens() {
        let ctx = egui::Context::default();
        let all = Wanted { down: true, up: true, moves: false, wheel: false };
        let mut tracked = Tracked::default();
        let at = egui::pos2(30.0, 35.0);
        let press = egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed: true, modifiers: egui::Modifiers::SHIFT };
        assert!(frame(&ctx, vec![egui::Event::PointerMoved(at)], all, &mut tracked).is_empty());
        let down = raised(&frame(&ctx, vec![press], all, &mut tracked));
        assert_eq!(down.len(), 1);
        let (kind, value) = &down[0];
        assert_eq!(*kind, EventKind::PointerDown);
        assert_eq!((number(value, "x"), number(value, "y")), (50.0, 50.0), "30 points across at 2 pixels a point, less the surface's 10 pixel left");
        assert_eq!((number(value, "u"), number(value, "v")), (0.25, 0.5));
        assert_eq!(number(value, "ppp"), 2.0);
        assert_eq!(value.get("button"), Some(&Value::Str(String::from("Left"))));
        assert_eq!(value.get("shift"), Some(&Value::Bool(true)));
        let away = egui::pos2(300.0, 200.0);
        let release = egui::Event::PointerButton { pos: away, button: egui::PointerButton::Primary, pressed: false, modifiers: egui::Modifiers::NONE };
        let up = raised(&frame(&ctx, vec![egui::Event::PointerMoved(away), release], all, &mut tracked));
        assert_eq!(up.iter().map(|(k, _)| *k).collect::<Vec<_>>(), [EventKind::PointerUp], "released outside the surface, still reported");
        assert!(number(&up[0].1, "u") > 1.0);
    }

    #[test]
    fn moves_are_reported_once_a_frame_over_the_surface_or_while_a_button_pressed_on_it_is_held() {
        let ctx = egui::Context::default();
        let moves = Wanted { moves: true, ..Wanted::default() };
        let mut tracked = Tracked::default();
        let moved = |x, y| egui::Event::PointerMoved(egui::pos2(x, y));
        assert!(frame(&ctx, vec![moved(300.0, 200.0)], moves, &mut tracked).is_empty(), "outside, with no button down");
        let inside = raised(&frame(&ctx, vec![moved(20.0, 20.0), moved(25.0, 20.0)], moves, &mut tracked));
        assert_eq!(inside.len(), 1, "two moves in one frame make one event");
        assert_eq!(number(&inside[0].1, "x"), 40.0, "the latest place");
        assert_eq!(inside[0].1.get("button"), Some(&Value::Null));
        assert!(frame(&ctx, Vec::new(), moves, &mut tracked).is_empty(), "no move, no event");
        let press = egui::Event::PointerButton { pos: egui::pos2(25.0, 20.0), button: egui::PointerButton::Middle, pressed: true, modifiers: egui::Modifiers::NONE };
        assert!(raised(&frame(&ctx, vec![press], moves, &mut tracked)).is_empty(), "a press whose event is off raises nothing");
        let dragged = raised(&frame(&ctx, vec![moved(300.0, 200.0)], moves, &mut tracked));
        assert_eq!(dragged.len(), 1, "outside, but the middle button pressed on the surface is still down");
        assert_eq!(dragged[0].1.get("button"), Some(&Value::Str(String::from("Middle"))));
    }

    #[test]
    fn a_wheel_turn_over_the_surface_reports_its_notches() {
        let ctx = egui::Context::default();
        let wheel = Wanted { wheel: true, ..Wanted::default() };
        let mut tracked = Tracked::default();
        let turn = egui::Event::MouseWheel { unit: egui::MouseWheelUnit::Line, delta: egui::vec2(0.0, -2.0), phase: egui::TouchPhase::Move, modifiers: egui::Modifiers::CTRL, source: egui::MouseWheelSource::Wheel };
        assert!(frame(&ctx, vec![egui::Event::PointerMoved(egui::pos2(300.0, 200.0)), turn.clone()], wheel, &mut tracked).is_empty());
        let over = raised(&frame(&ctx, vec![egui::Event::PointerMoved(egui::pos2(30.0, 35.0)), turn], wheel, &mut tracked));
        assert_eq!(over.len(), 1);
        assert_eq!(number(&over[0].1, "notches"), -2.0, "two lines toward the person");
        assert_eq!(over[0].1.get("ctrl"), Some(&Value::Bool(true)));
        assert_eq!(over[0].1.get("button"), Some(&Value::Null));
    }

    #[test]
    fn a_test_event_takes_fractions_and_fills_in_the_pixels() {
        let given = Value::Map(BTreeMap::from([(String::from("U"), Value::Float(0.25)), (String::from("v"), Value::Float(0.5)), (String::from("Button"), Value::Str(String::from("right")))]));
        let Value::Map(value) = test_value(PLACE, EventKind::PointerDown, &given).expect("a press") else {
            panic!("a map");
        };
        assert_eq!((number(&value, "x"), number(&value, "y"), number(&value, "u")), (50.0, 50.0, 0.25));
        assert_eq!(value.get("button"), Some(&Value::Str(String::from("Right"))));
        let Value::Map(turned) = test_value(PLACE, EventKind::Wheel, &Value::Map(BTreeMap::from([(String::from("u"), Value::Float(0.5)), (String::from("v"), Value::Float(0.5))]))).expect("a turn") else {
            panic!("a map");
        };
        assert_eq!((number(&turned, "notches"), turned.get("button")), (1.0, Some(&Value::Null)));
        assert!(test_value(PLACE, EventKind::PointerDown, &Value::Map(BTreeMap::from([(String::from("u"), Value::Float(0.5))]))).is_err_and(|e| e.contains(" v,")));
        assert!(test_value(PLACE, EventKind::PointerUp, &given).is_ok());
    }
}
