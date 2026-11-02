//! PWRS-GUI's HTML renderer. Each window a session opens is a tao window
//! with a wry webview, whose page is the widget runtime loaded from memory
//! through the `pwrs://` protocol: the runtime draws the window's
//! tree, applies patches and rows, and sends what people do back as JSON
//! over the webview's IPC channel. It takes the link every renderer takes,
//! [`proto::renderer::Link`], and reads its messages when its loop is woken
//! with a user event.
//!
//! Nothing here takes a lock: the renderer's state belongs to the closure
//! the event loop runs, on the loop's thread.

#[cfg(test)]
mod canvas_parity;
mod capture;
#[cfg(test)]
mod layout_parity;
mod chrome;
mod data;
#[cfg(any(target_os = "linux", target_os = "dragonfly", target_os = "freebsd", target_os = "openbsd", target_os = "netbsd"))]
mod egl;
mod files;
#[cfg(windows)]
mod fixed;
mod glsl;
mod gpu;
mod input;
mod look;
mod markdown;
mod media;

pub use data::{Data, default_shared_folder};
#[cfg(any(target_os = "linux", target_os = "dragonfly", target_os = "freebsd", target_os = "openbsd", target_os = "netbsd"))]
pub use egl::choose_renderer;
use proto::button_bar::{self, Role};
use proto::renderer::{Inbound, Link, Sink};
use proto::{
    Capabilities, CloseReason, EventKind, FromHelper, Image, Kind, Node, NodeId, PatchOp, RequestId, RingName, Row, Shortcut, SortKey, ToHelper,
    Value, WebGraphics, WindowId, WindowSummary,
};
use pwrs_gui_proto as proto;
use pwrs_gui_proto::{place, trace};
use serde_json::{Map, Number, Value as Json, json};
use arc_swap::ArcSwap;
use std::borrow::Cow;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, TryRecvError};
use std::time::{Duration, Instant};
use tao::event::{Event, StartCause, WindowEvent};
use tao::event_loop::{ControlFlow, EventLoopBuilder, EventLoopProxy, EventLoopWindowTarget};
use tao::platform::run_return::EventLoopExtRunReturn;
use tao::window::{Window, WindowBuilder};
use wry::http::header::{CONTENT_TYPE, HeaderValue};
use wry::http::{Response, StatusCode};
use wry::{PageLoadEvent, WebContext, WebView, WebViewBuilder};

const INDEX: &str = include_str!("runtime/index.html");
const RUNTIME_JS: &str = include_str!("runtime/runtime.js");
/// A canvas's arithmetic, which the runtime draws canvases with.
const CANVAS_JS: &str = include_str!("runtime/canvas.js");
/// A window's layout rules, which the runtime lays its widgets out by.
const LAYOUT_JS: &str = include_str!("runtime/layout.js");
const RUNTIME_CSS: &str = include_str!("runtime/runtime.css");
/// uPlot 1.6.32, which draws a Chart, with its MIT license as the first
/// comment of each of its files, so every copy a page loads includes it.
const UPLOT_JS: &str = concat!("/*\n", include_str!("runtime/uplot/LICENSE"), "*/\n", include_str!("runtime/uplot/uPlot.iife.min.js"));
const UPLOT_CSS: &str = concat!("/*\n", include_str!("runtime/uplot/LICENSE"), "*/\n", include_str!("runtime/uplot/uPlot.min.css"));
/// `window.pwrs`, run before any script of a page.
const BRIDGE: &str = include_str!("runtime/bridge.js");

/// Where the renderer's event loop may run and where its webviews store
/// their data.
#[derive(Clone, Debug, Default)]
pub struct Settings {
    /// Let the event loop run on a thread other than the process's first:
    /// Windows, Linux and FreeBSD; the in-process path sets it.
    pub any_thread: bool,
    /// Where the webviews store their data.
    pub data: Data,
    /// Bring each new window to the front as soon as it is built. Windows
    /// only; the in-process path sets it, since a helper's first window
    /// gets the front from Windows' rule for a newly started process.
    pub bring_to_front: bool,
    /// Lines for the session's log about how the renderer was set up
    /// before it started, written once it has said Hello: on Linux and
    /// FreeBSD, why WebKitGTK draws in software (`choose_renderer`).
    pub log: Vec<String>,
    /// The WebView2 runtimes of the `PWRS-GUI.WebView2.win-x64` add-on.
    /// Windows only.
    pub runtimes: Runtimes,
    /// The fonts pages draw text in.
    pub fonts: proto::Fonts,
}

/// The WebView2 runtimes of the `PWRS-GUI.WebView2.win-x64` add-on, which
/// includes WebView2's Fixed Version runtime.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Runtimes {
    /// The folder of the Fixed Version runtime the webviews run on; `None`
    /// for the evergreen runtime.
    pub fixed: Option<PathBuf>,
    /// The runtime folder of every version of the add-on installed, whose
    /// data folders a start does not remove.
    pub installed: Vec<PathBuf>,
}

/// The engine every webview of this platform runs on.
pub fn engine() -> &'static str {
    if cfg!(windows) {
        "WebView2"
    } else if cfg!(target_vendor = "apple") {
        "WebKit"
    } else {
        "WebKitGTK"
    }
}

/// The engine as `Hello` names it: on a Fixed Version runtime, WebView2's
/// name says so.
fn engine_name(runtimes: &Runtimes) -> String {
    if runtimes.fixed.is_some() { format!("{} Fixed Version", engine()) } else { engine().to_string() }
}

/// Sends `StartFailed` with `text` in place of `Hello`, and returns it.
fn start_failed(sink: &mut Box<dyn Sink + Send>, text: String) -> Result<(), String> {
    sink.hello(FromHelper::StartFailed { text: text.clone() });
    Err(text)
}

/// The version of the engine the webviews run on: the Fixed Version
/// runtime's that `runtimes` names, or else the system's.
#[cfg_attr(not(windows), expect(unused_variables, reason = "a Fixed Version runtime is Windows' alone"))]
fn engine_version(runtimes: &Runtimes) -> Result<String, String> {
    #[cfg(windows)]
    if let Some(folder) = &runtimes.fixed {
        return fixed::version(folder);
    }
    wry::webview_version().map_err(|e| format!("no {} engine was found: {e}", engine()))
}

/// The versions of the add-on's runtimes: the one the webviews run on,
/// `version`, and every installed one's; an installed runtime whose version
/// cannot be read leaves them unknown, which the note says.
#[cfg(windows)]
fn runtime_versions(runtimes: &Runtimes, version: &str) -> (data::Versions, Option<String>) {
    let fixed = runtimes.fixed.as_ref().map(|_| version.to_string());
    match runtimes.installed.iter().map(|folder| fixed::version(folder)).collect::<Result<Vec<String>, String>>() {
        Ok(installed) => (data::Versions { fixed, installed: Some(installed) }, None),
        Err(why) => (data::Versions { fixed, installed: None }, Some(format!("the web data of WebView2 runtimes no longer installed was not looked for: {why}"))),
    }
}

/// Elsewhere there is no add-on runtime.
#[cfg(not(windows))]
fn runtime_versions(_runtimes: &Runtimes, _version: &str) -> (data::Versions, Option<String>) {
    (data::Versions::default(), None)
}

/// The families WebKitGTK takes as its default fonts: those of the faces
/// egui draws each kind of text in, as the egui renderer finds them.
#[cfg(any(target_os = "linux", target_os = "dragonfly", target_os = "freebsd", target_os = "openbsd", target_os = "netbsd"))]
fn default_fonts(faces: &look::Text) -> chrome::DefaultFonts {
    chrome::DefaultFonts { ui: faces.ui.clone(), mono: faces.mono.clone() }
}

/// WebView2 and WebKit take no default fonts from the helper.
#[cfg(not(any(target_os = "linux", target_os = "dragonfly", target_os = "freebsd", target_os = "openbsd", target_os = "netbsd")))]
fn default_fonts(_faces: &look::Text) -> chrome::DefaultFonts {
    chrome::DefaultFonts::default()
}

enum UserEvent {
    /// The inbox has messages.
    Wake,
    /// A window's page sent a message.
    Page(WindowId, String),
    /// A capture a snapshot started has finished.
    Pixels(WindowId, RequestId, Result<Image, String>),
    /// A dialog the session requested has closed: its request, the ring its
    /// answer goes to, and how it was answered.
    DialogDone(RequestId, proto::RingName, Result<proto::DialogAnswer, proto::DialogFailure>),
    /// A window's page finished loading.
    Loaded(WindowId),
    /// A message reached a window's page from somewhere other than its own
    /// origin, which is named.
    Foreign(WindowId, String),
    /// The engine process drawing a window's page ended, as `what` names
    /// it. With `rebuild` the webview itself is gone and is built again;
    /// without, a reload starts another process.
    EngineEnded { window: WindowId, what: String, rebuild: bool },
    /// A window's page has stopped responding.
    #[cfg_attr(target_os = "macos", expect(dead_code, reason = "WebKit on macOS reports no page that stops responding"))]
    Unresponsive(WindowId),
    /// Something for the session's log, from work the loop started.
    Note(String),
    /// Files were dropped on a window's webview, at a place relative to its
    /// top left corner: in pixels on Windows, in points elsewhere.
    Dropped(WindowId, Vec<String>, (i32, i32)),
    /// A file chooser's dialog has closed: its window, its node, and how it
    /// was answered.
    Chosen(WindowId, NodeId, Result<proto::DialogAnswer, proto::DialogFailure>),
    /// A window's engine has taken a step of a test's input, or refused it:
    /// the window, the page's number for the step, and how.
    InputSent(WindowId, u32, Result<(), String>),
}

/// A snapshot the page is reading the state for.
struct Shot {
    reply: RingName,
    pixels: bool,
    /// When the session requested it, which [`CAPTURE_DEADLINE`] counts from.
    asked: Instant,
}

/// A snapshot whose state the page has read, waiting on its capture.
struct Capture {
    reply: RingName,
    state: BTreeMap<NodeId, Value>,
    bounds: BTreeMap<NodeId, proto::Bounds>,
    engine: Option<proto::Engine>,
    /// The widget with the keyboard focus as the page read its state.
    focused: Option<NodeId>,
    /// Whether each password field offered its peek button as the page
    /// read its state.
    peeks: BTreeMap<NodeId, bool>,
    /// The text the window last put on the clipboard as the page read its
    /// state.
    copied: Option<String>,
    /// The sound each media player handed the system as the page read its
    /// state.
    sound: BTreeMap<NodeId, Option<proto::media::Sound>>,
    /// The widget to crop to, in the page's CSS pixels.
    rect: Option<capture::Rect>,
    /// The page's visible size in CSS pixels.
    viewport: (f64, f64),
    /// When the session requested it, which [`CAPTURE_DEADLINE`] counts from.
    asked: Instant,
}

/// How long a capture waits for its page's state and its engine's image
/// before it is refused.
const CAPTURE_DEADLINE: Duration = Duration::from_secs(30);

/// One window's snapshots at `now` against [`CAPTURE_DEADLINE`]: the page's
/// numbers of pixel snapshots whose state it has not given, and the requests
/// whose image the engine has not, each waited the deadline, and when the
/// first of the rest will have.
fn late(shots: &HashMap<u32, (RequestId, Shot)>, captures: &HashMap<RequestId, Capture>, now: Instant) -> (Vec<u32>, Vec<RequestId>, Option<Instant>) {
    let due = |asked: Instant| now.duration_since(asked) >= CAPTURE_DEADLINE;
    let asks = shots.iter().filter(|(_, (_, s))| s.pixels && due(s.asked)).map(|(ask, _)| *ask).collect();
    let requests = captures.iter().filter(|(_, c)| due(c.asked)).map(|(request, _)| *request).collect();
    let waiting = shots.values().filter(|(_, s)| s.pixels).map(|(_, s)| s.asked).chain(captures.values().map(|c| c.asked));
    let next = waiting.filter(|asked| !due(*asked)).min().map(|asked| asked + CAPTURE_DEADLINE);
    (asks, requests, next)
}

/// Runs the renderer until the session sends `Shutdown` or drops its end
/// of the inbox. Sends `StartFailed` on the start ring in place of
/// `Hello` when no engine is found, and returns that error.
pub fn run(link: Link, settings: Settings) -> Result<(), String> {
    let Link { inbox, mut sink, on_ready } = link;
    trace::line(|| String::from("web: run starts; reading the engine's version"));
    let version = match engine_version(&settings.runtimes) {
        Ok(v) => v,
        Err(text) => return start_failed(&mut sink, text),
    };
    let (versions, unswept) = runtime_versions(&settings.runtimes, &version);
    let prepared = match data::prepare(&settings.data, &versions) {
        Ok(p) => p,
        Err(why) => return start_failed(&mut sink, format!("the web data cannot be set up: {why}")),
    };
    #[cfg(windows)]
    let environment = match &settings.runtimes.fixed {
        Some(folder) => match fixed::environment(folder, prepared.folder.as_deref()) {
            Ok(e) => Some(e),
            Err(text) => return start_failed(&mut sink, text),
        },
        None => None,
    };
    let families = proto::typeface::wanted();
    let mut faces = look::text(&families, settings.fonts);
    let fonts = default_fonts(&faces);
    let ui_font: Option<&'static [u8]> = faces.ui_font.take().map(|bytes| &*Box::leak(bytes.into_boxed_slice()));
    let served = ui_font.map(|_| look::ServedUi { tracking: faces.ui_tracking });
    let assets = Arc::new(look::Assets { css: look::stylesheet(&families, &faces.lines, faces.ui_face, served, settings.fonts), ui_font });
    trace::line(|| format!("web: engine {version}, data in {:?}; text in {:?} and {:?}, rows {:?}; building the event loop", prepared.folder, faces.ui, faces.mono, faces.lines));
    let (no_focus, focus_note) = match place::no_focus() {
        Ok(no_focus) => (no_focus, None),
        Err(why) => (false, Some(why)),
    };
    let mut builder = EventLoopBuilder::<UserEvent>::with_user_event();
    if settings.any_thread {
        set_any_thread(&mut builder)?;
    }
    let mut event_loop = builder.build();
    #[cfg(target_os = "macos")]
    if no_focus {
        use tao::platform::macos::EventLoopExtMacOS;
        event_loop.set_activate_ignoring_other_apps(false);
    }
    let hinting = hint_slightly();
    trace::line(|| String::from("web: event loop built"));
    let proxy = event_loop.create_proxy();
    let waker = proxy.clone();
    on_ready(Arc::new(move || match waker.send_event(UserEvent::Wake) {
        Ok(()) => {}
        Err(_the_loop_has_ended) => {}
    }));
    let mut context = WebContext::new(prepared.folder);
    let capabilities = Capabilities { backend: engine_name(&settings.runtimes), adapter: version.clone(), media: Some(engine().to_string()) };
    let build = env!("CARGO_PKG_VERSION").to_string();
    trace::line(|| String::from("web: listing monitors"));
    let (wanted, listed) = (place::wanted(), screens(&event_loop));
    trace::line(|| format!("web: monitors listed: {}", listed.as_ref().map_or_else(|e| e.clone(), |l| l.len().to_string())));
    let mut notes = prepared.notes;
    notes.extend(place::note(&wanted, &listed));
    notes.extend(focus_note);
    notes.extend(hinting);
    if no_focus && on_wayland(&event_loop) {
        notes.push(place::no_focus_on_wayland());
    }
    notes.extend(chrome::floor_note(engine(), &version));
    notes.extend(unswept);
    sink.hello(FromHelper::Hello { protocol: proto::PROTOCOL, build, renderer: String::from("web"), capabilities, notes });
    trace::line(|| String::from("web: hello sent"));
    for text in settings.log.into_iter().chain(families.problems).chain(faces.problems) {
        trace::line(|| format!("web: {text}"));
        sink.log(FromHelper::Log { level: proto::LogLevel::Warning, text });
    }
    let ring_waker = proxy.clone();
    let sources = proto::source::Sources::new(Arc::new(move || match ring_waker.send_event(UserEvent::Wake) {
        Ok(()) => {}
        Err(_the_loop_has_ended) => {}
    }));
    let mut state = State {
        sink,
        proxy,
        windows: BTreeMap::new(),
        closing: false,
        monitor: place::found(&wanted, &listed),
        no_focus,
        served: Arc::new(ArcSwap::from_pointee(HashMap::new())),
        pictures: Arc::new(ArcSwap::from_pointee(HashMap::new())),
        clips: Arc::new(ArcSwap::from_pointee(HashMap::new())),
        assets,
        fonts,
        version,
        #[cfg(windows)]
        environment,
        #[cfg(target_os = "macos")]
        store: prepared.store,
        #[cfg(windows)]
        bring_to_front: settings.bring_to_front,
        sources,
        copies: Arc::new(ArcSwap::from_pointee(HashMap::new())),
        graphics: None,
    };
    event_loop.run_return(|event, target, flow| {
        *flow = ControlFlow::Wait;
        match event {
            Event::NewEvents(StartCause::Init) => {
                #[cfg(target_os = "macos")]
                sweep_stores(state.proxy.clone(), state.store);
                state.read(&inbox, target, &mut context);
            }
            Event::UserEvent(UserEvent::Wake) => state.read(&inbox, target, &mut context),
            Event::UserEvent(UserEvent::Note(text)) => state.sink.log(FromHelper::Log { level: proto::LogLevel::Warning, text }),
            Event::UserEvent(UserEvent::EngineEnded { window, what, rebuild }) => state.engine_ended(window, &what, rebuild, &mut context),
            Event::UserEvent(UserEvent::Unresponsive(window)) => state.unresponsive(window),
            Event::UserEvent(UserEvent::Page(window, body)) => state.page(window, &body),
            Event::UserEvent(UserEvent::Pixels(window, request, image)) => state.pixels(window, request, image),
            Event::UserEvent(UserEvent::DialogDone(request, reply, outcome)) => state.sink.reply(&reply, FromHelper::DialogResult { request, outcome }),
            Event::UserEvent(UserEvent::Loaded(window)) => state.loaded(window),
            Event::UserEvent(UserEvent::Foreign(window, origin)) => state.foreign(window, &origin),
            Event::UserEvent(UserEvent::Dropped(window, paths, at)) => state.dropped(window, &paths, at),
            Event::UserEvent(UserEvent::Chosen(window, node, outcome)) => state.chosen(window, node, outcome),
            Event::UserEvent(UserEvent::InputSent(window, ask, outcome)) => state.input_sent(window, ask, outcome),
            Event::WindowEvent { window_id, event: WindowEvent::CloseRequested, .. } => state.close_requested(window_id),
            _ => {}
        }
        if let Some(at) = state.refuse_late_captures() {
            *flow = ControlFlow::WaitUntil(at);
        }
        if state.closing {
            *flow = ControlFlow::Exit;
        }
    });
    trace::line(|| String::from("web: run ends"));
    Ok(())
}

/// Ends the web helper's process with `code` once its run has returned. On
/// Linux and the BSDs it skips the C libraries' exit handlers and
/// finalizers: Mesa's libgallium runs one at exit while GTK's threads still
/// run, which can crash the process as it ends. The helper writes only its
/// log, on stderr, which has nothing buffered to be flushed.
#[cfg(any(target_os = "linux", target_os = "dragonfly", target_os = "freebsd", target_os = "openbsd", target_os = "netbsd"))]
pub fn end_helper(code: i32) -> ! {
    // SAFETY: _exit takes any status and returns to no caller.
    unsafe { libc::_exit(code) }
}

/// Ends the web helper's process with `code` once its run has returned.
#[cfg(not(any(target_os = "linux", target_os = "dragonfly", target_os = "freebsd", target_os = "openbsd", target_os = "netbsd")))]
pub fn end_helper(code: i32) -> ! {
    std::process::exit(code)
}

/// Gives the renderer's windows a class of their own on Windows. winit and
/// tao both name their window class "Window Class" and register it under
/// the module they are linked into, so in a module with both renderers
/// a window of the one registered second would run the other's window
/// procedure.
#[cfg(windows)]
fn own_class(builder: WindowBuilder) -> WindowBuilder {
    use tao::platform::windows::WindowBuilderExtWindows;
    builder.with_window_classname("PWRS-GUI Web Window")
}

#[cfg(not(windows))]
fn own_class(builder: WindowBuilder) -> WindowBuilder {
    builder
}

/// Requests that Windows make window `window`'s new operating-system window
/// the foreground window. Windows grants the front only to a process that may
/// take it, as pwsh may while a person's command runs in its console;
/// otherwise the window stays where it opened.
#[cfg(windows)]
fn raise_window(window: WindowId, built: &Window) {
    use tao::platform::windows::WindowExtWindows;
    use windows::Win32::Foundation::HWND;
    use windows::Win32::UI::WindowsAndMessaging::SetForegroundWindow;
    let granted = unsafe { SetForegroundWindow(HWND(built.hwnd() as *mut core::ffi::c_void)) }.as_bool();
    trace::line(|| format!("web: window {window} brought to the front: {granted}"));
}

#[cfg(windows)]
fn set_any_thread(builder: &mut EventLoopBuilder<UserEvent>) -> Result<(), String> {
    use tao::platform::windows::EventLoopBuilderExtWindows;
    builder.with_any_thread(true);
    Ok(())
}

#[cfg(any(target_os = "linux", target_os = "freebsd"))]
fn set_any_thread(builder: &mut EventLoopBuilder<UserEvent>) -> Result<(), String> {
    use tao::platform::unix::EventLoopBuilderExtUnix;
    builder.with_any_thread(true);
    Ok(())
}

#[cfg(not(any(windows, target_os = "linux", target_os = "freebsd")))]
fn set_any_thread(_builder: &mut EventLoopBuilder<UserEvent>) -> Result<(), String> {
    Err(String::from("the renderer runs off the process's first thread on Windows, Linux and FreeBSD only"))
}

/// Whether GTK draws on Wayland, whose compositor gives a new window the
/// focus as it chooses; on X11 a window built unfocused opens without it.
#[cfg(any(target_os = "linux", target_os = "freebsd"))]
fn on_wayland(event_loop: &tao::event_loop::EventLoop<UserEvent>) -> bool {
    use tao::platform::unix::EventLoopWindowTargetExtUnix;
    event_loop.is_wayland()
}

#[cfg(not(any(target_os = "linux", target_os = "freebsd")))]
fn on_wayland(_event_loop: &tao::event_loop::EventLoop<UserEvent>) -> bool {
    false
}

/// One open window and the webview drawing it.
struct WebWindow {
    id: WindowId,
    /// The ring its events go to; `None` once detached.
    ring: Option<RingName>,
    title: String,
    tree: Node,
    close_on_escape: bool,
    /// The window is drawn as a dialog, inside a dialog's margin, with or
    /// without a button bar.
    dialog: bool,
    /// The theme the window is drawn in.
    theme: proto::Theme,
    /// While a window of widgets fitting its content is hidden, waiting for
    /// its page to measure it; `None` once it is shown, and for every other
    /// window.
    fitting: Option<Fitting>,
    window: Window,
    webview: WebView,
    /// The window shows a page of the caller's own rather than the widget
    /// runtime drawing its tree.
    page: bool,
    /// The page can take scripts: the runtime has requested its tree, or a
    /// page of the caller's own has loaded.
    ready: bool,
    /// The page has said it opened; the windows a session lists are these.
    opened: bool,
    /// Scripts for the page, run in order once it is ready.
    pending: Vec<String>,
    /// The number the page is given for the next request it replies to.
    /// The page's numbers are JavaScript doubles, exact only below 2^53, so
    /// it is never given a request's own number.
    next_ask: u32,
    /// Requests the page replies to, by the page's number for them, with the
    /// request's own number and the ring its reply goes to.
    tests: HashMap<u32, (RequestId, RingName)>,
    /// Focus requests the page replies to, stored as `tests` are.
    focuses: HashMap<u32, (RequestId, RingName)>,
    /// Calls on media players the page replies to, stored as `tests` are,
    /// with the player each is on.
    media: HashMap<u32, (RequestId, RingName, NodeId)>,
    shots: HashMap<u32, (RequestId, Shot)>,
    /// The scripts requesting from the page the snapshots requested before
    /// it reports the window open, run once it does, so each reads the
    /// window as it is drawn at the size it is shown at.
    on_opened: Vec<String>,
    /// Snapshots waiting on their capture, by request.
    captures: HashMap<RequestId, Capture>,
    /// The page's numbers of pixel snapshots refused at [`CAPTURE_DEADLINE`]
    /// before the page replied, whose replies are dropped when they come.
    late: HashSet<u32>,
    /// A page of the caller's own, stored to build its webview again after
    /// its engine ends; `None` for the widget runtime.
    settings: Option<Page>,
    /// Each table's rows as the session sent them, for a page loaded again.
    rows: BTreeMap<NodeId, TableRows>,
    /// Each input's state as the runtime last reported it, for a page
    /// loaded again.
    inputs: BTreeMap<NodeId, Json>,
    /// What a page of the caller's own last saved with `window.pwrs.save`,
    /// as JSON.
    saved: Option<String>,
    /// The page is being loaded again after its engine ended and has not
    /// drawn since.
    recovering: bool,
    /// The page's command has been told that it stopped responding, and
    /// the page has sent nothing since.
    unresponsive: bool,
    /// The page's `window.pwrs.send` calls its command has yet to reply to.
    calls: Calls,
    /// The pictures its Images show, each with the name a page loads it by.
    pictures: Pictures,
    /// Each file chooser's open dialog, which closing the window closes.
    dialogs: HashMap<NodeId, files::Picking>,
    /// What each GpuSurface's page draws, as the session last gave it and
    /// the page took it, for a page loaded again.
    surfaces: BTreeMap<NodeId, proto::gpu::PageSurface>,
    /// GPU calls the page replies to, by the page's number for them, with
    /// the request's own number, the ring its reply goes to, and what was
    /// requested.
    gpu_asks: HashMap<u32, (RequestId, RingName, PageGpu)>,
    /// Each GpuSurface's camera, as the session set it and the person has
    /// moved it on the page, for its calls and a page loaded again.
    cameras: BTreeMap<NodeId, proto::camera::CameraSpec>,
    /// Each Canvas's items and view, so a change to its items costs what
    /// it changes and a page loaded again draws it as it was.
    canvases: BTreeMap<NodeId, WebCanvas>,
    /// The test's input the page is being given; `None` while none is.
    input: Option<Inputs>,
    /// The pointer as tests' input has left it on the webview.
    hand: input::Hand,
    /// The page's number for the step of a test's input whose session
    /// stopped waiting for it, a report of which the page may still send
    /// and which is dropped; `None` once another input begins.
    stopped: Option<u32>,
}

/// A test's input a window's page is being given, a step at a time: the
/// request and the ring its reply goes to, the steps not yet begun, and the
/// one under way, of `total` steps, `begun` of them begun.
struct Inputs {
    request: RequestId,
    reply: RingName,
    steps: std::collections::VecDeque<proto::test_input::Step>,
    current: Option<Current>,
    total: u32,
    begun: u32,
}

/// The step of a test's input under way: the page's number for it, its
/// number in the input from 1, the step, and whether the engine has taken
/// it and the page has seen it, which together end it. A wait has nothing
/// for the engine to take.
struct Current {
    ask: u32,
    number: u32,
    step: proto::test_input::Step,
    sent: bool,
    seen: bool,
}

/// A step of a test's input as the page waits to see it: a press or a
/// release of a button by the DOM's number for it, at its point; a move to
/// its point; a key; or a wait of its seconds. With `held`, the buttons held
/// before the step and those it leaves held, each as a pointer event's
/// buttons bits, which the page reads in place of an engine's that come from
/// the hardware ([`input::held_for_page`]).
fn step_json(step: &proto::test_input::Step, held: Option<[u32; 2]>) -> Json {
    use proto::test_input::{Button, Step};
    let dom = |button: &Button| match button {
        Button::Left => 0,
        Button::Middle => 1,
        Button::Right => 2,
    };
    let mut given = match step {
        Step::Press { button, at, .. } => json!({ "kind": "press", "button": dom(button), "x": at[0], "y": at[1] }),
        Step::Release { button, at, .. } => json!({ "kind": "release", "button": dom(button), "x": at[0], "y": at[1] }),
        Step::Move { at, .. } => json!({ "kind": "move", "x": at[0], "y": at[1] }),
        Step::Key { .. } => json!({ "kind": "key" }),
        Step::Wait { seconds } => json!({ "kind": "wait", "seconds": seconds }),
    };
    if let (Some([before, after]), Some(fields)) = (held, given.as_object_mut()) {
        fields.insert(String::from("held"), json!({ "before": before, "after": after }));
    }
    given
}

/// A Canvas of a window: its items, read once and changed in place; why
/// they cannot be read, the board then empty; their animations on the
/// helper's clock, which a page loaded again takes up where they are; and
/// its view as the page last drew it, which the page reports at most once a
/// frame while it changes, with whether it is moving, the controls being
/// the session's.
struct WebCanvas {
    board: proto::canvas::Board,
    unread: Option<String>,
    motions: proto::canvas::Motions,
    view: Option<proto::view::ViewShown>,
}

impl WebCanvas {
    /// A canvas of `board`, its animations starting at `now`, with `view`.
    fn of(board: proto::canvas::Board, view: Option<proto::view::ViewShown>, now: Instant) -> WebCanvas {
        let mut motions = proto::canvas::Motions::default();
        motions.started(&board, &board.all(), now);
        WebCanvas { board, unread: None, motions, view }
    }

    /// A canvas whose items cannot be read, for `why`.
    fn unread(why: String) -> WebCanvas {
        WebCanvas { board: proto::canvas::Board::default(), unread: Some(why), motions: proto::canvas::Motions::default(), view: None }
    }
}

/// A GPU call a window's page replies to.
enum PageGpu {
    /// What GpuSurface `node` draws, which the window stores once the page
    /// has taken it.
    Surface { node: NodeId, surface: proto::gpu::PageSurface },
    /// The texels of a frame of a GpuSurface.
    Save,
    /// A view set on Canvas `node` under `controls`, which the window stores
    /// with the view the page replies with.
    View { node: NodeId, controls: proto::view::ViewControls },
}

/// The `window.pwrs.send` calls of a window's page that its command has yet
/// to reply to, by the number the command was given, each with the page's
/// own number. A page loaded again numbers its calls from one again, so the
/// command's numbers are the window's, and none is reused while it is owed.
#[derive(Default)]
struct Calls {
    owed: BTreeMap<u32, u32>,
    last: u32,
}

impl Calls {
    /// The number the page's call `page_call` goes to its command under.
    fn ask(&mut self, page_call: u32) -> u32 {
        loop {
            self.last = self.last.wrapping_add(1);
            if let std::collections::btree_map::Entry::Vacant(free) = self.owed.entry(self.last) {
                free.insert(page_call);
                return self.last;
            }
        }
    }

    /// The page's own number for the command's call `call`, owed no more;
    /// `None` for a call the page now loaded did not make.
    fn answer(&mut self, call: u32) -> Option<u32> {
        self.owed.remove(&call)
    }

    /// The page that made the owed calls is gone.
    fn forget(&mut self) {
        self.owed.clear();
    }
}

/// A table's rows as the session sent them.
#[derive(Default)]
struct TableRows {
    rows: Vec<Json>,
    /// The largest batch the session sent, which a page loaded again gets
    /// them in.
    batch: usize,
}

impl TableRows {
    /// The rows a table opens with.
    fn opening(rows: &[Row]) -> TableRows {
        let mut table = TableRows::default();
        table.take(0, rows_json(rows));
        table
    }

    /// Rows from `start` on replaced by `rows`, as the runtime takes them.
    fn take(&mut self, start: usize, rows: Vec<Json>) {
        self.batch = self.batch.max(rows.len());
        self.rows.truncate(start);
        self.rows.extend(rows);
    }
}

/// `rows` as the runtime takes them: each cell's text and sort key, and
/// `date` for a date, which sorts by its number but stands at the left of
/// its column as text does.
fn rows_json(rows: &[Row]) -> Vec<Json> {
    let cell = |c: &proto::Cell| {
        let mut made = json!({ "text": c.text, "sort": sort_json(&c.sort) });
        if matches!(c.sort, SortKey::Date(_)) {
            made["date"] = Json::Bool(true);
        }
        made
    };
    rows.iter().map(|r| Json::Array(r.cells.iter().map(cell).collect())).collect()
}

/// Where a new window opens: the work area, in points, of the screen it
/// opens on, and the monitor `PWRS_GUI_MONITOR` names, which it is centered
/// on, when one is named and connected.
struct Placing {
    work: [f32; 2],
    on: Option<place::Monitor>,
}

/// How a window is built: its inside size in points, whether it is shown at
/// once, and the monitor it is centered on.
struct Shape<'a> {
    size: [f32; 2],
    visible: bool,
    on: Option<&'a place::Monitor>,
}

/// A window of widgets fitting its content while it is hidden: what it
/// opened with, the work area it fits within, the most each side may take
/// there, which its page lays its widgets out within, and the monitor it is
/// centered on.
struct Fitting {
    options: proto::WindowOptions,
    work: [f32; 2],
    most: [f32; 2],
    on: Option<place::Monitor>,
}

impl Fitting {
    /// The window's inside size for content its page measured as
    /// `measured`, or the most it may take where the page could not measure
    /// it.
    fn size(&self, measured: Option<[f32; 2]>) -> [f32; 2] {
        match measured {
            Some(content) => self.options.fitted(content, Some(self.work)),
            None => self.most,
        }
    }
}

/// What a press of Enter or Escape alone does in a window.
#[derive(Debug, PartialEq, Eq)]
enum RolePress {
    /// Raises Click on the default or cancel button.
    Click(NodeId),
    /// Closes the window, as its close button does.
    Close,
    Nothing,
}

/// What a press of `key`, Enter or Escape, alone does in a window of
/// `tree`, `holder` with the focus, as the egui renderer acts on one: it
/// presses the default button for Enter and the cancel button for Escape,
/// unless that button is disabled or the focused widget uses the key itself
/// ([`button_bar::keeps`]); with no cancel button, Escape closes a window
/// `close_on_escape` lets it close.
fn role_press(tree: &Node, key: &str, holder: Option<NodeId>, close_on_escape: bool) -> Result<RolePress, String> {
    let role = match key {
        "Enter" => Role::Default,
        "Escape" => Role::Cancel,
        other => return Err(format!("the page sent {other:?} as a key that presses a window's button, which only Enter and Escape do")),
    };
    let held = holder.and_then(|id| tree.find(id));
    Ok(match button_bar::role_button(tree, role) {
        Some(b) if matches!(b.props.get("enabled"), Some(Value::Bool(false))) || held.is_some_and(|n| button_bar::keeps(n, key)) => RolePress::Nothing,
        Some(b) => RolePress::Click(b.id),
        None if role == Role::Cancel && close_on_escape => RolePress::Close,
        None => RolePress::Nothing,
    })
}

/// The theme tao draws a window's frame in: the one the window names, or
/// the system's.
fn frame_theme(theme: proto::Theme) -> Option<tao::window::Theme> {
    match theme {
        proto::Theme::System => None,
        proto::Theme::Light => Some(tao::window::Theme::Light),
        proto::Theme::Dark => Some(tao::window::Theme::Dark),
    }
}

/// The theme a runtime page draws its window in, as the page takes it:
/// light, dark, or null for the system's.
fn page_theme(theme: proto::Theme) -> Json {
    match theme {
        proto::Theme::System => Json::Null,
        proto::Theme::Light => json!("light"),
        proto::Theme::Dark => json!("dark"),
    }
}

/// Shows a window built hidden: on macOS, when `no_focus`, ordered front
/// without being made the key window, where tao's show would make it one,
/// and otherwise made key with its application brought to the front, since
/// tao's show alone leaves the application where it is once its launch has
/// passed; elsewhere as tao shows it, which takes no focus for a window
/// built without it.
#[cfg_attr(not(target_os = "macos"), expect(unused_variables, reason = "tao's show takes no focus for a window built without it, but on macOS"))]
fn show_window(window: &Window, no_focus: bool) {
    #[cfg(target_os = "macos")]
    if no_focus {
        use tao::platform::macos::WindowExtMacOS;
        // The helper's loop runs on the main thread, where AppKit's windows
        // are used, and tao's pointer is the window it built.
        let ns_window = unsafe { &*(window.ns_window() as *const objc2_app_kit::NSWindow) };
        ns_window.orderFront(None);
        return;
    }
    window.set_visible(true);
    #[cfg(target_os = "macos")]
    window.set_focus();
}

impl WebWindow {
    /// A number for the page's next reply, unused by any it still owes.
    fn ask(&mut self) -> u32 {
        loop {
            self.next_ask = self.next_ask.wrapping_add(1);
            let ask = self.next_ask;
            let inputs = self.input.as_ref().and_then(|i| i.current.as_ref()).is_some_and(|c| c.ask == ask);
            if !self.tests.contains_key(&ask) && !self.focuses.contains_key(&ask) && !self.media.contains_key(&ask) && !self.shots.contains_key(&ask) && !self.gpu_asks.contains_key(&ask) && !inputs {
                return ask;
            }
        }
    }

    fn script(&mut self, script: String) {
        if self.ready {
            if let Err(e) = self.webview.evaluate_script(&script) {
                eprintln!("pwrs-gui-web: window {} did not take a script: {e}", self.id);
            }
        } else {
            self.pending.push(script);
        }
    }

    /// The scripts that draw the window's tree in a runtime page on
    /// `engine` ([`runtime_scripts`]), its canvases' animations as they
    /// stand now, then give each GpuSurface what it draws, with the
    /// versions of the copies stored in `copies`, and its camera, and each
    /// Canvas its view; last, while the window is hidden until it is
    /// measured, the measure within the most it may take across.
    fn open_scripts(&self, copies: &gpu::Copies, engine: &str) -> Vec<String> {
        let showing = Showing { pictures: &self.pictures, canvases: &self.canvases, now: Instant::now() };
        let mut scripts = runtime_scripts(&self.tree, self.theme, self.fitting.is_some(), self.dialog, &self.inputs, &self.rows, &showing, engine);
        for (node, surface) in &self.surfaces {
            scripts.push(format!("pwrsRuntime.surface({node}, {});", gpu::surface_json(surface, copies)));
        }
        for (node, camera) in &self.cameras {
            scripts.push(format!("pwrsRuntime.camera({node}, {});", gpu::camera_json(camera)));
        }
        for (node, shown) in self.canvases.iter().filter_map(|(node, c)| c.view.as_ref().map(|v| (node, v))) {
            scripts.push(format!("pwrsRuntime.view({node}, {});", view_json(&shown.view)));
        }
        if let Some(f) = &self.fitting {
            scripts.push(format!("pwrsRuntime.measure({});", json!({ "most": f.most })));
        }
        scripts
    }

    /// The window's tree as a snapshot shows it: each Canvas with the items
    /// its board has now, and each password field in `peeks` with `peek`,
    /// whether it offers its peek button.
    fn current_tree(&self, peeks: &BTreeMap<NodeId, bool>) -> Node {
        let mut tree = self.tree.clone();
        current_items(&mut tree, &self.canvases);
        for (id, offered) in peeks {
            if let Some(field) = tree.find_mut(*id) {
                field.props.insert(String::from("peek"), Value::Bool(*offered));
            }
        }
        tree
    }
}

/// Sets each Canvas node's `items` in `tree` to the items its board in
/// `canvases` has; one whose items cannot be read still has those it was
/// given.
fn current_items(tree: &mut Node, canvases: &BTreeMap<NodeId, WebCanvas>) {
    if let Some(c) = canvases.get(&tree.id)
        && c.unread.is_none()
    {
        tree.props.insert(String::from("items"), c.board.to_value());
    }
    for child in &mut tree.children {
        current_items(child, canvases);
    }
}

/// What a node is shown to a page with: the names a page loads its
/// window's pictures by, and its canvases' items, with their animations as
/// they stand at `now`.
struct Showing<'a> {
    pictures: &'a Pictures,
    canvases: &'a BTreeMap<NodeId, WebCanvas>,
    now: Instant,
}

/// The scripts that draw `tree` in a runtime page on `engine`, its name
/// and version: the tree with each input's state in `inputs`, its pictures
/// and canvases as `showing` shows them, the system's button order, its
/// `theme`, whether it is `fitting`, hidden until the page has measured it,
/// whether it opened as a `dialog`, inside a dialog's margin, the
/// protocol's lists of shortcut keys and the numbers a canvas's view moves
/// by, and the engine, which the page names when it refuses what the engine
/// cannot draw; then each table's rows in batches the size of the largest
/// the session sent.
#[allow(clippy::too_many_arguments)]
fn runtime_scripts(tree: &Node, theme: proto::Theme, fitting: bool, dialog: bool, inputs: &BTreeMap<NodeId, Json>, rows: &BTreeMap<NodeId, TableRows>, showing: &Showing, engine: &str) -> Vec<String> {
    let state: Map<String, Json> = inputs.iter().map(|(node, value)| (node.to_string(), value.clone())).collect();
    let open = json!({
        "tree": node_json(tree, showing),
        "windowsOrder": button_bar::WINDOWS_ORDER,
        "theme": page_theme(theme),
        "fitting": fitting,
        "dialog": dialog,
        "state": state,
        "keys": Shortcut::KEYS,
        "fieldCtrlKeys": Shortcut::FIELD_CTRL_KEYS,
        "view": view_numbers(),
        "engine": engine,
    });
    let mut scripts = vec![format!("pwrsRuntime.open({open});")];
    for (node, table) in rows {
        let size = table.batch.max(1);
        for (i, batch) in table.rows.chunks(size).enumerate() {
            scripts.push(format!("pwrsRuntime.rows({node}, {}, {});", i * size, Json::Array(batch.to_vec())));
        }
    }
    scripts
}

/// Everything the renderer stores, owned by the event loop's closure.
struct State {
    sink: Box<dyn Sink + Send>,
    proxy: EventLoopProxy<UserEvent>,
    windows: BTreeMap<WindowId, WebWindow>,
    closing: bool,
    /// The monitor `PWRS_GUI_MONITOR` names, which new windows open centered
    /// on; `None` for where the system places them.
    monitor: Option<u32>,
    /// Whether `PWRS_GUI_NO_FOCUS` opens windows without taking the focus:
    /// none is activated, focuses its webview or is brought to the front.
    no_focus: bool,
    /// The tokens' stylesheet every runtime page loads, and the font it
    /// names that the helper gives the page ([`look::Assets`]).
    assets: Arc<look::Assets>,
    /// The families WebKitGTK takes as its default fonts.
    fonts: chrome::DefaultFonts,
    /// The engine's version, which a snapshot reports.
    version: String,
    /// The environment of the Fixed Version runtime every webview runs in;
    /// `None` for the evergreen runtime, where wry makes one per webview.
    #[cfg(windows)]
    environment: Option<webview2_com::Microsoft::Web::WebView2::Win32::ICoreWebView2Environment>,
    /// Whether a new window is brought to the front as soon as it is built.
    #[cfg(windows)]
    bring_to_front: bool,
    /// What each open window gives its page at `pwrs://`, by its webview's
    /// id, the window's number. The scheme's handler reads it without a lock on
    /// whichever thread the engine calls it on; the loop's thread replaces
    /// it as windows open and close.
    served: Arc<ArcSwap<HashMap<String, Served>>>,
    /// The pictures each open window's Images show, which the scheme's
    /// handler gives pages at `picture/<name>` beside the runtime's files;
    /// replaced as a window's pictures change.
    pictures: Arc<PictureTable>,
    /// The clips each open window's media players play, which the scheme's
    /// handler gives pages at `media/<node>`; replaced as a window's tree
    /// changes.
    clips: Arc<media::Clips>,
    /// The macOS data store every webview takes; `None` for WebKit's
    /// default store.
    #[cfg(target_os = "macos")]
    store: Option<[u8; 16]>,
    /// The SubEtha rings windows' widgets read, each on a thread of its own
    /// that wakes the loop.
    sources: proto::source::Sources,
    /// The copies of the egui renderer's table objects pages' GpuSurfaces
    /// draw from, which the scheme's handler gives pages at
    /// `gpu/<kind>/<number>`.
    copies: Arc<gpu::Copies>,
    /// What the pages draw GpuSurfaces with, as the last page to settle it
    /// reported; `None` before any has.
    graphics: Option<WebGraphics>,
}

impl State {
    /// Handles the session's messages waiting, then what the rings widgets
    /// read have brought.
    fn read(&mut self, inbox: &Receiver<Inbound>, target: &EventLoopWindowTarget<UserEvent>, context: &mut WebContext) {
        loop {
            match inbox.try_recv() {
                Ok(m) => self.handle(m, target, context),
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    self.shut_down();
                    return;
                }
            }
        }
        for news in self.sources.take() {
            self.ring_news(news);
        }
    }

    /// What a widget's ring or shared memory brought: an item becomes the
    /// patch it makes, and a shared memory's rows the changes they make to
    /// its canvas's items, each applied as the session's patches are; an
    /// item or a row the widget cannot take, or a reader that cannot read,
    /// is reported to the window's command, a shared memory's refusal of
    /// its rows once until its rows are taken again.
    fn ring_news(&mut self, news: proto::source::News) {
        use proto::source::{Feed, News};
        let gone = "an item came for a widget the window no longer has";
        let (window, feed, text) = match news {
            News::Item { window, node, item } => {
                let Some(w) = self.windows.get(&window) else {
                    return;
                };
                match w.tree.find(node) {
                    Some(n) => match proto::source::item_ops(n, &item) {
                        Ok(ops) => return self.patch_window(window, ops, "ring"),
                        Err(why) => (window, Feed::Ring, proto::source::refused(n, &why, &item)),
                    },
                    None => (window, Feed::Ring, proto::source::stopped(None, node, Feed::Ring, gone)),
                }
            }
            News::Changes { window, node, item, changes } => {
                let Some(w) = self.windows.get(&window) else {
                    return;
                };
                let board = w.canvases.get(&node).filter(|c| c.unread.is_none()).map(|c| &c.board);
                match w.tree.find(node) {
                    Some(n) => match changes.and_then(|c| proto::source::canvas_ops(n, board, c)) {
                        Ok(ops) => return self.patch_window(window, ops, "ring"),
                        Err(why) => (window, Feed::Ring, proto::source::refused(n, &why, &item)),
                    },
                    None => (window, Feed::Ring, proto::source::stopped(None, node, Feed::Ring, gone)),
                }
            }
            News::Rows { window, node, rows } => {
                let Some(w) = self.windows.get(&window) else {
                    return;
                };
                let Some(n) = w.tree.find(node) else {
                    return;
                };
                let board = w.canvases.get(&node).filter(|c| c.unread.is_none()).map(|c| &c.board);
                let ops = match proto::shared::SharedSource::of(n) {
                    Some(Ok(source)) => proto::shared::canvas_ops(n, board, &source, &rows),
                    Some(Err(why)) => Err(why),
                    None => return,
                };
                let refused = match ops {
                    Ok(ops) => self.apply_patch(window, ops, Feed::SharedMemory.name()).into_iter().next(),
                    Err(why) => Some(why),
                };
                if !self.sources.refusal(window, node, refused.as_deref()) {
                    return;
                }
                let Some(why) = refused else {
                    return;
                };
                let n = self.windows.get(&window).and_then(|w| w.tree.find(node));
                (window, Feed::SharedMemory, proto::source::stopped(n, node, Feed::SharedMemory, &why))
            }
            News::Problem { window, node, feed, text } => {
                let n = self.windows.get(&window).and_then(|w| w.tree.find(node));
                (window, feed, proto::source::stopped(n, node, feed, &text))
            }
        };
        let ring = self.windows.get(&window).and_then(|w| w.ring.clone());
        self.send_window(ring.as_deref(), FromHelper::Error { window: Some(window), context: feed.name().to_string(), text });
    }

    /// Applies `ops` to window `window`'s tree, canvases and page as a
    /// patch from the session is applied, each that fails reported under
    /// `context` and left out of what the page applies, and reads the rings
    /// and shared memory its widgets name once they are applied.
    fn patch_window(&mut self, window: WindowId, ops: Vec<PatchOp>, context: &str) {
        let problems = self.apply_patch(window, ops, context);
        let ring = self.windows.get(&window).and_then(|w| w.ring.clone());
        for text in problems {
            self.send_window(ring.as_deref(), FromHelper::Error { window: Some(window), context: context.to_string(), text });
        }
    }

    /// Applies `ops` to window `window`'s tree, canvases and page as a
    /// patch from the session is applied, each that fails left out of what
    /// the page applies, reads the rings and shared memory its widgets name
    /// once they are applied, and returns why each op that failed did; a
    /// window not open is logged under `context`.
    fn apply_patch(&mut self, window: WindowId, ops: Vec<PatchOp>, context: &str) -> Vec<String> {
        let Some(w) = self.windows.get_mut(&window) else {
            self.log_error(context, format!("no open window {window}"));
            return Vec::new();
        };
        let now = Instant::now();
        let mut problems = Vec::new();
        let showing_pictures = w.pictures.any() || ops.iter().any(shows_pictures);
        let mut page_ops = Vec::with_capacity(ops.len());
        for op in &ops {
            match apply(&mut w.tree, &mut w.canvases, &mut w.pictures, op, now) {
                Ok(written) => {
                    if let PatchOp::RemoveRows { node, rows } = op
                        && let Some(kept) = w.rows.get_mut(node)
                    {
                        proto::remove_rows(&mut kept.rows, rows);
                    }
                    page_ops.push(written);
                }
                Err(e) => problems.push(e),
            }
        }
        w.surfaces.retain(|node, _| w.tree.find(*node).is_some_and(|n| n.kind == Kind::GpuSurface));
        w.cameras.retain(|node, _| w.tree.find(*node).is_some_and(|n| n.kind == Kind::GpuSurface));
        w.canvases.retain(|node, _| w.tree.find(*node).is_some_and(|n| n.kind == Kind::Canvas));
        if showing_pictures && w.pictures.take(&w.tree, &w.canvases) {
            serve_pictures(&self.pictures, window, &w.pictures);
        }
        media::follow(&self.clips, window, &w.tree);
        let script = format!("pwrsRuntime.patch({});", Json::Array(page_ops));
        w.script(script);
        self.sources.follow(window, &w.tree);
        problems
    }

    fn handle(&mut self, inbound: Inbound, target: &EventLoopWindowTarget<UserEvent>, context: &mut WebContext) {
        let Inbound { msg, attach } = inbound;
        if let Some(a) = attach {
            match attached_ring(&msg) {
                Some(ring) => self.sink.attach(ring, a),
                None => self.log_error("attach", format!("an attachment arrived with {msg:?}, which names no ring")),
            }
        }
        match msg {
            ToHelper::OpenWindow { window, ring, options, tree, rows: opened } => {
                // A window of widgets that fits a side to its content is
                // built hidden at the most it may take, and shown at its
                // size once its page has measured it; any other at its own.
                let placing = self.placing(window, target);
                let page = tree.kind == Kind::Html;
                let fitting = (!page && options.fits()).then(|| Fitting {
                    options: options.clone(),
                    work: placing.work,
                    most: options.fitted([f32::INFINITY; 2], Some(placing.work)),
                    on: placing.on.clone(),
                });
                let size = match &fitting {
                    Some(f) => f.most,
                    None if page => options.page_size(Some(placing.work)),
                    None => options.fitted([0.0; 2], Some(placing.work)),
                };
                let shape = Shape { size, visible: fitting.is_none(), on: placing.on.as_ref() };
                match self.open(window, &options, &tree, &shape, target, context) {
                    Ok((w, webview, settings)) => {
                        let (canvases, unread) = open_canvases(&tree, Instant::now());
                        for text in unread {
                            self.sink.window(&ring, FromHelper::Error { window: Some(window), context: String::from("OpenWindow"), text });
                        }
                        let mut web = WebWindow {
                            id: window,
                            ring: Some(ring),
                            title: options.title.clone(),
                            tree,
                            close_on_escape: options.close_on_escape,
                            dialog: options.dialog,
                            theme: options.theme,
                            fitting,
                            window: w,
                            webview,
                            page,
                            ready: false,
                            opened: false,
                            pending: Vec::new(),
                            next_ask: 0,
                            tests: HashMap::new(),
                            focuses: HashMap::new(),
                            media: HashMap::new(),
                            shots: HashMap::new(),
                            on_opened: Vec::new(),
                            captures: HashMap::new(),
                            late: HashSet::new(),
                            settings,
                            rows: opened.iter().map(|(node, rows)| (*node, TableRows::opening(rows))).collect(),
                            inputs: BTreeMap::new(),
                            saved: None,
                            recovering: false,
                            unresponsive: false,
                            calls: Calls::default(),
                            pictures: Pictures::default(),
                            dialogs: HashMap::new(),
                            surfaces: BTreeMap::new(),
                            gpu_asks: HashMap::new(),
                            cameras: BTreeMap::new(),
                            canvases,
                            input: None,
                            hand: input::Hand::default(),
                            stopped: None,
                        };
                        if !page {
                            if web.pictures.take(&web.tree, &web.canvases) {
                                serve_pictures(&self.pictures, window, &web.pictures);
                            }
                            media::follow(&self.clips, window, &web.tree);
                            web.pending = web.open_scripts(&self.copies, &format!("{} {}", engine(), self.version));
                        }
                        self.sources.follow(window, &web.tree);
                        self.windows.insert(window, web);
                    }
                    Err(text) => {
                        self.sink.window(&ring, FromHelper::Error { window: Some(window), context: String::from("OpenWindow"), text });
                        self.sink.window(&ring, FromHelper::Closed { window, reason: CloseReason::Failed });
                        self.sink.forget(&ring);
                    }
                }
            }
            ToHelper::Patch { window, .. } | ToHelper::Rows { window, .. } if self.windows.get(&window).is_some_and(|w| w.page) => {
                let ring = self.windows.get(&window).and_then(|w| w.ring.clone());
                let text = format!("window {window} shows a page of its own, which takes no patches or rows");
                self.send_window(ring.as_deref(), FromHelper::Error { window: Some(window), context: String::from("Patch"), text });
            }
            ToHelper::Patch { window, ops } => self.patch_window(window, ops, "Patch"),
            ToHelper::Rows { window, node, start, rows, added } => match self.windows.get_mut(&window) {
                Some(w) => {
                    let rows = rows_json(&rows);
                    w.rows.entry(node).or_default().take(usize::try_from(start).unwrap_or(usize::MAX), rows.clone());
                    w.script(format!("pwrsRuntime.rows({node}, {start}, {}, {added});", Json::Array(rows)));
                }
                None => self.log_error("Rows", format!("no open window {window}")),
            },
            ToHelper::CloseWindow { window } => self.close(window, CloseReason::Program),
            ToHelper::CloseOpen { request, reply, window } => {
                let outcome = if self.windows.contains_key(&window) {
                    self.close(window, CloseReason::Program);
                    Ok(())
                } else {
                    Err(format!("no window is open with the number {window}"))
                };
                self.sink.reply(&reply, FromHelper::CloseDone { request, outcome });
            }
            ToHelper::FocusNode { request, reply, window, node } => self.focus_node(request, reply, window, node),
            ToHelper::Detach { window } => match self.windows.get_mut(&window) {
                Some(w) => {
                    if let Some(ring) = w.ring.take() {
                        self.sink.forget(&ring);
                    }
                }
                None => self.log_error("Detach", format!("no open window {window}")),
            },
            ToHelper::Focus { window } => match self.windows.get(&window) {
                Some(w) => w.window.set_focus(),
                None => self.log_error("Focus", format!("no open window {window}")),
            },
            ToHelper::ListAdapters { request, reply } => self.sink.reply(&reply, FromHelper::Adapters { request, adapters: Vec::new() }),
            ToHelper::ListWindows { request, reply } => {
                let windows = self.windows.values().filter(|w| w.opened).map(|w| WindowSummary { window: w.id, title: w.title.clone(), names: w.tree.names() }).collect();
                self.sink.reply(&reply, FromHelper::Windows { request, windows });
            }
            ToHelper::PageGraphics { request, reply } => self.sink.reply(&reply, FromHelper::PageGraphics { request, graphics: self.graphics }),
            ToHelper::TestEvent { request, reply, window, name, kind, value } => self.test_event(request, reply, window, &name, kind, &value),
            ToHelper::TestInput { request, reply, window, steps } => self.test_input(request, reply, window, steps),
            ToHelper::StopInput { window, request } => self.stop_input(window, request),
            ToHelper::Snapshot { request, reply, window, name, pixels } => self.snapshot(request, reply, window, name, pixels),
            ToHelper::ForgetReply { reply } => self.sink.forget(&reply),
            ToHelper::Bridge { request, reply, window, json } => {
                let outcome = self.deliver_to_page(window, &json, |value| format!("window.pwrs.__deliver({value});"));
                self.sink.reply(&reply, FromHelper::BridgeDone { request, outcome });
            }
            ToHelper::BridgeReply { window, call, json } => {
                // A window that is not open, or shows widgets, is refused
                // below by the number the command gave.
                let page_call = match self.windows.get_mut(&window) {
                    Some(w) if w.page => w.calls.answer(call),
                    _ => Some(call),
                };
                let Some(page_call) = page_call else {
                    trace::line(|| format!("web: window {window}'s reply to call {call} is for a page its engine took, so the page loaded since does not get it"));
                    return;
                };
                if let Err(text) = self.deliver_to_page(window, &json, |value| format!("window.pwrs.__reply({page_call}, {value});")) {
                    match self.windows.get(&window).and_then(|w| w.ring.clone()) {
                        Some(ring) => self.send_window(Some(&ring), FromHelper::Error { window: Some(window), context: String::from("BridgeReply"), text }),
                        None => self.log_error("BridgeReply", text),
                    }
                }
            }
            ToHelper::Shutdown => self.shut_down(),
            ToHelper::Dialog { request, reply, title, kind } => {
                let proxy = self.proxy.clone();
                pwrs_gui_dialogs::show(request, title, kind, !self.no_focus, None, Box::new(move |outcome| post(&proxy, UserEvent::DialogDone(request, reply, outcome))));
            }
            ToHelper::CloseDialog { dialog } => {
                if let Err(why) = pwrs_gui_dialogs::close(dialog) {
                    self.log_error("CloseDialog", why);
                }
            }
            ToHelper::ListDialogs { request, reply } => self.sink.reply(&reply, FromHelper::Dialogs { request, dialogs: pwrs_gui_dialogs::list() }),
            ToHelper::AnswerDialog { request, reply, dialog, answer } => {
                let outcome = pwrs_gui_dialogs::answer(dialog, answer).map(|()| Value::Null);
                self.sink.reply(&reply, FromHelper::TestEventDone { request, outcome });
            }
            ToHelper::PickerAnswer { window, node, outcome } => self.give_chooser(window, node, outcome),
            ToHelper::Gpu { request, reply, op } => {
                if let Some(outcome) = self.gpu_call(request, &reply, op).transpose() {
                    self.sink.reply(&reply, FromHelper::GpuResult { request, outcome });
                }
            }
            ToHelper::GpuFree { buffer } => self.log_error("GpuFree", format!("GPU buffer {buffer} was to be freed, and the HTML renderer has no GPU layer")),
            ToHelper::GpuFreePipeline { pipeline } => self.log_error("GpuFreePipeline", format!("render pipeline {pipeline} was to be freed, and the HTML renderer has no GPU layer")),
            ToHelper::GpuFreeTexture { texture } => self.log_error("GpuFreeTexture", format!("texture {texture} was to be freed, and the HTML renderer has no GPU layer")),
            ToHelper::GpuFreeSampler { sampler } => self.log_error("GpuFreeSampler", format!("sampler {sampler} was to be freed, and the HTML renderer has no GPU layer")),
            ToHelper::Media { request, reply, window, node, op } => self.media_call(request, reply, window, node, &op),
            // The loader reads the headers on the helper's own thread, before
            // the window that shows the clip opens.
            ToHelper::Probe { request, reply, source } => self.sink.reply(&reply, FromHelper::Probed { request, outcome: pwrs_gui_fetch::clip_format(&source) }),
        }
    }

    /// Makes call `op` on MediaPlayer `node` of window `window`: the page's
    /// player makes it and replies with its state. Refused here for a window
    /// that is not open or shows a page of the caller's own, and a node that
    /// is no MediaPlayer.
    fn media_call(&mut self, request: RequestId, reply: RingName, window: WindowId, node: NodeId, op: &proto::media::MediaOp) {
        let Some(w) = self.windows.get_mut(&window) else {
            self.sink.reply(&reply, FromHelper::MediaDone { request, outcome: Err(format!("no window is open with the number {window}")) });
            return;
        };
        let refusal = match w.tree.find(node) {
            _ if w.page => Some(format!("window {window} shows a page of its own, which has no widgets")),
            None => Some(format!("the window has no widget with the number {node}")),
            Some(n) if n.kind != Kind::MediaPlayer => Some(format!("node {node} is a {:?}, not a MediaPlayer", n.kind)),
            Some(_) => None,
        };
        if let Some(why) = refusal {
            self.sink.reply(&reply, FromHelper::MediaDone { request, outcome: Err(why) });
            return;
        }
        let ask = w.ask();
        w.media.insert(ask, (request, reply, node));
        w.script(format!("pwrsRuntime.media({ask}, {node}, {});", media_op_json(op)));
    }

    /// Replies to a GPU call: what a page's GpuSurface draws and a frame's
    /// texels, which the page replies with once it has them, so the call is
    /// left owed (`None`); a surface's camera, which the window stores and
    /// the page draws through; a canvas's view, a change checked as egui
    /// checks it against the view the page last drew and replied to by the
    /// page once it has made it, a read replied to from that view, a removal
    /// at once;
    /// and the copies of the egui renderer's table objects the page draws
    /// from. Every other GPU call is the egui renderer's.
    fn gpu_call(&mut self, request: RequestId, reply: &RingName, op: proto::gpu::GpuOp) -> Result<Option<proto::gpu::GpuValue>, proto::gpu::GpuFailure> {
        use proto::gpu::{GpuFailure, GpuOp, GpuValue, TableObject};
        let failure = |id: &str, text: String| GpuFailure { id: id.to_string(), text };
        match op {
            GpuOp::MirrorBuffer { buffer, length, offset, bytes } => {
                let version = gpu::keep_buffer(&self.copies, buffer, length, offset, &bytes).map_err(|text| failure("GuiGpuRange", text))?;
                self.copied(TableObject::Buffer(buffer), version);
                Ok(Some(GpuValue::Done))
            }
            GpuOp::MirrorTexture { texture, width, height, format, texels } => {
                let version = gpu::keep_texture(&self.copies, texture, width, height, format, texels);
                self.copied(TableObject::Texture(texture), version);
                Ok(Some(GpuValue::Done))
            }
            GpuOp::MirrorSampler { sampler, filter, address } => {
                let version = gpu::keep_sampler(&self.copies, sampler, filter, address);
                self.copied(TableObject::Sampler(sampler), version);
                Ok(Some(GpuValue::Done))
            }
            GpuOp::ForgetMirror { object } => {
                gpu::forget(&self.copies, object);
                Ok(Some(GpuValue::Done))
            }
            GpuOp::SetPageSurface { window, name, surface } => {
                let missing = gpu::missing(&surface, &self.copies);
                if !missing.is_empty() {
                    return Err(failure("GuiGpuNoCopy", format!("the HTML renderer has no copy of {}, which the surface draws from", missing.join(", "))));
                }
                let copies = Arc::clone(&self.copies);
                let (w, node) = html_surface(&mut self.windows, window, &name)?;
                let ask = w.ask();
                w.script(format!("pwrsRuntime.surface({node}, {}, {ask});", gpu::surface_json(&surface, &copies)));
                w.gpu_asks.insert(ask, (request, reply.clone(), PageGpu::Surface { node, surface }));
                Ok(None)
            }
            GpuOp::SaveSurface { window, name } => {
                let (w, node) = html_surface(&mut self.windows, window, &name)?;
                let ask = w.ask();
                w.script(format!("pwrsRuntime.save({node}, {ask});"));
                w.gpu_asks.insert(ask, (request, reply.clone(), PageGpu::Save));
                Ok(None)
            }
            GpuOp::SetCamera { window, name, change } => {
                let (w, node) = html_surface(&mut self.windows, window, &name)?;
                // A surface without a camera starts from the default one.
                let base = match w.cameras.get(&node) {
                    Some(spec) => spec.clone(),
                    None => proto::camera::CameraSpec::default(),
                };
                if let Some(text) = change.problem(&base) {
                    return Err(failure("GuiGpuCamera", text));
                }
                let made = change.applied_to(&base);
                w.script(format!("pwrsRuntime.camera({node}, {});", gpu::camera_json(&made)));
                w.cameras.insert(node, made.clone());
                Ok(Some(GpuValue::Camera(Some(made))))
            }
            GpuOp::GetCamera { window, name } => {
                let (w, node) = html_surface(&mut self.windows, window, &name)?;
                Ok(Some(GpuValue::Camera(w.cameras.get(&node).cloned())))
            }
            GpuOp::RemoveCamera { window, name } => {
                let (w, node) = html_surface(&mut self.windows, window, &name)?;
                w.cameras.remove(&node);
                w.script(format!("pwrsRuntime.camera({node}, null);"));
                Ok(Some(GpuValue::Done))
            }
            GpuOp::SetCanvasView { window, name, change } => {
                let (w, node) = html_canvas(&mut self.windows, window, &name)?;
                let canvas = w.canvases.get(&node).ok_or_else(|| failure("GuiCanvasView", format!("the window has no items for canvas {name}")))?;
                let base = match &canvas.view {
                    Some(shown) => shown.view.clone(),
                    None => proto::view::ViewSpec::default(),
                };
                if let Some(text) = change.problem(&base) {
                    return Err(failure("GuiCanvasView", text));
                }
                if let Some(proto::view::ViewFit::Items(names)) = &change.fit {
                    if let Some(why) = &canvas.unread {
                        return Err(failure("GuiCanvasView", format!("the view fits to items, and the canvas's items cannot be read: {why}")));
                    }
                    if let Some(missing) = names.iter().find(|n| canvas.board.find(n).is_none()) {
                        return Err(failure("GuiCanvasView", format!("the view fits to {missing}, and no item is named {missing}")));
                    }
                }
                let controls = change.applied_to(&base).controls;
                let ask = w.ask();
                w.script(format!("pwrsRuntime.setView({node}, {}, {ask});", change_json(&change, &controls)));
                w.gpu_asks.insert(ask, (request, reply.clone(), PageGpu::View { node, controls }));
                Ok(None)
            }
            GpuOp::GetCanvasView { window, name } => {
                let (w, node) = html_canvas(&mut self.windows, window, &name)?;
                Ok(Some(GpuValue::CanvasView(w.canvases.get(&node).and_then(|c| c.view.clone()))))
            }
            GpuOp::RemoveCanvasView { window, name } => {
                let (w, node) = html_canvas(&mut self.windows, window, &name)?;
                if let Some(c) = w.canvases.get_mut(&node) {
                    c.view = None;
                }
                w.script(format!("pwrsRuntime.view({node}, null);"));
                Ok(Some(GpuValue::Done))
            }
            _ => Err(failure(
                "GuiGpuNoLayer",
                String::from("the HTML renderer has no GPU layer: GPU buffers, textures, compute and render pipelines run in the egui renderer, and a page's GpuSurface draws from copies of them"),
            )),
        }
    }

    /// Tells every page drawing a GpuSurface that the copy of `object` is
    /// now at `version`, so one that draws from it takes it again.
    fn copied(&mut self, object: proto::gpu::TableObject, version: u64) {
        let script = format!("pwrsRuntime.copied({}, {version});", json!(gpu::name_of(object)));
        for w in self.windows.values_mut().filter(|w| !w.surfaces.is_empty()) {
            w.script(script.clone());
        }
    }

    /// Runs `script`, given the JSON value, in the page of window `window`;
    /// refuses a window that is not open or shows widgets, and text that is
    /// not JSON.
    fn deliver_to_page(&mut self, window: WindowId, json: &str, script: impl FnOnce(&Json) -> String) -> Result<(), String> {
        let Some(w) = self.windows.get_mut(&window) else {
            return Err(format!("no open window {window}"));
        };
        if !w.page {
            return Err(format!("window {window} shows widgets, not a page of its own, so a value for a page has nowhere to go"));
        }
        let value: Json = serde_json::from_str(json).map_err(|e| format!("a value for window {window}'s page is not JSON ({e}): {json}"))?;
        w.script(script(&value));
        Ok(())
    }

    /// Where window `window` opens: the work area of the monitor
    /// `PWRS_GUI_MONITOR` names, or else of the primary one, and the monitor
    /// named when it is connected. Where the monitors cannot be listed, or
    /// none is the primary one, the window fits within
    /// [`proto::ASSUMED_WORK_AREA`], which the log says.
    fn placing(&self, window: WindowId, target: &EventLoopWindowTarget<UserEvent>) -> Placing {
        let [across, down] = proto::ASSUMED_WORK_AREA;
        let listed = match screens(target) {
            Ok(listed) => listed,
            Err(why) => {
                post(&self.proxy, UserEvent::Note(format!("window {window} fits within {across} by {down} points and opens where the system places it: the monitors cannot be listed ({why})")));
                return Placing { work: proto::ASSUMED_WORK_AREA, on: None };
            }
        };
        let on = self.monitor.and_then(|n| match place::pick(&listed, n) {
            Some(m) => Some(m.clone()),
            None => {
                post(&self.proxy, UserEvent::Note(format!("window {window} opens where the system places it: {} names monitor {n}, which is not connected now", place::VAR)));
                None
            }
        });
        let work = match place::opening_on(&listed, self.monitor) {
            Some(m) => m.work_points(),
            None => {
                post(&self.proxy, UserEvent::Note(format!("window {window} fits within {across} by {down} points: no monitor listed is the primary one")));
                proto::ASSUMED_WORK_AREA
            }
        };
        Placing { work, on }
    }

    /// Builds window `window` as `shape` says, in its theme, with its
    /// webview: hidden when it is not shown at once, and brought to the
    /// front, where the settings request it, only when it is.
    fn open(
        &self,
        window: WindowId,
        options: &proto::WindowOptions,
        tree: &Node,
        shape: &Shape,
        target: &EventLoopWindowTarget<UserEvent>,
        context: &mut WebContext,
    ) -> Result<(Window, WebView, Option<Page>), String> {
        let page = if tree.kind == Kind::Html { Some(Page::of(tree)?) } else { None };
        let (w, h) = (f64::from(shape.size[0]), f64::from(shape.size[1]));
        let builder = WindowBuilder::new()
            .with_title(&options.title)
            .with_inner_size(tao::dpi::LogicalSize::new(w, h))
            .with_resizable(options.resizable)
            .with_always_on_top(options.always_on_top)
            .with_theme(frame_theme(options.theme))
            .with_visible(shape.visible)
            .with_focused(!self.no_focus);
        let builder = match shape.on {
            Some(m) => {
                let (x, y) = place::centered(m, w, h);
                trace::line(|| format!("web: window {window} opens at ({x}, {y}) on its monitor"));
                builder.with_position(tao::dpi::PhysicalPosition::new(x, y))
            }
            None => builder,
        };
        trace::line(|| format!("web: window {window} building, {}", if shape.visible { "shown" } else { "hidden until its page has measured it" }));
        let built = own_class(builder).build(target).map_err(|e| format!("the window cannot be created: {e}"))?;
        #[cfg(windows)]
        if shape.visible && self.bring_to_front && !self.no_focus {
            raise_window(window, &built);
        }
        trace::line(|| format!("web: window {window} built; building its webview"));
        let webview = self.build_view(window, &built, page.clone(), context)?;
        Ok((built, webview, page))
    }

    /// Builds the webview drawing window `window` in `built`: the page of
    /// the caller's own in `page`, or the widget runtime, with every
    /// engine event that ends its page posted to the loop.
    fn build_view(&self, window: WindowId, built: &Window, page: Option<Page>, context: &mut WebContext) -> Result<WebView, String> {
        let ipc = self.proxy.clone();
        let folder = context.data_directory().map(std::path::Path::to_path_buf);
        let id = window.to_string();
        let url = match &page {
            None => String::from("pwrs://localhost/index.html"),
            Some(p) => format!("pwrs://localhost{}", p.start),
        };
        let served = match &page {
            None => Served::Runtime,
            Some(p) => Served::Page { source: p.source.clone(), allow_navigation: p.allow_navigation },
        };
        self.served.rcu(|table| {
            let mut table = HashMap::clone(table);
            table.insert(id.clone(), served.clone());
            table
        });
        // WebKitGTK registers a custom scheme on the web context every
        // webview here shares, so the scheme is registered once and replies
        // to each webview by its id; WebView2 and WebKit register one per
        // webview, and their contexts never report it registered.
        let registered = context.is_custom_protocol_registered("pwrs");
        let dev_tools = page.as_ref().is_some_and(|p| p.dev_tools);
        let builder = chrome::builder(WebViewBuilder::new_with_web_context(context).with_id(&id).with_initialization_script(BRIDGE).with_focused(!self.no_focus), dev_tools);
        let builder = if registered {
            builder
        } else {
            let (served, pictures, assets, copies, clips) = (Arc::clone(&self.served), Arc::clone(&self.pictures), Arc::clone(&self.assets), Arc::clone(&self.copies), Arc::clone(&self.clips));
            // Answered at once, but for an https clip's part, which a thread
            // of its own fetches through the loader and answers.
            builder.with_asynchronous_custom_protocol(String::from("pwrs"), move |id, request, responder| {
                let path = request.uri().path();
                if let Some(node) = path.strip_prefix("/media/")
                    && matches!(served.load().get(id), Some(Served::Runtime))
                {
                    trace::line(|| format!("web: window {id} requests {path} {:?}", request.headers().get(wry::http::header::RANGE)));
                    match media::serve(&clips, id, node, request.headers().get(wry::http::header::RANGE)) {
                        media::Answer::Now(response) => responder.respond(response),
                        media::Answer::Fetch(url, range) => {
                            let fetching = std::thread::Builder::new().name(String::from("pwrs-gui-media-part")).spawn(move || responder.respond(media::fetch_part(&url, range.as_deref())));
                            if let Err(e) = fetching {
                                eprintln!("pwrs-gui-web: window {id}'s request for {path} goes unanswered: no thread could be started to fetch it: {e}");
                            }
                        }
                    }
                    return;
                }
                responder.respond(answer(&served, &pictures, &assets, &copies, id, path));
            })
        };
        #[cfg(windows)]
        let builder = match &self.environment {
            Some(environment) => {
                use wry::WebViewBuilderExtWindows;
                builder.with_environment(environment.clone())
            }
            None => builder,
        };
        #[cfg(target_os = "macos")]
        let builder = {
            use wry::WebViewBuilderExtDarwin;
            let ended = self.proxy.clone();
            let builder = builder.with_on_web_content_process_terminate_handler(move || {
                post(&ended, UserEvent::EngineEnded { window, what: String::from("web content process"), rebuild: false });
            });
            match self.store {
                Some(id) => builder.with_data_store_identifier(id),
                None => builder,
            }
        };
        let drops = self.proxy.clone();
        let builder = match page {
            None => builder
                .with_ipc_handler(move |request| {
                    trace::line(|| format!("web: window {window} ipc arrives"));
                    post(&ipc, UserEvent::Page(window, request.body().clone()));
                })
                .with_drag_drop_handler(move |event| match event {
                    wry::DragDropEvent::Drop { paths, position } if !paths.is_empty() => {
                        post(&drops, UserEvent::Dropped(window, paths.iter().map(|p| p.display().to_string()).collect(), position));
                        true
                    }
                    wry::DragDropEvent::Enter { paths, .. } => !paths.is_empty(),
                    _ => false,
                })
                .with_navigation_handler(move |url| {
                    let allowed = own_origin(&url);
                    trace::line(|| format!("web: window {window} navigates to {url}: {}", if allowed { "allowed" } else { "refused" }));
                    allowed
                })
                .with_on_page_load_handler(move |event, url| trace::line(|| format!("web: window {window} page load {} at {url}", load_name(&event)))),
            Some(page) => {
                let Page { allow_navigation, dev_tools, .. } = page;
                let loads = self.proxy.clone();
                builder
                    .with_ipc_handler(move |request| {
                        let origin = request.uri().to_string();
                        trace::line(|| format!("web: window {window} ipc arrives from {origin}"));
                        if allow_navigation || own_origin(&origin) {
                            post(&ipc, UserEvent::Page(window, request.body().clone()));
                        } else {
                            post(&ipc, UserEvent::Foreign(window, origin));
                        }
                    })
                    .with_navigation_handler(move |url| {
                        let allowed = allow_navigation || own_origin(&url);
                        trace::line(|| format!("web: window {window} navigates to {url}: {}", if allowed { "allowed" } else { "refused" }));
                        allowed
                    })
                    .with_on_page_load_handler(move |event, url| {
                        trace::line(|| format!("web: window {window} page load {} at {url}", load_name(&event)));
                        if matches!(event, PageLoadEvent::Finished) {
                            post(&loads, UserEvent::Loaded(window));
                        }
                    })
                    .with_devtools(dev_tools)
            }
        };
        // WebKitGTK gives a page the languages its context has as the page
        // loads, so there the page is loaded once they are set.
        let unix = cfg!(any(target_os = "linux", target_os = "dragonfly", target_os = "freebsd", target_os = "openbsd", target_os = "netbsd"));
        let builder = if unix { builder } else { builder.with_url(&url) };
        let webview = match build_webview(builder, built, folder.as_deref()) {
            Ok(webview) => webview,
            Err(why) => {
                self.stop_serving(&id);
                return Err(why);
            }
        };
        if let Err(why) = chrome::after_build(&webview, dev_tools, &self.fonts) {
            post(&self.proxy, UserEvent::Note(format!("window {window}'s engine was not set as the helper sets it: {why}")));
        }
        if unix {
            prefer_languages(&webview);
            if let Err(e) = webview.load_url(&url) {
                self.stop_serving(&id);
                return Err(format!("the webview cannot load its page: {e}"));
            }
        }
        trace::line(|| format!("web: window {window} webview built"));
        watch_engine(&webview, window, &self.proxy);
        Ok(webview)
    }

    /// Gives the page of window `window` the files dropped at `at`, which it
    /// passes to the widget there taking drops; on Windows `at` is in pixels,
    /// which the window's scale turns into the page's points.
    fn dropped(&mut self, window: WindowId, paths: &[String], at: (i32, i32)) {
        let Some(w) = self.windows.get_mut(&window) else {
            return;
        };
        let scale = if cfg!(windows) { w.window.scale_factor() } else { 1.0 };
        let (x, y) = (f64::from(at.0) / scale, f64::from(at.1) / scale);
        w.script(format!("pwrsRuntime.dropped({}, {x}, {y});", json!(paths)));
    }

    /// Opens the dialog of file chooser `node` in window `window`, starting
    /// where the first of `chosen` is, unless one is open already.
    fn browse(&mut self, window: WindowId, node: NodeId, chosen: &[String]) {
        let Some(w) = self.windows.get_mut(&window) else {
            return;
        };
        let ring = w.ring.clone();
        let opened = match w.tree.find(node) {
            Some(n) if n.kind == Kind::FilePicker => {
                if w.dialogs.contains_key(&node) {
                    return;
                }
                proto::chooser::dialog(n, chosen).map(|(title, kind)| {
                    w.dialogs.insert(node, files::open(&self.proxy, window, node, title, kind, !self.no_focus));
                })
            }
            Some(n) => Err(format!("the page requested a file dialog from a {:?}, node {node}", n.kind)),
            None => Err(format!("the page requested a file dialog from node {node}, which the window lacks")),
        };
        if let Err(text) = opened {
            self.send_window(ring.as_deref(), FromHelper::Error { window: Some(window), context: String::from("page"), text });
        }
    }

    /// Takes the answer of file chooser `node`'s dialog in window `window`.
    /// Where the system has no dialog of the kind and the window has a
    /// command, the command is given what the dialog requests, so the egui
    /// renderer draws it, and its answer comes back as
    /// [`ToHelper::PickerAnswer`]; any other answer goes to the page.
    fn chosen(&mut self, window: WindowId, node: NodeId, outcome: Result<proto::DialogAnswer, proto::DialogFailure>) {
        let Some(w) = self.windows.get_mut(&window) else {
            return;
        };
        if let Err(proto::DialogFailure::NoNative(why)) = &outcome
            && let Some(ring) = w.ring.clone()
            && let Some(picking) = w.dialogs.get_mut(&node)
        {
            picking.dialog = None;
            let wanted = FromHelper::PickerWanted { window, node, title: picking.title.clone(), kind: picking.kind.clone() };
            let text = format!("file chooser {node} of window {window} has its window's command draw its dialog: {why}");
            self.sink.log(FromHelper::Log { level: proto::LogLevel::Verbose, text });
            self.send_window(Some(&ring), wanted);
            return;
        }
        self.give_chooser(window, node, outcome);
    }

    /// Gives the page of window `window` the answer of file chooser `node`'s
    /// dialog, which ends its wait: the paths chosen, or none when it was
    /// canceled or could not show; a dialog that could not show is reported
    /// to the window's command. A window closed since closed its dialog with
    /// it.
    fn give_chooser(&mut self, window: WindowId, node: NodeId, outcome: Result<proto::DialogAnswer, proto::DialogFailure>) {
        let Some(w) = self.windows.get_mut(&window) else {
            return;
        };
        w.dialogs.remove(&node);
        let ring = w.ring.clone();
        let (paths, failed) = match outcome {
            Ok(proto::DialogAnswer::Paths(paths)) => (Some(paths), None),
            Ok(proto::DialogAnswer::Canceled) => (None, None),
            Ok(proto::DialogAnswer::Button(button)) => (None, Some(format!("the file dialog of node {node} returned the button {button:?}"))),
            Err(proto::DialogFailure::NoNative(why) | proto::DialogFailure::Failed(why)) => (None, Some(format!("the file dialog of node {node} could not show: {why}"))),
        };
        w.script(format!("pwrsRuntime.dialogDone({node}, {});", json!(paths)));
        if let Some(text) = failed {
            self.send_window(ring.as_deref(), FromHelper::Error { window: Some(window), context: String::from("dialog"), text });
        }
    }

    /// Gives widget `node` of window `window` the keyboard focus through its
    /// page, whose reply says whether the widget took it. Refused here for a
    /// window that is not open or shows a page of the caller's own, a node
    /// the window does not have, and one whose properties say it takes no
    /// focus ([`Node::focus_refusal`]); the page refuses a widget it has not
    /// drawn and a Canvas taking no keys.
    fn focus_node(&mut self, request: RequestId, reply: RingName, window: WindowId, node: NodeId) {
        let Some(w) = self.windows.get_mut(&window) else {
            self.sink.reply(&reply, FromHelper::FocusDone { request, outcome: Err(format!("no window is open with the number {window}")) });
            return;
        };
        let refusal = match w.tree.find(node) {
            _ if w.page => Some(format!("window {window} shows a page of its own, which has no widgets")),
            None => Some(format!("the window has no widget with the number {node}")),
            Some(found) => found.focus_refusal(),
        };
        if let Some(why) = refusal {
            self.sink.reply(&reply, FromHelper::FocusDone { request, outcome: Err(why) });
            return;
        }
        let ask = w.ask();
        w.focuses.insert(ask, (request, reply));
        w.script(format!("pwrsRuntime.focus({ask}, {node});"));
    }

    /// Acts on a widget of window `window` as a person would: a drop, a key
    /// and a click on a chart's point the renderer raises itself
    /// ([`helper_test`]), and anything else the page acts on.
    fn test_event(&mut self, request: RequestId, reply: RingName, window: WindowId, name: &str, kind: EventKind, value: &Value) {
        let Some(w) = self.windows.get_mut(&window) else {
            self.sink.reply(&reply, FromHelper::TestEventDone { request, outcome: Err(format!("no open window {window}")) });
            return;
        };
        let names_widget = kind != EventKind::Shortcut && !(kind == EventKind::Drop && name.is_empty());
        if names_widget && w.tree.find_named(name).is_none() {
            let why = format!("window {window} has no widget named {name}; its widgets are {}", w.tree.names().join(", "));
            self.sink.reply(&reply, FromHelper::TestEventDone { request, outcome: Err(why) });
            return;
        }
        match helper_test(&w.tree, &w.canvases, name, kind, value) {
            Ok(HelperTest::Page(v)) => {
                let ask = w.ask();
                let call = json!({ "request": ask, "name": name, "kind": format!("{kind:?}"), "value": value_json(&v) });
                w.tests.insert(ask, (request, reply));
                w.script(format!("pwrsRuntime.test({call});"));
            }
            Ok(HelperTest::Raise(node, v)) => {
                let ring = w.ring.clone();
                self.sink.reply(&reply, FromHelper::TestEventDone { request, outcome: Ok(v.clone()) });
                self.send_window(ring.as_deref(), FromHelper::Event { window, node, kind, value: v, at: proto::clock::now_ns() });
            }
            Ok(HelperTest::Browse(node)) => {
                let chosen: Vec<String> = match w.tree.find(node).and_then(|n| n.props.get("paths")) {
                    Some(Value::List(paths)) => paths.iter().filter_map(Value::as_str).map(String::from).collect(),
                    _no_paths_yet => Vec::new(),
                };
                self.sink.reply(&reply, FromHelper::TestEventDone { request, outcome: Ok(Value::Null) });
                self.browse(window, node, &chosen);
            }
            Err(why) => self.sink.reply(&reply, FromHelper::TestEventDone { request, outcome: Err(why) }),
        }
    }

    /// Gives window `window`'s page a test's input, `steps`, through its
    /// engine's own input ([`input`]), a step at a time. The page is told
    /// what to wait for, and the engine is given the step once the page is
    /// waiting. The next step begins once the engine has taken it and the
    /// page has seen it, and the reply comes once the page has seen the last.
    /// Refused for a window not open, a page of the caller's own, which has
    /// no runtime to see its steps, a window taking another test's input
    /// or not yet drawn, and steps putting the pointer outside its inside.
    fn test_input(&mut self, request: RequestId, reply: RingName, window: WindowId, steps: Vec<proto::test_input::Step>) {
        let Some(w) = self.windows.get_mut(&window) else {
            self.sink.reply(&reply, FromHelper::TestEventDone { request, outcome: Err(format!("no open window {window}")) });
            return;
        };
        let refused = if w.page {
            Some(format!("window {window} shows a page of its own, and a test's input goes to windows of widgets"))
        } else if w.input.is_some() {
            Some(format!("window {window} is still taking another test's input"))
        } else if !w.opened {
            Some(format!("window {window} has not been drawn yet, so there is no place to put the pointer on"))
        } else {
            let inside = w.window.inner_size().to_logical::<f64>(w.window.scale_factor());
            proto::test_input::refusal(&steps, window, [inside.width, inside.height], w.hand.held())
        };
        if let Some(why) = refused {
            self.sink.reply(&reply, FromHelper::TestEventDone { request, outcome: Err(why) });
            return;
        }
        let total = match u32::try_from(steps.len()) {
            Ok(total) => total,
            Err(e) => {
                self.sink.reply(&reply, FromHelper::TestEventDone { request, outcome: Err(format!("{} steps are more than an input counts: {e}", steps.len())) });
                return;
            }
        };
        w.stopped = None;
        w.input = Some(Inputs { request, reply, steps: steps.into(), current: None, total, begun: 0 });
        self.next_input(window);
    }

    /// Begins the next step of the test's input window `window` is taking,
    /// or replies to the test once none is left.
    fn next_input(&mut self, window: WindowId) {
        let Some(w) = self.windows.get_mut(&window) else {
            return;
        };
        let next = match w.input.as_mut() {
            Some(given) => {
                given.current = None;
                given.steps.pop_front()
            }
            None => return,
        };
        let Some(step) = next else {
            if let Some(done) = w.input.take() {
                self.sink.reply(&done.reply, FromHelper::TestEventDone { request: done.request, outcome: Ok(Value::Null) });
            }
            return;
        };
        let ask = w.ask();
        let wait = matches!(step, proto::test_input::Step::Wait { .. });
        let held = input::held_for_page(&step, w.hand.held());
        w.script(format!("pwrsRuntime.input({ask}, {});", step_json(&step, held)));
        if let Some(given) = w.input.as_mut() {
            given.begun += 1;
            let (number, total) = (given.begun, given.total);
            trace::line(|| format!("web: window {window} input step {number} of {total} begins: {}", step.description()));
            self.sink.reply(&given.reply, FromHelper::InputStep { request: given.request, step: number });
            given.current = Some(Current { ask, number, step, sent: wait, seen: false });
        }
    }

    /// Drops the rest of the test's input of request `request` that window
    /// `window` is taking, which its session no longer waits for, and tells
    /// the page to stop waiting for the step under way; a report of that
    /// step the page sends later is dropped.
    fn stop_input(&mut self, window: WindowId, request: RequestId) {
        let Some(w) = self.windows.get_mut(&window) else {
            return;
        };
        let Some(given) = w.input.take_if(|given| given.request == request) else {
            return;
        };
        if let Some(current) = given.current {
            w.script(format!("pwrsRuntime.input({}, null);", current.ask));
            w.stopped = Some(current.ask);
        }
        trace::line(|| format!("web: window {window} drops the rest of input {request}, which its session no longer waits for"));
    }

    /// The page of window `window` says, of the step of a test's input it
    /// numbered `ask`, that it is waiting for it, which gives the engine the
    /// step, or that it has seen it, which ends the step once the engine has
    /// taken it too.
    fn input_seen(&mut self, window: WindowId, ask: Option<u32>, armed: bool, body: &str) {
        let Some(w) = self.windows.get_mut(&window) else {
            return;
        };
        let ring = w.ring.clone();
        let Some(current) = w.input.as_mut().and_then(|i| i.current.as_mut()).filter(|c| Some(c.ask) == ask) else {
            if ask.is_some() && w.stopped == ask {
                trace::line(|| format!("web: window {window}'s page reported a step of an input its session stopped waiting for: {body}"));
                return;
            }
            let text = format!("the page reported a step of a test's input no request is giving it: {body}");
            self.send_window(ring.as_deref(), FromHelper::Error { window: Some(window), context: String::from("page"), text });
            return;
        };
        if armed {
            trace::line(|| format!("web: window {window}'s page waits for input step {}", current.number));
            let proxy = self.proxy.clone();
            let number = current.ask;
            let sent: input::Sent = Box::new(move |outcome| post(&proxy, UserEvent::InputSent(window, number, outcome)));
            input::send(&w.webview, &current.step, &mut w.hand, w.window.scale_factor(), sent);
            return;
        }
        trace::line(|| format!("web: window {window}'s page saw input step {}", current.number));
        current.seen = true;
        if current.sent {
            self.next_input(window);
        }
    }

    /// Window `window`'s engine has taken the step of its test's input the
    /// page numbered `ask`, which ends it once the page has seen it, or
    /// refused it, which refuses the test. A window closed since, or a test
    /// answered since, replied as it ended.
    fn input_sent(&mut self, window: WindowId, ask: u32, outcome: Result<(), String>) {
        let Some(w) = self.windows.get_mut(&window) else {
            return;
        };
        let Some(current) = w.input.as_mut().and_then(|i| i.current.as_mut()).filter(|c| c.ask == ask) else {
            return;
        };
        match outcome {
            Ok(()) => {
                trace::line(|| format!("web: window {window}'s engine took input step {}", current.number));
                current.sent = true;
                if current.seen {
                    self.next_input(window);
                }
            }
            Err(why) => {
                trace::line(|| format!("web: window {window}'s engine refused input step {}: {why}", current.number));
                w.script(format!("pwrsRuntime.input({ask}, null);"));
                if let Some(given) = w.input.take() {
                    self.sink.reply(&given.reply, FromHelper::TestEventDone { request: given.request, outcome: Err(why) });
                }
            }
        }
    }

    /// A message from a window's page. It is `ready` once the runtime
    /// requests its tree, `opened` after its first frame, an input's `state`,
    /// an `event`, or the reply to a `test`, a `focus` or a `snapshot`. It is
    /// `input` waiting for or having seen a step of a test's input, a `key`,
    /// `browse` for a file chooser's dialog, a GpuSurface's `camera` as a
    /// person moved it, a Canvas's `view` as the page drew it, or the reply to
    /// a `gpu` call. It is a request for a draw's `glsl` for WebGL 2, or the
    /// `graphics` the page draws GpuSurfaces with once it has settled them.
    /// It is an `error` about the page or the context it names, such as a
    /// GpuSurface's GPU, `fit` with the size it measured a window hidden until
    /// then at, or `role` with Enter or Escape pressed alone and the widget
    /// with the focus.
    fn page(&mut self, window: WindowId, body: &str) {
        let Some(w) = self.windows.get_mut(&window) else {
            return;
        };
        let msg: Json = match serde_json::from_str(body) {
            Ok(m) => m,
            Err(e) => {
                let ring = w.ring.clone();
                let text = format!("the page sent a message that is not JSON ({e}): {body}");
                self.send_window(ring.as_deref(), FromHelper::Error { window: Some(window), context: String::from("page"), text });
                return;
            }
        };
        let ring = w.ring.clone();
        trace::line(|| format!("web: window {window} page says {}", msg.get("t").and_then(Json::as_str).unwrap_or("nothing it names")));
        w.unresponsive = false;
        if w.page {
            self.own_page_message(window, ring, &msg, body);
            return;
        }
        match msg.get("t").and_then(Json::as_str) {
            Some("ready") => {
                w.ready = true;
                for script in std::mem::take(&mut w.pending) {
                    if let Err(e) = w.webview.evaluate_script(&script) {
                        eprintln!("pwrs-gui-web: window {window} did not take a script: {e}");
                    }
                }
            }
            Some("opened") => {
                w.recovering = false;
                for script in std::mem::take(&mut w.on_opened) {
                    w.script(script);
                }
                if !w.opened {
                    w.opened = true;
                    self.send_window(ring.as_deref(), FromHelper::Opened { window });
                }
            }
            Some("state") => match (msg.get("node").and_then(Json::as_u64).map(NodeId::try_from), msg.get("value")) {
                (Some(Ok(node)), Some(value)) => {
                    w.inputs.insert(node, value.clone());
                }
                _ => {
                    let text = format!("the page reported an input's state without a node number or a value: {body}");
                    self.send_window(ring.as_deref(), FromHelper::Error { window: Some(window), context: String::from("page"), text });
                }
            },
            Some("event") => match page_event(&msg) {
                Ok((node, kind, value)) => {
                    let value = media::named(&w.tree, node, kind, as_sent(&w.tree, node, value));
                    self.send_window(ring.as_deref(), FromHelper::Event { window, node, kind, value, at: proto::clock::now_ns() });
                }
                Err(text) => self.send_window(ring.as_deref(), FromHelper::Error { window: Some(window), context: String::from("page"), text }),
            },
            Some("test") => match page_ask(&msg).and_then(|a| w.tests.remove(&a)) {
                Some((request, reply)) => {
                    let outcome = match msg.get("error").and_then(Json::as_str) {
                        Some(e) => Err(e.to_string()),
                        None => test_answer(&w.tree, &msg),
                    };
                    self.sink.reply(&reply, FromHelper::TestEventDone { request, outcome });
                }
                None => {
                    let text = format!("the page replied to a test event no request is waiting for: {body}");
                    self.send_window(ring.as_deref(), FromHelper::Error { window: Some(window), context: String::from("page"), text });
                }
            },
            Some("input") => self.input_seen(window, page_ask(&msg), msg.get("armed").and_then(Json::as_bool) == Some(true), body),
            Some("focus") => match page_ask(&msg).and_then(|a| w.focuses.remove(&a)) {
                Some((request, reply)) => {
                    let outcome = match msg.get("error").and_then(Json::as_str) {
                        Some(e) => Err(e.to_string()),
                        None => Ok(()),
                    };
                    self.sink.reply(&reply, FromHelper::FocusDone { request, outcome });
                }
                None => {
                    let text = format!("the page sent a focus reply no request is waiting for: {body}");
                    self.send_window(ring.as_deref(), FromHelper::Error { window: Some(window), context: String::from("page"), text });
                }
            },
            Some("media") => match page_ask(&msg).and_then(|a| w.media.remove(&a)) {
                Some((request, reply, node)) => {
                    let outcome = match msg.get("error").and_then(Json::as_str) {
                        Some(e) => Err(e.to_string()),
                        None => page_media_state(&msg, |code, detail| media::page_message(&w.tree, node, code, detail)),
                    };
                    self.sink.reply(&reply, FromHelper::MediaDone { request, outcome });
                }
                None => {
                    let text = format!("the page replied to a media call no request is waiting for: {body}");
                    self.send_window(ring.as_deref(), FromHelper::Error { window: Some(window), context: String::from("page"), text });
                }
            },
            Some("snapshot") => match page_ask(&msg).and_then(|a| w.shots.remove(&a)) {
                None if page_ask(&msg).is_some_and(|a| w.late.remove(&a)) => {}
                Some((request, shot)) => {
                    let read = match msg.get("error").and_then(Json::as_str) {
                        Some(e) => Err(e.to_string()),
                        None => page_state(&w.tree, &msg).and_then(|state| {
                            let bounds = page_bounds(&w.tree, &msg)?;
                            let focused = page_focused(&w.tree, &msg)?;
                            let peeks = page_peeks(&w.tree, &msg)?;
                            let copied = page_copied(&msg)?;
                            let sound = page_sound(&w.tree, &msg)?;
                            Ok((state, bounds, focused, peeks, copied, sound))
                        }),
                    };
                    let (engine, unread) = engine_report(&w.webview, &self.version);
                    match read {
                        Ok((state, bounds, focused, peeks, copied, sound)) if shot.pixels => match page_view(&msg) {
                            Ok((rect, viewport)) => {
                                w.captures.insert(request, Capture { reply: shot.reply, state, bounds, engine, focused, peeks, copied, sound, rect, viewport, asked: shot.asked });
                                let proxy = self.proxy.clone();
                                capture::start(
                                    &w.webview,
                                    Box::new(move |image| match proxy.send_event(UserEvent::Pixels(window, request, image)) {
                                        Ok(()) => {}
                                        Err(_the_loop_has_ended) => {}
                                    }),
                                );
                            }
                            Err(why) => self.sink.reply(&shot.reply, FromHelper::Snapshot { request, outcome: Err(why) }),
                        },
                        Ok((state, bounds, focused, peeks, copied, sound)) => {
                            let outcome = Ok(proto::Snapshot { tree: w.current_tree(&peeks), state, pixels: None, bounds, engine, focused, copied, sound });
                            self.sink.reply(&shot.reply, FromHelper::Snapshot { request, outcome });
                        }
                        Err(why) => self.sink.reply(&shot.reply, FromHelper::Snapshot { request, outcome: Err(why) }),
                    }
                    if let Some(text) = unread {
                        self.send_window(ring.as_deref(), FromHelper::Error { window: Some(window), context: String::from("snapshot"), text });
                    }
                }
                None => {
                    let text = format!("the page replied with a snapshot no request is waiting for: {body}");
                    self.send_window(ring.as_deref(), FromHelper::Error { window: Some(window), context: String::from("page"), text });
                }
            },
            Some("glsl") => match page_ask(&msg) {
                Some(ask) => {
                    let answer = page_glsl(&msg, &format!("{} {}", engine(), self.version));
                    w.script(format!("pwrsRuntime.glsl({ask}, {answer});"));
                }
                None => {
                    let text = format!("the page requested a draw's GLSL without its number for the reply: {body}");
                    self.send_window(ring.as_deref(), FromHelper::Error { window: Some(window), context: String::from("page"), text });
                }
            },
            Some("graphics") => match msg.get("graphics").and_then(Json::as_str).and_then(WebGraphics::parse) {
                Some(graphics) => {
                    if self.graphics != Some(graphics) {
                        let text = format!("window {window}'s page draws GpuSurfaces with {}", graphics.name());
                        self.sink.log(FromHelper::Log { level: proto::LogLevel::Verbose, text });
                    }
                    self.graphics = Some(graphics);
                }
                None => {
                    let text = format!("the page reported what it draws GpuSurfaces with as neither WebGPU nor WebGL2: {body}");
                    self.send_window(ring.as_deref(), FromHelper::Error { window: Some(window), context: String::from("page"), text });
                }
            },
            Some("camera") => match page_pose(&msg) {
                Ok((node, target, distance, yaw, pitch)) => {
                    // A camera the session took away since the person moved
                    // it stays away.
                    if let Some(spec) = w.cameras.get_mut(&node) {
                        spec.target = target;
                        spec.distance = distance;
                        spec.yaw = yaw;
                        spec.pitch = pitch;
                    }
                }
                Err(text) => self.send_window(ring.as_deref(), FromHelper::Error { window: Some(window), context: String::from("page"), text }),
            },
            Some("gpu") => match page_ask(&msg).and_then(|a| w.gpu_asks.remove(&a)) {
                Some((request, reply, asked)) => match page_gpu_answer(&msg, &asked) {
                    Ok(value) => {
                        match (asked, &value) {
                            (PageGpu::Surface { node, surface }, _) => {
                                w.surfaces.insert(node, surface);
                            }
                            (PageGpu::View { node, .. }, proto::gpu::GpuValue::CanvasView(shown)) => match w.canvases.get_mut(&node) {
                                Some(c) => c.view = shown.clone(),
                                None => trace::line(|| format!("web: canvas {node} of window {window} is gone, so the view its page replied with is not stored")),
                            },
                            (PageGpu::View { .. } | PageGpu::Save, _) => {}
                        }
                        self.sink.reply(&reply, FromHelper::GpuResult { request, outcome: Ok(value) });
                    }
                    Err(failure) => self.sink.reply(&reply, FromHelper::GpuResult { request, outcome: Err(failure) }),
                },
                None => {
                    let text = format!("the page replied to a GPU call no request is waiting for: {body}");
                    self.send_window(ring.as_deref(), FromHelper::Error { window: Some(window), context: String::from("page"), text });
                }
            },
            Some("view") => match page_node(&msg) {
                Ok(node) => {
                    // A view the session took away since the person moved it
                    // stays away.
                    if let Some(kept) = w.canvases.get_mut(&node).and_then(|c| c.view.as_mut()) {
                        match page_view_shown(&msg, &kept.view.controls) {
                            Ok(Some(shown)) => *kept = shown,
                            Ok(None) => {
                                let text = format!("the page reported canvas {node} drawn without a view while the window has one: {body}");
                                self.send_window(ring.as_deref(), FromHelper::Error { window: Some(window), context: String::from("page"), text });
                            }
                            Err(text) => self.send_window(ring.as_deref(), FromHelper::Error { window: Some(window), context: String::from("page"), text }),
                        }
                    }
                }
                Err(text) => self.send_window(ring.as_deref(), FromHelper::Error { window: Some(window), context: String::from("page"), text }),
            },
            Some("key") => match page_key(&w.tree, &w.canvases, &msg) {
                Ok(Some((node, written))) => self.send_window(ring.as_deref(), FromHelper::Event { window, node, kind: EventKind::Key, value: Value::Str(written), at: proto::clock::now_ns() }),
                Ok(None) => {}
                Err(text) => self.send_window(ring.as_deref(), FromHelper::Error { window: Some(window), context: String::from("page"), text }),
            },
            Some("browse") => match (page_node(&msg), msg.get("paths").and_then(Json::as_array)) {
                (Ok(node), Some(paths)) => match paths.iter().map(|p| p.as_str().map(str::to_string)).collect::<Option<Vec<String>>>() {
                    Some(chosen) => self.browse(window, node, &chosen),
                    None => {
                        let text = format!("the page requested a file dialog with paths that are not all text: {body}");
                        self.send_window(ring.as_deref(), FromHelper::Error { window: Some(window), context: String::from("page"), text });
                    }
                },
                (Ok(_), None) => {
                    let text = format!("the page requested a file dialog without the paths its chooser has: {body}");
                    self.send_window(ring.as_deref(), FromHelper::Error { window: Some(window), context: String::from("page"), text });
                }
                (Err(text), _) => self.send_window(ring.as_deref(), FromHelper::Error { window: Some(window), context: String::from("page"), text }),
            },
            Some("error") => {
                let text = msg.get("text").and_then(Json::as_str).unwrap_or("the page reported an error with no text").to_string();
                let context = msg.get("context").and_then(Json::as_str).unwrap_or("page").to_string();
                self.send_window(ring.as_deref(), FromHelper::Error { window: Some(window), context, text });
            }
            Some("fit") => {
                let measured = page_size(&msg);
                if let Err(text) = &measured {
                    self.send_window(ring.as_deref(), FromHelper::Error { window: Some(window), context: String::from("page"), text: text.clone() });
                }
                self.show_fitted(window, measured.ok());
            }
            Some("role") => match msg.get("key").and_then(Json::as_str) {
                Some(key) => {
                    let holder = msg.get("node").and_then(Json::as_u64).and_then(|n| NodeId::try_from(n).ok());
                    match role_press(&w.tree, key, holder, w.close_on_escape) {
                        Ok(RolePress::Click(node)) => self.send_window(ring.as_deref(), FromHelper::Event { window, node, kind: EventKind::Click, value: Value::Null, at: proto::clock::now_ns() }),
                        Ok(RolePress::Close) => self.close(window, CloseReason::User),
                        Ok(RolePress::Nothing) => {}
                        Err(text) => self.send_window(ring.as_deref(), FromHelper::Error { window: Some(window), context: String::from("page"), text }),
                    }
                }
                None => {
                    let text = format!("the page sent a press for a window's button without its key: {body}");
                    self.send_window(ring.as_deref(), FromHelper::Error { window: Some(window), context: String::from("page"), text });
                }
            },
            other => {
                let text = format!("the page sent a message of an unrecognized kind ({other:?}): {body}");
                self.send_window(ring.as_deref(), FromHelper::Error { window: Some(window), context: String::from("page"), text });
            }
        }
    }

    /// Shows window `window`, hidden until its page measured its widgets at
    /// `measured`: sized to fit them within its bounds and the work area, or
    /// at the most it may take where they could not be measured, centered on
    /// the monitor `PWRS_GUI_MONITOR` names, and shown; brought to the front
    /// where the settings request it, and its webview given the keyboard,
    /// unless `PWRS_GUI_NO_FOCUS` says neither. Its page then reports it open
    /// once it has drawn at that size, and is sent the snapshots requested
    /// before then. A window
    /// shown already is left as it is.
    fn show_fitted(&mut self, window: WindowId, measured: Option<[f32; 2]>) {
        let Some(w) = self.windows.get_mut(&window) else {
            return;
        };
        let Some(fitting) = w.fitting.take() else {
            trace::line(|| format!("web: window {window}'s page measured it, and it is shown already"));
            return;
        };
        let [across, down] = fitting.size(measured);
        w.window.set_inner_size(tao::dpi::LogicalSize::new(f64::from(across), f64::from(down)));
        if let Some(m) = &fitting.on {
            let (x, y) = place::centered(m, f64::from(across), f64::from(down));
            w.window.set_outer_position(tao::dpi::PhysicalPosition::new(x, y));
        }
        trace::line(|| format!("web: window {window} measured at {measured:?}, shown at {across} by {down} points"));
        show_window(&w.window, self.no_focus);
        #[cfg(windows)]
        if self.bring_to_front && !self.no_focus {
            raise_window(window, &w.window);
        }
        let focused = if self.no_focus { Ok(()) } else { w.webview.focus() };
        w.script(String::from("pwrsRuntime.shown();"));
        if let Err(e) = focused {
            let text = format!("window {window}'s webview did not take the keyboard as the window was shown: {e}");
            self.sink.log(FromHelper::Log { level: proto::LogLevel::Verbose, text });
        }
    }

    /// A message from a page of the caller's own, which may send only these:
    /// `message` from `window.pwrs.send`, `pageclose` from
    /// `window.pwrs.close`, `save` and `restore` from `window.pwrs.save` and
    /// `window.pwrs.restore`, and `opened` after its first frame.
    fn own_page_message(&mut self, window: WindowId, ring: Option<RingName>, msg: &Json, body: &str) {
        match msg.get("t").and_then(Json::as_str) {
            Some("message") => {
                let value = msg.get("value").cloned().unwrap_or(Json::Null);
                let call = msg.get("call").and_then(Json::as_u64).and_then(|c| u32::try_from(c).ok());
                match (call, ring) {
                    (Some(call), Some(r)) => {
                        let Some(w) = self.windows.get_mut(&window) else {
                            return;
                        };
                        let asked = w.calls.ask(call);
                        self.sink.window(&r, FromHelper::BridgeMessage { window, call: asked, json: value.to_string() });
                    }
                    (Some(call), None) => {
                        if let Some(w) = self.windows.get_mut(&window) {
                            w.script(format!("window.pwrs.__reply({call}, null);"));
                        }
                    }
                    (None, r) => {
                        let text = format!("a page's message has no call number: {body}");
                        self.send_window(r.as_deref(), FromHelper::Error { window: Some(window), context: String::from("page"), text });
                    }
                }
            }
            Some("pageclose") => {
                let value = msg.get("value").cloned().unwrap_or(Json::Null);
                if let Some(w) = self.windows.remove(&window) {
                    self.retire(w, FromHelper::PageClosed { window, json: value.to_string() });
                }
            }
            Some("opened") => {
                if let Some(w) = self.windows.get_mut(&window) {
                    w.recovering = false;
                    if !w.opened {
                        w.opened = true;
                        self.send_window(ring.as_deref(), FromHelper::Opened { window });
                    }
                }
            }
            Some("save") => {
                if let Some(w) = self.windows.get_mut(&window) {
                    w.saved = Some(msg.get("value").cloned().unwrap_or(Json::Null).to_string());
                }
            }
            Some("restore") => {
                if let Some(w) = self.windows.get_mut(&window) {
                    let value = w.saved.clone().unwrap_or_else(|| String::from("null"));
                    w.script(format!("window.pwrs.__restored({value});"));
                }
            }
            other => {
                let text = format!("the page sent a message of no kind it may send ({other:?}): {body}");
                self.send_window(ring.as_deref(), FromHelper::Error { window: Some(window), context: String::from("page"), text });
            }
        }
    }

    /// A page of the caller's own has loaded: the scripts waiting on it run,
    /// and it says it has opened after its first frame.
    fn loaded(&mut self, window: WindowId) {
        let Some(w) = self.windows.get_mut(&window) else {
            return;
        };
        if !w.page || w.ready {
            return;
        }
        w.ready = true;
        let first_frame = String::from("requestAnimationFrame(function () { requestAnimationFrame(function () { window.pwrs.__post({ t: 'opened' }); }); });");
        for script in std::mem::take(&mut w.pending).into_iter().chain(std::iter::once(first_frame)) {
            if let Err(e) = w.webview.evaluate_script(&script) {
                eprintln!("pwrs-gui-web: window {window} did not take a script: {e}");
            }
        }
    }

    /// A message from outside a page's own origin was refused.
    fn foreign(&mut self, window: WindowId, origin: &str) {
        let ring = self.windows.get(&window).and_then(|w| w.ring.clone());
        let text = format!("a message from {origin}, which is not the page's own origin, was refused; -AllowNavigation lets other origins send");
        self.send_window(ring.as_deref(), FromHelper::Error { window: Some(window), context: String::from("page"), text });
    }

    /// Requests the window's state from the page and, with `pixels`, starts
    /// the capture once the page has replied, of the window or of the widget
    /// named `name`; a window whose page has not reported it open, as one
    /// hidden for its page to measure it, is sent the request once it has.
    fn snapshot(&mut self, request: RequestId, reply: RingName, window: WindowId, name: Option<String>, pixels: bool) {
        let Some(w) = self.windows.get_mut(&window) else {
            self.sink.reply(&reply, FromHelper::Snapshot { request, outcome: Err(format!("no open window {window}")) });
            return;
        };
        let node = match &name {
            None => Json::Null,
            Some(n) => match w.tree.find_named(n) {
                Some(found) => Json::from(found.id),
                None => {
                    let why = format!("window {window} has no widget named {n}; its widgets are {}", w.tree.names().join(", "));
                    self.sink.reply(&reply, FromHelper::Snapshot { request, outcome: Err(why) });
                    return;
                }
            },
        };
        if pixels && w.window.is_minimized() {
            let why = format!("window {window} is minimized, so its webview draws nothing to capture");
            self.sink.reply(&reply, FromHelper::Snapshot { request, outcome: Err(why) });
            return;
        }
        if w.page {
            let (engine, unread) = engine_report(&w.webview, &self.version);
            if pixels {
                w.captures.insert(request, Capture { reply, state: BTreeMap::new(), bounds: BTreeMap::new(), engine, focused: None, peeks: BTreeMap::new(), copied: None, sound: BTreeMap::new(), rect: None, viewport: (0.0, 0.0), asked: Instant::now() });
                let proxy = self.proxy.clone();
                capture::start(&w.webview, Box::new(move |image| post(&proxy, UserEvent::Pixels(window, request, image))));
            } else {
                let outcome = Ok(proto::Snapshot { tree: w.tree.clone(), state: BTreeMap::new(), pixels: None, bounds: BTreeMap::new(), engine, focused: None, copied: None, sound: BTreeMap::new() });
                self.sink.reply(&reply, FromHelper::Snapshot { request, outcome });
            }
            if let Some(text) = unread {
                let ring = w.ring.clone();
                self.send_window(ring.as_deref(), FromHelper::Error { window: Some(window), context: String::from("snapshot"), text });
            }
            return;
        }
        let ask = w.ask();
        w.shots.insert(ask, (request, Shot { reply, pixels, asked: Instant::now() }));
        let script = format!("pwrsRuntime.snapshot({ask}, {node});");
        if w.opened {
            w.script(script);
        } else {
            w.on_opened.push(script);
        }
    }

    /// A capture has finished: crops it to the widget requested and replies
    /// with the snapshot.
    fn pixels(&mut self, window: WindowId, request: RequestId, image: Result<Image, String>) {
        // A window that closed first replied to its captures as it closed.
        let Some(w) = self.windows.get_mut(&window) else {
            return;
        };
        let Some(shot) = w.captures.remove(&request) else {
            return;
        };
        let pixels = match shot.rect {
            None => image,
            Some(rect) => image.and_then(|whole| capture::crop(&whole, rect, shot.viewport)),
        };
        let outcome = pixels.map(|p| proto::Snapshot { tree: w.current_tree(&shot.peeks), state: shot.state, pixels: Some(p), bounds: shot.bounds, engine: shot.engine, focused: shot.focused, copied: shot.copied, sound: shot.sound });
        self.sink.reply(&shot.reply, FromHelper::Snapshot { request, outcome });
    }

    /// Refuses each pixel snapshot that has waited [`CAPTURE_DEADLINE`] for
    /// its page's state or its engine's image, and returns when the next one
    /// still waiting reaches it, for the loop to wake then; `None` when none
    /// waits.
    fn refuse_late_captures(&mut self) -> Option<Instant> {
        let now = Instant::now();
        let mut next: Option<Instant> = None;
        for w in self.windows.values_mut() {
            let window = w.id;
            let (late_asks, late_requests, due) = late(&w.shots, &w.captures, now);
            for ask in late_asks {
                if let Some((request, shot)) = w.shots.remove(&ask) {
                    w.late.insert(ask);
                    let why = format!("window {window}'s page gave no state for the capture in the {} s after it was requested", CAPTURE_DEADLINE.as_secs());
                    self.sink.reply(&shot.reply, FromHelper::Snapshot { request, outcome: Err(why) });
                }
            }
            for request in late_requests {
                if let Some(c) = w.captures.remove(&request) {
                    let why = format!("window {window}'s engine gave no image in the {} s after the capture was requested", CAPTURE_DEADLINE.as_secs());
                    self.sink.reply(&c.reply, FromHelper::Snapshot { request, outcome: Err(why) });
                }
            }
            next = match (next, due) {
                (Some(a), Some(b)) => Some(a.min(b)),
                (a, b) => a.or(b),
            };
        }
        next
    }

    fn close_requested(&mut self, id: tao::window::WindowId) {
        let found = self.windows.values().find(|w| w.window.id() == id).map(|w| w.id);
        if let Some(window) = found {
            self.close(window, CloseReason::User);
        }
    }

    /// Closes a window, saying why on its ring.
    fn close(&mut self, window: WindowId, reason: CloseReason) {
        if let Some(w) = self.windows.remove(&window) {
            self.retire(w, FromHelper::Closed { window, reason });
        }
    }

    /// Ends a window that has closed: sends `last` on its ring, replies to
    /// what its page still owed, forgets the ring, stops replying to its
    /// webview's `pwrs://` requests, and stops reading the rings its widgets
    /// read.
    fn retire(&mut self, mut w: WebWindow, last: FromHelper) {
        self.sources.forget(w.id);
        self.stop_serving(&w.id.to_string());
        self.send_window(w.ring.as_deref(), last);
        self.answer_owed(&mut w, "closed");
        if let Some(ring) = &w.ring {
            self.sink.forget(ring);
        }
    }

    /// Stops replying to `pwrs://` requests from the webview whose id is `id`.
    fn stop_serving(&self, id: &str) {
        self.served.rcu(|table| {
            let mut table = HashMap::clone(table);
            table.remove(id);
            table
        });
        self.pictures.rcu(|table| {
            let mut table = HashMap::clone(table);
            table.remove(id);
            table
        });
        media::forget(&self.clips, id);
    }

    /// Replies to every request window `w`'s page still owed with an error
    /// saying the window `happened` first.
    fn answer_owed(&mut self, w: &mut WebWindow, happened: &str) {
        let window = w.id;
        for (request, reply) in std::mem::take(&mut w.tests).into_values() {
            self.sink.reply(&reply, FromHelper::TestEventDone { request, outcome: Err(format!("window {window} {happened} before the event was raised")) });
        }
        for (request, reply) in std::mem::take(&mut w.focuses).into_values() {
            self.sink.reply(&reply, FromHelper::FocusDone { request, outcome: Err(format!("window {window} {happened} before the widget took the focus")) });
        }
        for (request, reply, _node) in std::mem::take(&mut w.media).into_values() {
            self.sink.reply(&reply, FromHelper::MediaDone { request, outcome: Err(format!("window {window} {happened} before its player made the call")) });
        }
        for (request, shot) in std::mem::take(&mut w.shots).into_values() {
            self.sink.reply(&shot.reply, FromHelper::Snapshot { request, outcome: Err(format!("window {window} {happened} before its state was read")) });
        }
        w.on_opened.clear();
        for (request, shot) in std::mem::take(&mut w.captures) {
            self.sink.reply(&shot.reply, FromHelper::Snapshot { request, outcome: Err(format!("window {window} {happened} before its capture")) });
        }
        for (request, reply, _) in std::mem::take(&mut w.gpu_asks).into_values() {
            let failure = proto::gpu::GpuFailure { id: String::from("GuiGpuNoSurface"), text: format!("window {window} {happened} before its page replied to the GPU call") };
            self.sink.reply(&reply, FromHelper::GpuResult { request, outcome: Err(failure) });
        }
        if let Some(given) = w.input.take() {
            // A last step that closes the window, as a click of its OK button
            // does, has been given once the engine has taken it.
            let left = given.current.as_ref().is_some_and(|c| !c.sent || matches!(c.step, proto::test_input::Step::Wait { .. }));
            let outcome = if given.steps.is_empty() && !left { Ok(Value::Null) } else { Err(format!("window {window} {happened} before it was given every step of its input")) };
            self.sink.reply(&given.reply, FromHelper::TestEventDone { request: given.request, outcome });
        }
    }

    /// The engine drawing window `window`'s page ended, as `what` names it.
    /// The page is loaded again, in a webview built again when `rebuild`
    /// says the engine took the webview with it, and comes back as it was:
    /// a runtime page with its tree, rows and inputs' state, a page of the
    /// caller's own with what it saved. A page that ends again before it
    /// has drawn since closes its window, and its command is told what ended.
    fn engine_ended(&mut self, window: WindowId, what: &str, rebuild: bool, context: &mut WebContext) {
        let Some(mut w) = self.windows.remove(&window) else {
            return;
        };
        if w.recovering {
            let text = format!("the page's {what} ended again before the page, loaded again after the last time, could draw, so the window closes");
            self.send_window(w.ring.as_deref(), FromHelper::Error { window: Some(window), context: String::from("page"), text });
            self.retire(w, FromHelper::Closed { window, reason: CloseReason::Failed });
            return;
        }
        trace::line(|| format!("web: window {window} lost its {what}; loading its page again"));
        self.sink.log(FromHelper::Log { level: proto::LogLevel::Warning, text: format!("window {window}'s page lost its {what}, so the page is loaded again") });
        self.answer_owed(&mut w, "lost its page's engine");
        w.calls.forget();
        w.recovering = true;
        w.ready = false;
        w.unresponsive = false;
        if !w.page {
            w.pending = w.open_scripts(&self.copies, &format!("{} {}", engine(), self.version));
        }
        let again = if rebuild {
            self.build_view(window, &w.window, w.settings.clone(), context).map(|fresh| {
                w.webview = fresh;
                w.hand = input::Hand::default();
            })
        } else {
            w.webview.reload().map_err(|e| format!("the page cannot be loaded again: {e}"))
        };
        match again {
            Ok(()) => {
                self.windows.insert(window, w);
            }
            Err(why) => {
                let text = format!("the page's {what} ended, and {why}, so the window closes");
                self.send_window(w.ring.as_deref(), FromHelper::Error { window: Some(window), context: String::from("page"), text });
                self.retire(w, FromHelper::Closed { window, reason: CloseReason::Failed });
            }
        }
    }

    /// Window `window`'s page has stopped responding: its command is told
    /// once, until the page speaks again.
    fn unresponsive(&mut self, window: WindowId) {
        let Some(w) = self.windows.get_mut(&window) else {
            return;
        };
        if w.unresponsive {
            return;
        }
        w.unresponsive = true;
        let ring = w.ring.clone();
        let text = String::from("the page has stopped responding");
        self.send_window(ring.as_deref(), FromHelper::Error { window: Some(window), context: String::from("page"), text });
    }

    fn send_window(&mut self, ring: Option<&str>, msg: FromHelper) {
        if let Some(r) = ring {
            self.sink.window(r, msg);
        }
    }

    fn log_error(&mut self, context: &str, text: String) {
        self.sink.log(FromHelper::Error { window: None, context: context.to_string(), text });
    }

    fn shut_down(&mut self) {
        if self.closing {
            return;
        }
        let open: Vec<WindowId> = self.windows.keys().copied().collect();
        for window in open {
            self.close(window, CloseReason::Shutdown);
        }
        self.closing = true;
    }
}

#[cfg(any(target_os = "linux", target_os = "freebsd"))]
fn build_webview(builder: WebViewBuilder, window: &Window, _folder: Option<&std::path::Path>) -> Result<WebView, String> {
    use tao::platform::unix::WindowExtUnix;
    use wry::WebViewBuilderExtUnix;
    let vbox = window.default_vbox().ok_or("the window has no GTK box for a webview")?;
    builder.build_gtk(vbox).map_err(|e| format!("the webview cannot be created: {e}"))
}

#[cfg(not(any(target_os = "linux", target_os = "freebsd")))]
fn build_webview(builder: WebViewBuilder, window: &Window, folder: Option<&std::path::Path>) -> Result<WebView, String> {
    builder.build(window).map_err(|e| webview_error(&e, folder))
}

/// Why WebView2 made no webview. Two refusals come from a browser process
/// already running on the data folder `folder`, and each names the fix, a
/// folder of this process's own. On runtimes that refuse it (154.0.4258.53
/// among them), one started with other options than this process's, such as
/// another `WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS`, refuses every webview
/// requested with these, with `ERROR_INVALID_STATE`; runtimes from
/// 154.0.4258.62 join such a webview to that browser process instead. One
/// this process cannot join, as one in another logon session of the same
/// user is, refuses with `ERROR_BUSY`.
#[cfg(windows)]
fn webview_error(e: &wry::Error, folder: Option<&std::path::Path>) -> String {
    use windows::Win32::Foundation::{ERROR_BUSY, ERROR_INVALID_STATE};
    let wry::Error::WebView2Error(webview2_com::Error::WindowsError(w)) = e else {
        return format!("the webview cannot be created: {e}");
    };
    let folder = || folder.map_or_else(|| String::from("this renderer's data folder"), |f| f.display().to_string());
    let fix = "Set-GuiOption -WebView PerProcess, or a -WebViewDataFolder of this process's own, gives its pages a browser process of their own";
    if w.code() == ERROR_INVALID_STATE.to_hresult() {
        return format!(
            "WebView2 refused the webview: the browser process running on the web data folder {} was started with other options than this process requests, such as another WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS; {fix}",
            folder()
        );
    }
    if w.code() == ERROR_BUSY.to_hresult() {
        return format!(
            "WebView2 refused the webview: the web data folder {} is in use by a browser process this process cannot join, such as one in another sign-in of this user, as a second Remote Desktop session is; {fix}",
            folder()
        );
    }
    format!("the webview cannot be created: {e}")
}

#[cfg(not(any(windows, target_os = "linux", target_os = "freebsd")))]
fn webview_error(e: &wry::Error, _folder: Option<&std::path::Path>) -> String {
    format!("the webview cannot be created: {e}")
}

/// The runtime's files, by path under `pwrs://localhost/`: the page, its
/// script and styles, the tokens' stylesheet and the system UI font in
/// `assets`, uPlot, the fonts the build includes, and egui's own fonts;
/// anything else is not found.
fn serve(path: &str, assets: &look::Assets) -> Response<Cow<'static, [u8]>> {
    if path == "/tokens.css" {
        let mut response = Response::new(Cow::Owned(assets.css.as_bytes().to_vec()));
        response.headers_mut().insert(CONTENT_TYPE, HeaderValue::from_static("text/css; charset=utf-8"));
        return response;
    }
    let carried = look::BUNDLED_FONTS.iter().find(|(served, _, _)| *served == path).map(|(_, _, bytes)| *bytes);
    let own = look::OWN_FONTS.iter().find(|(served, _)| *served == path).map(|(_, bytes)| *bytes);
    if let Some(bytes) = carried.or(own).or(assets.ui_font.filter(|_| path == look::SYSTEM_UI_PATH)) {
        let mut response = Response::new(Cow::Borrowed(bytes));
        response.headers_mut().insert(CONTENT_TYPE, HeaderValue::from_static("font/ttf"));
        return response;
    }
    let (status, mime, body) = match path {
        "/" | "/index.html" => (StatusCode::OK, "text/html; charset=utf-8", INDEX),
        "/runtime.js" => (StatusCode::OK, "text/javascript; charset=utf-8", RUNTIME_JS),
        "/canvas.js" => (StatusCode::OK, "text/javascript; charset=utf-8", CANVAS_JS),
        "/layout.js" => (StatusCode::OK, "text/javascript; charset=utf-8", LAYOUT_JS),
        "/runtime.css" => (StatusCode::OK, "text/css; charset=utf-8", RUNTIME_CSS),
        "/uPlot.iife.min.js" => (StatusCode::OK, "text/javascript; charset=utf-8", UPLOT_JS),
        "/uPlot.min.css" => (StatusCode::OK, "text/css; charset=utf-8", UPLOT_CSS),
        _missing => (StatusCode::NOT_FOUND, "text/plain; charset=utf-8", "not found"),
    };
    let mut response = Response::new(Cow::Borrowed(body.as_bytes()));
    *response.status_mut() = status;
    response.headers_mut().insert(CONTENT_TYPE, HeaderValue::from_static(mime));
    response
}

/// A picture an Image shows, which a page loads from `picture/<name>`
/// beside the runtime's files: its bytes and their media type.
#[derive(Clone)]
struct Picture {
    bytes: Arc<[u8]>,
    mime: &'static str,
}

/// The pictures of each open window's Images, by its webview's id and then
/// by name.
type PictureTable = ArcSwap<HashMap<String, Arc<HashMap<String, Picture>>>>;

/// A picture's bytes, or not found when its window has no picture of that
/// name.
fn serve_picture(picture: Option<&Picture>) -> Response<Cow<'static, [u8]>> {
    let mut response = match picture {
        Some(p) => {
            let mut response = Response::new(Cow::Owned(p.bytes.to_vec()));
            response.headers_mut().insert(CONTENT_TYPE, HeaderValue::from_static(p.mime));
            return response;
        }
        None => Response::new(Cow::Borrowed(&b"the window shows no such picture"[..])),
    };
    *response.status_mut() = StatusCode::NOT_FOUND;
    response.headers_mut().insert(CONTENT_TYPE, HeaderValue::from_static("text/plain; charset=utf-8"));
    response
}

/// The pictures of a window's Images, Markdown documents and canvases, by
/// where their bytes are stored, each with the name a page loads it by,
/// which no picture before it had in this helper, so a page never takes a
/// picture it loaded before for a new one; and whether one was named since
/// the pictures were last taken.
#[derive(Default)]
struct Pictures {
    named: HashMap<usize, (String, Picture)>,
    fresh: bool,
}

/// The number the next picture's name takes.
static PICTURES: AtomicU64 = AtomicU64::new(0);

impl Pictures {
    fn any(&self) -> bool {
        !self.named.is_empty()
    }

    /// Names `bytes`, a picture in `format`, when it has no name yet, so a
    /// change shows it before the window's pictures are next taken; a format
    /// no page draws is left unnamed, as [`Pictures::take`] leaves it.
    fn name(&mut self, bytes: &Arc<[u8]>, format: &str) {
        let key = address(bytes);
        if self.named.contains_key(&key) {
            return;
        }
        let Some((extension, mime)) = media_of(format) else {
            return;
        };
        self.named.insert(key, (format!("picture-{}.{extension}", PICTURES.fetch_add(1, Ordering::Relaxed)), Picture { bytes: Arc::clone(bytes), mime }));
        self.fresh = true;
    }

    /// Takes the pictures the Images and Markdown documents of `tree` show,
    /// and the Image items of its canvases in `canvases`: a picture it had
    /// stays under its name, a new one is given one, and one nothing shows
    /// any more goes. Returns whether any came, went, or was named since the
    /// last take.
    fn take(&mut self, tree: &Node, canvases: &BTreeMap<NodeId, WebCanvas>) -> bool {
        let mut kept: HashMap<usize, (String, Picture)> = HashMap::new();
        let mut came = std::mem::take(&mut self.fresh);
        let mut keep = |bytes: &Arc<[u8]>, format: &str| {
            let Some((extension, mime)) = media_of(format) else {
                return;
            };
            let key = address(bytes);
            if kept.contains_key(&key) {
                return;
            }
            let entry = match self.named.remove(&key) {
                Some(entry) => entry,
                None => {
                    came = true;
                    (format!("picture-{}.{extension}", PICTURES.fetch_add(1, Ordering::Relaxed)), Picture { bytes: Arc::clone(bytes), mime })
                }
            };
            kept.insert(key, entry);
        };
        let mut stack = vec![tree];
        while let Some(n) = stack.pop() {
            stack.extend(n.children.iter());
            for (bytes, format) in pictures_in(n) {
                keep(bytes, format);
            }
        }
        for c in canvases.values() {
            for id in c.board.all() {
                if let Some(proto::canvas::Shape::Image(_, picture)) = c.board.entry(id).map(|e| &e.item.shape) {
                    keep(&picture.bytes, &picture.format);
                }
            }
        }
        let went = !self.named.is_empty();
        self.named = kept;
        came || went
    }

    /// Where a page loads the picture `bytes` from, beside the runtime's
    /// files; none for bytes no Image of the window shows.
    fn src(&self, bytes: &Arc<[u8]>) -> Option<String> {
        self.named.get(&address(bytes)).map(|(name, _)| format!("picture/{name}"))
    }

    /// The pictures by their names.
    fn by_name(&self) -> HashMap<String, Picture> {
        self.named.values().map(|(name, p)| (name.clone(), p.clone())).collect()
    }
}

/// Gives the scheme's handler `pictures` as the pictures of window
/// `window`'s webview.
fn serve_pictures(table: &PictureTable, window: WindowId, pictures: &Pictures) {
    let id = window.to_string();
    let named = Arc::new(pictures.by_name());
    table.rcu(|t| {
        let mut t = HashMap::clone(t);
        t.insert(id.clone(), Arc::clone(&named));
        t
    });
}

/// Where `bytes` are stored, which tells two pictures apart while a tree
/// has them.
fn address(bytes: &Arc<[u8]>) -> usize {
    Arc::as_ptr(bytes).cast::<u8>() as usize
}

/// The pictures a node shows, each its bytes and their format: an Image's,
/// and those the session read for a Markdown document's images; none for
/// any other node, and a picture that is not one is left out.
fn pictures_in(n: &Node) -> Vec<(&Arc<[u8]>, &str)> {
    match n.kind {
        Kind::Image => n.props.get("image").and_then(picture_parts).into_iter().collect(),
        Kind::Markdown => match n.props.get("document").and_then(Value::as_map).and_then(|d| d.get("pictures")).and_then(Value::as_map) {
            Some(read) => read.values().filter_map(picture_parts).collect(),
            None => Vec::new(),
        },
        _ => Vec::new(),
    }
}

/// A picture's bytes and their format, from the map the session sends; none
/// for a value that is not one.
fn picture_parts(picture: &Value) -> Option<(&Arc<[u8]>, &str)> {
    let m = picture.as_map()?;
    match (m.get("bytes"), m.get("format")) {
        (Some(Value::Bytes(b)), Some(Value::Str(f))) => Some((b, f.as_str())),
        _ => None,
    }
}

/// The file extension and media type of a picture's format.
fn media_of(format: &str) -> Option<(&'static str, &'static str)> {
    match format {
        "png" => Some(("png", "image/png")),
        "jpeg" => Some(("jpg", "image/jpeg")),
        "gif" => Some(("gif", "image/gif")),
        "webp" => Some(("webp", "image/webp")),
        "bmp" => Some(("bmp", "image/bmp")),
        "ico" => Some(("ico", "image/x-icon")),
        "svg" => Some(("svg", "image/svg+xml")),
        _ => None,
    }
}

/// Whether `op` may give a window a picture: it sets an Image's picture, a
/// Markdown widget's document or a canvas's items, adds or replaces a node
/// with any of them, sets a canvas item's picture or adds canvas items,
/// any of which may be an Image.
fn shows_pictures(op: &PatchOp) -> bool {
    match op {
        PatchOp::SetProps { props, .. } => props.contains_key("image") || props.contains_key("document") || props.contains_key("items"),
        PatchOp::Insert { node, .. } | PatchOp::Replace { node } => holds_pictures(node),
        PatchOp::SetItem { props, .. } => props.contains_key("image"),
        PatchOp::AddItems { .. } => true,
        PatchOp::Remove { .. } | PatchOp::Append { .. } | PatchOp::RemoveRows { .. } | PatchOp::RemoveItems { .. } => false,
    }
}

fn holds_pictures(n: &Node) -> bool {
    matches!(n.kind, Kind::Image | Kind::Markdown | Kind::Canvas) || n.children.iter().any(holds_pictures)
}

/// What a window gives its page at `pwrs://`.
#[derive(Clone)]
enum Served {
    /// The widget runtime's files.
    Runtime,
    /// A page of the caller's own, which may leave its origin only with
    /// `allow_navigation`.
    Page { source: PageSource, allow_navigation: bool },
}

/// The outcome of GPU call `asked` the page's reply `msg` gives: the error
/// it names, or what was requested, a surface taken or a frame's texels,
/// which come as base64.
fn page_gpu_answer(msg: &Json, asked: &PageGpu) -> Result<proto::gpu::GpuValue, proto::gpu::GpuFailure> {
    use proto::gpu::{GpuFailure, GpuValue};
    let failure = |id: &str, text: String| GpuFailure { id: id.to_string(), text };
    if let Some(error) = msg.get("error") {
        return match (error.get("id").and_then(Json::as_str), error.get("text").and_then(Json::as_str)) {
            (Some(id), Some(text)) => Err(failure(id, text.to_string())),
            _ => Err(failure("GuiGpuDraw", format!("the page refused the GPU call without an error's id and text: {error}"))),
        };
    }
    match asked {
        PageGpu::Surface { .. } => Ok(GpuValue::Done),
        PageGpu::View { controls, .. } => {
            let ok = msg.get("ok").ok_or_else(|| failure("GuiCanvasView", format!("the page replied to a view call without the view it made: {msg}")))?;
            page_view_shown(ok, controls).map(GpuValue::CanvasView).map_err(|text| failure("GuiCanvasView", text))
        }
        PageGpu::Save => {
            let ok = msg.get("ok");
            let size = |key: &str| ok.and_then(|o| o.get(key)).and_then(Json::as_u64).and_then(|n| u32::try_from(n).ok());
            let (Some(width), Some(height)) = (size("width"), size("height")) else {
                return Err(failure("GuiGpuSave", String::from("the page replied to a save without the surface's width and height")));
            };
            let text = ok.and_then(|o| o.get("rgba")).and_then(Json::as_str).ok_or_else(|| failure("GuiGpuSave", String::from("the page replied to a save without its texels")))?;
            let rgba = proto::base64_bytes(text).map_err(|c| failure("GuiGpuSave", format!("the page's texels contain {c:?}, which is not base64")))?;
            if rgba.len() as u64 != u64::from(width) * u64::from(height) * 4 {
                return Err(failure("GuiGpuSave", format!("the page replied with {} bytes for {width} by {height} texels", rgba.len())));
            }
            Ok(GpuValue::Texels { width, height, rgba })
        }
    }
}

/// Open window `window` of `windows` and the node of its GpuSurface named
/// `name`, or why there is none.
fn html_surface<'a>(windows: &'a mut BTreeMap<WindowId, WebWindow>, window: WindowId, name: &str) -> Result<(&'a mut WebWindow, NodeId), proto::gpu::GpuFailure> {
    let failure = |text: String| proto::gpu::GpuFailure { id: String::from("GuiGpuNoSurface"), text };
    let w = windows.get_mut(&window).ok_or_else(|| failure(format!("no open window {window} is drawn by the HTML renderer")))?;
    let node = named_node(&w.tree, name, Kind::GpuSurface).map_err(failure)?;
    Ok((w, node))
}

/// Open window `window` of `windows` and the node of its Canvas named
/// `name`, or why there is none, as the egui renderer says it.
fn html_canvas<'a>(windows: &'a mut BTreeMap<WindowId, WebWindow>, window: WindowId, name: &str) -> Result<(&'a mut WebWindow, NodeId), proto::gpu::GpuFailure> {
    let failure = |text: String| proto::gpu::GpuFailure { id: String::from("GuiNoCanvas"), text };
    let w = windows.get_mut(&window).ok_or_else(|| failure(format!("no open window {window} is drawn by the HTML renderer")))?;
    let node = match w.tree.find_named(name) {
        Some(n) if n.kind == Kind::Canvas => n.id,
        Some(n) => return Err(failure(format!("{name} in window {window} is a {:?}, not a Canvas", n.kind))),
        None => return Err(failure(format!("window {window} has no widget named {name}; its widgets are {}", w.tree.names().join(", ")))),
    };
    Ok((w, node))
}

/// The node of the widget of `kind` named `name` in `tree`, compared
/// without regard to case.
fn named_node(tree: &Node, name: &str, kind: Kind) -> Result<NodeId, String> {
    let mut stack = vec![tree];
    while let Some(n) = stack.pop() {
        if n.name.as_deref().is_some_and(|given| given.eq_ignore_ascii_case(name)) {
            if n.kind == kind {
                return Ok(n.id);
            }
            return Err(format!("{name} is a {:?}, not a {kind:?}", n.kind));
        }
        stack.extend(n.children.iter());
    }
    Err(format!("the window has no widget named {name}"))
}

/// The view a page reports a canvas drawn with, `v` having it as `view`
/// with whether it is `moving`, under the session's `controls`: its center,
/// null for the canvas's own middle, its zoom, and its nearest and
/// farthest, a farthest of "Infinity" for none; none for null, a canvas
/// drawn without a view.
fn page_view_shown(v: &Json, controls: &proto::view::ViewControls) -> Result<Option<proto::view::ViewShown>, String> {
    let view = match v.get("view") {
        None | Some(Json::Null) => return Ok(None),
        Some(view) => view,
    };
    let number = |key: &str| match view.get(key) {
        Some(Json::String(s)) if s == "Infinity" => Ok(f64::INFINITY),
        Some(n) => n.as_f64().ok_or_else(|| format!("the page reported a canvas view whose {key} is not a number: {v}")),
        None => Err(format!("the page reported a canvas view without its {key}: {v}")),
    };
    let center = match view.get("center") {
        None | Some(Json::Null) => None,
        Some(c) => match c.as_array().map(|a| a.iter().map(Json::as_f64).collect::<Option<Vec<f64>>>()) {
            Some(Some(p)) if p.len() == 2 => Some([p[0], p[1]]),
            _ => return Err(format!("the page reported a canvas view whose center is not two numbers: {v}")),
        },
    };
    let moving = v.get("moving").and_then(Json::as_bool).ok_or_else(|| format!("the page reported a canvas view without whether it is moving: {v}"))?;
    let spec = proto::view::ViewSpec { center, zoom: number("zoom")?, min_zoom: number("minZoom")?, max_zoom: number("maxZoom")?, controls: controls.clone() };
    Ok(Some(proto::view::ViewShown { view: spec, moving }))
}

/// A number as a page takes a view's nearest or farthest: "Infinity" for
/// one with no end, which JSON has no number for.
fn view_number(n: f64) -> Json {
    if n.is_infinite() { Json::from("Infinity") } else { json!(n) }
}

/// Each gesture `controls` take, written as a script writes it, with the
/// name of the move it makes.
fn bindings_json(controls: &proto::view::ViewControls) -> Json {
    Json::Object(controls.bindings().into_iter().map(|(gesture, made)| (gesture.written(), Json::from(made.name()))).collect())
}

/// A view as a page takes it: its center, null for the canvas's own
/// middle, its zoom, its nearest and farthest, and its controls' bindings.
fn view_json(v: &proto::view::ViewSpec) -> Json {
    json!({ "center": v.center, "zoom": v.zoom, "minZoom": view_number(v.min_zoom), "maxZoom": view_number(v.max_zoom), "bindings": bindings_json(&v.controls) })
}

/// A view change as a page makes it: the parts it sets, each absent where
/// it leaves the view's as it is, the bindings of `controls`, the controls
/// the view has once changed, what it fits to, "All" or the names, and how
/// it moves there, none for at once.
fn change_json(change: &proto::view::ViewChange, controls: &proto::view::ViewControls) -> Json {
    let mut out = Map::new();
    if let Some(c) = change.center {
        out.insert(String::from("center"), json!(c));
    }
    for (key, part) in [("zoom", change.zoom), ("minZoom", change.min_zoom), ("maxZoom", change.max_zoom)] {
        if let Some(n) = part {
            out.insert(String::from(key), view_number(n));
        }
    }
    out.insert(String::from("bindings"), bindings_json(controls));
    out.insert(
        String::from("fit"),
        match &change.fit {
            None => Json::Null,
            Some(proto::view::ViewFit::All) => Json::from("All"),
            Some(proto::view::ViewFit::Items(names)) => json!(names),
        },
    );
    out.insert(String::from("ease"), change.ease.map_or(Json::Null, |e| json!({ "seconds": e.seconds, "easing": e.easing.name() })));
    Json::Object(out)
}

/// The numbers a canvas's view moves by and how it draws a box zoom's box,
/// as the protocol gives them, for a page's views to move as egui's do.
fn view_numbers() -> Json {
    use proto::view as v;
    json!({
        "zoomStep": v::ZOOM_STEP,
        "zoomInStep": v::ZOOM_IN_STEP,
        "scrollStep": v::SCROLL_STEP,
        "keyStep": v::KEY_STEP,
        "stepSeconds": v::STEP_SECONDS,
        "stepEasing": v::STEP_EASING.name(),
        "glideWindow": v::GLIDE_WINDOW,
        "glideDeceleration": v::GLIDE_DECELERATION,
        "glideLinearity": v::GLIDE_LINEARITY,
        "glideEasing": v::GLIDE_EASING.name(),
        "boxBorder": v::BOX_BORDER.hex(),
        "boxBorderWidth": v::BOX_BORDER_WIDTH,
        "boxFill": v::BOX_FILL.hex(),
    })
}

/// The reply to `path` for the webview whose id is `id`, from what its
/// window gives its page, a runtime window's pictures, the copies its
/// GpuSurfaces draw from and the `assets` among it; a webview with no
/// window here gets not found.
fn answer(served: &ArcSwap<HashMap<String, Served>>, pictures: &PictureTable, assets: &look::Assets, copies: &gpu::Copies, id: &str, path: &str) -> Response<Cow<'static, [u8]>> {
    let table = served.load();
    match table.get(id) {
        Some(Served::Runtime) => {
            trace::line(|| format!("web: window {id} requests {path}"));
            if let Some(name) = path.strip_prefix("/picture/") {
                return serve_picture(pictures.load().get(id).and_then(|shown| shown.get(name)));
            }
            match path.strip_prefix("/gpu/") {
                Some(name) => gpu::serve(copies, name),
                None => serve(path, assets),
            }
        }
        Some(Served::Page { source, allow_navigation }) => serve_page(source, path, *allow_navigation),
        None => {
            let mut response = Response::new(Cow::Owned(format!("no window here shows webview {id}").into_bytes()));
            *response.status_mut() = StatusCode::NOT_FOUND;
            response.headers_mut().insert(CONTENT_TYPE, HeaderValue::from_static("text/plain; charset=utf-8"));
            response
        }
    }
}

/// The monitors a window can open on: Windows' own list, whose numbers are
/// the ones its display settings show, or the windowing system's.
#[cfg(windows)]
fn screens(_target: &EventLoopWindowTarget<UserEvent>) -> Result<Vec<place::Monitor>, String> {
    place::monitors()
}

#[cfg(not(windows))]
fn screens(target: &EventLoopWindowTarget<UserEvent>) -> Result<Vec<place::Monitor>, String> {
    Ok(target
        .available_monitors()
        .map(|m| {
            let (at, size) = (m.position(), m.size());
            place::Monitor { name: m.name(), x: at.x, y: at.y, width: size.width, height: size.height, work: [size.width, size.height], scale: m.scale_factor() }
        })
        .collect())
}

/// Gives the event loop an event; a loop that has ended has no one left to
/// take it.
fn post(proxy: &EventLoopProxy<UserEvent>, event: UserEvent) {
    match proxy.send_event(event) {
        Ok(()) => {}
        Err(_the_loop_has_ended) => {}
    }
}

/// A page-load event's name, for the trace.
fn load_name(event: &PageLoadEvent) -> &'static str {
    match event {
        PageLoadEvent::Started => "started",
        PageLoadEvent::Finished => "finished",
    }
}

/// Handles the failures of the processes behind a window's webview: a
/// browser process that ends takes the webview with it, so it is built
/// again; a renderer that ends is started again by a reload; a renderer
/// that stops responding is reported to the window's command; any other
/// process WebView2 starts again by itself, and the session's log notes it.
#[cfg(windows)]
fn watch_engine(webview: &WebView, window: WindowId, proxy: &EventLoopProxy<UserEvent>) {
    use webview2_com::Microsoft::Web::WebView2::Win32::{
        COREWEBVIEW2_PROCESS_FAILED_KIND, COREWEBVIEW2_PROCESS_FAILED_KIND_BROWSER_PROCESS_EXITED, COREWEBVIEW2_PROCESS_FAILED_KIND_RENDER_PROCESS_EXITED,
        COREWEBVIEW2_PROCESS_FAILED_KIND_RENDER_PROCESS_UNRESPONSIVE, COREWEBVIEW2_PROCESS_FAILED_REASON, ICoreWebView2ProcessFailedEventArgs2,
    };
    use webview2_com::ProcessFailedEventHandler;
    use windows::core::Interface;
    use wry::WebViewExtWindows;
    let core = webview.webview();
    if trace::enabled() {
        let mut pid = 0u32;
        match unsafe { core.BrowserProcessId(&mut pid) } {
            Ok(()) => trace::line(|| format!("web: window {window} runs in browser process {pid}")),
            Err(e) => trace::line(|| format!("web: window {window}'s browser process cannot be read: {e}")),
        }
    }
    let failed = proxy.clone();
    let handler = ProcessFailedEventHandler::create(Box::new(move |_, args| {
        let Some(args) = args else {
            return Ok(());
        };
        let mut kind = COREWEBVIEW2_PROCESS_FAILED_KIND::default();
        unsafe { args.ProcessFailedKind(&mut kind)? };
        let detail = match args.cast::<ICoreWebView2ProcessFailedEventArgs2>() {
            Ok(more) => {
                let (mut reason, mut code) = (COREWEBVIEW2_PROCESS_FAILED_REASON::default(), 0i32);
                match (unsafe { more.Reason(&mut reason) }, unsafe { more.ExitCode(&mut code) }) {
                    (Ok(()), Ok(())) => format!("reason {}, exit code {code}", reason.0),
                    (Err(e), _) | (_, Err(e)) => format!("its reason and exit code cannot be read: {}", e.message()),
                }
            }
            Err(e) => format!("this WebView2 gives no reason or exit code: {}", e.message()),
        };
        trace::line(|| format!("web: window {window} process failed: kind {}, {detail}", kind.0));
        let event = if kind == COREWEBVIEW2_PROCESS_FAILED_KIND_BROWSER_PROCESS_EXITED {
            UserEvent::EngineEnded { window, what: format!("WebView2 browser process ({detail})"), rebuild: true }
        } else if kind == COREWEBVIEW2_PROCESS_FAILED_KIND_RENDER_PROCESS_EXITED {
            UserEvent::EngineEnded { window, what: format!("WebView2 renderer process ({detail})"), rebuild: false }
        } else if kind == COREWEBVIEW2_PROCESS_FAILED_KIND_RENDER_PROCESS_UNRESPONSIVE {
            UserEvent::Unresponsive(window)
        } else {
            UserEvent::Note(format!("window {window}'s page lost a WebView2 process of kind {} ({detail}), which WebView2 starts again", kind.0))
        };
        post(&failed, event);
        Ok(())
    }));
    let mut token = 0i64;
    if let Err(e) = unsafe { core.add_ProcessFailed(&handler, &mut token) } {
        let text = format!("WebView2 did not take the handler for window {window}'s process failures, so a page whose engine ends will not be loaded again: {}", e.message());
        post(proxy, UserEvent::Note(text));
    }
}

/// Handles the end of the web process behind a window's webview: one that
/// ends is started again by a reload, and one that stops responding is
/// reported to the window's command.
#[cfg(any(target_os = "linux", target_os = "dragonfly", target_os = "freebsd", target_os = "openbsd", target_os = "netbsd"))]
fn watch_engine(webview: &WebView, window: WindowId, proxy: &EventLoopProxy<UserEvent>) {
    use webkit2gtk::WebViewExt;
    use wry::WebViewExtUnix;
    let view = webview.webview();
    let ended = proxy.clone();
    view.connect_web_process_terminated(move |_, reason| {
        post(&ended, UserEvent::EngineEnded { window, what: format!("WebKitGTK web process ({reason:?})"), rebuild: false });
    });
    let stalled = proxy.clone();
    view.connect_is_web_process_responsive_notify(move |v| {
        if !v.is_web_process_responsive() {
            post(&stalled, UserEvent::Unresponsive(window));
        }
    });
}

/// On macOS the handlers are set as the webview is built
/// ([`State::build_view`]).
#[cfg(target_os = "macos")]
fn watch_engine(_webview: &WebView, _window: WindowId, _proxy: &EventLoopProxy<UserEvent>) {}

/// Sets GTK's hint style to slight, which WebKitGTK's web process takes
/// from this process for every page, so a page places each glyph by its
/// own advance, unrounded, as egui does. Under a medium or full style, a
/// desktop's or GTK's own where no desktop sets one, as under Xvfb,
/// FreeType hints each glyph's width too, which narrows some by a pixel.
/// The note for `Hello` when GTK has no settings to set.
#[cfg(any(target_os = "linux", target_os = "dragonfly", target_os = "freebsd", target_os = "openbsd", target_os = "netbsd"))]
fn hint_slightly() -> Option<String> {
    use gtk::prelude::GtkSettingsExt;
    let Some(settings) = gtk::Settings::default() else {
        return Some(String::from("GTK has no settings for the pages' hint style, so their text may be laid out narrower than egui's"));
    };
    settings.set_gtk_xft_hinting(1);
    settings.set_gtk_xft_hintstyle(Some("hintslight"));
    trace::line(|| String::from("web: pages take GTK's slight hint style"));
    None
}

/// WebView2 and WebKit take no hint style from GTK.
#[cfg(not(any(target_os = "linux", target_os = "dragonfly", target_os = "freebsd", target_os = "openbsd", target_os = "netbsd")))]
fn hint_slightly() -> Option<String> {
    None
}

/// Gives the pages of the context every webview here shares the language
/// en-US when the process locale names none: under the C or POSIX locale
/// WebKitGTK reports navigator.language "C", which no Intl constructor
/// takes.
#[cfg(any(target_os = "linux", target_os = "dragonfly", target_os = "freebsd", target_os = "openbsd", target_os = "netbsd"))]
fn prefer_languages(webview: &WebView) {
    use webkit2gtk::{WebContextExt, WebViewExt};
    use wry::WebViewExtUnix;
    let names = webkit2gtk::glib::language_names();
    if !names.first().is_some_and(|n| names_no_language(n.as_str())) {
        return;
    }
    match webview.webview().context() {
        Some(context) => context.set_preferred_languages(&["en-US"]),
        None => trace::line(|| String::from("web: the webview has no context, so its pages read the C locale's language")),
    }
}

/// WebView2 and WebKit give a page the system's languages.
#[cfg(not(any(target_os = "linux", target_os = "dragonfly", target_os = "freebsd", target_os = "openbsd", target_os = "netbsd")))]
fn prefer_languages(_webview: &WebView) {}

/// Whether `name`, a locale as glib names them, is the C or POSIX locale,
/// with or without a codeset or modifier.
#[cfg(any(test, target_os = "linux", target_os = "dragonfly", target_os = "freebsd", target_os = "openbsd", target_os = "netbsd"))]
fn names_no_language(name: &str) -> bool {
    let base = name.find(['.', '@']).map_or(name, |end| &name[..end]);
    base.eq_ignore_ascii_case("C") || base.eq_ignore_ascii_case("POSIX")
}

/// Removes the macOS data stores of processes that have ended; stores exist
/// from macOS 14. WebKit returns a listing on the main run loop it sets up
/// as it starts, and taking a store starts it, so the store this renderer's
/// webviews use, `own` or the default, is taken first. WebKit replies on
/// the event loop; a store it could not remove, or a listing it refused,
/// reaches the session's log.
#[cfg(target_os = "macos")]
fn sweep_stores(proxy: EventLoopProxy<UserEvent>, own: Option<[u8; 16]>) {
    use objc2::MainThreadMarker;
    use objc2_foundation::NSUUID;
    use objc2_web_kit::WKWebsiteDataStore;
    use wry::WebViewExtDarwin;
    if !data::stores_exist() {
        return;
    }
    let Some(mtm) = MainThreadMarker::new() else {
        post(&proxy, UserEvent::Note(String::from("the web data stores of ended processes were not looked for: the event loop runs off the main thread")));
        return;
    };
    // SAFETY: `mtm` shows this runs on the main thread, which both calls take.
    let started = match own {
        Some(id) => unsafe { WKWebsiteDataStore::dataStoreForIdentifier(&NSUUID::from_bytes(id), mtm) },
        None => unsafe { WKWebsiteDataStore::defaultDataStore(mtm) },
    };
    drop(started);
    let listing = proxy.clone();
    trace::line(|| String::from("web: listing the data stores"));
    let asked = WebView::fetch_data_store_identifiers(move |ids| {
        trace::line(|| format!("web: data stores listed: {}", ids.len()));
        for id in ids {
            let Some(owner) = data::Owner::of_store(&id) else {
                continue;
            };
            if !owner.has_ended(data::started_at) {
                continue;
            }
            let proxy = listing.clone();
            WebView::remove_data_store(&id, move |removed| match removed {
                Ok(()) => trace::line(|| format!("web: removed the data store of ended process {owner:?}")),
                Err(e) => post(&proxy, UserEvent::Note(format!("the web data store of an ended process could not be removed: {e}; a later start tries again"))),
            });
        }
    });
    if let Err(e) = asked {
        post(&proxy, UserEvent::Note(format!("the web data stores of ended processes could not be listed: {e}")));
    }
}

/// Whether `url` is at a page's own origin: `pwrs://localhost`, which wry
/// maps on Windows to `http://pwrs.localhost` or `https://pwrs.localhost`,
/// with the address ending there or going on with a path, a query or a
/// fragment. Any other host, port or user part, as
/// `http://pwrs.localhost.example.com/`, is another origin.
fn own_origin(url: &str) -> bool {
    ["pwrs://localhost", "http://pwrs.localhost", "https://pwrs.localhost"]
        .iter()
        .any(|origin| url.strip_prefix(origin).is_some_and(|rest| rest.is_empty() || rest.starts_with(['/', '?', '#'])))
}

/// The content-security policy sent with a page of the caller's own unless
/// it may navigate: its own origin, inline script and style, and
/// data and blob URLs, so nothing remote loads.
const PAGE_POLICY: &str = "default-src 'self' data: blob: 'unsafe-inline' 'unsafe-eval'";

/// Where a page's files come from.
#[derive(Clone)]
enum PageSource {
    /// The document's text, given as its only file.
    Text(Arc<str>),
    /// A folder, every file under it given by its path.
    Folder(PathBuf),
}

/// A page window's settings, from its node's properties.
#[derive(Clone)]
struct Page {
    source: PageSource,
    /// The path under `pwrs://localhost` the page opens at, from `/`.
    start: String,
    allow_navigation: bool,
    dev_tools: bool,
}

impl Page {
    fn of(node: &Node) -> Result<Page, String> {
        let flag = |key: &str| match node.props.get(key) {
            None => Ok(false),
            Some(Value::Bool(b)) => Ok(*b),
            Some(other) => Err(format!("a page's {key} is true or false, not {other:?}")),
        };
        let text = |key: &str| match node.props.get(key) {
            None => Ok(None),
            Some(Value::Str(s)) => Ok(Some(s.clone())),
            Some(other) => Err(format!("a page's {key} is a string, not {other:?}")),
        };
        let (source, start) = match (text("html")?, text("folder")?) {
            (Some(html), None) => (PageSource::Text(Arc::from(with_doctype(html))), String::from("/index.html")),
            (None, Some(folder)) => {
                let first = text("page")?.unwrap_or_else(|| String::from("index.html"));
                (PageSource::Folder(PathBuf::from(folder)), format!("/{}", encode_path(&first)))
            }
            (Some(_), Some(_)) => return Err(String::from("a page has html or a folder, not both")),
            (None, None) => return Err(String::from("a page has html or a folder, and this one has neither")),
        };
        Ok(Page { source, start, allow_navigation: flag("allow_navigation")?, dev_tools: flag("dev_tools")? })
    }
}

/// A page's text behind `<!DOCTYPE html>` when it does not begin with a
/// doctype of its own, after any whitespace or byte-order mark, so the
/// webview lays it out by the current standard rather than in the quirks
/// mode it uses for old pages; text with a doctype is returned as given.
fn with_doctype(html: String) -> String {
    let start = html.trim_start_matches(|c: char| c.is_whitespace() || c == '\u{feff}');
    if start.get(..9).is_some_and(|head| head.eq_ignore_ascii_case("<!doctype")) {
        html
    } else {
        format!("<!DOCTYPE html>\n{html}")
    }
}

/// A page's file at `path` under `pwrs://localhost`, with its
/// content-security policy unless the page may navigate.
fn serve_page(source: &PageSource, path: &str, allow_navigation: bool) -> Response<Cow<'static, [u8]>> {
    let found = match source {
        PageSource::Text(html) => match path {
            "/" | "/index.html" => Ok((html.as_bytes().to_vec(), "text/html; charset=utf-8")),
            other => Err((StatusCode::NOT_FOUND, format!("{other} is not part of this page"))),
        },
        PageSource::Folder(root) => folder_file(root, path),
    };
    let mut response = match found {
        Ok((body, mime)) => {
            let mut r = Response::new(Cow::Owned(body));
            r.headers_mut().insert(CONTENT_TYPE, HeaderValue::from_static(mime));
            r
        }
        Err((status, why)) => {
            let mut r = Response::new(Cow::Owned(why.into_bytes()));
            *r.status_mut() = status;
            r.headers_mut().insert(CONTENT_TYPE, HeaderValue::from_static("text/plain; charset=utf-8"));
            r
        }
    };
    if !allow_navigation {
        response.headers_mut().insert("content-security-policy", HeaderValue::from_static(PAGE_POLICY));
    }
    response
}

/// The file at `path`, a URL path, under the folder `root`; a path that
/// would leave the folder is refused.
fn folder_file(root: &Path, path: &str) -> Result<(Vec<u8>, &'static str), (StatusCode, String)> {
    let decoded = percent_decode(path).ok_or_else(|| (StatusCode::BAD_REQUEST, format!("{path} is not a path this page has")))?;
    let mut full = root.to_path_buf();
    let mut any = false;
    for part in decoded.split('/') {
        match part {
            "" | "." => {}
            ".." => return Err((StatusCode::FORBIDDEN, format!("{path} leaves the page's folder"))),
            p if p.contains('\\') || p.contains(':') => return Err((StatusCode::FORBIDDEN, format!("{path} names a file outside the page's folder"))),
            p => {
                full.push(p);
                any = true;
            }
        }
    }
    if !any {
        full.push("index.html");
    }
    match std::fs::read(&full) {
        Ok(bytes) => Ok((bytes, mime_of(&full))),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Err((StatusCode::NOT_FOUND, format!("{path} is not in the page's folder"))),
        Err(e) => Err((StatusCode::INTERNAL_SERVER_ERROR, format!("{} cannot be read: {e}", full.display()))),
    }
}

/// A URL path's bytes with every `%XX` decoded, or `None` when it is not
/// UTF-8 or a `%` starts no two hex digits.
fn percent_decode(path: &str) -> Option<String> {
    let bytes = path.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            let hex = path.get(i + 1..i + 3)?;
            out.push(u8::from_str_radix(hex, 16).ok()?);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8(out).ok()
}

/// A relative file path as a URL path, every byte but the unreserved ones
/// and `/` written as `%XX`.
fn encode_path(path: &str) -> String {
    let mut out = String::with_capacity(path.len());
    for b in path.replace('\\', "/").bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'_' | b'~' | b'/') {
            out.push(char::from(b));
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// The media type a file is sent as, by its extension.
fn mime_of(path: &Path) -> &'static str {
    let ext = path.extension().and_then(|e| e.to_str()).map(str::to_ascii_lowercase);
    match ext.as_deref() {
        Some("html" | "htm") => "text/html; charset=utf-8",
        Some("js" | "mjs") => "text/javascript; charset=utf-8",
        Some("css") => "text/css; charset=utf-8",
        Some("json") => "application/json",
        Some("svg") => "image/svg+xml",
        Some("png") => "image/png",
        Some("jpg" | "jpeg") => "image/jpeg",
        Some("gif") => "image/gif",
        Some("webp") => "image/webp",
        Some("ico") => "image/x-icon",
        Some("wasm") => "application/wasm",
        Some("txt") => "text/plain; charset=utf-8",
        Some("woff") => "font/woff",
        Some("woff2") => "font/woff2",
        Some("ttf") => "font/ttf",
        Some("otf") => "font/otf",
        _ => "application/octet-stream",
    }
}

/// The engine drawing `webview` as it reports itself now, its version
/// `version`; when it cannot report itself, none and why.
fn engine_report(webview: &WebView, version: &str) -> (Option<proto::Engine>, Option<String>) {
    match chrome::settings(webview).and_then(|settings| chrome::folder(webview).map(|folder| (settings, folder))) {
        Ok((settings, folder)) => (Some(proto::Engine { name: engine().to_string(), version: version.to_string(), folder: Some(folder), settings }), None),
        Err(why) => (None, Some(format!("the engine did not report itself, so the snapshot has none: {why}"))),
    }
}

/// The box each named node was drawn in, from a page's snapshot reply, in
/// the page's CSS pixels, which are points.
fn page_bounds(tree: &Node, msg: &Json) -> Result<BTreeMap<NodeId, proto::Bounds>, String> {
    let entries = msg.get("bounds").and_then(Json::as_object).ok_or_else(|| format!("a snapshot reply has no bounds: {msg}"))?;
    entries
        .iter()
        .map(|(k, v)| {
            let id = k.parse::<NodeId>().map_err(|e| format!("a snapshot reply keys bounds by {k}, which is not a node id: {e}"))?;
            if tree.find(id).is_none() {
                return Err(format!("a snapshot reply gives bounds for node {id}, which the window lacks"));
            }
            let number = |key: &str| v.get(key).and_then(Json::as_f64).map(|n| n as f32).ok_or_else(|| format!("node {id}'s bounds have no {key}: {v}"));
            Ok((id, proto::Bounds { x: number("x")?, y: number("y")?, width: number("width")?, height: number("height")? }))
        })
        .collect()
}

/// The widget with the keyboard focus, from the node a page's snapshot
/// reply gives as focused: the innermost widget around that node that
/// takes the focus ([`Node::focus_holder`]); none when the reply gives
/// null.
fn page_focused(tree: &Node, msg: &Json) -> Result<Option<NodeId>, String> {
    match msg.get("focused") {
        None => Err(format!("a snapshot reply has no focused widget: {msg}")),
        Some(Json::Null) => Ok(None),
        Some(v) => {
            let n = v.as_u64().ok_or_else(|| format!("a snapshot reply gives {v} as the focused widget, which is not a node id"))?;
            let id = NodeId::try_from(n).map_err(|e| format!("a snapshot reply gives {n} as the focused widget, which is not a node id: {e}"))?;
            Ok(tree.focus_holder(id))
        }
    }
}

/// Whether each password field offers its peek button, from a page's
/// snapshot reply.
fn page_peeks(tree: &Node, msg: &Json) -> Result<BTreeMap<NodeId, bool>, String> {
    let entries = msg.get("peeks").and_then(Json::as_object).ok_or_else(|| format!("a snapshot reply has no peek buttons: {msg}"))?;
    entries
        .iter()
        .map(|(k, v)| {
            let id = k.parse::<NodeId>().map_err(|e| format!("a snapshot reply keys peek buttons by {k}, which is not a node id: {e}"))?;
            if !tree.find(id).is_some_and(Node::is_password) {
                return Err(format!("a snapshot reply gives a peek button for node {id}, which is no password field of the window"));
            }
            let offered = v.as_bool().ok_or_else(|| format!("a snapshot reply gives {v} for node {id}'s peek button, which is not true or false"))?;
            Ok((id, offered))
        })
        .collect()
}

/// The input state a page's snapshot reply has, by node id.
fn page_state(tree: &Node, msg: &Json) -> Result<BTreeMap<NodeId, Value>, String> {
    let entries = msg.get("state").and_then(Json::as_object).ok_or_else(|| format!("a snapshot reply has no state: {msg}"))?;
    entries
        .iter()
        .map(|(k, v)| k.parse::<NodeId>().map(|id| (id, as_sent(tree, id, value_from(v)))).map_err(|e| format!("a snapshot reply keys state by {k}, which is not a node id: {e}")))
        .collect()
}

/// A value the page gave for node `node`, with a number field's or
/// slider's whole number, a chart point's whole-number X and Y, a GPU
/// surface's or a canvas's pointer event's whole-number place, scale,
/// notches and place among the items, and a media player's whole-number
/// position and length, as the `Float` the protocol sends it as: JSON does
/// not tell 5 from 5.0.
fn as_sent(tree: &Node, node: NodeId, value: Value) -> Value {
    let floats = |mut map: BTreeMap<String, Value>, keys: &[&str]| {
        for key in keys {
            if let Some(Value::Int(i)) = map.get(*key) {
                let whole = *i as f64;
                map.insert((*key).to_string(), Value::Float(whole));
            }
        }
        Value::Map(map)
    };
    match (tree.find(node).map(|n| n.kind), value) {
        (Some(Kind::NumberBox | Kind::Slider | Kind::MediaPlayer), Value::Int(i)) => Value::Float(i as f64),
        (Some(Kind::MediaPlayer), Value::Map(loaded)) => floats(loaded, &["duration"]),
        (Some(Kind::Chart), Value::Map(point)) => floats(point, &["x", "y"]),
        (Some(Kind::GpuSurface), Value::Map(at)) => floats(at, &["x", "y", "u", "v", "ppp", "notches"]),
        (Some(Kind::Canvas), Value::Map(at)) => floats(at, &["x", "y", "u", "v", "ppp", "notches", "scenex", "sceney"]),
        (_, value) => value,
    }
}

/// What a test event comes to in a window of widgets: one the renderer
/// raises itself, on the node with the value, or one the page acts on,
/// with the value it takes.
enum HelperTest {
    Raise(NodeId, Value),
    Page(Value),
    /// A file chooser's Browse button, pressed as the page's own press.
    Browse(NodeId),
}

/// A test event as the renderer takes it: files or text dropped on a widget
/// whose `drop` is true, or with no `name` on the window's root; a key for
/// a widget whose `keys` is true, which it does not use itself
/// ([`Shortcut::widget_keeps`]) and, on a Canvas, its view in `canvases`
/// does not take; a click on a chart's point
/// ([`proto::chart::Chart::find`]); a date chooser's minute and a file
/// chooser's paths, checked and written as the page stores them; anything
/// else as given, for the page.
fn helper_test(tree: &Node, canvases: &BTreeMap<NodeId, WebCanvas>, name: &str, kind: EventKind, value: &Value) -> Result<HelperTest, String> {
    let on = |n: &Node, key: &str| matches!(n.props.get(key), Some(Value::Bool(true)));
    if kind == EventKind::Drop && name.is_empty() {
        if !on(tree, "drop") {
            return Err(String::from("the window takes no drop of its own; Show-GuiWindow -OnDrop gives it one"));
        }
        return Ok(HelperTest::Raise(tree.id, dropped(value)?));
    }
    let Some(node) = tree.find_named(name) else {
        return Ok(HelperTest::Page(value.clone()));
    };
    match (node.kind, kind) {
        (Kind::FilePicker, EventKind::Click) => Ok(HelperTest::Browse(node.id)),
        (_, EventKind::Drop) if on(node, "drop") => Ok(HelperTest::Raise(node.id, dropped(value)?)),
        (_, EventKind::Drop) => Err(format!("{name} takes no drop; its command's -OnDrop gives it one")),
        (k, EventKind::Key) if on(node, "keys") => {
            let written = value.as_str().ok_or_else(|| format!("{value:?} is not a key written as a shortcut"))?;
            let pressed = Shortcut::parse(written)?;
            let typed = matches!(k, Kind::TextBox | Kind::NumberBox) || (k == Kind::ComboBox && on(node, "editable"));
            if pressed.node_keeps(node, typed) {
                return Err(format!("a {k:?} uses {} itself, so it raises no Key", pressed.written()));
            }
            if k == Kind::Canvas && view_keeps(canvases, node.id, &pressed) {
                return Err(format!("the canvas's view takes {} itself, so it raises no Key", pressed.written()));
            }
            Ok(HelperTest::Raise(node.id, Value::Str(pressed.written())))
        }
        (_, EventKind::Key) => Err(format!("{name} raises no Key; its command's -OnKey gives it one")),
        (Kind::Canvas, _) => Ok(HelperTest::Page(canvas_test(node, name, kind, value)?)),
        (Kind::Chart, EventKind::Select) => Ok(HelperTest::Raise(node.id, proto::chart::Chart::of(node)?.find(value)?)),
        (Kind::DatePicker, EventKind::Change) => {
            let minute = |v: Option<&Value>| match v {
                None | Some(Value::Null) => Ok(None),
                Some(Value::Str(text)) => proto::date::Minute::parse(text).map(Some),
                Some(other) => Err(format!("{other:?} is not a date and time")),
            };
            let wanted = minute(Some(value))?.ok_or_else(|| String::from("a person chooses a date and time, and none was given"))?;
            let (low, high) = (minute(node.props.get("minimum"))?, minute(node.props.get("maximum"))?);
            if low.is_some_and(|l| wanted < l) || high.is_some_and(|h| wanted > h) {
                return Err(format!("{} is outside the chooser's range", wanted.written()));
            }
            Ok(HelperTest::Page(Value::Str(wanted.written())))
        }
        (Kind::FilePicker, EventKind::Change) => {
            let paths = match value {
                Value::Str(p) => vec![Value::Str(p.clone())],
                Value::List(items) => items.iter().map(|p| p.as_str().map(|s| Value::Str(s.to_string())).ok_or_else(|| format!("{p:?} is not a path"))).collect::<Result<Vec<Value>, String>>()?,
                other => return Err(format!("{other:?} is not a path or a list of paths")),
            };
            if paths.is_empty() {
                return Err(String::from("a person chooses at least one path"));
            }
            if paths.len() > 1 && !on(node, "multiple") {
                return Err(format!("the chooser takes one path without Multiple, and {} were given", paths.len()));
            }
            Ok(HelperTest::Page(Value::List(paths)))
        }
        _ => Ok(HelperTest::Page(value.clone())),
    }
}

/// A Canvas's test event of `kind`, checked as the egui renderer checks it,
/// as the page raises it: an event the node requests, and `given`, null or
/// a map, its keys compared without regard to case, of Item, the name of an
/// item to put the pointer on at the middle of where it was drawn, or of U
/// and V, fractions of the canvas, with Button, Shift, Ctrl and Alt, and a
/// wheel turn's Notches. The page takes it as one map of `item`, `u`, `v`,
/// `button`, Left where a press, a release or an item's event other than
/// PointerEnter and PointerLeave names none, `shift`, `ctrl`, `alt`,
/// `notches` and whether it is an item's event, which raises nothing where
/// no named item is.
fn canvas_test(node: &Node, name: &str, kind: EventKind, given: &Value) -> Result<Value, String> {
    use EventKind::{Click, DoubleClick, DragEnd, DragMove, DragStart, PointerDown, PointerEnter, PointerLeave, PointerMove, PointerUp, Wheel};
    let on = |key: &str| matches!(node.props.get(key), Some(Value::Bool(true)));
    let asked = match kind {
        Click => on("click"),
        DoubleClick => on("doubleClick"),
        PointerEnter => on("pointerEnter"),
        PointerLeave => on("pointerLeave"),
        DragStart => on("dragStart"),
        DragMove => on("dragMove"),
        DragEnd => on("dragEnd"),
        PointerDown => on("pointerDown"),
        PointerUp => on("pointerUp"),
        PointerMove => on("pointerMove"),
        Wheel => on("wheel"),
        other => return Err(format!("a Canvas does not raise {other:?}")),
    };
    if !asked {
        return Err(format!("{name} raises no {kind:?}; its command's -On{kind:?} gives it one"));
    }
    let mut keys = match given {
        Value::Map(m) => m.clone(),
        Value::Null => BTreeMap::new(),
        other => return Err(format!("a canvas's test event takes a hashtable of Item, or of U and V, not {other:?}")),
    };
    let item = match keys.keys().find(|k| k.eq_ignore_ascii_case("item")).cloned() {
        Some(key) => match keys.remove(&key) {
            Some(Value::Str(s)) => Some(s),
            other => return Err(format!("Item names the item to put the pointer on, and is {other:?}")),
        },
        None => None,
    };
    let get = |key: &str| keys.iter().find(|(k, _)| k.eq_ignore_ascii_case(key)).map(|(_, v)| v);
    let fraction = |key: &str| match &item {
        Some(_) => Ok(Value::Null),
        None => get(key).and_then(Value::as_number).map(Value::Float).ok_or_else(|| format!("a pointer test event takes {key}, a fraction of the surface")),
    };
    let (u, v) = (fraction("u")?, fraction("v")?);
    let held = |key: &str| match get(key) {
        None => Ok(false),
        Some(Value::Bool(on)) => Ok(*on),
        Some(other) => Err(format!("{key} is true or false, not {other:?}")),
    };
    let (shift, ctrl, alt) = (held("shift")?, held("ctrl")?, held("alt")?);
    let surface_kind = match kind {
        PointerUp | DragEnd => PointerUp,
        PointerMove | DragMove | PointerEnter | PointerLeave => PointerMove,
        Wheel => Wheel,
        _ => PointerDown,
    };
    let button = match (surface_kind, get("button")) {
        (Wheel, _) | (PointerMove, None | Some(Value::Null)) => None,
        (_, None | Some(Value::Null)) => Some("Left"),
        (_, Some(Value::Str(named))) => Some(["Left", "Right", "Middle"].into_iter().find(|b| b.eq_ignore_ascii_case(named)).ok_or_else(|| format!("{named} is not a pointer button: Left, Right or Middle"))?),
        (_, Some(other)) => return Err(format!("button is Left, Right or Middle, not {other:?}")),
    };
    let notches = match (surface_kind, get("notches")) {
        (Wheel, None) => Value::Float(1.0),
        (Wheel, Some(n)) => Value::Float(n.as_number().ok_or_else(|| format!("notches is a number, not {n:?}"))?),
        _ => Value::Null,
    };
    let item_event = matches!(kind, Click | DoubleClick | PointerEnter | PointerLeave | DragStart | DragMove | DragEnd);
    let button = match button {
        None if item_event && !matches!(kind, PointerEnter | PointerLeave) => Some("Left"),
        b => b,
    };
    Ok(Value::Map(BTreeMap::from([
        (String::from("item"), item.map_or(Value::Null, Value::Str)),
        (String::from("u"), u),
        (String::from("v"), v),
        (String::from("button"), button.map_or(Value::Null, |b| Value::Str(b.to_string()))),
        (String::from("shift"), Value::Bool(shift)),
        (String::from("ctrl"), Value::Bool(ctrl)),
        (String::from("alt"), Value::Bool(alt)),
        (String::from("notches"), notches),
        (String::from("itemEvent"), Value::Bool(item_event)),
    ])))
}

/// What a test drops: the paths of files, a list of at least one, or text,
/// which a page on Windows takes none of: wry's drop handler stands in for
/// WebView2's there, and gives a page files alone.
fn dropped(value: &Value) -> Result<Value, String> {
    match value {
        Value::Str(_) if cfg!(windows) => Err(String::from("a page on Windows takes files dropped on it, given as a list of their paths; its drop handler takes no text")),
        Value::Str(text) => Ok(Value::Str(text.clone())),
        Value::List(items) if !items.is_empty() => {
            items.iter().map(|p| p.as_str().map(|s| Value::Str(s.to_string())).ok_or_else(|| format!("{p:?} is not a path"))).collect::<Result<Vec<Value>, String>>().map(Value::List)
        }
        Value::List(_) => Err(String::from("a drop has at least one file")),
        other => Err(format!("{other:?} is neither paths nor text")),
    }
}

/// The value a page's reply to a test event says its widget took.
fn test_answer(tree: &Node, msg: &Json) -> Result<Value, String> {
    let value = value_from(msg.get("ok").unwrap_or(&Json::Null));
    match msg.get("node").and_then(Json::as_u64).map(NodeId::try_from) {
        Some(Ok(node)) => Ok(as_sent(tree, node, value)),
        Some(Err(e)) => Err(format!("the page replied to a test event for a node number out of range: {e}")),
        None => Err(format!("the page replied to a test event without its node: {msg}")),
    }
}

/// The node a page's message names.
fn page_node(msg: &Json) -> Result<NodeId, String> {
    match msg.get("node").and_then(Json::as_u64).map(NodeId::try_from) {
        Some(Ok(node)) => Ok(node),
        Some(Err(e)) => Err(format!("the page named a node number out of range: {e}")),
        None => Err(format!("the page sent a message without its node: {msg}")),
    }
}

/// The size, in points across and down, a page measured its window's
/// widgets at; or why it could not measure them, or what it sent that is
/// no size.
fn page_size(msg: &Json) -> Result<[f32; 2], String> {
    let side = |v: &Json| v.as_f64().filter(|s| s.is_finite() && *s >= 0.0).map(|s| s as f32);
    match msg.get("size").and_then(Json::as_array).map(Vec::as_slice) {
        Some([across, down]) => match (side(across), side(down)) {
            (Some(a), Some(d)) => Ok([a, d]),
            _ => Err(format!("the page measured its widgets at a size that is not two lengths: {msg}")),
        },
        _ => Err(match msg.get("error").and_then(Json::as_str) {
            Some(why) => why.to_string(),
            None => format!("the page measured its widgets at no size: {msg}"),
        }),
    }
}

/// The Key a page's key press raises: the keys pressed, written as a
/// shortcut, on a node whose `keys` is true that does not use them itself
/// ([`Shortcut::node_keeps`]), the page saying with `typed` whether a
/// field typed into had them, and on a Canvas whose view in `canvases`
/// does not take them; none when the node or its view takes them.
fn page_key(tree: &Node, canvases: &BTreeMap<NodeId, WebCanvas>, msg: &Json) -> Result<Option<(NodeId, String)>, String> {
    let id = page_node(msg)?;
    let node = tree.find(id).ok_or_else(|| format!("the page sent a key for node {id}, which the window lacks"))?;
    if !matches!(node.props.get("keys"), Some(Value::Bool(true))) {
        return Err(format!("the page sent a key for node {id}, which raises no Key"));
    }
    let written = msg.get("keys").and_then(Json::as_str).ok_or_else(|| format!("the page sent a key without its keys: {msg}"))?;
    let pressed = Shortcut::parse(written)?;
    let typed = msg.get("typed").and_then(Json::as_bool) == Some(true) || node.kind == Kind::NumberBox;
    let kept = pressed.node_keeps(node, typed) || (node.kind == Kind::Canvas && view_keeps(canvases, id, &pressed));
    Ok((!kept).then(|| (id, pressed.written())))
}

/// Whether the view of the canvas `node` in `canvases` takes the press
/// `pressed` from the canvas's key handler.
fn view_keeps(canvases: &BTreeMap<NodeId, WebCanvas>, node: NodeId, pressed: &Shortcut) -> bool {
    canvases.get(&node).and_then(|c| c.view.as_ref()).is_some_and(|shown| shown.view.controls.keeps(pressed))
}

/// The reply to a page on `engine` requesting a draw's GLSL ES 3.00, as a
/// page with WebGL 2 alone runs it: the stages [`glsl::translate`]
/// writes from the WGSL `source`, entry points and constants the page
/// gives, or the error the draw is refused with, GuiGpuNoWebGpu for what
/// WebGL 2 has no place for.
fn page_glsl(msg: &Json, engine: &str) -> Json {
    let refused = |id: &str, text: String| json!({ "error": { "id": id, "text": text } });
    let Some(source) = msg.get("source").and_then(Json::as_str) else {
        return refused("GuiGpuDraw", String::from("the page requested a draw's GLSL without its WGSL"));
    };
    let constants = match msg.get("constants") {
        None | Some(Json::Null) => Vec::new(),
        Some(Json::Object(given)) => match given.iter().map(|(k, v)| v.as_f64().map(|n| (k.clone(), n))).collect::<Option<Vec<(String, f64)>>>() {
            Some(c) => c,
            None => return refused("GuiGpuDraw", format!("the page requested a draw's GLSL with constants that are not all numbers: {}", Json::Object(given.clone()))),
        },
        Some(other) => return refused("GuiGpuDraw", format!("the page requested a draw's GLSL with constants that are not a map of numbers: {other}")),
    };
    let entry = |key: &str| msg.get(key).and_then(Json::as_str);
    match glsl::translate(source, entry("vertexEntry"), entry("fragmentEntry"), &constants) {
        Ok(stages) => stages,
        Err(glsl::Unwritable::NeedsWebGpu(what)) => refused("GuiGpuNoWebGpu", format!("{engine} has no WebGPU, and {what}; a window opened with -Renderer Egui draws it")),
        Err(glsl::Unwritable::Refused(text)) => refused("GuiGpuDraw", format!("the draw cannot be written for WebGL 2: {text}")),
    }
}

/// Where a page reports a person moved a GpuSurface's camera: the node,
/// and the camera's target, distance, yaw and pitch, each finite.
fn page_pose(msg: &Json) -> Result<(NodeId, [f64; 3], f64, f64, f64), String> {
    let node = page_node(msg)?;
    let pose = msg.get("pose").ok_or_else(|| format!("the page reported a camera without its pose: {msg}"))?;
    let number = |v: Option<&Json>, what: &str| v.and_then(Json::as_f64).filter(|n| n.is_finite()).ok_or_else(|| format!("the page reported a camera whose {what} is not a finite number: {msg}"));
    let target = pose.get("target").and_then(Json::as_array).filter(|t| t.len() == 3).ok_or_else(|| format!("the page reported a camera whose target is not three numbers: {msg}"))?;
    let target = [number(target.first(), "target")?, number(target.get(1), "target")?, number(target.get(2), "target")?];
    Ok((node, target, number(pose.get("distance"), "distance")?, number(pose.get("yaw"), "yaw")?, number(pose.get("pitch"), "pitch")?))
}

/// The page's number for the request a reply is for.
fn page_ask(msg: &Json) -> Option<u32> {
    msg.get("request").and_then(Json::as_u64).and_then(|a| u32::try_from(a).ok())
}

/// The text a page's snapshot reply says its window last put on the
/// clipboard; none when the reply gives null.
fn page_copied(msg: &Json) -> Result<Option<String>, String> {
    match msg.get("copied") {
        None => Err(format!("a snapshot reply has no text last copied: {msg}")),
        Some(Json::Null) => Ok(None),
        Some(Json::String(text)) => Ok(Some(text.clone())),
        Some(v) => Err(format!("a snapshot reply gives {v} as the text last copied, which is not text")),
    }
}

/// The sound each media player of `tree` handed the system, from a page's
/// snapshot reply: its peak of the last half second and its seconds
/// played, or none where the page does not measure it, as for a player not
/// drawn yet.
fn page_sound(tree: &Node, msg: &Json) -> Result<BTreeMap<NodeId, Option<proto::media::Sound>>, String> {
    let entries = msg.get("sound").and_then(Json::as_object).ok_or_else(|| format!("a snapshot reply has no players' sound: {msg}"))?;
    let mut sound = BTreeMap::new();
    for (k, v) in entries {
        let id = k.parse::<NodeId>().map_err(|e| format!("a snapshot reply keys the players' sound by {k}, which is not a node id: {e}"))?;
        if !tree.find(id).is_some_and(|n| n.kind == Kind::MediaPlayer) {
            return Err(format!("a snapshot reply gives sound for node {id}, which is no media player of the window"));
        }
        let measured = match v {
            Json::Null => None,
            v => {
                let number = |key: &str| v.get(key).and_then(Json::as_f64).ok_or_else(|| format!("a snapshot reply gives node {id}'s sound as {v}, with no {key}"));
                Some(proto::media::Sound { peak: number("peak")? as f32, seconds: number("seconds")? })
            }
        };
        sound.insert(id, measured);
    }
    let mut stack = vec![tree];
    while let Some(n) = stack.pop() {
        stack.extend(n.children.iter());
        if n.kind == Kind::MediaPlayer {
            sound.entry(n.id).or_insert(None);
        }
    }
    Ok(sound)
}

/// The widget rectangle, when one was requested, and the viewport size a
/// page's snapshot reply has for a capture, in CSS pixels.
fn page_view(msg: &Json) -> Result<(Option<capture::Rect>, (f64, f64)), String> {
    let number = |v: &Json, key: &str| v.get(key).and_then(Json::as_f64).ok_or_else(|| format!("a snapshot reply has no {key}: {msg}"));
    let view = msg.get("viewport").ok_or_else(|| format!("a snapshot reply has no viewport: {msg}"))?;
    let viewport = (number(view, "width")?, number(view, "height")?);
    let rect = match msg.get("rect") {
        None | Some(Json::Null) => None,
        Some(r) => Some(capture::Rect { x: number(r, "x")?, y: number(r, "y")?, width: number(r, "width")?, height: number(r, "height")? }),
    };
    Ok((rect, viewport))
}

/// The ring a message names that an attachment belongs to.
fn attached_ring(msg: &ToHelper) -> Option<&str> {
    match msg {
        ToHelper::OpenWindow { ring, .. } => Some(ring),
        ToHelper::ListAdapters { reply, .. }
        | ToHelper::ListWindows { reply, .. }
        | ToHelper::PageGraphics { reply, .. }
        | ToHelper::TestEvent { reply, .. }
        | ToHelper::TestInput { reply, .. }
        | ToHelper::Snapshot { reply, .. }
        | ToHelper::Bridge { reply, .. }
        | ToHelper::CloseOpen { reply, .. }
        | ToHelper::FocusNode { reply, .. }
        | ToHelper::Dialog { reply, .. }
        | ToHelper::ListDialogs { reply, .. }
        | ToHelper::AnswerDialog { reply, .. }
        | ToHelper::Gpu { reply, .. }
        | ToHelper::Media { reply, .. }
        | ToHelper::Probe { reply, .. } => Some(reply),
        ToHelper::Patch { .. }
        | ToHelper::Rows { .. }
        | ToHelper::CloseWindow { .. }
        | ToHelper::Detach { .. }
        | ToHelper::Focus { .. }
        | ToHelper::ForgetReply { .. }
        | ToHelper::StopInput { .. }
        | ToHelper::BridgeReply { .. }
        | ToHelper::Shutdown
        | ToHelper::CloseDialog { .. }
        | ToHelper::PickerAnswer { .. }
        | ToHelper::GpuFree { .. }
        | ToHelper::GpuFreePipeline { .. }
        | ToHelper::GpuFreeTexture { .. }
        | ToHelper::GpuFreeSampler { .. } => None,
    }
}

/// An event a page sent: the node, the kind and the value.
fn page_event(msg: &Json) -> Result<(NodeId, EventKind, Value), String> {
    let node = msg.get("node").and_then(Json::as_u64).and_then(|n| NodeId::try_from(n).ok()).ok_or_else(|| format!("an event names no node: {msg}"))?;
    let kind = match msg.get("kind").and_then(Json::as_str) {
        Some("Click") => EventKind::Click,
        Some("Change") => EventKind::Change,
        Some("Select") => EventKind::Select,
        Some("Activate") => EventKind::Activate,
        Some("Submit") => EventKind::Submit,
        Some("Shortcut") => EventKind::Shortcut,
        Some("Expand") => EventKind::Expand,
        Some("Drop") => EventKind::Drop,
        Some("PointerDown") => EventKind::PointerDown,
        Some("PointerUp") => EventKind::PointerUp,
        Some("PointerMove") => EventKind::PointerMove,
        Some("Wheel") => EventKind::Wheel,
        Some("DoubleClick") => EventKind::DoubleClick,
        Some("PointerEnter") => EventKind::PointerEnter,
        Some("PointerLeave") => EventKind::PointerLeave,
        Some("DragStart") => EventKind::DragStart,
        Some("DragMove") => EventKind::DragMove,
        Some("DragEnd") => EventKind::DragEnd,
        Some("Loaded") => EventKind::Loaded,
        Some("Playing") => EventKind::Playing,
        Some("Paused") => EventKind::Paused,
        Some("Ended") => EventKind::Ended,
        Some("Position") => EventKind::Position,
        Some("Error") => EventKind::Error,
        other => return Err(format!("an event of an unrecognized kind ({other:?}): {msg}")),
    };
    Ok((node, kind, value_from(msg.get("value").unwrap_or(&Json::Null))))
}

/// A call on a media player as the page's `pwrsRuntime.media` takes it.
fn media_op_json(op: &proto::media::MediaOp) -> Json {
    use proto::media::MediaOp;
    match op {
        MediaOp::Play => json!({ "op": "play" }),
        MediaOp::Pause => json!({ "op": "pause" }),
        MediaOp::Seek { seconds } => json!({ "op": "seek", "seconds": seconds }),
        MediaOp::Rate { rate } => json!({ "op": "rate", "rate": rate }),
        MediaOp::Volume { volume } => json!({ "op": "volume", "volume": volume }),
        MediaOp::Muted { muted } => json!({ "op": "muted", "muted": muted }),
        MediaOp::Loop { looped } => json!({ "op": "loop", "looped": looped }),
        MediaOp::State => json!({ "op": "state" }),
    }
}

/// A media player's state from the page's reply to a call: its `state`, a
/// map of the phase's name, the position, duration, rate, volume, mute and
/// loop, the picture's width and height (0 or null for none), whether it
/// has sound, and its error, MediaError's code and the engine's text, which
/// `word` words as an Error event's message.
fn page_media_state(msg: &Json, word: impl Fn(i64, &str) -> String) -> Result<proto::media::MediaState, String> {
    use proto::media::{MediaState, Phase};
    let state = msg.get("state").ok_or_else(|| format!("a media reply has no state: {msg}"))?;
    let number = |key: &str| state.get(key).and_then(Json::as_f64).ok_or_else(|| format!("a media reply's state has no {key}: {msg}"));
    let flag = |key: &str| state.get(key).and_then(Json::as_bool).ok_or_else(|| format!("a media reply's state has no {key}: {msg}"));
    let phase = match state.get("phase").and_then(Json::as_str) {
        Some("Loading") => Phase::Loading,
        Some("Paused") => Phase::Paused,
        Some("Playing") => Phase::Playing,
        Some("Ended") => Phase::Ended,
        Some("Failed") => Phase::Failed,
        other => return Err(format!("a media reply's phase is {other:?}: {msg}")),
    };
    let side = |key: &str| -> Result<u32, String> {
        match state.get(key) {
            None | Some(Json::Null) => Ok(0),
            Some(v) => {
                let n = v.as_u64().ok_or_else(|| format!("a media reply's {key} is not a size in pixels: {msg}"))?;
                u32::try_from(n).map_err(|e| format!("a media reply's {key} of {n} pixels is too large: {e}"))
            }
        }
    };
    let (width, height) = (side("width")?, side("height")?);
    let error = match state.get("error") {
        None | Some(Json::Null) => None,
        Some(e) => {
            let code = e.get("code").and_then(Json::as_i64).ok_or_else(|| format!("a media reply's error has no MediaError code: {msg}"))?;
            Some(word(code, e.get("detail").and_then(Json::as_str).unwrap_or("it gave no reason")))
        }
    };
    Ok(MediaState {
        phase,
        position: number("position")?,
        duration: state.get("duration").and_then(Json::as_f64).filter(|d| d.is_finite() && *d > 0.0),
        rate: number("rate")?,
        volume: number("volume")?,
        muted: flag("muted")?,
        looped: flag("looped")?,
        video: (width > 0 && height > 0).then_some([width, height]),
        audio: flag("audio")?,
        error,
    })
}

/// Applies a patch to the tree the renderer stores for names and snapshots
/// and to its `canvases`, at `now`, naming in `pictures` the pictures the
/// change shows, and returns the change as the page applies it to what it
/// draws, written as the window stands right after it: a Chart's new
/// properties or points with its data as the page draws it
/// ([`chart_json`]), points added to a chart as that data alone, and a
/// change to a Canvas's items with the items it changes as they then stand
/// ([`item_json`]). A Canvas's items change on its board, its node still
/// having the items it was given, and a node that brings canvases whose items
/// cannot be read is refused before anything changes.
fn apply(tree: &mut Node, canvases: &mut BTreeMap<NodeId, WebCanvas>, pictures: &mut Pictures, op: &PatchOp, now: Instant) -> Result<Json, String> {
    let chart = |tree: &Node, node: NodeId| tree.find(node).filter(|n| n.kind == Kind::Chart).map(chart_json);
    match op {
        PatchOp::SetProps { node, props } => {
            let n = tree.find_mut(*node).ok_or_else(|| format!("no node {node}"))?;
            let canvas = n.kind == Kind::Canvas && props.contains_key("items");
            if canvas {
                let board = proto::canvas::Board::of_items(props.get("items").unwrap_or(&Value::Null)).map_err(|e| format!("node {node}: {e}"))?;
                let view = canvases.remove(node).and_then(|c| c.view);
                canvases.insert(*node, WebCanvas::of(board, view, now));
            }
            n.props.extend(props.clone());
            name_node_pictures(n, canvases, pictures);
            let showing = Showing { pictures, canvases, now };
            let mut shown = props_json(props, &showing, canvas.then_some(*node));
            if let (Some(c), Json::Object(m)) = (chart(tree, *node), &mut shown) {
                m.insert(String::from("chart"), c);
            }
            Ok(json!({ "op": "set", "node": node, "props": shown }))
        }
        PatchOp::Insert { parent, index, node } => {
            let boards = boards_in(node)?;
            let p = tree.find_mut(*parent).ok_or_else(|| format!("no node {parent}"))?;
            let at = usize::try_from(*index).map_err(|e| format!("position {index} is beyond this platform's address space: {e}"))?.min(p.children.len());
            p.children.insert(at, node.clone());
            take_boards(canvases, boards, now);
            name_node_pictures(node, canvases, pictures);
            Ok(json!({ "op": "insert", "parent": parent, "index": index, "node": node_json(node, &Showing { pictures, canvases, now }) }))
        }
        PatchOp::Remove { node } => {
            if !remove(tree, *node) {
                return Err(format!("no node {node} to remove"));
            }
            Ok(json!({ "op": "remove", "node": node }))
        }
        PatchOp::Replace { node } => {
            let boards = boards_in(node)?;
            let n = tree.find_mut(node.id).ok_or_else(|| format!("no node {} to replace", node.id))?;
            *n = node.clone();
            take_boards(canvases, boards, now);
            name_node_pictures(node, canvases, pictures);
            Ok(json!({ "op": "replace", "node": node_json(node, &Showing { pictures, canvases, now }) }))
        }
        PatchOp::Append { node, prop, values } => {
            let n = tree.find_mut(*node).ok_or_else(|| format!("no node {node}"))?;
            match n.props.entry(prop.clone()).or_insert_with(|| Value::List(Vec::new())) {
                Value::List(items) => items.extend(values.iter().cloned()),
                other => return Err(format!("node {node}'s {prop} is {other:?}, not a list to add to")),
            }
            Ok(match chart(tree, *node) {
                Some(c) => json!({ "op": "set", "node": node, "props": { "chart": c } }),
                None => json!({ "op": "append", "node": node, "prop": prop, "values": Json::Array(values.iter().map(value_json).collect()) }),
            })
        }
        PatchOp::RemoveRows { node, rows } => match tree.find(*node) {
            Some(n) if n.kind == Kind::Table => Ok(json!({ "op": "removeRows", "node": node, "rows": rows })),
            Some(n) => Err(format!("node {node} is a {:?}, which has no rows to take away", n.kind)),
            None => Err(format!("no node {node}")),
        },
        PatchOp::SetItem { node, item, props } => {
            let (board, motions) = canvas_items(tree, canvases, *node)?;
            let id = board.set(item, props).map_err(|e| format!("node {node}: {e}"))?;
            motions.changed(board, id, props, now);
            name_pictures(board, &[id], pictures);
            let set: Vec<&String> = props.keys().filter(|k| k.as_str() != "animate").collect();
            let animate = match props.get("animate") {
                None => Json::Null,
                Some(Value::Null) => Json::from("clear"),
                Some(_) => Json::from("start"),
            };
            Ok(json!({ "op": "setItem", "node": node, "item": item_json(board, id, pictures, false, None), "set": set, "animate": animate }))
        }
        PatchOp::AddItems { node, group, items: added } => {
            let (board, motions) = canvas_items(tree, canvases, *node)?;
            let ids = board.add(group.as_deref(), added).map_err(|e| format!("node {node}: {e}"))?;
            motions.started(board, &ids, now);
            name_pictures(board, &ids, pictures);
            let parent = group.as_deref().and_then(|g| board.find(g));
            let top: Vec<Json> = ids.iter().filter(|id| board.entry(**id).is_some_and(|e| e.parent == parent)).map(|id| item_json(board, *id, pictures, true, None)).collect();
            Ok(json!({ "op": "addItems", "node": node, "group": parent, "items": top }))
        }
        PatchOp::RemoveItems { node, names } => {
            let (board, motions) = canvas_items(tree, canvases, *node)?;
            let gone = board.remove(names).map_err(|e| format!("node {node}: {e}"))?;
            motions.forget(&gone);
            Ok(json!({ "op": "removeItems", "node": node, "ids": gone }))
        }
    }
}

/// Names in `pictures` every picture the subtree of `n` shows: its Images'
/// and Markdown documents', and its canvases' in `canvases`.
fn name_node_pictures(n: &Node, canvases: &BTreeMap<NodeId, WebCanvas>, pictures: &mut Pictures) {
    let mut stack = vec![n];
    while let Some(n) = stack.pop() {
        stack.extend(n.children.iter());
        for (bytes, format) in pictures_in(n) {
            pictures.name(bytes, format);
        }
        if let Some(c) = canvases.get(&n.id) {
            name_pictures(&c.board, &c.board.all(), pictures);
        }
    }
}

/// The board and animations of the canvas `node`, whose items a change
/// changes; why it has none: no such node, a node of another kind, or
/// items that cannot be read.
fn canvas_items<'a>(tree: &Node, canvases: &'a mut BTreeMap<NodeId, WebCanvas>, node: NodeId) -> Result<(&'a mut proto::canvas::Board, &'a mut proto::canvas::Motions), String> {
    let n = tree.find(node).ok_or_else(|| format!("no node {node}"))?;
    if n.kind != Kind::Canvas {
        return Err(format!("node {node} is a {:?}, which has no canvas items", n.kind));
    }
    let WebCanvas { board, unread, motions, .. } = canvases.get_mut(&node).ok_or_else(|| format!("node {node}: the window has no items for the canvas"))?;
    match unread {
        Some(why) => Err(format!("node {node}'s items cannot be read, so they take no change: {why}")),
        None => Ok((board, motions)),
    }
}

/// The boards the Canvases of `n`'s subtree give, by node, or why the
/// first whose items cannot be read cannot be.
fn boards_in(n: &Node) -> Result<Vec<(NodeId, proto::canvas::Board)>, String> {
    let mut out = Vec::new();
    let mut stack = vec![n];
    while let Some(n) = stack.pop() {
        stack.extend(n.children.iter());
        if n.kind == Kind::Canvas {
            out.push((n.id, proto::canvas::Board::of(n).map_err(|e| format!("node {}: {e}", n.id))?));
        }
    }
    Ok(out)
}

/// Gives each canvas of `boards` its board in place of the one it had,
/// their animations starting at `now`, each with the view it had.
fn take_boards(canvases: &mut BTreeMap<NodeId, WebCanvas>, boards: Vec<(NodeId, proto::canvas::Board)>, now: Instant) {
    for (node, board) in boards {
        let view = canvases.remove(&node).and_then(|c| c.view);
        canvases.insert(node, WebCanvas::of(board, view, now));
    }
}

/// The canvases of the window `tree` opens, each with its board, its
/// animations starting at `now`, or why its items cannot be read, which is
/// also among the problems returned, each naming its node.
fn open_canvases(tree: &Node, now: Instant) -> (BTreeMap<NodeId, WebCanvas>, Vec<String>) {
    let mut out = BTreeMap::new();
    let mut problems = Vec::new();
    let mut stack = vec![tree];
    while let Some(n) = stack.pop() {
        stack.extend(n.children.iter());
        if n.kind == Kind::Canvas {
            let canvas = match proto::canvas::Board::of(n) {
                Ok(board) => WebCanvas::of(board, None, now),
                Err(why) => {
                    let text = format!("Canvas {}: its items cannot be read: {why}", n.name.as_deref().unwrap_or(""));
                    problems.push(text.clone());
                    WebCanvas::unread(text)
                }
            };
            out.insert(n.id, canvas);
        }
    }
    (out, problems)
}

/// Names in `pictures` the pictures the items `ids` of `board` show, so a
/// change shows them before the window's pictures are next taken.
fn name_pictures(board: &proto::canvas::Board, ids: &[proto::canvas::ItemId], pictures: &mut Pictures) {
    for id in ids {
        if let Some(proto::canvas::Shape::Image(_, picture)) = board.entry(*id).map(|e| &e.item.shape) {
            pictures.name(&picture.bytes, &picture.format);
        }
    }
}

fn remove(tree: &mut Node, id: NodeId) -> bool {
    if let Some(at) = tree.children.iter().position(|c| c.id == id) {
        tree.children.remove(at);
        return true;
    }
    tree.children.iter_mut().any(|c| remove(c, id))
}

/// A protocol value as JSON for a page: bytes travel as a list of numbers.
fn value_json(v: &Value) -> Json {
    match v {
        Value::Null => Json::Null,
        Value::Bool(b) => Json::Bool(*b),
        Value::Int(i) => Json::Number(Number::from(*i)),
        Value::Float(f) => Number::from_f64(*f).map_or(Json::Null, Json::Number),
        Value::Str(s) => Json::String(s.clone()),
        Value::List(items) => Json::Array(items.iter().map(value_json).collect()),
        Value::Map(m) => Json::Object(m.iter().map(|(k, v)| (k.clone(), value_json(v))).collect::<Map<String, Json>>()),
        Value::Bytes(b) => Json::Array(b.iter().map(|x| Json::Number(Number::from(*x))).collect()),
    }
}

/// A page's JSON as a protocol value: an integral number is an `Int`.
fn value_from(j: &Json) -> Value {
    match j {
        Json::Null => Value::Null,
        Json::Bool(b) => Value::Bool(*b),
        Json::Number(n) => match (n.as_i64(), n.as_f64()) {
            (Some(i), _) => Value::Int(i),
            (None, Some(f)) => Value::Float(f),
            (None, None) => Value::Null,
        },
        Json::String(s) => Value::Str(s.clone()),
        Json::Array(items) => Value::List(items.iter().map(value_from).collect()),
        Json::Object(m) => Value::Map(m.iter().map(|(k, v)| (k.clone(), value_from(v))).collect()),
    }
}

fn node_json(n: &Node, showing: &Showing) -> Json {
    let mut props = props_json(&n.props, showing, (n.kind == Kind::Canvas).then_some(n.id));
    if n.kind == Kind::Chart
        && let Json::Object(m) = &mut props
    {
        m.insert(String::from("chart"), chart_json(n));
    }
    json!({
        "id": n.id,
        "kind": format!("{:?}", n.kind),
        "name": n.name,
        "props": props,
        "children": Json::Array(n.children.iter().map(|c| node_json(c, showing)).collect()),
    })
}

/// A Chart's data as a page draws it, read as [`proto::chart::Chart`]
/// reads it: whether its X are dates, and each series' name, how it is
/// drawn and its points, each its X and Y where it is drawn and its X as
/// the session wrote it, which a click on it reports. A chart that cannot
/// be read is the problem the page draws in its place.
fn chart_json(n: &Node) -> Json {
    match proto::chart::Chart::of(n) {
        Ok(c) => json!({
            "dates": c.dates,
            "series": c.series.iter().map(|s| json!({
                "name": s.name,
                "kind": match s.kind {
                    proto::chart::SeriesKind::Line => "line",
                    proto::chart::SeriesKind::Bar => "bar",
                    proto::chart::SeriesKind::Points => "points",
                },
                "points": s.points.iter().map(|p| json!([p.x, p.y, value_json(&p.written_x)])).collect::<Vec<Json>>(),
            })).collect::<Vec<Json>>(),
        }),
        Err(why) => json!({ "problem": why }),
    }
}

/// A node's properties as JSON for a page, with an Image's picture as the
/// address the page loads it from in place of its bytes, a Markdown
/// widget's document as the HTML it shows, and for `canvas`, a Canvas's
/// node, its items as the scene the page draws ([`scene_json`]).
fn props_json(props: &BTreeMap<String, Value>, showing: &Showing, canvas: Option<NodeId>) -> Json {
    let mut out: Map<String, Json> = props
        .iter()
        .filter(|(k, _)| !(canvas.is_some() && k.as_str() == "items"))
        .map(|(k, v)| {
            let shown = match (k.as_str(), v) {
                ("image", Value::Map(m)) => picture_json(m, showing.pictures),
                ("document", Value::Map(m)) => document_json(m, showing.pictures),
                _ => value_json(v),
            };
            (k.clone(), shown)
        })
        .collect();
    if let Some(node) = canvas {
        out.insert(String::from("scene"), scene_json(node, showing));
    }
    Json::Object(out)
}

/// A Canvas's items as its page draws them, in drawing order, each with
/// its own items and the animations it has as they stand at `showing`'s
/// moment ([`item_json`]); or the problem its items are.
fn scene_json(node: NodeId, showing: &Showing) -> Json {
    match showing.canvases.get(&node) {
        Some(WebCanvas { unread: Some(why), .. }) => json!({ "problem": why }),
        Some(WebCanvas { board, motions, .. }) => json!({
            "items": board.top().iter().map(|id| item_json(board, *id, showing.pictures, true, Some((motions, showing.now)))).collect::<Vec<Json>>(),
        }),
        None => json!({ "problem": format!("the window has no items for canvas {node}") }),
    }
}

/// Item `id` of `board` as a page draws it: its id, which lasts while it is
/// on the board, its name, its shape ([`shape_json`]), its style
/// ([`style_json`]) and the animations it gives; with `deep`, a Group's
/// items in drawing order, each the same; and with `motions`, its
/// animations under way or stopped at their ends at that moment, each with
/// the seconds since it began and the value it began from, null where the
/// theme sets it.
fn item_json(board: &proto::canvas::Board, id: proto::canvas::ItemId, pictures: &Pictures, deep: bool, motions: Option<(&proto::canvas::Motions, Instant)>) -> Json {
    let Some(entry) = board.entry(id) else {
        return Json::Null;
    };
    let item = &entry.item;
    let mut out = Map::new();
    out.insert(String::from("id"), json!(id));
    out.insert(String::from("name"), json!(item.name));
    out.insert(String::from("shape"), shape_json(&item.shape, pictures));
    out.insert(String::from("style"), style_json(&item.style));
    out.insert(String::from("animations"), Json::Array(item.animations.iter().map(animation_json).collect()));
    if deep && item.kind() == proto::canvas::ItemKind::Group {
        out.insert(String::from("children"), Json::Array(entry.children.iter().map(|c| item_json(board, *c, pictures, true, motions)).collect()));
    }
    if let Some((all, now)) = motions {
        let running = all.of(id).into_iter().flatten().map(|(_, m)| {
            json!({ "animation": animation_json(&m.animation), "elapsed": now.saturating_duration_since(m.began).as_secs_f64(), "from": m.from.map(target_json) })
        });
        out.insert(String::from("motions"), Json::Array(running.collect()));
    }
    Json::Object(out)
}

/// A shape as a page draws it, by its kind's name: a box's corner and
/// size, a rounded box's radius besides, a line's ends, points as one list
/// of numbers, x then y, a path's segments ([`segments_json`]), a text's
/// top left, text and size, null for the default, a picture's box and the
/// address a page loads it from, and a group's offset and the shape it
/// clips to.
fn shape_json(shape: &proto::canvas::Shape, pictures: &Pictures) -> Json {
    use proto::canvas::Shape;
    let boxed = |kind: &str, b: &proto::canvas::Bounds| json!({ "kind": kind, "x": b.x, "y": b.y, "width": b.width, "height": b.height });
    match shape {
        Shape::Rect(b) => boxed("Rect", b),
        Shape::RoundedRect(b, radius) => json!({ "kind": "RoundedRect", "x": b.x, "y": b.y, "width": b.width, "height": b.height, "radius": radius }),
        Shape::Ellipse(b) => boxed("Ellipse", b),
        Shape::Line(a, b) => json!({ "kind": "Line", "x1": a[0], "y1": a[1], "x2": b[0], "y2": b[1] }),
        Shape::Polyline(points) => json!({ "kind": "Polyline", "points": points.iter().flatten().collect::<Vec<_>>() }),
        Shape::Polygon(points) => json!({ "kind": "Polygon", "points": points.iter().flatten().collect::<Vec<_>>() }),
        Shape::Path(segments) => json!({ "kind": "Path", "segments": segments_json(segments) }),
        Shape::Text { at, text, size } => json!({ "kind": "Text", "x": at[0], "y": at[1], "text": text, "size": size }),
        Shape::Image(b, picture) => json!({ "kind": "Image", "x": b.x, "y": b.y, "width": b.width, "height": b.height, "src": pictures.src(&picture.bytes), "format": picture.format }),
        Shape::Group { at, clip } => json!({ "kind": "Group", "x": at[0], "y": at[1], "clip": clip.as_deref().map(|c| shape_json(c, pictures)) }),
    }
}

/// Path segments as a page draws them, each its SVG command letter and its
/// numbers in absolute coordinates, an arc's flags as true or false.
fn segments_json(segments: &[proto::svg_path::Segment]) -> Json {
    use proto::svg_path::Segment;
    Json::Array(
        segments
            .iter()
            .map(|s| match s {
                Segment::Move(p) => json!(["M", p[0], p[1]]),
                Segment::Line(p) => json!(["L", p[0], p[1]]),
                Segment::Quad(c, p) => json!(["Q", c[0], c[1], p[0], p[1]]),
                Segment::Cubic(a, b, p) => json!(["C", a[0], a[1], b[0], b[1], p[0], p[1]]),
                Segment::Arc { radii, rotation, large, sweep, to } => json!(["A", radii[0], radii[1], rotation, large, sweep, to[0], to[1]]),
                Segment::Close => json!(["Z"]),
            })
            .collect(),
    )
}

/// An item's style as a page draws it: each key it leaves to its group
/// null, colors as `#rrggbbaa`, a gradient as its colors under `linear` or
/// `radial` with its stops and points in fractions of the item's box, and
/// a cap's and a join's names.
fn style_json(s: &proto::canvas::Style) -> Json {
    use proto::canvas::{Cap, Join, Paint};
    let paint = |p: &Paint| match p {
        Paint::Color(c) => json!({ "color": c.hex() }),
        Paint::Linear { colors, stops, from, to } => json!({ "linear": colors.iter().map(|c| c.hex()).collect::<Vec<_>>(), "stops": stops, "from": from, "to": to }),
        Paint::Radial { colors, stops, center, radius } => json!({ "radial": colors.iter().map(|c| c.hex()).collect::<Vec<_>>(), "stops": stops, "center": center, "radius": radius }),
    };
    json!({
        "fill": s.fill.as_ref().map(paint),
        "stroke": s.stroke.map(|c| c.hex()),
        "strokeWidth": s.stroke_width,
        "dash": s.dash,
        "cap": s.cap.map(|c| match c { Cap::Butt => "Butt", Cap::Round => "Round", Cap::Square => "Square" }),
        "join": s.join.map(|j| match j { Join::Miter => "Miter", Join::Round => "Round", Join::Bevel => "Bevel" }),
        "opacity": s.opacity,
        "rotate": s.rotate,
        "scale": s.scale,
        "skew": s.skew,
        "origin": s.origin,
        "visible": s.visible,
        "order": s.order,
    })
}

/// An animation as a page runs it: the property it moves, by its lowercase
/// name, where to, in how many seconds, along which curve, its passes, a
/// count or "Forever", and whether each runs back.
fn animation_json(a: &proto::canvas::Animation) -> Json {
    let repeat = match a.repeat {
        proto::canvas::Repeat::Count(n) => json!(n),
        proto::canvas::Repeat::Forever => Json::from("Forever"),
    };
    json!({ "property": a.property, "to": target_json(a.to), "duration": a.duration, "easing": a.easing.name(), "repeat": repeat, "reverse": a.reverse })
}

/// Where an animation moves a property: a number, a pair, or a color as
/// `#rrggbbaa`.
fn target_json(t: proto::canvas::Target) -> Json {
    match t {
        proto::canvas::Target::Number(n) => json!(n),
        proto::canvas::Target::Pair(p) => json!(p),
        proto::canvas::Target::Color(c) => Json::from(c.hex()),
    }
}

/// A Markdown widget's document as a page shows it: its HTML, whose images
/// load the pictures the window has for them, and the addresses of its
/// links, which a test may click. A document whose text or pictures the
/// window cannot take is the problem the page draws in its place.
fn document_json(m: &BTreeMap<String, Value>, pictures: &Pictures) -> Json {
    let (Some(Value::Str(text)), Some(Value::Map(read))) = (m.get("text"), m.get("pictures")) else {
        return json!({ "problem": "document must have its text and the pictures the session read for its images" });
    };
    for (address, picture) in read {
        if picture_parts(picture).and_then(|(bytes, _)| pictures.src(bytes)).is_none() {
            return json!({ "problem": format!("the picture the session read for the image {address} is not one the window has") });
        }
    }
    let served = |address: &str| read.get(address).and_then(picture_parts).and_then(|(bytes, _)| pictures.src(bytes));
    json!({ "html": markdown::html(text, &served), "links": proto::markdown::link_addresses(text) })
}

/// An Image's picture as a page loads it: the address its bytes load from
/// and their format. Bytes the window has no picture of have no address,
/// which the page draws as the problem it is.
fn picture_json(m: &BTreeMap<String, Value>, pictures: &Pictures) -> Json {
    let src = match m.get("bytes") {
        Some(Value::Bytes(b)) => pictures.src(b),
        Some(_) | None => None,
    };
    json!({ "src": src, "format": m.get("format").map(value_json) })
}

fn sort_json(s: &SortKey) -> Json {
    match s {
        SortKey::Number(n) | SortKey::Date(n) => Number::from_f64(*n).map_or(Json::Null, Json::Number),
        SortKey::Text(t) => Json::String(t.clone()),
        SortKey::Empty => Json::Null,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_pixel_snapshot_waiting_the_deadline_is_late_and_the_loop_wakes_for_the_next() {
        let now = Instant::now();
        let ago = |d: Duration| now.checked_sub(d).expect("the clock has run longer than twice the deadline");
        let shot = |pixels: bool, asked: Instant| (0, Shot { reply: String::from("ring"), pixels, asked });
        let capture = |asked: Instant| Capture { reply: String::from("ring"), state: BTreeMap::new(), bounds: BTreeMap::new(), engine: None, focused: None, peeks: BTreeMap::new(), copied: None, sound: BTreeMap::new(), rect: None, viewport: (0.0, 0.0), asked };
        let shots = HashMap::from([(1, shot(true, ago(CAPTURE_DEADLINE))), (2, shot(false, ago(CAPTURE_DEADLINE * 2))), (3, shot(true, ago(Duration::from_secs(10))))]);
        let captures = HashMap::from([(7, capture(ago(CAPTURE_DEADLINE + Duration::from_millis(1)))), (8, capture(ago(Duration::from_secs(20))))]);
        let (asks, requests, next) = late(&shots, &captures, now);
        assert_eq!(asks, vec![1], "a pixel snapshot the page has not replied to for the deadline is late; one without pixels has no deadline");
        assert_eq!(requests, vec![7], "a capture the engine has not finished for the deadline is late");
        assert_eq!(next, Some(ago(Duration::from_secs(20)) + CAPTURE_DEADLINE), "the loop wakes when the oldest still waiting reaches the deadline");
        assert_eq!(late(&HashMap::new(), &HashMap::new(), now), (Vec::new(), Vec::new(), None), "with nothing waiting the loop has no deadline");
    }

    #[test]
    fn a_snapshot_answer_s_focused_node_is_read_as_the_widget_around_it_that_takes_the_focus() {
        let tree = Node::new(1, Kind::Column).child(Node::new(2, Kind::Table)).child(Node::new(3, Kind::Text));
        assert_eq!(page_focused(&tree, &json!({ "focused": 2 })), Ok(Some(2)));
        assert_eq!(page_focused(&tree, &json!({ "focused": null })), Ok(None));
        assert_eq!(page_focused(&tree, &json!({ "focused": 3 })), Ok(None), "a Text that is not a link takes no focus");
        assert!(page_focused(&tree, &json!({})).is_err(), "a reply without the key is from a page that cannot be read");
        assert!(page_focused(&tree, &json!({ "focused": "two" })).is_err());
    }

    #[test]
    fn a_snapshot_answer_s_text_last_copied_is_text_or_null() {
        let copied = page_copied(&json!({ "copied": "name1\t10\t\n" })).expect("a copy's text reads");
        assert_eq!(copied.as_deref(), Some("name1\t10\t\n"));
        let none = page_copied(&json!({ "copied": null })).expect("a null copy reads");
        assert!(none.is_none(), "a window that has copied nothing gives null");
        assert!(page_copied(&json!({})).is_err(), "a reply without the key is from a page that cannot be read");
        assert!(page_copied(&json!({ "copied": 3 })).is_err(), "a copy that is not text is refused");
    }

    #[test]
    fn a_snapshot_answer_s_peek_buttons_are_read_for_the_window_s_password_fields_alone() {
        let password = Node::new(2, Kind::TextBox).prop("text", Value::Str(String::new())).prop("password", Value::Bool(true));
        let plain = Node::new(3, Kind::TextBox).prop("text", Value::Str(String::new()));
        let tree = Node::new(1, Kind::Column).child(password).child(plain);
        assert_eq!(page_peeks(&tree, &json!({ "peeks": { "2": true } })), Ok(BTreeMap::from([(2, true)])));
        assert_eq!(page_peeks(&tree, &json!({ "peeks": {} })), Ok(BTreeMap::new()));
        assert!(page_peeks(&tree, &json!({})).is_err(), "a reply without the key is from a page that cannot be read");
        assert!(page_peeks(&tree, &json!({ "peeks": { "3": false } })).is_err(), "a field that is no password field has no peek button");
        assert!(page_peeks(&tree, &json!({ "peeks": { "9": false } })).is_err(), "a node the window lacks has none");
        assert!(page_peeks(&tree, &json!({ "peeks": { "2": "yes" } })).is_err());
    }

    #[test]
    fn values_cross_to_json_and_back() {
        let v = Value::Map(BTreeMap::from([
            (String::from("n"), Value::Int(-3)),
            (String::from("f"), Value::Float(1.5)),
            (String::from("l"), Value::List(vec![Value::Str("a".into()), Value::Null, Value::Bool(true)])),
        ]));
        assert_eq!(value_from(&value_json(&v)), v);
    }

    #[cfg(windows)]
    fn refusal(code: windows::core::HRESULT) -> wry::Error {
        wry::Error::WebView2Error(webview2_com::Error::WindowsError(windows::core::Error::from(code)))
    }

    #[cfg(windows)]
    #[test]
    fn a_webview_refused_on_a_folder_in_use_names_the_fix() {
        use windows::Win32::Foundation::{E_FAIL, ERROR_BUSY, ERROR_INVALID_STATE};
        let folder = std::path::Path::new(r"C:\data");
        for (code, cause) in [(ERROR_BUSY.to_hresult(), "another sign-in of this user"), (ERROR_INVALID_STATE.to_hresult(), "other options")] {
            let text = webview_error(&refusal(code), Some(folder));
            assert!(
                text.contains(r"C:\data") && text.contains(cause) && text.contains("-WebView PerProcess") && text.contains("-WebViewDataFolder"),
                "{text}"
            );
        }
        let other = webview_error(&refusal(E_FAIL), Some(folder));
        assert!(other.starts_with("the webview cannot be created") && !other.contains("PerProcess"), "{other}");
    }

    #[test]
    fn a_patch_changes_the_kept_tree_and_refuses_a_missing_node() {
        let mut tree = Node::new(1, Kind::Column).child(Node::new(2, Kind::Text).named("T").prop("text", Value::Str("a".into())));
        let (mut canvases, mut pictures, now) = (BTreeMap::new(), Pictures::default(), Instant::now());
        let mut patched = |tree: &mut Node, op: PatchOp| apply(tree, &mut canvases, &mut pictures, &op, now);
        let set = patched(&mut tree, PatchOp::SetProps { node: 2, props: BTreeMap::from([(String::from("text"), Value::Str("b".into()))]) }).expect("set");
        assert_eq!(set, json!({ "op": "set", "node": 2, "props": { "text": "b" } }), "the page gets the change as made");
        assert_eq!(tree.find(2).and_then(|n| n.props.get("text")), Some(&Value::Str("b".into())));
        patched(&mut tree, PatchOp::Insert { parent: 1, index: 9, node: Node::new(3, Kind::Separator) }).expect("insert at the end");
        assert_eq!(tree.children.iter().map(|c| c.id).collect::<Vec<_>>(), vec![2, 3]);
        patched(&mut tree, PatchOp::Remove { node: 2 }).expect("remove");
        assert!(patched(&mut tree, PatchOp::Remove { node: 2 }).is_err());
        assert!(patched(&mut tree, PatchOp::SetProps { node: 7, props: BTreeMap::new() }).is_err());
    }

    fn rect(name: &str, x: f64) -> Value {
        let number = |n: f64| Value::Float(n);
        Value::Map(BTreeMap::from([
            (String::from("kind"), Value::Str(String::from("Rect"))),
            (String::from("name"), Value::Str(name.to_string())),
            (String::from("x"), number(x)),
            (String::from("y"), number(0.0)),
            (String::from("width"), number(10.0)),
            (String::from("height"), number(10.0)),
        ]))
    }

    #[test]
    fn a_canvas_s_items_change_on_its_board_and_the_page_gets_each_change_as_the_item_then_stands() {
        let mut tree = Node::new(1, Kind::Column).child(Node::new(2, Kind::Canvas).named("Board").prop("items", Value::List(vec![rect("A", 0.0)])));
        let now = Instant::now();
        let (mut canvases, unread) = open_canvases(&tree, now);
        assert!(unread.is_empty(), "{unread:?}");
        let mut pictures = Pictures::default();
        let mut patched = |tree: &mut Node, canvases: &mut BTreeMap<NodeId, WebCanvas>, op: PatchOp| apply(tree, canvases, &mut pictures, &op, now);
        let moved = PatchOp::SetItem { node: 2, item: String::from("a"), props: BTreeMap::from([(String::from("x"), Value::Float(30.0))]) };
        let written = patched(&mut tree, &mut canvases, moved).expect("the item changes");
        assert_eq!((&written["op"], &written["set"], &written["animate"]), (&json!("setItem"), &json!(["x"]), &Json::Null));
        assert_eq!(written["item"]["shape"], json!({ "kind": "Rect", "x": 30.0, "y": 0.0, "width": 10.0, "height": 10.0 }), "the item as it then stands");
        assert_eq!(written["item"]["id"], json!(0));
        assert_eq!(tree.find(2).and_then(|n| n.props.get("items")), Some(&Value::List(vec![rect("A", 0.0)])), "the node still has the items it was given");
        let mut shown = tree.clone();
        current_items(&mut shown, &canvases);
        assert_eq!(shown.find(2).and_then(|n| n.props.get("items")), Some(&Value::List(vec![rect("A", 30.0)])), "a snapshot shows the board's");
        let added = patched(&mut tree, &mut canvases, PatchOp::AddItems { node: 2, group: None, items: vec![rect("B", 50.0)] }).expect("an item is added");
        assert_eq!((&added["group"], &added["items"][0]["name"], &added["items"][0]["id"]), (&Json::Null, &json!("B"), &json!(1)));
        let removed = patched(&mut tree, &mut canvases, PatchOp::RemoveItems { node: 2, names: vec![String::from("A")] }).expect("an item is taken away");
        assert_eq!(removed["ids"], json!([0]));
        let elsewhere = PatchOp::SetItem { node: 1, item: String::from("B"), props: BTreeMap::new() };
        assert!(patched(&mut tree, &mut canvases, elsewhere).is_err_and(|e| e.contains("has no canvas items")));
        let nobody = PatchOp::SetItem { node: 2, item: String::from("Nobody"), props: BTreeMap::new() };
        assert!(patched(&mut tree, &mut canvases, nobody).is_err_and(|e| e.contains("no item is named Nobody")));
        let unreadable = Node::new(2, Kind::Canvas).named("Board").prop("items", Value::Str(String::from("no item")));
        assert!(patched(&mut tree, &mut canvases, PatchOp::Replace { node: unreadable }).is_err(), "a canvas whose items cannot be read is refused");
        assert!(canvases.get(&2).is_some_and(|c| c.board.find("B").is_some()), "and nothing changed");
    }

    #[test]
    fn a_canvas_is_shown_to_its_page_as_the_scene_it_draws_with_its_animations_as_they_stand() {
        let map = |pairs: &[(&str, Value)]| Value::Map(pairs.iter().map(|(k, v)| (k.to_string(), v.clone())).collect());
        let (text, number) = (|s: &str| Value::Str(s.to_string()), Value::Float);
        let slide = map(&[("property", text("X")), ("to", number(100.0)), ("duration", number(2.0)), ("easing", text("Linear"))]);
        let Value::Map(mut sliding) = rect("Slider", 0.0) else { panic!("a map") };
        sliding.insert(String::from("animate"), slide);
        let gradient = map(&[("linear", Value::List(vec![text("Tomato"), text("Gold")]))]);
        let path = map(&[("kind", text("Path")), ("data", text("M0 0 L10 0")), ("fill", gradient), ("stroke", text("#000")), ("dash", Value::List(vec![number(2.0), number(1.0)]))]);
        let clip = map(&[("kind", text("Ellipse")), ("x", number(0.0)), ("y", number(0.0)), ("width", number(20.0)), ("height", number(20.0))]);
        let group = map(&[("kind", text("Group")), ("name", text("G")), ("x", number(5.0)), ("clip", clip), ("children", path)]);
        let tree = Node::new(1, Kind::Column).child(Node::new(2, Kind::Canvas).prop("items", Value::List(vec![Value::Map(sliding), group])));
        let now = Instant::now();
        let (canvases, _) = open_canvases(&tree, now);
        let pictures = Pictures::default();
        let json = node_json(&tree, &Showing { pictures: &pictures, canvases: &canvases, now: now + Duration::from_millis(500) });
        let props = &json["children"][0]["props"];
        assert!(props.get("items").is_none(), "the items' maps stay with the helper");
        let items = &props["scene"]["items"];
        let motion = &items[0]["motions"][0];
        assert_eq!((&motion["animation"]["property"], &motion["from"], &motion["elapsed"]), (&json!("x"), &json!(0.0), &json!(0.5)), "under way for half a second, from where it began");
        assert_eq!((&items[1]["shape"]["kind"], &items[1]["shape"]["clip"]["kind"]), (&json!("Group"), &json!("Ellipse")));
        let child = &items[1]["children"][0];
        assert_eq!(child["shape"]["segments"], json!([["M", 0.0, 0.0], ["L", 10.0, 0.0]]));
        assert_eq!(child["style"]["fill"], json!({ "linear": ["#ff6347ff", "#ffd700ff"], "stops": [0.0, 1.0], "from": [0.0, 0.0], "to": [1.0, 0.0] }));
        assert_eq!((&child["style"]["stroke"], &child["style"]["dash"], &child["style"]["cap"]), (&json!("#000000ff"), &json!([2.0, 1.0]), &Json::Null));
    }

    #[test]
    fn a_canvas_view_is_written_as_a_page_makes_it_and_read_back_under_the_session_s_controls() {
        use proto::view::{ViewChange, ViewControls, ViewEase, ViewFit, ViewShown, ViewSpec};
        let change = ViewChange { zoom: Some(2.0), controls: Some(ViewControls::Map), fit: Some(ViewFit::Items(vec![String::from("A")])), ease: Some(ViewEase { seconds: 0.5, easing: proto::canvas::Easing::EaseOut }), ..ViewChange::default() };
        let written = change_json(&change, &ViewControls::Map);
        assert!(written.get("center").is_none() && written.get("minZoom").is_none(), "a part the change leaves out is absent: {written}");
        assert_eq!((&written["zoom"], &written["bindings"]["Left"], &written["bindings"]["Shift+Left"]), (&json!(2.0), &json!("Glide"), &json!("BoxZoom")));
        assert_eq!((&written["fit"], &written["ease"]), (&json!(["A"]), &json!({ "seconds": 0.5, "easing": "EaseOut" })));
        let view = view_json(&ViewSpec::default());
        assert_eq!((&view["center"], &view["maxZoom"]), (&Json::Null, &json!("Infinity")), "no farthest crosses as Infinity: {view}");
        let reported = json!({ "view": { "center": [1, 2], "zoom": 3, "minZoom": 0.5, "maxZoom": "Infinity" }, "moving": true });
        let spec = ViewSpec { center: Some([1.0, 2.0]), zoom: 3.0, min_zoom: 0.5, max_zoom: f64::INFINITY, controls: ViewControls::Design };
        let shown = page_view_shown(&reported, &ViewControls::Design).expect("a reported view reads");
        assert_eq!(shown, Some(ViewShown { view: spec, moving: true }), "under the session's controls");
        let unset = page_view_shown(&json!({ "view": null, "moving": false }), &ViewControls::Map).expect("a canvas without a view reads");
        assert_eq!(unset, None);
        let short = json!({ "view": { "center": [1], "zoom": 1, "minZoom": 1, "maxZoom": 1 }, "moving": false });
        assert!(page_view_shown(&short, &ViewControls::Map).is_err_and(|e| e.contains("two numbers")));
        let numbers = view_numbers();
        assert_eq!((&numbers["zoomStep"], &numbers["stepEasing"], &numbers["boxBorder"]), (&json!(1.2), &json!("EaseOut"), &json!("#3388ffff")));
    }

    #[test]
    fn a_canvas_raises_the_keys_its_view_leaves_and_sends_its_events_numbers_as_floats() {
        let tree = Node::new(1, Kind::Column).child(Node::new(2, Kind::Canvas).named("Board").prop("keys", Value::Bool(true)));
        let (mut canvases, _) = open_canvases(&tree, Instant::now());
        if let Some(c) = canvases.get_mut(&2) {
            c.view = Some(proto::view::ViewShown { view: proto::view::ViewSpec::default(), moving: false });
        }
        let key = |written: &str| helper_test(&tree, &canvases, "Board", EventKind::Key, &Value::Str(written.to_string()));
        assert!(key("ArrowLeft").is_err_and(|e| e == "the canvas's view takes ArrowLeft itself, so it raises no Key"), "Map pans with the arrows");
        match key("Delete").expect("a key the view leaves is raised") {
            HelperTest::Raise(node, Value::Str(written)) => assert_eq!((node, written.as_str()), (2, "Delete")),
            _ => panic!("Delete is raised on the canvas by the renderer"),
        }
        let pressed = page_key(&tree, &canvases, &json!({ "t": "key", "node": 2, "keys": "Plus", "typed": false })).expect("the press reads");
        assert_eq!(pressed, None, "a press the view takes raises nothing");
        let given = value_from(&json!({ "x": 4, "y": 2, "u": 0, "v": 1, "ppp": 1, "scenex": 10, "sceney": -3, "item": "A" }));
        let Value::Map(sent) = as_sent(&tree, 2, given) else {
            panic!("a map");
        };
        for key in ["x", "y", "u", "v", "ppp", "scenex", "sceney"] {
            assert!(matches!(sent.get(key), Some(Value::Float(_))), "{key} is a Float, as egui sends it: {:?}", sent.get(key));
        }
        let (_, kind, _) = page_event(&json!({ "t": "event", "node": 2, "kind": "DragMove", "value": null })).expect("a drag's move reads");
        assert_eq!(kind, EventKind::DragMove);
    }

    #[test]
    fn a_page_event_names_its_node_kind_and_value() {
        let (node, kind, value) = page_event(&json!({ "t": "event", "node": 3, "kind": "Select", "value": [2] })).expect("an event");
        assert_eq!((node, kind, value), (3, EventKind::Select, Value::List(vec![Value::Int(2)])));
        let expanded = page_event(&json!({ "t": "event", "node": 3, "kind": "Expand", "value": 4 })).expect("a tree's Expand");
        assert_eq!(expanded, (3, EventKind::Expand, Value::Int(4)));
        assert!(page_event(&json!({ "t": "event", "node": 3, "kind": "Hover" })).is_err());
        assert!(page_event(&json!({ "t": "event", "kind": "Click" })).is_err());
    }

    #[test]
    fn a_number_widgets_whole_number_is_sent_as_a_float() {
        let tree = Node::new(1, Kind::Column).child(Node::new(2, Kind::Slider)).child(Node::new(3, Kind::NumberBox)).child(Node::new(4, Kind::ComboBox));
        assert_eq!(as_sent(&tree, 2, Value::Int(5)), Value::Float(5.0));
        assert_eq!(as_sent(&tree, 3, Value::Float(2.5)), Value::Float(2.5));
        assert_eq!(as_sent(&tree, 4, Value::Int(1)), Value::Int(1));
    }

    #[test]
    fn the_runtime_is_served_from_memory_and_nothing_else() {
        let assets = look::Assets { css: String::from(":root { --body: 13px; }"), ui_font: None };
        assert_eq!(serve("/index.html", &assets).status(), 200);
        assert_eq!(serve("/runtime.js", &assets).status(), 200);
        let canvas = serve("/canvas.js", &assets);
        assert_eq!((canvas.status(), canvas.body().as_ref()), (StatusCode::OK, CANVAS_JS.as_bytes()), "the page's canvas arithmetic");
        let layout = serve("/layout.js", &assets);
        assert_eq!((layout.status(), layout.body().as_ref()), (StatusCode::OK, LAYOUT_JS.as_bytes()), "the page's layout rules");
        assert!(INDEX.contains("src=\"layout.js\""), "the page loads its layout rules");
        let tokens = serve("/tokens.css", &assets);
        assert_eq!((tokens.status(), tokens.body().as_ref()), (StatusCode::OK, assets.css.as_bytes()));
        let font = serve("/fonts/Ubuntu-Light.ttf", &assets);
        assert_eq!((font.status(), font.body().len()), (StatusCode::OK, epaint_default_fonts::UBUNTU_LIGHT.len()));
        assert_eq!(font.headers()[CONTENT_TYPE].to_str().expect("the media type is text"), "font/ttf");
        let inter = serve("/fonts/Inter.ttf", &assets);
        assert_eq!((inter.status(), inter.body().len()), (StatusCode::OK, pwrs_gui_fonts::INTER.len()));
        let mono = serve("/fonts/JetBrainsMonoNL.ttf", &assets);
        assert_eq!((mono.status(), mono.body().len()), (StatusCode::OK, pwrs_gui_fonts::JETBRAINS_MONO.len()));
        assert_eq!(serve(look::SYSTEM_UI_PATH, &assets).status(), 404, "no system font is given where none was stored");
        let with_ui = look::Assets { css: String::new(), ui_font: Some(&b"ttf system ui"[..]) };
        let ui = serve(look::SYSTEM_UI_PATH, &with_ui);
        assert_eq!((ui.status(), ui.body().as_ref()), (StatusCode::OK, &b"ttf system ui"[..]));
        assert_eq!(ui.headers()[CONTENT_TYPE].to_str().expect("the media type is text"), "font/ttf");
        assert_eq!(serve("/elsewhere.js", &assets).status(), 404);
        assert!(INDEX.contains("href=\"tokens.css\""), "the page loads the tokens");
    }

    #[test]
    fn no_function_of_the_runtime_is_declared_twice() {
        // Each script's functions sit at one indent inside its wrapping
        // function, where a second declaration of a name replaces the first
        // for every caller.
        for (script, text) in [("runtime.js", RUNTIME_JS), ("canvas.js", CANVAS_JS)] {
            let mut seen = BTreeMap::new();
            for (number, line) in text.lines().enumerate() {
                let Some(rest) = line.strip_prefix("  function ") else { continue };
                let name: String = rest.chars().take_while(|c| c.is_alphanumeric() || *c == '_' || *c == '$').collect();
                if let Some(first) = seen.insert(name.clone(), number + 1) {
                    panic!("{script} declares {name} at lines {first} and {}", number + 1);
                }
            }
            assert!(seen.len() > 10, "{script}'s functions were found: {}", seen.len());
        }
    }

    #[test]
    fn a_pwrs_request_is_answered_from_what_the_asking_webviews_window_serves() {
        let served = ArcSwap::from_pointee(HashMap::from([
            (String::from("1"), Served::Runtime),
            (String::from("2"), Served::Page { source: PageSource::Text(Arc::from("<p>a page of its own</p>")), allow_navigation: false }),
        ]));
        let pictures = ArcSwap::from_pointee(HashMap::new());
        let assets = look::Assets { css: String::new(), ui_font: None };
        let copies = gpu::Copies::default();
        gpu::keep_buffer(&copies, 3, 4, 0, &[1, 2, 3, 4]).expect("stored");
        let runtime = answer(&served, &pictures, &assets, &copies, "1", "/index.html");
        assert_eq!(runtime.status(), 200);
        assert_eq!(runtime.body().as_ref(), INDEX.as_bytes());
        assert_eq!(answer(&served, &pictures, &assets, &copies, "1", "/gpu/buffer/3").body().as_ref(), &[1, 2, 3, 4], "a runtime page takes a copy's bytes");
        assert_eq!(answer(&served, &pictures, &assets, &copies, "1", "/gpu/buffer/4").status(), 404, "a copy not stored is not found");
        let page = answer(&served, &pictures, &assets, &copies, "2", "/index.html");
        assert_eq!(page.status(), 200);
        assert_eq!(page.body().as_ref(), b"<p>a page of its own</p>");
        assert_eq!(answer(&served, &pictures, &assets, &copies, "2", "/runtime.js").status(), 404, "a page loads its own files, not the runtime's");
        assert_eq!(answer(&served, &pictures, &assets, &copies, "2", "/tokens.css").status(), 404, "a page of its own gets none of the runtime's look");
        assert_eq!(answer(&served, &pictures, &assets, &copies, "2", "/gpu/buffer/3").status(), 404, "a page of its own gets no copies");
        assert_eq!(answer(&served, &pictures, &assets, &copies, "3", "/index.html").status(), 404, "a webview with no window here gets nothing");
    }

    fn image(id: NodeId, bytes: &Arc<[u8]>, format: &str) -> Node {
        let picture = BTreeMap::from([(String::from("bytes"), Value::Bytes(Arc::clone(bytes))), (String::from("format"), Value::Str(format.to_string()))]);
        Node::new(id, Kind::Image).prop("image", Value::Map(picture))
    }

    #[test]
    fn a_windows_pictures_are_served_by_names_never_reused_while_its_images_show_them() {
        let (red, blue): (Arc<[u8]>, Arc<[u8]>) = (Arc::from(&b"\x89PNG red"[..]), Arc::from(&b"<svg blue"[..]));
        let mut tree = Node::new(1, Kind::Column).child(image(2, &red, "png")).child(image(3, &red, "png")).child(image(4, &blue, "svg"));
        let mut pictures = Pictures::default();
        let none = BTreeMap::new();
        assert!(pictures.take(&tree, &none), "the pictures came");
        let (red_at, blue_at) = (pictures.src(&red).expect("red has an address"), pictures.src(&blue).expect("blue has an address"));
        assert!(red_at.starts_with("picture/picture-") && red_at.ends_with(".png") && blue_at.ends_with(".svg"), "{red_at} {blue_at}");
        assert_eq!(pictures.by_name().len(), 2, "two Images of one picture name it once");
        assert!(!pictures.take(&tree, &none), "nothing came or went");
        assert_eq!(pictures.src(&red).as_deref(), Some(red_at.as_str()), "a picture stays under its name");
        let table = ArcSwap::from_pointee(HashMap::new());
        serve_pictures(&table, 9, &pictures);
        let served = ArcSwap::from_pointee(HashMap::from([(String::from("9"), Served::Runtime)]));
        let assets = look::Assets { css: String::new(), ui_font: None };
        let copies = gpu::Copies::default();
        let got = answer(&served, &table, &assets, &copies, "9", &format!("/{red_at}"));
        let media = got.headers()[CONTENT_TYPE].to_str().expect("the media type is text");
        assert_eq!((got.status(), got.body().as_ref(), media), (StatusCode::OK, &b"\x89PNG red"[..], "image/png"));
        assert_eq!(answer(&served, &table, &assets, &copies, "9", "/picture/picture-none.png").status(), 404);
        let green: Arc<[u8]> = Arc::from(&b"GIF89a green"[..]);
        tree.children.truncate(1);
        tree.children.push(image(5, &green, "gif"));
        assert!(pictures.take(&tree, &none), "blue went and green came");
        assert!(pictures.src(&blue).is_none(), "a picture no Image shows is let go");
        let green_at = pictures.src(&green).expect("green has an address");
        assert!(green_at != red_at && green_at != blue_at, "a new picture takes a name no picture had");
        let json = node_json(&tree, &Showing { pictures: &pictures, canvases: &none, now: Instant::now() });
        assert_eq!(json["children"][1]["props"]["image"], json!({ "src": green_at, "format": "gif" }), "the page loads a picture from its address, without its bytes");
        let picture = BTreeMap::from([(String::from("bytes"), Value::Bytes(Arc::clone(&blue))), (String::from("format"), Value::Str(String::from("svg")))]);
        let Value::Map(mut pictured) = rect("Logo", 0.0) else { panic!("a map") };
        pictured.insert(String::from("kind"), Value::Str(String::from("Image")));
        pictured.insert(String::from("image"), Value::Map(picture));
        let canvas = Node::new(6, Kind::Canvas).prop("items", Value::List(vec![Value::Map(pictured)]));
        tree.children.push(canvas);
        let (canvases, _) = open_canvases(&tree, Instant::now());
        assert!(pictures.take(&tree, &canvases), "a canvas's Image item shows its picture");
        assert!(pictures.src(&blue).is_some(), "named for the canvas");
    }

    #[test]
    fn a_table_keeps_its_rows_as_the_runtime_does_and_the_largest_batch() {
        let row = |n: i64| json!([{ "text": n.to_string(), "sort": n }]);
        let mut table = TableRows::default();
        table.take(0, vec![row(0), row(1), row(2)]);
        table.take(3, vec![row(3), row(4)]);
        assert_eq!((table.rows.len(), table.batch), (5, 3));
        table.take(1, vec![row(9)]);
        assert_eq!(table.rows, vec![row(0), row(9)]);
        table.take(7, vec![row(5)]);
        assert_eq!(table.rows, vec![row(0), row(9), row(5)], "a start past the end appends, as the runtime's slice does");
        assert_eq!(table.batch, 3);
    }

    #[test]
    fn a_page_loaded_again_never_takes_an_answer_owed_to_the_page_before() {
        let mut calls = Calls::default();
        let before = calls.ask(1);
        calls.forget();
        let again = calls.ask(1);
        assert_ne!(before, again, "the page loaded again numbers its first call 1 as well");
        assert_eq!(calls.answer(before), None);
        assert_eq!(calls.answer(again), Some(1));
        assert_eq!(calls.answer(again), None, "a call is replied to once");
    }

    #[test]
    fn a_number_still_owed_is_never_given_again_when_the_numbers_wrap() {
        let mut calls = Calls { owed: BTreeMap::new(), last: u32::MAX - 1 };
        let owed = calls.ask(7);
        assert_eq!(owed, u32::MAX);
        calls.last = u32::MAX - 1;
        assert_eq!(calls.ask(8), 0, "the owed number is passed over");
        assert_eq!((calls.answer(owed), calls.answer(0)), (Some(7), Some(8)));
    }

    #[test]
    fn a_page_s_gpu_answer_gives_the_texels_it_saved_or_the_error_it_names() {
        use proto::gpu::{GpuFailure, GpuValue};
        let saved = json!({ "t": "gpu", "request": 4, "ok": { "width": 1, "height": 2, "rgba": "/wAA/wD/AP8=" } });
        assert_eq!(page_gpu_answer(&saved, &PageGpu::Save), Ok(GpuValue::Texels { width: 1, height: 2, rgba: vec![255, 0, 0, 255, 0, 255, 0, 255] }));
        let short = json!({ "t": "gpu", "request": 4, "ok": { "width": 2, "height": 2, "rgba": "/wAA/wD/AP8=" } });
        assert_eq!(page_gpu_answer(&short, &PageGpu::Save).map_err(|f| f.id), Err(String::from("GuiGpuSave")), "texels too few for the size are refused");
        let refused = json!({ "t": "gpu", "request": 5, "error": { "id": "GuiGpuNotDrawn", "text": "GpuSurface View is not drawn" } });
        assert_eq!(page_gpu_answer(&refused, &PageGpu::Save), Err(GpuFailure { id: String::from("GuiGpuNotDrawn"), text: String::from("GpuSurface View is not drawn") }));
        let surface = proto::gpu::PageSurface { clear: [0.0; 4], clear_depth: None, clear_stencil: None, draws: Vec::new(), passes: Vec::new(), uniform: Vec::new() };
        assert_eq!(page_gpu_answer(&json!({ "t": "gpu", "request": 6, "ok": null }), &PageGpu::Surface { node: 3, surface }), Ok(GpuValue::Done));
        let bare = json!({ "t": "gpu", "request": 7, "error": "no" });
        assert_eq!(page_gpu_answer(&bare, &PageGpu::Save).map_err(|f| f.id), Err(String::from("GuiGpuDraw")), "an error without its id and text is still an error");
    }

    #[test]
    fn a_surface_s_pointer_event_sends_its_numbers_as_floats_and_a_page_reports_its_camera_s_pose() {
        let tree = Node::new(1, Kind::Column).child(Node::new(2, Kind::GpuSurface).named("View"));
        let given = value_from(&json!({ "x": 16, "y": 8.5, "u": 0, "v": 1, "ppp": 1, "notches": -2, "button": "Left", "shift": true }));
        let Value::Map(sent) = as_sent(&tree, 2, given) else {
            panic!("a map");
        };
        for key in ["x", "u", "v", "ppp", "notches"] {
            assert!(matches!(sent.get(key), Some(Value::Float(_))), "{key} is a Float, as egui sends it: {:?}", sent.get(key));
        }
        assert_eq!((sent.get("y"), sent.get("button")), (Some(&Value::Float(8.5)), Some(&Value::Str(String::from("Left")))));
        let pose = json!({ "t": "camera", "node": 2, "pose": { "target": [1, 2.5, -3], "distance": 4, "yaw": -90, "pitch": 12.5 } });
        assert_eq!(page_pose(&pose), Ok((2, [1.0, 2.5, -3.0], 4.0, -90.0, 12.5)));
        let short = json!({ "t": "camera", "node": 2, "pose": { "target": [1, 2], "distance": 4, "yaw": 0, "pitch": 0 } });
        assert!(page_pose(&short).is_err_and(|e| e.contains("three numbers")));
        let unset = json!({ "t": "camera", "node": 2, "pose": { "target": [0, 0, 0], "distance": null, "yaw": 0, "pitch": 0 } });
        assert!(page_pose(&unset).is_err_and(|e| e.contains("distance")));
    }

    #[test]
    fn a_page_with_webgl_alone_gets_a_draw_s_glsl_or_why_its_engine_cannot_draw_it() {
        let plain = "@vertex fn vs(@builtin(vertex_index) i: u32) -> @builtin(position) vec4<f32> { return vec4<f32>(0.0, 0.0, 0.0, 1.0); }
            @fragment fn fs() -> @location(0) vec4<f32> { return vec4<f32>(1.0); }";
        let written = page_glsl(&json!({ "t": "glsl", "request": 1, "source": plain, "constants": {} }), "WebKitGTK 2.46.6");
        assert!(written["vertex"]["glsl"].as_str().is_some_and(|g| g.starts_with("#version 300 es")), "{written}");
        let stored = "@group(0) @binding(0) var<storage, read> tint: array<f32, 4>;
            @vertex fn vs() -> @builtin(position) vec4<f32> { return vec4<f32>(0.0, 0.0, 0.0, 1.0); }
            @fragment fn fs() -> @location(0) vec4<f32> { return vec4<f32>(tint[0]); }";
        let refused = page_glsl(&json!({ "t": "glsl", "request": 2, "source": stored }), "WebKitGTK 2.46.6");
        assert_eq!(refused["error"]["id"], "GuiGpuNoWebGpu");
        let text = refused["error"]["text"].as_str().expect("the reason");
        assert!(text.starts_with("WebKitGTK 2.46.6 has no WebGPU") && text.contains("-Renderer Egui"), "{text}");
        assert_eq!(page_glsl(&json!({ "t": "glsl", "request": 3, "source": plain, "constants": { "scale": "two" } }), "WebKit 17.6")["error"]["id"], "GuiGpuDraw");
    }

    #[test]
    fn a_runtime_page_loaded_again_gets_its_tree_inputs_and_rows_in_batches() {
        let tree = Node::new(1, Kind::Column).child(Node::new(3, Kind::ListBox)).child(Node::new(7, Kind::Table));
        let inputs = BTreeMap::from([(3, json!([1])), (7, json!({ "selected": [0], "filter": "b", "sort": null, "anchor": 0 }))]);
        let mut table = TableRows::default();
        table.take(0, vec![json!(["a"]), json!(["b"])]);
        table.take(2, vec![json!(["c"]), json!(["d"]), json!(["e"])]);
        let (pictures, canvases) = (Pictures::default(), BTreeMap::new());
        let scripts = runtime_scripts(&tree, proto::Theme::Dark, false, false, &inputs, &BTreeMap::from([(7, table)]), &Showing { pictures: &pictures, canvases: &canvases, now: Instant::now() }, "WebKitGTK 2.46.6");
        assert_eq!(scripts.len(), 3, "{scripts:?}");
        assert!(scripts[0].starts_with("pwrsRuntime.open("));
        let open: Json = serde_json::from_str(scripts[0].trim_start_matches("pwrsRuntime.open(").trim_end_matches(");")).expect("the open script has JSON");
        assert_eq!(open["windowsOrder"], json!(button_bar::WINDOWS_ORDER), "a button bar orders its buttons as the helper's system does");
        assert_eq!(open["theme"], json!("dark"));
        assert_eq!(page_theme(proto::Theme::System), Json::Null, "a window in the system's theme names none");
        assert_eq!(open["fitting"], json!(false), "a page loaded again is shown already");
        assert_eq!(open["view"]["keyStep"], json!(80.0), "a canvas's view moves by the protocol's numbers");
        assert_eq!(open["engine"], json!("WebKitGTK 2.46.6"), "the page names its engine when it refuses what the engine cannot draw");
        assert_eq!(open["state"]["3"], json!([1]));
        assert_eq!(open["state"]["7"]["filter"], json!("b"));
        assert_eq!(scripts[1], r#"pwrsRuntime.rows(7, 0, [["a"],["b"],["c"]]);"#);
        assert_eq!(scripts[2], r#"pwrsRuntime.rows(7, 3, [["d"],["e"]]);"#);
    }

    #[test]
    fn enter_and_escape_press_the_windows_buttons_unless_the_focused_widget_uses_the_key() {
        let button = |id, text: &str, role: &str| Node::new(id, Kind::Button).prop("text", Value::Str(text.into())).prop(role, Value::Bool(true));
        let fields = || {
            Node::new(1, Kind::Column)
                .child(Node::new(2, Kind::TextBox))
                .child(Node::new(3, Kind::TextBox).prop("multiline", Value::Bool(true)))
                .child(Node::new(4, Kind::ListBox).prop("items", Value::List(Vec::new())))
        };
        let tree = fields().child(Node::new(5, Kind::ButtonBar).child(button(6, "OK", "default")).child(button(7, "Cancel", "cancel")));
        assert_eq!(role_press(&tree, "Enter", None, false), Ok(RolePress::Click(6)));
        assert_eq!(role_press(&tree, "Enter", Some(2), false), Ok(RolePress::Click(6)), "a field of one line leaves Enter to the default button, as egui does");
        assert_eq!(role_press(&tree, "Enter", Some(3), false), Ok(RolePress::Nothing), "a field of several lines takes Enter");
        assert_eq!(role_press(&tree, "Enter", Some(4), false), Ok(RolePress::Nothing), "a list takes Enter for its item");
        assert_eq!(role_press(&tree, "Escape", Some(2), true), Ok(RolePress::Click(7)), "Escape presses the cancel button, which closes the window itself");
        let off = fields().child(Node::new(5, Kind::ButtonBar).child(button(6, "OK", "default").prop("enabled", Value::Bool(false))));
        assert_eq!(role_press(&off, "Enter", None, false), Ok(RolePress::Nothing), "a disabled button is not pressed");
        assert_eq!(role_press(&off, "Escape", None, true), Ok(RolePress::Close), "with no cancel button Escape closes a dialog");
        assert_eq!(role_press(&off, "Escape", None, false), Ok(RolePress::Nothing), "and leaves any other window open");
        assert!(role_press(&tree, "Space", None, false).is_err());
    }

    #[test]
    fn a_window_hidden_until_measured_opens_at_its_fitted_size_or_at_its_most() {
        let options = proto::WindowOptions { fit: proto::Fit::DIALOG, ..proto::WindowOptions::titled("d") };
        let work = [1920.0, 1040.0];
        let fitting = Fitting { most: options.fitted([f32::INFINITY; 2], Some(work)), options, work, on: None };
        assert_eq!(fitting.most, [548.0, 756.0], "a dialog is laid out within its most");
        assert_eq!(fitting.size(Some([200.0, 120.0])), [320.0, 120.0]);
        assert_eq!(fitting.size(None), [548.0, 756.0], "a window its page could not measure opens at the most it may take");
        assert_eq!(page_size(&json!({ "t": "fit", "size": [300.5, 120] })), Ok([300.5, 120.0]));
        let why = page_size(&json!({ "t": "fit", "size": null, "error": "the window could not be measured: no fonts" })).expect_err("no size");
        assert!(why.contains("could not be measured"), "{why}");
        assert!(page_size(&json!({ "t": "fit", "size": [1] })).is_err(), "a size is two lengths");
        assert!(page_size(&json!({ "t": "fit", "size": [-1, 2] })).is_err(), "and neither is below 0");
    }

    #[test]
    fn a_page_given_as_text_is_its_only_file_and_carries_the_policy_unless_it_may_navigate() {
        let page = PageSource::Text(Arc::from("<p>hi</p>"));
        let served = serve_page(&page, "/index.html", false);
        assert_eq!(served.status(), 200);
        assert_eq!(served.body().as_ref(), b"<p>hi</p>");
        assert_eq!(served.headers().get("content-security-policy").and_then(|v| v.to_str().ok()), Some(PAGE_POLICY));
        assert_eq!(serve_page(&page, "/other.js", false).status(), 404);
        assert!(serve_page(&page, "/", true).headers().get("content-security-policy").is_none());
    }

    #[test]
    fn a_page_folder_serves_its_files_and_refuses_paths_that_leave_it() {
        let root = std::env::temp_dir().join(format!("pwrs-gui-web-page-test-{}", std::process::id()));
        std::fs::create_dir_all(root.join("sub dir")).expect("a page folder");
        std::fs::write(root.join("index.html"), b"<p>home</p>").expect("index.html");
        std::fs::write(root.join("sub dir").join("app.js"), b"1;").expect("app.js");
        let folder = PageSource::Folder(root.clone());
        assert_eq!(serve_page(&folder, "/", false).body().as_ref(), b"<p>home</p>");
        let script = serve_page(&folder, "/sub%20dir/app.js", false);
        assert_eq!(script.status(), 200);
        assert_eq!(script.headers().get(CONTENT_TYPE).and_then(|v| v.to_str().ok()), Some("text/javascript; charset=utf-8"));
        assert_eq!(serve_page(&folder, "/../secret.txt", false).status(), 403);
        assert_eq!(serve_page(&folder, "/sub%20dir/..%2F..%2Fsecret.txt", false).status(), 403);
        assert_eq!(serve_page(&folder, "/missing.css", false).status(), 404);
        std::fs::remove_dir_all(&root).expect("the page folder removed");
    }

    #[test]
    fn paths_decode_and_encode_as_urls_write_them() {
        assert_eq!(percent_decode("/a%20b/c%2Fd"), Some(String::from("/a b/c/d")));
        assert_eq!(percent_decode("/bad%2"), None);
        assert_eq!(encode_path("sub dir/page #1.html"), "sub%20dir/page%20%231.html");
        assert_eq!(encode_path("sub\\page.html"), "sub/page.html");
    }

    #[test]
    fn only_the_pages_own_origin_counts_as_its_own() {
        assert!(own_origin("pwrs://localhost/index.html"));
        assert!(own_origin("http://pwrs.localhost/index.html"));
        assert!(own_origin("https://pwrs.localhost/app/?view=2#top"));
        assert!(own_origin("pwrs://localhost"));
        assert!(!own_origin("https://example.com/"));
        for lookalike in [
            "http://pwrs.example.com/",
            "https://pwrs.attacker.example/index.html",
            "http://pwrs.localhost.example.com/",
            "http://pwrs.localhost:8080/",
            "http://pwrs.localhost@example.com/",
            "pwrs://localhostile/",
            "pwrs://other/index.html",
        ] {
            assert!(!own_origin(lookalike), "{lookalike} was taken as the page's own origin");
        }
    }

    #[test]
    fn only_the_c_and_posix_locales_name_no_language() {
        for name in ["C", "C.UTF-8", "C.utf8", "c", "POSIX", "C@euro"] {
            assert!(names_no_language(name), "{name}");
        }
        for name in ["en_US.UTF-8", "de_DE", "de", "Cy", "ca_ES@valencia"] {
            assert!(!names_no_language(name), "{name}");
        }
    }

    #[test]
    fn a_page_takes_html_or_a_folder_and_names_what_it_lacks() {
        let html = Node::new(1, Kind::Html).prop("html", Value::Str("<p/>".into())).prop("dev_tools", Value::Bool(true));
        let page = Page::of(&html).expect("a page of text");
        assert_eq!((page.start.as_str(), page.allow_navigation, page.dev_tools), ("/index.html", false, true));
        let folder = Node::new(1, Kind::Html).prop("folder", Value::Str("C:/site".into())).prop("page", Value::Str("start here.html".into()));
        assert_eq!(Page::of(&folder).expect("a page of a folder").start, "/start%20here.html");
        let neither = Page::of(&Node::new(1, Kind::Html));
        assert!(matches!(neither, Err(ref why) if why.contains("neither")), "{:?}", neither.err());
        let wrong = Page::of(&Node::new(1, Kind::Html).prop("html", Value::Int(3)));
        assert!(matches!(wrong, Err(ref why) if why.contains("html")), "{:?}", wrong.err());
    }

    #[test]
    fn a_date_cell_reaches_the_page_sorting_by_its_time_and_marked_as_a_date() {
        let row = Row {
            cells: vec![
                proto::Cell { text: String::from("10/20/2026 9:39:55 AM"), sort: SortKey::Date(639_280_859_950_000_000.0) },
                proto::Cell { text: String::from("3"), sort: SortKey::Number(3.0) },
            ],
        };
        let expected = json!([{ "text": "10/20/2026 9:39:55 AM", "sort": 639_280_859_950_000_000.0, "date": true }, { "text": "3", "sort": 3.0 }]);
        assert_eq!(rows_json(&[row]), vec![expected], "a number has no date mark, so it stands at the right");
    }

    #[test]
    fn a_page_given_as_text_without_a_doctype_is_served_behind_one() {
        let served = |html: &str| match Page::of(&Node::new(1, Kind::Html).prop("html", Value::Str(html.into()))).expect("a page of text").source {
            PageSource::Text(text) => String::from(&*text),
            PageSource::Folder(_) => panic!("a page of text is given from its text"),
        };
        assert_eq!(served("<p>hi</p>"), "<!DOCTYPE html>\n<p>hi</p>");
        assert_eq!(served(""), "<!DOCTYPE html>\n");
        assert_eq!(served("<!-- a note --><!doctype html><p>hi</p>"), "<!DOCTYPE html>\n<!-- a note --><!doctype html><p>hi</p>", "a page that begins with a comment gets one too, and the parser passes over its own, which comes second");
        for given in ["<!DOCTYPE html><p>hi</p>", "<!doctype html>", "\u{feff}\n  <!DocType html PUBLIC \"-//W3C//DTD HTML 4.01//EN\">"] {
            assert_eq!(served(given), given, "text with a doctype is given as it is");
        }
        assert_eq!(served("<!doc"), "<!DOCTYPE html>\n<!doc", "too short to be a doctype");
        assert_eq!(served("é<!doctype html>"), "<!DOCTYPE html>\né<!doctype html>", "text before a doctype is a page without one");
    }
}
