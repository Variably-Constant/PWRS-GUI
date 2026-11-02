//! `Measure-GuiLatency`: how long a click takes to reach the module, a
//! handler script block, the pipeline and an engine-event action, on the
//! clock every process on the machine reads.

use crate::session::{self, gui_error};
use crate::tree::Builder;
use pwrs::prelude::*;
use pwrs_gui_client::proto::clock::now_ns;
use pwrs_gui_client::proto::{EventKind, Fit, Kind, NodeId, PatchOp, Value, WindowOptions};
use pwrs_gui_client::{Session, Window, WindowEvent};
use std::collections::BTreeMap;

/// The clock the renderer stamps every event with: QueryPerformanceCounter
/// on Windows, CLOCK_MONOTONIC elsewhere, in nanoseconds, the same reading
/// in every process on the machine.
#[psclass(name = "PwrsGui.Clock")]
#[derive(Clone, Default)]
pub struct Clock {}

#[psmethods]
impl Clock {
    /// Nanoseconds now on the clock events are stamped with.
    pub fn now() -> PsResult<u64> {
        Ok(now_ns())
    }
}

/// How long one click took to reach each place, in microseconds from the
/// moment the renderer took it.
#[psclass(name = "PwrsGui.Latency")]
#[derive(Clone, Default)]
pub struct Latency {
    /// The click's number, from 1.
    pub click: i64,
    /// To the module reading the click.
    pub to_module_us: f64,
    /// To a handler script block run on the pipeline thread reading the
    /// clock; null with -NoWait.
    pub to_handler_us: Option<f64>,
    /// To the command writing this record, after the handler; null with
    /// -NoWait.
    pub to_pipeline_us: Option<f64>,
    /// To an engine-event action reading the clock; set with -NoWait only.
    pub to_event_us: Option<f64>,
}

/// Microseconds from `from` to `to`, both on [`now_ns`]'s clock; negative
/// when `to` reads earlier, which no correct reading does.
fn us(from: u64, to: u64) -> f64 {
    (i128::from(to) - i128::from(from)) as f64 / 1000.0
}

/// A -NoWait measurement under way: its window, and the clicks the
/// engine-event action has recorded.
#[psclass(name = "PwrsGui.LatencyRun", mode = proxy)]
pub struct LatencyRun {
    /// The window's number.
    pub window: u64,
    /// The engine event each click raises; Unregister-Event removes the
    /// action the command subscribed to it.
    pub source_identifier: String,
    #[psfield(skip)]
    samples: Vec<Latency>,
    #[psfield(skip)]
    session: Session,
}

#[psmethods]
impl LatencyRun {
    /// Records a click's engine-event action reading the clock at
    /// `event_ns`; the action the command subscribes calls it.
    pub fn record(&mut self, click: i64, at_ns: u64, module_ns: u64, event_ns: u64) -> PsResult<()> {
        self.samples.push(Latency { click, to_module_us: us(at_ns, module_ns), to_handler_us: None, to_pipeline_us: None, to_event_us: Some(us(at_ns, event_ns)) });
        Ok(())
    }

    /// The clicks recorded so far, in order.
    pub fn samples(&self) -> PsResult<Vec<Latency>> {
        Ok(self.samples.clone())
    }

    /// Closes the window, which ends the measurement.
    pub fn close(&self) -> PsResult<()> {
        self.session.close_window(self.window).map_err(gui_error)
    }
}

/// What a click's engine event gives the action.
#[psclass(name = "PwrsGui.LatencyClick")]
#[derive(Clone, Default)]
pub struct LatencyClick {
    /// The measurement the click belongs to.
    pub run: PsObject,
    /// The click's number, from 1.
    pub click: i64,
    /// When the renderer took the click.
    pub at_ns: u64,
    /// When the module read it.
    pub module_ns: u64,
}

/// The action each -NoWait click runs, on the pipeline thread once the
/// engine is idle.
const ACTION: &str = "$d = $Event.MessageData; $d.Run.Record($d.Click, $d.AtNs, $d.ModuleNs, [PwrsGui.Clock]::Now())";

/// Opens a window with one button and measures each click on it, a
/// person's or one Invoke-GuiTestEvent sends: how long from the renderer
/// taking it until the module read it, a handler script block run on the
/// pipeline thread read the clock, and the command wrote its record.
///
/// It writes one PwrsGui.Latency per click, until -Count clicks or the
/// window closes. With -NoWait it returns a PwrsGui.LatencyRun once the
/// window has opened, and each click raises an engine event whose action
/// records how long it took to run; read them with the run's Samples().
/// Every time is on the clock the renderer stamps events with, which every
/// process on the machine reads alike, so it measures the shipped build on
/// your machine.
///
/// # Examples
/// Measure-GuiLatency -Count 5
/// $run = Measure-GuiLatency -NoWait; Wait-Event -Timeout 10; $run.Samples()
#[cmdlet(verb = "Measure", noun = "GuiLatency", output = ["PwrsGui.Latency", "PwrsGui.LatencyRun"])]
#[derive(Default)]
pub struct MeasureGuiLatency {
    /// How many clicks to measure; the window closes after the last.
    /// Without it, clicks are measured until the window is closed.
    #[param(position = 0, validate_range(1, 100000))]
    pub count: Option<i64>,
    /// Returns once the window has opened; each click raises an engine
    /// event whose action records when it ran.
    #[param]
    pub no_wait: bool,
}

impl Cmdlet for MeasureGuiLatency {
    fn process(&mut self, ps: &Pipeline<'_>) -> PsResult<()> {
        // A dialog: what to do as its main line, the clicks measured so far,
        // and the button at its bottom right, the default, so Enter clicks
        // it too.
        let mut b = Builder::new();
        let ask = b.node(Kind::Text).prop("text", Value::Str(String::from("Click the button to time each click"))).prop("style", Value::Str(String::from("instruction")));
        let status = b.named(Kind::Text, "Status").prop("text", Value::Str(status_text(0, self.count)));
        let button = b.button("Click", "Click me").prop("default", Value::Bool(true));
        let (status_id, button_id) = (status.id, button.id);
        let bar = b.node(Kind::ButtonBar).child(button);
        let tree = b.node(Kind::Column).child(ask).child(status).child(bar);
        let options = WindowOptions {
        title: String::from("PWRS-GUI latency"),
        width: None,
        height: None,
        resizable: true,
        always_on_top: false,
        close_on_escape: true,
        dialog: true,
        fit: Fit::WINDOW,
        theme: crate::options::theme(),
        hidden: false,
    };
        let session = session::widget_session(ps)?;
        let window = session.open_window(options, tree).map_err(gui_error)?;
        let marks = Marks { status: status_id, button: button_id, count: self.count };
        if self.no_wait { start_run(ps, session, window, marks) } else { measure(ps, window, marks) }
    }
}

/// The window's nodes and the clicks it is open for.
#[derive(Clone, Copy)]
struct Marks {
    status: NodeId,
    button: NodeId,
    count: Option<i64>,
}

fn status_text(clicks: i64, count: Option<i64>) -> String {
    match count {
        Some(n) => format!("Clicks measured: {clicks} of {n}"),
        None => format!("Clicks measured: {clicks}"),
    }
}

fn status_patch(marks: Marks, clicks: i64) -> PatchOp {
    PatchOp::SetProps { node: marks.status, props: BTreeMap::from([(String::from("text"), Value::Str(status_text(clicks, marks.count)))]) }
}

/// Measures clicks on the pipeline thread, writing one record each.
fn measure(ps: &Pipeline<'_>, mut window: Window, marks: Marks) -> PsResult<()> {
    let handler = PsScriptBlock(PsType::from_name("scriptblock").call_static("Create", &["[PwrsGui.Clock]::Now()".into_ps()?])?);
    let mut clicks = 0i64;
    let mut problem = None;
    while marks.count.is_none_or(|n| clicks < n) {
        let Some(event) = session::next_event(ps, &mut window)? else {
            return Ok(());
        };
        match event {
            WindowEvent::Event { node, kind: EventKind::Click, at, .. } if node == marks.button => {
                let read = now_ns();
                clicks += 1;
                let answered = handler.call(ps, &[])?;
                let handled = match answered.first() {
                    Some(reading) => u64::from_ps(reading)?,
                    None => return Err(PsError::new(ErrorCategory::InvalidResult, "GuiHandlerSilent", "the handler wrote no clock reading").terminating()),
                };
                let written = now_ns();
                ps.write(Latency { click: clicks, to_module_us: us(at, read), to_handler_us: Some(us(at, handled)), to_pipeline_us: Some(us(at, written)), to_event_us: None })?;
                window.patch(vec![status_patch(marks, clicks)]).map_err(gui_error)?;
            }
            WindowEvent::Closed(reason) => return session::closed_error(reason, problem.as_deref()).map_or(Ok(()), Err),
            WindowEvent::PageClosed(_) => return Ok(()),
            WindowEvent::Problem(p) => {
                pwrs::warning!(ps, "the latency window reported: {p}")?;
                problem = Some(p);
            }
            WindowEvent::Message { json, .. } => pwrs::warning!(ps, "the latency window, which shows no page, got a page's message: {json}")?,
            WindowEvent::Opened | WindowEvent::Event { .. } | WindowEvent::PickerWanted { .. } | WindowEvent::FileChanged { .. } => {}
        }
    }
    window.close().map_err(gui_error)
}

/// Waits for the window's first frame, then subscribes the action, passes
/// the window to a thread that raises an engine event per click, and writes
/// the run. Ctrl+C during the wait closes the window and writes nothing.
fn start_run(ps: &Pipeline<'_>, session: Session, mut window: Window, marks: Marks) -> PsResult<()> {
    let Some(before) = crate::session::first_frame(ps, &mut window)? else {
        return Ok(());
    };
    let id = window.id();
    let source = format!("PwrsGui.Latency.{}.{id}", session.id());
    let action = PsType::from_name("scriptblock").call_static("Create", &[ACTION.into_ps()?])?;
    let subscribed = ps.invoke("Register-EngineEvent", &[("SourceIdentifier", source.clone().into_ps()?), ("Action", action)])?;
    pwrs::verbose!(ps, "subscribed {} action to {source}", subscribed.len())?;
    let run = LatencyRun { window: id, source_identifier: source.clone(), samples: Vec::new(), session }.into_ps()?;
    let events = ps.events()?;
    let held = run.clone();
    std::thread::Builder::new()
        .name(format!("pwrs-gui-latency-{id}"))
        .spawn(move || raise_clicks(window, before, events, held, source, marks))
        .map_err(|e| PsError::new(ErrorCategory::ResourceUnavailable, "GuiThread", format!("the latency window's reader thread cannot start: {e}")).terminating())?;
    ps.write_object(&run)
}

/// Reads the window's clicks, `before`, the events that came before its
/// first frame, first, and raises an engine event for each, until the
/// window closes or the last of `marks.count` clicks.
fn raise_clicks(mut window: Window, before: Vec<WindowEvent>, events: PsEvents, run: PsObject, source: String, marks: Marks) {
    let mut before = before.into_iter();
    let mut clicks = 0i64;
    while marks.count.is_none_or(|n| clicks < n) {
        let next = match before.next() {
            Some(event) => Ok(event),
            None => window.next_event(),
        };
        match next {
            Ok(WindowEvent::Event { node, kind: EventKind::Click, at, .. }) if node == marks.button => {
                let module_ns = now_ns();
                clicks += 1;
                if let Err(e) = events.raise(&source, LatencyClick { run: run.clone(), click: clicks, at_ns: at, module_ns }) {
                    eprintln!("PWRS-GUI: Measure-GuiLatency could not raise {source} for click {clicks}: {e}");
                }
                if let Err(e) = window.patch(vec![status_patch(marks, clicks)]) {
                    eprintln!("PWRS-GUI: Measure-GuiLatency could not update its window: {e}");
                }
            }
            Ok(WindowEvent::Closed(reason)) => {
                if let Some(e) = crate::session::closed_error(reason, None) {
                    eprintln!("PWRS-GUI: the latency window {}: {}", window.id(), e.message);
                }
                return;
            }
            Ok(WindowEvent::PageClosed(_)) => return,
            Ok(WindowEvent::Problem(p)) => eprintln!("PWRS-GUI: the latency window reported: {p}"),
            Ok(WindowEvent::Message { json, .. }) => eprintln!("PWRS-GUI: the latency window, which shows no page, got a page's message: {json}"),
            Ok(WindowEvent::Opened | WindowEvent::Event { .. } | WindowEvent::PickerWanted { .. } | WindowEvent::FileChanged { .. }) => {}
            Err(e) => {
                eprintln!("PWRS-GUI: Measure-GuiLatency stopped reading its window: {e}");
                return;
            }
        }
    }
    if let Err(e) = window.close() {
        eprintln!("PWRS-GUI: Measure-GuiLatency could not close its window: {e}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_later_reading_is_a_positive_span_and_an_earlier_one_negative() {
        assert_eq!(us(1_000, 3_500), 2.5);
        assert_eq!(us(3_500, 1_000), -2.5);
    }

    #[test]
    fn the_status_counts_against_the_clicks_asked_for() {
        assert_eq!(status_text(2, Some(5)), "Clicks measured: 2 of 5");
        assert_eq!(status_text(7, None), "Clicks measured: 7");
    }
}
