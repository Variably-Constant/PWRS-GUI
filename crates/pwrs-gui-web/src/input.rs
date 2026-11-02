//! A test's input to a webview through its engine's own input
//! ([`proto::test_input`]), so the engine does with it what it does with a
//! person's, its own contextmenu events among it. On Windows that is
//! WebView2's DevTools Protocol, `Input.dispatchMouseEvent` and
//! `Input.dispatchKeyEvent`. On Linux and the BSDs it is GDK events made
//! here and handed to WebKitGTK's widget, and on macOS NSEvents made here
//! and handed to the WKWebView. A press
//! or a release moves the pointer to its point first, and a key is pressed
//! and let go. The page says when it has seen each step, which paces them.

use pwrs_gui_proto as proto;
use proto::test_input::{Button, Keys, Step};
#[cfg(any(windows, target_os = "macos"))]
use std::time::Duration;
use std::time::Instant;
use wry::WebView;

/// Runs once, on the event loop's thread, with whether the engine took the
/// step.
pub(crate) type Sent = Box<dyn FnOnce(Result<(), String>)>;

/// The pointer as a test's input has left it on a webview: the buttons
/// held, by their place in [`Button::ALL`], where it is, and, where the
/// engine takes each press's clicks from its event, the last press, which
/// the next one counts its clicks from. WebKitGTK counts them itself.
#[derive(Default)]
pub(crate) struct Hand {
    held: [bool; 3],
    at: Option<[f64; 2]>,
    #[cfg(any(windows, target_os = "macos"))]
    last_press: Option<Press>,
}

/// A press a test's input made: its button, its point, when it came and how
/// many clicks long it was.
#[cfg(any(windows, target_os = "macos"))]
#[derive(Clone, Copy, Debug, PartialEq)]
struct Press {
    button: Button,
    at: [f64; 2],
    when: Instant,
    count: u32,
}

impl Hand {
    /// The clicks a press of `button` at `at` and `when` is, counted as the
    /// system counts them: one more than the press before when it is of
    /// the same button within `interval` of it and, where `slop` is given,
    /// within that many points of it across and down, else one.
    #[cfg(any(windows, target_os = "macos"))]
    fn count(&mut self, button: Button, at: [f64; 2], when: Instant, interval: Duration, slop: Option<[f64; 2]>) -> u32 {
        let near = |p: &Press| slop.is_none_or(|s| (p.at[0] - at[0]).abs() <= s[0] && (p.at[1] - at[1]).abs() <= s[1]);
        let count = match self.last_press {
            Some(p) if p.button == button && when.duration_since(p.when) <= interval && near(&p) => p.count + 1,
            _ => 1,
        };
        self.last_press = Some(Press { button, at, when, count });
        count
    }

    /// The clicks the press being let go was, for its release.
    #[cfg(any(windows, target_os = "macos"))]
    fn released_count(&self, button: Button) -> u32 {
        match self.last_press {
            Some(p) if p.button == button => p.count,
            _ => 1,
        }
    }

    /// The first of Left, Right and Middle held.
    #[cfg(any(windows, target_os = "macos"))]
    fn first_held(&self) -> Option<Button> {
        Button::ALL.into_iter().zip(self.held).find(|(_, held)| *held).map(|(b, _)| b)
    }

    /// The buttons held, by their places in [`Button::ALL`].
    pub(crate) fn held(&self) -> [bool; 3] {
        self.held
    }
}

/// The number of the F key a step names, F1 to F24; none for any other key.
fn f_key(name: &str) -> Option<u32> {
    (1..=24u32).find(|n| name == format!("F{n}"))
}

/// Gives `webview` `step` through its engine's input, at `scale` physical
/// pixels a point, `hand` holding where the pointer is and what is held;
/// `sent` runs once the engine has taken it, or with why it did not.
pub(crate) fn send(webview: &WebView, step: &Step, hand: &mut Hand, scale: f64, sent: Sent) {
    match engine_events(step, hand, scale, Instant::now()) {
        Ok(events) => dispatch(webview, events, sent),
        Err(why) => sent(Err(why)),
    }
}

/// The buttons held before `step` and those it leaves held, from those
/// `held` by their places in [`Button::ALL`], each as a pointer event's
/// buttons bits, for a page whose engine takes the buttons of every event a
/// program makes from the hardware, which then reports none held: WebKit on
/// macOS. None elsewhere, where the engine reads them from the event.
pub(crate) fn held_for_page(step: &Step, held: [bool; 3]) -> Option<[u32; 2]> {
    cfg!(target_os = "macos").then(|| held_around(step, held))
}

/// The buttons held before `step` and those it leaves held, from `held`,
/// each as a pointer event's buttons bits.
fn held_around(step: &Step, held: [bool; 3]) -> [u32; 2] {
    [held_bits(held), held_bits(proto::test_input::held_after(std::slice::from_ref(step), held))]
}

/// The buttons `held`, by their places in [`Button::ALL`], as a pointer
/// event's buttons bits: Left 1, Right 2, Middle 4.
fn held_bits(held: [bool; 3]) -> u32 {
    Button::ALL
        .into_iter()
        .zip(held)
        .filter(|(_, down)| *down)
        .map(|(button, _)| match button {
            Button::Left => 1,
            Button::Right => 2,
            Button::Middle => 4,
        })
        .sum()
}

/// Moves `hand` by `step`: the buttons a press or a release leaves held and
/// the point the pointer is at.
fn moved(hand: &mut Hand, step: &Step) {
    match step {
        Step::Press { button, at, .. } => {
            hand.held[button.slot()] = true;
            hand.at = Some(*at);
        }
        Step::Release { button, at, .. } => {
            hand.held[button.slot()] = false;
            hand.at = Some(*at);
        }
        Step::Move { at, .. } => hand.at = Some(*at),
        Step::Key { .. } | Step::Wait { .. } => {}
    }
}

// ---- WebView2 ----

/// One call of WebView2's DevTools Protocol: its method and its parameters
/// as JSON.
#[cfg(windows)]
type EngineEvent = (&'static str, String);

/// The DevTools Protocol's modifier bits for `keys`: Alt 1, Ctrl 2, Shift 8.
#[cfg(windows)]
fn cdp_modifiers(keys: Keys) -> u32 {
    u32::from(keys.alt) | (u32::from(keys.ctrl) << 1) | (u32::from(keys.shift) << 3)
}

/// The DevTools Protocol's bits for the buttons `hand` holds: Left 1,
/// Right 2, Middle 4.
#[cfg(windows)]
fn cdp_buttons(hand: &Hand) -> u32 {
    u32::from(hand.held[0]) | (u32::from(hand.held[1]) << 1) | (u32::from(hand.held[2]) << 2)
}

#[cfg(windows)]
fn cdp_button(button: Option<Button>) -> &'static str {
    match button {
        None => "none",
        Some(Button::Left) => "left",
        Some(Button::Right) => "right",
        Some(Button::Middle) => "middle",
    }
}

/// The DevTools Protocol's calls for `step` at `now`. For a press or a
/// release, the mouse moves to its point when it is elsewhere, then the
/// button is pressed or let go, its clicks counted by Windows' double-click
/// time and rectangle at `scale`. A move moves the mouse, and a key is
/// pressed and released with the text it types.
#[cfg(windows)]
fn engine_events(step: &Step, hand: &mut Hand, scale: f64, now: Instant) -> Result<Vec<EngineEvent>, String> {
    use serde_json::json;
    use windows::Win32::UI::Input::KeyboardAndMouse::GetDoubleClickTime;
    use windows::Win32::UI::WindowsAndMessaging::{GetSystemMetrics, SM_CXDOUBLECLK, SM_CYDOUBLECLK};
    let mut out = Vec::new();
    let mouse = |kind: &str, at: [f64; 2], button: Option<Button>, buttons: u32, keys: Keys, count: u32| {
        ("Input.dispatchMouseEvent", json!({ "type": kind, "x": at[0], "y": at[1], "modifiers": cdp_modifiers(keys), "button": cdp_button(button), "buttons": buttons, "clickCount": count, "pointerType": "mouse" }).to_string())
    };
    match step {
        Step::Press { button, at, keys } | Step::Release { button, at, keys } => {
            let pressed = matches!(step, Step::Press { .. });
            if hand.at != Some(*at) {
                out.push(mouse("mouseMoved", *at, hand.first_held(), cdp_buttons(hand), *keys, 0));
            }
            let count = if pressed {
                // SAFETY: each reads one of the system's settings and takes nothing.
                let (time, across, down) = unsafe { (GetDoubleClickTime(), GetSystemMetrics(SM_CXDOUBLECLK), GetSystemMetrics(SM_CYDOUBLECLK)) };
                let slop = [f64::from(across) / scale / 2.0, f64::from(down) / scale / 2.0];
                hand.count(*button, *at, now, Duration::from_millis(u64::from(time)), Some(slop))
            } else {
                hand.released_count(*button)
            };
            moved(hand, step);
            out.push(mouse(if pressed { "mousePressed" } else { "mouseReleased" }, *at, Some(*button), cdp_buttons(hand), *keys, count));
        }
        Step::Move { at, keys } => {
            moved(hand, step);
            out.push(mouse("mouseMoved", *at, hand.first_held(), cdp_buttons(hand), *keys, 0));
        }
        Step::Key { key, keys } => {
            let k = windows_key(key, keys.shift)?;
            let text = if keys.ctrl || keys.alt { None } else { k.text };
            let mut down = json!({ "type": if text.is_some() { "keyDown" } else { "rawKeyDown" }, "modifiers": cdp_modifiers(*keys), "windowsVirtualKeyCode": k.code, "nativeVirtualKeyCode": k.code, "key": k.key, "code": k.place, "autoRepeat": false, "isKeypad": false, "location": 0 });
            if let Some(t) = text {
                down["text"] = json!(t);
                down["unmodifiedText"] = json!(t);
            }
            let up = json!({ "type": "keyUp", "modifiers": cdp_modifiers(*keys), "windowsVirtualKeyCode": k.code, "nativeVirtualKeyCode": k.code, "key": k.key, "code": k.place, "location": 0 });
            out.push(("Input.dispatchKeyEvent", down.to_string()));
            out.push(("Input.dispatchKeyEvent", up.to_string()));
        }
        Step::Wait { .. } => {}
    }
    Ok(out)
}

/// A key as Windows and the DevTools Protocol name it: its `key` value, the
/// `code` of its place, its virtual-key code, and the text it types.
#[cfg(windows)]
struct WindowsKey {
    key: String,
    place: String,
    code: u32,
    text: Option<String>,
}

/// The key a step names, as a US keyboard has it, with Shift held as
/// `shift` says.
#[cfg(windows)]
fn windows_key(name: &str, shift: bool) -> Result<WindowsKey, String> {
    let named = |key: &str, place: &str, code: u32, text: Option<&str>| WindowsKey { key: key.to_string(), place: place.to_string(), code, text: text.map(String::from) };
    if let [c] = name.as_bytes()
        && c.is_ascii_alphabetic()
    {
        let typed = char::from(if shift { c.to_ascii_uppercase() } else { c.to_ascii_lowercase() }).to_string();
        return Ok(WindowsKey { key: typed.clone(), place: format!("Key{}", char::from(c.to_ascii_uppercase())), code: u32::from(c.to_ascii_uppercase()), text: Some(typed) });
    }
    if let [c] = name.as_bytes()
        && c.is_ascii_digit()
    {
        let shifted = b")!@#$%^&*(";
        let typed = char::from(if shift { shifted[usize::from(c - b'0')] } else { *c }).to_string();
        return Ok(WindowsKey { key: typed.clone(), place: format!("Digit{}", char::from(*c)), code: u32::from(*c), text: Some(typed) });
    }
    if let Some(n) = f_key(name) {
        return Ok(named(name, name, 0x6F + n, None));
    }
    Ok(match name {
        "Enter" => named("Enter", "Enter", 0x0D, Some("\r")),
        "Escape" => named("Escape", "Escape", 0x1B, None),
        "Tab" => named("Tab", "Tab", 0x09, None),
        "Space" => named(" ", "Space", 0x20, Some(" ")),
        "Backspace" => named("Backspace", "Backspace", 0x08, None),
        "Delete" => named("Delete", "Delete", 0x2E, None),
        "Insert" => named("Insert", "Insert", 0x2D, None),
        "Home" => named("Home", "Home", 0x24, None),
        "End" => named("End", "End", 0x23, None),
        "PageUp" => named("PageUp", "PageUp", 0x21, None),
        "PageDown" => named("PageDown", "PageDown", 0x22, None),
        "ArrowLeft" => named("ArrowLeft", "ArrowLeft", 0x25, None),
        "ArrowUp" => named("ArrowUp", "ArrowUp", 0x26, None),
        "ArrowRight" => named("ArrowRight", "ArrowRight", 0x27, None),
        "ArrowDown" => named("ArrowDown", "ArrowDown", 0x28, None),
        "Plus" => named("+", "NumpadAdd", 0x6B, Some("+")),
        "Minus" => named("-", "Minus", 0xBD, Some("-")),
        "Equals" => named("=", "Equal", 0xBB, Some("=")),
        proto::test_input::MENU => named("ContextMenu", "ContextMenu", 0x5D, None),
        other => return Err(format!("WebView2 has no key named {other}")),
    })
}

/// Makes WebView2's DevTools Protocol calls `events` one after another,
/// each once the one before has completed, and runs `sent` once the last
/// has, or with the first refusal.
#[cfg(windows)]
fn dispatch(webview: &WebView, events: Vec<EngineEvent>, sent: Sent) {
    use wry::WebViewExtWindows;
    call_each(webview.webview(), events.into(), sent);
}

#[cfg(windows)]
fn call_each(core: webview2_com::Microsoft::Web::WebView2::Win32::ICoreWebView2, mut events: std::collections::VecDeque<EngineEvent>, sent: Sent) {
    use std::cell::Cell;
    use std::rc::Rc;
    use webview2_com::CallDevToolsProtocolMethodCompletedHandler;
    use windows::core::HSTRING;
    let Some((method, params)) = events.pop_front() else {
        return sent(Ok(()));
    };
    let slot = Rc::new(Cell::new(Some(sent)));
    let answer = Rc::clone(&slot);
    let next = core.clone();
    let handler = CallDevToolsProtocolMethodCompletedHandler::create(Box::new(move |result, _reply| {
        if let Some(sent) = answer.take() {
            match result {
                Ok(()) => call_each(next, events, sent),
                Err(e) => sent(Err(format!("WebView2 refused the DevTools Protocol's {method}: {}", e.message()))),
            }
        }
        Ok(())
    }));
    // SAFETY: the method's name, its parameters and the handler are live
    // for the call, which copies the strings and holds the handler until it
    // has run.
    let started = unsafe { core.CallDevToolsProtocolMethod(&HSTRING::from(method), &HSTRING::from(params.as_str()), &handler) };
    if let Err(e) = started {
        let why = format!("WebView2 refused to call the DevTools Protocol's {method}: {}", e.message());
        match slot.take() {
            Some(sent) => sent(Err(why)),
            None => eprintln!("pwrs-gui-web: {why}, after it had answered the call"),
        }
    }
}

// ---- WebKitGTK ----

/// A GDK event made for a step: a motion, a button pressed or let go, or a
/// key pressed or let go, each with the modifier and button bits held.
#[cfg(any(target_os = "linux", target_os = "dragonfly", target_os = "freebsd", target_os = "openbsd", target_os = "netbsd"))]
#[derive(Debug)]
enum EngineEvent {
    Motion { at: [f64; 2], state: u32 },
    Button { pressed: bool, button: u32, at: [f64; 2], state: u32 },
    Key { pressed: bool, keyval: u32, state: u32 },
}

/// GDK's state bits for `keys` and the buttons `hand` holds.
#[cfg(any(target_os = "linux", target_os = "dragonfly", target_os = "freebsd", target_os = "openbsd", target_os = "netbsd"))]
fn gdk_state(keys: Keys, hand: &Hand) -> u32 {
    use gtk::gdk::ffi::{GDK_BUTTON1_MASK, GDK_BUTTON2_MASK, GDK_BUTTON3_MASK, GDK_CONTROL_MASK, GDK_MOD1_MASK, GDK_SHIFT_MASK};
    let bit = |on: bool, mask: u32| if on { mask } else { 0 };
    bit(keys.shift, GDK_SHIFT_MASK) | bit(keys.ctrl, GDK_CONTROL_MASK) | bit(keys.alt, GDK_MOD1_MASK) | bit(hand.held[0], GDK_BUTTON1_MASK) | bit(hand.held[2], GDK_BUTTON2_MASK) | bit(hand.held[1], GDK_BUTTON3_MASK)
}

/// GDK's events for `step`: a motion to a press's or a release's point when
/// the pointer is elsewhere, then the button, numbered 1 left, 2 middle and
/// 3 right, its state the one before it as X gives it; a motion; or a key's
/// press and release. WebKitGTK counts the clicks itself.
#[cfg(any(target_os = "linux", target_os = "dragonfly", target_os = "freebsd", target_os = "openbsd", target_os = "netbsd"))]
fn engine_events(step: &Step, hand: &mut Hand, _scale: f64, _now: Instant) -> Result<Vec<EngineEvent>, String> {
    let mut out = Vec::new();
    match step {
        Step::Press { button, at, keys } | Step::Release { button, at, keys } => {
            if hand.at != Some(*at) {
                out.push(EngineEvent::Motion { at: *at, state: gdk_state(*keys, hand) });
            }
            let state = gdk_state(*keys, hand);
            let number = match button {
                Button::Left => 1,
                Button::Middle => 2,
                Button::Right => 3,
            };
            out.push(EngineEvent::Button { pressed: matches!(step, Step::Press { .. }), button: number, at: *at, state });
            moved(hand, step);
        }
        Step::Move { at, keys } => {
            moved(hand, step);
            out.push(EngineEvent::Motion { at: *at, state: gdk_state(*keys, hand) });
        }
        Step::Key { key, keys } => {
            let keyval = gdk_keyval(key, keys.shift)?;
            let state = gdk_state(*keys, hand);
            out.push(EngineEvent::Key { pressed: true, keyval, state });
            out.push(EngineEvent::Key { pressed: false, keyval, state });
        }
        Step::Wait { .. } => {}
    }
    Ok(out)
}

/// The GDK key value of the key a step names, with Shift held as `shift`
/// says.
#[cfg(any(target_os = "linux", target_os = "dragonfly", target_os = "freebsd", target_os = "openbsd", target_os = "netbsd"))]
fn gdk_keyval(name: &str, shift: bool) -> Result<u32, String> {
    if let [c] = name.as_bytes()
        && c.is_ascii_alphanumeric()
    {
        return Ok(u32::from(if shift { c.to_ascii_uppercase() } else { c.to_ascii_lowercase() }));
    }
    if let Some(n) = f_key(name) {
        return Ok(0xFFBD + n);
    }
    Ok(match name {
        "Enter" => 0xFF0D,
        "Escape" => 0xFF1B,
        "Tab" => 0xFF09,
        "Space" => 0x20,
        "Backspace" => 0xFF08,
        "Delete" => 0xFFFF,
        "Insert" => 0xFF63,
        "Home" => 0xFF50,
        "End" => 0xFF57,
        "PageUp" => 0xFF55,
        "PageDown" => 0xFF56,
        "ArrowLeft" => 0xFF51,
        "ArrowUp" => 0xFF52,
        "ArrowRight" => 0xFF53,
        "ArrowDown" => 0xFF54,
        "Plus" => 0x2B,
        "Minus" => 0x2D,
        "Equals" => 0x3D,
        proto::test_input::MENU => 0xFF67,
        other => return Err(format!("GDK has no key named {other}")),
    })
}

/// When a GDK event made now happens, in milliseconds on GLib's monotonic
/// clock, which only goes forward. GDK's times are 32 bits, so this one
/// wraps as theirs do.
#[cfg(any(target_os = "linux", target_os = "dragonfly", target_os = "freebsd", target_os = "openbsd", target_os = "netbsd"))]
fn gdk_time() -> u32 {
    ((gtk::glib::monotonic_time() / 1000) & i64::from(u32::MAX)) as u32
}

/// Hands each of `events` to WebKitGTK's widget, as GDK hands it a
/// person's, on its own window and the display's pointer and keyboard;
/// `sent` runs once the widget has them all.
#[cfg(any(target_os = "linux", target_os = "dragonfly", target_os = "freebsd", target_os = "openbsd", target_os = "netbsd"))]
fn dispatch(webview: &WebView, events: Vec<EngineEvent>, sent: Sent) {
    sent(handed(webview, events));
}

#[cfg(any(target_os = "linux", target_os = "dragonfly", target_os = "freebsd", target_os = "openbsd", target_os = "netbsd"))]
fn handed(webview: &WebView, events: Vec<EngineEvent>) -> Result<(), String> {
    use gtk::gdk::ffi as gdk_ffi;
    use gtk::gdk::prelude::*;
    use gtk::glib::translate::ToGlibPtr;
    use gtk::prelude::*;
    use wry::WebViewExtUnix;
    let view = webview.webview();
    let widget: &gtk::Widget = view.upcast_ref();
    let window = widget.window().ok_or("WebKitGTK's widget has no window to take input on, as before it is shown")?;
    let display = window.display();
    let seat = display.default_seat().ok_or("the display has no seat to give input through")?;
    let pointer = seat.pointer().ok_or("the display's seat has no pointer")?;
    let keyboard = seat.keyboard().ok_or("the display's seat has no keyboard")?;
    let keymap = gtk::gdk::Keymap::for_display(&display).ok_or("the display has no keymap to find keys on")?;
    let (_, root_x, root_y) = window.origin();
    // A widget drawing on its parent's window takes points from its own
    // top left on it.
    let (dx, dy) = if widget.has_window() {
        (0.0, 0.0)
    } else {
        let a = widget.allocation();
        (f64::from(a.x()), f64::from(a.y()))
    };
    for event in events {
        let time = gdk_time();
        let entry = match &event {
            EngineEvent::Key { keyval, .. } => Some(keymap.entries_for_keyval(*keyval).into_iter().next().ok_or_else(|| format!("the keyboard's keymap has no key for GDK key value {keyval:#x}"))?),
            EngineEvent::Motion { .. } | EngineEvent::Button { .. } => None,
        };
        let code = match &entry {
            Some(e) => Some((u16::try_from(e.keycode()).map_err(|why| format!("a key whose code passes 16 bits: {why}"))?, u8::try_from(e.group()).map_err(|why| format!("a key whose group passes 8 bits: {why}"))?)),
            None => None,
        };
        // SAFETY: each event is made by GDK, filled in with the window it
        // takes a reference to and the seat's device, which it references
        // itself, handed to the widget on this, GTK's thread, and freed,
        // which drops both references.
        unsafe {
            let made = match event {
                EngineEvent::Motion { at, state } => {
                    let e = gdk_ffi::gdk_event_new(gdk_ffi::GDK_MOTION_NOTIFY);
                    let m = &mut (*e).motion;
                    m.window = window.to_glib_full();
                    m.time = time;
                    (m.x, m.y) = (at[0] + dx, at[1] + dy);
                    (m.x_root, m.y_root) = (f64::from(root_x) + m.x, f64::from(root_y) + m.y);
                    m.state = state;
                    gdk_ffi::gdk_event_set_device(e, pointer.to_glib_none().0);
                    gdk_ffi::gdk_event_set_source_device(e, pointer.to_glib_none().0);
                    e
                }
                EngineEvent::Button { pressed, button, at, state } => {
                    let e = gdk_ffi::gdk_event_new(if pressed { gdk_ffi::GDK_BUTTON_PRESS } else { gdk_ffi::GDK_BUTTON_RELEASE });
                    let b = &mut (*e).button;
                    b.window = window.to_glib_full();
                    b.time = time;
                    (b.x, b.y) = (at[0] + dx, at[1] + dy);
                    (b.x_root, b.y_root) = (f64::from(root_x) + b.x, f64::from(root_y) + b.y);
                    b.state = state;
                    b.button = button;
                    gdk_ffi::gdk_event_set_device(e, pointer.to_glib_none().0);
                    gdk_ffi::gdk_event_set_source_device(e, pointer.to_glib_none().0);
                    e
                }
                EngineEvent::Key { pressed, keyval, state } => {
                    let e = gdk_ffi::gdk_event_new(if pressed { gdk_ffi::GDK_KEY_PRESS } else { gdk_ffi::GDK_KEY_RELEASE });
                    let k = &mut (*e).key;
                    k.window = window.to_glib_full();
                    k.time = time;
                    k.state = state;
                    k.keyval = keyval;
                    k.length = 0;
                    k.string = gtk::glib::ffi::g_strdup(c"".as_ptr());
                    if let Some((hardware, group)) = code {
                        k.hardware_keycode = hardware;
                        k.group = group;
                    }
                    gdk_ffi::gdk_event_set_device(e, keyboard.to_glib_none().0);
                    gdk_ffi::gdk_event_set_source_device(e, keyboard.to_glib_none().0);
                    e
                }
            };
            gtk::ffi::gtk_widget_event(widget.to_glib_none().0, made);
            gdk_ffi::gdk_event_free(made);
        }
    }
    Ok(())
}

// ---- WebKit on macOS ----

/// An NSEvent made for a step: a mouse event of its kind at a point with
/// its clicks, or a key's press or release.
#[cfg(target_os = "macos")]
#[derive(Debug)]
enum EngineEvent {
    Mouse { kind: MouseKind, at: [f64; 2], count: isize, flags: usize },
    Key { pressed: bool, code: u16, characters: String, flags: usize },
}

/// What an NSEvent for the mouse is: a button pressed, let go or dragged,
/// or a move with none held.
#[cfg(target_os = "macos")]
#[derive(Clone, Copy, Debug, PartialEq)]
enum MouseKind {
    Down(Button),
    Up(Button),
    Dragged(Button),
    Moved,
}

/// AppKit's modifier flags for `keys`: Ctrl is the Command key.
#[cfg(target_os = "macos")]
fn ns_flags(keys: Keys) -> usize {
    let bit = |on: bool, shift: u32| if on { 1usize << shift } else { 0 };
    bit(keys.shift, 17) | bit(keys.alt, 19) | bit(keys.ctrl, 20)
}

/// AppKit's events for `step` at `now`: a move or a drag to a press's or a
/// release's point when the pointer is elsewhere, then the button's event,
/// its clicks counted by the system's double-click interval; a move or a
/// drag; or a key's press and release.
#[cfg(target_os = "macos")]
fn engine_events(step: &Step, hand: &mut Hand, _scale: f64, now: Instant) -> Result<Vec<EngineEvent>, String> {
    use objc2_app_kit::NSEvent;
    let mut out = Vec::new();
    let movement = |hand: &Hand| match hand.first_held() {
        Some(b) => MouseKind::Dragged(b),
        None => MouseKind::Moved,
    };
    match step {
        Step::Press { button, at, keys } | Step::Release { button, at, keys } => {
            let pressed = matches!(step, Step::Press { .. });
            if hand.at != Some(*at) {
                out.push(EngineEvent::Mouse { kind: movement(hand), at: *at, count: 0, flags: ns_flags(*keys) });
            }
            let count = if pressed {
                let interval = Duration::try_from_secs_f64(NSEvent::doubleClickInterval()).map_err(|e| format!("the system's double-click interval is no length of time: {e}"))?;
                hand.count(*button, *at, now, interval, None)
            } else {
                hand.released_count(*button)
            };
            moved(hand, step);
            let kind = if pressed { MouseKind::Down(*button) } else { MouseKind::Up(*button) };
            out.push(EngineEvent::Mouse { kind, at: *at, count: isize::try_from(count).map_err(|e| format!("a press of {count} clicks: {e}"))?, flags: ns_flags(*keys) });
        }
        Step::Move { at, keys } => {
            moved(hand, step);
            out.push(EngineEvent::Mouse { kind: movement(hand), at: *at, count: 0, flags: ns_flags(*keys) });
        }
        Step::Key { key, keys } => {
            let (code, characters) = mac_key(key, keys.shift)?;
            out.push(EngineEvent::Key { pressed: true, code, characters: characters.clone(), flags: ns_flags(*keys) });
            out.push(EngineEvent::Key { pressed: false, code, characters, flags: ns_flags(*keys) });
        }
        Step::Wait { .. } => {}
    }
    Ok(out)
}

/// The virtual key code of the key a step names on a Mac's keyboard, as
/// Carbon's kVK constants number them, and the characters its event
/// carries, with Shift held as `shift` says: AppKit's function-key
/// characters for keys that type none.
#[cfg(target_os = "macos")]
fn mac_key(name: &str, shift: bool) -> Result<(u16, String), String> {
    const LETTERS: [u16; 26] = [0x00, 0x0B, 0x08, 0x02, 0x0E, 0x03, 0x05, 0x04, 0x22, 0x26, 0x28, 0x25, 0x2E, 0x2D, 0x1F, 0x23, 0x0C, 0x0F, 0x01, 0x11, 0x20, 0x09, 0x0D, 0x07, 0x10, 0x06];
    const DIGITS: [u16; 10] = [0x1D, 0x12, 0x13, 0x14, 0x15, 0x17, 0x16, 0x1A, 0x1C, 0x19];
    const F_KEYS: [u16; 20] = [0x7A, 0x78, 0x63, 0x76, 0x60, 0x61, 0x62, 0x64, 0x65, 0x6D, 0x67, 0x6F, 0x69, 0x6B, 0x71, 0x6A, 0x40, 0x4F, 0x50, 0x5A];
    let function = |c: u32| char::from_u32(c).map(String::from).ok_or_else(|| format!("{c:#x} is no character"));
    if let [c] = name.as_bytes()
        && c.is_ascii_alphabetic()
    {
        let typed = if shift { c.to_ascii_uppercase() } else { c.to_ascii_lowercase() };
        return Ok((LETTERS[usize::from(c.to_ascii_uppercase() - b'A')], char::from(typed).to_string()));
    }
    if let [c] = name.as_bytes()
        && c.is_ascii_digit()
    {
        return Ok((DIGITS[usize::from(c - b'0')], char::from(*c).to_string()));
    }
    if let Some(n) = f_key(name) {
        let Some(code) = F_KEYS.iter().zip(1u32..).find(|(_, number)| *number == n).map(|(code, _)| *code) else {
            return Err(format!("a Mac's keyboard has no {name}; its F keys end at F20"));
        };
        return Ok((code, function(0xF703 + n)?));
    }
    Ok(match name {
        "Enter" => (0x24, String::from("\r")),
        "Escape" => (0x35, String::from("\u{1b}")),
        "Tab" => (0x30, String::from("\t")),
        "Space" => (0x31, String::from(" ")),
        "Backspace" => (0x33, String::from("\u{7f}")),
        "Delete" => (0x75, function(0xF728)?),
        "Insert" => (0x72, function(0xF727)?),
        "Home" => (0x73, function(0xF729)?),
        "End" => (0x77, function(0xF72B)?),
        "PageUp" => (0x74, function(0xF72C)?),
        "PageDown" => (0x79, function(0xF72D)?),
        "ArrowLeft" => (0x7B, function(0xF702)?),
        "ArrowUp" => (0x7E, function(0xF700)?),
        "ArrowRight" => (0x7C, function(0xF703)?),
        "ArrowDown" => (0x7D, function(0xF701)?),
        "Plus" => (0x45, String::from("+")),
        "Minus" => (0x1B, String::from("-")),
        "Equals" => (0x18, String::from("=")),
        proto::test_input::MENU => (0x6E, function(0xF735)?),
        other => return Err(format!("WebKit has no key named {other}")),
    })
}

/// Hands each of `events` to the WKWebView as AppKit hands it a person's,
/// in its window's coordinates, the clicks and the time AppKit gives them;
/// `sent` runs once the view has them all.
#[cfg(target_os = "macos")]
fn dispatch(webview: &WebView, events: Vec<EngineEvent>, sent: Sent) {
    sent(handed(webview, events));
}

#[cfg(target_os = "macos")]
fn handed(webview: &WebView, events: Vec<EngineEvent>) -> Result<(), String> {
    use objc2_app_kit::{NSEvent, NSEventModifierFlags, NSEventType};
    use objc2_foundation::{NSPoint, NSProcessInfo, NSString};
    use wry::WebViewExtMacOS;
    let view = webview.webview();
    let window = view.window().ok_or("the webview is in no window to take input in, as before it is shown")?;
    let number = window.windowNumber();
    let bounds = view.bounds();
    let flipped = view.isFlipped();
    let in_window = |at: [f64; 2]| view.convertPoint_toView(NSPoint::new(at[0], if flipped { at[1] } else { bounds.size.height - at[1] }), None);
    let mut last = NSPoint::new(0.0, 0.0);
    for (numbered, event) in (0isize..).zip(events) {
        let time = NSProcessInfo::processInfo().systemUptime();
        match event {
            EngineEvent::Mouse { kind, at, count, flags } => {
                let location = in_window(at);
                last = location;
                let (made, pressure) = match kind {
                    MouseKind::Down(Button::Left) => (NSEventType::LeftMouseDown, 1.0),
                    MouseKind::Down(Button::Right) => (NSEventType::RightMouseDown, 1.0),
                    MouseKind::Down(Button::Middle) => (NSEventType::OtherMouseDown, 1.0),
                    MouseKind::Up(Button::Left) => (NSEventType::LeftMouseUp, 0.0),
                    MouseKind::Up(Button::Right) => (NSEventType::RightMouseUp, 0.0),
                    MouseKind::Up(Button::Middle) => (NSEventType::OtherMouseUp, 0.0),
                    MouseKind::Dragged(Button::Left) => (NSEventType::LeftMouseDragged, 1.0),
                    MouseKind::Dragged(Button::Right) => (NSEventType::RightMouseDragged, 1.0),
                    MouseKind::Dragged(Button::Middle) => (NSEventType::OtherMouseDragged, 1.0),
                    MouseKind::Moved => (NSEventType::MouseMoved, 0.0),
                };
                let e = NSEvent::mouseEventWithType_location_modifierFlags_timestamp_windowNumber_context_eventNumber_clickCount_pressure(made, location, NSEventModifierFlags::from_bits_retain(flags), time, number, None, numbered, count, pressure)
                    .ok_or("AppKit made no mouse event for the step")?;
                match kind {
                    MouseKind::Down(Button::Left) => view.mouseDown(&e),
                    MouseKind::Down(Button::Right) => view.rightMouseDown(&e),
                    MouseKind::Down(Button::Middle) => view.otherMouseDown(&e),
                    MouseKind::Up(Button::Left) => view.mouseUp(&e),
                    MouseKind::Up(Button::Right) => view.rightMouseUp(&e),
                    MouseKind::Up(Button::Middle) => view.otherMouseUp(&e),
                    MouseKind::Dragged(Button::Left) => view.mouseDragged(&e),
                    MouseKind::Dragged(Button::Right) => view.rightMouseDragged(&e),
                    MouseKind::Dragged(Button::Middle) => view.otherMouseDragged(&e),
                    MouseKind::Moved => moved_over(&view, &e)?,
                }
            }
            EngineEvent::Key { pressed, code, characters, flags } => {
                let text = NSString::from_str(&characters);
                let made = if pressed { NSEventType::KeyDown } else { NSEventType::KeyUp };
                let e = NSEvent::keyEventWithType_location_modifierFlags_timestamp_windowNumber_context_characters_charactersIgnoringModifiers_isARepeat_keyCode(made, last, NSEventModifierFlags::from_bits_retain(flags), time, number, None, &text, &text, false, code)
                    .ok_or("AppKit made no key event for the step")?;
                if pressed {
                    view.keyDown(&e);
                } else {
                    view.keyUp(&e);
                }
            }
        }
    }
    Ok(())
}

/// Hands `event`, a move, to WebKit as AppKit hands it a person's: to the
/// owners of the view's tracking areas that follow moves, since a WKWebView
/// has no mouseMoved: of its own. Refused when no area follows moves.
#[cfg(target_os = "macos")]
fn moved_over(view: &objc2_app_kit::NSView, event: &objc2_app_kit::NSEvent) -> Result<(), String> {
    use objc2::runtime::Bool;
    use objc2::{msg_send, sel};
    use objc2_app_kit::NSTrackingAreaOptions;
    let mut given = false;
    for area in view.trackingAreas().iter() {
        if !area.options().contains(NSTrackingAreaOptions::MouseMoved) {
            continue;
        }
        let Some(owner) = area.owner() else {
            continue;
        };
        // SAFETY: respondsToSelector: is NSObject's, which every owner AppKit takes is.
        let follows: Bool = unsafe { msg_send![&*owner, respondsToSelector: sel!(mouseMoved:)] };
        if follows.as_bool() {
            // SAFETY: the owner answers mouseMoved:, whose one argument is an NSEvent.
            let () = unsafe { msg_send![&*owner, mouseMoved: event] };
            given = true;
        }
    }
    if given { Ok(()) } else { Err(String::from("the webview's tracking areas follow no moves, so a move has nowhere to go")) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(any(windows, target_os = "macos"))]
    #[test]
    fn a_press_counts_its_clicks_from_the_press_before_as_the_system_does() {
        let mut hand = Hand::default();
        let start = Instant::now();
        let at = |ms: u64| start + Duration::from_millis(ms);
        let interval = Duration::from_millis(500);
        let slop = Some([2.0, 2.0]);
        assert_eq!(hand.count(Button::Left, [10.0, 10.0], at(0), interval, slop), 1);
        assert_eq!(hand.count(Button::Left, [11.0, 10.0], at(200), interval, slop), 2, "near in place and time");
        assert_eq!(hand.count(Button::Left, [11.0, 11.0], at(400), interval, slop), 3);
        assert_eq!(hand.released_count(Button::Left), 3, "a release is the clicks of its press");
        assert_eq!(hand.count(Button::Left, [20.0, 10.0], at(500), interval, slop), 1, "too far");
        assert_eq!(hand.count(Button::Right, [20.0, 10.0], at(600), interval, slop), 1, "another button");
        assert_eq!(hand.count(Button::Right, [20.0, 10.0], at(1200), interval, slop), 1, "too late");
        assert_eq!(hand.count(Button::Right, [40.0, 10.0], at(1300), interval, None), 2, "no slop where the system counts none");
        assert_eq!(hand.released_count(Button::Left), 1, "a release of a button no press holds counts one");
    }

    #[test]
    fn a_press_holds_its_button_until_its_release_and_moves_the_pointer() {
        let mut hand = Hand::default();
        let keys = Keys::default();
        moved(&mut hand, &Step::Press { button: Button::Right, at: [5.0, 6.0], keys });
        assert_eq!((hand.held(), hand.at), ([false, true, false], Some([5.0, 6.0])));
        moved(&mut hand, &Step::Move { at: [7.0, 6.0], keys });
        moved(&mut hand, &Step::Press { button: Button::Left, at: [7.0, 6.0], keys });
        assert_eq!(hand.held(), [true, true, false]);
        moved(&mut hand, &Step::Release { button: Button::Left, at: [7.0, 6.0], keys });
        moved(&mut hand, &Step::Release { button: Button::Right, at: [8.0, 6.0], keys });
        assert_eq!((hand.held(), hand.at), ([false; 3], Some([8.0, 6.0])));
        assert_eq!((f_key("F10"), f_key("F24"), f_key("F25"), f_key("F01"), f_key("Menu")), (Some(10), Some(24), None, None, None));
    }

    #[test]
    fn a_step_gives_the_page_the_buttons_held_before_it_and_after_it_as_a_pointer_event_s_bits() {
        let keys = Keys::default();
        let at = [5.0, 6.0];
        let right = Step::Press { button: Button::Right, at, keys };
        assert_eq!(held_around(&right, [false; 3]), [0, 2], "a press of Right from nothing");
        assert_eq!(held_around(&Step::Release { button: Button::Right, at, keys }, [false, true, false]), [2, 0], "its release");
        assert_eq!(held_around(&Step::Press { button: Button::Left, at, keys }, [false, false, true]), [4, 5], "Left with Middle held");
        assert_eq!(held_around(&Step::Move { at, keys }, [true, true, true]), [7, 7], "a move keeps what is held");
        assert_eq!(held_for_page(&right, [false; 3]), cfg!(target_os = "macos").then_some([0, 2]), "only WebKit on macOS reads them from the page");
    }

    #[cfg(windows)]
    #[test]
    fn a_step_s_keys_are_the_keys_windows_and_the_devtools_protocol_name() {
        let menu = windows_key(proto::test_input::MENU, false).expect("the Menu key");
        assert_eq!((menu.key.as_str(), menu.place.as_str(), menu.code), ("ContextMenu", "ContextMenu", 0x5D));
        let f10 = windows_key("F10", true).expect("F10");
        assert_eq!((f10.key.as_str(), f10.code, f10.text), ("F10", 0x79, None));
        let a = windows_key("A", true).expect("A");
        assert_eq!((a.key.as_str(), a.place.as_str(), a.code, a.text.as_deref()), ("A", "KeyA", 0x41, Some("A")));
        let one = windows_key("1", true).expect("1");
        assert_eq!((one.key.as_str(), one.place.as_str()), ("!", "Digit1"));
        assert!(windows_key("Hyper", false).is_err());
        for written in proto::Shortcut::KEYS {
            assert!(windows_key(written, false).is_ok(), "{written}");
        }
    }

    #[cfg(windows)]
    #[test]
    fn a_right_click_moves_the_mouse_there_first_and_counts_its_click() {
        let mut hand = Hand::default();
        let keys = Keys { shift: false, ctrl: true, alt: false };
        let read = |e: &EngineEvent| serde_json::from_str::<serde_json::Value>(&e.1).expect("JSON parameters");
        let events = |step: &Step, hand: &mut Hand| engine_events(step, hand, 1.0, Instant::now()).expect("the step's calls");
        let press = events(&Step::Press { button: Button::Right, at: [30.0, 40.0], keys }, &mut hand);
        assert_eq!(press.len(), 2, "moved, then pressed: {press:?}");
        let (moved_to, pressed) = (read(&press[0]), read(&press[1]));
        assert_eq!((moved_to["type"].as_str(), moved_to["x"].as_f64()), (Some("mouseMoved"), Some(30.0)));
        assert_eq!((pressed["type"].as_str(), pressed["button"].as_str(), pressed["buttons"].as_u64(), pressed["clickCount"].as_u64(), pressed["modifiers"].as_u64()), (Some("mousePressed"), Some("right"), Some(2), Some(1), Some(2)));
        let release = events(&Step::Release { button: Button::Right, at: [30.0, 40.0], keys }, &mut hand);
        assert_eq!(release.len(), 1, "no move where the pointer is already");
        let released = read(&release[0]);
        assert_eq!((released["type"].as_str(), released["buttons"].as_u64(), released["clickCount"].as_u64()), (Some("mouseReleased"), Some(0), Some(1)));
        let menu = events(&Step::Key { key: String::from(proto::test_input::MENU), keys: Keys::default() }, &mut hand);
        let kinds: Vec<String> = menu.iter().map(|e| read(e)["type"].as_str().map_or_else(String::new, String::from)).collect();
        assert_eq!(kinds, ["rawKeyDown", "keyUp"]);
    }
}
