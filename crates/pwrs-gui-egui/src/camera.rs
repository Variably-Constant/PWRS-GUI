//! A GpuSurface's camera moved by the person: a press over the surface
//! chooses the move its drag makes, by its button and modifier keys, until
//! the button is released, and each wheel turn over the surface makes the
//! move its modifier keys are bound to.

use crate::{egui, proto};
use proto::camera::{CameraButton, CameraGesture, CameraModifiers, CameraMove, CameraSpec};

/// The camera's modifier keys from egui's, Command counting as Ctrl.
fn modifiers(held: egui::Modifiers) -> CameraModifiers {
    CameraModifiers { shift: held.shift, ctrl: held.ctrl || held.mac_cmd, alt: held.alt }
}

/// The camera's name for an egui pointer button, for the three it binds.
fn button(pressed: egui::PointerButton) -> Option<CameraButton> {
    match pressed {
        egui::PointerButton::Primary => Some(CameraButton::Left),
        egui::PointerButton::Secondary => Some(CameraButton::Right),
        egui::PointerButton::Middle => Some(CameraButton::Middle),
        egui::PointerButton::Extra1 | egui::PointerButton::Extra2 => None,
    }
}

/// The notches a wheel turn of `delta` in `unit` makes, positive when the
/// content would move down: a line or a page counts one, 100 points one,
/// as three.js counts a browser's pixel deltas.
pub(crate) fn notches(unit: egui::MouseWheelUnit, delta: f32) -> f64 {
    match unit {
        egui::MouseWheelUnit::Point => f64::from(delta) / 100.0,
        egui::MouseWheelUnit::Line | egui::MouseWheelUnit::Page => f64::from(delta),
    }
}

/// A wheel turn's `delta` in `unit` as points, a line counting egui's
/// `line_scroll_speed` and a page the surface's `height`.
fn points(unit: egui::MouseWheelUnit, delta: egui::Vec2, line: f32, height: f32) -> egui::Vec2 {
    match unit {
        egui::MouseWheelUnit::Point => delta,
        egui::MouseWheelUnit::Line => delta * line,
        egui::MouseWheelUnit::Page => delta * height,
    }
}

/// The drag a press over a surface started: the button down, and the move
/// its gesture makes.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Held {
    button: egui::PointerButton,
    made: CameraMove,
}

/// Moves `camera` by what the person did over `response`, the surface's,
/// this frame: a drag makes the move its press chose, stored in `held` until
/// the button is released, and each wheel turn while the pointer is over
/// the surface makes the move its modifier keys are bound to, zooming by
/// its notches or orbiting or panning by its points as a drag would. While
/// the pointer is over a surface whose controls bind the wheel, the
/// window's smoothed scrolling is taken every frame, so nothing behind the
/// surface scrolls. Returns whether the camera moved.
pub(crate) fn steer(ui: &egui::Ui, response: &egui::Response, camera: &mut CameraSpec, held: &mut Option<Held>) -> bool {
    let height = response.rect.height();
    let mut moved = false;
    if held.is_none() {
        let pressed = ui.input(|i| i.modifiers);
        *held = [egui::PointerButton::Primary, egui::PointerButton::Secondary, egui::PointerButton::Middle]
            .into_iter()
            .filter(|b| response.drag_started_by(*b))
            .find_map(|b| {
                let gesture = CameraGesture::Drag { button: button(b)?, modifiers: modifiers(pressed) };
                camera.controls.move_for(gesture).map(|made| Held { button: b, made })
            });
    }
    if let Some(h) = *held {
        if response.dragged_by(h.button) {
            let delta = response.drag_delta();
            if delta != egui::Vec2::ZERO {
                camera.drag(h.made, f64::from(delta.x), f64::from(delta.y), f64::from(height));
                moved = true;
            }
        } else {
            *held = None;
        }
    }
    if !response.contains_pointer() {
        return moved;
    }
    let line = ui.ctx().options(|o| o.input_options.line_scroll_speed);
    let turns: Vec<(egui::MouseWheelUnit, egui::Vec2, CameraModifiers)> = ui.input(|i| {
        i.events
            .iter()
            .filter_map(|e| match e {
                egui::Event::MouseWheel { unit, delta, modifiers: keys, .. } => Some((*unit, *delta, modifiers(*keys))),
                _ => None,
            })
            .collect()
    });
    for (unit, delta, keys) in turns {
        let Some(made) = camera.controls.move_for(CameraGesture::Wheel { modifiers: keys }) else {
            continue;
        };
        match made {
            CameraMove::Zoom => camera.zoom(notches(unit, delta.y)),
            CameraMove::Orbit | CameraMove::Pan => {
                let by = points(unit, delta, line, height);
                camera.drag(made, f64::from(by.x), f64::from(by.y), f64::from(height));
            }
        }
        moved = true;
    }
    if camera.controls.bindings().iter().any(|(gesture, _)| matches!(gesture, CameraGesture::Wheel { .. })) {
        ui.ctx().input_mut(|i| i.smooth_scroll_delta = egui::Vec2::ZERO);
    }
    moved
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_wheel_notch_counts_one_however_the_platform_measures_it() {
        assert_eq!(notches(egui::MouseWheelUnit::Line, 1.0), 1.0);
        assert_eq!(notches(egui::MouseWheelUnit::Point, -250.0), -2.5);
        assert_eq!(notches(egui::MouseWheelUnit::Page, 1.0), 1.0);
        assert_eq!(points(egui::MouseWheelUnit::Line, egui::vec2(0.0, 2.0), 40.0, 300.0), egui::vec2(0.0, 80.0));
        assert_eq!(points(egui::MouseWheelUnit::Page, egui::vec2(1.0, 0.0), 40.0, 300.0), egui::vec2(300.0, 0.0));
    }

    /// Runs one frame of a 200 by 100 point surface at the top left of a
    /// window, given `events` with `keys` pressed, steering `camera`; returns
    /// whether it moved.
    fn frame(ctx: &egui::Context, events: Vec<egui::Event>, keys: egui::Modifiers, camera: &mut CameraSpec, held: &mut Option<Held>) -> bool {
        let events = std::iter::once(egui::Event::ModifiersChanged(keys)).chain(events).collect();
        let input = egui::RawInput { events, screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(400.0, 300.0))), ..egui::RawInput::default() };
        let mut moved = false;
        ctx.run_ui(input, |ui| {
            let response = ui.allocate_rect(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(200.0, 100.0)), egui::Sense::drag());
            moved |= steer(ui, &response, camera, held);
        })
        .drop_without_applying_deltas();
        moved
    }

    /// A drag with button `with` from (50, 50) to `to`, `keys` pressed, over
    /// frames of its own: the pointer arrives, the button goes down, the
    /// pointer moves, the button comes up.
    fn drag(ctx: &egui::Context, with: egui::PointerButton, to: egui::Pos2, keys: egui::Modifiers, camera: &mut CameraSpec, held: &mut Option<Held>) -> bool {
        let from = egui::pos2(50.0, 50.0);
        let button = |pos, down| egui::Event::PointerButton { pos, button: with, pressed: down, modifiers: keys };
        let mut moved = frame(ctx, vec![egui::Event::PointerMoved(from)], keys, camera, held);
        moved |= frame(ctx, vec![button(from, true)], keys, camera, held);
        moved |= frame(ctx, vec![egui::Event::PointerMoved(to)], keys, camera, held);
        moved |= frame(ctx, vec![button(to, false)], keys, camera, held);
        moved |= frame(ctx, Vec::new(), keys, camera, held);
        moved
    }

    #[test]
    fn a_left_drag_orbits_under_the_default_controls_and_does_nothing_under_blender() {
        let ctx = egui::Context::default();
        let (mut camera, mut held) = (CameraSpec::default(), None);
        assert!(drag(&ctx, egui::PointerButton::Primary, egui::pos2(75.0, 50.0), egui::Modifiers::NONE, &mut camera, &mut held));
        assert!((camera.yaw - -90.0).abs() < 1e-6, "a quarter of the surface's height to the right turns a quarter circle: {}", camera.yaw);
        assert_eq!(held, None, "the release ends the drag");
        let blender = CameraSpec { controls: proto::camera::CameraControls::Blender, ..CameraSpec::default() };
        let mut moved = blender.clone();
        assert!(!drag(&ctx, egui::PointerButton::Primary, egui::pos2(75.0, 50.0), egui::Modifiers::NONE, &mut moved, &mut held));
        assert_eq!(moved, blender, "Blender leaves the left button to the scene");
    }

    #[test]
    fn blender_orbits_with_the_middle_button_and_pans_with_shift() {
        let ctx = egui::Context::default();
        let start = CameraSpec { controls: proto::camera::CameraControls::Blender, ..CameraSpec::default() };
        let (mut camera, mut held) = (start.clone(), None);
        assert!(drag(&ctx, egui::PointerButton::Middle, egui::pos2(50.0, 60.0), egui::Modifiers::NONE, &mut camera, &mut held));
        assert!((camera.pitch - 36.0).abs() < 1e-6, "ten points down on a surface 100 tall raise the camera 36 degrees: {}", camera.pitch);
        let mut panned = start.clone();
        assert!(drag(&ctx, egui::PointerButton::Middle, egui::pos2(60.0, 50.0), egui::Modifiers::SHIFT, &mut panned, &mut held));
        assert!(panned.target[0] < 0.0 && panned.target[1].abs() < 1e-9 && (panned.yaw, panned.pitch) == (0.0, 0.0), "Shift pans the target left as the scene follows the pointer right: {:?}", panned.target);
    }

    #[test]
    fn the_wheel_over_the_surface_zooms_and_elsewhere_does_nothing() {
        let ctx = egui::Context::default();
        let (mut camera, mut held) = (CameraSpec::default(), None);
        let turn = egui::Event::MouseWheel { unit: egui::MouseWheelUnit::Line, delta: egui::vec2(0.0, 1.0), phase: egui::TouchPhase::Move, modifiers: egui::Modifiers::NONE, source: egui::MouseWheelSource::Wheel };
        assert!(!frame(&ctx, vec![egui::Event::PointerMoved(egui::pos2(300.0, 200.0)), turn.clone()], egui::Modifiers::NONE, &mut camera, &mut held));
        assert_eq!(camera.distance, 5.0, "a turn outside the surface leaves the camera");
        assert!(frame(&ctx, vec![egui::Event::PointerMoved(egui::pos2(50.0, 50.0)), turn], egui::Modifiers::NONE, &mut camera, &mut held));
        assert!((camera.distance - 4.75).abs() < 1e-9, "one notch forward moves the camera in by a twentieth: {}", camera.distance);
    }

    #[test]
    fn command_counts_as_ctrl_and_only_the_three_buttons_bind() {
        let command = egui::Modifiers { mac_cmd: true, command: true, ..egui::Modifiers::NONE };
        assert_eq!(modifiers(command), CameraModifiers::CTRL);
        assert_eq!(modifiers(egui::Modifiers::SHIFT), CameraModifiers::SHIFT);
        assert_eq!(button(egui::PointerButton::Middle), Some(CameraButton::Middle));
        assert_eq!(button(egui::PointerButton::Extra1), None);
    }
}
