//! The date and file choosers. A DatePicker is a field naming the day
//! chosen, in a text field's look with a drop-down's triangle, which opens
//! a calendar of a month, its days outside the chooser's minimum and
//! maximum disabled, and the hour and minute beside it. A FilePicker shows the paths chosen beside a button that opens the
//! system's dialog through pwrs-gui-dialogs; the dialog's answer lands
//! where the window's next frame takes it, so nothing waits on it, and the
//! dialog stays open only while the window is.

use super::egui::AtomExt as _;
use super::{Inputs, egui, event, flag, kept, look, standin};
use arc_swap::ArcSwapOption;
use pwrs_gui_proto::date::{self, Minute};
use pwrs_gui_proto::{DialogAnswer, DialogFailure, EventKind, FromHelper, Node, NodeId, Value, WindowId};
use std::sync::Arc;

/// What a date chooser stores: the minute chosen, none until one is, and
/// the month its calendar shows, the chosen one's until a person moves it.
#[derive(Clone, Copy, Debug, Default)]
pub(super) struct DateState {
    pub chosen: Option<Minute>,
    pub shown: Option<(i64, i64)>,
}

/// How a dialog was answered, or why it gave no answer.
type Outcome = Result<DialogAnswer, DialogFailure>;

/// A file chooser's open dialog, and where its answer lands once it
/// closes. Dropping it, as when the window closes, closes the dialog.
pub(super) struct Picking {
    _dialog: pwrs_gui_dialogs::Owned,
    answer: Arc<ArcSwapOption<Outcome>>,
}

/// What a file chooser stores: the paths chosen, its dialog while one is
/// open, and whether a test pressed Browse, which the window's next frame
/// presses as the button would.
#[derive(Default)]
pub(super) struct FileState {
    pub paths: Vec<String>,
    pub dialog: Option<Picking>,
    pub browse: bool,
}

/// The minute a date property names; none when the node lacks it or it is
/// null.
pub(super) fn minute(node: &Node, key: &str) -> Result<Option<Minute>, String> {
    match node.props.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Str(text)) => Minute::parse(text).map(Some).map_err(|e| format!("{key}: {e}")),
        Some(other) => Err(format!("{key} must be a date written as text, not {other:?}")),
    }
}

/// The paths in a file chooser's `paths`; none when it lacks them.
pub(super) fn paths(node: &Node) -> Result<Vec<String>, String> {
    match node.props.get("paths") {
        None | Some(Value::Null) => Ok(Vec::new()),
        Some(Value::List(items)) => items.iter().map(|p| p.as_str().map(str::to_string).ok_or_else(|| format!("each path must be text, not {p:?}"))).collect(),
        Some(other) => Err(format!("paths must be a list of text, not {other:?}")),
    }
}

/// Draws a date chooser and raises Change with the minute a person chose,
/// moved into the chooser's range. The day is a field in a text field's
/// look naming it, with a drop-down's triangle, a drop-down's width at
/// least, or under a stretched layout the row's width less the hour and
/// minute beside it. The focus Set-GuiFocus gives it goes to the day.
pub(super) fn draw_date(ui: &mut egui::Ui, node: &Node, inputs: &mut Inputs, window: WindowId, out: &mut Vec<FromHelper>) -> Result<egui::Rect, String> {
    let enabled = flag(node, "enabled", true)?;
    let (low, high) = (minute(node, "minimum")?, minute(node, "maximum")?);
    let take = inputs.take_focus(node.id);
    let state = kept(&mut inputs.dates, node)?;
    let mut changed = false;
    let stretch = ui.layout().horizontal_justify() && !ui.is_sizing_pass();
    let r = ui.add_enabled_ui(enabled, |ui| {
        ui.horizontal(|ui| {
            look::well(ui, |ui| {
                let text = match state.chosen {
                    Some(m) => format!("{:04}-{:02}-{:02}", m.year, m.month, m.day),
                    None => String::from("Choose a date"),
                };
                let mut width = ui.spacing().combo_width;
                if stretch {
                    let time = if state.chosen.is_some() { time_width(ui) } else { 0.0 };
                    width = width.max(ui.available_width() - time);
                }
                let height = ui.spacing().interact_size.y;
                let button = ui.add(egui::Button::new((egui::RichText::new(text), down_arrow(ui.spacing().icon_width))).min_size(egui::vec2(width, height)));
                if take {
                    button.request_focus();
                }
                egui::Popup::from_toggle_button_response(&button).close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside).show(|ui| {
                    if calendar(ui, node.id, state, low, high) {
                        changed = true;
                        ui.close();
                    }
                });
                if let Some(m) = state.chosen.as_mut() {
                    let two = |n: f64, _: std::ops::RangeInclusive<usize>| format!("{n:02}");
                    let hours = ui.add(egui::DragValue::new(&mut m.hour).range(0..=23).custom_formatter(two));
                    ui.label(":");
                    let minutes = ui.add(egui::DragValue::new(&mut m.minute).range(0..=59).custom_formatter(two));
                    changed |= hours.changed() || minutes.changed();
                }
            })
        })
    });
    if changed && let Some(m) = state.chosen.as_mut() {
        *m = within(*m, low, high);
        event(out, window, node.id, EventKind::Change, Value::Str(m.written()));
    }
    Ok(r.response.rect)
}

/// The width the hour and the minute take beside a chosen day: two fields
/// of two digits, each a control's width at least as a drag value is, and
/// the colon between them, an item's gap apart and from the day.
fn time_width(ui: &egui::Ui) -> f32 {
    let spacing = ui.spacing();
    let measure = |text: &str, style: egui::TextStyle| egui::WidgetText::from(text).into_galley(ui, Some(egui::TextWrapMode::Extend), f32::INFINITY, style).size().x;
    let field = (measure("00", egui::TextStyle::Button) + 2.0 * spacing.button_padding.x).max(spacing.interact_size.x);
    2.0 * field + measure(":", egui::TextStyle::Body) + 3.0 * spacing.item_spacing.x
}

/// A drop-down's triangle, as egui's combo box paints it, pointing down in
/// the text's color at the right end of the button it is put in.
fn down_arrow(size: f32) -> egui::Atom<'static> {
    egui::Atom::paint(egui::Vec2::splat(size), |ui, args| {
        let rect = egui::Rect::from_center_size(args.rect.center(), egui::vec2(args.rect.width() * 0.7, args.rect.height() * 0.45));
        ui.painter().add(egui::Shape::rotated_triangle(rect, 0.0, args.fallback_text_color));
    })
    .atom_grow(true)
    .atom_align(egui::Align2::RIGHT_CENTER)
}

/// `m` moved into `low` to `high`, to the nearer end when outside.
pub(super) fn within(m: Minute, low: Option<Minute>, high: Option<Minute>) -> Minute {
    match (low, high) {
        (Some(l), _) if m < l => l,
        (_, Some(h)) if m > h => h,
        _ => m,
    }
}

/// Draws the calendar of the month `state` shows, its days outside `low`
/// to `high` disabled, and returns whether a day was chosen, which leaves
/// the hour and minute chosen before as they were, or midnight when none
/// was.
fn calendar(ui: &mut egui::Ui, node: NodeId, state: &mut DateState, low: Option<Minute>, high: Option<Minute>) -> bool {
    let (mut year, mut month) = match (state.shown, state.chosen, low) {
        (Some(shown), _, _) => shown,
        (None, Some(m), _) | (None, None, Some(m)) => (m.year, m.month),
        (None, None, None) => this_month(),
    };
    ui.horizontal(|ui| {
        if ui.add_enabled((year, month) > (1, 1), egui::Button::new("<")).clicked() {
            (year, month) = if month == 1 { (year - 1, 12) } else { (year, month - 1) };
        }
        ui.label(format!("{year:04}-{month:02}"));
        if ui.add_enabled((year, month) < (9999, 12), egui::Button::new(">")).clicked() {
            (year, month) = if month == 12 { (year + 1, 1) } else { (year, month + 1) };
        }
    });
    state.shown = Some((year, month));
    let first = date::first_weekday(year, month);
    let mut picked = None;
    egui::Grid::new(("pwrs-calendar", node)).show(ui, |ui| {
        for name in ["Mo", "Tu", "We", "Th", "Fr", "Sa", "Su"] {
            ui.label(name);
        }
        ui.end_row();
        for _ in 0..first {
            ui.label("");
        }
        for day in 1..=date::days_in_month(year, month) {
            let start = Minute { year, month, day, hour: 0, minute: 0 };
            let end = Minute { year, month, day, hour: 23, minute: 59 };
            let allowed = low.is_none_or(|l| end >= l) && high.is_none_or(|h| start <= h);
            let chosen = state.chosen.is_some_and(|m| (m.year, m.month, m.day) == (year, month, day));
            if ui.add_enabled(allowed, egui::Button::selectable(chosen, format!("{day}"))).clicked() {
                picked = Some(day);
            }
            if (first + day) % 7 == 0 {
                ui.end_row();
            }
        }
    });
    let Some(day) = picked else {
        return false;
    };
    let (hour, minute) = state.chosen.map_or((0, 0), |m| (m.hour, m.minute));
    state.chosen = Some(Minute { year, month, day, hour, minute });
    true
}

/// The year and month of now, counted on the Unix clock.
fn this_month() -> (i64, i64) {
    let seconds = match std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH) {
        Ok(since) => since.as_secs_f64(),
        Err(before) => -before.duration().as_secs_f64(),
    };
    let now = Minute::of(date::from_unix_seconds(seconds));
    (now.year, now.month)
}

/// Draws a file chooser: its paths, or what it has none of, beside the
/// button that opens its dialog, which waits while one is open. The focus
/// Set-GuiFocus gives it goes to that button.
pub(super) fn draw_file(ui: &mut egui::Ui, node: &Node, inputs: &mut Inputs) -> Result<egui::Rect, String> {
    let enabled = flag(node, "enabled", true)?;
    let folder = flag(node, "folder", false)?;
    let focus = !inputs.no_focus;
    let stand_ins = inputs.stand_ins.clone();
    let take = inputs.take_focus(node.id);
    let state = kept(&mut inputs.files, node)?;
    let shown = match state.paths.as_slice() {
        [] if folder => String::from("No folder chosen"),
        [] => String::from("No file chosen"),
        [one] => one.clone(),
        [first, rest @ ..] => format!("{first} and {} more", rest.len()),
    };
    let waiting = state.dialog.is_some();
    let mut browse = false;
    let r = ui.add_enabled_ui(enabled, |ui| {
        ui.horizontal(|ui| {
            ui.label(shown);
            let button = ui.add_enabled(!waiting, egui::Button::new("Browse…"));
            if take {
                button.request_focus();
            }
            browse = button.clicked();
        })
    });
    let pressed = std::mem::take(&mut state.browse);
    if (browse || pressed) && state.dialog.is_none() {
        state.dialog = Some(open(ui.ctx(), node, &state.paths, focus, stand_ins.as_ref())?);
    }
    Ok(r.response.rect)
}

/// Opens the system's dialog for a file chooser, starting where the first
/// of its chosen paths, `chosen`, is, without taking the focus when
/// `focus` is false, or where the system has none of the kind, the
/// renderer's own through `stand_ins`; its answer lands in the [`Picking`]
/// and repaints the window, whose next frame takes it.
fn open(ctx: &egui::Context, node: &Node, chosen: &[String], focus: bool, stand_ins: Option<&standin::Waiting>) -> Result<Picking, String> {
    let (title, kind) = pwrs_gui_proto::chooser::dialog(node, chosen)?;
    let answer = Arc::new(ArcSwapOption::empty());
    let landed = Arc::clone(&answer);
    let stand_in = stand_ins.map(|waiting| {
        let ctx = ctx.clone();
        standin::hook(waiting, move || ctx.request_repaint_of(egui::ViewportId::ROOT))
    });
    let (ctx, viewport) = (ctx.clone(), ctx.viewport_id());
    let dialog = pwrs_gui_dialogs::open(
        title,
        kind,
        focus,
        stand_in,
        Box::new(move |outcome| {
            landed.store(Some(Arc::new(outcome)));
            ctx.request_repaint_of(viewport);
        }),
    );
    Ok(Picking { _dialog: dialog, answer })
}

/// Takes the answers of the window's dialogs that have closed: a file
/// chooser whose dialog chose paths takes them and raises Change with them,
/// one whose dialog was canceled leaves what it had as it was, and one
/// whose dialog could not show does too, which the window's command is
/// told.
pub(super) fn answer_dialogs(inputs: &mut Inputs, window: WindowId, out: &mut Vec<FromHelper>) {
    for (node, state) in inputs.files.iter_mut() {
        let Some(outcome) = state.dialog.as_ref().and_then(|picking| picking.answer.swap(None)) else {
            continue;
        };
        state.dialog = None;
        match &*outcome {
            Ok(DialogAnswer::Paths(paths)) => {
                state.paths.clone_from(paths);
                event(out, window, *node, EventKind::Change, Value::List(paths.iter().cloned().map(Value::Str).collect()));
            }
            Ok(DialogAnswer::Canceled) => {}
            Ok(DialogAnswer::Button(button)) => {
                let text = format!("the file dialog of node {node} returned the button {button:?}");
                out.push(FromHelper::Error { window: Some(window), context: String::from("dialog"), text });
            }
            Err(DialogFailure::NoNative(why) | DialogFailure::Failed(why)) => {
                let text = format!("the file dialog of node {node} could not show: {why}");
                out.push(FromHelper::Error { window: Some(window), context: String::from("dialog"), text });
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_minute_is_moved_into_its_range_to_the_nearer_end() {
        let at = |day: i64, hour: i64| Minute { year: 2026, month: 10, day, hour, minute: 0 };
        let (low, high) = (Some(at(5, 9)), Some(at(20, 17)));
        assert_eq!(within(at(1, 12), low, high), at(5, 9));
        assert_eq!(within(at(25, 12), low, high), at(20, 17));
        assert_eq!(within(at(10, 12), low, high), at(10, 12));
        assert_eq!(within(at(1, 12), None, None), at(1, 12));
    }
}
