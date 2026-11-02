//! The helper path: a helper process the session starts, and the rings it
//! talks over, which a session taken through another module's native
//! entry talks over too.
//!
//! Every thread that sends to a renderer has, in thread-local storage, its
//! own sender on the renderer's inbound ring and its own reply ring, so
//! no two threads share a producer lane and no lock is taken to send.

use crate::cell::TakeOnce;
use crate::entry::NativeEntry;
#[cfg(windows)]
use crate::platform;
use crate::pace::Pace;
use crate::platform::ExitWatch;
use crate::watch::{Canceler, RingWake, Wake};
use crate::{EventRx, GuiError, Inner, Link, Renderer, StartOptions, StartRx, Starting, WebViewData};
use pwrs_gui_proto::transport::{self, Receiver, Ring, Role, Sender};
use pwrs_gui_proto::{FromHelper, Node, NodeId, RequestId, RingName, Row, ToHelper, WindowId, WindowOptions};
use std::cell::RefCell;
use std::fs::File;
use std::path::PathBuf;
use std::process::{ChildStdin, Command, Stdio};
use std::sync::{Arc, OnceLock, Weak};
use std::time::Instant;

/// A running helper: its inbound ring, the stdin pipe that tells it this
/// process lives, its log, and the wait for its exit.
pub(crate) struct HelperLink {
    pub(crate) inbound: Arc<Ring>,
    stdin: TakeOnce<ChildStdin>,
    pub(crate) log: PathBuf,
    exit: OnceLock<ExitWatch>,
    #[cfg(windows)]
    job: TakeOnce<platform::Job>,
    /// The helper's process id, which each new window's right to the front
    /// is granted to.
    #[cfg(windows)]
    pid: u32,
}

impl HelperLink {
    /// Closes the helper's stdin; a helper on Linux, FreeBSD or macOS ends
    /// when it reads the end.
    pub(crate) fn close_stdin(&self) {
        drop(self.stdin.take());
    }

    /// Ends the helper now.
    pub(crate) fn kill(&self) {
        #[cfg(windows)]
        drop(self.job.take());
        #[cfg(unix)]
        if let Some(w) = self.exit.get() {
            w.kill();
        }
        self.close_stdin();
    }
}

fn start_failed(e: impl std::fmt::Display) -> GuiError {
    GuiError::Start(e.to_string())
}

/// Starts a helper and returns the start, whose `wait` reads its `Hello`.
pub(crate) fn begin(options: &StartOptions) -> Result<Starting, GuiError> {
    let path = options.helper.clone().map_err(GuiError::Start)?;
    let id = transport::session_id().map_err(start_failed)?;
    let start = Ring::create(&transport::start_ring(&id), Role::Start).map_err(start_failed)?;
    let start_rx = start.receiver().map_err(start_failed)?;
    let inbound = Ring::create(&transport::inbound_ring(&id), Role::Inbound).map_err(start_failed)?;
    std::fs::create_dir_all(&options.log_dir)
        .map_err(|e| GuiError::Start(format!("the folder for the helper's log, {}, cannot be created: {e}", options.log_dir.display())))?;
    let log = options.log_dir.join(format!("{id}.log"));
    let log_file = File::create(&log).map_err(|e| GuiError::Start(format!("the helper's log {} cannot be created: {e}", log.display())))?;
    let mut cmd = Command::new(&path);
    cmd.arg("--session").arg(&id).arg("--fonts").arg(options.fonts.arg()).stdin(Stdio::piped()).stdout(Stdio::null()).stderr(Stdio::from(log_file));
    if let Some(vars) = &options.environment {
        cmd.env_clear().envs(vars.iter().map(|(name, value)| (name, value)));
    }
    match options.renderer {
        Renderer::Egui => {
            if let Some(a) = &options.adapter {
                cmd.arg("--adapter").arg(a);
            }
            if options.low_power {
                cmd.arg("--low-power");
            }
            #[cfg(windows)]
            if let Some(mesa) = crate::mesa::find(options).map_err(GuiError::Start)? {
                cmd.arg(if mesa.chosen { "--mesa" } else { "--mesa-fallback" }).arg(&mesa.folder);
            }
        }
        Renderer::Html => {
            match (options.web_view, &options.web_view_data_folder) {
                (WebViewData::PerProcess, _) => {
                    cmd.arg("--web-view").arg("per-process");
                }
                (WebViewData::Shared, Some(folder)) => {
                    cmd.arg("--web-view-data-folder").arg(folder);
                }
                (WebViewData::Shared, None) => {}
            }
            #[cfg(any(target_os = "linux", target_os = "freebsd"))]
            if let Some(addon) = crate::webkit::in_use() {
                addon.apply(&mut cmd, options);
            }
            #[cfg(windows)]
            {
                let runtimes = crate::webview2::find(options).map_err(GuiError::Start)?;
                if let Some(fixed) = &runtimes.fixed {
                    cmd.arg("--webview2-runtime").arg(fixed);
                }
                for runtime in &runtimes.installed {
                    cmd.arg("--webview2-installed").arg(runtime);
                }
            }
        }
    }
    let mut child = cmd.spawn().map_err(|e| GuiError::Start(format!("{} cannot start: {e}", path.display())))?;
    let ended = |child: &mut std::process::Child| match child.kill() {
        Ok(()) => String::from("it was ended"),
        Err(e) => format!("it could not be ended either: {e}"),
    };
    #[cfg(windows)]
    let job = match platform::kill_on_close(&child, options.renderer == Renderer::Html && options.web_view == WebViewData::Shared) {
        Ok(j) => j,
        Err(e) => return Err(GuiError::Start(format!("the helper could not be tied to this process: {e}; {}", ended(&mut child)))),
    };
    let Some(stdin) = child.stdin.take() else {
        return Err(GuiError::Start(format!("the helper's stdin was not piped; {}", ended(&mut child))));
    };
    let link = HelperLink {
        inbound,
        stdin: TakeOnce::new(stdin),
        log: log.clone(),
        exit: OnceLock::new(),
        #[cfg(windows)]
        job: TakeOnce::new(job),
        #[cfg(windows)]
        pid: child.id(),
    };
    let inner = Arc::new(Inner::new(id, Link::Helper(link), options.renderer));
    #[cfg(windows)]
    let armed = ExitWatch::arm(&child, Arc::downgrade(&inner), log);
    #[cfg(unix)]
    let armed = ExitWatch::arm(child, Arc::downgrade(&inner), log);
    let watch = match armed {
        Ok(w) => w,
        Err(e) => {
            inner.kill();
            return Err(GuiError::Start(format!("the helper's exit cannot be waited for: {e}; it was ended")));
        }
    };
    if let Link::Helper(h) = &inner.link
        && h.exit.set(watch).is_err()
    {
        return Err(GuiError::Start(String::from("the wait for the helper's exit was armed twice")));
    }
    let wake: Arc<dyn Wake> = Arc::new(RingWake(start));
    inner.watches.add(&wake);
    let canceler = Canceler::new(wake);
    if inner.death().is_some() {
        canceler.wake().wake();
    }
    Ok(Starting { inner, rx: StartRx::Ring(start_rx), canceler, settled: false })
}

thread_local! {
    static LINKS: RefCell<Vec<ThreadLink>> = const { RefCell::new(Vec::new()) };
}

/// Where a session's messages go when its replies and window events come
/// back on rings.
pub(crate) enum Outlet<'a> {
    /// The renderer's inbound ring, through this thread's sender on it.
    Inbound(&'a Arc<Ring>),
    /// The in-process renderer another module offers, through its native
    /// entry.
    Entry(&'a NativeEntry),
}

/// How this thread sends to one session.
enum ThreadOut {
    Ring(Sender),
    Entry(NativeEntry, String),
}

impl ThreadOut {
    /// Sends `msg`. A taken session the offering module has let go of has
    /// ended, and is recorded as ended, so every wait on it wakes.
    fn send(&mut self, inner: &Inner, msg: &ToHelper) -> Result<(), GuiError> {
        match self {
            ThreadOut::Ring(sender) => sender.send(msg).map_err(|e| inner.transport_failed(e)),
            ThreadOut::Entry(entry, id) => match entry.send_message(id, msg) {
                Err(GuiError::Gone(reason)) => {
                    inner.die(reason.clone());
                    Err(GuiError::Gone(reason))
                }
                sent => sent,
            },
        }
    }
}

/// This thread's way to one session, and its reply ring.
struct ThreadLink {
    session: Weak<Inner>,
    out: ThreadOut,
    reply: Option<ReplyRing>,
}

/// A reply ring this thread made, and the wake that closes its handle.
struct ReplyRing {
    name: RingName,
    rx: Receiver,
    wake: Arc<dyn Wake>,
}

impl Drop for ThreadLink {
    /// Tells a live renderer this thread's reply ring is gone; a renderer
    /// that has ended let go of it already.
    fn drop(&mut self) {
        let live = self.session.upgrade().is_some_and(|s| s.death().is_none());
        if let Some(reply) = self.reply.take()
            && live
        {
            forget_reply(&mut self.out, reply);
        }
    }
}

/// Tells the renderer a reply ring is gone, without waiting: a renderer
/// that cannot take the message now is left with a handle on a ring nothing
/// reads.
fn forget_reply(out: &mut ThreadOut, reply: ReplyRing) {
    let msg = ToHelper::ForgetReply { reply: reply.name.clone() };
    match out {
        ThreadOut::Ring(sender) => match transport::encode(sender.ring(), &msg).and_then(|bytes| sender.try_send_encoded(&bytes)) {
            Ok(true) => {}
            Ok(false) => eprintln!("PWRS-GUI: the renderer's inbound ring was full, so the renderer still has a handle on the reply ring {}", reply.name),
            Err(e) => eprintln!("PWRS-GUI: the renderer could not be told the reply ring {} is gone: {e}", reply.name),
        },
        ThreadOut::Entry(entry, id) => {
            if let Err(e) = entry.send_message(id, &msg) {
                eprintln!("PWRS-GUI: the renderer could not be told the reply ring {} is gone: {e}", reply.name);
            }
        }
    }
}

/// Runs `f` on this thread's link to `inner`, making it on first use.
fn with_link<R>(inner: &Arc<Inner>, outlet: &Outlet<'_>, f: impl FnOnce(&mut ThreadLink) -> Result<R, GuiError>) -> Result<R, GuiError> {
    LINKS.with(|cell| {
        let mut links = cell.borrow_mut();
        links.retain(|l| l.session.upgrade().is_some_and(|s| s.death().is_none()));
        let me = Arc::as_ptr(inner);
        let at = match links.iter().position(|l| Weak::as_ptr(&l.session) == me) {
            Some(at) => at,
            None => {
                let out = match outlet {
                    Outlet::Inbound(ring) => ThreadOut::Ring(ring.sender().map_err(|e| GuiError::Transport(e.to_string()))?),
                    Outlet::Entry(entry) => ThreadOut::Entry(**entry, inner.id.clone()),
                };
                links.push(ThreadLink { session: Arc::downgrade(inner), out, reply: None });
                links.len() - 1
            }
        };
        f(&mut links[at])
    })
}

/// Sends a message to the renderer, waiting while its inbound ring is
/// full.
pub(crate) fn send(inner: &Arc<Inner>, outlet: &Outlet<'_>, msg: &ToHelper) -> Result<(), GuiError> {
    with_link(inner, outlet, |l| l.out.send(inner, msg))
}

/// Makes a window's ring and sends the renderer the window to open on it. On
/// Windows the helper is first granted the right to take the front, which a
/// helper started for an earlier window no longer has by Windows' rule for
/// a newly started process.
pub(crate) fn open_window(inner: &Arc<Inner>, outlet: &Outlet<'_>, window: WindowId, options: WindowOptions, tree: Node, rows: Vec<(NodeId, Vec<Row>)>) -> Result<(EventRx, Arc<dyn Wake>), GuiError> {
    let name = transport::window_ring(&inner.id, window);
    let ring = Ring::create(&name, Role::Window).map_err(|e| GuiError::Transport(e.to_string()))?;
    let rx = ring.receiver().map_err(|e| GuiError::Transport(e.to_string()))?;
    let wake: Arc<dyn Wake> = Arc::new(RingWake(ring));
    inner.watches.add(&wake);
    if let Some(reason) = inner.death() {
        return Err(GuiError::Gone(reason));
    }
    #[cfg(windows)]
    if let Link::Helper(h) = &inner.link {
        let granted = platform::allow_front(h.pid);
        pwrs_gui_proto::trace::line(|| format!("client: window {window}: the front granted to helper {}: {granted}", h.pid));
    }
    send(inner, outlet, &ToHelper::OpenWindow { window, ring: name, options, tree, rows })?;
    Ok((EventRx::Ring(rx), wake))
}

/// Sends a request and waits for the reply with its number on this
/// thread's reply ring, as `pace` says: with no time limit, or up to the
/// moment it gives, which messages with the request's number may move. A
/// wait that reaches that moment forgets the ring, so what the renderer
/// still sends for the request reaches no later request. `arm` gets the
/// wait's canceler before the wait begins.
pub(crate) fn request<G>(
    inner: &Arc<Inner>,
    outlet: &Outlet<'_>,
    build: impl FnOnce(RequestId, RingName) -> ToHelper,
    arm: &mut impl FnMut(Canceler) -> Result<G, GuiError>,
    pace: &mut impl Pace,
) -> Result<FromHelper, GuiError> {
    with_link(inner, outlet, |l| {
        if l.reply.as_ref().is_some_and(|r| r.rx.ring().is_closed())
            && let Some(stale) = l.reply.take()
        {
            forget_reply(&mut l.out, stale);
        }
        if l.reply.is_none() {
            let name = transport::reply_ring(&inner.id, inner.number());
            let ring = Ring::create(&name, Role::Reply).map_err(|e| GuiError::Transport(e.to_string()))?;
            let rx = ring.receiver().map_err(|e| GuiError::Transport(e.to_string()))?;
            let wake: Arc<dyn Wake> = Arc::new(RingWake(ring));
            inner.watches.add(&wake);
            l.reply = Some(ReplyRing { name, rx, wake });
        }
        let ThreadLink { out, reply: slot, .. } = l;
        let Some(reply) = slot.as_mut() else {
            return Err(GuiError::Transport(String::from("this thread's reply ring is missing")));
        };
        if let Some(reason) = inner.death() {
            return Err(GuiError::Gone(reason));
        }
        let request = inner.number();
        out.send(inner, &build(request, reply.name.clone()))?;
        let canceler = Canceler::new(Arc::clone(&reply.wake));
        let guard = arm(canceler.clone())?;
        let answer = loop {
            let next = match pace.due() {
                None => reply.rx.recv::<FromHelper>().map(Some),
                Some(due) => reply.rx.recv_within::<FromHelper>(due.saturating_duration_since(Instant::now())),
            };
            match next {
                Ok(Some(msg)) if crate::reply_number(&msg) == Some(request) => {
                    if pace.ends(&msg, Instant::now()) {
                        break Ok(msg);
                    }
                }
                Ok(Some(other)) => eprintln!("PWRS-GUI: a reply for another request arrived on {}: {other:?}", reply.name),
                Ok(None) => break Err(pace.late()),
                Err(transport::TransportError::Closed { .. }) => break Err(inner.woken(&canceler)),
                Err(e) => break Err(inner.transport_failed(e)),
            }
        };
        drop(guard);
        if matches!(answer, Err(GuiError::Late(_)))
            && let Some(stale) = slot.take()
        {
            forget_reply(out, stale);
        }
        answer
    })
}
