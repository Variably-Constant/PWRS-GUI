//! A Chart drawn with egui_plot as an app draws one: on the window's own
//! background with no plot box, hairlines across at one step in the line
//! color and no lines down, the axes' text in the weak text color, its
//! series as solid bars without outlines, lines 2 points wide or points 8
//! points across, and a legend without a box only when it has several
//! series. It pans by dragging, zooms with Ctrl and the wheel or a box
//! dragged with the secondary button, resets at a double click, shows the
//! nearest point's values beside the pointer, and raises Select for a point
//! clicked. A chart of dates ticks its X at seconds, minutes, hours, days,
//! weeks, months and years, each label naming only what changes along the
//! axis, and offers wider steps until its labels fit.

use super::{egui, event, finite, opt_str};
use egui_plot::{Bar, BarChart, GridInput, GridMark, HoverPosition, Legend, Line, Plot, PlotPoint, PlotTransform, Points};
use pwrs_gui_proto::chart::{Chart, Series, SeriesKind, date_sample, date_tick, x_text};
use pwrs_gui_proto::{EventKind, FromHelper, Node, WindowId};

/// The steps, in seconds, a date axis's grid lines can be apart, a month its
/// average length and a year the Gregorian calendar's; [`date_steps`] says
/// which of them a chart offers.
const DATE_STEPS: &[f64] = &[
    0.001,
    0.01,
    0.1,
    1.0,
    5.0,
    15.0,
    30.0,
    60.0,
    300.0,
    900.0,
    1_800.0,
    3_600.0,
    10_800.0,
    21_600.0,
    43_200.0,
    86_400.0,
    604_800.0,
    2_629_746.0,
    31_556_952.0,
    315_569_520.0,
    3_155_695_200.0,
    31_556_952_000.0,
];

/// egui_plot's least and most gap between grid lines, in points, its own
/// defaults, set on a chart of dates so its steps can be told in points.
const GRID_SPACING: egui::Rangef = egui::Rangef { min: 8.0, max: 300.0 };

/// The gap between an X axis's labels at which egui_plot draws them at full
/// strength, in points, its default.
const LABEL_SPACING: f32 = 80.0;

/// The least gap, in points, between a chart's hairlines across.
const Y_GAP: f32 = 40.0;

/// A line series' width, in points.
const LINE_WIDTH: f32 = 2.0;

/// A points series' points' size across, in points.
const POINT_SIZE: f32 = 8.0;

/// The color egui_plot gives series `i` when none is set, which the page's
/// script gives it too: hues the golden ratio apart, saturation 0.85 and
/// value 0.5 in linear light.
fn series_color(i: usize) -> egui::Color32 {
    let golden = (5.0_f32.sqrt() - 1.0) / 2.0;
    egui::ecolor::Hsva::new((i as f32 * golden).fract(), 0.85, 0.5, 1.0).into()
}

/// A Y axis's label for `value` on lines `step` apart: as many decimals as
/// the step needs and no trailing zeros, so lines half a unit apart read
/// 1.5 and 2, never 2 and 2.
fn step_text(value: f64, step: f64) -> String {
    let decimals = if step > 0.0 && step.is_finite() { (-step.log10().floor()).max(0.0) as usize } else { 0 };
    let text = format!("{value:.decimals$}");
    if text.contains('.') { text.trim_end_matches('0').trim_end_matches('.').to_string() } else { text }
}

/// A Y axis's marks at one step, so its hairlines are all drawn alike: the
/// first of 1, 2 and 5 times a power of ten that stands at least [`Y_GAP`]
/// points apart, `input.base_step_size` being the step egui_plot's least gap
/// between lines, [`GRID_SPACING`]'s least, takes.
fn one_step(input: GridInput) -> Vec<GridMark> {
    let least = input.base_step_size * f64::from(Y_GAP / GRID_SPACING.min);
    if !(least.is_finite() && least > 0.0) {
        return Vec::new();
    }
    let power = 10_f64.powf(least.log10().floor());
    let step = [1.0, 2.0, 5.0, 10.0].into_iter().map(|m| m * power).find(|s| *s >= least).unwrap_or(10.0 * power);
    let (low, high) = input.bounds;
    let (first, last) = ((low / step).ceil() as i64, (high / step).floor() as i64);
    (first..=last).map(|k| GridMark { value: k as f64 * step, step_size: step }).collect()
}

/// Draws the chart in `node` and raises Select for a point clicked.
pub(super) fn draw_chart(ui: &mut egui::Ui, node: &Node, window: WindowId, out: &mut Vec<FromHelper>) -> Result<egui::Rect, String> {
    let chart = Chart::of(node)?;
    let dates = chart.dates;
    let mut plot = Plot::new(("pwrs-chart", window, node.id))
        .show_background(false)
        .show_grid([false, true])
        .grid_color(ui.visuals().widgets.noninteractive.bg_stroke.color)
        .grid_fade(0.0)
        .y_grid_spacer(one_step)
        .y_axis_formatter(|mark: GridMark, _range: &std::ops::RangeInclusive<f64>| step_text(mark.value, mark.step_size))
        .label_formatter(move |hover: &HoverPosition<'_>| {
            Some(match hover {
                HoverPosition::NearDataPoint { plot_name, position, .. } => format!("{plot_name}\n{}\n{}", x_text(position.x, dates), position.y),
                HoverPosition::Elsewhere { position } => format!("{}\n{}", x_text(position.x, dates), position.y),
            })
        });
    if chart.series.len() > 1 {
        plot = plot.legend(Legend::default().background_alpha(0.0));
    }
    if let Some(h) = node.props.get("height") {
        plot = plot.height(finite("height", h)? as f32);
    }
    if let Some(l) = opt_str(node, "x_label")? {
        plot = plot.x_axis_label(l.to_string());
    }
    if let Some(l) = opt_str(node, "y_label")? {
        plot = plot.y_axis_label(l.to_string());
    }
    if dates {
        // The steps offered run to one as wide as the widest label a date
        // axis writes, in the font egui_plot writes its labels in.
        let font = egui::TextStyle::Body.resolve(ui.style());
        let widest = DATE_STEPS.iter().map(|s| ui.painter().layout_no_wrap(date_sample(*s).to_owned(), font.clone(), egui::Color32::PLACEHOLDER).size().x).fold(LABEL_SPACING, f32::max);
        let reach = f64::from(widest / GRID_SPACING.min);
        plot = plot.grid_spacing(GRID_SPACING).x_grid_spacer(move |input: GridInput| date_marks(input, reach)).x_axis_formatter(|mark: GridMark, _range: &std::ops::RangeInclusive<f64>| date_tick(mark.value, mark.step_size));
    }
    let weak = ui.visuals().weak_text_color();
    let shown = ui
        .scope(|ui| {
            ui.visuals_mut().override_text_color = Some(weak);
            plot.show(ui, |plot_ui| {
                for (i, s) in chart.series.iter().enumerate() {
                    let color = series_color(i);
                    let points: Vec<[f64; 2]> = s.points.iter().map(|p| [p.x, p.y]).collect();
                    match s.kind {
                        SeriesKind::Line => plot_ui.line(Line::new(s.name.clone(), points).color(color).width(LINE_WIDTH)),
                        SeriesKind::Points => plot_ui.points(Points::new(s.name.clone(), points).color(color).filled(true).radius(POINT_SIZE / 2.0)),
                        SeriesKind::Bar => {
                            let bars = s.points.iter().map(|p| Bar::new(p.x, p.y).fill(color).stroke(egui::Stroke::NONE)).collect();
                            plot_ui.bar_chart(BarChart::new(s.name.clone(), bars).color(color).width(bar_width(s)));
                        }
                    }
                }
            })
        })
        .inner;
    let r = &shown.response;
    if r.clicked()
        && let Some(at) = r.interact_pointer_pos()
        && let Some((series, index)) = point_at(&chart, &shown.transform, at, ui.style().interaction.interact_radius)
        && let Some(value) = chart.clicked(series, index)
    {
        event(out, window, node.id, EventKind::Select, value);
    }
    Ok(r.rect)
}

/// The point under `at`, in screen points: a bar with `at` on it, or else
/// the point of a line or a set of points nearest it within `radius`,
/// egui's own reach for a pointer to act on a widget. Returns the series'
/// and the point's places in the chart.
fn point_at(chart: &Chart, transform: &PlotTransform, at: egui::Pos2, radius: f32) -> Option<(usize, usize)> {
    let mut nearest: Option<(f32, usize, usize)> = None;
    for (si, s) in chart.series.iter().enumerate() {
        let half = bar_width(s) / 2.0;
        for (pi, p) in s.points.iter().enumerate() {
            if s.kind == SeriesKind::Bar {
                let corner = transform.position_from_point(&PlotPoint::new(p.x - half, 0.0));
                let other = transform.position_from_point(&PlotPoint::new(p.x + half, p.y));
                if egui::Rect::from_two_pos(corner, other).contains(at) {
                    return Some((si, pi));
                }
                continue;
            }
            let distance = transform.position_from_point(&PlotPoint::new(p.x, p.y)).distance(at);
            if distance <= radius && nearest.is_none_or(|(d, _, _)| distance < d) {
                nearest = Some((distance, si, pi));
            }
        }
    }
    nearest.map(|(_, si, pi)| (si, pi))
}

/// A bar's width in X: half the narrowest gap between the series' X, as
/// egui_plot's bars of one unit apart are half a unit wide, or half a unit
/// when the series has no gap to measure.
fn bar_width(s: &Series) -> f64 {
    let mut xs: Vec<f64> = s.points.iter().map(|p| p.x).collect();
    xs.sort_by(f64::total_cmp);
    let gap = xs.windows(2).map(|w| w[1] - w[0]).filter(|g| *g > 0.0).fold(f64::INFINITY, f64::min);
    if gap.is_finite() { gap / 2.0 } else { 0.5 }
}

/// The steps a date axis's grid offers: of [`DATE_STEPS`], from the
/// narrowest at least `base` seconds, the least egui_plot accepts, to the
/// first at least `reach` times that, so one step's labels fit between its
/// lines; the widest alone when none is as wide as `base`.
fn date_steps(base: f64, reach: f64) -> &'static [f64] {
    let widest = DATE_STEPS.len() - 1;
    let first = DATE_STEPS.iter().position(|s| *s >= base).unwrap_or(widest);
    let last = DATE_STEPS.iter().position(|s| *s >= base * reach).unwrap_or(widest).max(first);
    &DATE_STEPS[first..=last]
}

/// A date axis's grid: a mark at each multiple of each step [`date_steps`]
/// offers within the axis's bounds, one several steps fall on taking the
/// widest, by which egui_plot draws its line and writes its label.
fn date_marks(input: GridInput, reach: f64) -> Vec<GridMark> {
    let (low, high) = input.bounds;
    let mut marks = Vec::new();
    for &step in date_steps(input.base_step_size, reach) {
        let mut at = (low / step).ceil();
        while at * step <= high {
            marks.push(GridMark { value: at * step, step_size: step });
            at += 1.0;
        }
    }
    marks.sort_by(|a, b| a.value.total_cmp(&b.value).then(b.step_size.total_cmp(&a.step_size)));
    marks.dedup_by(|later, kept| (later.value - kept.value).abs() < 0.1 * later.step_size.min(kept.step_size));
    marks
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_date_axis_offers_steps_until_its_labels_fit() {
        assert_eq!(date_steps(40.0, 16.0), [60.0, 300.0, 900.0]);
        assert_eq!(date_steps(6.4, 10.0), [15.0, 30.0, 60.0, 300.0]);
        assert_eq!(date_steps(1e12, 10.0), [31_556_952_000.0]);
    }

    #[test]
    fn a_date_axis_marks_each_multiple_once_with_the_widest_step_on_it() {
        // The seven minutes from 09:30:30 the tutorial's errors chart spans.
        let (low, high) = (1_792_488_630.0, 1_792_489_050.0);
        let marks = date_marks(GridInput { bounds: (low, high), base_step_size: 6.4 }, 10.0);
        assert!(marks.windows(2).all(|w| w[0].value < w[1].value));
        let step_at = |hms: &str| marks.iter().find(|m| date_tick(m.value, 1.0) == hms).map(|m| m.step_size);
        assert_eq!(step_at("09:35:00"), Some(300.0));
        assert_eq!(step_at("09:31:00"), Some(60.0));
        assert_eq!(step_at("09:31:30"), Some(30.0));
        assert_eq!(step_at("09:31:15"), Some(15.0));
        assert_eq!(marks.len(), 29);
    }

    #[test]
    fn bars_are_half_their_narrowest_gap_wide() {
        let s = Series {
            name: String::from("s"),
            kind: SeriesKind::Bar,
            points: [0.0, 10.0, 14.0].iter().map(|x| pwrs_gui_proto::chart::Point { x: *x, y: 1.0, written_x: pwrs_gui_proto::Value::Float(*x) }).collect(),
        };
        assert_eq!(bar_width(&s), 2.0);
    }

    /// The hairlines across stand at one step, the first of 1, 2 and 5
    /// times a power of ten at least Y_GAP points apart, from the first
    /// multiple of it inside the bounds to the last.
    #[test]
    fn a_charts_hairlines_stand_at_one_round_step() {
        let marks = one_step(GridInput { bounds: (0.05, 2.5), base_step_size: 0.03 });
        let values: Vec<f64> = marks.iter().map(|m| m.value).collect();
        assert!(marks.iter().all(|m| (m.step_size - 0.2).abs() < 1e-12), "0.03 per 8 points needs at least 0.15, so the step is 0.2");
        assert_eq!(values.len(), 12);
        assert!((values[0] - 0.2).abs() < 1e-12 && (values[11] - 2.4).abs() < 1e-9, "{values:?}");
        assert!(one_step(GridInput { bounds: (0.0, 1.0), base_step_size: 0.0 }).is_empty(), "no step to take, no marks");
    }

    /// A Y label has the decimals its step needs and no trailing zeros.
    #[test]
    fn a_y_label_has_the_decimals_its_step_needs() {
        let labels: Vec<String> = [0.0, 0.5, 1.0, 1.5, 2.0].iter().map(|v| step_text(*v, 0.5)).collect();
        assert_eq!(labels, ["0", "0.5", "1", "1.5", "2"]);
        assert_eq!(step_text(250.0, 50.0), "250");
        assert_eq!(step_text(0.04, 0.02), "0.04");
    }

    /// A series takes the color egui_plot gives it, which the page's script
    /// works out the same way: the first is rgb(188, 77, 77).
    #[test]
    fn a_series_takes_egui_plots_color() {
        assert_eq!(series_color(0), egui::Color32::from_rgb(188, 77, 77));
        assert_ne!(series_color(1), series_color(0));
    }
}
