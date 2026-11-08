//! The native entry this module offers, so another module in the process
//! draws through this module's session rather than starting a renderer of
//! its own.
//!
//! PWRS loads every native library from a renamed copy under a
//! per-process temporary folder, so an entry cannot be found by name;
//! `Get-GuiNativeEntry` returns the addresses of the copy that is loaded.
//! The taking module passes them to `pwrs_gui_client::NativeEntry`.

use crate::session;
use pwrs::prelude::*;
use pwrs_gui_client::entry::{self as abi, ArmV1, DisarmV1, InfoV1, SendV1, StartV1, UnwatchV1, WakeV1, WatchV1};
use pwrs_gui_client::proto::PROTOCOL;
use std::ffi::c_void;

/// The live session of `renderer`, or one started as this module's options
/// say; see [`abi::StartV1`].
///
/// # Safety
/// The caller follows [`abi::StartV1`]'s contract.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pwrs_gui_start_v1(
    renderer: u32,
    arm: ArmV1,
    disarm: DisarmV1,
    ctx: *mut c_void,
    info: *mut InfoV1,
    error: *mut u8,
    error_cap: usize,
    error_len: *mut usize,
) -> i32 {
    // SAFETY: the caller's contract, passed through.
    unsafe { abi::serve_start(session::slot, session::start_options, renderer, arm, disarm, ctx, info, error, error_cap, error_len) }
}

/// A wake for the end of the session `id` names; see [`abi::WatchV1`].
///
/// # Safety
/// The caller follows [`abi::WatchV1`]'s contract.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pwrs_gui_watch_v1(id: *const u8, id_len: usize, wake: WakeV1, ctx: *const c_void) -> u64 {
    // SAFETY: the caller's contract, passed through.
    unsafe { abi::serve_watch(&session::slots(), id, id_len, wake, ctx) }
}

/// Removes a wake `pwrs_gui_watch_v1` registered; see [`abi::UnwatchV1`].
///
/// # Safety
/// `token` is one `pwrs_gui_watch_v1` returned.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pwrs_gui_unwatch_v1(token: u64) {
    abi::serve_unwatch(token);
}

/// Gives the in-process renderer one message; see [`abi::SendV1`].
///
/// # Safety
/// The caller follows [`abi::SendV1`]'s contract.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pwrs_gui_send_v1(id: *const u8, id_len: usize, message: *const u8, len: usize, error: *mut u8, error_cap: usize, error_len: *mut usize) -> i32 {
    // SAFETY: the caller's contract, passed through.
    unsafe { abi::serve_send(&session::slots(), id, id_len, message, len, error, error_cap, error_len) }
}

/// Where the native entry's functions are in the library loaded now, and
/// the versions they speak.
#[psclass(name = "PwrsGui.NativeEntry")]
#[derive(Clone, Default)]
pub struct NativeEntry {
    /// The address of `pwrs_gui_start_v1`.
    pub start_v1: u64,
    /// The address of `pwrs_gui_watch_v1`.
    pub watch_v1: u64,
    /// The address of `pwrs_gui_unwatch_v1`.
    pub unwatch_v1: u64,
    /// The address of `pwrs_gui_send_v1`.
    pub send_v1: u64,
    /// The ABI version of these functions.
    pub abi_version: u32,
    /// The protocol the sessions' renderers speak; a module taking a
    /// session must speak the same.
    pub protocol: u32,
    /// Whether a session of either renderer is running now, read without
    /// starting one.
    pub session_running: bool,
}

/// Gets the addresses of the native entry through which another module in
/// this process draws with this module's sessions, so the process runs one
/// helper per renderer whichever modules open windows. A module taking a
/// session names the renderer it wants, egui or HTML.
///
/// The library PWRS loads is a renamed copy in a per-process folder, so an
/// entry cannot be found by name; this returns the addresses in the copy
/// that is loaded. An address stays callable for the life of the process, since
/// PWRS never frees a library it has loaded. A rebuild loaded by a later
/// import is a new copy with its own session, so a module used in several
/// runspaces stores one result per runspace, taken in that runspace.
///
/// # Examples
/// Get-GuiNativeEntry
#[cmdlet(verb = "Get", noun = "GuiNativeEntry", output = ["PwrsGui.NativeEntry"])]
#[derive(Default)]
pub struct GetGuiNativeEntry {}

impl Cmdlet for GetGuiNativeEntry {
    fn process(&mut self, ps: &Pipeline<'_>) -> PsResult<()> {
        ps.write(NativeEntry {
            start_v1: pwrs_gui_start_v1 as StartV1 as usize as u64,
            watch_v1: pwrs_gui_watch_v1 as WatchV1 as usize as u64,
            unwatch_v1: pwrs_gui_unwatch_v1 as UnwatchV1 as usize as u64,
            send_v1: pwrs_gui_send_v1 as SendV1 as usize as u64,
            abi_version: abi::ABI_VERSION,
            protocol: PROTOCOL,
            session_running: !session::running().is_empty(),
        })
    }
}
