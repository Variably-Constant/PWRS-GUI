//! Opens a window from a plain Rust program, with no PowerShell: starts the
//! renderer, shows a list of colors with an OK button, and prints the one
//! chosen when OK is pressed. egui draws it, or a webview with `--html`. The
//! helper is the program the first argument names, or the one built beside
//! this example by `cargo build -p pwrs-gui --bin pwrs-gui-egui` (or
//! `--bin pwrs-gui-web`) with the same profile. With `--self-test` the
//! program makes the choice itself through test events, as a test would,
//! instead of waiting for a person.
//!
//!     cargo run -p pwrs-gui-client --example choose -- [<helper>] [--html] [--self-test]

use pwrs_gui_client::proto::{EventKind, Kind, Node, Value, WindowOptions};
use pwrs_gui_client::{Canceler, GuiError, Mode, Renderer, Session, StartOptions, WebViewData, WindowEvent};
use std::path::PathBuf;

/// The arm for every wait here: nothing cancels them, so it stores nothing.
fn never_canceled(_: Canceler) -> Result<(), GuiError> {
    Ok(())
}

/// The row a list's selection names first; `None` for an empty selection.
fn first_row(rows: &[Value]) -> Result<Option<usize>, String> {
    match rows.first() {
        None => Ok(None),
        Some(Value::Int(i)) => match usize::try_from(*i) {
            Ok(row) => Ok(Some(row)),
            Err(e) => Err(format!("row {i} is not a row number: {e}")),
        },
        Some(other) => Err(format!("a selection names rows by number, not {other:?}")),
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let self_test = args.iter().any(|a| a == "--self-test");
    let renderer = if args.iter().any(|a| a == "--html") { Renderer::Html } else { Renderer::Egui };
    let helper = match args.iter().find(|a| !a.starts_with("--")) {
        Some(path) => Ok(PathBuf::from(path)),
        None => beside_this_program(renderer),
    };
    let options = StartOptions {
        mode: Mode::Auto,
        renderer,
        helper,
        adapter: None,
        low_power: false,
        log_dir: std::env::temp_dir().join("pwrs-gui-example"),
        web_view: WebViewData::Shared,
        web_view_data_folder: None,
        fonts: pwrs_gui_client::proto::Fonts::Bundled,
        environment: None,
    };
    let (session, fell_back) = Session::start(&options, never_canceled)?;
    if let Some(why) = fell_back {
        println!("drawing in-process, since the helper could not start: {why}");
    }
    if let Some(hello) = session.hello() {
        println!("{} renderer on {}, through {}", hello.renderer, hello.capabilities.adapter, hello.capabilities.backend);
    }

    let colors = ["red", "green", "blue"];
    let items = Value::List(colors.iter().map(|c| Value::Str(c.to_string())).collect());
    let tree = Node::new(1, Kind::Column)
        .child(Node::new(2, Kind::Text).prop("text", Value::Str(String::from("Pick a color"))))
        .child(Node::new(3, Kind::ListBox).named("Choice").prop("items", items))
        .child(Node::new(4, Kind::Button).named("Ok").prop("text", Value::Str(String::from("OK"))));
    let mut window = session.open_window(WindowOptions::titled("PWRS-GUI from Rust"), tree)?;

    let mut selected = None;
    loop {
        match window.next_event()? {
            WindowEvent::Opened => {
                println!("window {} opened", window.id());
                if self_test {
                    session.test_event(window.id(), "Choice", EventKind::Select, Value::Int(1), never_canceled)?;
                    session.test_event(window.id(), "Ok", EventKind::Click, Value::Null, never_canceled)?;
                }
            }
            WindowEvent::Event { node: 3, kind: EventKind::Select, value: Value::List(rows), .. } => {
                selected = first_row(&rows)?;
                match selected {
                    Some(row) => println!("selected row {row}"),
                    None => println!("selection cleared"),
                }
            }
            WindowEvent::Event { node: 4, kind: EventKind::Click, .. } => break,
            WindowEvent::Closed(reason) => {
                println!("the window closed ({reason:?}) with nothing chosen");
                session.shut_down();
                return Ok(());
            }
            other => println!("{other:?}"),
        }
    }
    window.close()?;
    match selected {
        Some(row) => match colors.get(row) {
            Some(color) => println!("chosen: {color}"),
            None => return Err(format!("the window chose row {row}, and there are {} colors", colors.len()).into()),
        },
        None => println!("OK pressed with nothing selected"),
    }
    session.shut_down();
    Ok(())
}

/// The renderer's helper in this example's profile folder, beside its
/// `examples` folder.
fn beside_this_program(renderer: Renderer) -> Result<PathBuf, String> {
    let exe = std::env::current_exe().map_err(|e| format!("this program's path cannot be read: {e}"))?;
    let profile = exe.parent().and_then(|examples| examples.parent()).ok_or("this program has no profile folder")?;
    let base = renderer.helper_name();
    let path = profile.join(if cfg!(windows) { format!("{base}.exe") } else { base.to_string() });
    if path.is_file() { Ok(path) } else { Err(format!("{} is not built; build it with cargo build -p pwrs-gui --bin {base} and the same profile", path.display())) }
}
