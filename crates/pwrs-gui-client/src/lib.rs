//! A PWRS-GUI session for any Rust program: it starts a renderer, opens
//! windows, and waits on them, with no PowerShell involved. The PWRS-GUI
//! module is one user of it.
//!
//! The renderer runs in a helper process the session starts, or on
//! Windows in this process on a UI thread of its own. Every wait - for the
//! renderer's `Hello`, a window's next event, a request's reply - sleeps
//! until something happens: the event itself, the caller's [`Canceler`]
//! (which a PowerShell module ties to Ctrl+C), or the renderer's death,
//! which the operating system reports and the session passes to every wait
//! at once. Nothing polls but a window's file watch on Windows, whose
//! file system does not report every write ([`Window::watch_files`]), and
//! nothing takes a lock to send.

pub use pwrs_gui_proto as proto;

#[cfg(any(windows, target_os = "linux", target_os = "freebsd"))]
mod addon;
mod cell;
pub mod entry;
mod file_watch;
mod helper;
#[cfg(all(windows, feature = "in-process"))]
mod inprocess;
#[cfg(windows)]
mod mesa;
mod pace;
mod platform;
mod watch;
#[cfg(any(target_os = "linux", target_os = "freebsd"))]
mod webkit;
#[cfg(windows)]
mod webview2;

pub use entry::NativeEntry;
pub use proto::Snapshot;
pub use watch::Canceler;

use arc_swap::ArcSwapOption;
use proto::transport::{self, TransportError};
use proto::{
    Adapter, Capabilities, CloseReason, DialogAnswer, DialogFailure, DialogKind, DialogSummary, EventKind, FromHelper, Node, NodeId, PatchOp, RequestId, RingName, Row, ToHelper, Value, WebGraphics, WindowId,
    WindowOptions, WindowSummary,
};
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::Duration;
use watch::Watches;

/// Where windows are drawn.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Mode {
    /// The helper process; on Windows, this process when the helper cannot
    /// start for any reason.
    #[default]
    Auto,
    /// The helper process only.
    Helper,
    /// This process, on a UI thread of its own. Windows only.
    InProcess,
}

/// Which renderer draws a session's windows.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Renderer {
    /// egui on wgpu.
    #[default]
    Egui,
    /// A webview: WebView2 on Windows, WebKit on macOS, WebKitGTK on Linux
    /// and FreeBSD.
    Html,
}

impl Renderer {
    /// The name PowerShell gives it.
    pub fn name(self) -> &'static str {
        match self {
            Renderer::Egui => "Egui",
            Renderer::Html => "Html",
        }
    }

    /// The helper executable that runs it, as named in the module's
    /// `helpers`.
    pub fn helper_name(self) -> &'static str {
        match self {
            Renderer::Egui => "pwrs-gui-egui",
            Renderer::Html => "pwrs-gui-web",
        }
    }

    /// The bits set in every number its session gives a window, a request
    /// or a ring, so the two renderers' sessions never share a number.
    pub fn number_base(self) -> u64 {
        match self {
            Renderer::Egui => 0,
            Renderer::Html => HTML_NUMBERS,
        }
    }

    /// The renderer whose session gave out `number`.
    pub fn of_number(number: u64) -> Renderer {
        if number & HTML_NUMBERS != 0 { Renderer::Html } else { Renderer::Egui }
    }
}

/// The bit set in an HTML session's numbers, above every taker's base.
const HTML_NUMBERS: u64 = 1 << 62;

/// Whether a process's HTML renderer shares its web data with other
/// processes'.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum WebViewData {
    /// One folder for every process of the user. On Windows they all share
    /// one WebView2 browser process, so a crash of it ends every page.
    #[default]
    Shared,
    /// A folder of each process's own, a data store on macOS 14 and later,
    /// and on Windows a WebView2 browser process of its own.
    PerProcess,
}

impl WebViewData {
    /// The name PowerShell gives it.
    pub fn name(self) -> &'static str {
        match self {
            WebViewData::Shared => "Shared",
            WebViewData::PerProcess => "PerProcess",
        }
    }
}

/// How a session's renderer starts.
#[derive(Clone, Debug)]
pub struct StartOptions {
    pub mode: Mode,
    pub renderer: Renderer,
    /// The renderer's helper executable, or why there is none.
    pub helper: Result<PathBuf, String>,
    /// Part of an adapter's name to prefer; egui only.
    pub adapter: Option<String>,
    /// Prefer the adapter that draws the least power; egui only.
    pub low_power: bool,
    /// The folder the helper's log goes in.
    pub log_dir: PathBuf,
    /// Whether the renderer shares its web data with other processes';
    /// HTML only.
    pub web_view: WebViewData,
    /// The shared web data folder in place of the default; HTML only, with
    /// [`WebViewData::Shared`].
    pub web_view_data_folder: Option<PathBuf>,
    /// The fonts the renderer draws text in.
    pub fonts: proto::Fonts,
    /// The environment a helper starts with, from which the start also
    /// reads `DISPLAY`, `WAYLAND_DISPLAY`, `PSModulePath`,
    /// `PWRS_GUI_WEBKIT_ADDON` and `PWRS_GUI_MESA_ADDON`; `None` takes the
    /// process's own. On Linux,
    /// FreeBSD and macOS what PowerShell sets with `$env:` stays out of the
    /// process's environment, so a PowerShell session passes its own.
    pub environment: Option<Vec<(OsString, OsString)>>,
}

impl StartOptions {
    /// The value of variable `name` in the environment the start reads;
    /// `None` when it is not set. Names match without regard to ASCII case
    /// on Windows, as its environment does.
    pub fn var(&self, name: &str) -> Option<OsString> {
        let Some(vars) = &self.environment else {
            return std::env::var_os(name);
        };
        let same = |key: &OsString| if cfg!(windows) { key.eq_ignore_ascii_case(name) } else { key == name };
        vars.iter().find(|(key, _)| same(key)).map(|(_, value)| value.clone())
    }
}

/// Why a session, a window or a request failed.
#[derive(Debug)]
pub enum GuiError {
    /// There is no display a window could open on.
    NoDisplay(String),
    /// The renderer could not start.
    Start(String),
    /// The helper speaks another protocol than this build.
    Mismatch { helper: String, here: String },
    /// The wait was ended through its [`Canceler`].
    Canceled,
    /// The renderer has gone.
    Gone(String),
    /// The renderer refused the request.
    Refused(String),
    /// A step of a test's input was not taken and seen in its time; the
    /// text names the step.
    Late(String),
    /// A ring or a channel failed.
    Transport(String),
    /// The caller's arm refused the wait.
    Arm(String),
    /// The system has no dialog of the kind requested; the egui renderer's
    /// own stands in.
    NoNativeDialog(String),
    /// A window's files could not be watched.
    Watch(String),
}

impl std::fmt::Display for GuiError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            GuiError::NoDisplay(why) => f.write_str(why),
            GuiError::Start(why) => write!(f, "the GUI renderer could not start: {why}"),
            GuiError::Mismatch { helper, here } => write!(f, "the GUI helper is {helper}, and this is {here}; they come from different builds"),
            GuiError::Canceled => f.write_str("the wait was canceled"),
            GuiError::Gone(why) => write!(f, "the GUI renderer has gone: {why}"),
            GuiError::Refused(why) => write!(f, "the GUI renderer refused: {why}"),
            GuiError::Late(why) => f.write_str(why),
            GuiError::Transport(why) => write!(f, "the link to the GUI renderer failed: {why}"),
            GuiError::Arm(why) => f.write_str(why),
            GuiError::NoNativeDialog(why) => write!(f, "the system has no dialog of this kind: {why}"),
            GuiError::Watch(why) => write!(f, "the window's files cannot be watched: {why}"),
        }
    }
}

impl std::error::Error for GuiError {}

/// What the renderer said when it started.
#[derive(Clone, Debug)]
pub struct Hello {
    pub build: String,
    pub renderer: String,
    pub capabilities: Capabilities,
    /// What the person should hear about the start: a setting the platform
    /// cannot honor, a monitor that is not connected, or data of an ended
    /// process that could not be removed.
    pub notes: Vec<String>,
}

/// A message, boxed since a snapshot's is large, or the wake that ends a
/// wait, on an in-memory channel.
#[cfg(all(windows, feature = "in-process"))]
#[derive(Debug)]
pub(crate) enum Delivery {
    Msg(Box<FromHelper>),
    Wake,
}

pub(crate) enum Link {
    Helper(helper::HelperLink),
    #[cfg(all(windows, feature = "in-process"))]
    InProcess(inprocess::InProcessLink),
    /// A session another module offers, taken through its native entry.
    Shared(entry::SharedLink),
}

pub(crate) enum StartRx {
    Ring(transport::Receiver),
    #[cfg(all(windows, feature = "in-process"))]
    Channel(std::sync::mpsc::Receiver<Delivery>),
}

/// Where a window's events arrive: its ring, or on the in-process path its
/// channel, with a sender the window's file watch posts through.
pub(crate) enum EventRx {
    Ring(transport::Receiver),
    #[cfg(all(windows, feature = "in-process"))]
    Channel(std::sync::mpsc::Receiver<Delivery>, std::sync::mpsc::Sender<Delivery>),
}

/// The state of one session, shared by everything that uses it.
pub(crate) struct Inner {
    pub(crate) id: String,
    pub(crate) link: Link,
    pub(crate) renderer: Renderer,
    hello: OnceLock<Hello>,
    next: AtomicU64,
    pub(crate) watches: Watches,
    death: OnceLock<String>,
}

impl Inner {
    /// A session of `renderer` whose numbers start at 1 above the
    /// renderer's base.
    pub(crate) fn new(id: String, link: Link, renderer: Renderer) -> Inner {
        Inner::with_base(id, link, renderer, renderer.number_base() + 1)
    }

    /// A session whose numbers start at `base`.
    pub(crate) fn with_base(id: String, link: Link, renderer: Renderer, base: u64) -> Inner {
        Inner { id, link, renderer, hello: OnceLock::new(), next: AtomicU64::new(base), watches: Watches::new(), death: OnceLock::new() }
    }

    /// A number unique in this session, for windows, requests and rings.
    pub(crate) fn number(&self) -> u64 {
        self.next.fetch_add(1, Ordering::Relaxed)
    }

    pub(crate) fn death(&self) -> Option<String> {
        self.death.get().cloned()
    }

    /// Records the renderer's death, then wakes every wait. The first
    /// reason stands.
    pub(crate) fn die(&self, reason: String) {
        if self.death.set(reason).is_ok() {
            self.watches.wake_all();
        }
    }

    /// Why a wait that was woken returned without a message.
    pub(crate) fn woken(&self, canceler: &Canceler) -> GuiError {
        match self.death() {
            Some(reason) => GuiError::Gone(reason),
            None if canceler.is_canceled() => GuiError::Canceled,
            None => GuiError::Gone(String::from("the wait's handle was closed")),
        }
    }

    pub(crate) fn transport_failed(&self, e: TransportError) -> GuiError {
        match self.death() {
            Some(reason) => GuiError::Gone(reason),
            None => GuiError::Transport(e.to_string()),
        }
    }

    pub(crate) fn kill(&self) {
        match &self.link {
            Link::Helper(h) => h.kill(),
            #[cfg(all(windows, feature = "in-process"))]
            Link::InProcess(_) => {}
            Link::Shared(_) => {}
        }
    }

    fn send(self: &Arc<Self>, msg: ToHelper) -> Result<(), GuiError> {
        match &self.link {
            Link::Helper(h) => helper::send(self, &helper::Outlet::Inbound(&h.inbound), &msg),
            #[cfg(all(windows, feature = "in-process"))]
            Link::InProcess(l) => l.send(msg, None),
            Link::Shared(s) => helper::send(self, &s.outlet(), &msg),
        }
    }

    fn request<G>(self: &Arc<Self>, build: impl FnOnce(RequestId, RingName) -> ToHelper, arm: &mut impl FnMut(Canceler) -> Result<G, GuiError>) -> Result<FromHelper, GuiError> {
        self.request_paced(build, arm, &mut pace::Unpaced)
    }

    /// Sends a request and waits for its reply as `pace` says.
    fn request_paced<G>(self: &Arc<Self>, build: impl FnOnce(RequestId, RingName) -> ToHelper, arm: &mut impl FnMut(Canceler) -> Result<G, GuiError>, pace: &mut impl pace::Pace) -> Result<FromHelper, GuiError> {
        match &self.link {
            Link::Helper(h) => helper::request(self, &helper::Outlet::Inbound(&h.inbound), build, arm, pace),
            #[cfg(all(windows, feature = "in-process"))]
            Link::InProcess(l) => inprocess::request(self, l, build, arm, pace),
            Link::Shared(s) => helper::request(self, &s.outlet(), build, arm, pace),
        }
    }
}

/// The request number in a reply.
pub(crate) fn reply_number(msg: &FromHelper) -> Option<RequestId> {
    match msg {
        FromHelper::Adapters { request, .. }
        | FromHelper::Windows { request, .. }
        | FromHelper::PageGraphics { request, .. }
        | FromHelper::TestEventDone { request, .. }
        | FromHelper::InputStep { request, .. }
        | FromHelper::Snapshot { request, .. }
        | FromHelper::BridgeDone { request, .. }
        | FromHelper::CloseDone { request, .. }
        | FromHelper::FocusDone { request, .. }
        | FromHelper::DialogResult { request, .. }
        | FromHelper::Dialogs { request, .. }
        | FromHelper::GpuResult { request, .. } => Some(*request),
        FromHelper::Hello { .. }
        | FromHelper::StartFailed { .. }
        | FromHelper::Opened { .. }
        | FromHelper::Event { .. }
        | FromHelper::Closed { .. }
        | FromHelper::BridgeMessage { .. }
        | FromHelper::PageClosed { .. }
        | FromHelper::Log { .. }
        | FromHelper::Error { .. }
        | FromHelper::PickerWanted { .. }
        | FromHelper::DeviceLost { .. }
        | FromHelper::FileChanged { .. }
        | FromHelper::PageCall { .. }
        | FromHelper::PageOpen { .. } => None,
    }
}

/// A renderer starting, waiting to be read.
pub struct Starting {
    pub(crate) inner: Arc<Inner>,
    pub(crate) rx: StartRx,
    pub(crate) canceler: Canceler,
    pub(crate) settled: bool,
}

impl Starting {
    /// Ends [`wait`](Self::wait) from any thread.
    pub fn canceler(&self) -> Canceler {
        self.canceler.clone()
    }

    /// Reads the renderer's `Hello`, with no time limit. A renderer that
    /// dies first is a [`GuiError::Start`] naming why.
    pub fn wait(mut self) -> Result<Session, GuiError> {
        let first = match &mut self.rx {
            StartRx::Ring(rx) => match rx.recv::<FromHelper>() {
                Ok(m) => Ok(m),
                Err(TransportError::Closed { .. }) => Err(self.inner.woken(&self.canceler)),
                Err(e) => Err(self.inner.transport_failed(e)),
            },
            #[cfg(all(windows, feature = "in-process"))]
            StartRx::Channel(rx) => match rx.recv() {
                Ok(Delivery::Msg(m)) => Ok(*m),
                Ok(Delivery::Wake) => Err(self.inner.woken(&self.canceler)),
                Err(e) => Err(GuiError::Start(format!("the start channel closed: {e}"))),
            },
        };
        let outcome = match first {
            Ok(FromHelper::Hello { protocol, build, renderer, capabilities, notes }) if protocol == proto::PROTOCOL => {
                if self.inner.hello.set(Hello { build, renderer, capabilities, notes }).is_err() {
                    Err(GuiError::Start(String::from("the renderer said Hello twice")))
                } else {
                    Ok(())
                }
            }
            Ok(FromHelper::Hello { protocol, build, renderer, .. }) => Err(GuiError::Mismatch {
                helper: format!("the {renderer} renderer {build} speaking protocol {protocol}"),
                here: format!("PWRS-GUI {} speaking protocol {}", env!("CARGO_PKG_VERSION"), proto::PROTOCOL),
            }),
            Ok(FromHelper::StartFailed { text }) => Err(GuiError::Start(text)),
            Ok(other) => Err(GuiError::Start(format!("the renderer's first message was {other:?}"))),
            Err(GuiError::Gone(reason)) => Err(GuiError::Start(reason)),
            Err(e) => Err(e),
        };
        outcome?;
        self.settled = true;
        Ok(Session { inner: Arc::clone(&self.inner) })
    }
}

impl Drop for Starting {
    fn drop(&mut self) {
        if !self.settled {
            self.inner.kill();
        }
    }
}

/// A running session. Clones share it.
#[derive(Clone)]
pub struct Session {
    inner: Arc<Inner>,
}

impl Session {
    /// Starts a renderer as `options` says and waits for its `Hello` with
    /// no time limit. `arm` is given each wait's canceler just before the
    /// wait begins, and what it returns lives as long as the wait; it must
    /// not call back into the session. Returns the session and, when Auto
    /// mode drew in-process instead, why the helper could not start.
    pub fn start<G>(options: &StartOptions, mut arm: impl FnMut(Canceler) -> Result<G, GuiError>) -> Result<(Session, Option<String>), GuiError> {
        platform::check_display(options)?;
        match options.mode {
            Mode::Helper => Ok((start_helper(options, &mut arm)?, None)),
            Mode::InProcess => Ok((start_in_process(options, &mut arm)?, None)),
            Mode::Auto => match start_helper(options, &mut arm) {
                Ok(s) => Ok((s, None)),
                Err(GuiError::Start(why)) if cfg!(all(windows, feature = "in-process")) => Ok((start_in_process(options, &mut arm)?, Some(why))),
                Err(e) => Err(e),
            },
        }
    }

    pub fn id(&self) -> &str {
        &self.inner.id
    }

    /// The renderer drawing this session's windows.
    pub fn renderer(&self) -> Renderer {
        self.inner.renderer
    }

    /// `Helper` or `InProcess`; for a session taken through another
    /// module's entry, where that module's renderer draws.
    pub fn path(&self) -> &'static str {
        match &self.inner.link {
            Link::Helper(_) => "Helper",
            #[cfg(all(windows, feature = "in-process"))]
            Link::InProcess(_) => "InProcess",
            Link::Shared(s) => match s.path {
                entry::SharedPath::Helper(_) => "Helper",
                entry::SharedPath::InProcess => "InProcess",
            },
        }
    }

    /// Whether this session was taken through another module's native
    /// entry.
    pub fn is_taken(&self) -> bool {
        matches!(self.inner.link, Link::Shared(_))
    }

    pub fn hello(&self) -> Option<&Hello> {
        self.inner.hello.get()
    }

    /// The helper's log file; `None` in-process and for a session taken
    /// through another module's entry.
    pub fn log(&self) -> Option<&Path> {
        match &self.inner.link {
            Link::Helper(h) => Some(&h.log),
            #[cfg(all(windows, feature = "in-process"))]
            Link::InProcess(_) => None,
            Link::Shared(_) => None,
        }
    }

    pub fn is_alive(&self) -> bool {
        self.inner.death().is_none()
    }

    /// Why the renderer is gone, once it is.
    pub fn death(&self) -> Option<String> {
        self.inner.death()
    }

    /// Whether `other` is this session.
    pub fn same(&self, other: &Session) -> bool {
        Arc::ptr_eq(&self.inner, &other.inner)
    }

    /// Opens a window. Its events are read through the [`Window`].
    pub fn open_window(&self, options: WindowOptions, tree: Node) -> Result<Window, GuiError> {
        self.open_window_with_rows(options, tree, Vec::new())
    }

    /// Opens a window whose tables are filled with `rows` from their first
    /// row, by each table's node, so a window that fits its content is
    /// measured with them; [`Window::rows`] adds more.
    pub fn open_window_with_rows(&self, options: WindowOptions, tree: Node, rows: Vec<(NodeId, Vec<Row>)>) -> Result<Window, GuiError> {
        if let Some(reason) = self.inner.death() {
            return Err(GuiError::Gone(reason));
        }
        let id = self.inner.number();
        let (rx, wake) = match &self.inner.link {
            Link::Helper(h) => helper::open_window(&self.inner, &helper::Outlet::Inbound(&h.inbound), id, options, tree, rows)?,
            #[cfg(all(windows, feature = "in-process"))]
            Link::InProcess(l) => inprocess::open_window(&self.inner, l, id, options, tree, rows)?,
            Link::Shared(s) => helper::open_window(&self.inner, &s.outlet(), id, options, tree, rows)?,
        };
        Ok(Window { session: self.clone(), id, rx, canceler: Canceler::new(wake), open: true, files: None })
    }

    /// The adapters the renderer's backends find.
    pub fn adapters<G>(&self, mut arm: impl FnMut(Canceler) -> Result<G, GuiError>) -> Result<Vec<Adapter>, GuiError> {
        match self.inner.request(|request, reply| ToHelper::ListAdapters { request, reply }, &mut arm)? {
            FromHelper::Adapters { adapters, .. } => Ok(adapters),
            other => Err(unexpected("ListAdapters", other)),
        }
    }

    /// Runs `op` on the renderer's GPU layer: what the layer returned, or
    /// why it refused the call. Only the egui renderer has a GPU layer.
    pub fn gpu<G>(&self, op: proto::gpu::GpuOp, mut arm: impl FnMut(Canceler) -> Result<G, GuiError>) -> Result<Result<proto::gpu::GpuValue, proto::gpu::GpuFailure>, GuiError> {
        match self.inner.request(|request, reply| ToHelper::Gpu { request, reply, op }, &mut arm)? {
            FromHelper::GpuResult { outcome, .. } => Ok(outcome),
            other => Err(unexpected("Gpu", other)),
        }
    }

    /// Frees GPU buffer `buffer` without waiting for the renderer.
    pub fn gpu_free(&self, buffer: u64) -> Result<(), GuiError> {
        self.inner.send(ToHelper::GpuFree { buffer })
    }

    /// Frees render pipeline `pipeline` without waiting for the renderer.
    pub fn gpu_free_pipeline(&self, pipeline: u64) -> Result<(), GuiError> {
        self.inner.send(ToHelper::GpuFreePipeline { pipeline })
    }

    /// Frees texture `texture` without waiting for the renderer.
    pub fn gpu_free_texture(&self, texture: u64) -> Result<(), GuiError> {
        self.inner.send(ToHelper::GpuFreeTexture { texture })
    }

    /// Frees sampler `sampler` without waiting for the renderer.
    pub fn gpu_free_sampler(&self, sampler: u64) -> Result<(), GuiError> {
        self.inner.send(ToHelper::GpuFreeSampler { sampler })
    }

    /// Every window the renderer has open.
    pub fn windows<G>(&self, mut arm: impl FnMut(Canceler) -> Result<G, GuiError>) -> Result<Vec<WindowSummary>, GuiError> {
        match self.inner.request(|request, reply| ToHelper::ListWindows { request, reply }, &mut arm)? {
            FromHelper::Windows { windows, .. } => Ok(windows),
            other => Err(unexpected("ListWindows", other)),
        }
    }

    /// What the renderer's pages draw GpuSurfaces with, as the last page to
    /// settle it reported: `None` before any has, and from a renderer that
    /// shows no pages.
    pub fn page_graphics<G>(&self, mut arm: impl FnMut(Canceler) -> Result<G, GuiError>) -> Result<Option<WebGraphics>, GuiError> {
        match self.inner.request(|request, reply| ToHelper::PageGraphics { request, reply }, &mut arm)? {
            FromHelper::PageGraphics { graphics, .. } => Ok(graphics),
            other => Err(unexpected("PageGraphics", other)),
        }
    }

    /// Shows a native dialog titled `title` and returns how the person
    /// answered it once it closes. A wait ended through the arm's
    /// [`Canceler`] closes the dialog the platform's own way and returns
    /// [`GuiError::Canceled`].
    pub fn dialog<G>(&self, title: &str, kind: DialogKind, mut arm: impl FnMut(Canceler) -> Result<G, GuiError>) -> Result<DialogAnswer, GuiError> {
        let mut asked = None;
        let title = title.to_string();
        let outcome = self.inner.request(
            |request, reply| {
                asked = Some(request);
                ToHelper::Dialog { request, reply, title, kind }
            },
            &mut arm,
        );
        match outcome {
            Ok(FromHelper::DialogResult { outcome: Ok(answer), .. }) => Ok(answer),
            Ok(FromHelper::DialogResult { outcome: Err(DialogFailure::Failed(why)), .. }) => Err(GuiError::Refused(why)),
            Ok(FromHelper::DialogResult { outcome: Err(DialogFailure::NoNative(why)), .. }) => Err(GuiError::NoNativeDialog(why)),
            Ok(other) => Err(unexpected("Dialog", other)),
            Err(GuiError::Canceled) => match asked {
                Some(dialog) => {
                    self.inner.send(ToHelper::CloseDialog { dialog })?;
                    Err(GuiError::Canceled)
                }
                None => Err(GuiError::Canceled),
            },
            Err(e) => Err(e),
        }
    }

    /// The dialogs the renderer has open.
    pub fn dialogs<G>(&self, mut arm: impl FnMut(Canceler) -> Result<G, GuiError>) -> Result<Vec<DialogSummary>, GuiError> {
        match self.inner.request(|request, reply| ToHelper::ListDialogs { request, reply }, &mut arm)? {
            FromHelper::Dialogs { dialogs, .. } => Ok(dialogs),
            other => Err(unexpected("ListDialogs", other)),
        }
    }

    /// Closes the dialog `dialog` with `answer` as its command's result, as
    /// a person answering it would.
    pub fn answer_dialog<G>(&self, dialog: RequestId, answer: DialogAnswer, mut arm: impl FnMut(Canceler) -> Result<G, GuiError>) -> Result<(), GuiError> {
        match self.inner.request(|request, reply| ToHelper::AnswerDialog { request, reply, dialog, answer }, &mut arm)? {
            FromHelper::TestEventDone { outcome: Ok(_), .. } => Ok(()),
            FromHelper::TestEventDone { outcome: Err(why), .. } => Err(GuiError::Refused(why)),
            other => Err(unexpected("AnswerDialog", other)),
        }
    }

    /// Acts on the node named `name` in window `window` as a person would.
    /// The event reaches the window's reader; this returns the value the
    /// widget took.
    pub fn test_event<G>(&self, window: WindowId, name: &str, kind: EventKind, value: Value, mut arm: impl FnMut(Canceler) -> Result<G, GuiError>) -> Result<Value, GuiError> {
        let name = name.to_string();
        match self.inner.request(|request, reply| ToHelper::TestEvent { request, reply, window, name, kind, value }, &mut arm)? {
            FromHelper::TestEventDone { outcome: Ok(v), .. } => Ok(v),
            FromHelper::TestEventDone { outcome: Err(why), .. } => Err(GuiError::Refused(why)),
            other => Err(unexpected("TestEvent", other)),
        }
    }

    /// Gives window `window` `steps` of a person's input, a frame apart,
    /// through its renderer's own input; returns once the frame after the
    /// last step has run, so a snapshot then shows what the input did.
    /// Each step has `time` to be taken and seen from when it begins, a wait
    /// its own seconds besides ([`proto::test_input::Step::allowed`]). The
    /// first step whose time runs out fails the input with
    /// [`GuiError::Late`], naming the step, and the renderer is told to drop
    /// the steps left, as it is when the wait is canceled.
    pub fn test_input<G>(&self, window: WindowId, steps: Vec<proto::test_input::Step>, time: Duration, mut arm: impl FnMut(Canceler) -> Result<G, GuiError>) -> Result<(), GuiError> {
        let mut pace = pace::InputPace::new(&steps, time, std::time::Instant::now())?;
        let mut asked = None;
        let answer = self.inner.request_paced(
            |request, reply| {
                asked = Some(request);
                ToHelper::TestInput { request, reply, window, steps }
            },
            &mut arm,
            &mut pace,
        );
        if let (Err(GuiError::Late(_) | GuiError::Canceled), Some(request)) = (&answer, asked)
            && let Err(e) = self.inner.send(ToHelper::StopInput { window, request })
        {
            eprintln!("PWRS-GUI: window {window} could not be told to drop the rest of input {request}: {e}");
        }
        match answer? {
            FromHelper::TestEventDone { outcome: Ok(_), .. } => Ok(()),
            FromHelper::TestEventDone { outcome: Err(why), .. } => Err(GuiError::Refused(why)),
            other => Err(unexpected("TestInput", other)),
        }
    }

    /// Window `window`'s tree and state and, with `pixels`, its rendered
    /// image or the image of the node named `name`.
    pub fn snapshot<G>(&self, window: WindowId, name: Option<String>, pixels: bool, mut arm: impl FnMut(Canceler) -> Result<G, GuiError>) -> Result<Snapshot, GuiError> {
        match self.inner.request(|request, reply| ToHelper::Snapshot { request, reply, window, name, pixels }, &mut arm)? {
            FromHelper::Snapshot { outcome: Ok(s), .. } => Ok(s),
            FromHelper::Snapshot { outcome: Err(why), .. } => Err(GuiError::Refused(why)),
            other => Err(unexpected("Snapshot", other)),
        }
    }

    /// Closes a window by number, from anywhere; the window's `Closed`
    /// reaches its reader.
    pub fn close_window(&self, window: WindowId) -> Result<(), GuiError> {
        self.inner.send(ToHelper::CloseWindow { window })
    }

    /// Closes a window by number as [`Session::close_window`] does, once the
    /// renderer has said it had that window open; refused, with the
    /// renderer's reason, when it had none.
    pub fn close_open_window<G>(&self, window: WindowId, mut arm: impl FnMut(Canceler) -> Result<G, GuiError>) -> Result<(), GuiError> {
        match self.inner.request(|request, reply| ToHelper::CloseOpen { request, reply, window }, &mut arm)? {
            FromHelper::CloseDone { outcome: Ok(()), .. } => Ok(()),
            FromHelper::CloseDone { outcome: Err(why), .. } => Err(GuiError::Refused(why)),
            other => Err(unexpected("CloseOpen", other)),
        }
    }

    /// Gives node `node` of a window by number the keyboard focus, once the
    /// renderer has said it gave it; refused, with the renderer's reason,
    /// for a window it has not open, a node the window lacks, a node that
    /// takes no focus or is disabled, and one the window has not drawn.
    pub fn focus_node<G>(&self, window: WindowId, node: NodeId, mut arm: impl FnMut(Canceler) -> Result<G, GuiError>) -> Result<(), GuiError> {
        match self.inner.request(|request, reply| ToHelper::FocusNode { request, reply, window, node }, &mut arm)? {
            FromHelper::FocusDone { outcome: Ok(()), .. } => Ok(()),
            FromHelper::FocusDone { outcome: Err(why), .. } => Err(GuiError::Refused(why)),
            other => Err(unexpected("FocusNode", other)),
        }
    }

    /// Changes a window's tree by number, from anywhere.
    pub fn patch(&self, window: WindowId, ops: Vec<PatchOp>) -> Result<(), GuiError> {
        self.inner.send(ToHelper::Patch { window, ops })
    }

    /// Table rows for node `node` of a window by number, from anywhere,
    /// starting at row `start`; `added` for rows Add-GuiItem added, which a
    /// table showing its last row follows.
    pub fn rows(&self, window: WindowId, node: NodeId, start: u64, rows: Vec<Row>, added: bool) -> Result<(), GuiError> {
        self.inner.send(ToHelper::Rows { window, node, start, rows, added })
    }

    /// Gives the page of window `window`, by number, from anywhere, a
    /// value, as JSON, for its `window.pwrs.on` handlers, waiting for the
    /// renderer to take it: refused for a window that is not open or shows
    /// no page of its own, and for text that is not JSON.
    pub fn bridge<G>(&self, window: WindowId, json: String, mut arm: impl FnMut(Canceler) -> Result<G, GuiError>) -> Result<(), GuiError> {
        match self.inner.request(|request, reply| ToHelper::Bridge { request, reply, window, json }, &mut arm)? {
            FromHelper::BridgeDone { outcome: Ok(()), .. } => Ok(()),
            FromHelper::BridgeDone { outcome: Err(why), .. } => Err(GuiError::Refused(why)),
            other => Err(unexpected("Bridge", other)),
        }
    }

    /// Replies to the `window.pwrs.send` numbered `call` of window
    /// `window`'s page, by number, from anywhere, with a value, as JSON.
    pub fn reply(&self, window: WindowId, call: u32, json: String) -> Result<(), GuiError> {
        self.inner.send(ToHelper::BridgeReply { window, call, json })
    }

    /// Answers the `window.pwrs.call` numbered `call` of window `window`'s
    /// page, by number, from anywhere: its output as JSON, which the call's
    /// Promise resolves with, or as JSON an object whose `message` says why
    /// it failed, which the Promise rejects with.
    pub fn answer_call(&self, window: WindowId, call: u32, outcome: Result<String, String>) -> Result<(), GuiError> {
        self.inner.send(ToHelper::CallReply { window, call, outcome })
    }

    /// Ends the channel window `window`'s page has open under `name`, by
    /// number, from anywhere ([`ToHelper::CloseChannel`]).
    pub fn close_channel(&self, window: WindowId, name: String) -> Result<(), GuiError> {
        self.inner.send(ToHelper::CloseChannel { window, name })
    }

    /// Gives file chooser `node` of window `window`, by number, from
    /// anywhere, the answer of the dialog [`WindowEvent::PickerWanted`]
    /// requested, which it takes as a person's choice.
    pub fn answer_picker(&self, window: WindowId, node: NodeId, outcome: Result<DialogAnswer, DialogFailure>) -> Result<(), GuiError> {
        self.inner.send(ToHelper::PickerAnswer { window, node, outcome })
    }

    /// Sends the renderer `Shutdown`, which closes every window and ends
    /// it. A helper also loses its stdin, which ends it on Linux, FreeBSD
    /// and macOS; an in-process renderer gives its UI thread up to the next
    /// session's start, which runs once this one's run has ended. A session
    /// taken through another module's entry leaves that module's renderer
    /// running; its windows close as they are dropped.
    pub fn shut_down(&self) {
        if let Link::Shared(_) = &self.inner.link {
            return;
        }
        if self.inner.death().is_none()
            && let Err(e) = self.inner.send(ToHelper::Shutdown)
        {
            eprintln!("PWRS-GUI: the renderer could not be told to end: {e}");
        }
        match &self.inner.link {
            Link::Helper(h) => h.close_stdin(),
            #[cfg(all(windows, feature = "in-process"))]
            Link::InProcess(l) => l.release(),
            Link::Shared(_) => {}
        }
    }
}

fn wait_for<G>(starting: Starting, arm: &mut impl FnMut(Canceler) -> Result<G, GuiError>) -> Result<Session, GuiError> {
    let guard = arm(starting.canceler())?;
    let session = starting.wait();
    drop(guard);
    session
}

#[cfg(all(windows, feature = "in-process"))]
fn start_in_process<G>(options: &StartOptions, arm: &mut impl FnMut(Canceler) -> Result<G, GuiError>) -> Result<Session, GuiError> {
    wait_for(inprocess::begin(options)?, arm)
}

/// Starts a helper and waits for its `Hello`. On Linux and FreeBSD a web
/// helper starts from the WebKitGTK add-on module when one is installed, so
/// the engine is the one the add-on pins, and from the system's WebKitGTK
/// otherwise; the add-on found is the one every later web helper of the
/// process starts from.
fn start_helper<G>(options: &StartOptions, arm: &mut impl FnMut(Canceler) -> Result<G, GuiError>) -> Result<Session, GuiError> {
    #[cfg(any(target_os = "linux", target_os = "freebsd"))]
    if options.renderer == Renderer::Html && webkit::in_use().is_none() {
        match webkit::find(options) {
            Ok(Some(addon)) => {
                let addon = webkit::adopt(addon);
                proto::trace::line(|| format!("client: the web helper starts from the WebKitGTK add-on at {}", addon.root().display()));
            }
            Ok(None) => proto::trace::line(|| String::from("client: no WebKitGTK add-on is installed, so the web helper starts on the system's WebKitGTK")),
            Err(e) => return Err(GuiError::Start(format!("the WebKitGTK add-on could not be used: {e}"))),
        }
    }
    helper::begin(options).and_then(|s| wait_for(s, arm))
}

#[cfg(not(all(windows, feature = "in-process")))]
fn start_in_process<G>(_options: &StartOptions, _arm: &mut impl FnMut(Canceler) -> Result<G, GuiError>) -> Result<Session, GuiError> {
    Err(GuiError::Start(String::from("drawing in-process is Windows only, in a build with the in-process feature; use the helper")))
}

fn unexpected(asked: &str, got: FromHelper) -> GuiError {
    GuiError::Transport(format!("{asked} got the reply {got:?}"))
}

/// What happened to a window.
#[derive(Clone, Debug, PartialEq)]
pub enum WindowEvent {
    /// Its first frame was drawn.
    Opened,
    /// A widget raised an event; `at` is when the renderer took the input,
    /// in nanoseconds on [`proto::clock::now_ns`].
    Event { node: NodeId, kind: EventKind, value: Value, at: u64 },
    Closed(CloseReason),
    /// The window's page sent a value with `window.pwrs.send`, as JSON;
    /// [`Window::reply`] with `call` replies to it.
    Message { call: u32, json: String },
    /// The window's page asked with `window.pwrs.call` for what the window
    /// allows under `name`, with `json` its arguments, `null` when it gave
    /// none; [`Window::answer_call`] with `call` answers it.
    Call { call: u32, name: String, json: String },
    /// The window's page asked with `window.pwrs.window.open(options)` for
    /// another window of its app, with `json` the options;
    /// [`Window::answer_call`] with `call` answers it, with the window's
    /// `id` and `title` or why it did not open.
    Open { call: u32, json: String },
    /// The window's page closed it with `window.pwrs.close(value)`; the
    /// value as JSON, `null` when it gave none.
    PageClosed(String),
    /// Something the renderer could not do in this window.
    Problem(String),
    /// File chooser `node` needs a dialog its renderer cannot show, the
    /// system having none of its kind: the command has the egui renderer
    /// draw it, and sends the person's answer back with
    /// [`Session::answer_picker`].
    PickerWanted { node: NodeId, title: String, kind: DialogKind },
    /// A file the window watches changed ([`Window::watch_files`]): `file`
    /// is its place among the files watched, from 0, `path` its full path,
    /// and `length` its length in bytes, none when it is gone.
    FileChanged { file: usize, path: PathBuf, length: Option<u64> },
}

/// An open window and the events it raises.
pub struct Window {
    session: Session,
    id: WindowId,
    rx: EventRx,
    canceler: Canceler,
    open: bool,
    /// The files the window watches, whose watch ends with the window.
    files: Option<file_watch::FileWatch>,
}

impl Window {
    pub fn id(&self) -> WindowId {
        self.id
    }

    pub fn session(&self) -> &Session {
        &self.session
    }

    /// Ends a wait in [`next_event`](Self::next_event) from any thread. The
    /// window reads nothing more afterwards.
    pub fn canceler(&self) -> Canceler {
        self.canceler.clone()
    }

    /// The next thing that happened to the window, waiting with no time
    /// limit.
    pub fn next_event(&mut self) -> Result<WindowEvent, GuiError> {
        let msg = match &mut self.rx {
            EventRx::Ring(rx) => match rx.recv::<FromHelper>() {
                Ok(m) => m,
                Err(TransportError::Closed { .. }) => return Err(self.session.inner.woken(&self.canceler)),
                Err(e) => return Err(self.session.inner.transport_failed(e)),
            },
            #[cfg(all(windows, feature = "in-process"))]
            EventRx::Channel(rx, _) => match rx.recv() {
                Ok(Delivery::Msg(m)) => *m,
                Ok(Delivery::Wake) => return Err(self.session.inner.woken(&self.canceler)),
                Err(e) => return Err(GuiError::Gone(format!("window {}'s channel closed: {e}", self.id))),
            },
        };
        Ok(self.event_of(msg))
    }

    /// The next thing that happened to the window, if anything has.
    pub fn try_next_event(&mut self) -> Result<Option<WindowEvent>, GuiError> {
        let msg = match &mut self.rx {
            EventRx::Ring(rx) => match rx.try_recv::<FromHelper>() {
                Ok(m) => m,
                Err(e) => return Err(self.session.inner.transport_failed(e)),
            },
            #[cfg(all(windows, feature = "in-process"))]
            EventRx::Channel(rx, _) => match rx.try_recv() {
                Ok(Delivery::Msg(m)) => Some(*m),
                Ok(Delivery::Wake) => return Err(self.session.inner.woken(&self.canceler)),
                Err(std::sync::mpsc::TryRecvError::Empty) => None,
                Err(e) => return Err(GuiError::Gone(format!("window {}'s channel closed: {e}", self.id))),
            },
        };
        Ok(msg.map(|m| self.event_of(m)))
    }

    fn event_of(&mut self, msg: FromHelper) -> WindowEvent {
        match msg {
            FromHelper::Opened { .. } => WindowEvent::Opened,
            FromHelper::Event { node, kind, value, at, .. } => WindowEvent::Event { node, kind, value, at },
            FromHelper::Closed { reason, .. } => {
                self.open = false;
                WindowEvent::Closed(reason)
            }
            FromHelper::BridgeMessage { call, json, .. } => WindowEvent::Message { call, json },
            FromHelper::PageCall { call, name, json, .. } => WindowEvent::Call { call, name, json },
            FromHelper::PageOpen { call, json, .. } => WindowEvent::Open { call, json },
            FromHelper::PageClosed { json, .. } => {
                self.open = false;
                WindowEvent::PageClosed(json)
            }
            FromHelper::Error { context, text, .. } => WindowEvent::Problem(format!("{context}: {text}")),
            FromHelper::PickerWanted { node, title, kind, .. } => WindowEvent::PickerWanted { node, title, kind },
            FromHelper::FileChanged { file, length, .. } => {
                let file = file as usize;
                match self.files.as_ref().and_then(|w| w.path(file).map(|p| (w, p.to_path_buf()))) {
                    Some((watch, path)) => {
                        watch.taken(file);
                        WindowEvent::FileChanged { file, path, length }
                    }
                    None => WindowEvent::Problem(format!("window {} was told of a change to file {file}, which it does not watch", self.id)),
                }
            }
            other => WindowEvent::Problem(format!("window {} was sent {other:?}", self.id)),
        }
    }

    /// Watches `files`, each a full path, for as long as the window is
    /// open: a change to one comes among its events as
    /// [`WindowEvent::FileChanged`], one at a time for each file, the next
    /// sent once this one is taken. The system's notifications tell of the
    /// changes, and on Windows a comparison of each file once a second as
    /// well, since NTFS reports no write a writer leaves in the system's
    /// cache; with `every`, the files are compared at that pace instead, on
    /// any system. Fails when a file's folder cannot be watched, or when
    /// the window watches files already.
    pub fn watch_files(&mut self, files: Vec<PathBuf>, every: Option<Duration>) -> Result<(), GuiError> {
        if self.files.is_some() {
            return Err(GuiError::Watch(format!("window {} watches files already", self.id)));
        }
        let post = match &self.rx {
            EventRx::Ring(rx) => file_watch::Post::Ring(rx.ring().sender().map_err(|e| GuiError::Transport(e.to_string()))?),
            #[cfg(all(windows, feature = "in-process"))]
            EventRx::Channel(_, tx) => file_watch::Post::Channel(tx.clone()),
        };
        self.files = Some(file_watch::FileWatch::start(self.id, files, every, post)?);
        Ok(())
    }

    /// Table rows for node `node`, starting at row `start`; `added` for rows
    /// Add-GuiItem added, which a table showing its last row follows.
    pub fn rows(&self, node: NodeId, start: u64, rows: Vec<Row>, added: bool) -> Result<(), GuiError> {
        self.session.inner.send(ToHelper::Rows { window: self.id, node, start, rows, added })
    }

    pub fn patch(&self, ops: Vec<PatchOp>) -> Result<(), GuiError> {
        self.session.inner.send(ToHelper::Patch { window: self.id, ops })
    }

    pub fn focus(&self) -> Result<(), GuiError> {
        self.session.inner.send(ToHelper::Focus { window: self.id })
    }

    /// Gives the window's page a value, as JSON, for its `window.pwrs.on`
    /// handlers, waiting for the renderer to take it ([`Session::bridge`]).
    pub fn bridge<G>(&self, json: String, arm: impl FnMut(Canceler) -> Result<G, GuiError>) -> Result<(), GuiError> {
        self.session.bridge(self.id, json, arm)
    }

    /// Replies to the page's `window.pwrs.send` numbered `call` with a
    /// value, as JSON.
    pub fn reply(&self, call: u32, json: String) -> Result<(), GuiError> {
        self.session.reply(self.id, call, json)
    }

    /// Answers the page's `window.pwrs.call` numbered `call`
    /// ([`Session::answer_call`]).
    pub fn answer_call(&self, call: u32, outcome: Result<String, String>) -> Result<(), GuiError> {
        self.session.answer_call(self.id, call, outcome)
    }

    /// Closes the window; one the person has closed already is left as it
    /// is.
    pub fn close(mut self) -> Result<(), GuiError> {
        if !self.open {
            return Ok(());
        }
        self.open = false;
        self.end()
    }

    /// Leaves the window open with nothing reading its events.
    pub fn detach(mut self) -> Result<(), GuiError> {
        self.open = false;
        self.session.inner.send(ToHelper::Detach { window: self.id })
    }

    /// Closes the window from its own reader, which is going away: the
    /// renderer forgets the window's ring first, so the window's `Closed`
    /// is not sent to a reader that no longer exists.
    fn end(&self) -> Result<(), GuiError> {
        self.session.inner.send(ToHelper::Detach { window: self.id })?;
        self.session.inner.send(ToHelper::CloseWindow { window: self.id })
    }
}

impl Drop for Window {
    fn drop(&mut self) {
        if self.open
            && self.session.is_alive()
            && let Err(e) = self.end()
        {
            eprintln!("PWRS-GUI: window {} could not be closed: {e}", self.id);
        }
    }
}

/// A start under way, which every other caller of
/// [`SessionSlot::get_or_start`] waits on rather than starting a second
/// renderer.
struct Pending {
    outcome: OnceLock<Settled>,
    waiters: Watches,
}

/// What a start came to.
enum Settled {
    Started(Arc<Inner>),
    /// Its caller canceled it, which says nothing about whether another
    /// caller's start would succeed.
    Canceled,
    Failed(String),
}

impl Pending {
    fn settle(&self, outcome: Settled) {
        if self.outcome.set(outcome).is_ok() {
            self.waiters.wake_all();
        }
    }
}

/// The session a process or a module shares, read without a lock. One
/// caller at a time starts it; the others wait on that start.
pub struct SessionSlot {
    current: ArcSwapOption<Inner>,
    pending: ArcSwapOption<Pending>,
}

impl SessionSlot {
    pub const fn new() -> SessionSlot {
        SessionSlot { current: ArcSwapOption::const_empty(), pending: ArcSwapOption::const_empty() }
    }

    /// The live session in the slot, or one started as `options` says.
    /// While another caller is starting one, this waits for that start
    /// with no time limit: its failure is this caller's too, and when it is
    /// canceled this caller starts one itself. `arm` is
    /// [`Session::start`]'s. Returns the session and, for the caller whose
    /// start drew in-process in Auto mode, why the helper could not start.
    pub fn get_or_start<G>(&self, options: &StartOptions, mut arm: impl FnMut(Canceler) -> Result<G, GuiError>) -> Result<(Session, Option<String>), GuiError> {
        loop {
            if let Some(live) = self.current() {
                return Ok((live, None));
            }
            let seen = self.pending.load_full();
            match seen {
                Some(other) => {
                    if let Some(live) = self.wait_on(&other, &mut arm)? {
                        return Ok((live, None));
                    }
                    self.clear_pending(&other);
                }
                None => {
                    let mine = Arc::new(Pending { outcome: OnceLock::new(), waiters: Watches::new() });
                    let before = self.pending.compare_and_swap(&seen, Some(Arc::clone(&mine)));
                    if before.is_some() {
                        continue;
                    }
                    let started = match self.current() {
                        Some(live) => Ok((live, None)),
                        None => Session::start(options, &mut arm).map(|(s, why)| (self.install(s), why)),
                    };
                    match &started {
                        Ok((s, _)) => mine.settle(Settled::Started(Arc::clone(&s.inner))),
                        Err(GuiError::Canceled) => mine.settle(Settled::Canceled),
                        Err(e) => mine.settle(Settled::Failed(e.to_string())),
                    }
                    self.clear_pending(&mine);
                    return started;
                }
            }
        }
    }

    /// Waits for another caller's start to settle: its session when it
    /// started one that is still alive, its failure when it failed, and
    /// `None` when it was canceled or its session has already died.
    fn wait_on<G>(&self, other: &Arc<Pending>, arm: &mut impl FnMut(Canceler) -> Result<G, GuiError>) -> Result<Option<Session>, GuiError> {
        let parked = Arc::new(watch::ParkWake::here());
        let wake: Arc<dyn watch::Wake> = parked.clone();
        other.waiters.add(&wake);
        if other.outcome.get().is_none() {
            let canceler = Canceler::new(Arc::clone(&wake));
            let guard = arm(canceler.clone())?;
            parked.wait();
            drop(guard);
            if other.outcome.get().is_none() && canceler.is_canceled() {
                return Err(GuiError::Canceled);
            }
        }
        match other.outcome.get() {
            Some(Settled::Started(inner)) if inner.death().is_none() => Ok(Some(Session { inner: Arc::clone(inner) })),
            Some(Settled::Failed(why)) => Err(GuiError::Start(format!("another command's start of the renderer failed: {why}"))),
            Some(Settled::Started(_)) | Some(Settled::Canceled) | None => Ok(None),
        }
    }

    /// Clears the pending start when it is still `start`.
    fn clear_pending(&self, start: &Arc<Pending>) {
        let mine = Some(Arc::clone(start));
        let before = self.pending.compare_and_swap(&mine, None);
        drop(before);
    }

    /// The live session in the slot.
    pub fn current(&self) -> Option<Session> {
        self.current.load_full().filter(|i| i.death().is_none()).map(|inner| Session { inner })
    }

    /// Installs `session` unless a live one is already there, and returns
    /// whichever is in the slot; a session that lost the race is shut down.
    pub fn install(&self, session: Session) -> Session {
        loop {
            let seen = self.current.load_full();
            if let Some(live) = &seen
                && live.death().is_none()
            {
                session.shut_down();
                return Session { inner: Arc::clone(live) };
            }
            let before = self.current.compare_and_swap(&seen, Some(Arc::clone(&session.inner)));
            let same = match (&*before, &seen) {
                (Some(a), Some(b)) => Arc::ptr_eq(a, b),
                (None, None) => true,
                _ => false,
            };
            if same {
                return session;
            }
        }
    }

    /// Empties the slot, returning what it had.
    pub fn take(&self) -> Option<Session> {
        self.current.swap(None).map(|inner| Session { inner })
    }
}

impl Default for SessionSlot {
    fn default() -> Self {
        SessionSlot::new()
    }
}
