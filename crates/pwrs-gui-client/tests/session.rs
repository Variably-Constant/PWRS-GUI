//! An egui session driven from a plain Rust program: start the renderer,
//! open a window, act on it as a person would, read it back, and end. Every
//! wait has a deadline and every test a watchdog; see `common`. The web
//! renderer's sessions are driven in `web_session`, a test binary of their
//! own, so their windows never cover this one's.

mod common;

use common::{Ends, Watchdog, drive, options, step, within};
use pwrs_gui_client::{Mode, Renderer, Session, SessionSlot};

#[test]
fn a_helper_session_opens_drives_and_ends_a_window() {
    let _watchdog = Watchdog::start("a_helper_session_opens_drives_and_ends_a_window");
    step("starting a helper session");
    let (session, fell_back) = Session::start(&options(Mode::Helper), within).expect("start a helper session");
    let session = Ends(session);
    assert!(fell_back.is_none());
    assert_eq!(session.0.path(), "Helper");
    let hello = session.0.hello().expect("the renderer's hello");
    assert_eq!(hello.renderer, "egui");
    drive(&session.0, Renderer::Egui);
}

#[test]
fn two_starts_through_one_slot_at_once_share_one_session() {
    static SLOT: SessionSlot = SessionSlot::new();
    let _watchdog = Watchdog::start("two_starts_through_one_slot_at_once_share_one_session");
    step("starting through one slot from two threads at once");
    let start = || std::thread::spawn(|| SLOT.get_or_start(&options(Mode::Helper), within).map(|(s, _)| s));
    let (first, second) = (start(), start());
    let first = first.join().expect("the first starting thread").expect("the first start");
    let first = Ends(first);
    let second = second.join().expect("the second starting thread").expect("the second start");
    assert!(first.0.same(&second), "both callers have the one session the slot started");
    let current = SLOT.current().expect("the slot has the session");
    assert!(current.same(&second));
    step("emptying the slot");
    let taken = SLOT.take().expect("the slot gives its session up");
    assert!(taken.same(&first.0));
    assert!(SLOT.current().is_none());
}

#[cfg(all(windows, feature = "in-process"))]
#[test]
fn an_in_process_session_opens_drives_and_ends_a_window() {
    let _watchdog = Watchdog::start("an_in_process_session_opens_drives_and_ends_a_window");
    step("starting an in-process session");
    let (session, _) = Session::start(&options(Mode::InProcess), within).expect("start an in-process session");
    let session = Ends(session);
    assert_eq!(session.0.path(), "InProcess");
    drive(&session.0, Renderer::Egui);
    drop(session);
    step("starting a second in-process session on the same UI thread");
    let (again, _) = Session::start(&options(Mode::InProcess), within).expect("a second in-process session on the same UI thread");
    let again = Ends(again);
    drive(&again.0, Renderer::Egui);
}
