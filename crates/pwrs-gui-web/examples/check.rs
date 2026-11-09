//! Drives the web renderer through its link as a session does, on this
//! process's first thread: opens a choice window, raises events by name,
//! reads its state, captures its pixels and one widget's, patches it, lists
//! it, closes it and shuts the renderer down. Prints PASS or FAIL for each
//! step, and exits 1 at the first failure. Every wait has a deadline, so a
//! renderer that stops replying fails a step rather than stalling the run,
//! and a failed step shuts the renderer down.
//!
//! `cargo run -p pwrs-gui-web --example check`

use pwrs_gui_proto as proto;
use pwrs_gui_proto::renderer::{Inbound, Link, Sink, WakeLoop};
use pwrs_gui_proto::{CloseReason, EventKind, FromHelper, Kind, Node, PatchOp, ToHelper, Value, WindowOptions};
use std::any::Any;
use std::collections::BTreeMap;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

const DEADLINE: Duration = Duration::from_secs(30);

/// Where each message went: `start`, a ring's name, or `log`.
struct ChannelSink(Sender<(String, FromHelper)>);

impl ChannelSink {
    fn put(&self, route: &str, msg: FromHelper) {
        if self.0.send((route.to_string(), msg)).is_err() {
            eprintln!("check: the driver has stopped reading");
        }
    }
}

impl Sink for ChannelSink {
    fn attach(&mut self, ring: &str, _attach: Box<dyn Any + Send>) {
        eprintln!("check: {ring} came with an attachment, which this check never sends");
    }
    fn hello(&mut self, msg: FromHelper) {
        self.put("start", msg);
    }
    fn window(&mut self, ring: &str, msg: FromHelper) {
        self.put(ring, msg);
    }
    fn reply(&mut self, ring: &str, msg: FromHelper) {
        self.put(ring, msg);
    }
    fn forget(&mut self, _ring: &str) {}
    fn log(&mut self, msg: FromHelper) {
        self.put("log", msg);
    }
}

struct Driver {
    tx: Sender<Inbound>,
    rx: Receiver<(String, FromHelper)>,
    wake: Arc<OnceLock<WakeLoop>>,
}

impl Driver {
    fn send(&self, msg: ToHelper) -> Result<(), String> {
        self.tx.send(Inbound::plain(msg)).map_err(|e| format!("the renderer stopped taking messages: {e}"))?;
        match self.wake.get() {
            Some(w) => {
                w();
                Ok(())
            }
            None => Err(String::from("the renderer never gave its wake")),
        }
    }

    /// The next message on `route`, skipping log lines and printing them.
    fn next(&self, route: &str) -> Result<FromHelper, String> {
        let end = Instant::now() + DEADLINE;
        loop {
            let left = end.saturating_duration_since(Instant::now());
            match self.rx.recv_timeout(left) {
                Ok((r, msg)) if r == route => return Ok(msg),
                Ok((r, msg)) if r == "log" => eprintln!("check: log {msg:?}"),
                Ok((r, msg)) => return Err(format!("expected a message on {route}, and {msg:?} came on {r}")),
                Err(RecvTimeoutError::Timeout) => return Err(format!("nothing arrived on {route} within {DEADLINE:?}")),
                Err(RecvTimeoutError::Disconnected) => return Err(format!("the renderer ended while {route} was awaited")),
            }
        }
    }

    /// One message on each of `first` and `second`, in whichever order they
    /// arrive: an event and the reply to the request that raised it travel
    /// on different rings.
    fn pair(&self, first: &str, second: &str) -> Result<(FromHelper, FromHelper), String> {
        let end = Instant::now() + DEADLINE;
        let (mut a, mut b) = (None, None);
        loop {
            match (a.take(), b.take()) {
                (Some(x), Some(y)) => return Ok((x, y)),
                (x, y) => (a, b) = (x, y),
            }
            let left = end.saturating_duration_since(Instant::now());
            match self.rx.recv_timeout(left) {
                Ok((r, msg)) if r == first && a.is_none() => a = Some(msg),
                Ok((r, msg)) if r == second && b.is_none() => b = Some(msg),
                Ok((r, msg)) if r == "log" => eprintln!("check: log {msg:?}"),
                Ok((r, msg)) => return Err(format!("expected a message on {first} and one on {second}, and {msg:?} came on {r}")),
                Err(RecvTimeoutError::Timeout) => return Err(format!("a message did not come on both {first} and {second} within {DEADLINE:?}")),
                Err(RecvTimeoutError::Disconnected) => return Err(format!("the renderer ended while {first} and {second} were awaited")),
            }
        }
    }
}

/// A message for a failure line, with any pixels given by size alone.
fn described(msg: &FromHelper) -> String {
    match msg {
        FromHelper::Snapshot { request, outcome: Ok(s) } => match &s.pixels {
            Some(p) => format!("snapshot {request}: {}x{} with {} bytes", p.width, p.height, p.rgba.len()),
            None => format!("snapshot {request} with no pixels"),
        },
        other => format!("{other:?}"),
    }
}

fn tree() -> Node {
    Node::new(1, Kind::Column)
        .child(Node::new(2, Kind::Text).prop("text", Value::Str("Pick one".into())))
        .child(Node::new(3, Kind::ListBox).named("Choice").prop("items", Value::List(vec![Value::Str("a".into()), Value::Str("b".into()), Value::Str("c".into())])))
        .child(Node::new(4, Kind::Row).child(Node::new(5, Kind::Button).named("Ok").prop("text", Value::Str("OK".into()))))
}

fn steps(d: &Driver) -> Result<(), String> {
    let step = |name: &str, r: Result<(), String>| -> Result<(), String> {
        match &r {
            Ok(()) => println!("PASS {name}"),
            Err(e) => println!("FAIL {name}: {e}"),
        }
        r
    };
    step("hello", match d.next("start")? {
        FromHelper::Hello { renderer, capabilities, .. } if renderer == "web" => {
            println!("  engine {} {}", capabilities.backend, capabilities.adapter);
            Ok(())
        }
        other => Err(format!("{other:?}")),
    })?;
    d.send(ToHelper::OpenWindow { window: 1, ring: "w1".into(), options: WindowOptions::titled("PWRS-GUI web check"), tree: tree(), rows: Vec::new() })?;
    step("opened", match d.next("w1")? {
        FromHelper::Opened { window: 1 } => Ok(()),
        other => Err(format!("{other:?}")),
    })?;
    d.send(ToHelper::TestEvent { request: 1, reply: "r1".into(), window: 1, name: "choice".into(), kind: EventKind::Select, value: Value::Int(2) })?;
    let (event, answer) = d.pair("w1", "r1")?;
    step("select raises the event", match event {
        FromHelper::Event { window: 1, node: 3, kind: EventKind::Select, value, .. } if value == Value::List(vec![Value::Int(2)]) => Ok(()),
        other => Err(format!("{other:?}")),
    })?;
    step("select replies with its value", match answer {
        FromHelper::TestEventDone { request: 1, outcome: Ok(v) } if v == Value::List(vec![Value::Int(2)]) => Ok(()),
        other => Err(format!("{other:?}")),
    })?;
    d.send(ToHelper::TestEvent { request: 2, reply: "r2".into(), window: 1, name: "Ok".into(), kind: EventKind::Click, value: Value::Null })?;
    step("click", match d.pair("w1", "r2")? {
        (FromHelper::Event { node: 5, kind: EventKind::Click, .. }, FromHelper::TestEventDone { request: 2, outcome: Ok(Value::Null) }) => Ok(()),
        other => Err(format!("{other:?}")),
    })?;
    d.send(ToHelper::TestEvent { request: 3, reply: "r3".into(), window: 1, name: "Nope".into(), kind: EventKind::Click, value: Value::Null })?;
    step("a missing widget is refused", match d.next("r3")? {
        FromHelper::TestEventDone { request: 3, outcome: Err(e) } if e.contains("no widget named Nope") => Ok(()),
        other => Err(format!("{other:?}")),
    })?;
    d.send(ToHelper::Snapshot { request: 4, reply: "r4".into(), window: 1, name: None, pixels: false })?;
    step("snapshot has the selection", match d.next("r4")? {
        FromHelper::Snapshot { request: 4, outcome: Ok(s) } if s.state.get(&3) == Some(&Value::List(vec![Value::Int(2)])) => Ok(()),
        other => Err(format!("{other:?}")),
    })?;
    d.send(ToHelper::Snapshot { request: 7, reply: "r7".into(), window: 1, name: None, pixels: true })?;
    let whole = match d.next("r7")? {
        FromHelper::Snapshot { request: 7, outcome: Ok(proto::Snapshot { pixels: Some(p), .. }) } => p,
        FromHelper::Snapshot { outcome: Err(e), .. } => return step("pixels of the window", Err(e)),
        other => return step("pixels of the window", Err(described(&other))),
    };
    step("pixels of the window", match (whole.width, whole.height) {
        (w, h) if w > 0 && h > 0 && whole.rgba.len() == (w as usize) * (h as usize) * 4 => {
            println!("  window {w}x{h}");
            Ok(())
        }
        (w, h) => Err(format!("{w}x{h} with {} bytes", whole.rgba.len())),
    })?;
    d.send(ToHelper::Snapshot { request: 8, reply: "r8".into(), window: 1, name: Some("Ok".into()), pixels: true })?;
    step("pixels of one widget", match d.next("r8")? {
        FromHelper::Snapshot { request: 8, outcome: Ok(proto::Snapshot { pixels: Some(p), .. }) } if p.width < whole.width && p.height < whole.height && p.rgba.len() == (p.width as usize) * (p.height as usize) * 4 => {
            println!("  button {}x{}", p.width, p.height);
            Ok(())
        }
        other => Err(described(&other)),
    })?;
    let props = BTreeMap::from([(String::from("text"), Value::Str("Changed".into()))]);
    d.send(ToHelper::Patch { window: 1, ops: vec![PatchOp::SetProps { node: 2, props }] })?;
    d.send(ToHelper::Snapshot { request: 5, reply: "r5".into(), window: 1, name: None, pixels: false })?;
    step("a patch reaches the tree", match d.next("r5")? {
        FromHelper::Snapshot { request: 5, outcome: Ok(s) } if s.tree.find(2).and_then(|n| n.props.get("text")) == Some(&Value::Str("Changed".into())) => Ok(()),
        other => Err(format!("{other:?}")),
    })?;
    d.send(ToHelper::ListWindows { request: 6, reply: "r6".into() })?;
    step("list windows", match d.next("r6")? {
        FromHelper::Windows { request: 6, windows } if windows.len() == 1 && windows[0].title == "PWRS-GUI web check" && windows[0].names == ["Choice", "Ok"] => Ok(()),
        other => Err(format!("{other:?}")),
    })?;
    d.send(ToHelper::CloseWindow { window: 1 })?;
    step("close", match d.next("w1")? {
        FromHelper::Closed { window: 1, reason: CloseReason::Program } => Ok(()),
        other => Err(format!("{other:?}")),
    })?;
    d.send(ToHelper::Shutdown)
}

fn main() {
    let (tx, inbox) = mpsc::channel::<Inbound>();
    let (out, rx) = mpsc::channel::<(String, FromHelper)>();
    let wake: Arc<OnceLock<WakeLoop>> = Arc::new(OnceLock::new());
    let handed = Arc::clone(&wake);
    let on_ready = Box::new(move |w: WakeLoop| {
        if handed.set(w).is_err() {
            eprintln!("check: the renderer gave its wake twice");
        }
    });
    let driver = std::thread::spawn(move || {
        let d = Driver { tx, rx, wake };
        let driven = steps(&d);
        // A failed step leaves its window open, and the renderer runs until
        // it is told to stop.
        if driven.is_err()
            && let Err(e) = d.send(ToHelper::Shutdown)
        {
            eprintln!("check: the renderer could not be told to stop: {e}");
        }
        driven
    });
    let ran = pwrs_gui_web::run(Link { inbox, sink: Box::new(ChannelSink(out)), on_ready }, pwrs_gui_web::Settings::default());
    let driven = match driver.join() {
        Ok(r) => r,
        Err(_panicked) => Err(String::from("the driver panicked")),
    };
    match (ran, driven) {
        (Ok(()), Ok(())) => println!("CHECK passed"),
        (ran, driven) => {
            println!("CHECK failed: renderer {ran:?}, driver {driven:?}");
            std::process::exit(1);
        }
    }
}
