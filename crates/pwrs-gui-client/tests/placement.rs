//! A window opens centered on the monitor `PWRS_GUI_MONITOR` names, drawn
//! by each renderer in the helper and in-process: a monitor other than the
//! primary one when the machine has one. The variable is set for this
//! test's process, which a helper inherits; the test reads monitors and
//! windows in physical pixels, as the renderers place them. Every wait has
//! a deadline and the test a watchdog; see `common`.

mod common;

#[cfg(windows)]
#[test]
fn a_window_opens_centered_on_the_monitor_pwrs_gui_monitor_names() {
    use common::{Ends, Watchdog, choice_tree, next, options_for, step, within};
    use pwrs_gui_client::proto::{WindowOptions, place};
    use pwrs_gui_client::{Mode, Renderer, Session, WindowEvent};
    use windows_sys::Win32::Foundation::{ERROR_ACCESS_DENIED, GetLastError, RECT};
    use windows_sys::Win32::UI::HiDpi::{DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, SetProcessDpiAwarenessContext};
    use windows_sys::Win32::UI::WindowsAndMessaging::{FindWindowW, GetWindowRect};

    let _watchdog = Watchdog::start("a_window_opens_centered_on_the_monitor_pwrs_gui_monitor_names");
    if unsafe { SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2) } == 0 {
        let error = unsafe { GetLastError() };
        assert_eq!(error, ERROR_ACCESS_DENIED, "this process's scale awareness cannot be set, and it was not set before");
    }
    let listed = place::monitors().expect("the monitors list");
    let target = listed.iter().find(|m| (m.x, m.y) != (0, 0)).or_else(|| listed.first()).expect("a monitor").clone();
    let n = place::number(&target).expect("the monitor's display number");
    // SAFETY: this test is the only one in its process, and it sets the
    // variable before any renderer starts.
    unsafe { std::env::set_var(place::VAR, n.to_string()) };
    let (width, height) = (400.0f32, 200.0f32);
    for (mode, renderer, name) in [
        (Mode::Helper, Renderer::Egui, "egui in the helper"),
        (Mode::Helper, Renderer::Html, "HTML in the helper"),
        (Mode::InProcess, Renderer::Egui, "egui in-process"),
        (Mode::InProcess, Renderer::Html, "HTML in-process"),
    ] {
        step(&format!("opening a window drawn by {name}, with {} at {n}", place::VAR));
        let title = format!("PWRS-GUI placement test: {name}");
        let (session, _) = Session::start(&options_for(mode, renderer), within).expect("start the session");
        let session = Ends(session);
        let mut options = WindowOptions::titled(&title);
        options.width = Some(width);
        options.height = Some(height);
        let mut window = session.0.open_window(options, choice_tree()).expect("open the window");
        assert_eq!(next(&mut window), WindowEvent::Opened);

        step("reading where the window opened");
        let wide: Vec<u16> = title.encode_utf16().chain(std::iter::once(0)).collect();
        let hwnd = unsafe { FindWindowW(std::ptr::null(), wide.as_ptr()) };
        assert!(!hwnd.is_null(), "no window is titled {title}");
        let mut r = RECT { left: 0, top: 0, right: 0, bottom: 0 };
        assert_ne!(unsafe { GetWindowRect(hwnd, &mut r) }, 0, "the window's rectangle reads");
        let center = ((r.left + r.right) / 2, (r.top + r.bottom) / 2);
        let wanted = (target.x + i32::try_from(target.width / 2).expect("a monitor's width"), target.y + i32::try_from(target.height / 2).expect("a monitor's height"));
        // The window's frame and title bar lie outside the size requested,
        // so its center sits within a frame's width of the monitor's.
        let off = ((center.0 - wanted.0).abs(), (center.1 - wanted.1).abs());
        assert!(off.0 <= 64 && off.1 <= 64, "{name}: the window's center {center:?} is {off:?} from monitor {n}'s center {wanted:?}; the window is {:?}, the monitor {target:?}", (r.left, r.top, r.right, r.bottom));
        window.close().expect("close the window");
    }
}
