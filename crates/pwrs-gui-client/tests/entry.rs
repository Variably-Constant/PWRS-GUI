//! The native entry, offered and taken in one process. The offering side is
//! a slot per renderer this test owns, reached only through the entry's
//! functions by address, as another module reaches PWRS-GUI's. Every wait
//! has a deadline and every test a watchdog; see `common`.

mod common;

use common::{Watchdog, choice_tree, next, options_for, step, within};
use pwrs_gui_client::entry::{self, ArmV1, DisarmV1, InfoV1, SendV1, StartV1, UnwatchV1, WakeV1, WatchV1};
use pwrs_gui_client::proto::{CloseReason, EventKind, PROTOCOL, Value, WindowOptions};
use pwrs_gui_client::{GuiError, Mode, NativeEntry, Renderer, SessionSlot, WindowEvent};
use std::ffi::c_void;

static HELPER_EGUI: SessionSlot = SessionSlot::new();
static HELPER_HTML: SessionSlot = SessionSlot::new();

fn helper_slot(renderer: Renderer) -> &'static SessionSlot {
    match renderer {
        Renderer::Egui => &HELPER_EGUI,
        Renderer::Html => &HELPER_HTML,
    }
}

unsafe extern "C" fn helper_start(renderer: u32, arm: ArmV1, disarm: DisarmV1, ctx: *mut c_void, info: *mut InfoV1, error: *mut u8, cap: usize, len: *mut usize) -> i32 {
    // SAFETY: the taker follows StartV1's contract.
    unsafe { entry::serve_start(helper_slot, |r| Ok(options_for(Mode::Helper, r)), renderer, arm, disarm, ctx, info, error, cap, len) }
}

unsafe extern "C" fn helper_watch(id: *const u8, id_len: usize, wake: WakeV1, ctx: *const c_void) -> u64 {
    // SAFETY: the taker follows WatchV1's contract.
    unsafe { entry::serve_watch(&[&HELPER_EGUI, &HELPER_HTML], id, id_len, wake, ctx) }
}

unsafe extern "C" fn helper_send(id: *const u8, id_len: usize, message: *const u8, len: usize, error: *mut u8, cap: usize, error_len: *mut usize) -> i32 {
    // SAFETY: the taker follows SendV1's contract.
    unsafe { entry::serve_send(&[&HELPER_EGUI, &HELPER_HTML], id, id_len, message, len, error, cap, error_len) }
}

unsafe extern "C" fn unwatch(token: u64) {
    entry::serve_unwatch(token);
}

fn entry_for(start: StartV1, watch: WatchV1, send: SendV1) -> NativeEntry {
    // SAFETY: the addresses are this test's own functions of ABI version 1.
    let taken = unsafe { NativeEntry::from_addresses(entry::ABI_VERSION, PROTOCOL, start as usize as u64, watch as usize as u64, unwatch as UnwatchV1 as usize as u64, send as usize as u64) };
    taken.expect("the entry")
}

/// Takes the session of `renderer` that `slot` offers through `taker`,
/// drives a window from both sides, and ends the offering session.
fn take_and_drive(slot: &'static SessionSlot, taker: NativeEntry, renderer: Renderer, path: &str) {
    step("taking the session through the entry, which starts it");
    let taken = taker.session(renderer, within).expect("take the session");
    assert!(taken.is_taken());
    assert_eq!(taken.path(), path);
    assert_eq!(taken.renderer(), renderer);
    let owner = slot.current().expect("the offering slot has the session the take started");
    assert_eq!(owner.id(), taken.id());
    assert_eq!(owner.renderer(), renderer);

    step("opening a window from the taken session");
    let mut window = taken.open_window(WindowOptions::titled("PWRS-GUI entry test"), choice_tree()).expect("open a window through the entry");
    assert!(window.id() >= 1 << 40, "the taker's numbers start at its base, and window {} does not", window.id());
    assert_eq!(Renderer::of_number(window.id()), renderer, "window {}'s number names its renderer", window.id());
    assert_eq!(next(&mut window), WindowEvent::Opened);

    step("finding it from the offering session and acting on it there");
    let listed = owner.windows(within).expect("the offering session's windows");
    assert!(listed.iter().any(|w| w.window == window.id()), "the offering session lists the taker's window");
    owner.test_event(window.id(), "Choice", EventKind::Select, Value::Int(2), within).expect("select through the offering session");
    match next(&mut window) {
        WindowEvent::Event { kind: EventKind::Select, value, .. } => assert_eq!(value, Value::List(vec![Value::Int(2)])),
        other => panic!("expected the selection, read {other:?}"),
    }

    step("a request from the taken session");
    let shot = taken.snapshot(window.id(), None, false, within).expect("a snapshot through the entry");
    assert_eq!(shot.state.get(&3), Some(&Value::List(vec![Value::Int(2)])));

    step("closing the window from the taken session");
    window.close().expect("close the taker's window");

    step("ending the offering session, which closes the taker's window and ends its session");
    let mut second = taken.open_window(WindowOptions::titled("PWRS-GUI entry test, second window"), choice_tree()).expect("a second window");
    assert_eq!(next(&mut second), WindowEvent::Opened);
    let owner = slot.take().expect("the offering session");
    owner.shut_down();
    assert_eq!(next(&mut second), WindowEvent::Closed(CloseReason::Shutdown));
    let after = taken.windows(within);
    assert!(matches!(after, Err(GuiError::Gone(_))), "a request after the end reads the end: {after:?}");
    assert!(!taken.is_alive(), "the offering session's end reached the taken one");
}

#[test]
fn a_session_served_in_a_helper_is_taken_and_driven_through_the_entry() {
    let _watchdog = Watchdog::start("a_session_served_in_a_helper_is_taken_and_driven_through_the_entry");
    take_and_drive(&HELPER_EGUI, entry_for(helper_start, helper_watch, helper_send), Renderer::Egui, "Helper");
}

#[test]
fn an_html_session_served_in_a_helper_is_taken_and_driven_through_the_entry() {
    let _watchdog = Watchdog::start("an_html_session_served_in_a_helper_is_taken_and_driven_through_the_entry");
    take_and_drive(&HELPER_HTML, entry_for(helper_start, helper_watch, helper_send), Renderer::Html, "Helper");
}

#[cfg(all(windows, feature = "in-process"))]
static IN_EGUI: SessionSlot = SessionSlot::new();
#[cfg(all(windows, feature = "in-process"))]
static IN_HTML: SessionSlot = SessionSlot::new();

#[cfg(all(windows, feature = "in-process"))]
fn in_slot(renderer: Renderer) -> &'static SessionSlot {
    match renderer {
        Renderer::Egui => &IN_EGUI,
        Renderer::Html => &IN_HTML,
    }
}

#[cfg(all(windows, feature = "in-process"))]
unsafe extern "C" fn in_start(renderer: u32, arm: ArmV1, disarm: DisarmV1, ctx: *mut c_void, info: *mut InfoV1, error: *mut u8, cap: usize, len: *mut usize) -> i32 {
    // SAFETY: the taker follows StartV1's contract.
    unsafe { entry::serve_start(in_slot, |r| Ok(options_for(Mode::InProcess, r)), renderer, arm, disarm, ctx, info, error, cap, len) }
}

#[cfg(all(windows, feature = "in-process"))]
unsafe extern "C" fn in_watch(id: *const u8, id_len: usize, wake: WakeV1, ctx: *const c_void) -> u64 {
    // SAFETY: the taker follows WatchV1's contract.
    unsafe { entry::serve_watch(&[&IN_EGUI, &IN_HTML], id, id_len, wake, ctx) }
}

#[cfg(all(windows, feature = "in-process"))]
unsafe extern "C" fn in_send(id: *const u8, id_len: usize, message: *const u8, len: usize, error: *mut u8, cap: usize, error_len: *mut usize) -> i32 {
    // SAFETY: the taker follows SendV1's contract.
    unsafe { entry::serve_send(&[&IN_EGUI, &IN_HTML], id, id_len, message, len, error, cap, error_len) }
}

#[cfg(all(windows, feature = "in-process"))]
#[test]
fn a_session_served_in_process_is_taken_and_driven_through_the_entry() {
    let _watchdog = Watchdog::start("a_session_served_in_process_is_taken_and_driven_through_the_entry");
    take_and_drive(&IN_EGUI, entry_for(in_start, in_watch, in_send), Renderer::Egui, "InProcess");
}

#[cfg(all(windows, feature = "in-process"))]
#[test]
fn an_html_session_served_in_process_is_taken_and_driven_through_the_entry() {
    let _watchdog = Watchdog::start("an_html_session_served_in_process_is_taken_and_driven_through_the_entry");
    take_and_drive(&IN_HTML, entry_for(in_start, in_watch, in_send), Renderer::Html, "InProcess");
}

#[test]
fn an_entry_of_another_version_or_with_a_missing_address_is_refused() {
    let address = helper_start as StartV1 as usize as u64;
    // SAFETY: refused before any address is called.
    let other_abi = unsafe { NativeEntry::from_addresses(entry::ABI_VERSION + 1, PROTOCOL, address, address, address, address) };
    assert!(matches!(other_abi, Err(GuiError::Mismatch { .. })), "another ABI version is refused");
    // SAFETY: as above.
    let other_protocol = unsafe { NativeEntry::from_addresses(entry::ABI_VERSION, PROTOCOL + 1, address, address, address, address) };
    assert!(matches!(other_protocol, Err(GuiError::Mismatch { .. })), "another protocol is refused");
    // SAFETY: as above.
    let missing = unsafe { NativeEntry::from_addresses(entry::ABI_VERSION, PROTOCOL, address, 0, address, address) };
    assert!(matches!(missing, Err(GuiError::Start(ref why)) if why.contains("watch")), "a missing address is named");
}

#[test]
fn a_start_naming_no_renderer_is_refused() {
    for r in [Renderer::Egui, Renderer::Html] {
        let back = entry::renderer_of_code(entry::renderer_code(r));
        assert!(matches!(back, Ok(b) if b == r), "{r:?} came back as {back:?}");
    }
    let refused = entry::renderer_of_code(3);
    assert!(matches!(refused, Err(GuiError::Start(ref why)) if why.contains("renderer 3")), "{refused:?}");
}
