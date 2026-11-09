//! A web session driven from a plain Rust program, as `session` drives an
//! egui one. A test binary of its own, since cargo runs test binaries one
//! after another: its windows and egui's are never open at once.

mod common;

use common::{Ends, Watchdog, drive, options_for, step, within};
use pwrs_gui_client::{Mode, Renderer, Session};

#[test]
fn a_web_helper_session_opens_drives_and_ends_a_window() {
    let _watchdog = Watchdog::start("a_web_helper_session_opens_drives_and_ends_a_window");
    step("starting a web helper session");
    let (session, fell_back) = Session::start(&options_for(Mode::Helper, Renderer::Html), within).expect("start a web helper session");
    let session = Ends(session);
    assert!(fell_back.is_none());
    assert_eq!(session.0.path(), "Helper");
    let hello = session.0.hello().expect("the renderer's hello");
    assert_eq!(hello.renderer, "web");
    drive(&session.0, Renderer::Html);
}

#[cfg(all(windows, feature = "in-process"))]
#[test]
fn an_in_process_web_session_opens_drives_and_ends_a_window() {
    let _watchdog = Watchdog::start("an_in_process_web_session_opens_drives_and_ends_a_window");
    step("starting an in-process web session");
    let (session, _) = Session::start(&options_for(Mode::InProcess, Renderer::Html), within).expect("start an in-process web session");
    let session = Ends(session);
    assert_eq!(session.0.path(), "InProcess");
    assert_eq!(session.0.hello().expect("the renderer's hello").renderer, "web");
    drive(&session.0, Renderer::Html);
    drop(session);
    step("starting a second in-process web session on the same UI thread");
    let (again, _) = Session::start(&options_for(Mode::InProcess, Renderer::Html), within).expect("a second in-process web session on the same UI thread");
    let again = Ends(again);
    drive(&again.0, Renderer::Html);
}
