//! A Photoshop layer's vector mask, its vmsk or vsms block, and the
//! coverage it gives each pixel, as Photoshop 2026 draws it (psd_raster):
//! each subpath flattened and point-sampled 17 by 15 times a pixel, the
//! subpaths an operation of -1 joins sampled together under the even-odd
//! rule, and those groups combined sample by sample by their operations. A
//! mask whose flags invert it covers 1 minus that. The box a vector mask
//! spans comes with it, and the box of the pixels it covers enough to show
//! at 8 bits, which a shape's gradient is laid out over.

use crate::psd::Reader;
use crate::psd_raster::{self, ACROSS, Combine, DOWN, Run};

/// A point of a path, as a fraction of the document's height and width.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Point {
    y: f64,
    x: f64,
}

/// A knot of a subpath: the control point before it, its anchor and the
/// control point after it.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Knot {
    preceding: Point,
    anchor: Point,
    leaving: Point,
}

/// A subpath: whether it is closed, its operation, -1 joining the subpath
/// before it, 0 exclude, 1 union, 2 subtract and 3 intersect, and its knots.
#[derive(Debug, PartialEq)]
struct Subpath {
    closed: bool,
    operation: i16,
    knots: Vec<Knot>,
}

/// A vector mask as its block holds it: whether its flags disable or invert
/// it, its initial fill rule, and its subpaths.
#[derive(Debug, PartialEq)]
pub(crate) struct VectorMask {
    pub(crate) disabled: bool,
    /// Whether the mask covers 1 minus what its paths draw.
    pub(crate) inverted: bool,
    /// Whether the initial fill rule is 1, which, for a mask of no paths,
    /// covers every pixel.
    initial_fill: bool,
    paths: Vec<Subpath>,
}

/// A path point's two 8.24 fixed-point numbers, y then x.
fn point(r: &mut Reader) -> Result<Point, String> {
    let y = f64::from(r.signed32()?) / f64::from(1u32 << 24);
    let x = f64::from(r.signed32()?) / f64::from(1u32 << 24);
    Ok(Point { y, x })
}

/// The vector mask a vmsk or vsms block holds: its version, 3, its flags,
/// whose first bit inverts it and third disables it, and its path's
/// records, 26 bytes each, a 2-byte selector and 24 of data, as psd-tools
/// reads them. A subpath's
/// length record, 0 closed and 3 open, gives its knot count and operation,
/// and that many records follow it; 1, 2, 4 and 5 are knots, 8 the initial
/// fill rule, and 6, the path fill rule, and 7, the clipboard, hold
/// nothing the mask needs.
pub(crate) fn read(data: &[u8]) -> Result<VectorMask, String> {
    let mut r = Reader::new(data);
    let version = r.number(4)?;
    if version != 3 {
        return Err(format!("a vector mask's version is {version}, where the specification gives 3"));
    }
    let flags = r.number(4)?;
    let mut mask = VectorMask { disabled: flags & 4 != 0, inverted: flags & 1 != 0, initial_fill: false, paths: Vec::new() };
    while r.left() >= 26 {
        let selector = r.number(2)?;
        match selector {
            0 | 3 => {
                let count = r.number(2)? as usize;
                let operation = r.signed16()?;
                r.take(20)?;
                let mut knots = Vec::new();
                for _ in 0..count {
                    if r.left() < 26 {
                        return Err("a vector mask's subpath is cut short".to_string());
                    }
                    match r.number(2)? {
                        1 | 2 | 4 | 5 => knots.push(Knot { preceding: point(&mut r)?, anchor: point(&mut r)?, leaving: point(&mut r)? }),
                        6..=8 => {
                            r.take(24)?;
                        }
                        other => return Err(format!("a vector mask's subpath holds a record of selector {other}, which the specification does not give")),
                    }
                }
                mask.paths.push(Subpath { closed: selector == 0, operation, knots });
            }
            8 => {
                mask.initial_fill = r.number(2)? != 0;
                r.take(22)?;
            }
            6 | 7 => {
                r.take(24)?;
            }
            other => return Err(format!("a vector mask holds a record of selector {other}, which the specification does not give")),
        }
    }
    Ok(mask)
}

impl VectorMask {
    /// The pixels of a document `width` by `height` the mask may cover any
    /// of: left, top, right and bottom, its paths' box rounded out, or the
    /// document where it covers past its paths, as an inverted mask does
    /// or one whose first subpath subtracts.
    pub(crate) fn reach(&self, width: usize, height: usize) -> [i64; 4] {
        if self.inverted || self.paths.first().is_some_and(|p| p.operation == 2) {
            return [0, 0, width as i64, height as i64];
        }
        let [l, t, r, b] = self.bbox();
        let (w, h) = (width as f64, height as f64);
        [(l * w).floor() as i64, (t * h).floor() as i64, (r * w).ceil() as i64, (b * h).ceil() as i64]
    }

    /// The box the mask's paths span, as fractions of the document: left,
    /// top, right and bottom, every cubic's extremes in, as psd-tools'
    /// VectorMask.bbox gives it; the whole document for a mask of no knots.
    pub(crate) fn bbox(&self) -> [f64; 4] {
        let (mut xs, mut ys) = (Vec::new(), Vec::new());
        for path in &self.paths {
            let knots = &path.knots;
            if knots.is_empty() {
                continue;
            }
            if knots.len() == 1 && !path.closed {
                xs.push(knots[0].anchor.x);
                ys.push(knots[0].anchor.y);
                continue;
            }
            for (k0, k1) in pairs(knots, path.closed) {
                let (x0, x1, x2, x3) = (k0.anchor.x, k0.leaving.x, k1.preceding.x, k1.anchor.x);
                let (y0, y1, y2, y3) = (k0.anchor.y, k0.leaving.y, k1.preceding.y, k1.anchor.y);
                xs.extend([x0, x3]);
                ys.extend([y0, y3]);
                xs.extend(extrema(x0, x1, x2, x3).into_iter().map(|t| bezier(x0, x1, x2, x3, t)));
                ys.extend(extrema(y0, y1, y2, y3).into_iter().map(|t| bezier(y0, y1, y2, y3, t)));
            }
        }
        if xs.is_empty() {
            return [0.0, 0.0, 1.0, 1.0];
        }
        let low = |v: &[f64]| v.iter().copied().fold(f64::INFINITY, f64::min);
        let high = |v: &[f64]| v.iter().copied().fold(f64::NEG_INFINITY, f64::max);
        [low(&xs), low(&ys), high(&xs), high(&ys)]
    }

    /// The mask's coverage of each pixel of a document `width` by `height`,
    /// row by row: all its rows, as `coverage_rows` gives them.
    #[cfg(test)]
    pub(crate) fn coverage(&self, width: usize, height: usize) -> Vec<f32> {
        self.coverage_rows(width, height, 0, height)
    }

    /// Rows `top` to `bottom` of the mask's coverage of each pixel of a
    /// document `width` by `height`, row by row, each pixel's covered
    /// samples over 255: only those rows worked out, each row the same bits
    /// whichever rows are asked for.
    pub(crate) fn coverage_rows(&self, width: usize, height: usize, top: usize, bottom: usize) -> Vec<f32> {
        let rows = bottom.min(height).saturating_sub(top);
        self.coverage_over(width, height, [0, top as i64, width as i64, (top + rows) as i64])
    }

    /// The box of the pixels the mask covers enough to show at 8 bits,
    /// above half of 1/255, on a document `width` by `height`: left, top,
    /// right and bottom. Where it covers that much a pixel no path reaches,
    /// as an inverted mask, one whose first path subtracts or one revealing
    /// all does, the pixels it shows run on past any box and it gives the
    /// canvas; otherwise its paths are rasterized over all they span, past
    /// the canvas too. `None` where it has no path of two knots or more, or
    /// covers no pixel that much.
    pub(crate) fn visible_box(&self, width: usize, height: usize) -> Option<[i64; 4]> {
        let [l, t, r, b] = self.bbox();
        let (w, h) = (width as f64, height as f64);
        let area = [(l * w).floor() as i64, (t * h).floor() as i64, (r * w).ceil() as i64, (b * h).ceil() as i64];
        let unreached = [area[0] - 1, area[1], area[0], area[1] + 1];
        if self.coverage_over(width, height, unreached)[0] * 255.0 > 0.5 {
            return Some([0, 0, width as i64, height as i64]);
        }
        if self.paths.iter().all(|p| p.knots.len() < 2) {
            return None;
        }
        let across = (area[2] - area[0]).max(0) as usize;
        let mut found: Option<[i64; 4]> = None;
        for (i, &c) in self.coverage_over(width, height, area).iter().enumerate() {
            if c * 255.0 > 0.5 {
                let (x, y) = (area[0] + (i % across) as i64, area[1] + (i / across) as i64);
                found = Some(match found {
                    None => [x, y, x + 1, y + 1],
                    Some([l, t, r, b]) => [l.min(x), t.min(y), r.max(x + 1), b.max(y + 1)],
                });
            }
        }
        found
    }

    /// The mask's coverage of each pixel of `area`, its left, top, right and
    /// bottom on a document `width` by `height`, row by row, as `coverage`
    /// gives it over the canvas.
    fn coverage_over(&self, width: usize, height: usize, area: [i64; 4]) -> Vec<f32> {
        let (across, down) = ((area[2] - area[0]).max(0) as usize, (area[3] - area[1]).max(0) as usize);
        let runs = self.sampled(width, height, area[1] * DOWN, down * DOWN as usize, [area[0] * ACROSS, area[2] * ACROSS]);
        psd_raster::pixel_counts(&runs, down, area[0], across).into_iter().map(|c| f32::from(c) / 255.0).collect()
    }

    /// The mask's covered samples in sample rows `first` to
    /// `first + count - 1` and sample columns `span[0]` to `span[1] - 1`, as
    /// `sample_paths` gives them for its subpaths in the pixels of a
    /// document `width` by `height`.
    pub(crate) fn sampled(&self, width: usize, height: usize, first: i64, count: usize, span: [i64; 2]) -> Vec<Vec<Run>> {
        let paths = self.drawn(width, height);
        sample_paths(&paths, self.initial_fill && self.paths.is_empty(), self.inverted, first, count, span)
    }

    /// The pixels of a document `width` by `height` the mask's paths pass
    /// through, each cubic walked in steps of under 1/64 pixel and an open
    /// subpath's closing line with them: the pixels its edges may cover in
    /// part. Every other pixel it covers wholly or not at all.
    #[cfg(test)]
    pub(crate) fn crossed(&self, width: usize, height: usize) -> Vec<bool> {
        let mut marked = vec![false; width * height];
        let mut walk = |at: &dyn Fn(f64) -> [f64; 2], reach: f64| {
            let steps = (reach * 64.0).ceil().max(1.0) as usize;
            for k in 0..=steps {
                let [x, y] = at(k as f64 / steps as f64);
                if x >= 0.0 && y >= 0.0 && (x as usize) < width && (y as usize) < height {
                    marked[y as usize * width + x as usize] = true;
                }
            }
        };
        let length = |a: [f64; 2], b: [f64; 2]| (b[0] - a[0]).hypot(b[1] - a[1]);
        for path in self.drawn(width, height) {
            for c in &path.cubics {
                let at = |t: f64| {
                    let u = 1.0 - t;
                    let mix = |i: usize| u * u * u * c[0][i] + 3.0 * u * u * t * c[1][i] + 3.0 * u * t * t * c[2][i] + t * t * t * c[3][i];
                    [mix(0), mix(1)]
                };
                walk(&at, length(c[0], c[1]) + length(c[1], c[2]) + length(c[2], c[3]));
            }
            if !path.closed && let (Some(first), Some(last)) = (path.cubics.first(), path.cubics.last()) {
                let (a, b) = (last[3], first[0]);
                walk(&|t| [a[0] + t * (b[0] - a[0]), a[1] + t * (b[1] - a[1])], length(a, b));
            }
        }
        marked
    }

    /// The mask's subpaths in the pixels of a document `width` by `height`:
    /// each knot's anchor and leaving handle to the next knot's preceding
    /// handle and anchor, the last back to the first where it is closed; no
    /// cubics for a subpath of fewer than two knots.
    pub(crate) fn drawn(&self, width: usize, height: usize) -> Vec<Drawn> {
        let (w, h) = (width as f64, height as f64);
        let scaled = |p: Point| [p.x * w, p.y * h];
        self.paths
            .iter()
            .map(|path| Drawn {
                closed: path.closed,
                operation: path.operation,
                cubics: if path.knots.len() > 1 {
                    pairs(&path.knots, path.closed).iter().map(|(a, b)| [scaled(a.anchor), scaled(a.leaving), scaled(b.preceding), scaled(b.anchor)]).collect()
                } else {
                    Vec::new()
                },
            })
            .collect()
    }
}

/// A subpath as it is drawn: whether it is closed, its operation, and its
/// cubics in the document's pixels, none for a subpath too short to draw.
pub(crate) struct Drawn {
    pub(crate) closed: bool,
    pub(crate) operation: i16,
    pub(crate) cubics: Vec<psd_raster::Cubic>,
}

/// The covered samples of `paths` in sample rows `first` to
/// `first + count - 1` and sample columns `span[0]` to `span[1] - 1`: every
/// sample covered where `fill_all`, and otherwise the subpaths, those an
/// operation of -1 joins sampled together under the even-odd rule,
/// combined in order (exclude, union, subtract, intersect), subtract and
/// intersect starting from full coverage where they come first. Where
/// `inverted` the samples all that leaves are covered instead.
pub(crate) fn sample_paths(paths: &[Drawn], fill_all: bool, inverted: bool, first: i64, count: usize, span: [i64; 2]) -> Vec<Vec<Run>> {
    let full: Vec<Run> = if span[1] > span[0] { vec![(span[0], span[1] - 1)] } else { Vec::new() };
    let inverse = |runs: &[Run]| psd_raster::combine_runs(&full, runs, Combine::Subtract);
    let start = if fill_all { full.clone() } else { Vec::new() };
    let mut mask = vec![start; count];
    let mut groups: Vec<Vec<&Drawn>> = Vec::new();
    for path in paths {
        match groups.last_mut() {
            Some(group) if path.operation == -1 => group.push(path),
            _ => groups.push(vec![path]),
        }
    }
    let mut first_group = true;
    for group in groups {
        let rings: Vec<Vec<psd_raster::Units>> = group.iter().filter(|p| !p.cubics.is_empty()).map(|p| psd_raster::flatten(&p.cubics, p.closed)).collect();
        let plane = psd_raster::sample_rings(&rings, first, count, psd_raster::Rule::EvenOdd);
        for (m, p) in mask.iter_mut().zip(&plane) {
            let from = || if first_group { inverse(m) } else { m.clone() };
            *m = match group[0].operation {
                0 => psd_raster::combine_runs(m, p, Combine::Exclude),
                1 => psd_raster::combine_runs(m, p, Combine::Union),
                2 => psd_raster::combine_runs(&from(), p, Combine::Subtract),
                3 => psd_raster::combine_runs(&from(), p, Combine::Intersect),
                _ => m.clone(),
            };
        }
        first_group = false;
    }
    if inverted {
        for m in &mut mask {
            *m = inverse(m);
        }
    }
    mask
}

/// A subpath's knots in the pairs its curves join: each with the next, and
/// the last with the first where it is closed.
fn pairs(knots: &[Knot], closed: bool) -> Vec<(Knot, Knot)> {
    let mut out: Vec<(Knot, Knot)> = knots.windows(2).map(|w| (w[0], w[1])).collect();
    if closed && !knots.is_empty() {
        out.push((knots[knots.len() - 1], knots[0]));
    }
    out
}

/// A cubic's value at `t` along one coordinate, its powers taken by the C
/// library's pow, as Python's ** takes them.
fn bezier(p0: f64, p1: f64, p2: f64, p3: f64, t: f64) -> f64 {
    let u = 1.0 - t;
    u.powf(3.0) * p0 + 3.0 * u.powf(2.0) * t * p1 + 3.0 * u * t.powf(2.0) * p2 + t.powf(3.0) * p3
}

/// Where between 0 and 1, both left out, a cubic along one coordinate turns,
/// as psd-tools' _bezier_extrema finds it.
fn extrema(p0: f64, p1: f64, p2: f64, p3: f64) -> Vec<f64> {
    let a = -p0 + 3.0 * p1 - 3.0 * p2 + p3;
    let b = 2.0 * (p0 - 2.0 * p1 + p2);
    let c = p1 - p0;
    let mut ts = Vec::new();
    if a.abs() < 1e-12 {
        if b.abs() > 1e-12 {
            let t = -c / b;
            if 0.0 < t && t < 1.0 {
                ts.push(t);
            }
        }
    } else {
        let disc = b * b - 4.0 * a * c;
        if disc >= -1e-12 {
            let sq = disc.max(0.0).sqrt();
            for t in [(-b + sq) / (2.0 * a), (-b - sq) / (2.0 * a)] {
                if 0.0 < t && t < 1.0 {
                    ts.push(t);
                }
            }
        }
    }
    ts
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::psd_vector_reference as r;

    /// A mask of tools/psd_vector_reference.py: its name, its block, and
    /// psd-tools' coverage and box of it.
    type Case<'a> = (&'a str, &'a [u8], &'a [u32], [f64; 4]);

    /// The masks of tools/psd_vector_reference.py, each with psd-tools'
    /// coverage and box of it.
    fn cases() -> [Case<'static>; 12] {
        [
            ("rectangle", &r::RECTANGLE_MASK, &r::RECTANGLE_COVERAGE, r::RECTANGLE_BBOX),
            ("ellipse", &r::ELLIPSE_MASK, &r::ELLIPSE_COVERAGE, r::ELLIPSE_BBOX),
            ("hole", &r::HOLE_MASK, &r::HOLE_COVERAGE, r::HOLE_BBOX),
            ("union", &r::UNION_MASK, &r::UNION_COVERAGE, r::UNION_BBOX),
            ("subtract", &r::SUBTRACT_MASK, &r::SUBTRACT_COVERAGE, r::SUBTRACT_BBOX),
            ("intersect", &r::INTERSECT_MASK, &r::INTERSECT_COVERAGE, r::INTERSECT_BBOX),
            ("exclude", &r::EXCLUDE_MASK, &r::EXCLUDE_COVERAGE, r::EXCLUDE_BBOX),
            ("subtract first", &r::SUBTRACT_FIRST_MASK, &r::SUBTRACT_FIRST_COVERAGE, r::SUBTRACT_FIRST_BBOX),
            ("open", &r::OPEN_MASK, &r::OPEN_COVERAGE, r::OPEN_BBOX),
            ("reveal all", &r::REVEAL_ALL_MASK, &r::REVEAL_ALL_COVERAGE, r::REVEAL_ALL_BBOX),
            ("outside", &r::OUTSIDE_MASK, &r::OUTSIDE_COVERAGE, r::OUTSIDE_BBOX),
            ("tiny", &r::TINY_MASK, &r::TINY_COVERAGE, r::TINY_BBOX),
        ]
    }

    /// Each vector mask psd-tools writes, read and drawn on the reference
    /// canvas, covers every pixel it holds wholly inside or outside exactly
    /// as psd-tools does, and spans psd-tools' box: every pixel no path
    /// passes through and psd-tools covers 1 or 0. (A pixel a path crosses
    /// can still get 0 from psd-tools, which takes a subtraction or an
    /// intersection of two partly covered pixels from their fractions.) The
    /// pixels its edges cross are drawn as Photoshop draws them, which
    /// psd_merge's draws_fills_as_photoshop_drew_them checks.
    #[test]
    fn covers_the_pixels_psd_tools_draws_wholly_inside_or_outside_alike() {
        for (name, block, expected, bbox) in cases() {
            let mask = read(block).expect("a vector mask");
            assert!(!mask.disabled && !mask.inverted, "{name}: the mask reads as disabled or inverted");
            let coverage = mask.coverage(r::WIDTH, r::HEIGHT);
            assert_eq!(coverage.len(), expected.len(), "{name}: the coverage has another count of pixels");
            let crossed = mask.crossed(r::WIDTH, r::HEIGHT);
            let mut whole = 0;
            for (i, (got, want)) in coverage.iter().zip(expected).enumerate() {
                let want = f32::from_bits(*want);
                if !crossed[i] && (want == 0.0 || want == 1.0) {
                    whole += 1;
                    assert_eq!(*got, want, "{name}: pixel {}, {} is {got}, where psd-tools draws it wholly {}", i % r::WIDTH, i / r::WIDTH, if want == 1.0 { "inside" } else { "outside" });
                }
            }
            assert!(whole > 0, "{name}: psd-tools draws no pixel wholly inside or outside");
            assert_eq!(mask.bbox(), bbox, "{name}: the box");
        }
    }

    /// Each of those masks with its flags' first bit set reads as inverted,
    /// covers the samples the plain mask leaves and no other, so at every
    /// pixel the two cover 255 samples between them, and spans the same
    /// box: its paths are where they were.
    #[test]
    fn covers_the_samples_its_paths_leave_where_its_flags_invert_it() {
        let samples = |c: f32| (c * 255.0).round() as i32;
        for (name, block, _, bbox) in cases() {
            let plain = read(block).expect("a vector mask").coverage(r::WIDTH, r::HEIGHT);
            let mut flipped = block.to_vec();
            flipped[7] |= 1;
            let mask = read(&flipped).expect("a vector mask");
            assert!(mask.inverted && !mask.disabled, "{name}: inverted {}, disabled {}", mask.inverted, mask.disabled);
            let coverage = mask.coverage(r::WIDTH, r::HEIGHT);
            assert_eq!(coverage.len(), plain.len(), "{name}: the coverage has another count of pixels");
            for (i, (got, base)) in coverage.iter().zip(&plain).enumerate() {
                assert_eq!(samples(*got) + samples(*base), 255, "{name}: pixel {}, {} covers {got} inverted and {base} plain", i % r::WIDTH, i / r::WIDTH);
            }
            assert_eq!(mask.bbox(), bbox, "{name}: the box");
        }
    }

    /// Any run of rows of a mask's coverage is those rows of its whole
    /// coverage, every float32 alike, inverted or not.
    #[test]
    fn gives_any_rows_of_its_coverage_alike() {
        for (name, block, _, _) in cases() {
            for flip in [false, true] {
                let mut data = block.to_vec();
                if flip {
                    data[7] |= 1;
                }
                let mask = read(&data).expect("a vector mask");
                let whole = mask.coverage(r::WIDTH, r::HEIGHT);
                for (top, bottom) in [(0, r::HEIGHT), (0, 1), (3, 9), (r::HEIGHT - 1, r::HEIGHT), (5, 5)] {
                    let rows = mask.coverage_rows(r::WIDTH, r::HEIGHT, top, bottom);
                    let want = &whole[top * r::WIDTH..bottom * r::WIDTH];
                    assert_eq!(rows.len(), want.len(), "{name}, inverted {flip}, rows {top} to {bottom}: the count of pixels");
                    assert!(rows.iter().zip(want).all(|(a, b)| a.to_bits() == b.to_bits()), "{name}, inverted {flip}, rows {top} to {bottom}");
                }
            }
        }
    }

    /// A rectangle `left`, `top`, `right` and `bottom` pixels on a canvas
    /// `width` by `height`, as a vmsk block holds it: one closed subpath of
    /// four corners, each point's fractions of the canvas in 8.24 fixed
    /// point.
    fn rectangle_block(left: f64, top: f64, right: f64, bottom: f64, width: f64, height: f64) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend(3u32.to_be_bytes());
        out.extend(0u32.to_be_bytes());
        out.extend(0u16.to_be_bytes());
        out.extend(4u16.to_be_bytes());
        out.extend(1i16.to_be_bytes());
        out.extend([0u8; 20]);
        let fixed = |v: f64| ((v * f64::from(1u32 << 24)).round() as i32).to_be_bytes();
        for (x, y) in [(left, top), (right, top), (right, bottom), (left, bottom)] {
            out.extend(1u16.to_be_bytes());
            for _ in 0..3 {
                out.extend(fixed(y / height));
                out.extend(fixed(x / width));
            }
        }
        out
    }

    /// The box of the pixels a mask covers enough to show at 8 bits, one
    /// sample or more, is the box of the pixels its coverage shows over its
    /// paths' box and two pixels round it, so it reaches past the canvas
    /// where the paths do; it is the canvas for a mask that covers past its
    /// paths; and a column the mask reaches into short of the column's
    /// first sample, 31/64 of a sample in, stays out of it, while one
    /// reaching past that sample shows.
    #[test]
    fn spans_the_pixels_it_covers_enough_to_show() {
        let (w, h) = (r::WIDTH as i64, r::HEIGHT as i64);
        let shown = |mask: &VectorMask| -> Option<[i64; 4]> {
            let [l, t, right, b] = mask.bbox();
            let area = [(l * w as f64).floor() as i64 - 2, (t * h as f64).floor() as i64 - 2, (right * w as f64).ceil() as i64 + 2, (b * h as f64).ceil() as i64 + 2];
            let across = (area[2] - area[0]) as usize;
            let mut found: Option<[i64; 4]> = None;
            for (i, &c) in mask.coverage_over(r::WIDTH, r::HEIGHT, area).iter().enumerate() {
                if c > 0.0 {
                    let (x, y) = (area[0] + (i % across) as i64, area[1] + (i / across) as i64);
                    found = Some(found.map_or([x, y, x + 1, y + 1], |[l, t, right, b]| [l.min(x), t.min(y), right.max(x + 1), b.max(y + 1)]));
                }
            }
            found
        };
        for (name, block, _, _) in cases() {
            let mask = read(block).expect("a vector mask");
            let covers_past = (mask.initial_fill && mask.paths.is_empty()) || mask.paths.first().is_some_and(|p| p.operation == 2);
            let want = if covers_past { Some([0, 0, w, h]) } else { shown(&mask) };
            assert_eq!(mask.visible_box(r::WIDTH, r::HEIGHT), want, "{name}: the box");
        }
        let (width, height) = (r::WIDTH as f64, r::HEIGHT as f64);
        let short = read(&rectangle_block(2.0, 2.0, 10.028, 6.0, width, height)).expect("a vector mask");
        assert_eq!(short.visible_box(r::WIDTH, r::HEIGHT), Some([2, 2, 10, 6]), "a column reached 0.028 of a pixel in, short of its first sample, does not show");
        let reaches = read(&rectangle_block(2.0, 2.0, 10.03, 6.0, width, height)).expect("a vector mask");
        assert_eq!(reaches.visible_box(r::WIDTH, r::HEIGHT), Some([2, 2, 11, 6]), "a column reached 0.03 of a pixel in, past its first sample, shows");
        let mut flipped = r::RECTANGLE_MASK.to_vec();
        flipped[7] |= 1;
        assert_eq!(read(&flipped).expect("a vector mask").visible_box(r::WIDTH, r::HEIGHT), Some([0, 0, w, h]), "an inverted mask covers past its paths");
    }

    /// A block of another version, or a record of a selector the
    /// specification does not give, is refused with the reason.
    #[test]
    fn refuses_what_it_does_not_read() {
        let mut other = r::RECTANGLE_MASK.to_vec();
        other[3] = 2;
        assert!(read(&other).unwrap_err().contains("version is 2"));
        let mut unknown = r::RECTANGLE_MASK.to_vec();
        unknown[9] = 9;
        assert!(read(&unknown).unwrap_err().contains("selector 9"));
    }
}
