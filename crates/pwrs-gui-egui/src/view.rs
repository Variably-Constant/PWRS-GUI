//! A canvas's view moved by the person, as its controls say: a press over
//! the canvas chooses the move its drag makes, by its button, its modifier
//! keys and the space bar, until the button is let go; each wheel turn, a
//! pinch, a double-click and, while the canvas has the focus, each key
//! press make the move they are bound to. Steps, glides and a script's
//! eased changes move the view over time, and a gesture or a change during
//! a move ends it where it is.

use crate::{egui, pointer, proto, routed};
use proto::camera::{CameraButton, CameraModifiers};
use proto::view::{self, ViewEase, ViewGesture, ViewKey, ViewMotion, ViewMove, ViewShown, ViewSpec};
use std::time::Instant;

/// A canvas's view and what is moving it.
#[derive(Default)]
pub(crate) struct ViewState {
    /// The view, none while the items' coordinates are the canvas's own;
    /// while a move is under way, where the move ends.
    pub(crate) view: Option<ViewSpec>,
    /// A step, a glide or an eased change under way.
    motion: Option<ViewMotion>,
    /// The drag a press over the canvas began, while its button is down.
    held: Option<Held>,
}

/// A drag the view takes: the button down, the move it makes, where it
/// began and where the pointer was at the last frame, and the pointer's
/// places, each at egui's time in seconds, which a glide measures.
struct Held {
    button: egui::PointerButton,
    made: ViewMove,
    from: egui::Pos2,
    last: egui::Pos2,
    places: Vec<(f64, [f64; 2])>,
}

impl ViewState {
    /// The view as drawn at `now` on a canvas `size` points across and
    /// down: where a move under way has it, or else the view.
    pub(crate) fn shown(&self, now: Instant, size: [f64; 2]) -> Option<ViewSpec> {
        match &self.motion {
            Some(m) if m.moving(now) => Some(m.at(now, size)),
            _ => self.view.clone(),
        }
    }

    /// Whether a move is under way at `now`, or the person is dragging the
    /// view.
    pub(crate) fn moving(&self, now: Instant) -> bool {
        self.motion.as_ref().is_some_and(|m| m.moving(now)) || self.held.is_some()
    }

    /// Ends a move under way where it is at `now`, so what comes next
    /// starts from the view drawn then.
    pub(crate) fn settle(&mut self, now: Instant, size: [f64; 2]) {
        if let Some(m) = self.motion.take() {
            self.view = Some(m.at(now, size));
        }
    }

    /// Moves the view to `to` from the view drawn at `now`, over `ease`; a
    /// canvas without a view is drawn under the default one, so its move
    /// starts there.
    pub(crate) fn ease_to(&mut self, to: ViewSpec, ease: ViewEase, now: Instant, size: [f64; 2]) {
        self.settle(now, size);
        let from = self.view.take().unwrap_or_default();
        self.motion = Some(ViewMotion { from, to: to.clone(), began: now, ease });
        self.view = Some(to);
    }

    /// Takes the view away, with any move and drag of it.
    pub(crate) fn remove(&mut self) {
        *self = ViewState::default();
    }

    /// The view as a call returns it at `now`: as drawn, its center the
    /// one its canvas's size `size` gives it once the canvas has a size,
    /// and whether it is moving.
    pub(crate) fn answer(&self, now: Instant, size: Option<[f64; 2]>) -> Option<ViewShown> {
        let mut shown = match size {
            Some(s) => self.shown(now, s)?,
            None => self.view.clone()?,
        };
        if let Some(s) = size {
            shown.center = Some(shown.center_in(s));
        }
        Some(ViewShown { view: shown, moving: self.moving(now) })
    }

    /// Whether the view's controls take the press `pressed` from the
    /// canvas's key handler.
    pub(crate) fn keeps(&self, pressed: &proto::Shortcut) -> bool {
        self.view.as_ref().is_some_and(|v| v.controls.keeps(pressed))
    }

    /// Whether the view's controls bind a gesture `of` picks out.
    fn binds(&self, of: impl Fn(&ViewGesture) -> bool) -> bool {
        self.view.as_ref().is_some_and(|v| v.controls.bindings().iter().any(|(g, _)| of(g)))
    }

    /// Whether the view's controls take a drag, so the canvas senses one.
    pub(crate) fn takes_drags(&self) -> bool {
        self.binds(|g| matches!(g, ViewGesture::Drag { .. }))
    }

    /// Whether the view's controls take a double-click.
    pub(crate) fn takes_double_clicks(&self) -> bool {
        self.binds(|g| matches!(g, ViewGesture::DoubleClick { .. }))
    }

    /// Whether the view's controls take a key, so the canvas takes the
    /// focus.
    pub(crate) fn takes_keys(&self) -> bool {
        self.view.as_ref().is_some_and(|v| v.controls.binds_keys())
    }
}

/// The view's modifier keys from egui's, Command counting as Ctrl.
fn modifiers(held: egui::Modifiers) -> CameraModifiers {
    CameraModifiers { shift: held.shift, ctrl: held.ctrl || held.mac_cmd, alt: held.alt }
}

/// The view's name for an egui pointer button, for the three it binds.
fn button(pressed: egui::PointerButton) -> Option<CameraButton> {
    match pressed {
        egui::PointerButton::Primary => Some(CameraButton::Left),
        egui::PointerButton::Secondary => Some(CameraButton::Right),
        egui::PointerButton::Middle => Some(CameraButton::Middle),
        egui::PointerButton::Extra1 | egui::PointerButton::Extra2 => None,
    }
}

/// How far a wheel turn of `delta` in `unit` moves a page's content, in
/// points: a line or a notch [`view::SCROLL_STEP`] points, a page the
/// canvas's `height`, and points as they are.
fn wheel_points(unit: egui::MouseWheelUnit, delta: egui::Vec2, height: f32) -> [f64; 2] {
    let by = match unit {
        egui::MouseWheelUnit::Point => delta,
        egui::MouseWheelUnit::Line => delta * view::SCROLL_STEP as f32,
        egui::MouseWheelUnit::Page => delta * height,
    };
    [f64::from(by.x), f64::from(by.y)]
}

/// What a named item takes from the view: a left drag begun on it, or a
/// double-click on it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Taken {
    Drag,
    DoubleClick,
}

/// What a frame's input did to the view: whether it moved it, whether it
/// took a drag begun this frame, which an item then does not take, and
/// the box a box zoom draws, to draw over the items.
#[derive(Debug, Default, PartialEq)]
pub(crate) struct Steered {
    pub(crate) moved: bool,
    pub(crate) took_drag: bool,
    pub(crate) drawn_box: Option<egui::Rect>,
}

/// Moves the view in `state` of a canvas whose response this frame is
/// `response`, drawn at `place`, as its controls take what the person did,
/// at `now`; lengths are points of the canvas's place in pixels. A left
/// drag without the space bar begun where `takes` says a named item takes
/// the drag, and a left double-click where one takes it, are left to the
/// item. Escape, with which egui ends every drag, lets the view's drag go
/// where it is, a box zoom's box going without its zoom and a pan without
/// its glide. `fit` is the box every item was last drawn in, in the items'
/// coordinates, for a fit. While the pointer is over a canvas whose
/// controls bind the wheel, the window's smoothed scrolling is taken, so
/// nothing behind the canvas scrolls; while two fingers touch it, the
/// pinch moves the view and the pointer's drag is let go.
pub(crate) fn steer(ui: &egui::Ui, response: &egui::Response, place: pointer::Place, state: &mut ViewState, takes: &dyn Fn(egui::Pos2, Taken) -> bool, fit: Option<[f64; 4]>, now: Instant) -> Steered {
    let mut out = Steered::default();
    let Some(controls) = state.view.as_ref().map(|v| v.controls.clone()) else {
        state.held = None;
        return out;
    };
    let rect = response.rect;
    let size = [f64::from(place.across / place.ppp), f64::from(place.down / place.ppp)];
    let local = |p: egui::Pos2| {
        let (x, y) = place.pixels(p);
        [f64::from(x / place.ppp), f64::from(y / place.ppp)]
    };
    let (held_keys, space, time, latest) = ui.input(|i| (i.modifiers, i.key_down(egui::Key::Space), i.time, i.pointer.latest_pos()));
    let touch = ui.input(egui::InputState::multi_touch).filter(|_| response.contains_pointer());
    if touch.is_some() {
        state.held = None;
    }
    if state.held.is_none() && touch.is_none() {
        for b in [egui::PointerButton::Primary, egui::PointerButton::Secondary, egui::PointerButton::Middle] {
            let Some(bound) = button(b).filter(|_| response.drag_started_by(b)) else {
                continue;
            };
            let origin = ui.input(|i| i.pointer.press_origin()).unwrap_or(rect.center());
            let Some(made) = controls.move_for(ViewGesture::Drag { button: bound, modifiers: modifiers(held_keys), space }) else {
                continue;
            };
            if b == egui::PointerButton::Primary && !space && takes(origin, Taken::Drag) {
                continue;
            }
            state.settle(now, size);
            state.held = Some(Held { button: b, made, from: origin, last: origin, places: vec![(time, local(origin))] });
            out.took_drag = true;
            break;
        }
    }
    let escaped = ui.input(|i| i.key_pressed(egui::Key::Escape));
    if let Some(mut h) = state.held.take().filter(|_| !escaped) {
        let at = latest.unwrap_or(h.last);
        if response.dragged_by(h.button) {
            if h.made == ViewMove::BoxZoom {
                out.drawn_box = Some(egui::Rect::from_two_pos(h.from, at));
            } else if at != h.last
                && let Some(v) = state.view.as_mut()
            {
                v.dragged(h.made, f64::from(at.x - h.last.x), f64::from(at.y - h.last.y), local(h.from), size);
                out.moved = true;
            }
            h.places.push((time, local(at)));
            h.places.retain(|(t, _)| time - t <= 2.0 * view::GLIDE_WINDOW);
            h.last = at;
            state.held = Some(h);
        } else {
            let ended = match (h.made, state.view.clone()) {
                (ViewMove::Glide, Some(v)) => view::release_velocity(&h.places, time).and_then(view::glide).map(|(by, seconds)| {
                    let mut to = v;
                    to.pan(by[0], by[1], size);
                    (to, ViewEase { seconds, easing: view::GLIDE_EASING })
                }),
                (ViewMove::BoxZoom, Some(v)) => v.boxed(local(h.from), local(at), size).map(|to| (to, ViewEase { seconds: view::STEP_SECONDS, easing: view::STEP_EASING })),
                _ => None,
            };
            if let Some((to, ease)) = ended {
                state.ease_to(to, ease, now, size);
                out.moved = true;
            }
        }
    }
    if response.contains_pointer() {
        let pointer = local(latest.unwrap_or(rect.center()));
        let turns: Vec<(egui::MouseWheelUnit, egui::Vec2, egui::Modifiers)> = ui.input(|i| {
            i.events
                .iter()
                .filter_map(|e| match e {
                    egui::Event::MouseWheel { unit, delta, modifiers: keys, .. } => Some((*unit, *delta, *keys)),
                    _ => None,
                })
                .collect()
        });
        for (unit, delta, keys) in turns {
            let Some(made) = controls.move_for(ViewGesture::Wheel { modifiers: modifiers(keys) }) else {
                continue;
            };
            state.settle(now, size);
            if let Some(v) = state.view.as_mut() {
                v.wheel(made, wheel_points(unit, delta, rect.height()), pointer, size);
                out.moved = true;
            }
        }
        if controls.bindings().iter().any(|(g, _)| matches!(g, ViewGesture::Wheel { .. })) {
            ui.ctx().input_mut(|i| i.smooth_scroll_delta = egui::Vec2::ZERO);
        }
        if let Some(made) = controls.move_for(ViewGesture::Pinch) {
            let spread: f32 = ui.input(|i| i.events.iter().filter_map(|e| if let egui::Event::Zoom(f) = e { Some(*f) } else { None }).product());
            let (factor, across, middle) = match &touch {
                Some(t) => (t.zoom_delta * spread, t.translation_delta, local(t.center_pos)),
                None => (spread, egui::Vec2::ZERO, pointer),
            };
            if factor != 1.0 || across != egui::Vec2::ZERO {
                state.settle(now, size);
                if let Some(v) = state.view.as_mut() {
                    v.pan(f64::from(across.x), f64::from(across.y), size);
                    if made == ViewMove::Zoom {
                        v.zoom_about(f64::from(factor), middle, size);
                    }
                    out.moved = true;
                }
            }
        }
    }
    for b in [egui::PointerButton::Primary, egui::PointerButton::Secondary, egui::PointerButton::Middle] {
        let Some(bound) = button(b).filter(|_| response.double_clicked_by(b)) else {
            continue;
        };
        let at = ui.input(|i| i.pointer.interact_pos()).unwrap_or(rect.center());
        if b == egui::PointerButton::Primary && takes(at, Taken::DoubleClick) {
            continue;
        }
        if let Some(made) = controls.move_for(ViewGesture::DoubleClick { button: bound, modifiers: modifiers(held_keys) }) {
            out.moved |= step(state, made, local(at), None, fit, now, size);
        }
    }
    if response.has_focus() {
        let presses: Vec<(egui::Key, Option<egui::Key>, egui::Modifiers)> = ui.input(|i| {
            i.events
                .iter()
                .filter_map(|e| match e {
                    egui::Event::Key { key, physical_key, pressed: true, modifiers: keys, .. } => Some((*key, *physical_key, *keys)),
                    _ => None,
                })
                .collect()
        });
        let middle = [size[0] / 2.0, size[1] / 2.0];
        for (key, place, keys) in presses {
            let mut pressed = match routed::named(key, place) {
                Ok(s) => s,
                Err(why) => {
                    proto::trace::line(|| format!("egui: the key {} moves no canvas view: {why}", key.name()));
                    continue;
                }
            };
            pressed.ctrl = keys.command;
            pressed.shift = keys.shift;
            pressed.alt = keys.alt;
            let Some(gesture) = ViewGesture::of_press(&pressed) else {
                continue;
            };
            if let Some(made) = controls.move_for(gesture) {
                let arrow = ViewKey::named(&pressed.key);
                out.moved |= step(state, made, middle, arrow, fit, now, size);
            }
        }
    }
    out
}

/// Starts the step `made` from the view drawn at `now`, about `at` on a
/// canvas `size` points across and down, an arrow's pan from `key`, a fit
/// to `fit`, eased over [`view::STEP_SECONDS`]; returns whether it began.
fn step(state: &mut ViewState, made: ViewMove, at: [f64; 2], key: Option<ViewKey>, fit: Option<[f64; 4]>, now: Instant, size: [f64; 2]) -> bool {
    state.settle(now, size);
    let Some(to) = state.view.as_ref().and_then(|v| v.stepped(made, at, key, fit, size)) else {
        return false;
    };
    state.ease_to(to, ViewEase { seconds: view::STEP_SECONDS, easing: view::STEP_EASING }, now, size);
    true
}

/// The box a box zoom draws over the items, as Leaflet draws it: half-clear
/// white with a dotted blue border.
pub(crate) fn draw_box(painter: &egui::Painter, area: egui::Rect) {
    let color = |c: proto::canvas::Rgba| egui::Color32::from_rgba_unmultiplied(c.0[0], c.0[1], c.0[2], c.0[3]);
    painter.rect_filled(area, 0.0, color(view::BOX_FILL));
    let radius = view::BOX_BORDER_WIDTH as f32 / 2.0;
    let corners = [area.left_top(), area.right_top(), area.right_bottom(), area.left_bottom(), area.left_top()];
    painter.extend(egui::Shape::dotted_line(&corners, color(view::BOX_BORDER), 2.0 * view::BOX_BORDER_WIDTH as f32, radius));
}

#[cfg(test)]
mod tests {
    use super::*;
    use proto::view::ViewControls;

    const SIZE: [f64; 2] = [200.0, 100.0];

    /// A view of `controls` centered on the canvas's own middle.
    fn state(controls: ViewControls) -> ViewState {
        ViewState { view: Some(ViewSpec { controls, ..ViewSpec::default() }), ..ViewState::default() }
    }

    /// A canvas 200 by 100 points at the top left of a window, run a frame
    /// at a time, each a hundredth of a second after the last at egui's
    /// clock, all at `now` on the view's: focused when `focus`, and with a
    /// named item that takes drags and double-clicks where `item` lies.
    struct Run {
        ctx: egui::Context,
        now: Instant,
        time: f64,
        focus: bool,
        item: Option<egui::Rect>,
    }

    impl Run {
        fn new() -> Run {
            Run { ctx: egui::Context::default(), now: Instant::now(), time: 0.0, focus: false, item: None }
        }

        /// One frame given `events` with `keys` pressed, steering `state`.
        fn frame(&mut self, events: Vec<egui::Event>, keys: egui::Modifiers, state: &mut ViewState) -> Steered {
            self.time += 0.01;
            let events = std::iter::once(egui::Event::ModifiersChanged(keys)).chain(events).collect();
            let input = egui::RawInput { events, time: Some(self.time), screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(400.0, 300.0))), ..egui::RawInput::default() };
            let (focus, item, now) = (self.focus, self.item, self.now);
            let mut steered = Steered::default();
            self.ctx
                .run_ui(input, |ui| {
                    let response = ui.allocate_rect(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(200.0, 100.0)), egui::Sense::click_and_drag());
                    if focus {
                        response.request_focus();
                    }
                    let takes = |at: egui::Pos2, _: Taken| item.is_some_and(|r| r.contains(at));
                    let place = pointer::Place { left: 0.0, top: 0.0, across: 200.0, down: 100.0, ppp: 1.0 };
                    steered = steer(ui, &response, place, state, &takes, Some([0.0, 0.0, 400.0, 100.0]), now);
                })
                .drop_without_applying_deltas();
            steered
        }

        /// A drag with `with` from `from` through `through`, `keys` pressed,
        /// then let go where it last moved; returns what the last frame did.
        fn drag(&mut self, with: egui::PointerButton, from: egui::Pos2, through: &[egui::Pos2], keys: egui::Modifiers, state: &mut ViewState) -> Steered {
            let button = |pos, down| egui::Event::PointerButton { pos, button: with, pressed: down, modifiers: keys };
            self.frame(vec![egui::Event::PointerMoved(from)], keys, state);
            self.frame(vec![button(from, true)], keys, state);
            for p in through {
                self.frame(vec![egui::Event::PointerMoved(*p)], keys, state);
            }
            let end = through.last().copied().unwrap_or(from);
            self.frame(vec![button(end, false)], keys, state)
        }

        /// The center of `state`'s view as drawn `seconds` after `now`.
        fn center_after(&self, state: &ViewState, seconds: f64) -> Option<[f64; 2]> {
            state.shown(self.now + std::time::Duration::from_secs_f64(seconds), SIZE).map(|v| v.center_in(SIZE))
        }
    }

    fn near(a: Option<[f64; 2]>, b: [f64; 2]) -> bool {
        a.is_some_and(|a| (a[0] - b[0]).abs() < 1e-3 && (a[1] - b[1]).abs() < 1e-3)
    }

    #[test]
    fn a_map_drag_pans_where_no_item_takes_it_and_is_left_to_an_item_that_does() {
        let mut run = Run::new();
        let mut map = state(ViewControls::Map);
        run.drag(egui::PointerButton::Primary, egui::pos2(50.0, 50.0), &[egui::pos2(70.0, 50.0), egui::pos2(90.0, 60.0)], egui::Modifiers::NONE, &mut map);
        let drawn = run.center_after(&map, 0.0);
        assert!(near(drawn, [60.0, 40.0]), "the items follow the pointer 40 across and 10 down: {drawn:?}");
        run.item = Some(egui::Rect::from_min_size(egui::pos2(40.0, 40.0), egui::vec2(20.0, 20.0)));
        let mut over_item = state(ViewControls::Map);
        run.drag(egui::PointerButton::Primary, egui::pos2(50.0, 50.0), &[egui::pos2(90.0, 50.0)], egui::Modifiers::NONE, &mut over_item);
        assert_eq!(over_item.view.as_ref().and_then(|v| v.center), None, "a drag begun on an item that takes drags is the item's");
    }

    #[test]
    fn a_flicked_map_pan_glides_on_and_a_shift_drag_zooms_to_its_box() {
        let mut run = Run::new();
        let mut map = state(ViewControls::Map);
        let path: Vec<egui::Pos2> = (1..=5).map(|i| egui::pos2(50.0 + 10.0 * i as f32, 50.0)).collect();
        let last = run.drag(egui::PointerButton::Primary, egui::pos2(50.0, 50.0), &path, egui::Modifiers::NONE, &mut map);
        assert!(last.moved && map.moving(run.now), "a pan let go at 1000 points a second glides on");
        assert!(near(run.center_after(&map, 0.0), [50.0, 50.0]), "drawn where the pointer let go: {:?}", run.center_after(&map, 0.0));
        let glided = run.center_after(&map, 1.0);
        assert!(glided.is_some_and(|c| c[0] < 50.0 - 30.0), "the glide moves the items on past where the pointer let go: {glided:?}");
        let mut boxed = state(ViewControls::Map);
        let shown = run.drag(egui::PointerButton::Primary, egui::pos2(0.0, 0.0), &[egui::pos2(100.0, 50.0)], egui::Modifiers::SHIFT, &mut boxed);
        assert!(shown.moved && boxed.view.as_ref().is_some_and(|v| (v.zoom - 2.0).abs() < 1e-9), "Shift and a drag over a quarter of the canvas zoom to it: {:?}", boxed.view);
        assert!(near(run.center_after(&boxed, 1.0), [50.0, 25.0]), "centered on the box");
    }

    #[test]
    fn escape_lets_a_box_zoom_and_a_glide_go_where_they_are() {
        let mut run = Run::new();
        let escape = || egui::Event::Key { key: egui::Key::Escape, physical_key: Some(egui::Key::Escape), pressed: true, repeat: false, modifiers: egui::Modifiers::NONE };
        let button = |pos, down, keys| egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed: down, modifiers: keys };
        let (shift, none) = (egui::Modifiers::SHIFT, egui::Modifiers::NONE);
        let mut boxed = state(ViewControls::Map);
        run.frame(vec![egui::Event::PointerMoved(egui::pos2(0.0, 0.0))], shift, &mut boxed);
        run.frame(vec![button(egui::pos2(0.0, 0.0), true, shift)], shift, &mut boxed);
        let held = run.frame(vec![egui::Event::PointerMoved(egui::pos2(100.0, 50.0))], shift, &mut boxed);
        assert!(held.drawn_box.is_some(), "a Shift drag draws its box");
        let escaped = run.frame(vec![escape()], shift, &mut boxed);
        assert_eq!(escaped.drawn_box, None, "Escape takes the box away");
        run.frame(vec![button(egui::pos2(100.0, 50.0), false, shift)], shift, &mut boxed);
        assert!(boxed.view.as_ref().is_some_and(|v| v.zoom == 1.0 && v.center.is_none()), "and nothing zooms: {:?}", boxed.view);
        let mut map = state(ViewControls::Map);
        run.frame(vec![egui::Event::PointerMoved(egui::pos2(50.0, 50.0))], none, &mut map);
        run.frame(vec![button(egui::pos2(50.0, 50.0), true, none)], none, &mut map);
        for i in 1..=5 {
            run.frame(vec![egui::Event::PointerMoved(egui::pos2(50.0 + 10.0 * i as f32, 50.0))], none, &mut map);
        }
        run.frame(vec![escape()], none, &mut map);
        run.frame(vec![button(egui::pos2(100.0, 50.0), false, none)], none, &mut map);
        assert!(near(run.center_after(&map, 1.0), [50.0, 50.0]), "the items stay where the pointer had them, with no glide: {:?}", run.center_after(&map, 1.0));
    }

    #[test]
    fn design_pans_with_the_space_bar_and_scrolls_with_the_wheel() {
        let mut run = Run::new();
        let mut design = state(ViewControls::Design);
        run.drag(egui::PointerButton::Primary, egui::pos2(50.0, 50.0), &[egui::pos2(80.0, 50.0)], egui::Modifiers::NONE, &mut design);
        assert_eq!(design.view.as_ref().and_then(|v| v.center), None, "a plain left drag is the scene's");
        let space = egui::Event::Key { key: egui::Key::Space, physical_key: None, pressed: true, repeat: false, modifiers: egui::Modifiers::NONE };
        run.frame(vec![space], egui::Modifiers::NONE, &mut design);
        run.drag(egui::PointerButton::Primary, egui::pos2(50.0, 50.0), &[egui::pos2(80.0, 50.0)], egui::Modifiers::NONE, &mut design);
        assert!(near(run.center_after(&design, 1.0), [70.0, 50.0]), "with the space bar down a left drag pans, and no glide follows: {:?}", run.center_after(&design, 1.0));
        let turn = egui::Event::MouseWheel { unit: egui::MouseWheelUnit::Line, delta: egui::vec2(0.0, 1.0), phase: egui::TouchPhase::Move, modifiers: egui::Modifiers::NONE, source: egui::MouseWheelSource::Wheel };
        run.frame(vec![egui::Event::PointerMoved(egui::pos2(100.0, 50.0)), turn], egui::Modifiers::NONE, &mut design);
        assert!(near(run.center_after(&design, 0.0), [70.0, 50.0 - view::SCROLL_STEP]), "a notch away shows what is above: {:?}", run.center_after(&design, 0.0));
    }

    #[test]
    fn a_double_click_and_a_key_step_the_view_over_a_quarter_second() {
        let mut run = Run::new();
        let mut map = state(ViewControls::Map);
        let click = |down| egui::Event::PointerButton { pos: egui::pos2(150.0, 50.0), button: egui::PointerButton::Primary, pressed: down, modifiers: egui::Modifiers::NONE };
        for events in [vec![egui::Event::PointerMoved(egui::pos2(150.0, 50.0))], vec![click(true)], vec![click(false)], vec![click(true)], vec![click(false)]] {
            run.frame(events, egui::Modifiers::NONE, &mut map);
        }
        assert!(map.view.as_ref().is_some_and(|v| (v.zoom - view::ZOOM_IN_STEP).abs() < 1e-9), "a double-click zooms in a step: {:?}", map.view);
        let halfway = map.shown(run.now + std::time::Duration::from_millis(125), SIZE).map(|v| v.zoom);
        assert!(halfway.is_some_and(|z| z > 1.0 && z < view::ZOOM_IN_STEP), "eased over a quarter second: {halfway:?}");
        let ended = map.shown(run.now + std::time::Duration::from_millis(300), SIZE).map(|v| v.to_items([150.0, 50.0], SIZE));
        assert!(near(ended, [150.0, 50.0]), "about the pointer: {ended:?}");
        let mut keyed = state(ViewControls::Map);
        let right = egui::Event::Key { key: egui::Key::ArrowRight, physical_key: Some(egui::Key::ArrowRight), pressed: true, repeat: false, modifiers: egui::Modifiers::NONE };
        run.frame(vec![right.clone()], egui::Modifiers::NONE, &mut keyed);
        assert_eq!(keyed.view.as_ref().and_then(|v| v.center), None, "a key moves the view only while the canvas has the focus");
        run.focus = true;
        run.frame(Vec::new(), egui::Modifiers::NONE, &mut keyed);
        run.frame(vec![right], egui::Modifiers::NONE, &mut keyed);
        assert!(near(run.center_after(&keyed, 1.0), [100.0 + view::KEY_STEP, 50.0]), "the right arrow shows what lies to the right: {:?}", run.center_after(&keyed, 1.0));
    }

    #[test]
    fn a_set_during_a_move_starts_from_where_the_move_has_the_view() {
        let now = Instant::now();
        let mut moving = state(ViewControls::Map);
        let far = ViewSpec { center: Some([1000.0, 0.0]), ..ViewSpec::default() };
        moving.ease_to(far, ViewEase { seconds: 1.0, easing: proto::canvas::Easing::Linear }, now, SIZE);
        let halfway = now + std::time::Duration::from_millis(500);
        let answer = moving.answer(halfway, Some(SIZE));
        assert!(answer.as_ref().is_some_and(|a| a.moving && near(a.view.center, [550.0, 25.0])), "Get reads the view in between: {answer:?}");
        moving.settle(halfway, SIZE);
        assert!(near(moving.view.as_ref().and_then(|v| v.center), [550.0, 25.0]), "the move ends where it is");
        assert!(!moving.moving(halfway));
        assert!(state(ViewControls::Design).keeps(&proto::Shortcut::parse("Space").expect("a key")), "Design keeps Space for its drag");
        moving.remove();
        assert_eq!(moving.answer(halfway, Some(SIZE)), None);
    }

    #[test]
    fn an_eased_set_on_a_canvas_without_a_view_moves_from_the_view_it_is_drawn_under() {
        let now = Instant::now();
        let mut fresh = ViewState::default();
        let far = ViewSpec { center: Some([1000.0, 50.0]), ..ViewSpec::default() };
        fresh.ease_to(far, ViewEase { seconds: 1.0, easing: proto::canvas::Easing::Linear }, now, SIZE);
        let halfway = fresh.answer(now + std::time::Duration::from_millis(500), Some(SIZE));
        assert!(halfway.as_ref().is_some_and(|a| a.moving && near(a.view.center, [550.0, 50.0])), "the move starts at the canvas's own middle: {halfway:?}");
    }
}
