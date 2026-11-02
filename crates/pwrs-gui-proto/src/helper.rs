//! A helper process's side of its session, the same for every renderer: the
//! start and inbound rings it opens by the session's identifier, one
//! inbound thread that waits on the inbound ring's notifier, and on Linux,
//! FreeBSD and macOS on stdin's end as well, and passes each message to the
//! renderer, and the sink that writes the renderer's messages to the
//! session's rings.

use crate::outbound::RingOutlets;
use crate::renderer::{Inbound, Link, Sink, WakeLoop};
use crate::transport::{self, Receiver, Ring, Role, Sender};
use crate::{FromHelper, ToHelper};
use std::any::Any;
use std::sync::mpsc;
use std::sync::{Arc, OnceLock};

/// Opens the rings of the session `session`, starts the inbound thread, and
/// runs `renderer` on this thread with the link they make, returning what
/// it returns. `name` begins every line the helper writes to standard
/// error, which the session writes to a log file.
pub fn serve<F>(session: &str, name: &'static str, renderer: F) -> Result<(), String>
where
    F: FnOnce(Link) -> Result<(), String>,
{
    let start = Ring::open(&transport::start_ring(session), Role::Start).map_err(|e| e.to_string())?;
    let inbound = Ring::open(&transport::inbound_ring(session), Role::Inbound).map_err(|e| e.to_string())?;
    let notifier = inbound.notifier().map_err(|e| e.to_string())?;
    let rx = inbound.receiver().map_err(|e| e.to_string())?;
    let (tx, inbox) = mpsc::channel::<Inbound>();
    let wake: Arc<OnceLock<WakeLoop>> = Arc::new(OnceLock::new());
    let woken = Arc::clone(&wake);
    std::thread::Builder::new()
        .name(String::from("pwrs-gui-inbound"))
        .spawn(move || inbound_loop(name, rx, notifier, tx, woken))
        .map_err(|e| format!("the inbound thread cannot start: {e}"))?;
    let sink = RingSink { name, rings: RingOutlets::new(), start: start.sender().map_err(|e| e.to_string())? };
    let on_ready = Box::new(move |w: WakeLoop| {
        if wake.set(w).is_err() {
            eprintln!("{name}: the renderer reported its wake twice");
        }
    });
    renderer(Link { inbox, sink: Box::new(sink), on_ready })
}

/// Passes each message on the inbound ring to the renderer, and ends it when
/// the session sends `Shutdown`, the ring fails, or stdin ends.
fn inbound_loop(name: &str, mut rx: Receiver, notifier: transport::Notifier, tx: mpsc::Sender<Inbound>, wake: Arc<OnceLock<WakeLoop>>) {
    loop {
        loop {
            match rx.try_recv::<ToHelper>() {
                Ok(Some(msg)) => {
                    let last = matches!(msg, ToHelper::Shutdown);
                    if tx.send(Inbound::plain(msg)).is_err() {
                        return;
                    }
                    wake_loop(&wake);
                    if last {
                        return;
                    }
                }
                Ok(None) => break,
                Err(e) => {
                    eprintln!("{name}: the inbound ring failed: {e}");
                    end(name, &tx, &wake);
                    return;
                }
            }
        }
        match wait(&notifier) {
            Woke::Notified => notifier.drain(),
            #[cfg(unix)]
            Woke::StdinEnded => {
                end(name, &tx, &wake);
                return;
            }
            Woke::Failed(e) => {
                eprintln!("{name}: the inbound wait failed: {e}");
                end(name, &tx, &wake);
                return;
            }
        }
    }
}

/// Sends the renderer `Shutdown`, which closes every window and ends it.
fn end(name: &str, tx: &mpsc::Sender<Inbound>, wake: &OnceLock<WakeLoop>) {
    if tx.send(Inbound::plain(ToHelper::Shutdown)).is_err() {
        eprintln!("{name}: the renderer had already stopped taking messages");
    }
    wake_loop(wake);
}

/// Wakes the renderer's loop to read its inbox, once the renderer has given
/// its wake; a message sent before that is read on its first turn.
fn wake_loop(wake: &OnceLock<WakeLoop>) {
    if let Some(w) = wake.get() {
        w();
    }
}

enum Woke {
    Notified,
    #[cfg(unix)]
    StdinEnded,
    Failed(String),
}

#[cfg(unix)]
fn wait(notifier: &transport::Notifier) -> Woke {
    let fd = match i32::try_from(notifier.native()) {
        Ok(fd) => fd,
        Err(e) => return Woke::Failed(format!("the notifier's descriptor {} does not fit an int: {e}", notifier.native())),
    };
    let mut fds = [libc::pollfd { fd, events: libc::POLLIN, revents: 0 }, libc::pollfd { fd: libc::STDIN_FILENO, events: libc::POLLIN, revents: 0 }];
    loop {
        // SAFETY: `fds` is a live array of two pollfd records.
        let rc = unsafe { libc::poll(fds.as_mut_ptr(), 2, -1) };
        if rc < 0 {
            let e = std::io::Error::last_os_error();
            if e.raw_os_error() == Some(libc::EINTR) {
                continue;
            }
            return Woke::Failed(e.to_string());
        }
        if fds[1].revents & (libc::POLLIN | libc::POLLHUP | libc::POLLERR) != 0 {
            let mut byte = [0u8; 64];
            // SAFETY: `byte` is a live buffer of the length passed.
            let n = unsafe { libc::read(libc::STDIN_FILENO, byte.as_mut_ptr().cast(), byte.len()) };
            if n <= 0 {
                return Woke::StdinEnded;
            }
        }
        if fds[0].revents != 0 {
            return Woke::Notified;
        }
    }
}

#[cfg(windows)]
fn wait(notifier: &transport::Notifier) -> Woke {
    use windows_sys::Win32::Foundation::{HANDLE, WAIT_OBJECT_0};
    use windows_sys::Win32::System::Threading::{INFINITE, WaitForSingleObject};
    let handle = notifier.native() as usize as HANDLE;
    // SAFETY: the notifier owns `handle` and outlives this wait.
    match unsafe { WaitForSingleObject(handle, INFINITE) } {
        WAIT_OBJECT_0 => Woke::Notified,
        other => Woke::Failed(format!("WaitForSingleObject returned {other:#x}: {}", std::io::Error::last_os_error())),
    }
}

/// Where the helper's messages go: SubEtha rings, opened by name on first
/// use and let go when the session says they are gone.
struct RingSink {
    name: &'static str,
    start: Sender,
    rings: RingOutlets,
}

impl RingSink {
    fn send_to(&mut self, ring: &str, role: Role, msg: FromHelper) {
        if let Err(e) = self.rings.send(ring, role, &msg) {
            eprintln!("{}: {ring} did not take this ({e}): {msg:?}", self.name);
        }
    }
}

impl Sink for RingSink {
    fn attach(&mut self, ring: &str, attach: Box<dyn Any + Send>) {
        eprintln!("{}: {ring} came with an attachment ({:?}), which a helper does not take", self.name, (*attach).type_id());
    }

    fn hello(&mut self, msg: FromHelper) {
        if let Err(e) = self.start.send(&msg) {
            eprintln!("{}: the start ring did not take {msg:?}: {e}", self.name);
        }
    }

    fn window(&mut self, ring: &str, msg: FromHelper) {
        self.send_to(ring, Role::Window, msg);
    }

    fn reply(&mut self, ring: &str, msg: FromHelper) {
        self.send_to(ring, Role::Reply, msg);
    }

    fn forget(&mut self, ring: &str) {
        self.rings.forget(ring);
    }

    fn log(&mut self, msg: FromHelper) {
        eprintln!("{}: {}", self.name, msg.log_line());
    }
}
