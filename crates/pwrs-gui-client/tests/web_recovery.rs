//! A page whose WebView2 process is killed comes back as it was: a widget
//! window with its tree, rows and inputs' state, a page of the caller's
//! own with what it saved and none of the replies owed to the page before
//! it, and a page that dies again before drawing closes its window saying
//! why. One in-process HTML session runs on a data
//! folder of the test's own, so the WebView2 processes on it are the test's
//! alone. Every wait has a deadline and the test a watchdog; see `common`.

mod common;

#[cfg(windows)]
#[test]
fn a_page_whose_engine_is_killed_comes_back_as_it_was() {
    use common::{Ends, WAIT, Watchdog, next, options_for, page, page_message, step, webview_gone, webview_processes, within};
    use pwrs_gui_client::proto::{Cell, CloseReason, EventKind, Kind, Node, Row, SortKey, Value, WindowOptions};
    use pwrs_gui_client::{Mode, Renderer, Session, Window, WindowEvent};
    use std::collections::BTreeMap;
    use std::time::{Duration, Instant};

    /// The window's snapshot once its page is loaded again; a request made
    /// while the page is being loaded again is refused.
    fn snapshot_again(session: &Session, window: &Window) -> pwrs_gui_client::proto::Snapshot {
        let until = Instant::now() + WAIT;
        loop {
            match session.snapshot(window.id(), None, false, within) {
                Ok(s) => return s,
                Err(e) => {
                    assert!(Instant::now() < until, "window {} gave no snapshot within {WAIT:?}: {e}", window.id());
                    std::thread::sleep(Duration::from_millis(100));
                }
            }
        }
    }

    /// The next value `window`'s page sends, and the call number a reply to
    /// it names, left without a reply.
    fn unanswered(window: &mut Window) -> (u32, String) {
        loop {
            match next(window) {
                WindowEvent::Message { call, json } => return (call, json),
                WindowEvent::Closed(reason) => panic!("window {} closed while its page was to speak: {reason:?}", window.id()),
                _ => {}
            }
        }
    }

    let _watchdog = Watchdog::start("a_page_whose_engine_is_killed_comes_back_as_it_was");
    let folder = std::env::temp_dir().join(format!("pwrs-gui-recovery-test-{}", std::process::id()));
    let mut options = options_for(Mode::InProcess, Renderer::Html);
    options.web_view_data_folder = Some(folder.clone());
    let (session, _) = Session::start(&options, within).expect("start the in-process web session");
    let session = Ends(session);

    let items = Value::List(["alpha", "beta", "gamma"].iter().map(|s| Value::Str((*s).to_string())).collect());
    let columns = Value::List(vec![Value::Map(BTreeMap::from([(String::from("label"), Value::Str(String::from("Name")))]))]);
    let tree = Node::new(1, Kind::Column)
        .child(Node::new(2, Kind::ListBox).named("Choice").prop("items", items))
        .child(Node::new(3, Kind::Table).named("Grid").prop("columns", columns).prop("selection", Value::Str(String::from("multiple"))));
    let rows = |from: usize, to: usize| (from..to).map(|i| Row { cells: vec![Cell { text: format!("row {i}"), sort: SortKey::Number(i as f64) }] }).collect::<Vec<_>>();

    for (what, renderer) in [("browser process", false), ("renderer process", true)] {
        step(&format!("opening a widget window with a list and a table of eight rows, for its {what}"));
        let mut window = session.0.open_window(WindowOptions::titled("PWRS-GUI automated test"), tree.clone()).expect("open the window");
        window.rows(3, 0, rows(0, 5), false).expect("send rows");
        window.rows(3, 5, rows(5, 8), false).expect("send rows");
        assert_eq!(next(&mut window), WindowEvent::Opened);
        session.0.test_event(window.id(), "Choice", EventKind::Select, Value::List(vec![Value::Int(1)]), within).expect("select in the list");
        session.0.test_event(window.id(), "Grid", EventKind::Select, Value::List(vec![Value::Int(2), Value::Int(4)]), within).expect("select in the table");
        let before = session.0.snapshot(window.id(), None, false, within).expect("read the window");

        step(&format!("killing the {what}"));
        let killed = webview_processes(&folder, renderer, true);
        assert!(!killed.is_empty(), "a {what} runs on {}", folder.display());

        step("reading the window back once its page is loaded again");
        let after = snapshot_again(&session.0, &window);
        assert_eq!(after.tree, before.tree);
        assert_eq!(after.state, before.state);
        assert_eq!(after.state.get(&3).and_then(|t| t.as_map()).and_then(|t| t.get("rows")), Some(&Value::Int(8)), "the table has its eight rows");
        window.close().expect("close the window");
    }

    step("opening a page that saves a count and reports what it got back");
    let saving = "window.pwrs.restore().then(function (v) { window.pwrs.send({ restored: v }); window.pwrs.save((v || 0) + 1); });";
    let mut saver = session.0.open_window(WindowOptions::titled("PWRS-GUI automated test"), page(saving)).expect("open the page");
    assert_eq!(page_message(&mut saver), r#"{"restored":null}"#, "a first load has nothing saved");
    step("killing the browser process under it");
    assert!(!webview_processes(&folder, false, true).is_empty(), "a browser process runs on {}", folder.display());
    assert_eq!(page_message(&mut saver), r#"{"restored":1}"#, "the page loaded again gets back what it saved");
    saver.close().expect("close the page");

    step("opening a page that sends to the session, and sends again once loaded again");
    let asking = "window.pwrs.restore().then(function (v) { if (v === null) { window.pwrs.save('asked'); window.pwrs.send('before'); } else { window.pwrs.send('again').then(function (r) { window.pwrs.send({ got: r }); }); } });";
    let mut asker = session.0.open_window(WindowOptions::titled("PWRS-GUI automated test"), page(asking)).expect("open the page");
    let (before, said) = unanswered(&mut asker);
    assert_eq!(said, r#""before""#);
    step("killing the browser process while the page's call has no reply");
    assert!(!webview_processes(&folder, false, true).is_empty(), "a browser process runs on {}", folder.display());
    let (again, said) = unanswered(&mut asker);
    assert_eq!(said, r#""again""#, "the page loaded again sends again");
    step("replying to the page before, then to the page loaded again");
    asker.reply(before, String::from(r#""meant for the page before""#)).expect("reply to the first call");
    asker.reply(again, String::from(r#""meant for this page""#)).expect("reply to the second call");
    assert_eq!(page_message(&mut asker), r#"{"got":"meant for this page"}"#, "the page loaded again takes its own reply alone");
    asker.close().expect("close the page");

    step("opening a page that never draws and killing its renderer twice");
    let mut hung = session.0.open_window(WindowOptions::titled("PWRS-GUI automated test"), page("while (true) {}")).expect("open the page");
    let until = Instant::now() + WAIT;
    let first = loop {
        let found = webview_processes(&folder, true, false);
        if !found.is_empty() {
            std::thread::sleep(Duration::from_millis(500));
            break webview_processes(&folder, true, true);
        }
        assert!(Instant::now() < until, "no renderer ran on {} within {WAIT:?}", folder.display());
        std::thread::sleep(Duration::from_millis(100));
    };
    let until = Instant::now() + WAIT;
    loop {
        let fresh: Vec<u32> = webview_processes(&folder, true, false).into_iter().filter(|p| !first.contains(p)).collect();
        if !fresh.is_empty() {
            std::thread::sleep(Duration::from_millis(500));
            webview_processes(&folder, true, true);
            break;
        }
        assert!(Instant::now() < until, "no renderer came back on {} within {WAIT:?} of the first kill", folder.display());
        std::thread::sleep(Duration::from_millis(100));
    }
    step("reading the window close, its command told why");
    let mut told = None;
    loop {
        match next(&mut hung) {
            WindowEvent::Problem(text) => told = Some(text),
            WindowEvent::Closed(reason) => {
                assert_eq!(reason, CloseReason::Failed);
                break;
            }
            _ => {}
        }
    }
    let told = told.expect("an error before the close");
    assert!(told.contains("renderer process") && told.contains("again"), "{told}");

    step("ending the session and removing the test's data folder");
    drop(session);
    webview_gone(&folder);
    std::fs::remove_dir_all(&folder).expect("remove the test's data folder");
}
