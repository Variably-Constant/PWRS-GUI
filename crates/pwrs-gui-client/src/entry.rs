//! PWRS-GUI's native entry: how the module offers its sessions to other
//! modules in the same process, and how another module takes one, so a
//! process runs one helper and at most one in-process UI thread per
//! renderer whichever modules draw.
//!
//! The offering module exports the functions typed below under their
//! versioned names and returns their addresses from `Get-GuiNativeEntry`.
//! A taker names the renderer it wants, and gets the offering module's
//! session of that renderer. It links this crate too and talks to the
//! renderer over the same SubEtha rings the offering module's commands use:
//! it opens the session's inbound ring and makes its own window and reply
//! rings. An in-process session has no inbound ring, so a taker's messages
//! go through `pwrs_gui_send_v1` and the renderer writes its events onto
//! the taker's rings. Every number a taker gives a window, a request or a
//! ring starts at the base its take was given, which encodes the session's
//! renderer, so no two modules' numbers meet.
//!
//! Every symbol has its ABI version in its name; a change to one once
//! a released module calls it is a new symbol beside the old one.

use crate::helper::Outlet;
use crate::watch::{Canceler, Wake};
use crate::{GuiError, Inner, Link, Renderer, Session, SessionSlot, StartOptions};
use arc_swap::ArcSwapOption;
use pwrs_gui_proto::transport::{self, Ring, Role};
use pwrs_gui_proto::{PROTOCOL, ToHelper};
use std::cell::Cell;
use std::ffi::c_void;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::ptr::null_mut;
use std::sync::atomic::{AtomicU8, AtomicU64, Ordering};
use std::sync::{Arc, OnceLock, Weak};

/// The version of the entry this crate offers and takes.
pub const ABI_VERSION: u32 = 1;

/// [`InfoV1::path`] of a session drawing in a helper process.
pub const PATH_HELPER: u32 = 1;
/// [`InfoV1::path`] of a session drawing in the offering module's process.
pub const PATH_IN_PROCESS: u32 = 2;

/// The renderer a [`StartV1`] call names: egui.
pub const RENDERER_EGUI: u32 = 1;
/// The renderer a [`StartV1`] call names: HTML.
pub const RENDERER_HTML: u32 = 2;

/// The code a [`StartV1`] call names `renderer` by.
pub fn renderer_code(renderer: Renderer) -> u32 {
    match renderer {
        Renderer::Egui => RENDERER_EGUI,
        Renderer::Html => RENDERER_HTML,
    }
}

/// The renderer a [`StartV1`] call's code names.
pub fn renderer_of_code(code: u32) -> Result<Renderer, GuiError> {
    match code {
        RENDERER_EGUI => Ok(Renderer::Egui),
        RENDERER_HTML => Ok(Renderer::Html),
        other => Err(GuiError::Start(format!("the module taking the session named renderer {other}, and the renderers are {RENDERER_EGUI} (egui) and {RENDERER_HTML} (HTML)"))),
    }
}

pub const STATUS_OK: i32 = 0;
/// The call failed; the error text says why.
pub const STATUS_FAILED: i32 = 1;
/// The taker's cancel ended the wait.
pub const STATUS_CANCELED: i32 = 2;
/// The session named is not the live one.
pub const STATUS_STALE: i32 = 3;

/// The longest session name an [`InfoV1`] has room for.
pub const ID_CAP: usize = 64;

/// Low bits of each taker's numbers: room for 2^40 of them.
const BASE_SHIFT: u32 = 40;

/// A session as the entry describes it to the module taking it.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct InfoV1 {
    /// The protocol the renderer speaks.
    pub protocol: u32,
    /// [`PATH_HELPER`] or [`PATH_IN_PROCESS`].
    pub path: u32,
    /// Where the taker's own window and request numbers start.
    pub base: u64,
    /// The bytes of `id` in use.
    pub id_len: u32,
    /// The session's name, UTF-8, which its rings' names begin with.
    pub id: [u8; ID_CAP],
}

/// A wait's cancel, given to the taker's arm. `cancel` ends the wait,
/// from any thread, any number of times; `release` gives the handle back,
/// once, after which the taker calls neither. Both take `handle`.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct CancelV1 {
    pub handle: *const c_void,
    pub cancel: unsafe extern "C" fn(*const c_void),
    pub release: unsafe extern "C" fn(*const c_void),
}

/// The taker's arm: given a wait's cancel just before the wait, it returns
/// a guard the entry passes to the [`DisarmV1`] when the wait ends, or null
/// to refuse the wait, which fails the call. It releases the cancel in
/// either case.
pub type ArmV1 = unsafe extern "C" fn(ctx: *mut c_void, cancel: CancelV1) -> *mut c_void;
/// Ends what an [`ArmV1`] armed.
pub type DisarmV1 = unsafe extern "C" fn(ctx: *mut c_void, guard: *mut c_void);
/// The taker's wake for the renderer's end, called once, from any thread,
/// with the reason in UTF-8.
pub type WakeV1 = unsafe extern "C" fn(ctx: *const c_void, reason: *const u8, reason_len: usize);

/// `pwrs_gui_start_v1`: the live session of `renderer` ([`RENDERER_EGUI`]
/// or [`RENDERER_HTML`]), or one the offering module starts as its options
/// say, waiting with no time limit through the taker's arm. Writes `info`
/// and returns [`STATUS_OK`]; or writes the start of why into `error` (at
/// most `error_cap` bytes, the full length in `error_len`) and returns
/// [`STATUS_FAILED`] or [`STATUS_CANCELED`].
pub type StartV1 =
    unsafe extern "C" fn(renderer: u32, arm: ArmV1, disarm: DisarmV1, ctx: *mut c_void, info: *mut InfoV1, error: *mut u8, error_cap: usize, error_len: *mut usize) -> i32;
/// `pwrs_gui_watch_v1`: registers `wake` for the end of the session `id`
/// names and returns a token for [`UnwatchV1`], or 0 when that session is
/// not the live one. A session already ended is woken for at once. `ctx`
/// stays usable from any thread until the unwatch returns.
pub type WatchV1 = unsafe extern "C" fn(id: *const u8, id_len: usize, wake: WakeV1, ctx: *const c_void) -> u64;
/// `pwrs_gui_unwatch_v1`: removes a wake [`WatchV1`] registered. A wake
/// already running finishes first, unless it is running on the calling
/// thread.
pub type UnwatchV1 = unsafe extern "C" fn(token: u64);
/// `pwrs_gui_send_v1`: gives the in-process renderer of the session `id`
/// names one message, as [`transport::to_bytes`] encodes it. Returns
/// [`STATUS_OK`], [`STATUS_STALE`], or [`STATUS_FAILED`] with why in `error`
/// as [`StartV1`] writes it.
pub type SendV1 = unsafe extern "C" fn(id: *const u8, id_len: usize, message: *const u8, len: usize, error: *mut u8, error_cap: usize, error_len: *mut usize) -> i32;

/// Writes `text` into a caller's error buffer.
///
/// # Safety
/// `error` must be writable for `cap` bytes, and `len` writable.
unsafe fn write_error(text: &str, error: *mut u8, cap: usize, len: *mut usize) {
    let n = text.len().min(cap);
    if !error.is_null() && n > 0 {
        // SAFETY: the caller's buffer has room for `cap` bytes, and `n <= cap`.
        unsafe { std::ptr::copy_nonoverlapping(text.as_ptr(), error, n) };
    }
    if !len.is_null() {
        // SAFETY: the caller's length is writable.
        unsafe { *len = text.len() };
    }
}

/// A caller's UTF-8 bytes as a string.
///
/// # Safety
/// `ptr` must be readable for `len` bytes.
unsafe fn read_str(ptr: *const u8, len: usize) -> Result<String, String> {
    if ptr.is_null() {
        return Err(String::from("a null name"));
    }
    // SAFETY: the caller's bytes are readable for `len`.
    let bytes = unsafe { std::slice::from_raw_parts(ptr, len) };
    std::str::from_utf8(bytes).map(str::to_string).map_err(|e| format!("a name that is not UTF-8: {e}"))
}

fn panic_text(payload: &(dyn std::any::Any + Send)) -> String {
    if let Some(s) = payload.downcast_ref::<&str>() {
        (*s).to_string()
    } else if let Some(s) = payload.downcast_ref::<String>() {
        s.clone()
    } else {
        String::from("a panic with no message")
    }
}

// ---------------------------------------------------------------------
// The offering side.

/// Takers' indices, from 1; the session's own numbers sit below the first
/// base.
static NEXT_TAKER: AtomicU64 = AtomicU64::new(1);

fn info_of(session: &Session) -> Result<InfoV1, GuiError> {
    let id = session.id().as_bytes();
    if id.len() > ID_CAP {
        return Err(GuiError::Start(format!("the session's name is {} bytes, and the entry has room for {ID_CAP}", id.len())));
    }
    let path = match &session.inner.link {
        Link::Helper(_) => PATH_HELPER,
        #[cfg(all(windows, feature = "in-process"))]
        Link::InProcess(_) => PATH_IN_PROCESS,
        Link::Shared(_) => return Err(GuiError::Start(String::from("a session taken through the entry cannot be offered again"))),
    };
    let base = session.renderer().number_base() | (NEXT_TAKER.fetch_add(1, Ordering::Relaxed) << BASE_SHIFT);
    let mut info = InfoV1 { protocol: PROTOCOL, path, base, id_len: 0, id: [0; ID_CAP] };
    info.id[..id.len()].copy_from_slice(id);
    info.id_len = id.len() as u32;
    Ok(info)
}

/// What a taker's arm armed, disarmed when the wait ends.
struct ForeignGuard {
    guard: *mut c_void,
    disarm: DisarmV1,
    ctx: *mut c_void,
}

impl Drop for ForeignGuard {
    fn drop(&mut self) {
        // SAFETY: the taker's contract: `disarm` takes what its arm returned.
        unsafe { (self.disarm)(self.ctx, self.guard) };
    }
}

unsafe extern "C" fn cancel_handle(handle: *const c_void) {
    // SAFETY: a handle is an Arc<Canceler> from `serve_start`, alive until
    // the taker releases it.
    let canceler = unsafe { &*handle.cast::<Canceler>() };
    if let Err(p) = catch_unwind(AssertUnwindSafe(|| canceler.cancel())) {
        eprintln!("PWRS-GUI: a cancel through the native entry panicked: {}", panic_text(&*p));
    }
}

unsafe extern "C" fn release_handle(handle: *const c_void) {
    // SAFETY: as `cancel_handle`; the taker releases each handle once.
    drop(unsafe { Arc::from_raw(handle.cast::<Canceler>()) });
}

/// What `pwrs_gui_start_v1` does for a module offering a session per
/// renderer: `slot_for` gives a renderer's slot, and `options` what a
/// session of that renderer starts with.
///
/// # Safety
/// As [`StartV1`] documents: `arm`, `disarm` and `ctx` follow its contract,
/// `info` is writable, `error` is writable for `error_cap` bytes and
/// `error_len` is writable.
#[allow(clippy::too_many_arguments)]
pub unsafe fn serve_start<'s>(
    slot_for: impl Fn(Renderer) -> &'s SessionSlot,
    options: impl FnOnce(Renderer) -> Result<StartOptions, String>,
    renderer: u32,
    arm: ArmV1,
    disarm: DisarmV1,
    ctx: *mut c_void,
    info: *mut InfoV1,
    error: *mut u8,
    error_cap: usize,
    error_len: *mut usize,
) -> i32 {
    let outcome = catch_unwind(AssertUnwindSafe(|| -> Result<InfoV1, GuiError> {
        let renderer = renderer_of_code(renderer)?;
        let slot = slot_for(renderer);
        if let Some(live) = slot.current() {
            return info_of(&live);
        }
        let options = options(renderer).map_err(GuiError::Start)?;
        let foreign = |c: Canceler| -> Result<ForeignGuard, GuiError> {
            let handle = Arc::into_raw(Arc::new(c)).cast::<c_void>();
            let cancel = CancelV1 { handle, cancel: cancel_handle, release: release_handle };
            // SAFETY: the taker's contract for `arm` and `ctx`.
            let guard = unsafe { arm(ctx, cancel) };
            if guard.is_null() {
                Err(GuiError::Arm(String::from("the module taking the session refused the wait")))
            } else {
                Ok(ForeignGuard { guard, disarm, ctx })
            }
        };
        let (session, _) = slot.get_or_start(&options, foreign)?;
        info_of(&session)
    }));
    match outcome {
        Ok(Ok(made)) => {
            // SAFETY: the caller's `info` is writable.
            unsafe { *info = made };
            STATUS_OK
        }
        Ok(Err(GuiError::Canceled)) => {
            // SAFETY: the caller's error buffer, as documented.
            unsafe { write_error("the wait was canceled", error, error_cap, error_len) };
            STATUS_CANCELED
        }
        Ok(Err(e)) => {
            // SAFETY: as above.
            unsafe { write_error(&e.to_string(), error, error_cap, error_len) };
            STATUS_FAILED
        }
        Err(p) => {
            // SAFETY: as above.
            unsafe { write_error(&format!("the start panicked: {}", panic_text(&*p)), error, error_cap, error_len) };
            STATUS_FAILED
        }
    }
}

const ARMED: u8 = 0;
const FIRING: u8 = 1;
const DONE: u8 = 2;

thread_local! {
    /// The token of the wake this thread is running, so an unwatch from
    /// inside that wake does not wait for itself.
    static FIRING_HERE: Cell<u64> = const { Cell::new(0) };
}

/// A taker's wake, registered with the session whose end it waits for.
struct ForeignWake {
    token: u64,
    wake: WakeV1,
    ctx: *const c_void,
    state: AtomicU8,
    session: Weak<Inner>,
}

// SAFETY: the taker's contract: `ctx` is usable from any thread until its
// unwatch returns, and `wake` may be called from any thread.
unsafe impl Send for ForeignWake {}
unsafe impl Sync for ForeignWake {}

impl ForeignWake {
    fn fire(&self) {
        if self.state.compare_exchange(ARMED, FIRING, Ordering::AcqRel, Ordering::Acquire).is_err() {
            return;
        }
        let reason = match self.session.upgrade().and_then(|s| s.death()) {
            Some(r) => r,
            None => String::from("the session ended"),
        };
        let before = FIRING_HERE.with(|f| f.replace(self.token));
        // SAFETY: the taker's contract for `wake` and `ctx`.
        let called = catch_unwind(AssertUnwindSafe(|| unsafe { (self.wake)(self.ctx, reason.as_ptr(), reason.len()) }));
        FIRING_HERE.with(|f| f.set(before));
        if let Err(p) = called {
            eprintln!("PWRS-GUI: a native-entry taker's wake panicked: {}", panic_text(&*p));
        }
        self.state.store(DONE, Ordering::Release);
    }

    /// Stops the wake from running, waiting for a run already under way
    /// on another thread.
    fn disarm(&self) {
        if self.state.compare_exchange(ARMED, DONE, Ordering::AcqRel, Ordering::Acquire).is_ok() {
            return;
        }
        if FIRING_HERE.with(Cell::get) == self.token {
            return;
        }
        while self.state.load(Ordering::Acquire) == FIRING {
            std::thread::yield_now();
        }
    }
}

impl Wake for ForeignWake {
    fn wake(&self) {
        self.fire();
    }
}

/// Every taker's registered wake, alive here while the session has only a
/// weak reference to it; read and changed without a lock.
static FOREIGN: ArcSwapOption<Vec<Arc<ForeignWake>>> = ArcSwapOption::const_empty();
static NEXT_TOKEN: AtomicU64 = AtomicU64::new(1);

/// The live session named `id` among `slots`.
fn named(slots: &[&SessionSlot], id: &str) -> Option<Session> {
    slots.iter().find_map(|slot| slot.current().filter(|s| s.id() == id))
}

/// What `pwrs_gui_watch_v1` does for a module offering `slots`. A panic is
/// reported on standard error and returns 0.
///
/// # Safety
/// As [`WatchV1`] documents.
pub unsafe fn serve_watch(slots: &[&SessionSlot], id: *const u8, id_len: usize, wake: WakeV1, ctx: *const c_void) -> u64 {
    // SAFETY: the caller's contract, passed through.
    match catch_unwind(AssertUnwindSafe(|| unsafe { watch(slots, id, id_len, wake, ctx) })) {
        Ok(token) => token,
        Err(p) => {
            eprintln!("PWRS-GUI: pwrs_gui_watch_v1 panicked: {}", panic_text(&*p));
            0
        }
    }
}

/// # Safety
/// As [`WatchV1`] documents.
unsafe fn watch(slots: &[&SessionSlot], id: *const u8, id_len: usize, wake: WakeV1, ctx: *const c_void) -> u64 {
    // SAFETY: the caller's name, as documented.
    let id = match unsafe { read_str(id, id_len) } {
        Ok(id) => id,
        Err(e) => {
            eprintln!("PWRS-GUI: pwrs_gui_watch_v1 was given no session name it could read: {e}");
            return 0;
        }
    };
    let Some(session) = named(slots, &id) else {
        return 0;
    };
    let token = NEXT_TOKEN.fetch_add(1, Ordering::Relaxed);
    let watch = Arc::new(ForeignWake { token, wake, ctx, state: AtomicU8::new(ARMED), session: Arc::downgrade(&session.inner) });
    FOREIGN.rcu(|current| {
        let mut next = match current {
            Some(list) => (**list).clone(),
            None => Vec::new(),
        };
        next.push(Arc::clone(&watch));
        Some(Arc::new(next))
    });
    let as_wake: Arc<dyn Wake> = watch.clone();
    session.inner.watches.add(&as_wake);
    if session.inner.death().is_some() {
        watch.fire();
    }
    token
}

/// What `pwrs_gui_unwatch_v1` does. A panic is reported on standard error.
pub fn serve_unwatch(token: u64) {
    if let Err(p) = catch_unwind(|| unwatch(token)) {
        eprintln!("PWRS-GUI: pwrs_gui_unwatch_v1 panicked: {}", panic_text(&*p));
    }
}

fn unwatch(token: u64) {
    let mut removed = None;
    FOREIGN.rcu(|current| {
        let mut next = match current {
            Some(list) => (**list).clone(),
            None => Vec::new(),
        };
        removed = next.iter().position(|w| w.token == token).map(|at| next.remove(at));
        Some(Arc::new(next))
    });
    if let Some(watch) = removed {
        watch.disarm();
    }
}

/// What `pwrs_gui_send_v1` does for a module offering `slots`. A panic
/// returns [`STATUS_FAILED`], with the panic's text.
///
/// # Safety
/// As [`SendV1`] documents.
#[allow(clippy::too_many_arguments)]
pub unsafe fn serve_send(slots: &[&SessionSlot], id: *const u8, id_len: usize, message: *const u8, len: usize, error: *mut u8, error_cap: usize, error_len: *mut usize) -> i32 {
    // SAFETY: the caller's contract, passed through.
    match catch_unwind(AssertUnwindSafe(|| unsafe { send(slots, id, id_len, message, len, error, error_cap, error_len) })) {
        Ok(status) => status,
        Err(p) => {
            // SAFETY: the caller's error buffer, as documented.
            unsafe { write_error(&format!("the send panicked: {}", panic_text(&*p)), error, error_cap, error_len) };
            STATUS_FAILED
        }
    }
}

/// # Safety
/// As [`SendV1`] documents.
#[allow(clippy::too_many_arguments)]
unsafe fn send(slots: &[&SessionSlot], id: *const u8, id_len: usize, message: *const u8, len: usize, error: *mut u8, error_cap: usize, error_len: *mut usize) -> i32 {
    let fail = |text: &str| {
        // SAFETY: the caller's error buffer, as documented.
        unsafe { write_error(text, error, error_cap, error_len) };
        STATUS_FAILED
    };
    // SAFETY: the caller's name, as documented.
    let id = match unsafe { read_str(id, id_len) } {
        Ok(id) => id,
        Err(e) => return fail(&e),
    };
    let Some(session) = named(slots, &id) else {
        return STATUS_STALE;
    };
    if message.is_null() {
        return fail("a null message");
    }
    // SAFETY: the caller's message is readable for `len` bytes.
    let bytes = unsafe { std::slice::from_raw_parts(message, len) };
    let msg: ToHelper = match transport::from_bytes(bytes) {
        Ok(m) => m,
        Err(e) => return fail(&format!("the message cannot be read: {e}")),
    };
    match &session.inner.link {
        #[cfg(all(windows, feature = "in-process"))]
        Link::InProcess(l) => match l.send(msg, None) {
            Ok(()) => STATUS_OK,
            Err(e) => fail(&e.to_string()),
        },
        Link::Helper(_) => fail(&format!("the session draws in a helper, so a module taking it sends on its inbound ring, and {msg:?} was not delivered")),
        Link::Shared(_) => fail(&format!("a session taken through the entry is not offered again, so {msg:?} was not delivered")),
    }
}

// ---------------------------------------------------------------------
// The taking side.

/// Another module's native entry, from the addresses its
/// `Get-GuiNativeEntry` returned.
#[derive(Clone, Copy)]
pub struct NativeEntry {
    start: StartV1,
    watch: WatchV1,
    unwatch: UnwatchV1,
    send: SendV1,
}

impl NativeEntry {
    /// The entry at these addresses, refusing another ABI version or
    /// protocol than this crate's.
    ///
    /// # Safety
    /// The addresses must be ones `Get-GuiNativeEntry` returned in this
    /// process, for the functions of this ABI version; PWRS never unloads a
    /// library, so they stay callable.
    pub unsafe fn from_addresses(abi_version: u32, protocol: u32, start: u64, watch: u64, unwatch: u64, send: u64) -> Result<NativeEntry, GuiError> {
        if abi_version != ABI_VERSION {
            return Err(GuiError::Mismatch { helper: format!("a native entry of ABI version {abi_version}"), here: format!("a taker of ABI version {ABI_VERSION}") });
        }
        if protocol != PROTOCOL {
            return Err(GuiError::Mismatch { helper: format!("a session speaking protocol {protocol}"), here: format!("a taker speaking protocol {PROTOCOL}") });
        }
        for (name, address) in [("start", start), ("watch", watch), ("unwatch", unwatch), ("send", send)] {
            if address == 0 {
                return Err(GuiError::Start(format!("the native entry gives no address for its {name} function")));
            }
        }
        // SAFETY: the caller's guarantee that each address is that function
        // of this ABI version; a function pointer is an address wide.
        Ok(unsafe {
            NativeEntry {
                start: std::mem::transmute::<usize, StartV1>(start as usize),
                watch: std::mem::transmute::<usize, WatchV1>(watch as usize),
                unwatch: std::mem::transmute::<usize, UnwatchV1>(unwatch as usize),
                send: std::mem::transmute::<usize, SendV1>(send as usize),
            }
        })
    }

    /// The offering module's live session of `renderer`, or one it starts,
    /// waiting through `arm` as [`Session::start`] does. The result is a
    /// [`Session`] like any other, whose windows draw in the offering
    /// module's renderer; shutting it down leaves that renderer running.
    pub fn session<G>(&self, renderer: Renderer, mut arm: impl FnMut(Canceler) -> Result<G, GuiError>) -> Result<Session, GuiError> {
        let mut state = ArmState { arm: &mut |c| arm(c).map(|g| Box::new(g) as Box<dyn Held + '_>), guards: Vec::new(), refused: None };
        let mut info = InfoV1 { protocol: 0, path: 0, base: 0, id_len: 0, id: [0; ID_CAP] };
        let mut error = vec![0u8; 2048];
        let mut error_len = 0usize;
        // SAFETY: the entry's contract: the trampolines follow it, `state`
        // outlives the call, and every buffer is as documented.
        let status = unsafe {
            (self.start)(renderer_code(renderer), arm_trampoline, disarm_trampoline, (&raw mut state).cast(), &mut info, error.as_mut_ptr(), error.len(), &mut error_len)
        };
        let refused = state.refused.take();
        drop(state);
        match status {
            STATUS_OK => {}
            STATUS_CANCELED => return Err(GuiError::Canceled),
            _ => {
                if let Some(e) = refused {
                    return Err(e);
                }
                error.truncate(error_len.min(error.len()));
                return Err(GuiError::Start(String::from_utf8_lossy(&error).into_owned()));
            }
        }
        if info.protocol != PROTOCOL {
            return Err(GuiError::Mismatch { helper: format!("a session speaking protocol {}", info.protocol), here: format!("a taker speaking protocol {PROTOCOL}") });
        }
        let id_len = (info.id_len as usize).min(ID_CAP);
        let id = std::str::from_utf8(&info.id[..id_len]).map_err(|e| GuiError::Transport(format!("the session's name is not UTF-8: {e}")))?.to_string();
        let path = match info.path {
            PATH_HELPER => SharedPath::Helper(Ring::open(&transport::inbound_ring(&id), Role::Inbound).map_err(|e| GuiError::Transport(e.to_string()))?),
            PATH_IN_PROCESS => SharedPath::InProcess,
            other => return Err(GuiError::Transport(format!("the entry reports a path numbered {other}"))),
        };
        let inner = Arc::new(Inner::with_base(id.clone(), Link::Shared(SharedLink { entry: *self, path, watch: OnceLock::new() }), renderer, info.base));
        let ctx = Box::into_raw(Box::new(Arc::downgrade(&inner))).cast::<c_void>().cast_const();
        // SAFETY: the entry's contract; `ctx` lives until the unwatch in
        // SharedLink's drop returns.
        let token = unsafe { (self.watch)(id.as_ptr(), id.len(), wake_trampoline, ctx) };
        if token == 0 {
            // SAFETY: `ctx` came from Box::into_raw above and no registration
            // refers to it.
            drop(unsafe { Box::from_raw(ctx.cast_mut().cast::<Weak<Inner>>()) });
            return Err(GuiError::Gone(String::from("the session ended as it was taken")));
        }
        if let Link::Shared(s) = &inner.link
            && s.watch.set(Watch { token, ctx: ctx as usize }).is_err()
        {
            return Err(GuiError::Transport(String::from("the taken session's registration with the entry was set twice")));
        }
        Ok(Session { inner })
    }

    /// Gives an in-process session's renderer one message.
    pub(crate) fn send_message(&self, id: &str, msg: &ToHelper) -> Result<(), GuiError> {
        let bytes = transport::to_bytes(msg).map_err(GuiError::Transport)?;
        let mut error = vec![0u8; 2048];
        let mut error_len = 0usize;
        // SAFETY: the entry's contract; every buffer is as documented.
        let status = unsafe { (self.send)(id.as_ptr(), id.len(), bytes.as_ptr(), bytes.len(), error.as_mut_ptr(), error.len(), &mut error_len) };
        match status {
            STATUS_OK => Ok(()),
            STATUS_STALE => Err(GuiError::Gone(String::from("the session this module took has ended"))),
            _ => {
                error.truncate(error_len.min(error.len()));
                Err(GuiError::Transport(String::from_utf8_lossy(&error).into_owned()))
            }
        }
    }
}

/// Anything a guard can be: it is only ever dropped.
trait Held {}
impl<T> Held for T {}

/// The taker's arm and the guards it has returned, for one start call.
struct ArmState<'a> {
    arm: &'a mut dyn FnMut(Canceler) -> Result<Box<dyn Held + 'a>, GuiError>,
    guards: Vec<Option<Box<dyn Held + 'a>>>,
    refused: Option<GuiError>,
}

/// A cancel the offering module gave, as this crate's wake.
struct ForeignCancel(CancelV1);

// SAFETY: the entry's contract: `cancel` may be called from any thread
// until `release`, which is called once, when this drops.
unsafe impl Send for ForeignCancel {}
unsafe impl Sync for ForeignCancel {}

impl Wake for ForeignCancel {
    fn wake(&self) {
        // SAFETY: the handle is live until this drops.
        unsafe { (self.0.cancel)(self.0.handle) };
    }
}

impl Drop for ForeignCancel {
    fn drop(&mut self) {
        // SAFETY: released once, here.
        unsafe { (self.0.release)(self.0.handle) };
    }
}

unsafe extern "C" fn arm_trampoline(ctx: *mut c_void, cancel: CancelV1) -> *mut c_void {
    // SAFETY: `ctx` is the ArmState `session` passed, alive for the call.
    let state = unsafe { &mut *ctx.cast::<ArmState<'_>>() };
    let wake: Arc<dyn Wake> = Arc::new(ForeignCancel(cancel));
    let canceler = Canceler::new(wake);
    match catch_unwind(AssertUnwindSafe(|| (state.arm)(canceler))) {
        Ok(Ok(guard)) => {
            state.guards.push(Some(guard));
            // A guard is its index plus one, so it is never null.
            state.guards.len() as *mut c_void
        }
        Ok(Err(e)) => {
            state.refused = Some(e);
            null_mut()
        }
        Err(p) => {
            state.refused = Some(GuiError::Arm(format!("the arm panicked: {}", panic_text(&*p))));
            null_mut()
        }
    }
}

unsafe extern "C" fn disarm_trampoline(ctx: *mut c_void, guard: *mut c_void) {
    // SAFETY: as `arm_trampoline`.
    let state = unsafe { &mut *ctx.cast::<ArmState<'_>>() };
    let at = (guard as usize).wrapping_sub(1);
    match state.guards.get_mut(at) {
        Some(slot) => drop(slot.take()),
        None => eprintln!("PWRS-GUI: the native entry disarmed guard {}, which was never armed", guard as usize),
    }
}

unsafe extern "C" fn wake_trampoline(ctx: *const c_void, reason: *const u8, reason_len: usize) {
    let woke = catch_unwind(AssertUnwindSafe(|| {
        // SAFETY: `ctx` is the Weak `session` boxed, alive until unwatch.
        let session = unsafe { &*ctx.cast::<Weak<Inner>>() };
        let text = if reason.is_null() {
            String::from("the session ended")
        } else {
            // SAFETY: the entry's reason is readable for `reason_len` bytes.
            String::from_utf8_lossy(unsafe { std::slice::from_raw_parts(reason, reason_len) }).into_owned()
        };
        if let Some(inner) = session.upgrade() {
            inner.die(text);
        }
    }));
    if let Err(p) = woke {
        eprintln!("PWRS-GUI: the taken session's wake panicked: {}", panic_text(&*p));
    }
}

/// How a taken session reaches its renderer.
pub(crate) enum SharedPath {
    /// The helper's inbound ring, opened by this module.
    Helper(Arc<Ring>),
    /// The offering module's in-process renderer, through `pwrs_gui_send_v1`.
    InProcess,
}

/// A taken session's wake as the offering module registered it: its token,
/// and the context the wake reads.
struct Watch {
    token: u64,
    /// The boxed `Weak<Inner>` the wake reads, as an address.
    ctx: usize,
}

/// A session this module took through another module's entry.
pub(crate) struct SharedLink {
    pub(crate) entry: NativeEntry,
    pub(crate) path: SharedPath,
    watch: OnceLock<Watch>,
}

impl SharedLink {
    /// Where this session's messages go.
    pub(crate) fn outlet(&self) -> Outlet<'_> {
        match &self.path {
            SharedPath::Helper(ring) => Outlet::Inbound(ring),
            SharedPath::InProcess => Outlet::Entry(&self.entry),
        }
    }
}

impl Drop for SharedLink {
    fn drop(&mut self) {
        if let Some(w) = self.watch.take() {
            // SAFETY: the token is this link's own registration.
            unsafe { (self.entry.unwatch)(w.token) };
            // SAFETY: `ctx` came from Box::into_raw in `session`, and the
            // unwatch has returned, so no wake reads it any more.
            drop(unsafe { Box::from_raw(w.ctx as *mut Weak<Inner>) });
        }
    }
}
