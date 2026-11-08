//! A web page lives on, not loaded again, when the helper whose job started
//! its WebView2 browser process ends. Every PWRS-GUI process on the shared
//! data folder shares that browser process, so it must not die with the job
//! of the helper that happened to start it; a page the renderer loaded
//! again after its browser process died would hide that, so the page
//! reports an id it chose as it loaded with every heartbeat. Every wait has
//! a deadline and the test a watchdog; see `common`.

mod common;

#[cfg(windows)]
#[test]
fn a_page_lives_on_when_the_helper_that_started_its_browser_process_ends() {
    use common::{Ends, Watchdog, choice_tree, next, options_for, page, page_message, step, within};
    use pwrs_gui_client::proto::WindowOptions;
    use pwrs_gui_client::{Mode, Renderer, Session, WindowEvent};

    let _watchdog = Watchdog::start("a_page_lives_on_when_the_helper_that_started_its_browser_process_ends");
    step("starting a web helper session with a window, so its helper starts the shared browser process");
    let (helper, _) = Session::start(&options_for(Mode::Helper, Renderer::Html), within).expect("start the web helper");
    let mut helper_window = helper.open_window(WindowOptions::titled("PWRS-GUI automated test"), choice_tree()).expect("open the helper's window");
    assert_eq!(next(&mut helper_window), WindowEvent::Opened);

    step("starting an in-process web session whose page sends its id and a count every 25 ms");
    let (local, _) = Session::start(&options_for(Mode::InProcess, Renderer::Html), within).expect("start the in-process web session");
    let local = Ends(local);
    let script = "var id = Math.floor(Math.random() * 1e9); var n = 0; setInterval(function () { window.pwrs.send(id + ' ' + n++); }, 25);";
    let mut beating = local.0.open_window(WindowOptions::titled("PWRS-GUI automated test"), page(script)).expect("open the page");
    let heartbeat = |json: String| -> (String, u64) {
        let text = json.trim_matches('"');
        let (id, n) = text.split_once(' ').unwrap_or_else(|| panic!("a heartbeat is an id and a count, not {json}"));
        (id.to_string(), n.parse().unwrap_or_else(|e| panic!("a heartbeat's count, not {n}: {e}")))
    };
    let (loaded_as, first) = heartbeat(page_message(&mut beating));
    let first_read = std::time::Instant::now();

    step("ending the helper's session, which closes its job");
    drop(helper_window);
    helper.shut_down();
    drop(helper);
    let elapsed = u64::try_from(first_read.elapsed().as_millis()).expect("milliseconds since the first heartbeat fit");
    let sent_by_then = first + elapsed / 25 + 1;

    step("reading heartbeats, from the page as first loaded, until one it counted a second after the helper ended");
    loop {
        let (id, n) = heartbeat(page_message(&mut beating));
        assert_eq!(id, loaded_as, "the page was loaded again, so the browser process it ran in ended with the helper");
        if n >= sent_by_then + 40 {
            break;
        }
    }
    beating.close().expect("close the page");
}
