//! A canvas's view: the point of its items' coordinates seen at the
//! canvas's middle and how near, the gestures and keys that move it, as one
//! of two presets or the session's own mapping, the change between a point
//! on the canvas and the items' coordinates, and the eased moves from one
//! view to another. Lengths on the canvas are in points.

use crate::Shortcut;
use crate::camera::{CameraButton, CameraModifiers};
use crate::canvas::{Easing, Rgba};
use serde::{Deserialize, Serialize};
use std::time::Instant;

/// A key a view's controls take while its canvas has the focus, named as a
/// shortcut names it ([`Shortcut`]): a press is the key its character is,
/// or the letter or digit at its place.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ViewKey {
    ArrowLeft,
    ArrowRight,
    ArrowUp,
    ArrowDown,
    /// The key typing +; Shift is not told apart, as many keyboards type +
    /// with it.
    Plus,
    /// The key typing -; Shift is not told apart.
    Minus,
    /// The key typing =; Shift is not told apart.
    Equals,
    Home,
    End,
    PageUp,
    PageDown,
    /// A digit, 0 to 9.
    Digit(u8),
    /// A letter, A to Z.
    Letter(u8),
    /// F1 to F24.
    F(u8),
}

impl ViewKey {
    /// The keys a word names, in the order a refusal lists them.
    const WORDS: [ViewKey; 11] = [
        ViewKey::ArrowLeft,
        ViewKey::ArrowRight,
        ViewKey::ArrowUp,
        ViewKey::ArrowDown,
        ViewKey::Plus,
        ViewKey::Minus,
        ViewKey::Equals,
        ViewKey::Home,
        ViewKey::End,
        ViewKey::PageUp,
        ViewKey::PageDown,
    ];

    /// The key `name` names, compared without regard to case, as a
    /// shortcut's key is written; none for a key a view's controls do not
    /// take.
    pub fn named(name: &str) -> Option<ViewKey> {
        let name = name.trim();
        if let Some(word) = ViewKey::WORDS.iter().find(|k| k.name().eq_ignore_ascii_case(name)) {
            return Some(*word);
        }
        if let Some(n) = (1..=24u8).find(|n| format!("F{n}").eq_ignore_ascii_case(name)) {
            return Some(ViewKey::F(n));
        }
        match name.as_bytes() {
            [d] if d.is_ascii_digit() => Some(ViewKey::Digit(d - b'0')),
            [l] if l.is_ascii_alphabetic() => Some(ViewKey::Letter(l.to_ascii_uppercase())),
            _ => None,
        }
    }

    /// The key's name, as a shortcut writes it.
    pub fn name(self) -> String {
        let word = match self {
            ViewKey::ArrowLeft => "ArrowLeft",
            ViewKey::ArrowRight => "ArrowRight",
            ViewKey::ArrowUp => "ArrowUp",
            ViewKey::ArrowDown => "ArrowDown",
            ViewKey::Plus => "Plus",
            ViewKey::Minus => "Minus",
            ViewKey::Equals => "Equals",
            ViewKey::Home => "Home",
            ViewKey::End => "End",
            ViewKey::PageUp => "PageUp",
            ViewKey::PageDown => "PageDown",
            ViewKey::Digit(d) => return d.to_string(),
            ViewKey::Letter(l) => return char::from(l).to_string(),
            ViewKey::F(n) => return format!("F{n}"),
        };
        word.to_string()
    }

    /// Whether a shortcut could name the key: a digit from 0 to 9, a letter
    /// from A to Z and F1 to F24.
    fn valid(self) -> bool {
        match self {
            ViewKey::Digit(d) => d <= 9,
            ViewKey::Letter(l) => l.is_ascii_uppercase(),
            ViewKey::F(n) => (1..=24).contains(&n),
            _ => true,
        }
    }

    /// Whether Shift is told apart for the key: not for +, - and =, which
    /// keyboards differ on typing with Shift.
    fn shift_told(self) -> bool {
        !matches!(self, ViewKey::Plus | ViewKey::Minus | ViewKey::Equals)
    }

    /// The way an arrow points, across and down; none for any other key.
    pub fn arrow(self) -> Option<[f64; 2]> {
        match self {
            ViewKey::ArrowLeft => Some([-1.0, 0.0]),
            ViewKey::ArrowRight => Some([1.0, 0.0]),
            ViewKey::ArrowUp => Some([0.0, -1.0]),
            ViewKey::ArrowDown => Some([0.0, 1.0]),
            _ => None,
        }
    }

    /// Whether Ctrl and the key zoom the window, so no shortcut or binding
    /// can take them: +, -, = and 0.
    fn window_zoom(self) -> bool {
        matches!(self, ViewKey::Plus | ViewKey::Minus | ViewKey::Equals | ViewKey::Digit(0))
    }
}

/// What the person does on a canvas that a view's controls can take.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ViewGesture {
    /// A button dragged with modifier keys pressed, and with `space` while
    /// the space bar is down.
    Drag { button: CameraButton, modifiers: CameraModifiers, space: bool },
    /// The wheel turned with modifier keys pressed.
    Wheel { modifiers: CameraModifiers },
    /// Two fingers pinched on a touch screen or trackpad.
    Pinch,
    /// A button double-clicked where no named item takes the double-click.
    DoubleClick { button: CameraButton, modifiers: CameraModifiers },
    /// A key pressed with modifier keys while the canvas has the focus.
    Key { key: ViewKey, modifiers: CameraModifiers },
}

impl ViewGesture {
    /// The gesture `text` names, compared without regard to case: Shift,
    /// Ctrl and Alt, and Space for a drag, joined by + to Left, Right or
    /// Middle (a drag), Wheel, Pinch, DoubleClick, RightDoubleClick or
    /// MiddleDoubleClick, or a key a view takes ([`ViewKey::named`]);
    /// Control, Command and Cmd are Ctrl, and Option is Alt.
    pub fn named(text: &str) -> Result<ViewGesture, String> {
        let parts: Vec<&str> = text.split('+').map(str::trim).collect();
        let Some((last, held)) = parts.split_last() else {
            return Err(format!("'{text}' names no gesture"));
        };
        let mut modifiers = CameraModifiers::NONE;
        let mut space = false;
        for key in held {
            match key.to_ascii_lowercase().as_str() {
                "shift" => modifiers.shift = true,
                "ctrl" | "control" | "command" | "cmd" => modifiers.ctrl = true,
                "alt" | "option" => modifiers.alt = true,
                "space" => space = true,
                _ => return Err(format!("'{text}': {key} is not a modifier key; a gesture's modifiers are Shift, Ctrl and Alt, and a drag's also Space")),
            }
        }
        let gesture = match last.to_ascii_lowercase().as_str() {
            "left" => return Ok(ViewGesture::Drag { button: CameraButton::Left, modifiers, space }),
            "right" => return Ok(ViewGesture::Drag { button: CameraButton::Right, modifiers, space }),
            "middle" => return Ok(ViewGesture::Drag { button: CameraButton::Middle, modifiers, space }),
            "wheel" => ViewGesture::Wheel { modifiers },
            "pinch" if modifiers != CameraModifiers::NONE => return Err(format!("'{text}': a pinch is made without modifier keys")),
            "pinch" => ViewGesture::Pinch,
            "doubleclick" => ViewGesture::DoubleClick { button: CameraButton::Left, modifiers },
            "rightdoubleclick" => ViewGesture::DoubleClick { button: CameraButton::Right, modifiers },
            "middledoubleclick" => ViewGesture::DoubleClick { button: CameraButton::Middle, modifiers },
            _ => match ViewKey::named(last) {
                Some(key) => ViewGesture::Key { key, modifiers },
                None => {
                    return Err(format!(
                        "'{text}' ends in {last}; a gesture ends in Left, Right, Middle, Wheel, Pinch, DoubleClick, RightDoubleClick or MiddleDoubleClick, or in a key: an arrow, Plus, Minus, Equals, Home, End, PageUp, PageDown, a digit, a letter or F1 to F24"
                    ));
                }
            },
        };
        if space {
            return Err(format!("'{text}': Space goes only with a drag, and {last} is not one"));
        }
        Ok(gesture)
    }

    /// The gesture as [`ViewGesture::named`] reads it, its modifier keys
    /// first in the order Shift, Ctrl, Alt, Space.
    pub fn written(self) -> String {
        let button = |b: CameraButton| match b {
            CameraButton::Left => "Left",
            CameraButton::Right => "Right",
            CameraButton::Middle => "Middle",
        };
        let (modifiers, space, last) = match self {
            ViewGesture::Drag { button: b, modifiers, space } => (modifiers, space, button(b).to_string()),
            ViewGesture::Wheel { modifiers } => (modifiers, false, String::from("Wheel")),
            ViewGesture::Pinch => (CameraModifiers::NONE, false, String::from("Pinch")),
            ViewGesture::DoubleClick { button: b, modifiers } => (modifiers, false, if b == CameraButton::Left { String::from("DoubleClick") } else { format!("{}DoubleClick", button(b)) }),
            ViewGesture::Key { key, modifiers } => (modifiers, false, key.name()),
        };
        let held = [(modifiers.shift, "Shift"), (modifiers.ctrl, "Ctrl"), (modifiers.alt, "Alt"), (space, "Space")];
        let mut parts: Vec<String> = held.iter().filter(|(on, _)| *on).map(|(_, name)| name.to_string()).collect();
        parts.push(last);
        parts.join("+")
    }

    /// The moves the gesture can make, as a refusal names them.
    fn makeable(self) -> &'static str {
        match self {
            ViewGesture::Drag { .. } => "a drag makes Pan, Glide, Zoom or BoxZoom",
            ViewGesture::Wheel { .. } => "the wheel makes Zoom, Scroll or ScrollAcross",
            ViewGesture::Pinch => "a pinch makes Zoom or Pan",
            ViewGesture::DoubleClick { .. } => "a double-click makes ZoomIn, ZoomOut, Fit or ActualSize",
            ViewGesture::Key { key, .. } if key.arrow().is_some() => "an arrow key makes Pan, ZoomIn, ZoomOut, Fit or ActualSize",
            ViewGesture::Key { .. } => "a key other than an arrow makes ZoomIn, ZoomOut, Fit or ActualSize",
        }
    }

    /// The key gesture a press makes, as a shortcut names the press; none
    /// for a key a view's controls do not take.
    pub fn of_press(pressed: &Shortcut) -> Option<ViewGesture> {
        let key = ViewKey::named(&pressed.key)?;
        Some(ViewGesture::Key { key, modifiers: CameraModifiers { shift: pressed.shift, ctrl: pressed.ctrl, alt: pressed.alt } })
    }

    /// The gesture as bindings are matched against it: Shift left out of a
    /// key that does not tell it apart.
    fn matched(self) -> ViewGesture {
        match self {
            ViewGesture::Key { key, modifiers } if !key.shift_told() => ViewGesture::Key { key, modifiers: CameraModifiers { shift: false, ..modifiers } },
            other => other,
        }
    }

    /// Whether the gesture can make `made`: a drag pans, glides, zooms or
    /// draws a box; the wheel zooms or scrolls; a pinch zooms or pans; a
    /// double-click and a key make the step moves, and an arrow key pans.
    pub fn makes(self, made: ViewMove) -> bool {
        match self {
            ViewGesture::Drag { .. } => matches!(made, ViewMove::Pan | ViewMove::Glide | ViewMove::Zoom | ViewMove::BoxZoom),
            ViewGesture::Wheel { .. } => matches!(made, ViewMove::Zoom | ViewMove::Scroll | ViewMove::ScrollAcross),
            ViewGesture::Pinch => matches!(made, ViewMove::Zoom | ViewMove::Pan),
            ViewGesture::DoubleClick { .. } => made.is_step(),
            ViewGesture::Key { key, .. } => made.is_step() || (made == ViewMove::Pan && key.arrow().is_some()),
        }
    }
}

/// How a gesture moves the view.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ViewMove {
    /// The items follow the pointer, or the fingers' middle; from an arrow
    /// key, the view goes [`KEY_STEP`] points toward the arrow, showing
    /// what lies that way, eased.
    Pan,
    /// As Pan, and a drag let go while it moves glides on and slows to a
    /// stop, as [`glide`] gives.
    Glide,
    /// Nearer or farther about the pointer: by the wheel's notches, the
    /// fingers' spread, or a drag, nearer up and farther down, a notch for
    /// each 100 points.
    Zoom,
    /// The view moves as the wheel moves a page's content, up and down, and
    /// sideways with a sideways turn, as a trackpad's.
    Scroll,
    /// The view moves sideways as the wheel turns.
    ScrollAcross,
    /// Nearer by [`ZOOM_IN_STEP`], about the pointer, or the canvas's
    /// middle from a key, eased.
    ZoomIn,
    /// Farther by [`ZOOM_IN_STEP`], likewise.
    ZoomOut,
    /// The drag draws a box over the items, and the view fits it, eased,
    /// when the button is let go; Escape lets the box go.
    BoxZoom,
    /// The view fits every item, eased.
    Fit,
    /// Zoom 1, a point on the canvas for each unit of the items'
    /// coordinates, about the pointer, or the canvas's middle from a key,
    /// eased.
    ActualSize,
}

impl ViewMove {
    /// Every move, in the order a refusal lists them.
    pub const ALL: [ViewMove; 10] = [
        ViewMove::Pan,
        ViewMove::Glide,
        ViewMove::Zoom,
        ViewMove::Scroll,
        ViewMove::ScrollAcross,
        ViewMove::ZoomIn,
        ViewMove::ZoomOut,
        ViewMove::BoxZoom,
        ViewMove::Fit,
        ViewMove::ActualSize,
    ];

    /// The move's name, as a script writes it.
    pub fn name(self) -> &'static str {
        match self {
            ViewMove::Pan => "Pan",
            ViewMove::Glide => "Glide",
            ViewMove::Zoom => "Zoom",
            ViewMove::Scroll => "Scroll",
            ViewMove::ScrollAcross => "ScrollAcross",
            ViewMove::ZoomIn => "ZoomIn",
            ViewMove::ZoomOut => "ZoomOut",
            ViewMove::BoxZoom => "BoxZoom",
            ViewMove::Fit => "Fit",
            ViewMove::ActualSize => "ActualSize",
        }
    }

    /// The move `text` names, compared without regard to case.
    pub fn named(text: &str) -> Result<ViewMove, String> {
        ViewMove::ALL.iter().copied().find(|m| m.name().eq_ignore_ascii_case(text.trim())).ok_or_else(|| {
            let names: Vec<&str> = ViewMove::ALL.iter().map(|m| m.name()).collect();
            format!("'{text}' is no move of a view; a gesture makes {}", names.join(", "))
        })
    }

    /// Whether it is a step, made at once by a double-click or a key and
    /// eased over [`STEP_SECONDS`].
    fn is_step(self) -> bool {
        matches!(self, ViewMove::ZoomIn | ViewMove::ZoomOut | ViewMove::Fit | ViewMove::ActualSize)
    }
}

/// Which gestures move the view, and how.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum ViewControls {
    /// As Leaflet and Google Maps: a left drag where no named item takes
    /// the drag pans and glides on when let go moving, Shift and a left drag
    /// there draws a box to zoom to, the wheel, Ctrl and the wheel (as a
    /// trackpad's pinch reaches Windows and browsers) and a pinch zoom
    /// about the pointer, a double-click on empty canvas zooms in a step
    /// and with Shift out, the arrows pan, and + and = zoom in a step and -
    /// out.
    #[default]
    Map,
    /// As Figma and Inkscape: the wheel scrolls, Shift and the wheel scroll
    /// sideways, Ctrl and the wheel or a pinch zoom about the pointer, a
    /// middle drag or a left drag with the space bar down pans, + and = zoom
    /// in a step and - out, Shift+1 fits every item, Shift+0 returns to
    /// zoom 1, and Ctrl and the arrows pan, plain arrows being the scene's.
    Design,
    /// The session's own: each gesture and the move it makes; where several
    /// match the person's gesture, the first is used.
    Custom(Vec<(ViewGesture, ViewMove)>),
    /// No gesture: the view moves only when the session sets it.
    None,
}

impl ViewControls {
    /// Each gesture these controls take and the move it makes, in the order
    /// they are matched.
    pub fn bindings(&self) -> Vec<(ViewGesture, ViewMove)> {
        let drag = |button, modifiers, space| ViewGesture::Drag { button, modifiers, space };
        let wheel = |modifiers| ViewGesture::Wheel { modifiers };
        let double = |modifiers| ViewGesture::DoubleClick { button: CameraButton::Left, modifiers };
        let key = |key, modifiers| ViewGesture::Key { key, modifiers };
        let arrows = |modifiers| [ViewKey::ArrowLeft, ViewKey::ArrowRight, ViewKey::ArrowUp, ViewKey::ArrowDown].map(|a| (key(a, modifiers), ViewMove::Pan));
        let steps = [(key(ViewKey::Plus, CameraModifiers::NONE), ViewMove::ZoomIn), (key(ViewKey::Equals, CameraModifiers::NONE), ViewMove::ZoomIn), (key(ViewKey::Minus, CameraModifiers::NONE), ViewMove::ZoomOut)];
        match self {
            ViewControls::Map => [
                (drag(CameraButton::Left, CameraModifiers::NONE, false), ViewMove::Glide),
                (drag(CameraButton::Left, CameraModifiers::SHIFT, false), ViewMove::BoxZoom),
                (wheel(CameraModifiers::NONE), ViewMove::Zoom),
                (wheel(CameraModifiers::CTRL), ViewMove::Zoom),
                (ViewGesture::Pinch, ViewMove::Zoom),
                (double(CameraModifiers::NONE), ViewMove::ZoomIn),
                (double(CameraModifiers::SHIFT), ViewMove::ZoomOut),
            ]
            .into_iter()
            .chain(arrows(CameraModifiers::NONE))
            .chain(steps)
            .collect(),
            ViewControls::Design => [
                (wheel(CameraModifiers::NONE), ViewMove::Scroll),
                (wheel(CameraModifiers::SHIFT), ViewMove::ScrollAcross),
                (wheel(CameraModifiers::CTRL), ViewMove::Zoom),
                (ViewGesture::Pinch, ViewMove::Zoom),
                (drag(CameraButton::Middle, CameraModifiers::NONE, false), ViewMove::Pan),
                (drag(CameraButton::Left, CameraModifiers::NONE, true), ViewMove::Pan),
            ]
            .into_iter()
            .chain(steps)
            .chain([(key(ViewKey::Digit(1), CameraModifiers::SHIFT), ViewMove::Fit), (key(ViewKey::Digit(0), CameraModifiers::SHIFT), ViewMove::ActualSize)])
            .chain(arrows(CameraModifiers::CTRL))
            .collect(),
            ViewControls::Custom(bindings) => bindings.clone(),
            ViewControls::None => Vec::new(),
        }
    }

    /// The move `gesture` makes under these controls, if any.
    pub fn move_for(&self, gesture: ViewGesture) -> Option<ViewMove> {
        let gesture = gesture.matched();
        self.bindings().into_iter().find(|(bound, _)| bound.matched() == gesture).map(|(_, made)| made)
    }

    /// Whether these controls take the press `pressed` from the canvas's
    /// key handler: a key they bind, or Space without Ctrl or Alt while
    /// they bind a drag with the space bar down.
    pub fn keeps(&self, pressed: &Shortcut) -> bool {
        if pressed.key == "Space" && !pressed.ctrl && !pressed.alt {
            return self.bindings().iter().any(|(g, _)| matches!(g, ViewGesture::Drag { space: true, .. }));
        }
        ViewGesture::of_press(pressed).is_some_and(|g| self.move_for(g).is_some())
    }

    /// Whether any binding takes a key, so the canvas takes the focus.
    pub fn binds_keys(&self) -> bool {
        self.bindings().iter().any(|(g, _)| matches!(g, ViewGesture::Key { .. } | ViewGesture::Drag { space: true, .. }))
    }

    /// Why these controls cannot be a view's, or `None`: a custom binding
    /// whose gesture cannot make its move, two bindings of one gesture, a
    /// key no shortcut names, Shift with +, - or =, which are matched
    /// without it, or Ctrl with +, -, = or 0, which zoom the window.
    pub fn problem(&self) -> Option<String> {
        let ViewControls::Custom(bindings) = self else {
            return None;
        };
        for (at, (gesture, made)) in bindings.iter().enumerate() {
            if let ViewGesture::Key { key, modifiers } = gesture {
                if !key.valid() {
                    return Some(format!("{key:?} is no key a shortcut names"));
                }
                if modifiers.shift && !key.shift_told() {
                    return Some(format!("{} is {} alone, which keyboards type with Shift or without", gesture.written(), key.name()));
                }
                if modifiers.ctrl && key.window_zoom() {
                    return Some(format!("{} zooms the window, so no binding takes it", gesture.written()));
                }
            }
            if !gesture.makes(*made) {
                return Some(format!("{} cannot make {}: {}", gesture.written(), made.name(), gesture.makeable()));
            }
            if bindings[..at].iter().any(|(earlier, _)| earlier.matched() == gesture.matched()) {
                return Some(format!("{} is bound twice", gesture.written()));
            }
        }
        None
    }
}

/// The factor one wheel notch nearer scales the zoom by, a notch being a
/// line of the wheel or 100 points of a trackpad's scroll: the step the
/// image viewer's HTML side, OpenSeadragon, zooms by, so a canvas and an
/// image viewer zoom alike.
pub const ZOOM_STEP: f64 = 1.2;

/// How much nearer a ZoomIn goes and farther a ZoomOut, a zoom level of
/// Leaflet's.
pub const ZOOM_IN_STEP: f64 = 2.0;

/// How far one wheel notch scrolls, in points on the canvas: the 100 a
/// browser scrolls a notch by, so a trackpad's scroll, counted in points,
/// moves the items with the fingers.
pub const SCROLL_STEP: f64 = 100.0;

/// How far an arrow key pans, in points on the canvas, as Leaflet's
/// keyboard pans.
pub const KEY_STEP: f64 = 80.0;

/// How long a step's move takes, and along which curve: Leaflet's zoom
/// animation.
pub const STEP_SECONDS: f64 = 0.25;
pub const STEP_EASING: Easing = Easing::EaseOut;

/// How a glide is measured and runs, as Leaflet's inertia: the pointer's
/// places over the last 50 ms of the drag give its speed, and the glide
/// slows at 3000 points a second each second, scaled by Leaflet's ease
/// linearity, along the curve that linearity makes.
pub const GLIDE_WINDOW: f64 = 0.05;
pub const GLIDE_DECELERATION: f64 = 3000.0;
pub const GLIDE_LINEARITY: f64 = 0.2;
pub const GLIDE_EASING: Easing = Easing::EaseOutQuint;

/// How a box zoom's box is drawn over the items, as Leaflet draws it: a
/// dotted border 2 points wide and a half-clear white inside.
pub const BOX_BORDER: Rgba = Rgba([0x33, 0x88, 0xff, 0xff]);
pub const BOX_BORDER_WIDTH: f64 = 2.0;
pub const BOX_FILL: Rgba = Rgba([0xff, 0xff, 0xff, 0x80]);

/// The velocity, in points a second across and down, of a drag let go at
/// `released` seconds, from its pointer's places `samples`, each a time in
/// seconds and a place, oldest first: the move over the places within
/// [`GLIDE_WINDOW`] of the release, as Leaflet measures it; none from fewer
/// than two such places or no time between them, as when the pointer
/// rested before the release.
pub fn release_velocity(samples: &[(f64, [f64; 2])], released: f64) -> Option<[f64; 2]> {
    let recent: Vec<&(f64, [f64; 2])> = samples.iter().filter(|(t, _)| released - t <= GLIDE_WINDOW).collect();
    let (first, last) = (recent.first()?, recent.last()?);
    let span = last.0 - first.0;
    if recent.len() < 2 || span <= 0.0 {
        return None;
    }
    Some([(last.1[0] - first.1[0]) / span, (last.1[1] - first.1[1]) / span])
}

/// The glide after a pan let go at `velocity` points a second, as Leaflet's
/// inertia goes: how far the items go on, across and down, and in how many
/// seconds they stop, along [`GLIDE_EASING`]; none when they would go less
/// than half a point either way.
pub fn glide(velocity: [f64; 2]) -> Option<([f64; 2], f64)> {
    let speed = velocity[0].hypot(velocity[1]);
    if !speed.is_finite() || speed <= 0.0 {
        return None;
    }
    let seconds = speed / GLIDE_DECELERATION;
    let reach = GLIDE_LINEARITY * speed * seconds / 2.0;
    let by = [velocity[0] / speed * reach, velocity[1] / speed * reach];
    if by[0].abs() < 0.5 && by[1].abs() < 0.5 {
        return None;
    }
    Some((by, seconds))
}

/// A canvas's view: the point of its items' coordinates at the canvas's
/// middle, none for the canvas's own middle while the items' coordinates
/// are the canvas's; how many points on the canvas one unit of them takes;
/// the nearest and farthest it may go; and its controls.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ViewSpec {
    pub center: Option<[f64; 2]>,
    pub zoom: f64,
    pub min_zoom: f64,
    pub max_zoom: f64,
    pub controls: ViewControls,
}

impl Default for ViewSpec {
    /// The view a canvas has without one: its items' coordinates the
    /// canvas's own, under the Map controls, any zoom allowed.
    fn default() -> Self {
        ViewSpec { center: None, zoom: 1.0, min_zoom: f64::MIN_POSITIVE, max_zoom: f64::INFINITY, controls: ViewControls::Map }
    }
}

impl ViewSpec {
    /// The point of the items' coordinates at the middle of a canvas `size`
    /// points across and down.
    pub fn center_in(&self, size: [f64; 2]) -> [f64; 2] {
        match self.center {
            Some(c) => c,
            None => [size[0] / 2.0, size[1] / 2.0],
        }
    }

    /// The transform from the items' coordinates to points on a canvas
    /// `size` points across and down: `x' = a x + e`, `y' = d y + f`, as
    /// `[a, b, c, d, e, f]`.
    pub fn transform(&self, size: [f64; 2]) -> [f64; 6] {
        let (z, c) = (self.zoom, self.center_in(size));
        [z, 0.0, 0.0, z, size[0] / 2.0 - c[0] * z, size[1] / 2.0 - c[1] * z]
    }

    /// The point of the items' coordinates at `at`, in points on a canvas
    /// `size` points across and down.
    pub fn to_items(&self, at: [f64; 2], size: [f64; 2]) -> [f64; 2] {
        let c = self.center_in(size);
        [(at[0] - size[0] / 2.0) / self.zoom + c[0], (at[1] - size[1] / 2.0) / self.zoom + c[1]]
    }

    /// Moves the view so the items follow a drag `across` and `down` points
    /// on a canvas `size` points across and down.
    pub fn pan(&mut self, across: f64, down: f64, size: [f64; 2]) {
        let c = self.center_in(size);
        self.center = Some([c[0] - across / self.zoom, c[1] - down / self.zoom]);
    }

    /// Scales the zoom by `factor`, within the view's nearest and farthest,
    /// leaving the point under `at` where it is on a canvas `size` points
    /// across and down.
    pub fn zoom_about(&mut self, factor: f64, at: [f64; 2], size: [f64; 2]) {
        let before = self.to_items(at, size);
        self.zoom = (self.zoom * factor).clamp(self.min_zoom, self.max_zoom);
        let after = self.to_items(at, size);
        let c = self.center_in(size);
        self.center = Some([c[0] + before[0] - after[0], c[1] + before[1] - after[1]]);
    }

    /// Makes `made` for a wheel turn that moves a page's content `by`
    /// points across and down, positive down when the wheel turns away from
    /// the person, at `at` on a canvas `size` points across and down: a
    /// zoom of [`ZOOM_STEP`] nearer for each [`SCROLL_STEP`] points, or a
    /// scroll moving the view as the content would move, sideways for
    /// ScrollAcross; a move the wheel does not make leaves the view.
    pub fn wheel(&mut self, made: ViewMove, by: [f64; 2], at: [f64; 2], size: [f64; 2]) {
        match made {
            ViewMove::Zoom => self.zoom_about(ZOOM_STEP.powf((by[0] + by[1]) / SCROLL_STEP), at, size),
            ViewMove::Scroll => self.pan(by[0], by[1], size),
            ViewMove::ScrollAcross => self.pan(by[0] + by[1], 0.0, size),
            _ => {}
        }
    }

    /// Makes a drag's `made` for a move of the pointer `across` and `down`
    /// points from `from`, on a canvas `size` points across and down: a pan
    /// or a glide follows it, and a zoom goes a notch nearer for each 100
    /// points up, about where the drag began.
    pub fn dragged(&mut self, made: ViewMove, across: f64, down: f64, from: [f64; 2], size: [f64; 2]) {
        match made {
            ViewMove::Pan | ViewMove::Glide => self.pan(across, down, size),
            ViewMove::Zoom => self.zoom_about(ZOOM_STEP.powf(-down / SCROLL_STEP), from, size),
            _ => {}
        }
    }

    /// The view a step `made` ends at, from a press at `at`, in points on a
    /// canvas `size` points across and down, or from a press of `key`: a
    /// zoom in or out a step or to zoom 1 about `at`, a fit to `fit`, the
    /// box of every item in the items' coordinates, or an arrow's pan; none
    /// for a move that is no step, or a fit with nothing to fit.
    pub fn stepped(&self, made: ViewMove, at: [f64; 2], key: Option<ViewKey>, fit: Option<[f64; 4]>, size: [f64; 2]) -> Option<ViewSpec> {
        let mut to = self.clone();
        match made {
            ViewMove::ZoomIn => to.zoom_about(ZOOM_IN_STEP, at, size),
            ViewMove::ZoomOut => to.zoom_about(1.0 / ZOOM_IN_STEP, at, size),
            ViewMove::ActualSize => to.zoom_about(1.0 / self.zoom, at, size),
            ViewMove::Fit => to.fit(fit?, size),
            ViewMove::Pan => {
                let [across, down] = key?.arrow()?;
                to.pan(-across * KEY_STEP, -down * KEY_STEP, size);
            }
            _ => return None,
        }
        Some(to)
    }

    /// Centers the view on `bounds`, `[x, y, width, height]` in the items'
    /// coordinates, as near as shows all of it on a canvas `size` points
    /// across and down, within the view's nearest and farthest.
    pub fn fit(&mut self, bounds: [f64; 4], size: [f64; 2]) {
        let [x, y, w, h] = bounds;
        self.center = Some([x + w / 2.0, y + h / 2.0]);
        let across = if w > 0.0 { size[0] / w } else { f64::INFINITY };
        let down = if h > 0.0 { size[1] / h } else { f64::INFINITY };
        let zoom = across.min(down);
        if zoom.is_finite() {
            self.zoom = zoom.clamp(self.min_zoom, self.max_zoom);
        }
    }

    /// The view a box zoom ends at, for a box from `a` to `b`, in points on
    /// a canvas `size` points across and down: fitted to what the box
    /// covers; none for a box under a point both ways.
    pub fn boxed(&self, a: [f64; 2], b: [f64; 2], size: [f64; 2]) -> Option<ViewSpec> {
        if (a[0] - b[0]).abs() < 1.0 && (a[1] - b[1]).abs() < 1.0 {
            return None;
        }
        let (low, high) = (self.to_items([a[0].min(b[0]), a[1].min(b[1])], size), self.to_items([a[0].max(b[0]), a[1].max(b[1])], size));
        let mut to = self.clone();
        to.fit([low[0], low[1], high[0] - low[0], high[1] - low[1]], size);
        Some(to)
    }

    /// The view `share` of the way from this one to `to`, `share` from 0 to
    /// 1 as an easing gives it, on a canvas `size` points across and down:
    /// the zoom moves by equal factors, and the point the two views put at
    /// the same place on the canvas stays there throughout, so a zoom about
    /// a point leaves that point still and a move at one zoom goes straight;
    /// the parts other than the center and zoom are `to`'s.
    pub fn toward(&self, to: &ViewSpec, share: f64, size: [f64; 2]) -> ViewSpec {
        let (from_center, to_center) = (self.center_in(size), to.center_in(size));
        let ratio = (to.zoom / self.zoom).ln();
        let weight = if ratio.abs() < 1e-12 { share } else { (-share * ratio).exp_m1() / (-ratio).exp_m1() };
        ViewSpec {
            center: Some([from_center[0] + (to_center[0] - from_center[0]) * weight, from_center[1] + (to_center[1] - from_center[1]) * weight]),
            zoom: self.zoom * (share * ratio).exp(),
            ..to.clone()
        }
    }
}

/// How a change moves the view: over `seconds` along `easing`, from the
/// view drawn when it is made.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct ViewEase {
    pub seconds: f64,
    pub easing: Easing,
}

/// A move from one view to another under way: where it begins and ends,
/// when it began, and how long it takes along which curve.
#[derive(Clone, Debug, PartialEq)]
pub struct ViewMotion {
    pub from: ViewSpec,
    pub to: ViewSpec,
    pub began: Instant,
    pub ease: ViewEase,
}

impl ViewMotion {
    /// The view drawn at `now` on a canvas `size` points across and down,
    /// `to` once the move has run its seconds.
    pub fn at(&self, now: Instant, size: [f64; 2]) -> ViewSpec {
        if !self.moving(now) {
            return self.to.clone();
        }
        let t = now.saturating_duration_since(self.began).as_secs_f64() / self.ease.seconds;
        self.from.toward(&self.to, self.ease.easing.at(t), size)
    }

    /// Whether the move is still under way at `now`.
    pub fn moving(&self, now: Instant) -> bool {
        now.saturating_duration_since(self.began).as_secs_f64() < self.ease.seconds
    }
}

/// A canvas's view as a call returns it: as drawn when the call returns,
/// and whether a move or a glide is under way, so the view will go on
/// changing.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ViewShown {
    pub view: ViewSpec,
    pub moving: bool,
}

/// What a view fits itself to.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ViewFit {
    /// Every item.
    All,
    /// The items these names name.
    Items(Vec<String>),
}

/// The parts of a view a call changes; a part left `None` stays as the view
/// has it, or as [`ViewSpec::default`] gives it to a canvas without one. A
/// fit is made after the other parts, so it sets the center and the zoom.
/// With `ease` the view moves to the change over its seconds, from the
/// view drawn when the change is made; a canvas not yet drawn takes the
/// change at once.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ViewChange {
    pub center: Option<[f64; 2]>,
    pub zoom: Option<f64>,
    pub min_zoom: Option<f64>,
    pub max_zoom: Option<f64>,
    pub controls: Option<ViewControls>,
    pub fit: Option<ViewFit>,
    pub ease: Option<ViewEase>,
}

impl ViewChange {
    /// `spec` with this change's parts in place of its own; the fit is left
    /// to the renderer, which has the items' places.
    pub fn applied_to(&self, spec: &ViewSpec) -> ViewSpec {
        ViewSpec {
            center: self.center.or(spec.center),
            zoom: self.zoom.unwrap_or(spec.zoom),
            min_zoom: self.min_zoom.unwrap_or(spec.min_zoom),
            max_zoom: self.max_zoom.unwrap_or(spec.max_zoom),
            controls: self.controls.clone().unwrap_or_else(|| spec.controls.clone()),
        }
    }

    /// Why this change cannot make a view, naming the part, or `None`.
    pub fn problem(&self, spec: &ViewSpec) -> Option<String> {
        let made = self.applied_to(spec);
        if !(made.center.iter().flatten().all(|v| v.is_finite()) && made.zoom.is_finite()) {
            return Some(String::from("a view's center and zoom are finite numbers"));
        }
        if made.min_zoom <= 0.0 || made.max_zoom < made.min_zoom || made.min_zoom.is_nan() || made.max_zoom.is_nan() {
            return Some(format!("MinZoom would be {} and MaxZoom {}, and a view's MinZoom is greater than 0 and its MaxZoom no less than it", made.min_zoom, made.max_zoom));
        }
        if !(made.min_zoom..=made.max_zoom).contains(&made.zoom) {
            return Some(format!("the zoom would be {}, outside MinZoom {} and MaxZoom {}", made.zoom, made.min_zoom, made.max_zoom));
        }
        if let Some(ease) = self.ease
            && !(ease.seconds.is_finite() && ease.seconds > 0.0)
        {
            return Some(format!("a move takes more than 0 seconds, and this one {}", ease.seconds));
        }
        made.controls.problem()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn near(a: [f64; 2], b: [f64; 2]) -> bool {
        (a[0] - b[0]).abs() < 1e-9 && (a[1] - b[1]).abs() < 1e-9
    }

    fn press(written: &str) -> Shortcut {
        Shortcut::parse(written).expect("a shortcut")
    }

    #[test]
    fn each_preset_makes_the_moves_its_namesake_does() {
        let drag = |button, modifiers, space| ViewGesture::Drag { button, modifiers, space };
        let wheel = |modifiers| ViewGesture::Wheel { modifiers };
        let double = |modifiers| ViewGesture::DoubleClick { button: CameraButton::Left, modifiers };
        let key = |written: &str| ViewGesture::of_press(&press(written)).expect("a key a view takes");
        let map = ViewControls::Map;
        assert_eq!(map.move_for(drag(CameraButton::Left, CameraModifiers::NONE, false)), Some(ViewMove::Glide));
        assert_eq!(map.move_for(drag(CameraButton::Left, CameraModifiers::SHIFT, false)), Some(ViewMove::BoxZoom));
        assert_eq!(map.move_for(wheel(CameraModifiers::NONE)), Some(ViewMove::Zoom));
        assert_eq!(map.move_for(wheel(CameraModifiers::CTRL)), Some(ViewMove::Zoom), "a trackpad's pinch reaches Windows as the wheel with Ctrl");
        assert_eq!(map.move_for(double(CameraModifiers::NONE)), Some(ViewMove::ZoomIn));
        assert_eq!(map.move_for(double(CameraModifiers::SHIFT)), Some(ViewMove::ZoomOut));
        assert_eq!(map.move_for(key("ArrowLeft")), Some(ViewMove::Pan));
        assert_eq!(map.move_for(key("Shift+Plus")), Some(ViewMove::ZoomIn), "+ typed with Shift is +");
        assert_eq!(map.move_for(key("Equals")), Some(ViewMove::ZoomIn));
        assert_eq!(map.move_for(key("Minus")), Some(ViewMove::ZoomOut));
        assert!(!map.keeps(&press("Space")), "Map leaves Space to the scene");
        let design = ViewControls::Design;
        assert_eq!(design.move_for(wheel(CameraModifiers::NONE)), Some(ViewMove::Scroll));
        assert_eq!(design.move_for(wheel(CameraModifiers::SHIFT)), Some(ViewMove::ScrollAcross));
        assert_eq!(design.move_for(wheel(CameraModifiers::CTRL)), Some(ViewMove::Zoom));
        assert_eq!(design.move_for(drag(CameraButton::Left, CameraModifiers::NONE, true)), Some(ViewMove::Pan), "space and a left drag pan");
        assert_eq!(design.move_for(drag(CameraButton::Left, CameraModifiers::NONE, false)), None, "a plain left drag is the scene's");
        assert_eq!(design.move_for(key("Shift+1")), Some(ViewMove::Fit));
        assert_eq!(design.move_for(key("Shift+0")), Some(ViewMove::ActualSize));
        assert_eq!(design.move_for(key("Ctrl+ArrowUp")), Some(ViewMove::Pan));
        assert_eq!(design.move_for(key("ArrowUp")), None, "plain arrows are the scene's");
        assert!(design.keeps(&press("Space")) && design.keeps(&press("Shift+1")) && !design.keeps(&press("Delete")), "Design keeps Space for its drag and its keys, and leaves the rest");
        assert!(design.binds_keys() && map.binds_keys() && !ViewControls::None.binds_keys());
        assert_eq!(ViewControls::None.move_for(ViewGesture::Pinch), None);
    }

    #[test]
    fn a_gesture_and_a_move_read_as_written_in_any_case_and_write_back_in_one() {
        for written in ["Left", "Shift+Space+Left", "Ctrl+Wheel", "Pinch", "DoubleClick", "Shift+RightDoubleClick", "ArrowLeft", "Ctrl+ArrowUp", "Shift+1", "F", "Alt+F7"] {
            let gesture = ViewGesture::named(written).expect("a gesture");
            assert_eq!(gesture.written(), written);
        }
        assert_eq!(ViewGesture::named("space+cmd+left").map(ViewGesture::written), Ok(String::from("Ctrl+Space+Left")), "Cmd is Ctrl, and the modifiers are written in one order");
        assert!(ViewGesture::named("Ctrl+Pinch").is_err_and(|e| e.contains("without modifier keys")));
        assert!(ViewGesture::named("Space+Wheel").is_err_and(|e| e.contains("only with a drag")));
        assert!(ViewGesture::named("Hyper+Left").is_err_and(|e| e.contains("not a modifier key")));
        assert!(ViewGesture::named("Shift+Tab").is_err_and(|e| e.contains("ends in Tab")));
        assert_eq!(ViewMove::named("scrollacross"), Ok(ViewMove::ScrollAcross));
        assert!(ViewMove::named("Spin").is_err_and(|e| e.contains("Pan, Glide, Zoom")));
        let wheel_step = ViewControls::Custom(vec![(ViewGesture::named("Wheel").expect("a gesture"), ViewMove::ZoomIn)]).problem();
        assert_eq!(wheel_step.as_deref(), Some("Wheel cannot make ZoomIn: the wheel makes Zoom, Scroll or ScrollAcross"));
    }

    #[test]
    fn a_key_is_named_as_a_shortcut_names_it() {
        for written in ["ArrowLeft", "Plus", "Minus", "Equals", "PageDown", "7", "F", "F12"] {
            let key = ViewKey::named(written).expect("a key a view takes");
            assert_eq!(key.name(), written);
        }
        assert_eq!(ViewKey::named("pageup"), Some(ViewKey::PageUp));
        assert_eq!(ViewKey::named("q"), Some(ViewKey::Letter(b'Q')));
        assert_eq!(ViewKey::named("Space"), None, "Space is a drag's, not a key binding's");
        assert_eq!(ViewKey::named("F25"), None);
        assert_eq!(ViewKey::ArrowUp.arrow(), Some([0.0, -1.0]));
    }

    #[test]
    fn a_view_maps_points_both_ways_and_zooms_about_the_pointer() {
        let size = [200.0, 100.0];
        let mut view = ViewSpec::default();
        assert!(near(view.to_items([30.0, 40.0], size), [30.0, 40.0]), "a view with no center leaves the items' coordinates the canvas's");
        view.pan(20.0, -10.0, size);
        assert!(near(view.to_items([50.0, 30.0], size), [30.0, 40.0]), "the items follow a drag");
        let under = view.to_items([150.0, 60.0], size);
        view.zoom_about(2.0, [150.0, 60.0], size);
        assert!(near(view.to_items([150.0, 60.0], size), under), "the point under the pointer stays there");
        assert_eq!(view.zoom, 2.0);
        let t = view.transform(size);
        let item = [12.0, -7.0];
        let on_canvas = [t[0] * item[0] + t[4], t[3] * item[1] + t[5]];
        assert!(near(view.to_items(on_canvas, size), item), "the transform and to_items undo each other");
        view.max_zoom = 3.0;
        view.zoom_about(10.0, [0.0, 0.0], size);
        assert_eq!(view.zoom, 3.0, "stopped at MaxZoom");
        view.fit([0.0, 0.0, 400.0, 100.0], size);
        assert_eq!((view.center, view.zoom), (Some([200.0, 50.0]), 0.5), "fits the wider side");
        view.wheel(ViewMove::Scroll, [0.0, 2.0 * SCROLL_STEP], [100.0, 50.0], size);
        assert!(near(view.center_in(size), [200.0, 50.0 - 2.0 * SCROLL_STEP / 0.5]), "two notches away show what is above");
        view.wheel(ViewMove::ScrollAcross, [0.0, SCROLL_STEP], [100.0, 50.0], size);
        assert!(near(view.center_in(size), [200.0 - SCROLL_STEP / 0.5, 50.0 - 2.0 * SCROLL_STEP / 0.5]), "and sideways, what is to the left");
        let before = view.zoom;
        view.wheel(ViewMove::Zoom, [0.0, SCROLL_STEP], [100.0, 50.0], size);
        assert!((view.zoom - before * ZOOM_STEP).abs() < 1e-9, "a notch away zooms a step nearer");
        view.dragged(ViewMove::Zoom, 0.0, 2.0 * SCROLL_STEP, [100.0, 50.0], size);
        assert!((view.zoom - before / ZOOM_STEP).abs() < 1e-9, "a drag 200 points down zooms two notches farther");
    }

    #[test]
    fn a_step_ends_where_its_move_goes_and_a_box_fits_what_it_covers() {
        let size = [200.0, 100.0];
        let view = ViewSpec { center: Some([0.0, 0.0]), ..ViewSpec::default() };
        let zoomed = view.stepped(ViewMove::ZoomIn, [150.0, 50.0], None, None, size).expect("a step");
        assert_eq!(zoomed.zoom, ZOOM_IN_STEP);
        assert!(near(zoomed.to_items([150.0, 50.0], size), view.to_items([150.0, 50.0], size)), "about the pointer");
        let back = zoomed.stepped(ViewMove::ActualSize, [150.0, 50.0], None, None, size).expect("a step");
        assert!((back.zoom - 1.0).abs() < 1e-12);
        let panned = view.stepped(ViewMove::Pan, [100.0, 50.0], Some(ViewKey::ArrowRight), None, size).expect("an arrow's pan");
        assert!(near(panned.center_in(size), [KEY_STEP, 0.0]), "the right arrow shows what lies to the right");
        assert_eq!(view.stepped(ViewMove::Pan, [100.0, 50.0], Some(ViewKey::Home), None, size), None, "only an arrow pans");
        assert_eq!(view.stepped(ViewMove::Fit, [100.0, 50.0], None, None, size), None, "nothing to fit");
        let boxed = view.boxed([100.0, 50.0], [150.0, 75.0], size).expect("a box");
        assert!(near(boxed.center_in(size), [25.0, 12.5]) && (boxed.zoom - 4.0).abs() < 1e-12, "a box a quarter of the canvas zooms four times: {boxed:?}");
        assert_eq!(view.boxed([10.0, 10.0], [10.5, 10.2], size), None, "a click draws no box");
    }

    #[test]
    fn a_move_between_two_views_keeps_still_the_point_both_put_in_one_place() {
        let size = [200.0, 100.0];
        let from = ViewSpec { center: Some([0.0, 0.0]), ..ViewSpec::default() };
        let mut to = from.clone();
        to.zoom_about(8.0, [150.0, 25.0], size);
        let fixed = from.to_items([150.0, 25.0], size);
        for share in [0.0, 0.25, 0.5, 0.9, 1.0, 1.1] {
            let between = from.toward(&to, share, size);
            assert!(near(between.to_items([150.0, 25.0], size), fixed), "share {share}: {between:?}");
        }
        assert!((from.toward(&to, 0.5, size).zoom - 8f64.sqrt()).abs() < 1e-9, "halfway is the zoom's geometric middle");
        assert!(near(from.toward(&to, 1.0, size).center_in(size), to.center_in(size)));
        let mut panned = from.clone();
        panned.pan(-100.0, 0.0, size);
        assert!(near(from.toward(&panned, 0.25, size).center_in(size), [25.0, 0.0]), "a move at one zoom goes straight");
        let motion = ViewMotion { from: from.clone(), to: to.clone(), began: Instant::now(), ease: ViewEase { seconds: 0.25, easing: Easing::Linear } };
        let later = motion.began + std::time::Duration::from_millis(300);
        assert!(!motion.moving(later) && motion.at(later, size) == to, "a move over its seconds stays at its end");
        assert!(motion.moving(motion.began));
    }

    #[test]
    fn a_glide_goes_on_as_far_as_leaflets_inertia_and_not_after_a_rest() {
        let (by, seconds) = glide([1000.0, 0.0]).expect("a glide");
        assert!((seconds - 1.0 / 3.0).abs() < 1e-12 && (by[0] - 100.0 / 3.0).abs() < 1e-9 && by[1] == 0.0, "{by:?} over {seconds}");
        assert_eq!(glide([50.0, 0.0]), None, "too slow to go half a point");
        let samples = [(0.0, [0.0, 0.0]), (0.96875, [75.0, 0.0]), (1.0, [100.0, 0.0])];
        assert_eq!(release_velocity(&samples, 1.0), Some([800.0, 0.0]), "the last 50 ms give the speed");
        assert_eq!(release_velocity(&samples, 1.25), None, "a pointer that rested before the release glides nowhere");
    }

    #[test]
    fn a_change_keeps_the_parts_it_leaves_out_and_refuses_a_view_that_cannot_be() {
        let spec = ViewSpec::default();
        let change = ViewChange { zoom: Some(4.0), controls: Some(ViewControls::Design), ..ViewChange::default() };
        let made = change.applied_to(&spec);
        assert_eq!((made.center, made.zoom, made.controls), (None, 4.0, ViewControls::Design), "a center left out stays the canvas's own middle");
        assert_eq!(change.problem(&spec), None);
        let zero = ViewChange { min_zoom: Some(0.0), ..ViewChange::default() };
        assert!(zero.problem(&spec).is_some_and(|p| p.contains("MinZoom")));
        let outside = ViewChange { zoom: Some(5.0), max_zoom: Some(2.0), ..ViewChange::default() };
        assert!(outside.problem(&spec).is_some_and(|p| p.contains("outside")));
        let wild = ViewChange { center: Some([f64::NAN, 0.0]), ..ViewChange::default() };
        assert!(wild.problem(&spec).is_some());
        let still = ViewChange { ease: Some(ViewEase { seconds: 0.0, easing: Easing::Ease }), ..ViewChange::default() };
        assert!(still.problem(&spec).is_some_and(|p| p.contains("seconds")));
        let custom = |bindings: Vec<(ViewGesture, ViewMove)>| ViewChange { controls: Some(ViewControls::Custom(bindings)), ..ViewChange::default() }.problem(&spec);
        let key = |key, modifiers| ViewGesture::Key { key, modifiers };
        assert_eq!(custom(vec![(ViewGesture::Pinch, ViewMove::Pan), (key(ViewKey::Letter(b'F'), CameraModifiers::NONE), ViewMove::Fit)]), None);
        assert!(custom(vec![(ViewGesture::Wheel { modifiers: CameraModifiers::NONE }, ViewMove::ZoomIn)]).is_some_and(|p| p.contains("cannot make")), "a wheel makes no step");
        assert!(custom(vec![(key(ViewKey::Home, CameraModifiers::NONE), ViewMove::Pan)]).is_some_and(|p| p.contains("cannot make")), "only an arrow pans");
        assert!(custom(vec![(key(ViewKey::Plus, CameraModifiers::SHIFT), ViewMove::ZoomIn)]).is_some_and(|p| p.contains("Shift")));
        assert!(custom(vec![(key(ViewKey::Digit(0), CameraModifiers::CTRL), ViewMove::ActualSize)]).is_some_and(|p| p.contains("Ctrl+0 zooms the window")));
        assert!(custom(vec![(key(ViewKey::Digit(12), CameraModifiers::NONE), ViewMove::Fit)]).is_some_and(|p| p.contains("no key")));
        assert!(custom(vec![(ViewGesture::Pinch, ViewMove::Zoom), (ViewGesture::Pinch, ViewMove::Pan)]).is_some_and(|p| p.contains("twice")));
    }
}
