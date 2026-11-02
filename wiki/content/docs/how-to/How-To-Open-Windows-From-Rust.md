---
title: How To Open Windows From Rust
weight: 13
---

Open PWRS-GUI's windows from a plain Rust program, with no PowerShell involved: start a session, open a window from a tree of widgets, and read its events. Source: `crates/pwrs-gui-client/src/lib.rs`, whose `Session`, `Window` and `StartOptions` are the module's own session, and `crates/pwrs-gui-client/examples/choose.rs`, the program on this page.

The output on this page is from one run on Windows 11 Pro 10.0.26200 with an NVIDIA GeForce RTX 5070.

## The crate

`pwrs-gui-client` depends on no PWRS crate: it starts the renderer, sends and receives the messages, and waits on windows, for the module and for any other program. It is not published; depend on it from a checkout:

```toml
[dependencies]
pwrs-gui-client = { path = "../PWRS-GUI/crates/pwrs-gui-client" }
```

Its default `in-process` feature links the egui and HTML renderers, so `Mode::Auto` can draw in the program's own process on Windows when the helper cannot start. Each renderer's helper is a separate program built from the `pwrs-gui` crate, `pwrs-gui-egui` for `Renderer::Egui` and `pwrs-gui-web` for `Renderer::Html`; the program names the renderer and its helper's path in `StartOptions`.

## A program that lets a person pick a color

```rust
use pwrs_gui_client::proto::{EventKind, Kind, Node, Value, WindowOptions};
use pwrs_gui_client::{Canceler, GuiError, Mode, Renderer, Session, StartOptions, WindowEvent};
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
    let options = StartOptions { mode: Mode::Auto, renderer, helper, adapter: None, low_power: false, log_dir: std::env::temp_dir().join("pwrs-gui-example") };
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
```

`beside_this_program`, in the example's source, finds the renderer's helper in the build's profile folder.

Every wait takes an arm: a function given the wait's `Canceler` just before the wait begins, which returns a guard; the wait ends when anything calls `cancel()` on that canceler, from any thread. A PowerShell command passes the canceler to the pipeline's stop, so Ctrl+C ends the wait. This program has nothing to end its waits, so its arm stores nothing. `Window::canceler()` gives a window's own, for `next_event`.

## Run it

Build the helper with the same profile, then run the example. With `--self-test` it makes the choice itself through `Session::test_event`, as a test would; without it, it waits for a person:

```text
cargo build --profile test-fast -p pwrs-gui --bin pwrs-gui-egui
cargo run --profile test-fast -p pwrs-gui-client --example choose -- --self-test
```

```text
egui renderer on NVIDIA GeForce RTX 5070, through Dx12
window 1 opened
selected row 1
chosen: green
```

## What else a session does

The PowerShell commands are thin over the same calls:

- `Session::windows` lists the open windows, as `Get-GuiWindow` does;
- `Session::test_event` raises an event, as `Invoke-GuiTestEvent` does;
- `Session::snapshot` reads a window's tree, state and pixels, as `Get-GuiSnapshot` does;
- `Session::adapters` lists the GPU adapters, as `Get-GuiGpuAdapter` does;
- `Window::rows` and `Window::patch` add a table's rows and change a tree, as `Out-GuiGridView` and `Read-GuiChoice` do.

`Session::shut_down` closes every window and ends the renderer.

The client crate's own tests, `crates/pwrs-gui-client/tests/session.rs` and `tests/entry.rs`, drive a session this way end to end, each wait with a deadline.
