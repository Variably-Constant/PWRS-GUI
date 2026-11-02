//! Windows' in-process path: the renderer runs on a UI thread of this
//! process, and messages move to and from it over in-memory channels as
//! values, never encoded.
//!
//! A loaded copy of this code starts one UI thread per renderer, on first
//! use, which lasts for the life of the process: winit lets a copy build
//! one event loop ever, and eframe runs it again only on the thread that
//! built it. tao builds a loop for each run. Between sessions a thread
//! waits for its renderer's next run.

use crate::pace::Pace;
use crate::watch::{Canceler, Wake};
use crate::{Delivery, EventRx, GuiError, Inner, Link, Renderer, StartOptions, StartRx, Starting, WebViewData};
use pwrs_gui_proto::outbound::RingOutlets;
use pwrs_gui_proto::renderer::{Inbound, Link as RendererLink, Sink, WakeLoop};
use pwrs_gui_proto::transport::{self, Role};
use pwrs_gui_proto::{FromHelper, Node, NodeId, RequestId, RingName, Row, ToHelper, WindowId, WindowOptions};
use std::any::Any;
use std::collections::HashMap;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, OnceLock};
use std::time::Instant;

/// The in-memory link to the renderer on this copy's UI thread.
pub(crate) struct InProcessLink {
    /// `None` once the link is dropped, which the renderer reads as its
    /// session leaving.
    inbox: Option<mpsc::Sender<Inbound>>,
    /// Wakes the renderer's loop, once the renderer has given its wake.
    wake: Arc<OnceLock<WakeLoop>>,
    /// The UI thread the renderer runs on.
    thread: &'static UiThread,
    /// The run of the renderer this session started.
    run: u64,
}

impl InProcessLink {
    /// Gives the renderer a message and wakes its loop to read it.
    pub(crate) fn send(&self, msg: ToHelper, attach: Option<Box<dyn Any + Send>>) -> Result<(), GuiError> {
        let Some(inbox) = &self.inbox else {
            return Err(GuiError::Gone(format!("the in-process link is closed, so {msg:?} was not delivered")));
        };
        if let Err(e) = inbox.send(Inbound { msg, attach }) {
            return Err(GuiError::Gone(format!("the in-process renderer has stopped taking messages, so {:?} was not delivered", e.0.msg)));
        }
        self.wake_loop();
        Ok(())
    }

    /// A message sent before the renderer gives its wake is read on its
    /// first frame.
    fn wake_loop(&self) {
        if let Some(w) = self.wake.get() {
            w();
        }
    }

    /// Gives the UI thread up to the next session: this one has been told
    /// to end, and a start from now on runs after its renderer stops.
    pub(crate) fn release(&self) {
        self.thread.release(self.run);
    }
}

impl Drop for InProcessLink {
    /// Closes the renderer's inbox, then wakes its loop so it reads the
    /// close and ends the run.
    fn drop(&mut self) {
        drop(self.inbox.take());
        self.wake_loop();
        self.thread.release(self.run);
    }
}

/// Ends a wait on an in-memory channel by sending it a wake.
pub(crate) struct ChannelWake(pub(crate) mpsc::Sender<Delivery>);

impl Wake for ChannelWake {
    fn wake(&self) {
        if let Err(e) = self.0.send(Delivery::Wake) {
            // The receiver is gone, so no wait remains to end.
            drop(e.0);
        }
    }
}

/// Where the in-process renderer's messages go: the start channel, the
/// channels this session attached to its windows and requests, and, for a
/// module that took the session through the native entry, that module's
/// own rings, opened by name.
struct ChannelSink {
    start: mpsc::Sender<Delivery>,
    channels: HashMap<String, mpsc::Sender<Delivery>>,
    rings: RingOutlets,
}

/// Gives `msg` to the command reading the channel. A channel whose reader
/// has gone belongs to a command that has ended, its window closed or its
/// wait stopped, so a message still on its way, such as a window's
/// `Opened` racing its command's Ctrl+C, has no one to tell; the renderer
/// forgets the window or request when that command's close reaches it.
fn deliver(tx: &mpsc::Sender<Delivery>, msg: FromHelper) {
    match tx.send(Delivery::Msg(Box::new(msg))) {
        Ok(()) => {}
        Err(mpsc::SendError(_after_its_reader_ended)) => {}
    }
}

impl ChannelSink {
    fn send_ring(&mut self, ring: &str, role: Role, msg: FromHelper) {
        if let Err(e) = self.rings.send(ring, role, &msg) {
            eprintln!("PWRS-GUI: {ring} did not take this ({e}): {msg:?}");
        }
    }
}

impl Sink for ChannelSink {
    fn attach(&mut self, ring: &str, attach: Box<dyn Any + Send>) {
        match attach.downcast::<mpsc::Sender<Delivery>>() {
            Ok(tx) => {
                self.channels.insert(ring.to_string(), *tx);
            }
            Err(other) => eprintln!("PWRS-GUI: {ring} came with an attachment that is not a channel ({:?})", (*other).type_id()),
        }
    }

    fn hello(&mut self, msg: FromHelper) {
        deliver(&self.start, msg);
    }

    fn window(&mut self, ring: &str, msg: FromHelper) {
        match self.channels.get(ring) {
            Some(tx) => deliver(tx, msg),
            None => self.send_ring(ring, Role::Window, msg),
        }
    }

    /// A message for the request whose channel `ring` names. The reply that
    /// ends the request takes its channel with it; a report on the request
    /// under way, as a test input's step begun, leaves it for what follows.
    fn reply(&mut self, ring: &str, msg: FromHelper) {
        let channel = if msg.ends_request() { self.channels.remove(ring) } else { self.channels.get(ring).cloned() };
        match channel {
            Some(tx) => deliver(&tx, msg),
            None => self.send_ring(ring, Role::Reply, msg),
        }
    }

    fn forget(&mut self, ring: &str) {
        self.channels.remove(ring);
        self.rings.forget(ring);
    }

    fn log(&mut self, msg: FromHelper) {
        eprintln!("PWRS-GUI: {}", msg.log_line());
    }
}

/// A renderer run with its settings, on the UI thread that takes it.
type Render = Box<dyn FnOnce(RendererLink) -> Result<(), String> + Send>;

/// One run of a renderer for one session.
struct UiRun {
    link: RendererLink,
    render: Render,
    done: Box<dyn FnOnce(Result<(), String>) + Send>,
}

/// A renderer's UI thread, started on first use and lasting for the life of
/// the process, and the run whose session has it.
pub(crate) struct UiThread {
    name: &'static str,
    runs: OnceLock<Result<mpsc::Sender<UiRun>, String>>,
    /// The run whose session has the thread and has not been told to end;
    /// 0 when none. The thread runs one renderer at a time, so a start
    /// while a live session has it would wait for that session to end, and
    /// is refused instead. A start after that session was told to end
    /// queues behind its renderer, which is about to stop.
    live: AtomicU64,
}

static EGUI_THREAD: UiThread = UiThread::new("pwrs-gui-egui-ui");
static WEB_THREAD: UiThread = UiThread::new("pwrs-gui-web-ui");

/// Run numbers, from 1, shared by every renderer's thread.
static NEXT_RUN: AtomicU64 = AtomicU64::new(1);

impl UiThread {
    const fn new(name: &'static str) -> UiThread {
        UiThread { name, runs: OnceLock::new(), live: AtomicU64::new(0) }
    }

    fn of(renderer: Renderer) -> &'static UiThread {
        match renderer {
            Renderer::Egui => &EGUI_THREAD,
            Renderer::Html => &WEB_THREAD,
        }
    }

    /// Gives the thread up for `run`, returning whether `run` had it; a
    /// later run that already has it goes on with it.
    fn release(&self, run: u64) -> bool {
        self.live.compare_exchange(run, 0, Ordering::AcqRel, Ordering::Acquire).is_ok()
    }

    fn sender(&self) -> Result<mpsc::Sender<UiRun>, GuiError> {
        match self.runs.get_or_init(|| self.spawn()) {
            Ok(tx) => Ok(tx.clone()),
            Err(e) => Err(GuiError::Start(e.clone())),
        }
    }

    fn spawn(&self) -> Result<mpsc::Sender<UiRun>, String> {
        let (tx, rx) = mpsc::channel::<UiRun>();
        std::thread::Builder::new()
            .name(String::from(self.name))
            .spawn(move || ui_loop(rx))
            .map_err(|e| format!("the UI thread cannot start: {e}"))?;
        Ok(tx)
    }
}

/// A UI thread: enters a single-threaded apartment once, then runs each
/// session's renderer in turn.
fn ui_loop(runs: mpsc::Receiver<UiRun>) {
    let apartment = enter_sta();
    for run in runs.iter() {
        let UiRun { link, render, done } = run;
        let result = match &apartment {
            Err(e) => Err(e.clone()),
            Ok(()) => match catch_unwind(AssertUnwindSafe(|| render(link))) {
                Ok(r) => r,
                Err(payload) => Err(format!("the in-process renderer panicked: {}", panic_text(payload))),
            },
        };
        done(result);
    }
}

fn panic_text(payload: Box<dyn Any + Send>) -> String {
    if let Some(s) = payload.downcast_ref::<&str>() {
        (*s).to_string()
    } else if let Some(s) = payload.downcast_ref::<String>() {
        s.clone()
    } else {
        String::from("a panic with no message")
    }
}

/// Makes this thread single-threaded for COM, as winit's drag and drop and
/// WebView2 need; the thread stays in the apartment for its life.
fn enter_sta() -> Result<(), String> {
    use windows_sys::Win32::System::Com::{COINIT_APARTMENTTHREADED, CoInitializeEx};
    let hr = unsafe { CoInitializeEx(std::ptr::null(), COINIT_APARTMENTTHREADED as u32) };
    if hr < 0 {
        return Err(format!("the UI thread cannot enter a single-threaded apartment: HRESULT {hr:#010x}"));
    }
    Ok(())
}

/// Starts the renderer `options` names on this copy's UI thread for it, for
/// a new session.
pub(crate) fn begin(options: &StartOptions) -> Result<Starting, GuiError> {
    let thread = UiThread::of(options.renderer);
    let run = NEXT_RUN.fetch_add(1, Ordering::Relaxed);
    if thread.live.compare_exchange(0, run, Ordering::AcqRel, Ordering::Acquire).is_err() {
        return Err(GuiError::Start(format!("the in-process {} renderer is already drawing another session's windows in this process", options.renderer.name())));
    }
    let begun = begin_run(options, thread, run);
    if begun.is_err() {
        thread.release(run);
    }
    begun
}

/// The run of the renderer `options` names, with its in-process settings.
fn render(options: &StartOptions) -> Result<Render, GuiError> {
    Ok(match options.renderer {
        Renderer::Egui => {
            let mesa = crate::mesa::find(options).map_err(GuiError::Start)?.map(|m| pwrs_gui_egui::Mesa { folder: m.folder, chosen: m.chosen });
            let settings = pwrs_gui_egui::Settings { adapter: options.adapter.clone(), low_power: options.low_power, any_thread: true, bring_to_front: true, mesa, fonts: options.fonts };
            Box::new(move |link| pwrs_gui_egui::run(link, settings))
        }
        Renderer::Html => {
            let data = match options.web_view {
                WebViewData::Shared => pwrs_gui_web::Data::Shared(options.web_view_data_folder.clone()),
                WebViewData::PerProcess => pwrs_gui_web::Data::PerProcess,
            };
            let found = crate::webview2::find(options).map_err(GuiError::Start)?;
            let runtimes = pwrs_gui_web::Runtimes { fixed: found.fixed, installed: found.installed };
            let settings = pwrs_gui_web::Settings { any_thread: true, data, bring_to_front: true, log: Vec::new(), runtimes, fonts: options.fonts };
            Box::new(move |link| pwrs_gui_web::run(link, settings))
        }
    })
}

fn begin_run(options: &StartOptions, thread: &'static UiThread, run: u64) -> Result<Starting, GuiError> {
    let render = render(options)?;
    let ui = thread.sender()?;
    let id = transport::session_id().map_err(|e| GuiError::Start(e.to_string()))?;
    let (inbox_tx, inbox_rx) = mpsc::channel::<Inbound>();
    let (start_tx, start_rx) = mpsc::channel::<Delivery>();
    let wake: Arc<OnceLock<WakeLoop>> = Arc::new(OnceLock::new());
    let link = InProcessLink { inbox: Some(inbox_tx), wake: Arc::clone(&wake), thread, run };
    let inner = Arc::new(Inner::new(id, Link::InProcess(link), options.renderer));
    let sink = ChannelSink { start: start_tx.clone(), channels: HashMap::new(), rings: RingOutlets::new() };
    let on_ready = Box::new(move |w: WakeLoop| {
        if wake.set(w).is_err() {
            eprintln!("PWRS-GUI: the in-process renderer reported its wake twice");
        }
    });
    let watched = Arc::downgrade(&inner);
    let done = Box::new(move |result: Result<(), String>| {
        thread.release(run);
        if let Some(inner) = watched.upgrade() {
            let reason = match result {
                Ok(()) => String::from("the in-process renderer stopped"),
                Err(e) => format!("the in-process renderer stopped: {e}"),
            };
            inner.die(reason);
        }
    });
    let run_number = run;
    let run = UiRun { link: RendererLink { inbox: inbox_rx, sink: Box::new(sink), on_ready }, render, done };
    pwrs_gui_proto::trace::line(|| format!("in-process: {} run {run_number} given to {}", options.renderer.name(), thread.name));
    if let Err(e) = ui.send(run) {
        return Err(GuiError::Start(format!("the UI thread has stopped and could not take the renderer: {e}")));
    }
    let wake: Arc<dyn Wake> = Arc::new(ChannelWake(start_tx));
    inner.watches.add(&wake);
    let canceler = Canceler::new(wake);
    if inner.death().is_some() {
        canceler.wake().wake();
    }
    Ok(Starting { inner, rx: StartRx::Channel(start_rx), canceler, settled: false })
}

/// Opens a window whose events arrive on a channel of its own, its tables
/// filled with `rows`; the window gets a sender of the channel too, for its
/// file watch.
pub(crate) fn open_window(inner: &Arc<Inner>, link: &InProcessLink, window: WindowId, options: WindowOptions, tree: Node, rows: Vec<(NodeId, Vec<Row>)>) -> Result<(EventRx, Arc<dyn Wake>), GuiError> {
    let name = transport::window_ring(&inner.id, window);
    let (tx, rx) = mpsc::channel::<Delivery>();
    let wake: Arc<dyn Wake> = Arc::new(ChannelWake(tx.clone()));
    inner.watches.add(&wake);
    if let Some(reason) = inner.death() {
        return Err(GuiError::Gone(reason));
    }
    let post = tx.clone();
    link.send(ToHelper::OpenWindow { window, ring: name, options, tree, rows }, Some(Box::new(tx)))?;
    Ok((EventRx::Channel(rx, post), wake))
}

/// Sends a request with a reply channel of its own and waits for the
/// reply as `pace` says: with no time limit, or up to the moment it gives,
/// which messages with the request's number may move.
pub(crate) fn request<G>(
    inner: &Arc<Inner>,
    link: &InProcessLink,
    build: impl FnOnce(RequestId, RingName) -> ToHelper,
    arm: &mut impl FnMut(Canceler) -> Result<G, GuiError>,
    pace: &mut impl Pace,
) -> Result<FromHelper, GuiError> {
    let request = inner.number();
    let name = transport::reply_ring(&inner.id, request);
    let (tx, rx) = mpsc::channel::<Delivery>();
    let wake: Arc<dyn Wake> = Arc::new(ChannelWake(tx.clone()));
    inner.watches.add(&wake);
    if let Some(reason) = inner.death() {
        return Err(GuiError::Gone(reason));
    }
    link.send(build(request, name.clone()), Some(Box::new(tx)))?;
    let canceler = Canceler::new(wake);
    let guard = arm(canceler.clone())?;
    let answer = loop {
        let next = match pace.due() {
            None => rx.recv().map(Some).map_err(|e| e.to_string()),
            Some(due) => match rx.recv_timeout(due.saturating_duration_since(Instant::now())) {
                Ok(delivered) => Ok(Some(delivered)),
                Err(mpsc::RecvTimeoutError::Timeout) => Ok(None),
                Err(e @ mpsc::RecvTimeoutError::Disconnected) => Err(e.to_string()),
            },
        };
        match next {
            Ok(Some(Delivery::Msg(msg))) if crate::reply_number(&msg) == Some(request) => {
                if pace.ends(&msg, Instant::now()) {
                    break Ok(*msg);
                }
            }
            Ok(Some(Delivery::Msg(other))) => eprintln!("PWRS-GUI: a reply for another request arrived on {name}: {other:?}"),
            Ok(Some(Delivery::Wake)) => break Err(inner.woken(&canceler)),
            Ok(None) => break Err(pace.late()),
            Err(e) => break Err(GuiError::Gone(format!("the reply channel {name} closed: {e}"))),
        }
    };
    drop(guard);
    if answer.is_err()
        && let Err(e) = link.send(ToHelper::ForgetReply { reply: name.clone() }, None)
    {
        eprintln!("PWRS-GUI: the renderer could not be told {name} is gone: {e}");
    }
    answer
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_step_report_leaves_its_request_s_channel_and_the_reply_takes_it() {
        let (start, _starts) = mpsc::channel();
        let mut sink = ChannelSink { start, channels: HashMap::new(), rings: RingOutlets::new() };
        let (tx, rx) = mpsc::channel::<Delivery>();
        sink.attach("reply-1", Box::new(tx));
        sink.reply("reply-1", FromHelper::InputStep { request: 1, step: 1 });
        sink.reply("reply-1", FromHelper::InputStep { request: 1, step: 2 });
        assert!(sink.channels.contains_key("reply-1"), "a step report leaves the channel");
        sink.reply("reply-1", FromHelper::TestEventDone { request: 1, outcome: Ok(pwrs_gui_proto::Value::Null) });
        assert!(!sink.channels.contains_key("reply-1"), "the reply that ends the request takes its channel");
        let delivered: Vec<FromHelper> = rx
            .try_iter()
            .map(|d| match d {
                Delivery::Msg(m) => *m,
                Delivery::Wake => panic!("a wake came where replies were awaited"),
            })
            .collect();
        assert_eq!(
            delivered,
            vec![
                FromHelper::InputStep { request: 1, step: 1 },
                FromHelper::InputStep { request: 1, step: 2 },
                FromHelper::TestEventDone { request: 1, outcome: Ok(pwrs_gui_proto::Value::Null) },
            ]
        );
    }
}
