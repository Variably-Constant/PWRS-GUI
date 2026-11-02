//! Photoshop 2026's rasterizer for vector paths, as measured on its own
//! renders: each pixel is point-sampled on a grid of 17 samples across and
//! 15 down, 255 samples, and its alpha is the number covered.
//!
//! A path is drawn in four steps. Each cubic is cut at its interior x and y
//! turning points in fixed point, and each piece flattened by de Casteljau
//! halving on whole numbers of 1/32 sample, cut again while a control point
//! strays more than about a sample from the chord. The points are held to
//! the 1/32-sample grid, a fill's ties going up and a stroke's down. Every
//! edge crosses each sample row it spans where Photoshop's edge walk puts
//! it, and a sample counts between a row's crossings under the even-odd
//! rule for a fill or mask and the nonzero rule for a stroke.
//!
//! Coverage is kept per sample row as sorted runs of covered samples, so
//! subpaths combine by their operations sample by sample, and a pixel's
//! alpha is summed from the runs.

/// Samples across a pixel.
pub(crate) const ACROSS: i64 = 17;

/// Samples down a pixel.
pub(crate) const DOWN: i64 = 15;

/// Where a sample is within its cell, in samples, across and down.
const OFFSET: f64 = 31.0 / 64.0;

/// Grid steps a sample is cut into: points are held in whole numbers of
/// 1/32 sample.
const UNITS: i64 = 32;

/// Bits the parameter of a turning-point cut is held to.
const CUT_BITS: u32 = 24;

/// A point in pixels, x then y.
pub(crate) type Point = [f64; 2];

/// A cubic in pixels: its start, its two control points and its end.
pub(crate) type Cubic = [Point; 4];

/// A point in whole 1/32 samples, x then y.
pub(crate) type Units = [i64; 2];

/// A cubic in whole 1/32 samples.
type UnitCubic = [Units; 4];

/// The run of covered samples of one sample row, first and last inclusive.
pub(crate) type Run = (i64, i64);

/// How two coverages combine, sample by sample.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Combine {
    /// Covered where either is.
    Union,
    /// Covered where the first is and the second is not.
    Subtract,
    /// Covered where both are.
    Intersect,
    /// Covered where exactly one is.
    Exclude,
}

/// Which samples between a row's crossings count.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Rule {
    /// Those with an odd number of crossings to their left: fills and
    /// masks.
    EvenOdd,
    /// Those whose crossings to their left do not wind to zero, an edge
    /// going down counting one way and an edge going up the other:
    /// strokes.
    NonZero,
}

/// A point in pixels on the 1/32-sample grid, its ties going up: what a
/// fill's points are held at.
pub(crate) fn to_units(p: Point) -> Units {
    [(p[0] * (ACROSS * UNITS) as f64 + 0.5).floor() as i64, (p[1] * (DOWN * UNITS) as f64 + 0.5).floor() as i64]
}

/// A point in pixels on the 1/32-sample grid, its ties going down: what a
/// stroke's points are held at.
pub(crate) fn to_units_down(p: Point) -> Units {
    [(p[0] * (ACROSS * UNITS) as f64 - 0.5).ceil() as i64, (p[1] * (DOWN * UNITS) as f64 - 0.5).ceil() as i64]
}
/// Whether a cubic's control points are exactly on its chord, as a straight
/// segment written as a cubic has them.
fn straight(c: &Cubic) -> bool {
    let chord = [c[3][0] - c[0][0], c[3][1] - c[0][1]];
    if chord == [0.0, 0.0] {
        return true;
    }
    c[1..3].iter().all(|q| chord[0] * (q[1] - c[0][1]) - chord[1] * (q[0] - c[0][0]) == 0.0)
}

/// The parameters strictly between 0 and 1 where one coordinate of an
/// integer cubic turns, the roots of its derivative in double.
fn turning(q: &UnitCubic, axis: usize) -> Vec<f64> {
    let v = |i: usize| q[i][axis] as f64;
    let a = v(3) - 3.0 * v(2) + 3.0 * v(1) - v(0);
    let b = 2.0 * (v(2) - 2.0 * v(1) + v(0));
    let c = v(1) - v(0);
    let roots = if a == 0.0 {
        if b != 0.0 { vec![-c / b] } else { Vec::new() }
    } else {
        let disc = b * b - 4.0 * a * c;
        if disc < 0.0 { Vec::new() } else { vec![(-b - disc.sqrt()) / (2.0 * a), (-b + disc.sqrt()) / (2.0 * a)] }
    };
    roots.into_iter().filter(|&t| 0.0 < t && t < 1.0).collect()
}

/// `a` moved toward `b` by `t` of the way, `t` in 1/2^24, the step rounded
/// to the nearest whole unit with ties up.
fn lerp(a: Units, b: Units, t: i64) -> Units {
    let step = |a: i64, b: i64| a + (((b - a) * t + (1 << (CUT_BITS - 1))) >> CUT_BITS);
    [step(a[0], b[0]), step(a[1], b[1])]
}

/// An integer cubic cut at its interior x and y turning points, in order,
/// as Photoshop cuts it before flattening: each turning point's parameter
/// found in double from the integers and held to 24 bits, each cut a de
/// Casteljau step of rounded integer interpolations on what is left, at
/// the parameter rescaled to it. Where x and y turn at the same parameter
/// one cut serves both.
fn turning_pieces(q: UnitCubic) -> Vec<UnitCubic> {
    let mut cuts: Vec<f64> = turning(&q, 0).into_iter().chain(turning(&q, 1)).collect();
    cuts.sort_by(f64::total_cmp);
    cuts.dedup();
    let mut pieces = Vec::with_capacity(cuts.len() + 1);
    let mut rest = q;
    let mut done = 0.0;
    for t in cuts {
        let local = (t - done) / (1.0 - done);
        let held = (local * f64::from(1u32 << CUT_BITS) + 0.5).floor() as i64;
        let [p0, p1, p2, p3] = rest;
        let (a, b, c) = (lerp(p0, p1, held), lerp(p1, p2, held), lerp(p2, p3, held));
        let (d, e) = (lerp(a, b, held), lerp(b, c, held));
        let m = lerp(d, e, held);
        pieces.push([p0, a, d, m]);
        rest = [m, e, c, p3];
        done = t;
    }
    pieces.push(rest);
    pieces
}

/// Whether a piece is cut again: on its points cut down to whole samples,
/// when either control point's cross product with the chord exceeds the
/// chord's octagonal length, its longer side plus half its shorter.
fn too_bent(q: &UnitCubic) -> bool {
    let whole = |p: Units| [p[0].div_euclid(UNITS), p[1].div_euclid(UNITS)];
    let (p0, c0, c1, p1) = (whole(q[0]), whole(q[1]), whole(q[2]), whole(q[3]));
    let chord = [p1[0] - p0[0], p1[1] - p0[1]];
    let cross = |c: Units| (chord[0] * (c[1] - p0[1]) - chord[1] * (c[0] - p0[0])).abs();
    let (big, small) = (chord[0].abs().max(chord[1].abs()), chord[0].abs().min(chord[1].abs()));
    cross(c0).max(cross(c1)) > big + small.div_euclid(2)
}

/// The starts of a piece's chords, each piece halved while too bent, every
/// new point taken from the parent's points at once with its sum rounded
/// half up.
fn halved(q: UnitCubic, out: &mut Vec<Units>) {
    if !too_bent(&q) {
        out.push(q[0]);
        return;
    }
    let [p0, c0, c1, p1] = q;
    let a = [(p0[0] + c0[0] + 1) >> 1, (p0[1] + c0[1] + 1) >> 1];
    let f = [(p0[0] + 2 * c0[0] + c1[0] + 2) >> 2, (p0[1] + 2 * c0[1] + c1[1] + 2) >> 2];
    let m = [(p0[0] + 3 * c0[0] + 3 * c1[0] + p1[0] + 4) >> 3, (p0[1] + 3 * c0[1] + 3 * c1[1] + p1[1] + 4) >> 3];
    let g = [(c0[0] + 2 * c1[0] + p1[0] + 2) >> 2, (c0[1] + 2 * c1[1] + p1[1] + 2) >> 2];
    let e = [(c1[0] + p1[0] + 1) >> 1, (c1[1] + p1[1] + 1) >> 1];
    halved([p0, a, f, m], out);
    halved([m, g, e, p1], out);
}

/// A cubic's points before its end on the 1/32-sample grid, as Photoshop
/// flattens it: a cubic whose handles are on its anchors is its start; any
/// other is cut at its turning points (a straight one is not) and each
/// piece halved, a piece whose handles are on its ends standing for its
/// start alone.
pub(crate) fn flatten_cubic(c: &Cubic, out: &mut Vec<Units>) {
    let q = [to_units(c[0]), to_units(c[1]), to_units(c[2]), to_units(c[3])];
    if c[1] == c[0] && c[2] == c[3] {
        out.push(q[0]);
        return;
    }
    let pieces = if straight(c) { vec![q] } else { turning_pieces(q) };
    for piece in pieces {
        if piece[1] == piece[0] && piece[2] == piece[3] {
            out.push(piece[0]);
        } else {
            halved(piece, out);
        }
    }
}

/// A subpath of cubics as the ring Photoshop fills, on the 1/32-sample
/// grid: each cubic's points before its end, and for an open subpath its
/// last end, the ring closing back to its first point.
pub(crate) fn flatten(cubics: &[Cubic], closed: bool) -> Vec<Units> {
    let mut ring = Vec::new();
    for c in cubics {
        flatten_cubic(c, &mut ring);
    }
    if !closed && let Some(last) = cubics.last() {
        ring.push(to_units(last[3]));
    }
    ring
}

/// Where an edge from `upper` to `lower` (samples, top end first) crosses
/// the sample row at height `y`, by Photoshop's edge walk: the ends cut
/// down to 1/16 sample after 1/64 is added, the run across taken 1/32
/// longer than the cut ends' distance, and the crossing's offset from the
/// edge's exact top end cut down to 1/16 sample.
fn walk_crossing(upper: [f64; 2], lower: [f64; 2], y: f64) -> f64 {
    let cut = |v: f64| ((v + 1.0 / 64.0) * 16.0).floor() / 16.0;
    let dx = cut(lower[0]) - cut(upper[0]);
    let top = cut(upper[1]);
    let dy = cut(lower[1]) - top;
    let further = if dx < 0.0 { 1.0 / 32.0 } else { 0.0 };
    let size = (dx.abs() + 1.0 / 32.0) * (y - top) / dy - dx.abs() / 64.0 / dy + further;
    let sign = if dx > 0.0 {
        1.0
    } else if dx < 0.0 {
        -1.0
    } else {
        0.0
    };
    upper[0] + sign * (16.0 * size).floor() / 16.0
}

/// The covered runs of sample rows `first` to `first + count - 1`, counted
/// from the canvas's top, of rings on the 1/32-sample grid, each closed
/// implicitly: every edge that is not level crosses the rows whose sample
/// height is after its top end and at or before its bottom end, an upright
/// edge at its x and any other where the edge walk puts it; a sample
/// counts between two crossings x_k < s <= x_(k+1) when the crossings to
/// the left of the stretch pass the rule.
pub(crate) fn sample_rings(rings: &[Vec<Units>], first: i64, count: usize, rule: Rule) -> Vec<Vec<Run>> {
    let mut hits: Vec<Vec<(f64, i32)>> = vec![Vec::new(); count];
    let samples = |p: Units| [p[0] as f64 / UNITS as f64, p[1] as f64 / UNITS as f64];
    for ring in rings {
        for (i, &a) in ring.iter().enumerate() {
            let b = ring[(i + 1) % ring.len()];
            if a[1] == b[1] {
                continue;
            }
            let winding = if b[1] > a[1] { 1 } else { -1 };
            let (upper, lower) = if a[1] < b[1] { (samples(a), samples(b)) } else { (samples(b), samples(a)) };
            let top_row = (upper[1] - OFFSET).floor() as i64 + 1;
            let bottom_row = (lower[1] - OFFSET).floor() as i64;
            for row in top_row.max(first)..=bottom_row.min(first + count as i64 - 1) {
                let y = row as f64 + OFFSET;
                let x = if upper[0] == lower[0] { upper[0] } else { walk_crossing(upper, lower, y) };
                hits[(row - first) as usize].push((x, winding));
            }
        }
    }
    hits.into_iter()
        .map(|mut row| {
            row.sort_by(|p, q| p.0.total_cmp(&q.0));
            let mut runs: Vec<Run> = Vec::new();
            let mut total = 0;
            for k in 0..row.len().saturating_sub(1) {
                total += row[k].1;
                let counts = match rule {
                    Rule::EvenOdd => total % 2 != 0,
                    Rule::NonZero => total != 0,
                };
                if !counts {
                    continue;
                }
                let lo = (row[k].0 - OFFSET).floor() as i64 + 1;
                let hi = (row[k + 1].0 - OFFSET).floor() as i64;
                if lo > hi {
                    continue;
                }
                match runs.last_mut() {
                    Some(last) if last.1 + 1 >= lo => last.1 = last.1.max(hi),
                    _ => runs.push((lo, hi)),
                }
            }
            runs
        })
        .collect()
}

/// Two rows' runs combined sample by sample.
pub(crate) fn combine_runs(a: &[Run], b: &[Run], how: Combine) -> Vec<Run> {
    // Every place coverage may change, as the first sample of a stretch.
    let mut edges: Vec<i64> = a.iter().chain(b).flat_map(|&(lo, hi)| [lo, hi + 1]).collect();
    edges.sort_unstable();
    edges.dedup();
    let within = |runs: &[Run], s: i64| runs.iter().any(|&(lo, hi)| lo <= s && s <= hi);
    let mut out: Vec<Run> = Vec::new();
    for pair in edges.windows(2) {
        let (start, end) = (pair[0], pair[1] - 1);
        let (in_a, in_b) = (within(a, start), within(b, start));
        let covered = match how {
            Combine::Union => in_a || in_b,
            Combine::Subtract => in_a && !in_b,
            Combine::Intersect => in_a && in_b,
            Combine::Exclude => in_a != in_b,
        };
        if covered {
            match out.last_mut() {
                Some(last) if last.1 + 1 == start => last.1 = end,
                _ => out.push((start, end)),
            }
        }
    }
    out
}

/// Each pixel's covered samples, 0 to 255, of `rows` pixel rows and
/// `width` pixel columns from column `left`, from the runs of their sample
/// rows, 15 to a pixel row, in order.
pub(crate) fn pixel_counts(runs: &[Vec<Run>], rows: usize, left: i64, width: usize) -> Vec<u8> {
    let mut counts = vec![0u32; rows * width];
    let (first, last) = (left * ACROSS, (left + width as i64) * ACROSS - 1);
    for (r, row) in runs.iter().enumerate().take(rows * DOWN as usize) {
        let base = (r / DOWN as usize) * width;
        for &(lo, hi) in row {
            let (lo, hi) = (lo.max(first) - first, hi.min(last) - first);
            let mut s = lo;
            while s <= hi {
                let pixel = s / ACROSS;
                let end = hi.min(pixel * ACROSS + ACROSS - 1);
                counts[base + pixel as usize] += (end - s + 1) as u32;
                s = end + 1;
            }
        }
    }
    counts.into_iter().map(|c| c as u8).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A cubic written as a straight segment.
    fn line(a: Point, b: Point) -> Cubic {
        [a, a, b, b]
    }

    /// A pixel square drawn on whole pixels covers its pixels fully and
    /// nothing else.
    #[test]
    fn fills_whole_pixels_fully() {
        let ring = flatten(&[line([1.0, 1.0], [3.0, 1.0]), line([3.0, 1.0], [3.0, 3.0]), line([3.0, 3.0], [1.0, 3.0]), line([1.0, 3.0], [1.0, 1.0])], true);
        let runs = sample_rings(&[ring], 0, 5 * DOWN as usize, Rule::EvenOdd);
        let counts = pixel_counts(&runs, 5, 0, 5);
        for y in 0..5 {
            for x in 0..5 {
                let want = if (1..3).contains(&x) && (1..3).contains(&y) { 255 } else { 0 };
                assert_eq!(counts[y * 5 + x], want, "pixel {x}, {y}");
            }
        }
    }

    /// Combining runs takes each sample's own coverage: union, the first
    /// less the second, both, and exactly one.
    #[test]
    fn combines_runs_sample_by_sample() {
        let (a, b) = ([(0, 4), (10, 12)], [(3, 11)]);
        assert_eq!(combine_runs(&a, &b, Combine::Union), [(0, 12)]);
        assert_eq!(combine_runs(&a, &b, Combine::Subtract), [(0, 2), (12, 12)]);
        assert_eq!(combine_runs(&a, &b, Combine::Intersect), [(3, 4), (10, 11)]);
        assert_eq!(combine_runs(&a, &b, Combine::Exclude), [(0, 2), (5, 9), (12, 12)]);
    }

    /// The fixed-point cut leaves a cubic without turning points whole and
    /// cuts a half circle at its one turning point.
    #[test]
    fn cuts_cubics_at_their_turning_points() {
        let units = |c: Cubic| [to_units(c[0]), to_units(c[1]), to_units(c[2]), to_units(c[3])];
        let quarter = units([[0.0, 0.0], [5.0, 0.0], [10.0, 5.0], [10.0, 10.0]]);
        assert_eq!(turning_pieces(quarter).len(), 1);
        let half = units([[0.0, 0.0], [13.0, 0.0], [13.0, 20.0], [0.0, 20.0]]);
        let pieces = turning_pieces(half);
        assert_eq!(pieces.len(), 2);
        assert_eq!(pieces[0][3], pieces[1][0]);
    }
}
