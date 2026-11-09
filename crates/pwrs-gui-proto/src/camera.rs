//! A GpuSurface's built-in camera: the pose and projection that fill its
//! frame uniform's view and projection, and the pointer controls that move
//! it, as one of two presets or the session's own mapping.

use serde::{Deserialize, Serialize};

/// A pointer button a camera control is bound to.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum CameraButton {
    Left,
    Right,
    Middle,
}

/// The modifier keys pressed with a control's button or wheel, matched
/// exactly: a gesture bound with Shift alone is not made with Shift and Ctrl.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct CameraModifiers {
    pub shift: bool,
    /// Ctrl, or Command on macOS, as three.js takes its meta key.
    pub ctrl: bool,
    pub alt: bool,
}

impl CameraModifiers {
    pub const NONE: CameraModifiers = CameraModifiers { shift: false, ctrl: false, alt: false };
    pub const SHIFT: CameraModifiers = CameraModifiers { shift: true, ctrl: false, alt: false };
    pub const CTRL: CameraModifiers = CameraModifiers { shift: false, ctrl: true, alt: false };
}

/// What the person does inside the surface: a button dragged, or the wheel
/// turned, with modifier keys pressed.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum CameraGesture {
    Drag { button: CameraButton, modifiers: CameraModifiers },
    Wheel { modifiers: CameraModifiers },
}

/// How a gesture moves the camera.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum CameraMove {
    /// Around the target: a drag the surface's height across turns a full
    /// circle.
    Orbit,
    /// The target and the camera together, so the point under the pointer
    /// follows it.
    Pan,
    /// Toward the target or away from it.
    Zoom,
}

/// Which gestures move the camera, and how.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum CameraControls {
    /// three.js OrbitControls' mapping: the left button orbits, or pans
    /// with Shift or Ctrl; the right button pans, or orbits with Shift or
    /// Ctrl; the middle button and the wheel zoom.
    #[default]
    Default,
    /// Blender's: the middle button orbits, pans with Shift and zooms with
    /// Ctrl; the wheel zooms.
    Blender,
    /// The session's own: each gesture and the move it makes; where several
    /// match the person's gesture, the first is used.
    Custom(Vec<(CameraGesture, CameraMove)>),
    /// No gesture: the camera moves only when the session sets it.
    Fixed,
}

impl CameraControls {
    /// Each gesture these controls take and the move it makes, in the order
    /// they are matched.
    pub fn bindings(&self) -> Vec<(CameraGesture, CameraMove)> {
        let drag = |button, modifiers| CameraGesture::Drag { button, modifiers };
        match self {
            CameraControls::Default => vec![
                (drag(CameraButton::Left, CameraModifiers::NONE), CameraMove::Orbit),
                (drag(CameraButton::Left, CameraModifiers::SHIFT), CameraMove::Pan),
                (drag(CameraButton::Left, CameraModifiers::CTRL), CameraMove::Pan),
                (drag(CameraButton::Right, CameraModifiers::NONE), CameraMove::Pan),
                (drag(CameraButton::Right, CameraModifiers::SHIFT), CameraMove::Orbit),
                (drag(CameraButton::Right, CameraModifiers::CTRL), CameraMove::Orbit),
                (drag(CameraButton::Middle, CameraModifiers::NONE), CameraMove::Zoom),
                (CameraGesture::Wheel { modifiers: CameraModifiers::NONE }, CameraMove::Zoom),
            ],
            CameraControls::Blender => vec![
                (drag(CameraButton::Middle, CameraModifiers::NONE), CameraMove::Orbit),
                (drag(CameraButton::Middle, CameraModifiers::SHIFT), CameraMove::Pan),
                (drag(CameraButton::Middle, CameraModifiers::CTRL), CameraMove::Zoom),
                (CameraGesture::Wheel { modifiers: CameraModifiers::NONE }, CameraMove::Zoom),
            ],
            CameraControls::Custom(bindings) => bindings.clone(),
            CameraControls::Fixed => Vec::new(),
        }
    }

    /// The move `gesture` makes under these controls, if any.
    pub fn move_for(&self, gesture: CameraGesture) -> Option<CameraMove> {
        self.bindings().into_iter().find(|(bound, _)| *bound == gesture).map(|(_, made)| made)
    }
}

/// A GpuSurface camera's pose, projection and controls. Angles are in
/// degrees and lengths in the scene's units, right-handed with +Y up.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CameraSpec {
    /// The point the camera looks at and orbits.
    pub target: [f64; 3],
    /// How far the camera is from the target.
    pub distance: f64,
    /// The angle around +Y from +Z toward +X: at 0 the camera is on the
    /// target's +Z side, looking toward -Z.
    pub yaw: f64,
    /// How far the camera is raised above the target's level: from -90,
    /// under it looking up, to 90, over it looking down.
    pub pitch: f64,
    /// The vertical field of view.
    pub field_of_view: f64,
    /// The distances from the camera between which the scene is drawn.
    pub near: f64,
    pub far: f64,
    pub controls: CameraControls,
}

impl Default for CameraSpec {
    /// Five units out along +Z from the origin, with three.js
    /// PerspectiveCamera's projection: a 50 degree field of view, near 0.1
    /// and far 2000.
    fn default() -> Self {
        CameraSpec { target: [0.0; 3], distance: 5.0, yaw: 0.0, pitch: 0.0, field_of_view: 50.0, near: 0.1, far: 2000.0, controls: CameraControls::Default }
    }
}

/// A 4 by 4 matrix as four columns of four rows, as WGSL lays out a
/// `mat4x4<f32>`.
pub type Matrix = [[f32; 4]; 4];

/// The factor one wheel notch, or a drag of 100 points, scales the distance
/// by, as three.js OrbitControls zooms.
const ZOOM_STEP: f64 = 0.95;

fn dot(a: [f64; 3], b: [f64; 3]) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn cross(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}

impl CameraSpec {
    /// The camera's right, up and backward unit vectors in the scene; it
    /// looks along the backward one negated. Right comes from the yaw
    /// alone, so they stay a basis looking straight up or down.
    fn axes(&self) -> ([f64; 3], [f64; 3], [f64; 3]) {
        let (yaw, pitch) = (self.yaw.to_radians(), self.pitch.to_radians());
        let back = [pitch.cos() * yaw.sin(), pitch.sin(), pitch.cos() * yaw.cos()];
        let right = [yaw.cos(), 0.0, -yaw.sin()];
        (right, cross(back, right), back)
    }

    /// Where the camera is: `distance` from the target along its backward
    /// axis.
    pub fn eye(&self) -> [f64; 3] {
        let (_, _, back) = self.axes();
        let [x, y, z] = self.target;
        [x + self.distance * back[0], y + self.distance * back[1], z + self.distance * back[2]]
    }

    /// The matrix from the scene's coordinates to the camera's.
    pub fn view(&self) -> Matrix {
        let (right, up, back) = self.axes();
        let eye = self.eye();
        let column = |axis: usize| [right[axis] as f32, up[axis] as f32, back[axis] as f32, 0.0];
        [column(0), column(1), column(2), [-dot(right, eye) as f32, -dot(up, eye) as f32, -dot(back, eye) as f32, 1.0]]
    }

    /// The perspective matrix from the camera's coordinates to clip space
    /// for a surface `aspect` times as wide as it is tall, depth running
    /// from 0 at `near` to 1 at `far` as wgpu's clip space has it.
    pub fn projection(&self, aspect: f64) -> Matrix {
        let height = 1.0 / (self.field_of_view.to_radians() / 2.0).tan();
        let depth = self.far / (self.near - self.far);
        [[(height / aspect) as f32, 0.0, 0.0, 0.0], [0.0, height as f32, 0.0, 0.0], [0.0, 0.0, depth as f32, -1.0], [0.0, 0.0, (depth * self.near) as f32, 0.0]]
    }

    /// Turns the camera around the target for a drag `across` and `down`
    /// points on a surface `height` points tall: rightward turns it left,
    /// downward raises it. The yaw stays within -180 to 180 degrees and
    /// the pitch within -90 to 90.
    pub fn orbit(&mut self, across: f64, down: f64, height: f64) {
        self.yaw = (self.yaw - 360.0 * across / height + 180.0).rem_euclid(360.0) - 180.0;
        self.pitch = (self.pitch + 360.0 * down / height).clamp(-90.0, 90.0);
    }

    /// Moves the target and the camera together for a drag `across` and
    /// `down` points on a surface `height` points tall, so a point at the
    /// target's depth stays under the pointer.
    pub fn pan(&mut self, across: f64, down: f64, height: f64) {
        let (right, up, _) = self.axes();
        let half = self.distance * (self.field_of_view.to_radians() / 2.0).tan();
        let (sideways, upward) = (-2.0 * across * half / height, 2.0 * down * half / height);
        for axis in 0..3 {
            self.target[axis] += right[axis] * sideways + up[axis] * upward;
        }
    }

    /// Moves the camera toward the target by `notches` wheel notches, away
    /// from it for a negative count.
    pub fn zoom(&mut self, notches: f64) {
        self.distance *= ZOOM_STEP.powf(notches);
    }

    /// Makes `made` for a drag `across` and `down` points on a surface
    /// `height` points tall; a zoom drag moves 1 notch per 100 points up.
    pub fn drag(&mut self, made: CameraMove, across: f64, down: f64, height: f64) {
        match made {
            CameraMove::Orbit => self.orbit(across, down, height),
            CameraMove::Pan => self.pan(across, down, height),
            CameraMove::Zoom => self.zoom(-down / 100.0),
        }
    }
}

/// The parts of a camera a call changes; a part left `None` stays as the
/// camera has it, or as [`CameraSpec::default`] gives it to a surface
/// without one.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct CameraChange {
    pub target: Option<[f64; 3]>,
    pub distance: Option<f64>,
    pub yaw: Option<f64>,
    pub pitch: Option<f64>,
    pub field_of_view: Option<f64>,
    pub near: Option<f64>,
    pub far: Option<f64>,
    pub controls: Option<CameraControls>,
}

impl CameraChange {
    /// `spec` with this change's parts in place of its own.
    pub fn applied_to(&self, spec: &CameraSpec) -> CameraSpec {
        CameraSpec {
            target: self.target.unwrap_or(spec.target),
            distance: self.distance.unwrap_or(spec.distance),
            yaw: self.yaw.unwrap_or(spec.yaw),
            pitch: self.pitch.unwrap_or(spec.pitch),
            field_of_view: self.field_of_view.unwrap_or(spec.field_of_view),
            near: self.near.unwrap_or(spec.near),
            far: self.far.unwrap_or(spec.far),
            controls: self.controls.clone().unwrap_or_else(|| spec.controls.clone()),
        }
    }

    /// Why this change cannot make a camera, naming the part, or `None`.
    pub fn problem(&self, spec: &CameraSpec) -> Option<String> {
        let made = self.applied_to(spec);
        let finite = made.target.iter().chain([made.distance, made.yaw, made.pitch, made.field_of_view, made.near, made.far].iter()).all(|v| v.is_finite());
        if !finite {
            return Some(String::from("every number a camera takes is finite"));
        }
        if made.distance <= 0.0 {
            return Some(format!("the distance would be {}, and a camera's distance is greater than 0", made.distance));
        }
        if !(-90.0..=90.0).contains(&made.pitch) {
            return Some(format!("the pitch would be {} degrees, and a camera's pitch is from -90 to 90", made.pitch));
        }
        if made.field_of_view <= 0.0 || made.field_of_view >= 180.0 {
            return Some(format!("the field of view would be {} degrees, and a camera's is between 0 and 180", made.field_of_view));
        }
        if made.near <= 0.0 || made.far <= made.near {
            return Some(format!("near would be {} and far {}, and a camera's near is greater than 0 and its far greater than its near", made.near, made.far));
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_preset_makes_the_moves_its_namesake_does() {
        let drag = |button, modifiers| CameraGesture::Drag { button, modifiers };
        let wheel = CameraGesture::Wheel { modifiers: CameraModifiers::NONE };
        let three = CameraControls::Default;
        assert_eq!(three.move_for(drag(CameraButton::Left, CameraModifiers::NONE)), Some(CameraMove::Orbit));
        assert_eq!(three.move_for(drag(CameraButton::Left, CameraModifiers::SHIFT)), Some(CameraMove::Pan));
        assert_eq!(three.move_for(drag(CameraButton::Right, CameraModifiers::NONE)), Some(CameraMove::Pan));
        assert_eq!(three.move_for(drag(CameraButton::Middle, CameraModifiers::NONE)), Some(CameraMove::Zoom));
        assert_eq!(three.move_for(wheel), Some(CameraMove::Zoom));
        let blender = CameraControls::Blender;
        assert_eq!(blender.move_for(drag(CameraButton::Middle, CameraModifiers::NONE)), Some(CameraMove::Orbit));
        assert_eq!(blender.move_for(drag(CameraButton::Middle, CameraModifiers::SHIFT)), Some(CameraMove::Pan));
        assert_eq!(blender.move_for(drag(CameraButton::Middle, CameraModifiers::CTRL)), Some(CameraMove::Zoom));
        assert_eq!(blender.move_for(drag(CameraButton::Left, CameraModifiers::NONE)), None, "Blender leaves the left button to the scene");
        assert_eq!(blender.move_for(wheel), Some(CameraMove::Zoom));
        assert_eq!(CameraControls::Fixed.move_for(wheel), None);
    }

    #[test]
    fn custom_controls_match_modifiers_exactly_and_the_first_binding_wins() {
        let shift_left = CameraGesture::Drag { button: CameraButton::Left, modifiers: CameraModifiers::SHIFT };
        let custom = CameraControls::Custom(vec![(shift_left, CameraMove::Zoom), (shift_left, CameraMove::Orbit)]);
        assert_eq!(custom.move_for(shift_left), Some(CameraMove::Zoom));
        let both = CameraModifiers { shift: true, ctrl: true, alt: false };
        assert_eq!(custom.move_for(CameraGesture::Drag { button: CameraButton::Left, modifiers: both }), None);
    }

    #[test]
    fn a_change_keeps_the_parts_it_leaves_out_and_refuses_a_camera_that_cannot_draw() {
        let start = CameraSpec { yaw: 30.0, controls: CameraControls::Blender, ..CameraSpec::default() };
        let switched = CameraChange { controls: Some(CameraControls::Default), ..CameraChange::default() };
        let made = switched.applied_to(&start);
        assert_eq!((made.yaw, made.controls), (30.0, CameraControls::Default), "switching the controls leaves the pose as it was");
        assert_eq!(switched.problem(&start), None);
        assert!(CameraChange { distance: Some(0.0), ..CameraChange::default() }.problem(&start).is_some_and(|p| p.contains("distance")));
        assert!(CameraChange { far: Some(0.05), ..CameraChange::default() }.problem(&start).is_some_and(|p| p.contains("far")));
        assert!(CameraChange { field_of_view: Some(180.0), ..CameraChange::default() }.problem(&start).is_some_and(|p| p.contains("field of view")));
        assert!(CameraChange { yaw: Some(f64::NAN), ..CameraChange::default() }.problem(&start).is_some_and(|p| p.contains("finite")));
    }

    #[test]
    fn a_camera_survives_the_wire() {
        let gesture = CameraGesture::Drag { button: CameraButton::Right, modifiers: CameraModifiers { shift: false, ctrl: true, alt: true } };
        let spec = CameraSpec { target: [1.0, -2.0, 0.5], controls: CameraControls::Custom(vec![(gesture, CameraMove::Pan)]), ..CameraSpec::default() };
        let bytes = postcard::to_stdvec(&spec).expect("encodes");
        let back: CameraSpec = postcard::from_bytes(&bytes).expect("decodes");
        assert_eq!(back, spec);
        let change = CameraChange { pitch: Some(-15.0), controls: Some(CameraControls::Fixed), ..CameraChange::default() };
        let bytes = postcard::to_stdvec(&change).expect("encodes");
        let back: CameraChange = postcard::from_bytes(&bytes).expect("decodes");
        assert_eq!(back, change);
    }

    /// `matrix` times `vector`, the matrix given as columns.
    fn times(matrix: &Matrix, vector: [f64; 4]) -> [f64; 4] {
        let mut out = [0.0; 4];
        for (column, x) in matrix.iter().zip(vector) {
            for (row, value) in column.iter().enumerate() {
                out[row] += f64::from(*value) * x;
            }
        }
        out
    }

    /// Where `point` shows on a surface `width` by `height` pixels seen
    /// through `spec`, in pixels from its top left, and its depth.
    fn on_surface(spec: &CameraSpec, point: [f64; 3], width: f64, height: f64) -> (f64, f64, f64) {
        let clip = times(&spec.projection(width / height), times(&spec.view(), [point[0], point[1], point[2], 1.0]));
        let (x, y, depth) = (clip[0] / clip[3], clip[1] / clip[3], clip[2] / clip[3]);
        ((x + 1.0) / 2.0 * width, (1.0 - y) / 2.0 * height, depth)
    }

    fn near(a: f64, b: f64) -> bool {
        (a - b).abs() < 1e-3
    }

    #[test]
    fn the_default_camera_shows_the_origin_at_the_center_from_five_units_out_with_wgpu_depth() {
        let spec = CameraSpec::default();
        assert_eq!(spec.eye(), [0.0, 0.0, 5.0]);
        assert_eq!(spec.view()[3], [0.0, 0.0, -5.0, 1.0], "the origin is 5 units in front of the camera");
        let (x, y, _) = on_surface(&spec, [0.0; 3], 200.0, 100.0);
        assert!(near(x, 100.0) && near(y, 50.0), "the target shows at the center: {x}, {y}");
        let (_, above, _) = on_surface(&spec, [0.0, 1.0, 0.0], 200.0, 100.0);
        assert!(above < 50.0, "+Y shows up the surface: {above}");
        let (_, _, at_near) = on_surface(&spec, [0.0, 0.0, 5.0 - 0.1], 200.0, 100.0);
        let (_, _, at_far) = on_surface(&spec, [0.0, 0.0, 5.0 - 2000.0], 200.0, 100.0);
        assert!(at_near.abs() < 1e-4 && (at_far - 1.0).abs() < 1e-4, "depth runs from 0 at near to 1 at far: {at_near}, {at_far}");
        let projection = spec.projection(2.0);
        assert!((projection[0][0] * 2.0 - projection[1][1]).abs() < 1e-6, "x is scaled for a surface twice as wide as tall");
    }

    #[test]
    fn a_drag_orbits_as_three_js_does_and_stops_at_straight_down() {
        let mut spec = CameraSpec::default();
        spec.orbit(25.0, 0.0, 100.0);
        let eye = spec.eye();
        assert!(near(spec.yaw, -90.0) && near(eye[0], -5.0) && near(eye[2], 0.0), "a quarter-height drag right puts the camera on -X: {eye:?}");
        spec.orbit(-300.0, 0.0, 100.0);
        assert!(near(spec.yaw, -90.0), "three turns later the yaw is back within -180 to 180: {}", spec.yaw);
        spec.orbit(0.0, 1000.0, 100.0);
        assert_eq!(spec.pitch, 90.0, "dragging down raises the camera, as far as straight down");
        let (x, y, _) = on_surface(&spec, [0.0; 3], 200.0, 100.0);
        assert!(near(x, 100.0) && near(y, 50.0), "looking straight down still shows the target at the center: {x}, {y}");
        assert!(spec.view().iter().flatten().all(|v| v.is_finite()));
    }

    #[test]
    fn a_pan_keeps_the_point_at_the_target_under_the_pointer() {
        for (yaw, pitch) in [(0.0, 0.0), (35.0, -20.0), (-120.0, 60.0), (10.0, 90.0)] {
            let mut spec = CameraSpec { yaw, pitch, target: [1.0, 2.0, -3.0], ..CameraSpec::default() };
            let held = spec.target;
            spec.pan(10.0, 20.0, 100.0);
            let (x, y, _) = on_surface(&spec, held, 200.0, 100.0);
            assert!(near(x, 110.0) && near(y, 70.0), "yaw {yaw}, pitch {pitch}: {x}, {y}");
            assert_eq!(spec.distance, 5.0);
        }
    }

    #[test]
    fn a_notch_toward_the_target_scales_the_distance_by_0_95_as_a_drag_100_points_up_does() {
        let mut spec = CameraSpec::default();
        spec.zoom(1.0);
        assert!((spec.distance - 4.75).abs() < 1e-12);
        spec.zoom(-1.0);
        assert!((spec.distance - 5.0).abs() < 1e-12);
        spec.drag(CameraMove::Zoom, 0.0, -100.0, 100.0);
        assert!((spec.distance - 4.75).abs() < 1e-12);
        assert!(CameraChange { pitch: Some(90.5), ..CameraChange::default() }.problem(&spec).is_some_and(|p| p.contains("pitch")));
    }
}
