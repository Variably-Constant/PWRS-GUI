//! The in-process egui renderer runs one session after another on its UI
//! thread, as reloads of the module do: twenty runs, each opening a window
//! and closing it, in one process. AMD's hook library, which AMD's drivers
//! load into a process that probes an AMD adapter, is still loaded after
//! the runs end, so its hook never calls into an unloaded library; on a
//! machine without an AMD adapter it is never loaded. No other renderer
//! runs in the process, so nothing else has the library loaded. Every wait has
//! a deadline and the test a watchdog; see `common`.

mod common;

#[cfg(windows)]
#[test]
fn twenty_in_process_egui_runs_follow_one_another_and_keep_the_driver_hook_loaded() {
    use common::{Ends, Watchdog, choice_tree, next, options_for, step, within};
    use pwrs_gui_client::proto::WindowOptions;
    use pwrs_gui_client::{Mode, Renderer, Session, WindowEvent};
    use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;

    /// Whether the library `name` is loaded in this process.
    fn loaded(name: &str) -> bool {
        let wide: Vec<u16> = name.encode_utf16().chain(std::iter::once(0)).collect();
        !unsafe { GetModuleHandleW(wide.as_ptr()) }.is_null()
    }

    let _watchdog = Watchdog::start("twenty_in_process_egui_runs_follow_one_another_and_keep_the_driver_hook_loaded");
    let mut hook_seen = false;
    for run in 1..=20 {
        step(&format!("run {run}: starting an in-process egui session, opening a window and closing it"));
        let (session, _) = Session::start(&options_for(Mode::InProcess, Renderer::Egui), within).expect("start the in-process session");
        let session = Ends(session);
        let mut window = session.0.open_window(WindowOptions::titled("PWRS-GUI automated test"), choice_tree()).expect("open the window");
        assert_eq!(next(&mut window), WindowEvent::Opened);
        hook_seen |= loaded("amdihk64.dll");
        window.close().expect("close the window");
        drop(session);
    }
    step(&format!("checking AMD's hook library, {} during the runs", if hook_seen { "loaded" } else { "never loaded" }));
    if hook_seen {
        // The renderer's run ends asynchronously on its UI thread after the
        // session is dropped; give the driver time to unload what it would.
        std::thread::sleep(std::time::Duration::from_millis(1500));
        assert!(loaded("amdihk64.dll"), "amdihk64.dll was unloaded after the runs ended, leaving its hook to call into it");
    }
}
