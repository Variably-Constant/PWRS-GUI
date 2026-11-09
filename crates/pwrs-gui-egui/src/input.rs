//! A test's input to a window ([`proto::test_input`]): each step given to
//! the window's viewport as the events egui-winit makes of a person's input,
//! a step a frame, through eframe's raw input hook, so egui takes it as it
//! takes a person's. A press or a release moves the pointer to its point in
//! the same frame, a key is pressed in one frame and let go in the next, and
//! a wait gives nothing until its time has passed. The reply goes at the
//! frame after the last step, once that step's frame has run.

use crate::egui;
use pwrs_gui_proto as proto;
use proto::test_input::{Button, Keys, Step};
use proto::{RequestId, RingName};
use std::collections::VecDeque;
use std::time::{Duration, Instant};

/// A step as egui takes it: the pointer moved to a point, with a button
/// pressed (true) or let go (false) there; a key pressed, with the text it
/// types; or a wait.
#[derive(Clone, Debug, PartialEq)]
enum Due {
    Pointer { at: egui::Pos2, button: Option<(egui::PointerButton, bool)>, modifiers: egui::Modifiers },
    Key { key: egui::Key, text: Option<String>, modifiers: egui::Modifiers },
    Wait(Duration),
}

/// A test's input being given to a window: the request and the ring its
/// reply goes to, the steps not yet given, when the wait under way ends,
/// and the key pressed in the frame before, to be let go in this one. It
/// counts the steps begun, of `total`, and the buttons held as the steps
/// given so far leave them, by their places in
/// [`proto::test_input::Button::ALL`].
pub(crate) struct Given {
    pub request: RequestId,
    pub reply: RingName,
    pub total: u32,
    steps: VecDeque<Due>,
    until: Option<Instant>,
    held_key: Option<(egui::Key, egui::Modifiers)>,
    begun: u32,
    reported: u32,
    holding: [bool; 3],
}

/// What one frame of the window did with a test's input.
#[derive(Debug, PartialEq)]
pub(crate) enum Fed {
    /// A step's events went into the frame's input; the next frame is due.
    Step,
    /// A wait goes on; a frame is due once it ends ([`Given::wait_left`]).
    Wait,
    /// Every step has been given and its frame has run: the reply goes now.
    Done,
    /// The wait due cannot be counted on this clock from now.
    Refused(String),
}

impl Given {
    /// The input of `steps` for the request `request`, its reply going to
    /// `reply`, given to a pointer holding the buttons `held`; refused for
    /// a key egui has no name for, for a wait no clock counts to, naming the
    /// step from 1, and for more steps than a count of them holds.
    pub(crate) fn new(request: RequestId, reply: RingName, steps: Vec<Step>, held: [bool; 3]) -> Result<Given, String> {
        let total = u32::try_from(steps.len()).map_err(|e| format!("{} steps are more than an input counts: {e}", steps.len()))?;
        let due = steps.into_iter().enumerate().map(|(i, step)| due_of(step).map_err(|why| format!("step {}: {why}", i + 1))).collect::<Result<VecDeque<Due>, String>>()?;
        Ok(Given { request, reply, total, steps: due, until: None, held_key: None, begun: 0, reported: 0, holding: held })
    }

    /// The step the last frame began, counted from 1, when it began one;
    /// each step is given once.
    pub(crate) fn began(&mut self) -> Option<u32> {
        (self.begun > self.reported).then(|| {
            self.reported = self.begun;
            self.begun
        })
    }

    /// The buttons held as the steps given so far leave them.
    pub(crate) fn holding(&self) -> [bool; 3] {
        self.holding
    }

    /// Whether steps are left to give: a window closing now leaves them
    /// untaken.
    pub(crate) fn unfinished(&self) -> bool {
        !self.steps.is_empty() || self.held_key.is_some() || self.until.is_some()
    }

    /// How long the wait under way has left at `now`, while one is.
    pub(crate) fn wait_left(&self, now: Instant) -> Option<Duration> {
        self.until.map(|until| until.saturating_duration_since(now))
    }

    /// Gives `raw`, the input of a frame of the window at `now`, this
    /// frame's part of the steps: a key let go that the frame before
    /// pressed, else nothing while a wait goes on, else the next step.
    pub(crate) fn feed(&mut self, raw: &mut egui::RawInput, now: Instant) -> Fed {
        if let Some((key, modifiers)) = self.held_key.take() {
            raw.events.push(egui::Event::ModifiersChanged(modifiers));
            raw.events.push(egui::Event::Key { key, physical_key: Some(key), pressed: false, repeat: false, modifiers });
            return Fed::Step;
        }
        if let Some(until) = self.until {
            if now < until {
                return Fed::Wait;
            }
            self.until = None;
        }
        let Some(step) = self.steps.pop_front() else {
            return Fed::Done;
        };
        self.begun += 1;
        match step {
            Due::Wait(length) => match now.checked_add(length) {
                Some(until) => {
                    self.until = Some(until);
                    Fed::Wait
                }
                None => Fed::Refused(format!("a wait of {} seconds runs past the end of this clock", length.as_secs_f64())),
            },
            Due::Pointer { at, button, modifiers } => {
                raw.events.push(egui::Event::ModifiersChanged(modifiers));
                raw.events.push(egui::Event::PointerMoved(at));
                if let Some((button, pressed)) = button {
                    raw.events.push(egui::Event::PointerButton { pos: at, button, pressed, modifiers });
                    if let Some(slot) = slot_of(button) {
                        self.holding[slot] = pressed;
                    }
                }
                Fed::Step
            }
            Due::Key { key, text, modifiers } => {
                raw.events.push(egui::Event::ModifiersChanged(modifiers));
                match clipboard_event(key, modifiers) {
                    Ok(Clipboard::Made(made)) => raw.events.push(made),
                    Ok(Clipboard::Nothing) => {}
                    Ok(Clipboard::Key) => {
                        raw.events.push(egui::Event::Key { key, physical_key: Some(key), pressed: true, repeat: false, modifiers });
                        if let Some(text) = text {
                            raw.events.push(egui::Event::Text(text));
                        }
                    }
                    Err(why) => return Fed::Refused(why),
                }
                self.held_key = Some((key, modifiers));
                Fed::Step
            }
        }
    }
}

/// What egui-winit makes of a key's press: a clipboard event in place of
/// its key event, nothing for a paste with no text to paste, or the key.
enum Clipboard {
    Made(egui::Event),
    Nothing,
    Key,
}

/// The clipboard event egui-winit makes of a press of `key` with
/// `modifiers` in place of its key event. Ctrl, or Cmd on macOS, with X is
/// Cut, with C Copy, and with V a paste of the system clipboard's text, its
/// CR LF line endings made LF. On Windows Shift+Delete is Cut, Ctrl+Insert
/// Copy and Shift+Insert a paste. Refused where the clipboard cannot be read.
fn clipboard_event(key: egui::Key, modifiers: egui::Modifiers) -> Result<Clipboard, String> {
    let windows = cfg!(windows);
    if key == egui::Key::Cut || (modifiers.command_only() && key == egui::Key::X) || (windows && modifiers.shift && key == egui::Key::Delete) {
        return Ok(Clipboard::Made(egui::Event::Cut));
    }
    if key == egui::Key::Copy || (modifiers.command_only() && key == egui::Key::C) || (windows && modifiers.ctrl && key == egui::Key::Insert) {
        return Ok(Clipboard::Made(egui::Event::Copy));
    }
    if key == egui::Key::Paste || (modifiers.command_only() && key == egui::Key::V) || (windows && modifiers.shift && key == egui::Key::Insert) {
        let mut clipboard = arboard::Clipboard::new().map_err(|e| format!("the system's clipboard cannot be opened to paste from: {e}"))?;
        return match clipboard.get_text() {
            Ok(text) if !text.is_empty() => Ok(Clipboard::Made(egui::Event::Paste(text.replace("\r\n", "\n")))),
            Ok(_) | Err(arboard::Error::ContentNotAvailable) => Ok(Clipboard::Nothing),
            Err(e) => Err(format!("the system's clipboard could not be read to paste from: {e}")),
        };
    }
    Ok(Clipboard::Key)
}

/// `step` as egui takes it; refused for a key egui has no name for and a
/// wait no clock counts to.
fn due_of(step: Step) -> Result<Due, String> {
    let point = |at: [f64; 2]| egui::pos2(at[0] as f32, at[1] as f32);
    Ok(match step {
        Step::Press { button, at, keys } => Due::Pointer { at: point(at), button: Some((pointer_button(button), true)), modifiers: modifiers_of(keys) },
        Step::Release { button, at, keys } => Due::Pointer { at: point(at), button: Some((pointer_button(button), false)), modifiers: modifiers_of(keys) },
        Step::Move { at, keys } => Due::Pointer { at: point(at), button: None, modifiers: modifiers_of(keys) },
        Step::Key { key, keys } => match egui::Key::from_name(&key) {
            Some(k) => Due::Key { key: k, text: typed(&key, keys), modifiers: modifiers_of(keys) },
            None => return Err(format!("egui has no key named {key}")),
        },
        Step::Wait { seconds } => Due::Wait(proto::test_input::wait_time(seconds)?),
    })
}

fn pointer_button(button: Button) -> egui::PointerButton {
    match button {
        Button::Left => egui::PointerButton::Primary,
        Button::Right => egui::PointerButton::Secondary,
        Button::Middle => egui::PointerButton::Middle,
    }
}

/// The place in [`Button::ALL`] of the button egui calls `button`; none
/// for the extra buttons, which no step presses.
fn slot_of(button: egui::PointerButton) -> Option<usize> {
    match button {
        egui::PointerButton::Primary => Some(Button::Left.slot()),
        egui::PointerButton::Secondary => Some(Button::Right.slot()),
        egui::PointerButton::Middle => Some(Button::Middle.slot()),
        egui::PointerButton::Extra1 | egui::PointerButton::Extra2 => None,
    }
}

/// The modifier keys as egui-winit reports them: Ctrl is the Command key on
/// macOS and the Control key elsewhere, either way egui's command.
fn modifiers_of(keys: Keys) -> egui::Modifiers {
    let mac = cfg!(target_os = "macos");
    egui::Modifiers { alt: keys.alt, ctrl: keys.ctrl && !mac, shift: keys.shift, mac_cmd: keys.ctrl && mac, command: keys.ctrl }
}

/// The text a key types with `keys` held, as egui-winit gives it: a
/// letter's, in upper case with Shift; a digit's without Shift; a space and
/// the signs Plus, Minus and Equals type; none with Ctrl or Alt, and none
/// for a key that types nothing.
fn typed(key: &str, keys: Keys) -> Option<String> {
    if keys.ctrl || keys.alt {
        return None;
    }
    match key {
        "Space" => Some(String::from(" ")),
        "Plus" => Some(String::from("+")),
        "Minus" => Some(String::from("-")),
        "Equals" => Some(String::from("=")),
        k if k.len() == 1 && k.bytes().all(|b| b.is_ascii_alphabetic()) => Some(if keys.shift { k.to_ascii_uppercase() } else { k.to_ascii_lowercase() }),
        k if k.len() == 1 && k.bytes().all(|b| b.is_ascii_digit()) && !keys.shift => Some(k.to_string()),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const NONE: Keys = Keys { shift: false, ctrl: false, alt: false };

    /// Runs `steps` through egui's own context, a frame at a time, each 16
    /// ms after the one before, until the input says it is done. Each frame
    /// draws a widget 100 by 40 points at (20, 30) sensing clicks and drags,
    /// and one draws it before the first step, so egui's hit test finds it.
    /// Returns, for each frame of the steps, whether Left, Right and Middle
    /// clicked the widget and whether its drag started, and whether egui
    /// took the Menu key as pressed.
    fn run(steps: Vec<Step>) -> Vec<([bool; 4], bool)> {
        let ctx = egui::Context::default();
        let mut given = Given::new(1, String::from("reply"), steps, [false; 3]).expect("the steps are egui's");
        let start = Instant::now();
        let rect = egui::Rect::from_min_size(egui::pos2(20.0, 30.0), egui::vec2(100.0, 40.0));
        let mut seen = Vec::new();
        for frame in 0..100u32 {
            let now = start + Duration::from_millis(u64::from(frame) * 16);
            let mut raw = egui::RawInput { time: Some(f64::from(frame) * 0.016), screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(400.0, 300.0))), ..egui::RawInput::default() };
            if frame > 0 {
                match given.feed(&mut raw, now) {
                    Fed::Done => return seen,
                    Fed::Refused(why) => panic!("the input was refused: {why}"),
                    Fed::Step | Fed::Wait => {}
                }
            }
            let mut out = ([false; 4], false);
            ctx.run_ui(raw, |ui| {
                let r = ui.allocate_rect(rect, egui::Sense::click_and_drag());
                out.0 = [r.clicked_by(egui::PointerButton::Primary), r.clicked_by(egui::PointerButton::Secondary), r.clicked_by(egui::PointerButton::Middle), r.drag_started()];
                out.1 = ui.input(|i| i.key_pressed(egui::Key::Menu));
            })
            .drop_without_applying_deltas();
            if frame > 0 {
                seen.push(out);
            }
        }
        panic!("the input never said it was done: {seen:?}");
    }

    fn press(button: Button, x: f64, y: f64) -> Step {
        Step::Press { button, at: [x, y], keys: NONE }
    }

    fn release(button: Button, x: f64, y: f64) -> Step {
        Step::Release { button, at: [x, y], keys: NONE }
    }

    #[test]
    fn a_right_click_clicks_by_the_right_button_and_a_right_drag_drags() {
        let clicked = run(vec![press(Button::Right, 50.0, 50.0), release(Button::Right, 52.0, 50.0)]);
        assert_eq!(clicked.len(), 2, "a frame a step");
        assert_eq!(clicked[1].0, [false, true, false, false], "released within a click's distance: a right click");
        let dragged = run(vec![press(Button::Right, 50.0, 50.0), Step::Move { at: [60.0, 50.0], keys: NONE }, release(Button::Right, 60.0, 50.0)]);
        assert!(dragged.iter().all(|(f, _)| !f[1]), "a right drag clicks nothing: {dragged:?}");
        assert!(dragged.iter().any(|(f, _)| f[3]), "and its drag starts: {dragged:?}");
    }

    #[test]
    fn a_long_press_waits_past_a_click_s_time_and_a_key_takes_two_frames() {
        let long = run(vec![press(Button::Right, 50.0, 50.0), Step::Wait { seconds: 0.9 }, release(Button::Right, 50.0, 50.0)]);
        assert!(long.len() > 50, "the wait takes its 0.9 s of frames: {}", long.len());
        assert!(long.iter().all(|(f, _)| !f[1]), "a right press held past 0.8 s is no click: {long:?}");
        let menu = run(vec![Step::Key { key: String::from(proto::test_input::MENU), keys: NONE }]);
        assert_eq!(menu.len(), 2, "pressed, then let go");
        assert!(menu[0].1 && !menu[1].1, "egui takes the Menu key as pressed in the first frame: {menu:?}");
    }

    #[test]
    fn keys_type_what_egui_winit_types_and_ctrl_is_egui_s_command() {
        let shifted = Keys { shift: true, ctrl: false, alt: false };
        assert_eq!(typed("A", shifted).as_deref(), Some("A"));
        assert_eq!(typed("A", NONE).as_deref(), Some("a"));
        assert_eq!(typed("7", shifted), None);
        assert_eq!(typed("F10", NONE), None);
        assert_eq!(typed("S", Keys { shift: false, ctrl: true, alt: false }), None);
        assert!(modifiers_of(Keys { shift: false, ctrl: true, alt: false }).command);
        let refused = |steps: Vec<Step>| match Given::new(1, String::from("reply"), steps, [false; 3]) {
            Ok(_) => panic!("the steps were taken"),
            Err(why) => why,
        };
        assert!(refused(vec![press(Button::Left, 1.0, 1.0), Step::Key { key: String::from("Hyper"), keys: NONE }]).starts_with("step 2: egui has no key named Hyper"));
        assert!(refused(vec![Step::Wait { seconds: f64::INFINITY }]).starts_with("step 1: "));
    }

    #[test]
    fn ctrl_c_and_ctrl_x_are_egui_s_copy_and_cut_as_egui_winit_makes_them() {
        let ctrl = Keys { shift: false, ctrl: true, alt: false };
        let key = |name: &str, keys: Keys| Step::Key { key: String::from(name), keys };
        let mut given = Given::new(1, String::from("reply"), vec![key("C", ctrl), key("X", ctrl), key("C", NONE)], [false; 3]).expect("three keys");
        let mut frames = Vec::new();
        loop {
            let mut raw = egui::RawInput::default();
            if given.feed(&mut raw, Instant::now()) == Fed::Done {
                break;
            }
            frames.push(raw.events);
        }
        let pressed = |events: &[egui::Event]| events.iter().any(|e| matches!(e, egui::Event::Key { pressed: true, .. }));
        assert!(frames[0].contains(&egui::Event::Copy) && !pressed(&frames[0]), "Ctrl+C is egui's Copy, with no key pressed: {:?}", frames[0]);
        assert!(frames[1].iter().any(|e| matches!(e, egui::Event::Key { key: egui::Key::C, pressed: false, .. })), "and its key is let go: {:?}", frames[1]);
        assert!(frames[2].contains(&egui::Event::Cut) && !pressed(&frames[2]), "Ctrl+X is egui's Cut: {:?}", frames[2]);
        assert!(pressed(&frames[4]) && !frames[4].contains(&egui::Event::Copy), "C alone is a key: {:?}", frames[4]);
    }

    #[test]
    fn a_wait_says_how_long_it_has_left_for_the_frame_at_its_end() {
        let start = Instant::now();
        let mut given = Given::new(1, String::from("reply"), vec![Step::Wait { seconds: 0.5 }], [false; 3]).expect("a wait");
        assert_eq!(given.wait_left(start), None, "no wait before its step");
        assert_eq!(given.feed(&mut egui::RawInput::default(), start), Fed::Wait);
        assert_eq!(given.wait_left(start + Duration::from_millis(200)), Some(Duration::from_millis(300)));
        assert_eq!(given.feed(&mut egui::RawInput::default(), start + Duration::from_millis(200)), Fed::Wait);
        assert_eq!(given.wait_left(start + Duration::from_secs(1)), Some(Duration::ZERO), "a wait past its end has none left");
        assert_eq!(given.feed(&mut egui::RawInput::default(), start + Duration::from_secs(1)), Fed::Done);
    }

    #[test]
    fn a_window_closing_with_steps_left_leaves_them_untaken() {
        let mut given = Given::new(1, String::from("reply"), vec![press(Button::Left, 1.0, 1.0)], [false; 3]).expect("a press");
        assert!(given.unfinished());
        assert_eq!(given.feed(&mut egui::RawInput::default(), Instant::now()), Fed::Step);
        assert!(!given.unfinished(), "its one step is given");
        assert_eq!(given.feed(&mut egui::RawInput::default(), Instant::now()), Fed::Done);
    }

    #[test]
    fn each_step_begun_is_counted_once_and_the_buttons_held_follow_the_steps_given() {
        let release = Step::Release { button: Button::Right, at: [2.0, 2.0], keys: NONE };
        let mut given = Given::new(1, String::from("reply"), vec![press(Button::Right, 1.0, 1.0), release], [false, false, true]).expect("a press and a release");
        assert_eq!(given.total, 2);
        assert_eq!(given.began(), None, "no step before the first frame");
        assert_eq!(given.feed(&mut egui::RawInput::default(), Instant::now()), Fed::Step);
        assert_eq!(given.began(), Some(1));
        assert_eq!(given.began(), None, "each step is given once");
        assert_eq!(given.holding(), [false, true, true], "the press holds Right beside the Middle held before");
        assert_eq!(given.feed(&mut egui::RawInput::default(), Instant::now()), Fed::Step);
        assert_eq!(given.began(), Some(2));
        assert_eq!(given.holding(), [false, false, true]);
    }
}
