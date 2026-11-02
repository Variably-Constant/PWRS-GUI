//! Photoshop 2026's vector strokes, as measured on its own renders. A
//! stroke is drawn as rings on the 1/32-sample grid under the nonzero rule
//! (psd_raster): each straight point held to the grid with its ties going
//! down, and each arc, of a round join or cap, flattened as a fill's curves
//! are.
//!
//! The stroker draws a lone subpath of lines, segments whose handles are on
//! their anchors, about its anchors: closed and turning one way throughout,
//! with mitered corners, at any alignment; or open and centered on its
//! path, with any joins and caps.

use crate::psd_descriptor::{Descriptor, Value};
use crate::psd_raster::{self, Cubic, Point, Run, Units};
use crate::psd_vector::Drawn;

/// The largest piece, in degrees, that a round join's arc is cut into: the
/// arc goes in the fewest equal pieces no larger than this.
const JOIN_PIECE_DEGREES: f64 = 90.0;

/// Taken off a round join's arc, counted in pieces, before it is rounded
/// up, so an arc that floating point puts a hair over a whole number of
/// pieces, as it can a right angle, is not given one more: the research
/// model's choice, not a measurement of Photoshop.
const JOIN_PIECE_SLACK: f64 = 1e-9;

/// The pieces of a round cap's half circle, each a quarter circle.
const CAP_PIECES: usize = 2;

/// Which side of its path a stroke is drawn on.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Align {
    /// Within the shape, from the path in.
    Inside,
    /// Half the width each side of the path.
    Center,
    /// Outside the shape, from the path out.
    Outside,
}

/// How a stroke's outer side turns a corner.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Join {
    /// Out to where its two edges meet, or straight across where that
    /// point is further out than the miter limit allows.
    Miter,
    /// Round a circle of half the width about the corner.
    Round,
    /// Straight across.
    Bevel,
}

/// How a stroke ends an open path.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Cap {
    /// Straight across at the end.
    Butt,
    /// A half circle about the end.
    Round,
    /// Straight across half the width past the end.
    Square,
}

/// A stroke's shape: its width in pixels, which side it is drawn on, how
/// it turns corners and ends, and its miter limit, the longest a miter may
/// be as a multiple of the width.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Stroke {
    pub(crate) width: f64,
    pub(crate) align: Align,
    pub(crate) join: Join,
    pub(crate) cap: Cap,
    pub(crate) miter_limit: f64,
}

impl Stroke {
    /// A vector stroke's shape from its vstk descriptor, or why the stroker
    /// does not draw it.
    pub(crate) fn from_descriptor(d: &Descriptor) -> Result<Stroke, String> {
        let width = match d.get("strokeStyleLineWidth") {
            Some(Value::Unit(unit, v)) if unit == "#Pxl" => *v,
            Some(Value::Double(v)) => *v,
            Some(Value::Unit(unit, _)) => return Err(format!("its width is in {unit}, where the stroker takes pixels")),
            _ => return Err("it has no width".to_string()),
        };
        let named = |key: &str| match d.get(key) {
            Some(Value::Enum(_, name)) => name.as_str(),
            _ => "",
        };
        let align = match named("strokeStyleLineAlignment") {
            "strokeStyleAlignInside" => Align::Inside,
            "strokeStyleAlignCenter" => Align::Center,
            "strokeStyleAlignOutside" => Align::Outside,
            other => return Err(format!("its alignment is {other:?}")),
        };
        let join = match named("strokeStyleLineJoinType") {
            "strokeStyleMiterJoin" => Join::Miter,
            "strokeStyleRoundJoin" => Join::Round,
            "strokeStyleBevelJoin" => Join::Bevel,
            other => return Err(format!("its join is {other:?}")),
        };
        let cap = match named("strokeStyleLineCapType") {
            "strokeStyleButtCap" => Cap::Butt,
            "strokeStyleRoundCap" => Cap::Round,
            "strokeStyleSquareCap" => Cap::Square,
            other => return Err(format!("its cap is {other:?}")),
        };
        let miter_limit = crate::psd_fill::number(d, "strokeStyleMiterLimit").ok_or("it has no miter limit")?;
        if matches!(d.get("strokeStyleLineDashSet"), Some(Value::List(dashes)) if !dashes.is_empty()) {
            return Err("it is dashed".to_string());
        }
        Ok(Stroke { width, align, join, cap, miter_limit })
    }
}

/// The covered samples of the stroke of `paths` in sample rows `first` to
/// `first + count - 1`, or why the stroker does not draw it. A lone closed
/// subpath that subtracts or intersects leaves nothing to stroke, as the
/// stroke follows the subpaths combined from nothing.
pub(crate) fn sample_stroke(paths: &[Drawn], stroke: &Stroke, first: i64, count: usize) -> Result<Vec<Vec<Run>>, String> {
    let drawn: Vec<&Drawn> = paths.iter().filter(|p| !p.cubics.is_empty()).collect();
    let only = match drawn[..] {
        [] => return Ok(vec![Vec::new(); count]),
        [only] => only,
        _ => return Err("its path has more than one subpath".to_string()),
    };
    if only.closed && matches!(only.operation, 2 | 3) {
        return Ok(vec![Vec::new(); count]);
    }
    let rings = rings(only, stroke)?;
    Ok(psd_raster::sample_rings(&rings, first, count, psd_raster::Rule::NonZero))
}

/// The rings of one subpath's stroke on the 1/32-sample grid, to be
/// sampled under the nonzero rule, or why the stroker does not draw it.
fn rings(path: &Drawn, stroke: &Stroke) -> Result<Vec<Vec<Units>>, String> {
    if !path.cubics.iter().all(|c| c[1] == c[0] && c[2] == c[3]) {
        return Err("its path has curves".to_string());
    }
    if path.closed { closed_lines(&path.cubics, stroke) } else { open_lines(&path.cubics, stroke) }
}

fn plus(a: Point, b: Point) -> Point {
    [a[0] + b[0], a[1] + b[1]]
}

fn minus(a: Point, b: Point) -> Point {
    [a[0] - b[0], a[1] - b[1]]
}

fn times(a: Point, k: f64) -> Point {
    [a[0] * k, a[1] * k]
}

fn dot(a: Point, b: Point) -> f64 {
    a[0] * b[0] + a[1] * b[1]
}

fn cross(a: Point, b: Point) -> f64 {
    a[0] * b[1] - a[1] * b[0]
}

fn unit(v: Point) -> Point {
    let length = v[0].hypot(v[1]);
    [v[0] / length, v[1] / length]
}

/// A corner's two edge normals' cosine, and whether its miter is within
/// `limit`: one over the sine of half the angle between the edges, the
/// miter's length over the width, at most the limit.
fn miter(before: Point, after: Point, limit: f64) -> (f64, bool) {
    let cos = dot(before, after);
    let half_sin = ((1.0 + cos) / 2.0).max(0.0).sqrt();
    (cos, half_sin > 0.0 && 1.0 / half_sin <= limit)
}

/// Where the edges of a corner at `p`, with normals `before` and `after`
/// whose cosine is `cos`, meet when both are moved `d` along their normals.
fn tip(p: Point, before: Point, after: Point, cos: f64, d: f64) -> Point {
    let sum = plus(before, after);
    [p[0] + sum[0] * d / (1.0 + cos), p[1] + sum[1] * d / (1.0 + cos)]
}

/// The stroke of a closed subpath of lines turning one way throughout, with
/// mitered corners, about its anchors, its edges' normals pointing away
/// from the anchors' mean point. A center stroke
/// runs from tip to tip of its outer corners, beveled where a miter passes
/// the limit, and through the moved edges' crossings on its inner side,
/// except at the first knot, where both moved edges run to their own ends
/// through the knot. An inside stroke runs from the path to its inward
/// offset by the width, cornered where the moved edges cross. An outside
/// stroke runs from the outward offset, each edge from its own moved ends
/// with the miter tip between, to the path.
fn closed_lines(cubics: &[Cubic], stroke: &Stroke) -> Result<Vec<Vec<Units>>, String> {
    if stroke.join != Join::Miter {
        return Err("its closed path's corners are not mitered".to_string());
    }
    let mut p: Vec<Point> = cubics.iter().map(|c| c[0]).collect();
    p.dedup();
    while p.len() > 1 && p.first() == p.last() {
        p.pop();
    }
    let n = p.len();
    if n < 3 {
        return Ok(Vec::new());
    }
    let turns: Vec<f64> = (0..n).map(|i| cross(minus(p[(i + 1) % n], p[i]), minus(p[(i + 2) % n], p[(i + 1) % n]))).collect();
    if turns.iter().any(|&t| t > 0.0) && turns.iter().any(|&t| t < 0.0) {
        return Err("its closed path turns both ways".to_string());
    }
    let sum = p.iter().fold([0.0, 0.0], |s, &q| plus(s, q));
    let mean = [sum[0] / n as f64, sum[1] / n as f64];
    let normals: Vec<Point> = (0..n)
        .map(|i| {
            let (a, b) = (p[i], p[(i + 1) % n]);
            let t = unit(minus(b, a));
            let normal = [t[1], -t[0]];
            let middle = [(a[0] + b[0]) / 2.0, (a[1] + b[1]) / 2.0];
            if dot(normal, minus(middle, mean)) < 0.0 { [-normal[0], -normal[1]] } else { normal }
        })
        .collect();
    let down = psd_raster::to_units_down;
    let corner = |i: usize| (normals[(i + n - 1) % n], normals[i]);
    let outward = |d: f64, own_ends: bool| -> Vec<Units> {
        let mut ring = Vec::new();
        for (i, &at) in p.iter().enumerate() {
            let (before, after) = corner(i);
            let (cos, mitred) = miter(before, after, stroke.miter_limit);
            if mitred && !own_ends {
                ring.push(down(tip(at, before, after, cos, d)));
                continue;
            }
            ring.push(down(plus(at, times(before, d))));
            if mitred {
                ring.push(down(tip(at, before, after, cos, d)));
            }
            ring.push(down(plus(at, times(after, d))));
        }
        ring
    };
    let inward = |d: f64, through_first: bool| -> Vec<Units> {
        let mut ring = Vec::new();
        for (i, &at) in p.iter().enumerate() {
            let (before, after) = corner(i);
            if through_first && i == 0 {
                ring.extend([down(minus(at, times(before, d))), down(at), down(minus(at, times(after, d)))]);
            } else {
                ring.push(down(tip(at, [-before[0], -before[1]], [-after[0], -after[1]], dot(before, after), d)));
            }
        }
        ring.reverse();
        ring
    };
    let w = stroke.width;
    let path: Vec<Units> = p.iter().map(|&at| down(at)).collect();
    Ok(match stroke.align {
        Align::Center => vec![outward(w / 2.0, false), inward(w / 2.0, true)],
        Align::Inside => vec![path, inward(w, false)],
        Align::Outside => vec![outward(w, true), path.into_iter().rev().collect()],
    })
}

/// The stroke of an open subpath of lines, centered on its anchors: each
/// side the segments moved half the width along their normals, the side a
/// corner turns toward cornered where its moved edges cross when that is on
/// both segments and through the corner otherwise, and the side it turns
/// away from joined round, mitered or beveled; the ends capped.
fn open_lines(cubics: &[Cubic], stroke: &Stroke) -> Result<Vec<Vec<Units>>, String> {
    if stroke.align != Align::Center {
        return Err("its open path's stroke is not centered".to_string());
    }
    let segments: Vec<(Point, Point)> = cubics.iter().map(|c| (c[0], c[3])).filter(|(a, b)| a != b).collect();
    let (Some(&first), Some(&last)) = (segments.first(), segments.last()) else { return Ok(Vec::new()) };
    let half = stroke.width / 2.0;
    let dirs: Vec<Point> = segments.iter().map(|&(a, b)| unit(minus(b, a))).collect();
    let normals: Vec<Point> = dirs.iter().map(|d| [-d[1], d[0]]).collect();
    let down = psd_raster::to_units_down;
    let side = |sign: f64| -> Vec<Units> {
        let reach = sign * half;
        let mut points = vec![down(plus(first.0, times(normals[0], reach)))];
        for k in 1..segments.len() {
            let vertex = segments[k].0;
            let end_before = plus(vertex, times(normals[k - 1], reach));
            let start_after = plus(vertex, times(normals[k], reach));
            let turn = cross(dirs[k - 1], dirs[k]);
            if turn == 0.0 {
                points.extend([down(end_before), down(start_after)]);
            } else if (turn > 0.0) == (sign > 0.0) {
                let gap = minus(start_after, end_before);
                let (t, u) = (cross(gap, dirs[k]) / turn, cross(gap, dirs[k - 1]) / turn);
                let length = |(a, b): (Point, Point)| (b[0] - a[0]).hypot(b[1] - a[1]);
                if -length(segments[k - 1]) <= t && t <= 0.0 && 0.0 <= u && u <= length(segments[k]) {
                    points.push(down(plus(end_before, times(dirs[k - 1], t))));
                } else {
                    points.extend([down(end_before), down(vertex), down(start_after)]);
                }
            } else {
                let (before, after) = (times(normals[k - 1], sign), times(normals[k], sign));
                match stroke.join {
                    Join::Round => points.extend(flattened(&join_arc(vertex, before, after, half))),
                    Join::Miter => match miter(before, after, stroke.miter_limit) {
                        (cos, true) => points.push(down(tip(vertex, before, after, cos, half))),
                        _ => points.extend([down(end_before), down(start_after)]),
                    },
                    Join::Bevel => points.extend([down(end_before), down(start_after)]),
                }
            }
        }
        points.push(down(plus(last.1, times(normals[normals.len() - 1], reach))));
        points
    };
    let cap = |end: Point, out: Point| -> Vec<Units> {
        let across = [-out[1], out[0]];
        match stroke.cap {
            Cap::Butt => vec![down(plus(end, times(across, half))), down(minus(end, times(across, half)))],
            Cap::Square => vec![down(plus(end, times(plus(across, out), half))), down(plus(end, times(minus(out, across), half)))],
            Cap::Round => flattened(&cap_arc(end, across, half)),
        }
    };
    let back = |d: Point| [-d[0], -d[1]];
    let ring = if segments.len() == 1 {
        [cap(first.1, dirs[0]), cap(first.0, back(dirs[0]))].concat()
    } else {
        let (left, right) = (side(1.0), side(-1.0));
        let mut ring = left[1..left.len() - 1].to_vec();
        ring.extend(cap(last.1, dirs[dirs.len() - 1]));
        ring.extend(right[1..right.len() - 1].iter().rev());
        ring.extend(cap(first.0, back(dirs[0])));
        ring
    };
    Ok(vec![ring])
}

/// Arc cubics as a stroke's points: each cubic's chord starts as a fill
/// flattens them, then the last end held to the grid with its ties going
/// down.
fn flattened(arcs: &[Cubic]) -> Vec<Units> {
    let mut points = psd_raster::flatten(arcs, true);
    if let Some(last) = arcs.last() {
        points.push(psd_raster::to_units_down(last[3]));
    }
    points
}

/// The piece from angle `a0` to `a1` of the circle about `center` of
/// `radius` whose angle 0 is along `u0` and a quarter turn along `v0`, its
/// handles (4/3) tan(sweep / 4) of the radius along its tangents.
fn arc_piece(center: Point, u0: Point, v0: Point, radius: f64, a0: f64, a1: f64, sweep: f64) -> Cubic {
    let along = |a: f64| [a.cos() * u0[0] + a.sin() * v0[0], a.cos() * u0[1] + a.sin() * v0[1]];
    let tangent = |a: f64| [-a.sin() * u0[0] + a.cos() * v0[0], -a.sin() * u0[1] + a.cos() * v0[1]];
    let handle = 4.0 / 3.0 * (sweep / 4.0).tan() * radius;
    let (d0, d1, t0, t1) = (along(a0), along(a1), tangent(a0), tangent(a1));
    let start = [center[0] + radius * d0[0], center[1] + radius * d0[1]];
    let end = [center[0] + radius * d1[0], center[1] + radius * d1[1]];
    [start, [start[0] + handle * t0[0], start[1] + handle * t0[1]], [end[0] - handle * t1[0], end[1] - handle * t1[1]], end]
}

/// The arc about `center` of `radius` from `center + radius * u0` to
/// `center + radius * u1` the short way, in the fewest equal pieces of at
/// most JOIN_PIECE_DEGREES.
fn join_arc(center: Point, u0: Point, u1: Point, radius: f64) -> Vec<Cubic> {
    let total = dot(u0, u1).clamp(-1.0, 1.0).acos();
    let turn = if cross(u0, u1) > 0.0 { 1.0 } else { -1.0 };
    let v0 = [turn * -u0[1], turn * u0[0]];
    let pieces = (total.to_degrees() / JOIN_PIECE_DEGREES - JOIN_PIECE_SLACK).ceil().max(1.0);
    let sweep = total / pieces;
    (0..pieces as usize).map(|k| arc_piece(center, u0, v0, radius, k as f64 * sweep, (k + 1) as f64 * sweep, sweep)).collect()
}

/// A round cap's half circle about `center` of `radius`, from
/// `center + radius * u0` to `center - radius * u0`, bulging toward
/// (u0.y, -u0.x).
fn cap_arc(center: Point, u0: Point, radius: f64) -> Vec<Cubic> {
    let v0 = [u0[1], -u0[0]];
    let sweep = std::f64::consts::PI / CAP_PIECES as f64;
    (0..CAP_PIECES).map(|k| arc_piece(center, u0, v0, radius, k as f64 * sweep, (k + 1) as f64 * sweep, sweep)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A right-angled join is one quarter circle with the circle's exact
    /// handles, a half turn two, and a join a hair past a right angle still
    /// one.
    #[test]
    fn joins_round_in_the_fewest_quarters() {
        let quarter = join_arc([0.0, 0.0], [1.0, 0.0], [0.0, 1.0], 10.0);
        assert_eq!(quarter.len(), 1);
        let kappa = 4.0 / 3.0 * (std::f64::consts::FRAC_PI_8).tan();
        assert!((quarter[0][1][1] - 10.0 * kappa).abs() < 1e-12, "{:?}", quarter[0]);
        assert!((quarter[0][3][0]).abs() < 1e-12 && (quarter[0][3][1] - 10.0).abs() < 1e-12, "{:?}", quarter[0]);
        assert_eq!(join_arc([0.0, 0.0], [1.0, 0.0], [-1.0, 1e-9], 10.0).len(), 2);
        let past = std::f64::consts::FRAC_PI_2 + 1e-12;
        assert_eq!(join_arc([0.0, 0.0], [1.0, 0.0], [past.cos(), past.sin()], 10.0).len(), 1);
    }

    /// A round cap runs from one side's end round the far side to the
    /// other's.
    #[test]
    fn caps_round_through_the_far_side() {
        let cap = cap_arc([5.0, 5.0], [0.0, 1.0], 2.0);
        assert_eq!(cap.len(), 2);
        let (start, middle, end) = (cap[0][0], cap[0][3], cap[1][3]);
        assert!((start[0] - 5.0).abs() < 1e-12 && (start[1] - 7.0).abs() < 1e-12, "{start:?}");
        assert!((middle[0] - 7.0).abs() < 1e-12 && (middle[1] - 5.0).abs() < 1e-12, "{middle:?}");
        assert!((end[0] - 5.0).abs() < 1e-12 && (end[1] - 3.0).abs() < 1e-12, "{end:?}");
    }

    /// A centered butt-capped stroke on a horizontal line over whole pixels
    /// covers the pixels it spans fully and nothing else.
    #[test]
    fn strokes_a_line_across_whole_pixels() {
        let line = Drawn { closed: false, operation: 1, cubics: vec![[[1.0, 2.0], [1.0, 2.0], [4.0, 2.0], [4.0, 2.0]]] };
        let stroke = Stroke { width: 2.0, align: Align::Center, join: Join::Miter, cap: Cap::Butt, miter_limit: 100.0 };
        let runs = sample_stroke(&[line], &stroke, 0, 5 * psd_raster::DOWN as usize).expect("a line is stroked");
        let counts = psd_raster::pixel_counts(&runs, 5, 0, 6);
        for y in 0..5 {
            for x in 0..6 {
                let want = if (1..4).contains(&x) && (1..3).contains(&y) { 255 } else { 0 };
                assert_eq!(counts[y * 6 + x], want, "pixel {x}, {y}");
            }
        }
    }
}
