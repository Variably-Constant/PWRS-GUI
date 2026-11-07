//! A color profile converted to sRGB pixel by pixel as littleCMS 2.19
//! converts with its optimizations off: the profile's own table for the
//! intent, read at each pixel, or its matrix and curves where moxcms's
//! conversion by them would differ from littleCMS's, and for a float
//! image, whose light may run past 1, either one to linear light in sRGB's
//! primaries. What depends on one integer sample alone is worked out once
//! for each of its values, and the results are those of working each pixel
//! out.
//!
//! The intent's table is its D2B table where the profile holds one: D2B0
//! for perceptual, D2B1 for relative colorimetric, D2B2 for saturation and
//! D2B3 for absolute colorimetric. Otherwise it is A2B0 for perceptual,
//! A2B1 for the colorimetric intents and A2B2 for saturation, or A2B0
//! where the profile lacks that one. The table's steps run in the
//! profile's order: a D2B table's curve sets of segmented curves, matrices
//! and grids of floating point values, or a lut8, lut16 or lutAtoB table's
//! curves, matrix and grid, a grid read tetrahedrally across its last
//! three inputs and linearly across any before them. The Lab or XYZ the
//! table gives goes to XYZ, a lut16's Lab rescaled from its version 2
//! encoding first, and on to sRGB as littleCMS builds it, a version 4
//! profile; so a perceptual or saturation conversion compensates the
//! profile's black point to sRGB's black, and the absolute intent scales
//! the profile's media white to sRGB's. Each rule follows littleCMS's
//! source: cmsio0.c and cmsio1.c for the tables, cmstypes.c for reading a
//! D2B table, cmsintrp.c for the grid, cmscnvrt.c and cmssamp.c for black
//! points and intents, cmsgamma.c for curves.

use std::marker::PhantomData;

use moxcms::{ColorProfile, DataColorSpace, LutDataType, LutMultidimensionalType, LutStore, LutType, LutWarehouse, ProfileClass, ProfileVersion, ToneReprCurve};

use crate::image_decode::RenderingIntent;

/// The profile connection space's illuminant, as littleCMS's cmsD50X, Y
/// and Z give it.
const D50: [f64; 3] = [0.9642, 1.0, 0.8249];

/// The black littleCMS takes for a version 4 table profile's perceptual
/// and saturation intents, its cmsPERCEPTUAL_BLACK.
const PERCEPTUAL_BLACK: [f64; 3] = [0.00336, 0.0034731, 0.00287];

/// The largest XYZ a 16-bit PCS value encodes, 1 + 32767/32768; littleCMS
/// carries XYZ divided by it.
const MAX_ENCODEABLE_XYZ: f64 = 1.0 + 32767.0 / 32768.0;

/// How near a parameter may be to 0 or 1 before littleCMS treats it as
/// that value, its MATRIX_DET_TOLERANCE.
const TOLERANCE: f64 = 0.0001;

/// The values littleCMS takes for the infinities a segmented curve's first
/// and last pieces run to, its MINUS_INF and PLUS_INF, floats.
const MINUS_INF: f64 = -1e22_f32 as f64;
const PLUS_INF: f64 = 1e22_f32 as f64;

/// A curve as a profile gives it.
enum Curve {
    /// Values for inputs spread evenly from 0 to 1, read linearly between.
    Table(Vec<f64>),
    /// The ICC's parametric functions 0 to 4, with their parameters in the
    /// ICC's order.
    Power(f64),
    Cie122([f64; 3]),
    Iec61966_3([f64; 4]),
    Srgb([f64; 5]),
    Offset([f64; 7]),
    /// A D2B table's segmented curve, its pieces in order.
    Segmented(Vec<Segment>),
}

impl Curve {
    /// The curve moxcms read from a lutAtoB table: a table of no entries
    /// the identity, of one a gamma in u8Fixed8, of more a table.
    fn of(curve: &ToneReprCurve) -> Result<Curve, String> {
        Ok(match curve {
            ToneReprCurve::Lut(entries) => match entries.as_slice() {
                [] => Curve::Power(1.0),
                [gamma] => Curve::Power(f64::from(*gamma) / 256.0),
                many => Curve::Table(many.iter().map(|&v| f64::from(v) / 65535.0).collect()),
            },
            ToneReprCurve::Parametric(parameters) => match parameters.iter().map(|&v| f64::from(v)).collect::<Vec<f64>>()[..] {
                [g] => Curve::Power(g),
                [g, a, b] => Curve::Cie122([g, a, b]),
                [g, a, b, c] => Curve::Iec61966_3([g, a, b, c]),
                [g, a, b, c, d] => Curve::Srgb([g, a, b, c, d]),
                [g, a, b, c, d, e, f] => Curve::Offset([g, a, b, c, d, e, f]),
                ref other => return Err(format!("a parametric curve has {} parameters, where the ICC's functions take 1, 3, 4, 5 or 7", other.len())),
            },
        })
    }

    /// The curve's value at `x`, a parametric one's as littleCMS's
    /// DefaultEvalParametricFn gives it.
    fn at(&self, x: f64) -> f64 {
        match self {
            Curve::Table(values) => linear(values, x),
            Curve::Power(g) => {
                if x >= 0.0 {
                    x.powf(*g)
                } else if (g - 1.0).abs() < TOLERANCE {
                    x
                } else {
                    0.0
                }
            }
            Curve::Cie122([g, a, b]) => {
                if a.abs() < TOLERANCE || x < -b / a {
                    0.0
                } else {
                    positive_power(a * x + b, *g).unwrap_or(0.0)
                }
            }
            Curve::Iec61966_3([g, a, b, c]) => {
                if a.abs() < TOLERANCE {
                    0.0
                } else if x >= (-b / a).max(0.0) {
                    positive_power(a * x + b, *g).map_or(0.0, |p| p + c)
                } else {
                    *c
                }
            }
            Curve::Srgb([g, a, b, c, d]) => {
                if x >= *d {
                    positive_power(a * x + b, *g).unwrap_or(0.0)
                } else {
                    x * c
                }
            }
            Curve::Offset([g, a, b, c, d, e, f]) => {
                if x >= *d {
                    positive_power(a * x + b, *g).unwrap_or(0.0) + e
                } else {
                    x * c + f
                }
            }
            Curve::Segmented(segments) => segmented(segments, x),
        }
    }
}

/// `base` raised to `gamma`, where `base` is above 0.
fn positive_power(base: f64, gamma: f64) -> Option<f64> {
    (base > 0.0).then(|| base.powf(gamma))
}

/// A piece of a segmented curve, for inputs above `from` up to `to`.
struct Segment {
    from: f64,
    to: f64,
    piece: Piece,
}

/// What a segmented curve's piece is: one of the ICC's three formulas, its
/// parameters in the ICC's order, or values sampled across it.
enum Piece {
    /// (a x + b)^g + c, from g, a, b and c.
    Power([f64; 4]),
    /// a log10(b x^g + c) + d, from g, a, b, c and d.
    Log([f64; 5]),
    /// a b^(c x + d) + e, from a, b, c, d and e.
    Exponential([f64; 5]),
    /// Values for inputs spread evenly across the piece, read linearly
    /// between, the first the curve's value where the piece begins.
    Sampled(Vec<f64>),
}

/// A segmented curve's value at `x`, as littleCMS's EvalSegmentedFn gives
/// it: by the last piece whose span holds `x`, and MINUS_INF where none
/// does. An infinite value gives PLUS_INF, whichever its sign, as C's
/// isinf, which littleCMS tests it with first, is true of both.
fn segmented(segments: &[Segment], x: f64) -> f64 {
    let Some(segment) = segments.iter().rev().find(|s| x > s.from && x <= s.to) else {
        return MINUS_INF;
    };
    let y = match &segment.piece {
        Piece::Power([g, a, b, c]) => {
            let e = a * x + b;
            if *g == 1.0 {
                e + c
            } else if e < 0.0 {
                *c
            } else {
                e.powf(*g) + c
            }
        }
        Piece::Log([g, a, b, c, d]) => {
            let e = b * x.powf(*g) + c;
            if e <= 0.0 { *d } else { a * e.log10() + d }
        }
        Piece::Exponential([a, b, c, d, e]) => a * b.powf(c * x + d) + e,
        Piece::Sampled(values) => linear(values, (x - segment.from) / (segment.to - segment.from)),
    };
    if y.is_infinite() { PLUS_INF } else { y }
}

/// The value of entries spread evenly from 0 to 1 at `x`, clamped to that
/// range, read linearly between the two entries around it.
fn linear(values: &[f64], x: f64) -> f64 {
    let (i0, i1, r) = cell(x, values.len());
    values[i0] + (values[i1] - values[i0]) * r
}

/// The grid points either side of `x` among `points` spread evenly from 0
/// to 1, and how far `x` is from the first to the second; at 1 both points
/// are the last. `x` is clamped to that range as littleCMS's fclamp clamps
/// it, which takes a value below 1e-9 or not a number as 0.
fn cell(x: f64, points: usize) -> (usize, usize, f64) {
    let last = points - 1;
    let x = if x.is_nan() || x < f64::from(1e-9_f32) { 0.0 } else { x.min(1.0) };
    let at = x * last as f64;
    let i0 = (at.floor() as usize).min(last);
    if i0 == last { (last, last, 0.0) } else { (i0, i0 + 1, at - i0 as f64) }
}

/// One step of a table.
enum Step {
    /// A matrix, a row for each output of a value for each of its `inputs`,
    /// and an offset added to each output.
    Matrix { inputs: usize, rows: Vec<f64>, offset: Vec<f64> },
    /// A curve for each channel.
    Curves(Vec<Curve>),
    /// A grid with `points` along each input, the first input's points
    /// farthest apart in `values`, `strides` apart, and `outputs` values at
    /// each point, read across three inputs trilinearly where `trilinear`
    /// says and otherwise tetrahedrally.
    Grid { points: Vec<usize>, strides: Vec<usize>, outputs: usize, values: Vec<f64>, trilinear: bool },
}

/// The values a table holds, from 0 to 1.
fn normalized(store: &LutStore) -> Vec<f64> {
    match store {
        LutStore::Store8(v) => v.iter().map(|&x| f64::from(x) / 255.0).collect(),
        LutStore::Store16(v) => v.iter().map(|&x| f64::from(x) / 65535.0).collect(),
    }
}

/// A grid step, its strides worked out from its points.
fn grid(points: Vec<usize>, outputs: usize, values: Vec<f64>) -> Result<Step, String> {
    let mut strides = vec![outputs; points.len()];
    for i in (0..points.len().saturating_sub(1)).rev() {
        strides[i] = strides[i + 1] * points[i + 1];
    }
    let needed = points.iter().product::<usize>() * outputs;
    if points.iter().any(|&p| p < 2) || values.len() < needed {
        return Err(format!("its grid of {points:?} points holds {} values where it needs {needed}", values.len()));
    }
    Ok(Step::Grid { points, strides, outputs, values, trilinear: false })
}

/// The curves of a lut8 or lut16 table: `count` entries for each of
/// `channels` curves, one curve after another.
fn tables(store: &LutStore, channels: usize, count: usize) -> Result<Vec<Curve>, String> {
    let values = normalized(store);
    if count < 2 || values.len() < channels * count {
        return Err(format!("its curves hold {} entries where {channels} curves of {count} need {}", values.len(), channels * count));
    }
    Ok(values.chunks(count).take(channels).map(|c| Curve::Table(c.to_vec())).collect())
}

/// A step of a 3 by 3 matrix and the offset added after it.
fn matrix3(m: &[[f64; 3]; 3], offset: [f64; 3]) -> Step {
    Step::Matrix { inputs: 3, rows: m.iter().flatten().copied().collect(), offset: offset.to_vec() }
}

/// Whether `m` is the identity to within littleCMS's 1/65535.
fn is_identity(m: &[[f64; 3]; 3]) -> bool {
    (0..3).all(|i| (0..3).all(|j| (m[i][j] - if i == j { 1.0 } else { 0.0 }).abs() < 1.0 / 65535.0))
}

/// The steps of a lut8 or lut16 table: its matrix where it has three
/// inputs and the matrix is not the identity, its input curves, its grid
/// and its output curves.
fn lut_steps(lut: &LutDataType) -> Result<Vec<Step>, String> {
    let (inputs, outputs) = (usize::from(lut.num_input_channels), usize::from(lut.num_output_channels));
    let mut steps = Vec::new();
    if inputs == 3 && !is_identity(&lut.matrix.v) {
        steps.push(matrix3(&lut.matrix.v, [0.0; 3]));
    }
    steps.push(Step::Curves(tables(&lut.input_table, inputs, usize::from(lut.num_input_table_entries))?));
    steps.push(grid(vec![usize::from(lut.num_clut_grid_points); inputs], outputs, normalized(&lut.clut_table))?);
    steps.push(Step::Curves(tables(&lut.output_table, outputs, usize::from(lut.num_output_table_entries))?));
    Ok(steps)
}

/// The steps of a lutAtoB table in the ICC's order: its A curves, grid, M
/// curves, matrix and B curves, each where the table has it. moxcms gives
/// a table without a matrix the identity and no offset, which change
/// nothing.
fn mab_steps(mab: &LutMultidimensionalType) -> Result<Vec<Step>, String> {
    let (inputs, outputs) = (usize::from(mab.num_input_channels), usize::from(mab.num_output_channels));
    let curves = |set: &[ToneReprCurve]| set.iter().map(Curve::of).collect::<Result<Vec<Curve>, String>>();
    let mut steps = Vec::new();
    if !mab.a_curves.is_empty() {
        steps.push(Step::Curves(curves(&mab.a_curves)?));
    }
    if let Some(clut) = &mab.clut {
        let points = mab.grid_points.iter().take(inputs).map(|&p| usize::from(p)).collect();
        steps.push(grid(points, outputs, normalized(clut))?);
    }
    if !mab.m_curves.is_empty() {
        steps.push(Step::Curves(curves(&mab.m_curves)?));
    }
    if outputs == 3 {
        steps.push(matrix3(&mab.matrix.v, mab.bias.v));
    }
    if !mab.b_curves.is_empty() {
        steps.push(Step::Curves(curves(&mab.b_curves)?));
    }
    Ok(steps)
}

/// The steps of a lutBtoA table in the ICC's order: its B curves, matrix,
/// M curves, grid and A curves, each where the table has it. moxcms reads
/// each curve set by its own offset, as for a lutAtoB table, and gives a
/// table without a matrix the identity and no offset.
fn mba_steps(mba: &LutMultidimensionalType) -> Result<Vec<Step>, String> {
    let (inputs, outputs) = (usize::from(mba.num_input_channels), usize::from(mba.num_output_channels));
    let curves = |set: &[ToneReprCurve]| set.iter().map(Curve::of).collect::<Result<Vec<Curve>, String>>();
    let mut steps = Vec::new();
    if !mba.b_curves.is_empty() {
        steps.push(Step::Curves(curves(&mba.b_curves)?));
    }
    if inputs == 3 {
        steps.push(matrix3(&mba.matrix.v, mba.bias.v));
    }
    if !mba.m_curves.is_empty() {
        steps.push(Step::Curves(curves(&mba.m_curves)?));
    }
    if let Some(clut) = &mba.clut {
        let points = mba.grid_points.iter().take(inputs).map(|&p| usize::from(p)).collect();
        steps.push(grid(points, outputs, normalized(clut))?);
    }
    if !mba.a_curves.is_empty() {
        steps.push(Step::Curves(curves(&mba.a_curves)?));
    }
    Ok(steps)
}

/// The grid's values at `input`, each from 0 to 1, into `out`: across three
/// inputs tetrahedrally, as littleCMS's TetrahedralInterp16 and Eval4Inputs
/// read a grid, or with `trilinear` along each input in turn, as its
/// TrilinearInterpFloat reads one; across more, linearly along the first
/// between the grids of the rest at the points either side; across one or
/// two, linearly.
fn interpolate(points: &[usize], strides: &[usize], values: &[f64], base: usize, input: &[f64], out: &mut [f64], trilinear: bool) {
    match input.len() {
        0 => out.copy_from_slice(&values[base..base + out.len()]),
        1 => {
            let (i0, i1, r) = cell(input[0], points[0]);
            for (o, v) in out.iter_mut().enumerate() {
                let (a, b) = (values[base + i0 * strides[0] + o], values[base + i1 * strides[0] + o]);
                *v = a + (b - a) * r;
            }
        }
        2 => {
            let (x0, x1, rx) = cell(input[0], points[0]);
            let (y0, y1, ry) = cell(input[1], points[1]);
            let at = |x: usize, y: usize, o: usize| values[base + x * strides[0] + y * strides[1] + o];
            for (o, v) in out.iter_mut().enumerate() {
                let near = at(x0, y0, o) + (at(x1, y0, o) - at(x0, y0, o)) * rx;
                let far = at(x0, y1, o) + (at(x1, y1, o) - at(x0, y1, o)) * rx;
                *v = near + (far - near) * ry;
            }
        }
        3 => {
            let (x0, x1, rx) = cell(input[0], points[0]);
            let (y0, y1, ry) = cell(input[1], points[1]);
            let (z0, z1, rz) = cell(input[2], points[2]);
            let at = |x: usize, y: usize, z: usize, o: usize| values[base + x * strides[0] + y * strides[1] + z * strides[2] + o];
            if trilinear {
                for (o, v) in out.iter_mut().enumerate() {
                    let along_x = |y: usize, z: usize| at(x0, y, z, o) + (at(x1, y, z, o) - at(x0, y, z, o)) * rx;
                    let (near, far) = (along_x(y0, z0) + (along_x(y1, z0) - along_x(y0, z0)) * ry, along_x(y0, z1) + (along_x(y1, z1) - along_x(y0, z1)) * ry);
                    *v = near + (far - near) * rz;
                }
                return;
            }
            for (o, v) in out.iter_mut().enumerate() {
                let c0 = at(x0, y0, z0, o);
                let (c1, c2, c3) = if rx >= ry && ry >= rz {
                    (at(x1, y0, z0, o) - c0, at(x1, y1, z0, o) - at(x1, y0, z0, o), at(x1, y1, z1, o) - at(x1, y1, z0, o))
                } else if rx >= rz && rz >= ry {
                    (at(x1, y0, z0, o) - c0, at(x1, y1, z1, o) - at(x1, y0, z1, o), at(x1, y0, z1, o) - at(x1, y0, z0, o))
                } else if rz >= rx && rx >= ry {
                    (at(x1, y0, z1, o) - at(x0, y0, z1, o), at(x1, y1, z1, o) - at(x1, y0, z1, o), at(x0, y0, z1, o) - c0)
                } else if ry >= rx && rx >= rz {
                    (at(x1, y1, z0, o) - at(x0, y1, z0, o), at(x0, y1, z0, o) - c0, at(x1, y1, z1, o) - at(x1, y1, z0, o))
                } else if ry >= rz && rz >= rx {
                    (at(x1, y1, z1, o) - at(x0, y1, z1, o), at(x0, y1, z0, o) - c0, at(x0, y1, z1, o) - at(x0, y1, z0, o))
                } else {
                    (at(x1, y1, z1, o) - at(x0, y1, z1, o), at(x0, y1, z1, o) - at(x0, y0, z1, o), at(x0, y0, z1, o) - c0)
                };
                *v = c0 + c1 * rx + c2 * ry + c3 * rz;
            }
        }
        _ => {
            let (k0, k1, rk) = cell(input[0], points[0]);
            let n = out.len();
            let mut near = [0.0; 16];
            let mut far = [0.0; 16];
            interpolate(&points[1..], &strides[1..], values, base + k0 * strides[0], &input[1..], &mut near[..n], trilinear);
            interpolate(&points[1..], &strides[1..], values, base + k1 * strides[0], &input[1..], &mut far[..n], trilinear);
            for (o, v) in out.iter_mut().enumerate() {
                *v = near[o] + (far[o] - near[o]) * rk;
            }
        }
    }
}

/// `values` run through `steps`, the first `inputs` of them used; the
/// first three of the result.
fn run(steps: &[Step], device: &[f64]) -> [f64; 3] {
    let (values, _) = evaluate(steps, device);
    [values[0], values[1], values[2]]
}

/// `device` run through `steps`: the values the last step gives, and how
/// many it gives.
fn evaluate(steps: &[Step], device: &[f64]) -> ([f64; 16], usize) {
    let mut values = [0.0; 16];
    let mut scratch = [0.0; 16];
    let mut n = device.len().min(16);
    values[..n].copy_from_slice(&device[..n]);
    for step in steps {
        match step {
            Step::Curves(curves) => {
                for (v, curve) in values.iter_mut().zip(curves) {
                    *v = curve.at(*v);
                }
            }
            Step::Matrix { inputs, rows, offset } => {
                let x = values;
                for ((v, row), o) in values.iter_mut().zip(rows.chunks(*inputs)).zip(offset) {
                    *v = row.iter().zip(&x).map(|(m, x)| m * x).sum::<f64>() + o;
                }
                n = offset.len();
            }
            Step::Grid { points, strides, outputs, values: grid, trilinear } => {
                interpolate(points, strides, grid, 0, &values[..n], &mut scratch[..*outputs], *trilinear);
                values[..*outputs].copy_from_slice(&scratch[..*outputs]);
                n = *outputs;
            }
        }
    }
    (values, n)
}

/// XYZ from Lab under D50, as littleCMS's cmsLab2XYZ gives it.
pub(crate) fn lab_to_xyz([l, a, b]: [f64; 3]) -> [f64; 3] {
    let f_1 = |t: f64| if t <= 24.0 / 116.0 { (108.0 / 841.0) * (t - 16.0 / 116.0) } else { t * t * t };
    let y = (l + 16.0) / 116.0;
    [f_1(y + 0.002 * a) * D50[0], f_1(y) * D50[1], f_1(y - 0.005 * b) * D50[2]]
}

/// Lab from XYZ under D50, as littleCMS's cmsXYZ2Lab gives it.
pub(crate) fn xyz_to_lab(xyz: [f64; 3]) -> [f64; 3] {
    let f = |t: f64| if t <= (24.0f64 / 116.0).powi(3) { (841.0 / 108.0) * t + 16.0 / 116.0 } else { t.powf(1.0 / 3.0) };
    let [fx, fy, fz] = [f(xyz[0] / D50[0]), f(xyz[1] / D50[1]), f(xyz[2] / D50[2])];
    [116.0 * fy - 16.0, 500.0 * (fx - fy), 200.0 * (fy - fz)]
}

/// What a table gives: Lab, a lut16's in its version 2 encoding, or XYZ.
#[derive(Clone, Copy)]
enum Pcs {
    Lab { version2: bool },
    Xyz,
}

impl Pcs {
    /// XYZ from the table's three values, each from 0 to 1.
    fn xyz(self, v: [f64; 3]) -> [f64; 3] {
        match self {
            Pcs::Xyz => v.map(|x| x * MAX_ENCODEABLE_XYZ),
            Pcs::Lab { version2 } => {
                let v = if version2 { v.map(|x| x * 65535.0 / 65280.0) } else { v };
                lab_to_xyz([v[0] * 100.0, v[1] * 255.0 - 128.0, v[2] * 255.0 - 128.0])
            }
        }
    }
}

/// A table a profile converts by: its steps, what its values are, the
/// channels it takes and its tag's name, `None` for a matrix-shaper's
/// matrix and curves made a table.
struct Table {
    steps: Vec<Step>,
    pcs: Pcs,
    inputs: usize,
    tag: Option<&'static str>,
}

/// The table a profile converts by for `intent`, as littleCMS's
/// _cmsReadInputLUT picks it: the intent's D2B table where the profile,
/// stored as `icc`, holds one; otherwise the intent's A2B table, or A2B0
/// where the profile lacks that. `None` where it holds none of them.
fn table_for(profile: &ColorProfile, icc: &[u8], intent: RenderingIntent) -> Result<Option<Table>, String> {
    let float = match intent {
        RenderingIntent::Perceptual => "D2B0",
        RenderingIntent::RelativeColorimetric => "D2B1",
        RenderingIntent::Saturation => "D2B2",
        RenderingIntent::AbsoluteColorimetric => "D2B3",
    };
    if let Some(place) = tag_at(icc, float) {
        return float_table(profile, icc, place, float).map(Some);
    }
    match own_table(profile, intent).or_else(|| profile.lut_a_to_b_perceptual.as_ref().map(|t| (t, "A2B0"))) {
        Some((table, tag)) => read(profile, table, tag).map(Some),
        None => Ok(None),
    }
}

/// The A2B table the profile holds for `intent` itself, and its tag's
/// name.
fn own_table(profile: &ColorProfile, intent: RenderingIntent) -> Option<(&LutWarehouse, &'static str)> {
    match intent {
        RenderingIntent::Perceptual => profile.lut_a_to_b_perceptual.as_ref().map(|t| (t, "A2B0")),
        RenderingIntent::RelativeColorimetric | RenderingIntent::AbsoluteColorimetric => profile.lut_a_to_b_colorimetric.as_ref().map(|t| (t, "A2B1")),
        RenderingIntent::Saturation => profile.lut_a_to_b_saturation.as_ref().map(|t| (t, "A2B2")),
    }
}

/// Whether littleCMS's cmsIsMatrixShaper takes the profile for one: gray
/// with a gray curve, or RGB with three colorants and three curves.
pub(crate) fn is_matrix_shaper(profile: &ColorProfile) -> bool {
    let colorant = |c: &moxcms::Xyzd| c.x != 0.0 || c.y != 0.0 || c.z != 0.0;
    match profile.color_space {
        DataColorSpace::Gray => profile.gray_trc.is_some(),
        DataColorSpace::Rgb => {
            profile.red_trc.is_some()
                && profile.green_trc.is_some()
                && profile.blue_trc.is_some()
                && colorant(&profile.red_colorant)
                && colorant(&profile.green_colorant)
                && colorant(&profile.blue_colorant)
        }
        _ => false,
    }
}

/// The darkest device values of a color space as littleCMS's
/// _cmsEndPointsBySpace gives them: no light for RGB and gray, every ink
/// full for CMYK and CMY; `None` for a space it gives none.
fn darkest(space: DataColorSpace) -> Option<&'static [f64]> {
    match space {
        DataColorSpace::Rgb => Some(&[0.0; 3]),
        DataColorSpace::Gray => Some(&[0.0]),
        DataColorSpace::Cmyk => Some(&[1.0; 4]),
        DataColorSpace::Cmy => Some(&[1.0; 3]),
        _ => None,
    }
}

/// The lut8, lut16 or lutAtoB table moxcms read from the tag `tag`.
fn read(profile: &ColorProfile, table: &LutWarehouse, tag: &'static str) -> Result<Table, String> {
    let (steps, inputs) = match table {
        LutWarehouse::Lut(lut) => (lut_steps(lut)?, usize::from(lut.num_input_channels)),
        LutWarehouse::Multidimensional(mab) => (mab_steps(mab)?, usize::from(mab.num_input_channels)),
    };
    let pcs = match profile.pcs {
        DataColorSpace::Lab => Pcs::Lab { version2: matches!(table, LutWarehouse::Lut(lut) if lut.lut_type == LutType::Lut16) },
        DataColorSpace::Xyz => Pcs::Xyz,
        other => return Err(format!("its tables give {other:?}, where a profile's tables give Lab or XYZ")),
    };
    Ok(Table { steps, pcs, inputs, tag: Some(tag) })
}

/// A matrix-shaper's matrix and curves as a table, as littleCMS's
/// BuildRGBInputMatrixShaper and BuildGrayInputMatrixPipeline build them:
/// for RGB each channel's curve and then the colorants; for gray the gray's
/// curve and then D50's white, or with a Lab PCS the gray's curve as L*
/// with a and b 0; XYZ in littleCMS's encoding, divided by
/// MAX_ENCODEABLE_XYZ. `None` for a profile that is no matrix-shaper.
fn matrix_shaper(profile: &ColorProfile) -> Result<Option<Table>, String> {
    if !is_matrix_shaper(profile) {
        return Ok(None);
    }
    let curve = |trc: &Option<ToneReprCurve>| match trc {
        Some(trc) => Curve::of(trc),
        None => Err("it is a matrix-shaper without a curve for each channel".to_string()),
    };
    let table = if profile.color_space == DataColorSpace::Gray {
        let gray = curve(&profile.gray_trc)?;
        if profile.pcs == DataColorSpace::Lab {
            // a and b of 0 in littleCMS's 16-bit Lab encoding, 0x8080.
            let zero = f64::from(0x8080_u16) / 65535.0;
            let steps = vec![Step::Matrix { inputs: 1, rows: vec![1.0; 3], offset: vec![0.0; 3] }, Step::Curves(vec![gray, Curve::Table(vec![zero; 2]), Curve::Table(vec![zero; 2])])];
            Table { steps, pcs: Pcs::Lab { version2: false }, inputs: 1, tag: None }
        } else {
            let steps = vec![Step::Curves(vec![gray]), Step::Matrix { inputs: 1, rows: D50.map(|w| w / MAX_ENCODEABLE_XYZ).to_vec(), offset: vec![0.0; 3] }];
            Table { steps, pcs: Pcs::Xyz, inputs: 1, tag: None }
        }
    } else {
        let curves = vec![curve(&profile.red_trc)?, curve(&profile.green_trc)?, curve(&profile.blue_trc)?];
        let [r, g, b] = [profile.red_colorant, profile.green_colorant, profile.blue_colorant];
        let rows = [r.x, g.x, b.x, r.y, g.y, b.y, r.z, g.z, b.z].map(|v| v / MAX_ENCODEABLE_XYZ).to_vec();
        Table { steps: vec![Step::Curves(curves), Step::Matrix { inputs: 3, rows, offset: vec![0.0; 3] }], pcs: Pcs::Xyz, inputs: 3, tag: None }
    };
    Ok(Some(table))
}

/// The D2B table `name` at `place`, its offset and size, in the profile
/// stored as `icc`. Its values are Lab or XYZ in their own units, and a
/// step after its own scales them to the 0 to 1 a `Pcs` reads, as
/// littleCMS's _cmsReadFloatInputTag adds _cmsStageNormalizeFromLabFloat
/// or _cmsStageNormalizeFromXyzFloat; a table of no elements is that step
/// alone, taking 3 channels.
fn float_table(profile: &ColorProfile, icc: &[u8], (offset, size): (usize, usize), name: &'static str) -> Result<Table, String> {
    if size < 8 {
        return Err(format!("its {name} tag is {size} bytes, too few to hold a table"));
    }
    let Elements { mut steps, ends } = mpe_steps(icc, offset, name)?;
    let (inputs, outputs) = ends.unwrap_or((3, 3));
    if outputs != 3 {
        return Err(format!("its {name} table gives {outputs} channels, where Lab and XYZ have 3"));
    }
    let pcs = match profile.pcs {
        DataColorSpace::Lab => {
            let rows = vec![1.0 / 100.0, 0.0, 0.0, 0.0, 1.0 / 255.0, 0.0, 0.0, 0.0, 1.0 / 255.0];
            steps.push(Step::Matrix { inputs: 3, rows, offset: vec![0.0, 128.0 / 255.0, 128.0 / 255.0] });
            Pcs::Lab { version2: false }
        }
        DataColorSpace::Xyz => {
            let n = 32768.0 / 65535.0;
            steps.push(Step::Matrix { inputs: 3, rows: vec![n, 0.0, 0.0, 0.0, n, 0.0, 0.0, 0.0, n], offset: vec![0.0; 3] });
            Pcs::Xyz
        }
        other => return Err(format!("its tables give {other:?}, where a profile's tables give Lab or XYZ")),
    };
    Ok(Table { steps, pcs, inputs, tag: Some(name) })
}

/// How a table that writes colors takes a color: Lab or XYZ encoded from 0
/// to 1 as a lut8, lut16 or lutBtoA table takes it, a lut16's Lab in its
/// version 2 encoding, or Lab or XYZ in their own units, as a BToD table
/// takes them.
#[derive(Clone, Copy)]
enum Takes {
    Encoded(Pcs),
    Lab,
    Xyz,
}

impl Takes {
    /// The table's three inputs for the color `xyz`, XYZ under D50.
    fn inputs(self, xyz: [f64; 3]) -> [f64; 3] {
        match self {
            Takes::Xyz => xyz,
            Takes::Lab => xyz_to_lab(xyz),
            Takes::Encoded(Pcs::Xyz) => xyz.map(|v| v / MAX_ENCODEABLE_XYZ),
            Takes::Encoded(Pcs::Lab { version2 }) => {
                let [l, a, b] = xyz_to_lab(xyz);
                let v = [l / 100.0, (a + 128.0) / 255.0, (b + 128.0) / 255.0];
                if version2 { v.map(|x| x * 65280.0 / 65535.0) } else { v }
            }
        }
    }
}

/// `curve` reversed, as littleCMS's cmsReverseToneCurve reverses a gray
/// curve: 4096 values spread evenly from 0 to 1, each the input the curve
/// takes to that value, found by halving the span 60 times; a value past
/// what the curve reaches takes the end that comes nearest.
fn reversed(curve: &Curve) -> Curve {
    let (low, high) = (curve.at(0.0), curve.at(1.0));
    let rising = high >= low;
    let values = (0..4096)
        .map(|i| {
            let y = f64::from(i) / 4095.0;
            let (mut from, mut to) = (0.0, 1.0);
            for _ in 0..60 {
                let mid = (from + to) / 2.0;
                if (curve.at(mid) < y) == rising {
                    from = mid;
                } else {
                    to = mid;
                }
            }
            (from + to) / 2.0
        })
        .collect();
    Curve::Table(values)
}

/// A matrix-shaper profile's tone curves: each channel's sample as linear
/// light by its curve, and back by the curve reversed, as littleCMS
/// reverses one.
pub(crate) struct ToneCurves {
    forward: Vec<Curve>,
    backward: Vec<Curve>,
}

impl ToneCurves {
    /// The curves of `profile`'s gray, or its red, green and blue; or why
    /// it has none, a profile that is not a matrix-shaper.
    pub(crate) fn of(profile: &ColorProfile) -> Result<ToneCurves, String> {
        if !is_matrix_shaper(profile) {
            return Err("its color profile has no tone curves its samples' light comes from".to_string());
        }
        let trcs = if profile.color_space == DataColorSpace::Gray { vec![&profile.gray_trc] } else { vec![&profile.red_trc, &profile.green_trc, &profile.blue_trc] };
        let mut forward = Vec::with_capacity(trcs.len());
        for trc in trcs {
            forward.push(Curve::of(trc.as_ref().ok_or("its color profile lacks a tone curve")?)?);
        }
        let backward = forward.iter().map(reversed).collect();
        Ok(ToneCurves { forward, backward })
    }

    /// Channel `channel`'s sample `v` as linear light.
    pub(crate) fn light(&self, channel: usize, v: f64) -> f64 {
        self.forward[channel].at(v)
    }

    /// Linear light `v` as channel `channel`'s sample.
    pub(crate) fn sample(&self, channel: usize, v: f64) -> f64 {
        self.backward[channel].at(v)
    }
}

/// A table a profile writes colors by with the relative colorimetric
/// intent, from a color's XYZ under D50 to the profile's device values, as
/// littleCMS's _cmsReadOutputLUT picks it: the profile's BToD1 table, tag
/// B2D1, then its B2A1, then its B2A0, whose grid is read trilinearly; or, for a gray
/// profile of a Lab PCS with none of them, its gray curve reversed, taking
/// L*. A matrix-shaper of an XYZ PCS with none writes by its matrix and
/// curves instead.
pub(crate) struct Writing {
    steps: Vec<Step>,
    takes: Takes,
}

impl Writing {
    /// The table `profile`, stored as `icc`, writes colors by; `None` where
    /// it holds none.
    pub(crate) fn relative(profile: &ColorProfile, icc: &[u8]) -> Result<Option<Writing>, String> {
        let real = match profile.pcs {
            DataColorSpace::Lab => Takes::Lab,
            DataColorSpace::Xyz => Takes::Xyz,
            other => return Err(format!("its tables take {other:?}, where a profile's tables take Lab or XYZ")),
        };
        if let Some((offset, size)) = tag_at(icc, "B2D1") {
            if size < 8 {
                return Err(format!("its B2D1 tag is {size} bytes, too few to hold a table"));
            }
            let Elements { steps, ends } = mpe_steps(icc, offset, "B2D1")?;
            if let Some((inputs, _)) = ends
                && inputs != 3
            {
                return Err(format!("its B2D1 table takes {inputs} channels, where Lab and XYZ have 3"));
            }
            return Ok(Some(Writing { steps, takes: real }));
        }
        let table = profile.lut_b_to_a_colorimetric.as_ref().map(|t| (t, "B2A1")).or_else(|| profile.lut_b_to_a_perceptual.as_ref().map(|t| (t, "B2A0")));
        if let Some((table, tag)) = table {
            let (mut steps, inputs) = match table {
                LutWarehouse::Lut(lut) => (lut_steps(lut)?, usize::from(lut.num_input_channels)),
                LutWarehouse::Multidimensional(mba) => (mba_steps(mba)?, usize::from(mba.num_input_channels)),
            };
            if inputs != 3 {
                return Err(format!("its {tag} table takes {inputs} channels, where Lab and XYZ have 3"));
            }
            // littleCMS reads the grid of a table that writes colors
            // trilinearly, its ChangeInterpolationToTrilinear.
            for step in &mut steps {
                if let Step::Grid { trilinear, .. } = step {
                    *trilinear = true;
                }
            }
            let pcs = match real {
                Takes::Lab => Pcs::Lab { version2: matches!(table, LutWarehouse::Lut(lut) if lut.lut_type == LutType::Lut16) },
                _ => Pcs::Xyz,
            };
            return Ok(Some(Writing { steps, takes: Takes::Encoded(pcs) }));
        }
        if profile.color_space == DataColorSpace::Gray
            && profile.pcs == DataColorSpace::Lab
            && let Some(trc) = &profile.gray_trc
        {
            let steps = vec![Step::Matrix { inputs: 3, rows: vec![1.0, 0.0, 0.0], offset: vec![0.0] }, Step::Curves(vec![reversed(&Curve::of(trc)?)])];
            return Ok(Some(Writing { steps, takes: Takes::Encoded(Pcs::Lab { version2: false }) }));
        }
        Ok(None)
    }

    /// The device values the table gives the color `xyz`, XYZ under D50,
    /// each held within 0 to 1.
    pub(crate) fn device(&self, xyz: [f64; 3]) -> Vec<f64> {
        let (values, n) = evaluate(&self.steps, &self.takes.inputs(xyz));
        values[..n].iter().map(|v| v.clamp(0.0, 1.0)).collect()
    }
}

/// The offset and size of the tag `signature` in the profile stored as
/// `icc`, as littleCMS's _cmsReadHeader admits a tag: the first entry of
/// that signature whose offset and size are not 0 and whose bytes end
/// within both the size the header states and the bytes there are. `None`
/// where no entry is admitted.
fn tag_at(icc: &[u8], signature: &str) -> Option<(usize, usize)> {
    let word = |at: usize| icc.get(at..at + 4).map(|b| u64::from(u32::from_be_bytes([b[0], b[1], b[2], b[3]])));
    let limit = word(0)?.min(icc.len() as u64);
    let count = word(128)? as usize;
    (0..count)
        .map_while(|i| Some((icc.get(132 + 12 * i..136 + 12 * i)?, word(136 + 12 * i)?, word(140 + 12 * i)?)))
        .find(|&(found, offset, size)| found == signature.as_bytes() && offset != 0 && size != 0 && offset + size <= limit)
        .map(|(_, offset, size)| (offset as usize, size as usize))
}

/// A tag or element type's four characters.
fn type_name(kind: [u8; 4]) -> String {
    String::from_utf8_lossy(&kind).into_owned()
}

/// Whether littleCMS reads a D2B table's element of `inputs` and `outputs`
/// channels: 1 to 15 of each, below its cmsMAXCHANNELS.
fn readable(inputs: usize, outputs: usize) -> bool {
    (1..16).contains(&inputs) && (1..16).contains(&outputs)
}

/// The elements of a D2B table as steps.
struct Elements {
    steps: Vec<Step>,
    /// The channels the first step takes and the last gives, `None` where
    /// there are no steps.
    ends: Option<(usize, usize)>,
}

/// The steps of the D2B table `name`, a multiProcessElementType at
/// `offset` in `icc`, as littleCMS's Type_MPE_Read reads one: its curve
/// sets, matrices and grids in order, bACS and eACS elements passed over,
/// each element taking the channels the one before gives.
fn mpe_steps(icc: &[u8], offset: usize, name: &'static str) -> Result<Elements, String> {
    let mut reader = Reader { icc, at: offset, name };
    let kind = reader.take::<4>()?;
    if &kind != b"mpet" {
        return Err(format!("its {name} table is of type '{}', where a D2B table is a multiProcessElementType", type_name(kind)));
    }
    reader.u32()?;
    let (inputs, outputs) = (reader.u16()?, reader.u16()?);
    if !readable(inputs, outputs) {
        return Err(format!("its {name} table takes {inputs} channels and gives {outputs}, where littleCMS reads 1 to 15 of each"));
    }
    let count = reader.u32()? as usize;
    let mut steps = Vec::new();
    let mut ends: Option<(usize, usize)> = None;
    for place in reader.positions(count, offset)? {
        reader.seek(place)?;
        let kind = reader.take::<4>()?;
        reader.u32()?;
        let (step, takes, gives) = match &kind {
            b"bACS" | b"eACS" => continue,
            b"cvst" => reader.curve_set(place)?,
            b"matf" => reader.matrix()?,
            b"clut" => reader.clut()?,
            _ => return Err(format!("its {name} table holds an element of type '{}', which littleCMS does not read", type_name(kind))),
        };
        ends = match ends {
            None => Some((takes, gives)),
            Some((first, last)) if last == takes => Some((first, gives)),
            Some((_, last)) => return Err(format!("its {name} table holds an element taking {takes} channels after one giving {last}")),
        };
        steps.push(step);
    }
    if let Some((takes, gives)) = ends
        && (takes, gives) != (inputs, outputs)
    {
        return Err(format!("its {name} table's elements take {takes} channels and give {gives}, where the table states {inputs} and {outputs}"));
    }
    Ok(Elements { steps, ends })
}

/// A reader of a profile's bytes from a place in them, as littleCMS's
/// memory reader reads them: a read past their end fails.
struct Reader<'a> {
    icc: &'a [u8],
    at: usize,
    /// The table read, D2B0 to D2B3, which its errors name.
    name: &'static str,
}

impl Reader<'_> {
    fn take<const N: usize>(&mut self) -> Result<[u8; N], String> {
        match self.icc.get(self.at..).and_then(|rest| rest.first_chunk::<N>()) {
            Some(bytes) => {
                self.at += N;
                Ok(*bytes)
            }
            None => Err(format!("its {} table is cut short", self.name)),
        }
    }

    fn u16(&mut self) -> Result<usize, String> {
        self.take().map(|b| usize::from(u16::from_be_bytes(b)))
    }

    fn u32(&mut self) -> Result<u32, String> {
        self.take().map(u32::from_be_bytes)
    }

    /// A float32Number as littleCMS's _cmsReadFloat32Number reads one: a
    /// number beyond 1e20 either way, or neither zero nor normal, refused.
    fn float(&mut self) -> Result<f64, String> {
        let v = f32::from_be_bytes(self.take()?);
        if (v == 0.0 || v.is_normal()) && f64::from(v).abs() <= 1e20 {
            Ok(f64::from(v))
        } else {
            Err(format!("its {} table holds the number {v}, which littleCMS refuses", self.name))
        }
    }

    /// `count` float32Numbers, refused before any is read where the bytes
    /// left cannot hold them.
    fn floats(&mut self, count: usize) -> Result<Vec<f64>, String> {
        if self.icc.len().saturating_sub(self.at) / 4 < count {
            return Err(format!("its {} table is cut short", self.name));
        }
        (0..count).map(|_| self.float()).collect()
    }

    /// `N` float32Numbers.
    fn parameters<const N: usize>(&mut self) -> Result<[f64; N], String> {
        let mut out = [0.0; N];
        for v in &mut out {
            *v = self.float()?;
        }
        Ok(out)
    }

    /// Moves to `at`, which littleCMS allows up to the end of the bytes.
    fn seek(&mut self, at: usize) -> Result<(), String> {
        if at > self.icc.len() {
            return Err(format!("its {} table places an element past the end of the profile", self.name));
        }
        self.at = at;
        Ok(())
    }

    /// Where the `count` items a position table lists begin, each its
    /// offset from `base`, as littleCMS's ReadPositionTable reads them:
    /// refused before any is read where the bytes left cannot hold the
    /// table. Each item's size goes unread, as littleCMS's readers leave
    /// it.
    fn positions(&mut self, count: usize, base: usize) -> Result<Vec<usize>, String> {
        if self.icc.len().saturating_sub(self.at) / 8 < count {
            return Err(format!("its {} table lists {count} elements, more than its bytes hold", self.name));
        }
        let mut places = Vec::with_capacity(count);
        for _ in 0..count {
            let offset = self.u32()?;
            self.u32()?;
            places.push(base.checked_add(offset as usize).ok_or_else(|| format!("its {} table places an element past the end of the profile", self.name))?);
        }
        Ok(places)
    }

    /// A curve set element, after its type: a segmented curve for each
    /// channel, placed from `base`, where the element begins.
    fn curve_set(&mut self, base: usize) -> Result<(Step, usize, usize), String> {
        let (inputs, outputs) = (self.u16()?, self.u16()?);
        if inputs != outputs || !readable(inputs, outputs) {
            return Err(format!("its {} table holds a curve set of {inputs} inputs and {outputs} outputs, where littleCMS reads as many outputs as inputs, 1 to 15", self.name));
        }
        let mut curves = Vec::with_capacity(inputs);
        for place in self.positions(inputs, base)? {
            self.seek(place)?;
            curves.push(self.segmented_curve()?);
        }
        Ok((Step::Curves(curves), inputs, outputs))
    }

    /// A segmented curve, as littleCMS's ReadSegmentedCurve reads one: its
    /// breakpoints, then a piece for each span they bound, the first from
    /// MINUS_INF and the last to PLUS_INF. A sampled piece's first value is
    /// the curve's value where the piece begins, as a float.
    fn segmented_curve(&mut self) -> Result<Curve, String> {
        let name = self.name;
        let kind = self.take::<4>()?;
        if &kind != b"curf" {
            return Err(format!("its {name} table holds a curve of type '{}', where a curve set holds segmented curves", type_name(kind)));
        }
        self.u32()?;
        let count = self.u16()?;
        self.u16()?;
        if count == 0 {
            return Err(format!("its {name} table holds a segmented curve of no pieces"));
        }
        let mut bounds = vec![MINUS_INF];
        bounds.extend(self.floats(count - 1)?);
        bounds.push(PLUS_INF);
        let mut segments = Vec::with_capacity(count);
        for span in bounds.windows(2) {
            let kind = self.take::<4>()?;
            self.u32()?;
            let piece = match &kind {
                b"parf" => {
                    let function = self.u16()?;
                    self.u16()?;
                    match function {
                        0 => Piece::Power(self.parameters()?),
                        1 => Piece::Log(self.parameters()?),
                        2 => Piece::Exponential(self.parameters()?),
                        other => return Err(format!("its {name} table holds a formula of type {other}, where the ICC's are 0, 1 and 2")),
                    }
                }
                b"samf" => {
                    let samples = self.u32()? as usize;
                    let mut values = vec![0.0];
                    values.extend(self.floats(samples)?);
                    Piece::Sampled(values)
                }
                _ => return Err(format!("its {name} table holds a curve piece of type '{}', which littleCMS does not read", type_name(kind))),
            };
            segments.push(Segment { from: span[0], to: span[1], piece });
        }
        for i in 0..segments.len() {
            if matches!(segments[i].piece, Piece::Sampled(_)) {
                let first = f64::from(segmented(&segments, segments[i].from) as f32);
                if let Piece::Sampled(values) = &mut segments[i].piece {
                    values[0] = first;
                }
            }
        }
        Ok(Curve::Segmented(segments))
    }

    /// A matrix element, after its type: a row of a value for each input
    /// for each output, then an offset for each output.
    fn matrix(&mut self) -> Result<(Step, usize, usize), String> {
        let (inputs, outputs) = (self.u16()?, self.u16()?);
        if !readable(inputs, outputs) {
            return Err(format!("its {} table holds a matrix of {inputs} inputs and {outputs} outputs, where littleCMS reads 1 to 15 of each", self.name));
        }
        let rows = self.floats(inputs * outputs)?;
        let offset = self.floats(outputs)?;
        Ok((Step::Matrix { inputs, rows, offset }, inputs, outputs))
    }

    /// A grid element, after its type: the points along each input, then
    /// the values at every point, the first input's farthest apart, as
    /// littleCMS's Type_MPEclut_Read reads one and its CubeSize bounds it.
    fn clut(&mut self) -> Result<(Step, usize, usize), String> {
        let (inputs, outputs) = (self.u16()?, self.u16()?);
        if !readable(inputs, outputs) {
            return Err(format!("its {} table holds a grid of {inputs} inputs and {outputs} outputs, where littleCMS reads 1 to 15 of each", self.name));
        }
        let counts = self.take::<16>()?;
        let points: Vec<usize> = counts[..inputs].iter().map(|&p| usize::from(p)).collect();
        if points.iter().any(|&p| p < 2) {
            return Err(format!("its {} table holds a grid of {points:?} points, where littleCMS reads 2 or more along each input", self.name));
        }
        let cells = points.iter().try_fold(1u64, |n, &p| n.checked_mul(p as u64)).filter(|&n| n <= u64::from(u32::MAX / 15));
        let Some(cells) = cells else {
            return Err(format!("its {} table holds a grid of {points:?} points, more than littleCMS reads", self.name));
        };
        let values = self.floats(outputs * cells as usize)?;
        Ok((grid(points, outputs, values)?, inputs, outputs))
    }
}

/// The profile's black point for `intent` as littleCMS's
/// BlackPointAsDarkerColorant finds it: the darkest device values through
/// the intent's table, as Lab with L* of 0 above 95 or below 0 and of 50
/// above 50; black where the profile has no A2B table for the intent and
/// is no matrix-shaper, as littleCMS's cmsIsIntentSupported asks, or its
/// space has no darkest values.
fn darker_colorant(profile: &ColorProfile, icc: &[u8], intent: RenderingIntent) -> Result<[f64; 3], String> {
    let supported = own_table(profile, intent).is_some() || is_matrix_shaper(profile);
    let (Some(darkest), true) = (darkest(profile.color_space), supported) else {
        return Ok([0.0; 3]);
    };
    let table = match table_for(profile, icc, intent)? {
        Some(table) => table,
        None => match matrix_shaper(profile)? {
            Some(table) => table,
            None => return Ok([0.0; 3]),
        },
    };
    let [l, a, b] = xyz_to_lab(table.pcs.xyz(run(&table.steps, darkest)));
    let l = if l > 95.0 || l < 0.0 { 0.0 } else { l.min(50.0) };
    Ok(lab_to_xyz([l, a, b]))
}

/// The profile's black point for a perceptual or saturation conversion,
/// as littleCMS's cmsDetectBlackPoint finds it: for a version 4 profile,
/// its darker colorant through the relative colorimetric table if it is a
/// matrix-shaper and the ICC's perceptual black if not; for an older one,
/// its darker colorant through the intent's table.
fn black_point(profile: &ColorProfile, icc: &[u8], intent: RenderingIntent) -> Result<[f64; 3], String> {
    if profile.version() >= ProfileVersion::V4_0 {
        if is_matrix_shaper(profile) {
            return darker_colorant(profile, icc, RenderingIntent::RelativeColorimetric);
        }
        return Ok(PERCEPTUAL_BLACK);
    }
    darker_colorant(profile, icc, intent)
}

/// The profile's media white as littleCMS's _cmsReadMediaWhitePoint reads
/// it: D50 where it records none or is an older display profile.
fn media_white(profile: &ColorProfile) -> [f64; 3] {
    match profile.media_white_point {
        Some(w) if profile.version() >= ProfileVersion::V4_0 || profile.profile_class != ProfileClass::DisplayDevice => [w.x, w.y, w.z],
        _ => D50,
    }
}

/// Whether a scale and offset in XYZ are near enough to none that
/// littleCMS's IsEmptyLayer leaves them out: their distances from 1 and
/// from 0, the offset's taken in littleCMS's XYZ encoding, sum below
/// 0.002.
fn negligible(scale: [f64; 3], offset: [f64; 3]) -> bool {
    let diagonal: f64 = scale.iter().map(|s| (s - 1.0).abs()).sum();
    let shift: f64 = offset.iter().map(|o| (o / MAX_ENCODEABLE_XYZ).abs()).sum();
    diagonal + shift < 0.002
}

/// A conversion from a profile to sRGB through the module's own steps, set
/// up for one intent.
pub(crate) struct TableConversion {
    steps: Vec<Step>,
    pcs: Pcs,
    /// A scale and an offset in XYZ after the table: black point
    /// compensation, or the absolute intent's white; `None` where
    /// littleCMS applies neither or leaves it out as negligible.
    adjust: Option<([f64; 3], [f64; 3])>,
    /// XYZ to linear sRGB.
    to_srgb: [[f64; 3]; 3],
    /// The channels the table takes.
    pub(crate) inputs: usize,
    /// The table's tag: D2B0 to D2B3, or A2B0, A2B1 or A2B2; `None` for a
    /// matrix-shaper's matrix and curves.
    pub(crate) tag: Option<&'static str>,
    /// What the conversion goes through and what was done beside it, for
    /// the import's notes.
    pub(crate) said: String,
}

/// What a conversion of `inputs` channels goes through, by the tag of its
/// table, `None` for a matrix-shaper's: its table, its gray curve, or its
/// matrix and curves.
fn through(tag: Option<&str>, inputs: usize) -> String {
    match tag {
        Some(tag) => format!("through its {tag} table"),
        None if inputs == 1 => "through its gray curve".to_string(),
        None => "through its matrix and curves".to_string(),
    }
}

impl TableConversion {
    /// The conversion from `profile`, stored as `icc`, to sRGB with
    /// `intent`, through the profile's table for the intent where it has
    /// one, and otherwise through its matrix and curves where moxcms's
    /// conversion by them would differ from littleCMS's: where littleCMS
    /// compensates a black point that is not 0 or scales a media white that
    /// is not D50, which moxcms does not, where the profile holds A2B
    /// tables, which take moxcms off its matrix path, or where its PCS is
    /// Lab, which moxcms's matrix path does not connect through. `None`
    /// where moxcms converts as littleCMS does. `to_srgb` takes XYZ to
    /// linear sRGB.
    pub(crate) fn new(profile: &ColorProfile, icc: &[u8], intent: RenderingIntent, to_srgb: [[f64; 3]; 3]) -> Result<Option<TableConversion>, String> {
        let Some(conversion) = TableConversion::evaluated(profile, icc, intent, to_srgb)? else {
            return Ok(None);
        };
        let tables = profile.lut_a_to_b_perceptual.is_some() || profile.lut_a_to_b_colorimetric.is_some() || profile.lut_a_to_b_saturation.is_some();
        if conversion.tag.is_none() && conversion.adjust.is_none() && !tables && profile.pcs == DataColorSpace::Xyz {
            return Ok(None);
        }
        Ok(Some(conversion))
    }

    /// The conversion from `profile`, stored as `icc`, with `intent`
    /// through the module's own steps, whatever moxcms's would give: the
    /// profile's table for the intent where it has one, and otherwise its
    /// matrix and curves; `None` for a profile with neither. `to_srgb` takes
    /// XYZ to linear sRGB.
    pub(crate) fn evaluated(profile: &ColorProfile, icc: &[u8], intent: RenderingIntent, to_srgb: [[f64; 3]; 3]) -> Result<Option<TableConversion>, String> {
        let table = match table_for(profile, icc, intent)? {
            Some(table) => table,
            None => match matrix_shaper(profile)? {
                Some(table) => table,
                None => return Ok(None),
            },
        };
        let Table { steps, pcs, inputs, tag } = table;
        let mut said = through(tag, inputs);
        // sRGB as littleCMS builds it, with sRGB's curves or with curves of
        // gamma 1, is a version 4 profile with a black of 0 and a media
        // white of D50: littleCMS compensates black points for the
        // perceptual and saturation intents toward it, and scales the
        // absolute intent's white to its D50.
        let adjust = match intent {
            RenderingIntent::RelativeColorimetric => None,
            RenderingIntent::AbsoluteColorimetric => {
                let white = media_white(profile);
                let scale = [white[0] / D50[0], white[1] / D50[1], white[2] / D50[2]];
                (!negligible(scale, [0.0; 3])).then(|| {
                    said.push_str(", the profile's media white scaled to sRGB's");
                    (scale, [0.0; 3])
                })
            }
            RenderingIntent::Perceptual | RenderingIntent::Saturation => {
                let black = black_point(profile, icc, intent)?;
                let scale = [0, 1, 2].map(|i| D50[i] / (D50[i] - black[i]));
                let offset = [0, 1, 2].map(|i| -scale[i] * black[i]);
                (black != [0.0; 3] && !negligible(scale, offset)).then(|| {
                    said.push_str(", its black point compensated to sRGB's black");
                    (scale, offset)
                })
            }
        };
        Ok(Some(TableConversion { steps, pcs, adjust, to_srgb, inputs, tag, said }))
    }

    /// The conversion from CIE Lab under D50 to sRGB, as littleCMS converts
    /// from the Lab profile its cmsCreateLab2Profile builds: an abstract
    /// profile whose table passes Lab through, whose black point is 0 and
    /// whose media white is D50, so no intent adjusts it. Its three inputs
    /// run from 0 to 1: L over 100, and a and b each plus 128 over 255.
    pub(crate) fn lab(to_srgb: [[f64; 3]; 3]) -> TableConversion {
        TableConversion { steps: Vec::new(), pcs: Pcs::Lab { version2: false }, adjust: None, to_srgb, inputs: 3, tag: None, said: "as littleCMS converts from its own Lab profile".to_string() }
    }

    /// What the conversion goes through: its table, its gray curve, or its
    /// matrix and curves.
    pub(crate) fn by(&self) -> String {
        through(self.tag, self.inputs)
    }

    /// The XYZ of a pixel whose `device` values run from 0 to 1, through
    /// `steps`, the conversion's own or those after the ones already taken.
    fn xyz(&self, steps: &[Step], device: &[f64]) -> [f64; 3] {
        let xyz = self.pcs.xyz(run(steps, device));
        match self.adjust {
            Some((scale, offset)) => [0, 1, 2].map(|i| xyz[i] * scale[i] + offset[i]),
            None => xyz,
        }
    }

    /// XYZ as sRGB, encoded and from 0 to 1.
    fn encoded(&self, xyz: [f64; 3]) -> [f64; 3] {
        self.to_srgb.map(|row| encode(row[0] * xyz[0] + row[1] * xyz[1] + row[2] * xyz[2]))
    }

    /// A pixel's sRGB, encoded and from 0 to 1, from `device` values that
    /// run from 0 to 1, worked out alone: what a tabled conversion gives the
    /// same pixel.
    #[cfg(test)]
    pub(crate) fn rgb(&self, device: &[f64]) -> [f64; 3] {
        self.encoded(self.xyz(&self.steps, device))
    }

    /// A gray pixel's sRGB-encoded gray from 0 to 1, its Y taken through
    /// sRGB's curve, from `device` values that run from 0 to 1.
    pub(crate) fn gray(&self, device: &[f64]) -> f64 {
        encode(self.xyz(&self.steps, device)[1])
    }

    /// A pixel's light in sRGB's primaries, linear, from `device` values,
    /// a float image's, which may run past 0 and 1: light below 0, which
    /// colors sRGB lacks give, clipped to 0, and light above 1 kept. A gray
    /// pixel's is its light in all three.
    pub(crate) fn linear(&self, device: &[f64]) -> [f64; 3] {
        let xyz = self.xyz(&self.steps, device);
        self.to_srgb.map(|row| (row[0] * xyz[0] + row[1] * xyz[1] + row[2] * xyz[2]).max(0.0))
    }

    /// A pixel's XYZ under D50 from `device` values that run from 0 to 1,
    /// nothing clipped.
    pub(crate) fn xyz_of(&self, device: &[f64]) -> [f64; 3] {
        self.xyz(&self.steps, device)
    }

    /// The conversion with what depends on one sample alone worked out for
    /// every value of `T`: a gray conversion whole, or else the curves of
    /// its first step where that is curves. The results are those of
    /// working each pixel out, from fewer calculations.
    pub(crate) fn tabled<T: Sample>(&self) -> Tabled<'_, T> {
        let unit = |v: usize| v as f64 / T::LARGEST as f64;
        let gray = (self.inputs == 1).then(|| (0..=T::LARGEST).map(|v| self.gray(&[unit(v)])).collect());
        let curves = match (&gray, self.steps.first()) {
            (None, Some(Step::Curves(curves))) => Some(curves.iter().map(|curve| (0..=T::LARGEST).map(|v| curve.at(unit(v))).collect()).collect()),
            _ => None,
        };
        Tabled { conversion: self, curves, gray, sample: PhantomData }
    }
}

/// An integer sample, of 8 or 16 bits, whose every value a table can hold.
pub(crate) trait Sample: Copy + Into<usize> {
    /// The sample's largest value, a full channel.
    const LARGEST: usize;
}

impl Sample for u8 {
    const LARGEST: usize = 255;
}

impl Sample for u16 {
    const LARGEST: usize = 65535;
}

/// A conversion with what depends on one sample alone worked out for every
/// value of the sample type `T`.
pub(crate) struct Tabled<'a, T> {
    conversion: &'a TableConversion,
    /// The first step's curves at every value, a table for each channel,
    /// where the first step is curves and the conversion is not of gray.
    curves: Option<Vec<Vec<f64>>>,
    /// A gray conversion's sRGB-encoded gray at every value.
    gray: Option<Vec<f64>>,
    sample: PhantomData<T>,
}

impl<T: Sample> Tabled<'_, T> {
    /// A pixel's sRGB, encoded and from 0 to 1, from its samples.
    pub(crate) fn rgb(&self, samples: &[T]) -> [f64; 3] {
        let conversion = self.conversion;
        let mut device = [0.0; 16];
        let n = samples.len().min(16);
        let steps = match &self.curves {
            Some(tables) => {
                for ((d, table), &s) in device.iter_mut().zip(tables).zip(samples) {
                    *d = table[s.into()];
                }
                &conversion.steps[1..]
            }
            None => {
                for (d, &s) in device.iter_mut().zip(samples) {
                    *d = s.into() as f64 / T::LARGEST as f64;
                }
                &conversion.steps[..]
            }
        };
        conversion.encoded(conversion.xyz(steps, &device[..n]))
    }

    /// A gray pixel's sRGB-encoded gray from 0 to 1, from its sample.
    pub(crate) fn gray(&self, sample: T) -> f64 {
        match &self.gray {
            Some(table) => table[sample.into()],
            None => self.conversion.gray(&[sample.into() as f64 / T::LARGEST as f64]),
        }
    }
}

/// Linear light to sRGB's encoding, clamped to 0 to 1, as littleCMS
/// inverts the sRGB curve its built-in profile carries.
pub(crate) fn encode(linear: f64) -> f64 {
    let (g, a, b, c, d): (f64, f64, f64, f64, f64) = (2.4, 1.0 / 1.055, 0.055 / 1.055, 1.0 / 12.92, 0.04045);
    let threshold = (a * d + b).powf(g);
    let encoded = if linear >= threshold { (linear.powf(1.0 / g) - b) / a } else { linear / c };
    encoded.clamp(0.0, 1.0)
}

/// A value from 0 to 1 as littleCMS writes a 16-bit sample: half up,
/// within 0 to 65535.
pub(crate) fn to_16(v: f64) -> u16 {
    let d = v * 65535.0 + 0.5;
    if d <= 0.0 {
        0
    } else if d >= 65535.0 {
        65535
    } else {
        d.floor() as u16
    }
}

/// A 16-bit sample as littleCMS's FROM_16_TO_8 takes it to 8 bits.
pub(crate) fn to_8(v: u16) -> u8 {
    ((u32::from(v) * 65281 + 8_388_608) >> 24) as u8
}
