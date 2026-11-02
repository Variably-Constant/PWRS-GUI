//! Times a renderer's start and the first frame of its windows, from a
//! plain Rust program, on the machine it runs on. Each run starts a
//! session, opens a window and waits for its first frame, opens and closes
//! more windows on the running renderer, and ends the session, printing in
//! milliseconds:
//!
//! - `start`: `Session::start` until it returns, the helper's process
//!   started and its first message read, or the in-process run started;
//! - `first`: the first window's `open_window` until its `Opened` event,
//!   which the renderer sends after the window's first frame;
//! - `next`: the same for each further window on the running renderer.
//!
//! `--renderer` picks egui (the default) or html. The helper is the
//! renderer's beside this example's profile folder, or the path `--helper`
//! names. `--adapter` prefers an adapter by part of its name, as
//! `Set-GuiOption -Adapter` does, for egui.
//!
//!     cargo run --release -p pwrs-gui-client --example first_frame -- [--renderer egui|html] [--mode helper|inprocess] [--runs N] [--windows N] [--helper <path>] [--adapter <name>]

use pwrs_gui_client::proto::{Kind, Node, Value, WindowOptions};
use pwrs_gui_client::{Canceler, GuiError, Mode, Renderer, Session, StartOptions, WebViewData, WindowEvent};
use std::path::PathBuf;
use std::time::Instant;

/// The arm for every wait here: nothing cancels them, so it stores nothing.
fn never_canceled(_: Canceler) -> Result<(), GuiError> {
    Ok(())
}

struct Args {
    mode: Mode,
    renderer: Renderer,
    runs: usize,
    windows: usize,
    helper: Result<PathBuf, String>,
    adapter: Option<String>,
}

fn parse() -> Result<Args, String> {
    let mut args = Args { mode: Mode::Helper, renderer: Renderer::Egui, runs: 5, windows: 5, helper: Err(String::new()), adapter: None };
    let mut named = None;
    let mut it = std::env::args().skip(1);
    while let Some(a) = it.next() {
        let value = |it: &mut dyn Iterator<Item = String>| it.next().ok_or_else(|| format!("{a} needs a value"));
        match a.as_str() {
            "--mode" => {
                args.mode = match value(&mut it)?.as_str() {
                    "helper" => Mode::Helper,
                    "inprocess" => Mode::InProcess,
                    other => return Err(format!("--mode is helper or inprocess, not {other}")),
                }
            }
            "--renderer" => {
                args.renderer = match value(&mut it)?.as_str() {
                    "egui" => Renderer::Egui,
                    "html" => Renderer::Html,
                    other => return Err(format!("--renderer is egui or html, not {other}")),
                }
            }
            "--runs" => args.runs = value(&mut it)?.parse().map_err(|e| format!("--runs: {e}"))?,
            "--windows" => args.windows = value(&mut it)?.parse().map_err(|e| format!("--windows: {e}"))?,
            "--helper" => named = Some(PathBuf::from(value(&mut it)?)),
            "--adapter" => args.adapter = Some(value(&mut it)?),
            other => return Err(format!("unknown argument {other}")),
        }
    }
    args.helper = match named {
        Some(path) => Ok(path),
        None => beside_this_program(args.renderer),
    };
    Ok(args)
}

fn tree() -> Node {
    let items = Value::List(vec![Value::Str(String::from("alpha")), Value::Str(String::from("beta"))]);
    Node::new(1, Kind::Column).child(Node::new(2, Kind::ListBox).named("Choice").prop("items", items)).child(Node::new(3, Kind::Button).named("Ok").prop("text", Value::Str(String::from("OK"))))
}

/// Milliseconds from opening a window until its first frame.
fn open_to_first_frame(session: &Session, title: &str) -> Result<f64, Box<dyn std::error::Error>> {
    let began = Instant::now();
    let mut window = session.open_window(WindowOptions::titled(title), tree())?;
    loop {
        match window.next_event()? {
            WindowEvent::Opened => break,
            WindowEvent::Closed(reason) => return Err(format!("window {title} closed before its first frame: {reason:?}").into()),
            _ => {}
        }
    }
    let ms = began.elapsed().as_secs_f64() * 1000.0;
    window.close()?;
    Ok(ms)
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = parse()?;
    let options = StartOptions {
        mode: args.mode,
        renderer: args.renderer,
        helper: args.helper.clone(),
        adapter: args.adapter.clone(),
        low_power: false,
        log_dir: std::env::temp_dir().join("pwrs-gui-first-frame"),
        web_view: WebViewData::Shared,
        web_view_data_folder: None,
        fonts: pwrs_gui_client::proto::Fonts::Bundled,
        environment: None,
    };
    println!("run start first next");
    for run in 1..=args.runs {
        let began = Instant::now();
        let (session, _) = Session::start(&options, never_canceled)?;
        let start = began.elapsed().as_secs_f64() * 1000.0;
        let first = open_to_first_frame(&session, &format!("first frame {run}"))?;
        let mut next = Vec::with_capacity(args.windows);
        for w in 1..=args.windows {
            next.push(open_to_first_frame(&session, &format!("first frame {run}.{w}"))?);
        }
        let next: Vec<String> = next.iter().map(|ms| format!("{ms:.1}")).collect();
        println!("{run} {start:.1} {first:.1} {}", next.join(","));
        if run == 1
            && let Some(hello) = session.hello()
        {
            eprintln!("{} renderer on {}, through {}", hello.renderer, hello.capabilities.adapter, hello.capabilities.backend);
        }
        session.shut_down();
    }
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
