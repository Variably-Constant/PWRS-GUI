//! A chart's data as both renderers draw it: the series a `Chart` node's
//! `series` gives, with the points its `points` adds joined to them by
//! name, each point's X a number or, on a chart whose `dates` is true, a
//! date counted in seconds from the Unix epoch, the value a click on a
//! point raises, and the labels of a date axis.

use crate::{Node, Value, date};
use std::collections::BTreeMap;

/// How a series is drawn.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SeriesKind {
    Line,
    Bar,
    Points,
}

impl SeriesKind {
    fn named(name: &str) -> Result<SeriesKind, String> {
        match name {
            "line" => Ok(SeriesKind::Line),
            "bar" => Ok(SeriesKind::Bar),
            "points" => Ok(SeriesKind::Points),
            other => Err(format!("{other} is no chart type; it is line, bar or points")),
        }
    }
}

/// One point: where it is drawn, and its X as the session wrote it, which
/// a click on it reports.
#[derive(Clone, Debug, PartialEq)]
pub struct Point {
    pub x: f64,
    pub y: f64,
    pub written_x: Value,
}

/// One series: its name, how it is drawn, and its points in order.
#[derive(Clone, Debug, PartialEq)]
pub struct Series {
    pub name: String,
    pub kind: SeriesKind,
    pub points: Vec<Point>,
}

/// A chart's series, and whether its X are dates.
#[derive(Clone, Debug, PartialEq)]
pub struct Chart {
    pub series: Vec<Series>,
    pub dates: bool,
}

impl Chart {
    /// The chart in `node`: its `series`, then each of its `points`,
    /// which joins the series of its name or starts one of the chart's own
    /// type. A point without X takes its series' count of points.
    pub fn of(node: &Node) -> Result<Chart, String> {
        let dates = matches!(node.props.get("dates"), Some(Value::Bool(true)));
        let default_kind = match node.props.get("type") {
            None | Some(Value::Null) => SeriesKind::Line,
            Some(Value::Str(t)) => SeriesKind::named(t)?,
            Some(other) => return Err(format!("type is {other:?}, not a chart type")),
        };
        let mut series = Vec::new();
        for (i, s) in list(node, "series")?.iter().enumerate() {
            series.push(series_of(s, default_kind, dates).map_err(|e| format!("series {}: {e}", i + 1))?);
        }
        let mut chart = Chart { series, dates };
        for (i, p) in list(node, "points")?.iter().enumerate() {
            chart.add(p, default_kind).map_err(|e| format!("point {}: {e}", i + 1))?;
        }
        Ok(chart)
    }

    /// Adds one of a chart's `points`, a map of its `series`, `y` and `x`.
    fn add(&mut self, point: &Value, default_kind: SeriesKind) -> Result<(), String> {
        let map = point.as_map().ok_or_else(|| format!("a point is a map of its series, x and y, not {point:?}"))?;
        let name = map.get("series").and_then(Value::as_str).ok_or_else(|| format!("a point names its series as text: {point:?}"))?;
        let y = number(map.get("y")).ok_or_else(|| format!("a point's y is a number: {point:?}"))?;
        let at = match self.series.iter().position(|s| s.name == name) {
            Some(at) => at,
            None => {
                self.series.push(Series { name: name.to_string(), kind: default_kind, points: Vec::new() });
                self.series.len() - 1
            }
        };
        let s = &mut self.series[at];
        let (x, written_x) = match map.get("x") {
            None | Some(Value::Null) => {
                let counted = s.points.len() as f64;
                (counted, Value::Float(counted))
            }
            Some(given) => (x_of(given, self.dates)?, given.clone()),
        };
        s.points.push(Point { x, y, written_x });
        Ok(())
    }

    /// The value a click on the `index`th point of series `series` raises:
    /// a map of the series' name, the point's X as the session wrote it,
    /// and its Y.
    pub fn clicked(&self, series: usize, index: usize) -> Option<Value> {
        let s = self.series.get(series)?;
        let p = s.points.get(index)?;
        Some(Value::Map(BTreeMap::from([
            (String::from("series"), Value::Str(s.name.clone())),
            (String::from("x"), p.written_x.clone()),
            (String::from("y"), Value::Float(p.y)),
        ])))
    }

    /// The point a test's click names, `wanted` a map of its series, its X
    /// and, when its series has several points at that X, its Y, the keys
    /// in any case: the value the click raises. A point the chart does not
    /// have is refused.
    pub fn find(&self, wanted: &Value) -> Result<Value, String> {
        let map = wanted.as_map().ok_or_else(|| format!("a chart's point is named by a map of its Series, X and Y, not {wanted:?}"))?;
        let get = |key: &str| map.iter().find(|(k, _)| k.eq_ignore_ascii_case(key)).map(|(_, v)| v);
        let name = get("series").and_then(Value::as_str).ok_or_else(|| String::from("the point's Series is its series' name, as text"))?;
        let Some(at) = self.series.iter().position(|s| s.name == name) else {
            let names: Vec<&str> = self.series.iter().map(|s| s.name.as_str()).collect();
            return Err(format!("the chart has no series {name}; its series are {}", names.join(", ")));
        };
        let x = x_of(get("x").ok_or_else(|| String::from("the point's X names it"))?, self.dates)?;
        let y = match get("y") {
            None | Some(Value::Null) => None,
            Some(v) => Some(number(Some(v)).ok_or_else(|| format!("the point's Y is a number, not {v:?}"))?),
        };
        let matching: Vec<usize> = self.series[at].points.iter().enumerate().filter(|(_, p)| p.x == x && y.is_none_or(|y| p.y == y)).map(|(i, _)| i).collect();
        match matching.as_slice() {
            [one] => self.clicked(at, *one).ok_or_else(|| format!("series {name} lost point {one}")),
            [] => Err(format!("series {name} has no point at that X{}", if y.is_some() { " and Y" } else { "" })),
            many => Err(format!("series {name} has {} points at that X; give its Y as well", many.len())),
        }
    }
}

/// The text a chart's axis or readout shows for an X: a date to the
/// second, or with its fraction when it has one, on a chart of dates, and
/// the number otherwise.
pub fn x_text(x: f64, dates: bool) -> String {
    if dates { date::full(date::from_unix_seconds(x)).replace('T', " ") } else { format!("{x}") }
}

/// A date axis's label for a grid line at `x`, `step` seconds from the
/// next, naming only what changes along the axis: the day for lines a day
/// or more apart and for a line at midnight, the time to the minute for
/// lines a minute or more apart, and the time to the second, with its
/// fraction when it has one, for closer ones.
pub fn date_tick(x: f64, step: f64) -> String {
    let full = x_text(x, true);
    let midnight = (x - (x / 86_400.0).round() * 86_400.0).abs() < step / 2.0;
    if step >= 86_400.0 || midnight {
        full.chars().take(10).collect()
    } else if step >= 60.0 {
        full.chars().skip(11).take(5).collect()
    } else {
        full.chars().skip(11).collect()
    }
}

/// The widest label [`date_tick`] writes for lines `step` seconds apart,
/// which a renderer measures to space a date axis's lines.
pub fn date_sample(step: f64) -> &'static str {
    if step >= 86_400.0 {
        "0000-00-00"
    } else if step >= 60.0 {
        "00:00"
    } else if step >= 1.0 {
        "00:00:00"
    } else {
        "00:00:00.000"
    }
}

/// A list property of the node, empty when absent or null.
fn list<'a>(node: &'a Node, key: &str) -> Result<&'a [Value], String> {
    match node.props.get(key) {
        None | Some(Value::Null) => Ok(&[]),
        Some(Value::List(items)) => Ok(items),
        Some(other) => Err(format!("{key} is {other:?}, not a list")),
    }
}

/// One series of `series`, a map of its `name`, `y`, `x` and `type`.
fn series_of(given: &Value, default_kind: SeriesKind, dates: bool) -> Result<Series, String> {
    let map = given.as_map().ok_or_else(|| format!("a series is a map of its name, x, y and type, not {given:?}"))?;
    let name = map.get("name").and_then(Value::as_str).ok_or_else(|| String::from("a series names itself as text"))?;
    let kind = match map.get("type") {
        None | Some(Value::Null) => default_kind,
        Some(Value::Str(t)) => SeriesKind::named(t)?,
        Some(other) => return Err(format!("type is {other:?}, not a chart type")),
    };
    let ys = match map.get("y") {
        Some(Value::List(ys)) => ys,
        other => return Err(format!("y is a list of numbers, not {other:?}")),
    };
    let xs = match map.get("x") {
        None | Some(Value::Null) => None,
        Some(Value::List(xs)) if xs.len() == ys.len() => Some(xs),
        Some(Value::List(xs)) => return Err(format!("x and y differ in length, {} and {}; x gives one value for each y", xs.len(), ys.len())),
        Some(other) => return Err(format!("x is a list, not {other:?}")),
    };
    let mut points = Vec::with_capacity(ys.len());
    for (i, y) in ys.iter().enumerate() {
        let y = number(Some(y)).ok_or_else(|| format!("y {} is {y:?}, not a number", i + 1))?;
        let (x, written_x) = match xs {
            Some(xs) => (x_of(&xs[i], dates)?, xs[i].clone()),
            None => (i as f64, Value::Float(i as f64)),
        };
        points.push(Point { x, y, written_x });
    }
    Ok(Series { name: name.to_string(), kind, points })
}

/// Where an X is drawn: a number as it is, or on a chart of dates a date
/// written as [`date::parse`] reads one, in seconds from the Unix epoch.
fn x_of(given: &Value, dates: bool) -> Result<f64, String> {
    match given {
        Value::Str(text) if dates => date::parse(text).map(date::unix_seconds),
        Value::Str(text) => Err(format!("x {text} is a date, and the chart's X are numbers")),
        other if dates => Err(format!("x {other:?} is no date, and the chart's X are dates")),
        other => number(Some(other)).ok_or_else(|| format!("x {other:?} is not a number")),
    }
}

/// The finite number in a value.
fn number(v: Option<&Value>) -> Option<f64> {
    v.and_then(Value::as_number).filter(|n| n.is_finite())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Kind, Node};

    fn chart(props: Vec<(&str, Value)>) -> Chart {
        let mut node = Node::new(1, Kind::Chart);
        for (k, v) in props {
            node = node.prop(k, v);
        }
        match Chart::of(&node) {
            Ok(c) => c,
            Err(why) => panic!("the chart reads: {why}"),
        }
    }

    fn map(entries: Vec<(&str, Value)>) -> Value {
        Value::Map(entries.into_iter().map(|(k, v)| (k.to_string(), v)).collect())
    }

    #[test]
    fn points_join_their_series_and_count_on_without_x() {
        let series = Value::List(vec![map(vec![("name", Value::Str("cpu".into())), ("y", Value::List(vec![Value::Float(3.0), Value::Float(5.0)]))])]);
        let points = Value::List(vec![
            map(vec![("series", Value::Str("cpu".into())), ("y", Value::Float(4.0))]),
            map(vec![("series", Value::Str("disk".into())), ("x", Value::Float(10.0)), ("y", Value::Float(1.0))]),
        ]);
        let c = chart(vec![("series", series), ("points", points), ("type", Value::Str("bar".into()))]);
        assert_eq!(c.series.len(), 2);
        assert_eq!(c.series[0].points.iter().map(|p| (p.x, p.y)).collect::<Vec<_>>(), vec![(0.0, 3.0), (1.0, 5.0), (2.0, 4.0)]);
        assert_eq!(c.series[0].kind, SeriesKind::Bar, "a series without a type takes the chart's");
        assert_eq!(c.series[1].name, "disk");
        assert_eq!(c.series[1].points[0].x, 10.0);
    }

    #[test]
    fn a_click_reports_the_x_as_written_and_a_test_finds_it_by_any_case() {
        let series = Value::List(vec![map(vec![
            ("name", Value::Str("load".into())),
            ("x", Value::List(vec![Value::Str("2026-10-08T14:30:00".into()), Value::Str("2026-10-08T14:31:00.25".into())])),
            ("y", Value::List(vec![Value::Float(0.5), Value::Float(0.7)])),
        ])]);
        let c = chart(vec![("series", series), ("dates", Value::Bool(true))]);
        assert_eq!(c.series[0].points[1].x - c.series[0].points[0].x, 60.25);
        let wanted = map(vec![("Series", Value::Str("load".into())), ("X", Value::Str("2026-10-08T14:31:00.25".into()))]);
        match c.find(&wanted) {
            Ok(v) => assert_eq!(v, map(vec![("series", Value::Str("load".into())), ("x", Value::Str("2026-10-08T14:31:00.25".into())), ("y", Value::Float(0.7))])),
            Err(why) => panic!("the point is found: {why}"),
        }
        let missing = map(vec![("Series", Value::Str("load".into())), ("X", Value::Str("2026-10-08T14:32:00".into()))]);
        assert!(c.find(&missing).is_err(), "a point the chart lacks is refused");
        assert_eq!(x_text(0.0, true), "1970-01-01 00:00:00");
    }

    #[test]
    fn a_date_axis_label_names_only_what_changes() {
        let at = 1_791_469_860.25;
        assert_eq!(date_tick(at, 86_400.0), "2026-10-08");
        assert_eq!(date_tick(at, 300.0), "14:31");
        assert_eq!(date_tick(at, 0.01), "14:31:00.25");
        assert_eq!(date_tick(1_791_417_600.0, 3_600.0), "2026-10-08", "a line at midnight names its day");
        assert_eq!(date_tick(1_791_417_630.0, 30.0), "00:00:30");
        for step in [0.001, 1.0, 60.0, 86_400.0] {
            let line = (at / step).floor() * step;
            assert!(date_tick(line, step).chars().count() <= date_sample(step).chars().count(), "a label on lines {step} s apart is no longer than its sample");
        }
    }
}
