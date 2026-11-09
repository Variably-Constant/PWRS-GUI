//! A test's input to a window, step by step, as a person's hand on the
//! mouse and the keyboard gives it. A step is a button pressed or let go
//! at a point of the window's inside, the pointer moved there, a key
//! pressed and let go, or a wait between them, each with the modifier keys
//! held.
//! Invoke-GuiTestEvent -Input reads each step from a hashtable
//! ([`Step::read`]), and each renderer gives the window its steps a frame
//! apart through its own input, so the input does what a person's would.

use crate::{Shortcut, Value};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::time::Duration;

/// A pointer button.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Button {
    Left,
    Right,
    Middle,
}

impl Button {
    /// The buttons, in the order a pointer event numbers them from 0.
    pub const ALL: [Button; 3] = [Button::Left, Button::Right, Button::Middle];

    /// The button's name, as a step names it.
    pub fn name(self) -> &'static str {
        match self {
            Button::Left => "Left",
            Button::Right => "Right",
            Button::Middle => "Middle",
        }
    }

    /// The button named `text`, without regard to case.
    pub fn named(text: &str) -> Option<Button> {
        Button::ALL.into_iter().find(|b| b.name().eq_ignore_ascii_case(text))
    }

    /// The button's place in [`Button::ALL`].
    pub fn slot(self) -> usize {
        match self {
            Button::Left => 0,
            Button::Right => 1,
            Button::Middle => 2,
        }
    }
}

/// The modifier keys held during a step; Ctrl is the Command key on macOS,
/// as it is in a shortcut.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Keys {
    pub shift: bool,
    pub ctrl: bool,
    pub alt: bool,
}

impl Keys {
    /// Whether any of the keys is held.
    pub fn any(self) -> bool {
        self.shift || self.ctrl || self.alt
    }
}

/// The key that opens a context menu where a keyboard has one, which a
/// step names `Menu`.
pub const MENU: &str = "Menu";

/// How long each step of a test's input has to be taken and seen when the
/// caller gives no time of its own; a wait has its own seconds besides.
pub const STEP_TIME: Duration = Duration::from_secs(10);

/// One step of a test's input. A point is in points from the top left of
/// the window's inside, where Get-GuiSnapshot's Bounds place widgets.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum Step {
    /// `button` pressed at `at`, the pointer moved there with it.
    Press { button: Button, at: [f64; 2], keys: Keys },
    /// `button` let go at `at`, the pointer moved there with it.
    Release { button: Button, at: [f64; 2], keys: Keys },
    /// The pointer moved to `at`.
    Move { at: [f64; 2], keys: Keys },
    /// A key pressed and let go: `key` is a shortcut's key, as
    /// [`Shortcut::parse`] reads one, or [`MENU`].
    Key { key: String, keys: Keys },
    /// Nothing happening for `seconds`.
    Wait { seconds: f64 },
}

/// What a step's hashtable may hold, as a refusal names it.
const TAKEN: &str = "Press, Release, Move, Key or Wait, with X and Y for a press, a release or a move, and Shift, Ctrl and Alt";

/// The keys a step's hashtable may hold, in lower case.
const KEYS: [&str; 10] = ["press", "release", "move", "key", "wait", "x", "y", "shift", "ctrl", "alt"];

/// The keys that say what a step does, in lower case.
const VERBS: [&str; 5] = ["press", "release", "move", "key", "wait"];

impl Step {
    /// The step a test's hashtable `given` describes, its keys compared
    /// without regard to case: one of `Press` or `Release`, naming the
    /// button, or `Move = $true`, each with `X` and `Y`; `Key`, the key
    /// written as a shortcut writes one, `Shift+F10`, or `Menu`; or `Wait`,
    /// seconds of 0 or more. Any of them but a wait takes `Shift`, `Ctrl`
    /// and `Alt`, true or false, which a key's own modifiers add to.
    pub fn read(given: &Value) -> Result<Step, String> {
        let Some(map) = given.as_map() else {
            return Err(format!("a step of input is a hashtable of {TAKEN}, not {}", described(given)));
        };
        let mut seen: BTreeMap<String, &Value> = BTreeMap::new();
        for (k, v) in map {
            let lower = k.to_ascii_lowercase();
            if !KEYS.contains(&lower.as_str()) {
                return Err(format!("a step of input takes {TAKEN}, and this one has {k}"));
            }
            if seen.insert(lower, v).is_some() {
                return Err(format!("a step of input names {k} twice, without regard to case"));
            }
        }
        let named: Vec<(&str, &Value)> = VERBS.into_iter().filter_map(|v| seen.get(v).map(|given| (v, *given))).collect();
        let (verb, value) = match named.as_slice() {
            [one] => *one,
            [] => return Err(String::from("a step of input names one of Press, Release, Move, Key or Wait, and this one names none")),
            more => return Err(format!("a step of input names one of Press, Release, Move, Key or Wait, and this one names {}", more.iter().map(|(v, _)| title(v)).collect::<Vec<_>>().join(" and "))),
        };
        let held = |name: &str| match seen.get(name) {
            None | Some(Value::Null) => Ok(false),
            Some(Value::Bool(on)) => Ok(*on),
            Some(other) => Err(format!("a step's {} is true or false, not {}", title(name), described(other))),
        };
        let keys = Keys { shift: held("shift")?, ctrl: held("ctrl")?, alt: held("alt")? };
        let coordinate = |name: &str| match seen.get(name).map(|v| v.as_number()) {
            Some(Some(n)) if n.is_finite() => Ok(n),
            Some(Some(n)) => Err(format!("a {}'s {} is a finite number of points, not {n}", title(verb), title(name))),
            Some(None) | None => Err(format!("a {} takes X and Y, numbers of points from the top left of the window's inside", title(verb))),
        };
        let pointless = !seen.contains_key("x") && !seen.contains_key("y");
        match verb {
            "press" | "release" => {
                let Some(button) = value.as_str().and_then(Button::named) else {
                    return Err(format!("a {}'s button is Left, Right or Middle, not {}", title(verb), described(value)));
                };
                let at = [coordinate("x")?, coordinate("y")?];
                Ok(if verb == "press" { Step::Press { button, at, keys } } else { Step::Release { button, at, keys } })
            }
            "move" => match value {
                Value::Bool(true) => Ok(Step::Move { at: [coordinate("x")?, coordinate("y")?], keys }),
                other => Err(format!("a move is written Move = $true with X and Y, not Move = {}", described(other))),
            },
            "key" if !pointless => Err(String::from("a key takes no X or Y")),
            "key" => {
                let Some(written) = value.as_str() else {
                    return Err(format!("a key is written as a shortcut writes one, as Shift+F10, or Menu, not {}", described(value)));
                };
                let (key, own) = key_of(written)?;
                Ok(Step::Key { key, keys: Keys { shift: keys.shift || own.shift, ctrl: keys.ctrl || own.ctrl, alt: keys.alt || own.alt } })
            }
            _ if !pointless => Err(String::from("a wait takes no X or Y")),
            _ if keys.any() => Err(String::from("a wait takes no Shift, Ctrl or Alt")),
            _ => match value.as_number() {
                Some(s) if s.is_finite() && s >= 0.0 => Ok(Step::Wait { seconds: s }),
                _ => Err(format!("a wait is a number of seconds, 0 or more, not {}", described(value))),
            },
        }
    }

    /// The point a press, a release or a move puts the pointer at.
    pub fn point(&self) -> Option<[f64; 2]> {
        match self {
            Step::Press { at, .. } | Step::Release { at, .. } | Step::Move { at, .. } => Some(*at),
            Step::Key { .. } | Step::Wait { .. } => None,
        }
    }

    /// The step as an error or a trace names it: a press of the Right
    /// button at 40, 12 with Ctrl held, the key Shift+F10, a wait of 0.9
    /// seconds.
    pub fn description(&self) -> String {
        let with = |keys: &Keys| match held_names(*keys).as_slice() {
            [] => String::new(),
            names => format!(" with {} held", names.join(" and ")),
        };
        match self {
            Step::Press { button, at, keys } => format!("a press of the {} button at {}, {}{}", button.name(), at[0], at[1], with(keys)),
            Step::Release { button, at, keys } => format!("a release of the {} button at {}, {}{}", button.name(), at[0], at[1], with(keys)),
            Step::Move { at, keys } => format!("a move to {}, {}{}", at[0], at[1], with(keys)),
            Step::Key { key, keys } => {
                let mut written = held_names(*keys);
                written.push(key);
                format!("the key {}", written.join("+"))
            }
            Step::Wait { seconds } => format!("a wait of {seconds} seconds"),
        }
    }

    /// How long the step has to be taken and seen when each step has
    /// `time`: a wait has its own seconds besides. Refused for a wait no
    /// clock counts, and for a sum no clock counts.
    pub fn allowed(&self, time: Duration) -> Result<Duration, String> {
        match self {
            Step::Wait { seconds } => {
                let wait = wait_time(*seconds)?;
                time.checked_add(wait).ok_or_else(|| format!("a wait of {seconds} seconds with {} seconds more is no length of time a clock counts", time.as_secs_f64()))
            }
            Step::Press { .. } | Step::Release { .. } | Step::Move { .. } | Step::Key { .. } => Ok(time),
        }
    }
}

/// The modifier keys `keys` holds, by name, in the order a shortcut writes
/// them.
fn held_names(keys: Keys) -> Vec<&'static str> {
    [(keys.ctrl, "Ctrl"), (keys.shift, "Shift"), (keys.alt, "Alt")].into_iter().filter_map(|(on, name)| on.then_some(name)).collect()
}

/// How long a wait of `seconds` lasts; refused for a number no clock counts
/// to.
pub fn wait_time(seconds: f64) -> Result<Duration, String> {
    Duration::try_from_secs_f64(seconds).map_err(|e| format!("a wait of {seconds} seconds is no length of time a clock counts: {e}"))
}

/// Why `steps` cannot be given to window `window`, whose inside is `size`
/// points across and down. Its pointer holds the buttons `held`, by their
/// places in [`Button::ALL`], as tests' input left them. A step is refused
/// for a point not inside the window, from its top left up to its right and
/// bottom edges, the edges themselves outside. So is a press of a button
/// held then, a release of one not held, or a wait no clock counts to.
/// `None` when every step can be given.
pub fn refusal(steps: &[Step], window: u64, size: [f64; 2], held: [bool; 3]) -> Option<String> {
    let mut holding = held;
    steps.iter().enumerate().find_map(|(i, step)| {
        match step {
            Step::Wait { seconds } => return wait_time(*seconds).err().map(|why| format!("step {}: {why}", i + 1)),
            Step::Press { button, .. } if holding[button.slot()] => return Some(format!("step {} presses the {} button, which the pointer holds already", i + 1, button.name())),
            Step::Release { button, .. } if !holding[button.slot()] => return Some(format!("step {} lets go of the {} button, which the pointer does not hold", i + 1, button.name())),
            Step::Press { button, .. } => holding[button.slot()] = true,
            Step::Release { button, .. } => holding[button.slot()] = false,
            Step::Move { .. } | Step::Key { .. } => {}
        }
        let [x, y] = step.point()?;
        let inside = x >= 0.0 && y >= 0.0 && x < size[0] && y < size[1];
        (!inside).then(|| format!("step {} puts the pointer at {x}, {y}, outside window {window}'s inside of {} by {} points", i + 1, size[0], size[1]))
    })
}

/// The buttons the pointer holds once `steps` are given, from `held`, by
/// their places in [`Button::ALL`].
pub fn held_after(steps: &[Step], held: [bool; 3]) -> [bool; 3] {
    steps.iter().fold(held, |mut holding, step| {
        match step {
            Step::Press { button, .. } => holding[button.slot()] = true,
            Step::Release { button, .. } => holding[button.slot()] = false,
            Step::Move { .. } | Step::Key { .. } | Step::Wait { .. } => {}
        }
        holding
    })
}

/// The key `written` names and the modifiers written with it: a shortcut's
/// key, or [`MENU`], without regard to case.
fn key_of(written: &str) -> Result<(String, Keys), String> {
    let parts: Vec<&str> = written.split('+').map(str::trim).collect();
    if let Some((last, modifiers)) = parts.split_last()
        && last.eq_ignore_ascii_case(MENU)
    {
        let mut keys = Keys::default();
        for m in modifiers {
            match m.to_ascii_lowercase().as_str() {
                "ctrl" => keys.ctrl = true,
                "shift" => keys.shift = true,
                "alt" => keys.alt = true,
                _ => return Err(format!("{written:?} has {m:?}, which is not Ctrl, Shift or Alt")),
            }
        }
        return Ok((MENU.to_string(), keys));
    }
    match Shortcut::parse(written) {
        Ok(s) => Ok((s.key, Keys { shift: s.shift, ctrl: s.ctrl, alt: s.alt })),
        Err(why) => Err(format!("{why}, or Menu")),
    }
}

/// A step's key as a refusal writes it.
fn title(name: &str) -> String {
    let mut out = name.to_string();
    if let Some(first) = out.get_mut(..1) {
        first.make_ascii_uppercase();
    }
    out
}

/// A value as a refusal names it.
fn described(v: &Value) -> String {
    match v {
        Value::Null => String::from("null"),
        Value::Bool(b) => format!("${b}"),
        Value::Int(i) => i.to_string(),
        Value::Float(f) => f.to_string(),
        Value::Str(s) => format!("{s:?}"),
        Value::List(_) => String::from("a list"),
        Value::Map(_) => String::from("a hashtable"),
        Value::Bytes(_) => String::from("bytes"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn step(pairs: &[(&str, Value)]) -> Result<Step, String> {
        Step::read(&Value::Map(pairs.iter().map(|(k, v)| (k.to_string(), v.clone())).collect()))
    }

    fn text(s: &str) -> Value {
        Value::Str(s.to_string())
    }

    const NONE: Keys = Keys { shift: false, ctrl: false, alt: false };

    #[test]
    fn each_kind_of_step_reads_from_its_keys_in_any_case() {
        let read = |pairs: &[(&str, Value)]| step(pairs).expect("the step reads");
        assert_eq!(read(&[("press", text("right")), ("X", Value::Int(40)), ("y", Value::Float(12.5))]), Step::Press { button: Button::Right, at: [40.0, 12.5], keys: NONE });
        assert_eq!(
            read(&[("Release", text("Middle")), ("X", Value::Int(1)), ("Y", Value::Int(2)), ("Ctrl", Value::Bool(true))]),
            Step::Release { button: Button::Middle, at: [1.0, 2.0], keys: Keys { shift: false, ctrl: true, alt: false } }
        );
        let moved = read(&[("Move", Value::Bool(true)), ("X", Value::Int(3)), ("Y", Value::Int(4)), ("Shift", Value::Bool(true))]);
        assert_eq!(moved, Step::Move { at: [3.0, 4.0], keys: Keys { shift: true, ctrl: false, alt: false } });
        assert_eq!(moved.point(), Some([3.0, 4.0]));
        assert_eq!(read(&[("Key", text("shift+f10"))]), Step::Key { key: String::from("F10"), keys: Keys { shift: true, ctrl: false, alt: false } });
        assert_eq!(read(&[("Key", text("menu")), ("Alt", Value::Bool(true))]), Step::Key { key: String::from("Menu"), keys: Keys { shift: false, ctrl: false, alt: true } });
        assert_eq!(read(&[("Key", text("Ctrl+s")), ("Shift", Value::Bool(true))]), Step::Key { key: String::from("S"), keys: Keys { shift: true, ctrl: true, alt: false } }, "the step's keys add to the key's own");
        assert_eq!(read(&[("Wait", Value::Float(0.9))]), Step::Wait { seconds: 0.9 });
        assert_eq!(read(&[("Wait", Value::Float(0.9))]).point(), None);
    }

    #[test]
    fn a_step_that_is_not_one_is_refused_saying_what_a_step_takes() {
        let refused = |pairs: &[(&str, Value)], says: &str| match step(pairs) {
            Err(why) => assert!(why.contains(says), "{pairs:?}: {why} does not say {says:?}"),
            Ok(s) => panic!("{pairs:?} was taken as {s:?}"),
        };
        refused(&[("X", Value::Int(1)), ("Y", Value::Int(2))], "names none");
        refused(&[("Press", text("Left")), ("Key", text("A")), ("X", Value::Int(1)), ("Y", Value::Int(2))], "names Press and Key");
        refused(&[("Press", text("Left")), ("X", Value::Int(1))], "takes X and Y");
        refused(&[("Press", text("Back")), ("X", Value::Int(1)), ("Y", Value::Int(2))], "Left, Right or Middle");
        refused(&[("Press", text("Left")), ("X", Value::Float(f64::NAN)), ("Y", Value::Int(2))], "finite");
        refused(&[("Move", Value::Bool(false)), ("X", Value::Int(1)), ("Y", Value::Int(2))], "Move = $true");
        refused(&[("Key", text("Ctrl+Hyper+A"))], "\"Hyper\"");
        refused(&[("Key", text("Shift+Banana"))], "or Menu");
        refused(&[("Key", text("A")), ("X", Value::Int(1))], "takes no X or Y");
        refused(&[("Wait", Value::Int(-1))], "0 or more");
        refused(&[("Wait", Value::Int(1)), ("Ctrl", Value::Bool(true))], "takes no Shift");
        refused(&[("Press", text("Left")), ("X", Value::Int(1)), ("Y", Value::Int(2)), ("Shift", text("yes"))], "true or false");
        refused(&[("Press", text("Left")), ("X", Value::Int(1)), ("Y", Value::Int(2)), ("Button", text("Left"))], "has Button");
        refused(&[("press", text("Left")), ("Press", text("Right")), ("X", Value::Int(1)), ("Y", Value::Int(2))], "twice");
        match Step::read(&Value::Str(String::from("Press"))) {
            Err(why) => assert!(why.contains("hashtable"), "{why}"),
            Ok(s) => panic!("a string was taken as {s:?}"),
        }
    }

    #[test]
    fn steps_are_refused_where_a_point_leaves_the_window_or_a_wait_no_clock_counts() {
        let at = |x: f64, y: f64| Step::Move { at: [x, y], keys: NONE };
        let size = [400.0, 300.0];
        let free = [false; 3];
        assert_eq!(refusal(&[at(0.0, 0.0), at(399.5, 299.5), Step::Key { key: String::from(MENU), keys: NONE }, Step::Wait { seconds: 0.5 }], 7, size, free), None);
        let Some(why) = refusal(&[at(10.0, 10.0), at(400.0, 10.0)], 7, size, free) else { panic!("a point on the right edge is outside") };
        assert_eq!(why, "step 2 puts the pointer at 400, 10, outside window 7's inside of 400 by 300 points");
        assert!(refusal(&[at(-0.5, 10.0)], 7, size, free).is_some_and(|why| why.starts_with("step 1 ")));
        assert!(refusal(&[Step::Wait { seconds: 1e300 }], 7, size, free).is_some_and(|why| why.contains("no length of time")));
        assert_eq!(wait_time(0.25).expect("a quarter second"), Duration::from_millis(250));
    }

    #[test]
    fn a_press_of_a_button_held_and_a_release_of_one_not_held_are_refused() {
        let press = |button: Button| Step::Press { button, at: [5.0, 5.0], keys: NONE };
        let release = |button: Button| Step::Release { button, at: [5.0, 5.0], keys: NONE };
        let size = [100.0, 100.0];
        let free = [false; 3];
        assert_eq!(refusal(&[press(Button::Right), press(Button::Left), release(Button::Right), release(Button::Left)], 3, size, free), None);
        assert_eq!(refusal(&[press(Button::Left), press(Button::Left)], 3, size, free).as_deref(), Some("step 2 presses the Left button, which the pointer holds already"));
        assert_eq!(refusal(&[release(Button::Middle)], 3, size, free).as_deref(), Some("step 1 lets go of the Middle button, which the pointer does not hold"));
        assert_eq!(refusal(&[release(Button::Right)], 3, size, [false, true, false]), None, "a button an earlier input left held");
        assert_eq!(held_after(&[press(Button::Right), press(Button::Middle), release(Button::Right)], free), [false, false, true]);
    }

    #[test]
    fn each_step_is_named_with_its_point_and_the_keys_held() {
        let ctrl_shift = Keys { shift: true, ctrl: true, alt: false };
        assert_eq!(Step::Press { button: Button::Right, at: [40.0, 12.5], keys: NONE }.description(), "a press of the Right button at 40, 12.5");
        assert_eq!(Step::Release { button: Button::Left, at: [3.0, 4.0], keys: ctrl_shift }.description(), "a release of the Left button at 3, 4 with Ctrl and Shift held");
        assert_eq!(Step::Move { at: [1.0, 2.0], keys: Keys { shift: false, ctrl: false, alt: true } }.description(), "a move to 1, 2 with Alt held");
        assert_eq!(Step::Key { key: String::from("F10"), keys: Keys { shift: true, ctrl: false, alt: false } }.description(), "the key Shift+F10");
        assert_eq!(Step::Key { key: String::from(MENU), keys: NONE }.description(), "the key Menu");
        assert_eq!(Step::Wait { seconds: 0.9 }.description(), "a wait of 0.9 seconds");
    }

    #[test]
    fn a_wait_has_its_own_seconds_besides_the_time_each_step_has() {
        let time = Duration::from_secs(10);
        assert_eq!(Step::Move { at: [1.0, 2.0], keys: NONE }.allowed(time), Ok(time));
        assert_eq!(Step::Wait { seconds: 0.5 }.allowed(time), Ok(Duration::from_millis(10_500)));
        assert!(Step::Wait { seconds: 1e300 }.allowed(time).is_err_and(|why| why.contains("no length of time")), "a wait no clock counts");
        assert!(Step::Wait { seconds: 1.0 }.allowed(Duration::MAX).is_err_and(|why| why.contains("seconds more")), "a sum no clock counts");
    }
}
