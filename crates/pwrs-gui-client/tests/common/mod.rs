//! What every end-to-end test here shares: no test can hang. Every wait is
//! armed with a deadline that cancels it through the same canceler a
//! Ctrl+C uses, and every test runs under a watchdog that ends the process
//! if the test as a whole stalls; the helper's job object ends the helper
//! with it. Each step is printed, so a failure shows where it stopped.
//! Needs a display and the helpers built for the same profile:
//! `cargo build --profile test-fast -p pwrs-gui --bin pwrs-gui-egui --bin pwrs-gui-web`.

#![allow(dead_code)]

use pwrs_gui_client::proto::{CloseReason, EventKind, Kind, Node, Value, WindowOptions};
use pwrs_gui_client::{Canceler, GuiError, Mode, Renderer, Session, StartOptions, WebViewData, Window, WindowEvent};
use std::io::Write;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::thread::Thread;
use std::time::{Duration, Instant};

/// How long one wait may take before the test cancels it.
pub const WAIT: Duration = Duration::from_secs(20);
/// The number of the next file a drive watches. Two sessions driven at once
/// in one process number their windows alike, so a file is named by this,
/// not by a window.
static WATCHED: AtomicUsize = AtomicUsize::new(0);
/// How long one test may take before its watchdog ends the process.
pub const TEST: Duration = Duration::from_secs(120);

/// Ends the process when the test it guards outlives its deadline.
pub struct Watchdog {
    done: Arc<AtomicBool>,
    thread: Thread,
}

impl Watchdog {
    pub fn start(test: &'static str) -> Watchdog {
        let done = Arc::new(AtomicBool::new(false));
        let watching = Arc::clone(&done);
        let handle = std::thread::spawn(move || {
            let until = Instant::now() + TEST;
            while !watching.load(Ordering::SeqCst) {
                let left = until.saturating_duration_since(Instant::now());
                if left.is_zero() {
                    eprintln!("{test}: stalled for {TEST:?}; the last step printed above is where. Ending the test process.");
                    std::process::exit(101);
                }
                std::thread::park_timeout(left);
            }
        });
        Watchdog { done, thread: handle.thread().clone() }
    }
}

impl Drop for Watchdog {
    fn drop(&mut self) {
        self.done.store(true, Ordering::SeqCst);
        self.thread.unpark();
    }
}

/// A wait's deadline: cancels the wait it was armed for unless dropped
/// first.
pub struct Deadline {
    done: Arc<AtomicBool>,
    thread: Thread,
}

impl Drop for Deadline {
    fn drop(&mut self) {
        self.done.store(true, Ordering::SeqCst);
        self.thread.unpark();
    }
}

/// An arm that cancels the wait after [`WAIT`].
pub fn within(canceler: Canceler) -> Result<Deadline, GuiError> {
    let done = Arc::new(AtomicBool::new(false));
    let watching = Arc::clone(&done);
    let handle = std::thread::spawn(move || {
        let until = Instant::now() + WAIT;
        while !watching.load(Ordering::SeqCst) {
            let left = until.saturating_duration_since(Instant::now());
            if left.is_zero() {
                eprintln!("    a wait passed {WAIT:?}; canceling it");
                canceler.cancel();
                return;
            }
            std::thread::park_timeout(left);
        }
    });
    Ok(Deadline { done, thread: handle.thread().clone() })
}

pub fn step(what: &str) {
    eprintln!("  step: {what}");
}

/// The window's next event, within [`WAIT`].
pub fn next(window: &mut Window) -> WindowEvent {
    let deadline = within(window.canceler());
    let event = window.next_event();
    drop(deadline);
    match event {
        Ok(e) => e,
        Err(GuiError::Canceled) => panic!("window {} raised nothing within {WAIT:?}", window.id()),
        Err(e) => panic!("window {}: {e}", window.id()),
    }
}

/// Shuts the session down when the test ends, passing or failing.
pub struct Ends(pub Session);

impl Drop for Ends {
    fn drop(&mut self) {
        self.0.shut_down();
    }
}

/// The renderer's helper beside this test's profile folder.
pub fn helper(renderer: Renderer) -> Result<PathBuf, String> {
    let exe = std::env::current_exe().map_err(|e| format!("this test's path cannot be read: {e}"))?;
    let profile = exe.parent().and_then(|deps| deps.parent()).ok_or("the test binary has no profile folder")?;
    let base = renderer.helper_name();
    let path = profile.join(if cfg!(windows) { format!("{base}.exe") } else { base.to_string() });
    if path.is_file() {
        Ok(path)
    } else {
        Err(format!("{} is not built; build it with cargo build -p pwrs-gui --bin {base} and the same profile", path.display()))
    }
}

/// The egui renderer, with a log folder of its own, which the first run on
/// a machine has to create.
pub fn options(mode: Mode) -> StartOptions {
    options_for(mode, Renderer::Egui)
}

pub fn options_for(mode: Mode, renderer: Renderer) -> StartOptions {
    StartOptions {
        mode,
        renderer,
        helper: helper(renderer),
        adapter: None,
        low_power: false,
        log_dir: std::env::temp_dir().join("pwrs-gui-session-tests"),
        web_view: WebViewData::Shared,
        web_view_data_folder: None,
        fonts: pwrs_gui_client::proto::Fonts::Bundled,
        environment: None,
    }
}

/// The next value `window`'s page sends, within [`WAIT`], replied to with
/// null.
pub fn page_message(window: &mut Window) -> String {
    loop {
        match next(window) {
            WindowEvent::Message { call, json } => {
                window.reply(call, String::from("null")).expect("reply to the page's message");
                return json;
            }
            WindowEvent::Closed(reason) => panic!("window {} closed while its page was to speak: {reason:?}", window.id()),
            _ => {}
        }
    }
}

/// A page of the caller's own whose script `script` runs as it loads.
pub fn page(script: &str) -> Node {
    let html = format!("<!doctype html><html><body><p>PWRS-GUI automated test: please leave this window alone</p><script>{script}</script></body></html>");
    Node::new(1, Kind::Html).prop("html", Value::Str(html))
}

/// The ids of the WebView2 processes running on the data folder `folder`:
/// the browser process, or with `renderer` the renderer processes. With
/// `kill` they are ended first.
#[cfg(windows)]
pub fn webview_processes(folder: &std::path::Path, renderer: bool, kill: bool) -> Vec<u32> {
    let kind = if renderer { "$_.CommandLine -like '*--type=renderer*'" } else { "$_.CommandLine -notlike '*--type=*'" };
    let act = if kill { "Stop-Process -Id $_.ProcessId -Force; " } else { "" };
    let query = format!(
        "Get-CimInstance Win32_Process | Where-Object {{ $_.Name -eq 'msedgewebview2.exe' -and $_.CommandLine -like '*{}*' -and {kind} }} | ForEach-Object {{ {act}$_.ProcessId }}",
        folder.display()
    );
    let out = std::process::Command::new("powershell.exe").args(["-NoProfile", "-Command", &query]).output().expect("powershell runs");
    assert!(out.status.success(), "the process query failed with {}: {}", out.status, String::from_utf8_lossy(&out.stderr));
    String::from_utf8_lossy(&out.stdout).split_whitespace().map(|p| p.parse().expect("a process id")).collect()
}

/// Waits until no WebView2 process runs on `folder`, within [`WAIT`].
#[cfg(windows)]
pub fn webview_gone(folder: &std::path::Path) {
    let until = Instant::now() + WAIT;
    loop {
        let browser = webview_processes(folder, false, false);
        let renderers = webview_processes(folder, true, false);
        if browser.is_empty() && renderers.is_empty() {
            return;
        }
        assert!(Instant::now() < until, "WebView2 processes {browser:?} and {renderers:?} still run on {} after {WAIT:?}", folder.display());
        std::thread::sleep(Duration::from_millis(200));
    }
}

pub fn choice_tree() -> Node {
    let items = Value::List(vec![Value::Str("alpha".into()), Value::Str("beta".into()), Value::Str("gamma".into())]);
    Node::new(1, Kind::Column)
        .child(Node::new(2, Kind::Text).prop("text", Value::Str("PWRS-GUI automated test: please leave this window alone".into())))
        .child(Node::new(3, Kind::ListBox).named("Choice").prop("items", items))
        .child(Node::new(4, Kind::Row).child(Node::new(5, Kind::Button).named("Ok").prop("text", Value::Str("OK".into()))))
}

/// Drives a window of `session` through what a session does with one:
/// events by name, snapshots with and without pixels, the keyboard focus,
/// the window and adapter lists, a wait canceled from another thread, and
/// closing from the window and from the session. Its pixel snapshots need its window
/// uncovered, so no other test of the same binary may open one at the same
/// time: on a display with no window manager every window opens at the
/// same place, and a covered one is not drawn.
pub fn drive(session: &Session, renderer: Renderer) {
    step("opening a window");
    let mut window = session.open_window(WindowOptions::titled("PWRS-GUI automated test"), choice_tree()).expect("open a window");
    let id = window.id();
    step("waiting for its first frame");
    assert_eq!(next(&mut window), WindowEvent::Opened);

    step("selecting row 1 by name");
    let took = session.test_event(id, "choice", EventKind::Select, Value::Int(1), within).expect("select a row by name");
    assert_eq!(took, Value::List(vec![Value::Int(1)]));
    step("reading the selection event");
    match next(&mut window) {
        WindowEvent::Event { node, kind, value, at } => {
            assert_eq!((node, kind), (3, EventKind::Select));
            assert_eq!(value, Value::List(vec![Value::Int(1)]));
            assert!(at > 0, "the event gives when the renderer took it");
        }
        other => panic!("expected the selection, read {other:?}"),
    }

    step("clicking OK by name");
    session.test_event(id, "OK", EventKind::Click, Value::Null, within).expect("click OK by name");
    step("reading the click event");
    match next(&mut window) {
        WindowEvent::Event { node, kind, .. } => assert_eq!((node, kind), (5, EventKind::Click)),
        other => panic!("expected the click, read {other:?}"),
    }

    step("acting on a node that does not exist");
    let refused = session.test_event(id, "Missing", EventKind::Click, Value::Null, within);
    assert!(matches!(refused, Err(GuiError::Refused(ref why)) if why.contains("Missing")), "{refused:?}");

    step("taking a snapshot with pixels");
    let shot = session.snapshot(id, None, true, within).expect("a snapshot with pixels");
    let state = shot.state.get(&3).expect("the list's state");
    assert_eq!(state, &Value::List(vec![Value::Int(1)]));
    let pixels = shot.pixels.expect("the window's pixels");
    assert!(pixels.width > 0 && pixels.height > 0);
    assert_eq!(pixels.rgba.len(), (pixels.width * pixels.height * 4) as usize);

    step("taking a snapshot of one widget");
    let button = session.snapshot(id, Some("Ok".into()), true, within).expect("a snapshot of one widget");
    let crop = button.pixels.expect("the button's pixels");
    assert!(crop.width < pixels.width && crop.height < pixels.height, "the widget is cropped from the window");

    step("taking a snapshot of a node that does not exist");
    let refused = session.snapshot(id, Some("Missing".into()), true, within);
    assert!(matches!(refused, Err(GuiError::Refused(ref why)) if why.contains("Missing") && why.contains("Choice")), "{refused:?}");

    step("giving the list the keyboard focus");
    session.focus_node(id, 3, within).expect("focus the list");
    let shot = session.snapshot(id, None, false, within).expect("a snapshot after the focus");
    assert_eq!(shot.focused, Some(3), "the snapshot names the list as focused");

    step("moving the keyboard focus to OK");
    session.focus_node(id, 5, within).expect("focus OK");
    let shot = session.snapshot(id, None, false, within).expect("a snapshot after the focus moved");
    assert_eq!(shot.focused, Some(5), "the snapshot names OK as focused");

    step("giving the focus to a Text, which takes none");
    let refused = session.focus_node(id, 2, within);
    assert!(matches!(refused, Err(GuiError::Refused(ref why)) if why.contains("Text")), "{refused:?}");

    step("giving the focus to a node the window does not have");
    let refused = session.focus_node(id, 99, within);
    assert!(matches!(refused, Err(GuiError::Refused(ref why)) if why.contains("99")), "{refused:?}");

    step("listing the open windows");
    let windows = session.windows(within).expect("the open windows");
    let me = windows.iter().find(|w| w.window == id).expect("this window is listed");
    assert_eq!(me.names, vec![String::from("Choice"), String::from("Ok")]);

    step("listing the adapters");
    let adapters = session.adapters(within).expect("the adapters");
    match renderer {
        Renderer::Egui => assert!(!adapters.is_empty(), "the renderer found at least one adapter"),
        Renderer::Html => assert!(adapters.is_empty(), "a webview chooses its own adapter, so the web renderer lists none: {adapters:?}"),
    }

    step("canceling a wait from another thread");
    let canceler = window.canceler();
    let began = Instant::now();
    let canceling = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(200));
        canceler.cancel();
    });
    let outcome = window.next_event();
    canceling.join().expect("the canceling thread");
    assert!(matches!(outcome, Err(GuiError::Canceled)), "{outcome:?}");
    assert!(began.elapsed() < WAIT, "the cancel woke the wait");

    step("closing the window");
    window.close().expect("close the window");

    step("watching a file from a window of its own, whose change comes among the window's events");
    let log = std::env::temp_dir().join(format!("pwrs-gui-client-watch-{}-{}.log", std::process::id(), WATCHED.fetch_add(1, Ordering::Relaxed)));
    std::fs::write(&log, "first\n").expect("the watched file is written");
    let mut watching = session.open_window(WindowOptions::titled("PWRS-GUI automated test, watching window"), choice_tree()).expect("open a watching window");
    assert_eq!(next(&mut watching), WindowEvent::Opened);
    watching.watch_files(vec![log.clone()], None).expect("watch the file");
    let refused = watching.watch_files(vec![log.clone()], None);
    assert!(matches!(refused, Err(GuiError::Watch(ref why)) if why.contains("already")), "{refused:?}");
    {
        let mut more = std::fs::OpenOptions::new().append(true).open(&log).expect("the file opens to be added to");
        more.write_all(b"second\n").expect("a line is added");
    }
    match next(&mut watching) {
        WindowEvent::FileChanged { file: 0, path, length } => {
            assert_eq!(path, log);
            assert!(length.is_some_and(|n| n > 6), "the change gives the file's new length, past its first 6 bytes: {length:?}");
        }
        other => panic!("expected the file's change, read {other:?}"),
    }
    watching.close().expect("close the watching window");
    std::fs::remove_file(&log).expect("the watched file goes");

    step("opening a second window and closing it through the session");
    let mut other = session.open_window(WindowOptions::titled("PWRS-GUI automated test, second window"), choice_tree()).expect("open a second window");
    let other_id = other.id();
    assert_eq!(next(&mut other), WindowEvent::Opened);
    session.close_window(other_id).expect("close the second window through the session");
    assert_eq!(next(&mut other), WindowEvent::Closed(CloseReason::Program), "its reader gets its close");

    step("taking a snapshot of a closed window");
    let refused = session.snapshot(other_id, None, false, within);
    assert!(matches!(refused, Err(GuiError::Refused(ref why)) if why.contains("no open window")), "{refused:?}");

    step("closing a closed window, which the renderer refuses");
    let refused = session.close_open_window(other_id, within);
    assert!(matches!(refused, Err(GuiError::Refused(ref why)) if why.contains(&format!("number {other_id}"))), "{refused:?}");

    step("giving the focus in a closed window, which the renderer refuses");
    let refused = session.focus_node(other_id, 3, within);
    assert!(matches!(refused, Err(GuiError::Refused(ref why)) if why.contains(&format!("number {other_id}"))), "{refused:?}");

    step("closing a third window before its first frame, which the renderer counts as open");
    let mut third = session.open_window(WindowOptions::titled("PWRS-GUI automated test, third window"), choice_tree()).expect("open a third window");
    let third_id = third.id();
    session.close_open_window(third_id, within).expect("the renderer had the third window open");
    let mut seen = next(&mut third);
    if seen == WindowEvent::Opened {
        seen = next(&mut third);
    }
    assert_eq!(seen, WindowEvent::Closed(CloseReason::Program), "its reader gets its close");
}
