//! Cube maps, and the maps image-based lighting reads from them. A cube is
//! made from six faces, or from one image holding them: a layout of square
//! cells, whose faces are cut as they are, or a panorama, an angular map or
//! an octahedral map, resampled on the GPU so each texel averages the image
//! over the solid angle it covers. A cube is written back into any of them
//! the same ways. From an environment cube come the irradiance cube and its
//! spherical harmonics and the prefiltered specular cube; the BRDF table
//! depends on no environment. Faces are in the order +X, -X, +Y, -Y, +Z,
//! -Z, each laid out as Direct3D, Vulkan and wgpu lay them.

use std::f64::consts::PI;

use pwrs::prelude::*;
use wgpu::util::DeviceExt;
use wgpu::TextureFormat as F;

use crate::convert::{self, MipOptions};
use crate::device::{core_for, default_core, CoreRef, Device};
use crate::error::invalid;
use crate::image_io::{self, parse_target, Source, Target};
use crate::tasks::Interrupt;
use crate::texel;
use crate::texfile::{self, Said, TextureFile};
use crate::texture::{format_name, tell, Texture};

/// How one image holds a cube map's six faces: in square cells, as
/// texassemble and Godot lay them out, or in a projection of every
/// direction.
#[psenum(name = "Wgpu.CubeLayout")]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum CubeLayout {
    /// Four cells across and three down: -X, +Z, +X and -Z across the
    /// middle row, +Y above +Z and -Y below it.
    #[default]
    HorizontalCross,
    /// Three cells across and four down: -X, +Z and +X across the second
    /// row, +Y above +Z, and -Y and then -Z below it.
    VerticalCross,
    /// The vertical cross with -Z turned half a turn, so it joins -Y above
    /// it, as HDRShop's light probes and texassemble's v-cross-fnz lay it
    /// out.
    VerticalCrossTurned,
    /// Four cells across and three down: +Z, +X, -Z and -X across the
    /// middle row, +Y above +Z and -Y below it.
    HorizontalTee,
    /// Six cells in a row, in face order.
    HorizontalStrip,
    /// Six cells in a column, in face order.
    VerticalStrip,
    /// Two cells across and three down, in face order row by row, as Godot
    /// reads a 2x3 cube map.
    Grid2x3,
    /// Three cells across and two down, in face order row by row, as Godot
    /// reads a 3x2 cube map.
    Grid3x2,
    /// Debevec's angular map, HDRShop's light probe: a disc whose center
    /// faces -Z and whose rim faces +Z, the angle from -Z growing evenly
    /// with the distance from the center, +X to the right and +Y up.
    AngularMap,
    /// Filament's octahedral map: +Y at the center, +X to the right and +Z
    /// down, the lower half of the sphere folded out to the corners.
    Octahedral,
}

/// How a prefiltered specular map's mip levels step through roughness.
#[psenum(name = "Wgpu.RoughnessMapping")]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum RoughnessMapping {
    /// Level n of N holds perceptual roughness n / (N - 1), down to 2x2, as
    /// Khronos's glTF IBL sampler writes the levels and glTF viewers read
    /// them.
    #[default]
    Linear,
    /// Level n of N holds perceptual roughness 1 - sqrt(1 - n / (N - 1)),
    /// down to 16x16, as Filament's cmgen writes the levels.
    Quadratic,
}

impl RoughnessMapping {
    /// The perceptual roughness level `level` of `count` holds.
    fn roughness(self, level: u32, count: u32) -> f64 {
        if count < 2 {
            return 0.0;
        }
        let lod = f64::from(level) / f64::from(count - 1);
        match self {
            RoughnessMapping::Linear => lod,
            RoughnessMapping::Quadratic => 1.0 - (1.0 - lod).sqrt(),
        }
    }

    /// The levels a specular map of faces `size` texels across has when
    /// the call names no count: down to 2x2 for Linear, and to 16x16 for
    /// Quadratic, at least two where the faces have room for them.
    fn levels(self, size: u32) -> u32 {
        let full = 32 - size.leading_zeros();
        match self {
            RoughnessMapping::Linear => full.saturating_sub(1).max(1),
            RoughnessMapping::Quadratic => full.saturating_sub(4).max(2).min(full),
        }
    }
}

/// The irradiance of an environment as nine spherical harmonics a color
/// channel, bands 0 to 2, as cmgen's --sh-irradiance writes them: each is
/// the environment's coefficient in the real orthonormal basis times the
/// clamped cosine's, pi, 2 pi / 3 or pi / 4 by band, so the irradiance a
/// surface facing the unit direction (x, y, z) receives is the sum of each
/// coefficient times its basis function there. Index 0 holds the
/// coefficient of 0.282095; 1, 2 and 3 of -0.488603 y, 0.488603 z and
/// -0.488603 x; 4 to 8 of 1.092548 xy, -1.092548 yz, 0.315392 (3z^2 - 1),
/// -1.092548 xz and 0.546274 (x^2 - y^2).
#[psclass(name = "Wgpu.Harmonics", columns = ["Red", "Green", "Blue"])]
#[derive(Default, Clone, Debug, PartialEq)]
pub struct Harmonics {
    /// Red's nine coefficients.
    pub red: Vec<f64>,
    /// Green's nine coefficients.
    pub green: Vec<f64>,
    /// Blue's nine coefficients.
    pub blue: Vec<f64>,
}

/// The direction of the point (s, t) of face `face`, s running right and t
/// down it, each from -1 to 1, not normalized, as the shaders compute it.
pub(crate) fn face_direction(face: usize, s: f64, t: f64) -> [f64; 3] {
    match face {
        0 => [1.0, -t, -s],
        1 => [-1.0, -t, s],
        2 => [s, 1.0, t],
        3 => [s, -1.0, -t],
        4 => [s, -t, 1.0],
        _ => [-s, -t, -1.0],
    }
}

fn normalized(d: [f64; 3]) -> [f64; 3] {
    let length = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt();
    d.map(|v| v / length)
}

/// The solid angle texel (x, y) of a face `size` texels across covers.
pub(crate) fn texel_solid_angle(size: u32, x: u32, y: u32) -> f64 {
    let at = |i: u32| 2.0 * f64::from(i) / f64::from(size) - 1.0;
    let area = |x: f64, y: f64| (x * y).atan2((x * x + y * y + 1.0).sqrt());
    let (x0, x1, y0, y1) = (at(x), at(x + 1), at(y), at(y + 1));
    area(x0, y0) - area(x0, y1) - area(x1, y0) + area(x1, y1)
}

/// The angle a texel of a face one texel across spans at the face's
/// corners, where a cube's texels are smallest: 2 / 3^(3/4).
fn corner_angle() -> f64 {
    2.0 / 3f64.powf(0.75)
}

/// A projection of every direction onto one image.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Projection {
    /// Center +X and left quarter -Z, as three.js and Khronos's glTF IBL
    /// sampler read it: u = atan2(z, x) / 2 pi + 1 / 2 and v = acos(y) / pi.
    Panorama,
    /// Center -Z and rim +Z, +X right and +Y up.
    AngularMap,
    /// Center +Y, +X right and +Z down, the lower half folded out.
    Octahedral,
}

impl Projection {
    /// The number the shaders know the projection by.
    fn code(self) -> u32 {
        match self {
            Projection::Panorama => 0,
            Projection::AngularMap => 1,
            Projection::Octahedral => 2,
        }
    }

    fn name(self) -> &'static str {
        match self {
            Projection::Panorama => "panorama",
            Projection::AngularMap => "angular map",
            Projection::Octahedral => "octahedral map",
        }
    }

    /// The width and height of an image whose faces are `size` texels
    /// across: a panorama 4 faces wide and 2 high, an angular map 4 faces
    /// across and an octahedral map 2.
    fn image_size(self, size: u32) -> (u32, u32) {
        match self {
            Projection::Panorama => (4 * size, 2 * size),
            Projection::AngularMap => (4 * size, 4 * size),
            Projection::Octahedral => (2 * size, 2 * size),
        }
    }

    /// The faces' size an image `width` texels across gives: a quarter of
    /// a panorama's or an angular map's width, half an octahedral map's.
    fn face_size(self, width: u32) -> u32 {
        match self {
            Projection::Panorama | Projection::AngularMap => width / 4,
            Projection::Octahedral => width / 2,
        }
        .max(1)
    }

    /// The angle a texel of an image `width` texels across spans where
    /// its texels are smallest, along the direction they are smallest in
    /// only at a pole or a rim, which no texel covers alone.
    fn finest(self, width: u32) -> f64 {
        match self {
            Projection::Panorama | Projection::AngularMap => 2.0 * PI / f64::from(width),
            Projection::Octahedral => 2.0 / f64::from(width),
        }
    }

    /// The angle a texel of an image `width` texels across spans where its
    /// texels are largest: a panorama's equator, an angular map's center
    /// and the middle of an octahedral map's edges.
    fn coarsest(self, width: u32) -> f64 {
        match self {
            Projection::Panorama | Projection::AngularMap => 2.0 * PI / f64::from(width),
            Projection::Octahedral => 2.0 * 2f64.powf(0.75) / f64::from(width),
        }
    }
}

/// Where a layout of square cells puts the faces.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Cells {
    columns: u32,
    rows: u32,
    /// Each face's cell, (column, row), in face order.
    at: [(u32, u32); 6],
    /// Whether -Z is turned half a turn in its cell.
    turned: bool,
}

/// What one image holds a cube in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Shape {
    Layout(CubeLayout, Cells),
    Projection(Projection),
}

/// The shape `layout` names, with the cells texassemble and Godot put the
/// faces in.
pub(crate) fn shape(layout: CubeLayout) -> Shape {
    use CubeLayout as L;
    let ring = [(2, 1), (0, 1), (1, 0), (1, 2), (1, 1), (3, 1)];
    let column = [(2, 1), (0, 1), (1, 0), (1, 2), (1, 1), (1, 3)];
    let cells = |columns, rows, at, turned| Shape::Layout(layout, Cells { columns, rows, at, turned });
    match layout {
        L::HorizontalCross => cells(4, 3, ring, false),
        L::VerticalCross => cells(3, 4, column, false),
        L::VerticalCrossTurned => cells(3, 4, column, true),
        L::HorizontalTee => cells(4, 3, [(1, 1), (3, 1), (0, 0), (0, 2), (0, 1), (2, 1)], false),
        L::HorizontalStrip => cells(6, 1, [(0, 0), (1, 0), (2, 0), (3, 0), (4, 0), (5, 0)], false),
        L::VerticalStrip => cells(1, 6, [(0, 0), (0, 1), (0, 2), (0, 3), (0, 4), (0, 5)], false),
        L::Grid2x3 => cells(2, 3, [(0, 0), (1, 0), (0, 1), (1, 1), (0, 2), (1, 2)], false),
        L::Grid3x2 => cells(3, 2, [(0, 0), (1, 0), (2, 0), (0, 1), (1, 1), (2, 1)], false),
        L::AngularMap => Shape::Projection(Projection::AngularMap),
        L::Octahedral => Shape::Projection(Projection::Octahedral),
    }
}

/// The shapes an image's aspect ratio gives, for a refusal.
const SHAPES: &str = "a panorama is 2:1, a horizontal cross 4:3, a vertical cross 3:4, a strip 6:1 or 1:6, and Godot's grids 2:3 and 3:2";

/// The shape a `width` by `height` image holds a cube in: the one `named`,
/// which the image must fit, or the one its aspect ratio gives.
pub(crate) fn shape_of(width: u32, height: u32, named: Option<CubeLayout>) -> PsResult<Shape> {
    let (w, h) = (u64::from(width), u64::from(height));
    if let Some(layout) = named {
        let found = shape(layout);
        let fits = match found {
            Shape::Layout(_, cells) => {
                let (columns, rows) = (u64::from(cells.columns), u64::from(cells.rows));
                w > 0 && w.is_multiple_of(columns) && h.is_multiple_of(rows) && w / columns == h / rows
            }
            Shape::Projection(_) => w == h,
        };
        if !fits {
            let needs = match found {
                Shape::Layout(_, cells) => format!("{} cells across and {} down", cells.columns, cells.rows),
                Shape::Projection(_) => "square".to_string(),
            };
            return Err(invalid("WgpuCubeMap", format!("a {layout:?} is {needs}, which a {width}x{height} image is not")));
        }
        return Ok(found);
    }
    let found = if w == 2 * h {
        Shape::Projection(Projection::Panorama)
    } else if 3 * w == 4 * h {
        shape(CubeLayout::HorizontalCross)
    } else if 4 * w == 3 * h {
        shape(CubeLayout::VerticalCross)
    } else if w == 6 * h {
        shape(CubeLayout::HorizontalStrip)
    } else if h == 6 * w {
        shape(CubeLayout::VerticalStrip)
    } else if 3 * w == 2 * h {
        shape(CubeLayout::Grid2x3)
    } else if 2 * w == 3 * h {
        shape(CubeLayout::Grid3x2)
    } else if w == h {
        return Err(invalid("WgpuCubeMap", format!("a square image may be an angular map or an octahedral map: give -Layout AngularMap or -Layout Octahedral for this {width}x{height} one")));
    } else {
        return Err(invalid("WgpuCubeMap", format!("a {width}x{height} image has no shape a cube comes in: {SHAPES}; -Layout names a tee, a turned cross, an angular map or an octahedral map")));
    };
    if w == 0 || h == 0 {
        return Err(invalid("WgpuCubeMap", format!("a {width}x{height} image holds no faces")));
    }
    Ok(found)
}

/// How a resampling reads its source, from `ratio`, the texels of the
/// source's level 0 a texel of the result spans: the level it reads, the
/// first where a texel of the result spans 4 of its texels or fewer, and
/// the samples a side each texel takes, enough to fall half a texel of
/// that level apart, from 2 to 16. `levels` is the source's full chain.
fn sampling(ratio: f64, levels: u32) -> (u32, u32) {
    let level = if ratio > 4.0 { ((ratio / 4.0).log2().ceil() as u32).min(levels.saturating_sub(1)) } else { 0 };
    let grid = ((2.0 * ratio / f64::from(1u32 << level)).ceil() as u32).clamp(2, 16);
    (level, grid)
}

/// The six faces `cells` puts in `image`, level 0 of a `width` texels wide
/// image whose texels are uncompressed or in blocks a face `size` texels
/// across covers whole, laid one after another; -Z turned back where the
/// cells turn it, which takes texels one to a block.
fn cut(image: &[u8], width: u32, format: F, cells: &Cells, size: u32) -> PsResult<Vec<u8>> {
    let (bw, bh) = format.block_dimensions();
    let block = format.block_copy_size(None).ok_or_else(|| invalid("WgpuCubeMap", format!("a {} texture has no layout faces can be cut from", format_name(format))))? as usize;
    let across = (width / bw) as usize;
    let (face_across, face_rows) = ((size / bw) as usize, (size / bh) as usize);
    let mut out = Vec::with_capacity(6 * face_across * face_rows * block);
    for (face, &(column, row)) in cells.at.iter().enumerate() {
        let turned = cells.turned && face == 5;
        for r in 0..face_rows {
            let from = if turned { face_rows - 1 - r } else { r };
            let start = ((row as usize * face_rows + from) * across + column as usize * face_across) * block;
            let line = &image[start..start + face_across * block];
            if turned {
                for b in (0..face_across).rev() {
                    out.extend_from_slice(&line[b * block..(b + 1) * block]);
                }
            } else {
                out.extend_from_slice(line);
            }
        }
    }
    Ok(out)
}

/// An image of `cells` holding the six faces of `faces`, uncompressed
/// faces `size` texels across laid one after another, each in its cell and
/// -Z turned where the cells turn it; the cells no face takes are zero.
fn paste(faces: &[u8], format: F, cells: &Cells, size: u32) -> PsResult<Vec<u8>> {
    let texel = format.block_copy_size(None).ok_or_else(|| invalid("WgpuCubeMap", format!("a {} texture has no layout faces can be laid out in", format_name(format))))? as usize;
    let size = size as usize;
    let across = cells.columns as usize * size;
    let mut out = vec![0u8; across * cells.rows as usize * size * texel];
    let face_len = size * size * texel;
    for (face, &(column, row)) in cells.at.iter().enumerate() {
        let turned = cells.turned && face == 5;
        let texels = &faces[face * face_len..(face + 1) * face_len];
        for r in 0..size {
            let to = if turned { size - 1 - r } else { r };
            let start = ((row as usize * size + to) * across + column as usize * size) * texel;
            let line = &texels[r * size * texel..(r + 1) * size * texel];
            if turned {
                for (i, t) in line.chunks_exact(texel).rev().enumerate() {
                    out[start + i * texel..start + (i + 1) * texel].copy_from_slice(t);
                }
            } else {
                out[start..start + size * texel].copy_from_slice(line);
            }
        }
    }
    Ok(out)
}

/// What every shader here starts with: the uniforms, the full-screen
/// triangle, the faces' directions, the projections, and the Hammersley
/// points and GGX lobe the lighting passes sample.
const COMMON: &str = "
struct Params {
    face: u32,
    width: u32,
    height: u32,
    kind: u32,
    grid: u32,
    lod: u32,
    yaw: f32,
    roughness: f32,
    samples: u32,
    env_size: u32,
    mapping: u32,
    unused: u32,
}

@group(0) @binding(0) var<uniform> p: Params;

const PI: f32 = 3.14159265358979;

@vertex fn vertex(@builtin(vertex_index) index: u32) -> @builtin(position) vec4<f32> {
    let corner = vec2<f32>(f32((index << 1u) & 2u), f32(index & 2u));
    return vec4<f32>(corner.x * 2.0 - 1.0, 1.0 - corner.y * 2.0, 0.0, 1.0);
}

// The direction of the point (s, t) of face `face`, s running right and t
// down it, each from -1 to 1, not normalized: the faces in the order +X,
// -X, +Y, -Y, +Z, -Z, laid out as Direct3D, Vulkan and wgpu lay them. It
// returns once, after the switch, as FXC refuses a function whose returns
// are all inside one.
fn face_direction(face: u32, s: f32, t: f32) -> vec3<f32> {
    var d = vec3<f32>(-s, -t, -1.0);
    switch face {
        case 0u: {
            d = vec3<f32>(1.0, -t, -s);
        }
        case 1u: {
            d = vec3<f32>(-1.0, -t, s);
        }
        case 2u: {
            d = vec3<f32>(s, 1.0, t);
        }
        case 3u: {
            d = vec3<f32>(s, -1.0, -t);
        }
        case 4u: {
            d = vec3<f32>(s, -t, 1.0);
        }
        default: {}
    }
    return d;
}

// The face a direction points at, and the point (s, t) on it.
struct Spot {
    face: u32,
    st: vec2<f32>,
}

fn spot(d: vec3<f32>) -> Spot {
    let a = abs(d);
    var r: Spot;
    if (a.x >= a.y && a.x >= a.z) {
        r = Spot(select(1u, 0u, d.x > 0.0), vec2<f32>(select(d.z, -d.z, d.x > 0.0), -d.y) / a.x);
    } else if (a.y >= a.z) {
        r = Spot(select(3u, 2u, d.y > 0.0), vec2<f32>(d.x, select(-d.z, d.z, d.y > 0.0)) / a.y);
    } else {
        r = Spot(select(5u, 4u, d.z > 0.0), vec2<f32>(select(-d.x, d.x, d.z > 0.0), -d.y) / a.z);
    }
    return r;
}

// d turned about the vertical axis by `angle` radians, +X toward -Z.
fn turned(d: vec3<f32>, angle: f32) -> vec3<f32> {
    let c = cos(angle);
    let s = sin(angle);
    return vec3<f32>(c * d.x + s * d.z, d.y, c * d.z - s * d.x);
}

// atan(x) for 0 <= x <= 1, as Cephes's atanf computes it: x brought to
// |t| <= tan(pi / 8) about 0 or pi / 4, then an odd polynomial in t. The
// angles here come from it, within 4e-7 radians on every adapter, where
// WGSL lets an adapter's atan2 and acos be 4096 ulp off.
fn atan_unit(x: f32) -> f32 {
    var base = 0.0;
    var t = x;
    if (x > 0.41421356237309503) {
        base = PI / 4.0;
        t = (x - 1.0) / (x + 1.0);
    }
    let z = t * t;
    return base + ((((8.05374449538e-2 * z - 1.38776856032e-1) * z + 1.99777106478e-1) * z - 3.33329491539e-1) * z * t + t);
}

// atan2(y, x): the angle of the point (x, y) from +X, -pi to pi.
fn precise_atan2(y: f32, x: f32) -> f32 {
    let ax = abs(x);
    let ay = abs(y);
    var a = 0.0;
    if (ay <= ax && ax > 0.0) {
        a = atan_unit(ay / ax);
    } else if (ay > ax) {
        a = PI / 2.0 - atan_unit(ax / ay);
    }
    if (x < 0.0) {
        a = PI - a;
    }
    return select(a, -a, y < 0.0);
}

// acos(x) of x held to -1..1, as the angle of the point (x, sqrt(1 - x^2)).
fn precise_acos(x: f32) -> f32 {
    let c = clamp(x, -1.0, 1.0);
    return precise_atan2(sqrt((1.0 - c) * (1.0 + c)), c);
}

// The direction the point `at` of an image of the projection `kind`,
// `size` texels, shows, and in w the solid angle a unit of the image's
// area covers there, up to a factor the whole image shares: 0 outside an
// angular map's disc. Projection 0 is a panorama, centre +X and left
// quarter -Z; 1 an angular map, centre -Z and rim +Z, +X right and +Y up;
// 2 an octahedral map, centre +Y, +X right and +Z down.
fn image_direction(kind: u32, at: vec2<f32>, size: vec2<f32>) -> vec4<f32> {
    let uv = at / size;
    var r = vec4<f32>(0.0);
    if (kind == 0u) {
        let phi = (uv.x - 0.5) * 2.0 * PI;
        let theta = uv.y * PI;
        r = vec4<f32>(sin(theta) * cos(phi), cos(theta), sin(theta) * sin(phi), sin(theta));
    } else if (kind == 1u) {
        let ab = vec2<f32>(uv.x * 2.0 - 1.0, 1.0 - uv.y * 2.0);
        let radius = length(ab);
        if (radius <= 1.0) {
            r = vec4<f32>(0.0, 0.0, -1.0, PI * PI);
            if (radius > 0.0) {
                let angle = PI * radius;
                r = vec4<f32>(sin(angle) * ab / radius, -cos(angle), PI * sin(angle) / radius);
            }
        }
    } else {
        let q = uv * 2.0 - 1.0;
        let up = 1.0 - abs(q.x) - abs(q.y);
        var d = vec3<f32>(q.x, up, q.y);
        if (up < 0.0) {
            d = vec3<f32>(sign(q.x) * (1.0 - abs(q.y)), up, sign(q.y) * (1.0 - abs(q.x)));
        }
        let span = length(d);
        r = vec4<f32>(d / span, 1.0 / (span * span * span));
    }
    return r;
}

// Where an image of the projection `kind`, `size` texels, shows the unit
// direction d, in texels.
fn image_spot(kind: u32, d: vec3<f32>, size: vec2<f32>) -> vec2<f32> {
    var uv = vec2<f32>(0.5);
    if (kind == 0u) {
        var around = 0.5;
        if (d.x != 0.0 || d.z != 0.0) {
            around = precise_atan2(d.z, d.x) / (2.0 * PI) + 0.5;
        }
        uv = vec2<f32>(around, precise_acos(d.y) / PI);
    } else if (kind == 1u) {
        let across = length(d.xy);
        var ab = vec2<f32>(select(0.0, 1.0, d.z > 0.0), 0.0);
        if (across > 0.0) {
            ab = d.xy / across * (precise_acos(-d.z) / PI);
        }
        uv = vec2<f32>((ab.x + 1.0) * 0.5, (1.0 - ab.y) * 0.5);
    } else {
        let q = d / (abs(d.x) + abs(d.y) + abs(d.z));
        var ab = q.xz;
        if (q.y < 0.0) {
            ab = vec2<f32>(select(-1.0, 1.0, q.x >= 0.0) * (1.0 - abs(q.z)), select(-1.0, 1.0, q.z >= 0.0) * (1.0 - abs(q.x)));
        }
        uv = (ab + 1.0) * 0.5;
    }
    return uv * size;
}

// Van der Corput's radical inverse of i in base 2.
fn radical_inverse(i: u32) -> f32 {
    var b = (i << 16u) | (i >> 16u);
    b = ((b & 0x55555555u) << 1u) | ((b & 0xAAAAAAAAu) >> 1u);
    b = ((b & 0x33333333u) << 2u) | ((b & 0xCCCCCCCCu) >> 2u);
    b = ((b & 0x0F0F0F0Fu) << 4u) | ((b & 0xF0F0F0F0u) >> 4u);
    b = ((b & 0x00FF00FFu) << 8u) | ((b & 0xFF00FF00u) >> 8u);
    return f32(b) * 2.3283064365386963e-10;
}

// The half vector of Hammersley point i of `count` sampling the GGX lobe
// whose alpha squared is a2, about +Z. The cosine's square is held at 1 or
// below, which rounding can pass when a2 is small.
fn ggx_half(i: u32, count: u32, a2: f32) -> vec3<f32> {
    let u = vec2<f32>(f32(i) / f32(count), radical_inverse(i));
    let phi = 2.0 * PI * u.x;
    let c2 = min((1.0 - u.y) / (1.0 + (a2 - 1.0) * u.y), 1.0);
    let s = sqrt(1.0 - c2);
    return vec3<f32>(s * cos(phi), s * sin(phi), sqrt(c2));
}
";

/// The pass that resamples a projection into a cube. The image is read
/// through `source_texel`, which brings a texel past its edges inside as
/// the projection carries on there.
const TO_CUBE: &str = "
@group(0) @binding(1) var source: texture_2d<f32>;

// Texel (x, y) of level p.lod of the image: past a panorama's top or
// bottom row, the row as far down or up, half a turn round; past an
// angular map's disc, the texel as far inside it, half a turn round, and
// at least 3/4 of a texel inside, so its center is in the disc; past an
// octahedral map's edge, the texel it folds onto, the edge mirrored about
// its middle.
fn source_texel(x: i32, y: i32) -> vec4<f32> {
    let size = vec2<i32>(textureDimensions(source, p.lod));
    var c = vec2<i32>(x, y);
    if (p.kind == 0u) {
        if (c.y < 0) {
            c = vec2<i32>(c.x + size.x / 2, -1 - c.y);
        } else if (c.y >= size.y) {
            c = vec2<i32>(c.x + size.x / 2, 2 * size.y - 1 - c.y);
        }
        c = vec2<i32>(((c.x % size.x) + size.x) % size.x, clamp(c.y, 0, size.y - 1));
    } else if (p.kind == 1u) {
        let n = vec2<f32>(size);
        let ab = vec2<f32>(2.0 * (f32(c.x) + 0.5) / n.x - 1.0, 1.0 - 2.0 * (f32(c.y) + 0.5) / n.y);
        let radius = length(ab);
        if (radius > 1.0) {
            let inside = -ab * min(2.0 - radius, 1.0 - 1.5 / n.x) / radius;
            c = vec2<i32>(floor(vec2<f32>((inside.x + 1.0) * 0.5 * n.x, (1.0 - inside.y) * 0.5 * n.y)));
        }
        c = clamp(c, vec2<i32>(0), size - 1);
    } else {
        if (c.x < 0) {
            c = vec2<i32>(-1 - c.x, size.y - 1 - c.y);
        } else if (c.x >= size.x) {
            c = vec2<i32>(2 * size.x - 1 - c.x, size.y - 1 - c.y);
        }
        if (c.y < 0) {
            c = vec2<i32>(size.x - 1 - c.x, -1 - c.y);
        } else if (c.y >= size.y) {
            c = vec2<i32>(size.x - 1 - c.x, 2 * size.y - 1 - c.y);
        }
        c = clamp(c, vec2<i32>(0), size - 1);
    }
    return textureLoad(source, c, i32(p.lod));
}

// The image at the point `at`, in texels, bilinear between the four
// texels nearest.
fn source_at(at: vec2<f32>) -> vec4<f32> {
    let f = at - 0.5;
    let base = floor(f);
    let w = f - base;
    let i = vec2<i32>(base);
    let top = mix(source_texel(i.x, i.y), source_texel(i.x + 1, i.y), w.x);
    let bottom = mix(source_texel(i.x, i.y + 1), source_texel(i.x + 1, i.y + 1), w.x);
    return mix(top, bottom, w.y);
}

// Texel (x, y) of face p.face, p.width texels across: the mean of p.grid
// by p.grid samples over it, each weighed by the solid angle it covers, of
// the image in the direction each faces, turned back by p.yaw.
@fragment fn to_cube(@builtin(position) position: vec4<f32>) -> @location(0) vec4<f32> {
    let size = vec2<f32>(textureDimensions(source, p.lod));
    let k = p.grid;
    var sum = vec4<f32>(0.0);
    var total = 0.0;
    for (var j = 0u; j < k; j++) {
        for (var i = 0u; i < k; i++) {
            let at = floor(position.xy) + (vec2<f32>(f32(i), f32(j)) + 0.5) / f32(k);
            let st = at / f32(p.width) * 2.0 - 1.0;
            let r = inverseSqrt(1.0 + dot(st, st));
            let weight = r * r * r;
            let d = turned(normalize(face_direction(p.face, st.x, st.y)), -p.yaw);
            sum += weight * source_at(image_spot(p.kind, d, size));
            total += weight;
        }
    }
    return sum / total;
}
";

/// The passes that read a cube: resampling it into a projection, and the
/// irradiance and prefiltered specular maps. The cube is read through
/// `cube_texel`, which carries a texel past a face's edge onto the face
/// beside it.
const FROM_CUBE: &str = "
@group(0) @binding(1) var faces: texture_2d_array<f32>;

// Texel (x, y) of face `face` of level `level`; a texel past the face's
// edge is the texel of the face beside it nearest where the face's plane,
// carried on, reaches there.
fn cube_texel(face: u32, x: i32, y: i32, level: i32) -> vec4<f32> {
    let n = i32(textureDimensions(faces, level).x);
    var c = vec2<i32>(x, y);
    var on = face;
    if (x < 0 || y < 0 || x >= n || y >= n) {
        let st = (vec2<f32>(c) + 0.5) / f32(n) * 2.0 - 1.0;
        let beside = spot(face_direction(face, st.x, st.y));
        on = beside.face;
        c = clamp(vec2<i32>(floor((beside.st + 1.0) * 0.5 * f32(n))), vec2<i32>(0), vec2<i32>(n - 1));
    }
    return textureLoad(faces, c, i32(on), level);
}

// Level `level` of the cube in the direction d, bilinear between the four
// texels nearest, across a face's edges as well as inside it. Past a
// corner of the face, where three faces meet and no fourth texel is, the
// texel is the mean of the other three, as seamless cube filtering takes
// it, so the result carries on unbroken from one face to the next.
fn cube_level(d: vec3<f32>, level: i32) -> vec4<f32> {
    let size = i32(textureDimensions(faces, level).x);
    let at = spot(d);
    let f = (at.st + 1.0) * 0.5 * f32(size) - 0.5;
    let base = floor(f);
    let w = f - base;
    let i = vec2<i32>(base);
    var t00 = cube_texel(at.face, i.x, i.y, level);
    var t10 = cube_texel(at.face, i.x + 1, i.y, level);
    var t01 = cube_texel(at.face, i.x, i.y + 1, level);
    var t11 = cube_texel(at.face, i.x + 1, i.y + 1, level);
    let past_left = i.x < 0;
    let past_right = i.x + 1 >= size;
    let past_top = i.y < 0;
    let past_bottom = i.y + 1 >= size;
    if (past_left && past_top) {
        t00 = (t10 + t01 + t11) / 3.0;
    } else if (past_right && past_top) {
        t10 = (t00 + t01 + t11) / 3.0;
    } else if (past_left && past_bottom) {
        t01 = (t00 + t10 + t11) / 3.0;
    } else if (past_right && past_bottom) {
        t11 = (t00 + t10 + t01) / 3.0;
    }
    return mix(mix(t00, t10, w.x), mix(t01, t11, w.x), w.y);
}

// The cube in the direction d at mip level `lod`, between the levels
// either side of it, and at the first or last level past them.
fn cube_at(d: vec3<f32>, lod: f32) -> vec4<f32> {
    let l = clamp(lod, 0.0, f32(textureNumLevels(faces) - 1u));
    let below = floor(l);
    var c = cube_level(d, i32(below));
    if (l > below) {
        c = mix(c, cube_level(d, i32(below) + 1), l - below);
    }
    return c;
}

// Texel (x, y) of an image of p.width by p.height texels of the projection
// p.kind: the mean of p.grid by p.grid samples over it, each weighed by
// the solid angle it covers, of the cube's level p.lod in the direction
// each shows, turned by p.yaw; transparent black where every sample is
// outside an angular map's disc.
@fragment fn from_cube(@builtin(position) position: vec4<f32>) -> @location(0) vec4<f32> {
    let size = vec2<f32>(f32(p.width), f32(p.height));
    let k = p.grid;
    var sum = vec4<f32>(0.0);
    var total = 0.0;
    for (var j = 0u; j < k; j++) {
        for (var i = 0u; i < k; i++) {
            let at = floor(position.xy) + (vec2<f32>(f32(i), f32(j)) + 0.5) / f32(k);
            let seen = image_direction(p.kind, at, size);
            if (seen.w > 0.0) {
                sum += seen.w * cube_level(turned(seen.xyz, p.yaw), i32(p.lod));
                total += seen.w;
            }
        }
    }
    if (total > 0.0) {
        sum = sum / total;
    }
    return sum;
}

// The integral of the density of directions over a face, from its centre
// to the point (x, y).
fn area(x: f32, y: f32) -> f32 {
    return precise_atan2(x * y, sqrt(x * x + y * y + 1.0));
}

// Texel (x, y) of face p.face of the cube's level p.lod, its color times
// the solid angle it covers, and that solid angle in alpha.
@fragment fn weigh(@builtin(position) position: vec4<f32>) -> @location(0) vec4<f32> {
    let n = f32(textureDimensions(faces, p.lod).x);
    let low = floor(position.xy) / n * 2.0 - 1.0;
    let high = (floor(position.xy) + 1.0) / n * 2.0 - 1.0;
    let omega = area(low.x, low.y) - area(low.x, high.y) - area(high.x, low.y) + area(high.x, high.y);
    let c = textureLoad(faces, vec2<i32>(position.xy), i32(p.face), i32(p.lod));
    return vec4<f32>(c.rgb * omega, omega);
}

// Texel (x, y) of face p.face of the irradiance cube, p.width texels
// across: the colors of every texel of the cube the weigh pass wrote, each
// weighed by its solid angle and the cosine between its direction and the
// texel's where that is above 0, divided by the sum of those weights.
@fragment fn irradiance(@builtin(position) position: vec4<f32>) -> @location(0) vec4<f32> {
    let st = position.xy / f32(p.width) * 2.0 - 1.0;
    let normal = normalize(face_direction(p.face, st.x, st.y));
    let m = i32(textureDimensions(faces).x);
    var sum = vec4<f32>(0.0);
    for (var f = 0u; f < 6u; f++) {
        for (var y = 0; y < m; y++) {
            for (var x = 0; x < m; x++) {
                let at = (vec2<f32>(f32(x), f32(y)) + 0.5) / f32(m) * 2.0 - 1.0;
                let cosine = dot(normal, normalize(face_direction(f, at.x, at.y)));
                if (cosine > 0.0) {
                    sum += cosine * textureLoad(faces, vec2<i32>(x, y), i32(f), 0);
                }
            }
        }
    }
    return vec4<f32>(sum.rgb / sum.a, 1.0);
}

// Texel (x, y) of face p.face of a level of the specular cube, p.width
// texels across: at roughness 0 the cube's own texel, and otherwise the
// cube as the GGX lobe of perceptual roughness p.roughness about the
// texel's direction reflects it, the view along that direction, by
// p.samples Hammersley points weighed by the cosine of each reflected
// direction, each read at the mip level whose texels cover about the solid
// angle the sample covers: lod = log2(6 W^2 / (N pdf)) / 2 for Khronos's
// sampler, and log2(Ws / Wp) / 2 + 1, Wp = 4 pi / (6 W^2), for cmgen.
@fragment fn specular(@builtin(position) position: vec4<f32>) -> @location(0) vec4<f32> {
    let st = position.xy / f32(p.width) * 2.0 - 1.0;
    let n = normalize(face_direction(p.face, st.x, st.y));
    var color = textureLoad(faces, vec2<i32>(position.xy), i32(p.face), 0).rgb;
    if (p.roughness > 0.0) {
        let up = select(vec3<f32>(1.0, 0.0, 0.0), vec3<f32>(0.0, 0.0, 1.0), abs(n.z) < 0.999);
        let tx = normalize(cross(up, n));
        let ty = cross(n, tx);
        let alpha = p.roughness * p.roughness;
        let a2 = alpha * alpha;
        let texels = 6.0 * f32(p.env_size) * f32(p.env_size);
        var sum = vec3<f32>(0.0);
        var total = 0.0;
        for (var i = 0u; i < p.samples; i++) {
            let h = ggx_half(i, p.samples, a2);
            let nl = 2.0 * h.z * h.z - 1.0;
            if (nl > 0.0) {
                let l = tx * (2.0 * h.z * h.x) + ty * (2.0 * h.z * h.y) + n * nl;
                let q = h.z * h.z * (a2 - 1.0) + 1.0;
                let pdf = a2 / (4.0 * PI * q * q);
                var lod = 0.5 * log2(texels / (f32(p.samples) * pdf));
                if (p.mapping == 1u) {
                    lod = 0.5 * log2(texels / (4.0 * PI * f32(p.samples) * pdf)) + 1.0;
                }
                sum += cube_at(l, lod).rgb * nl;
                total += nl;
            }
        }
        color = sum / total;
    }
    return vec4<f32>(color, 1.0);
}
";

/// The pass that makes the BRDF table.
const BRDF: &str = "
// Smith's height-correlated GGX visibility.
fn visibility(nv: f32, nl: f32, a2: f32) -> f32 {
    let gv = nl * sqrt(nv * nv * (1.0 - a2) + a2);
    let gl = nv * sqrt(nl * nl * (1.0 - a2) + a2);
    return 0.5 / (gv + gl);
}

// Texel (x, y) of the table, p.width by p.height texels: for the cosine
// (x + 1/2) / p.width of the view's angle and perceptual roughness
// (y + 1/2) / p.height, the integral over the GGX lobe of the specular
// BRDF times the cosine of the light's angle, split into the part F0
// scales, in red, and the part it adds, in green, by p.samples Hammersley
// points, as Khronos's glTF IBL sampler and Filament integrate it.
@fragment fn brdf(@builtin(position) position: vec4<f32>) -> @location(0) vec4<f32> {
    let nv = position.x / f32(p.width);
    let roughness = position.y / f32(p.height);
    let alpha = roughness * roughness;
    let a2 = alpha * alpha;
    let v = vec3<f32>(sqrt(1.0 - nv * nv), 0.0, nv);
    var scale = 0.0;
    var bias = 0.0;
    for (var i = 0u; i < p.samples; i++) {
        let h = ggx_half(i, p.samples, a2);
        let l = 2.0 * dot(v, h) * h - v;
        let nl = clamp(l.z, 0.0, 1.0);
        let nh = clamp(h.z, 0.0, 1.0);
        let vh = clamp(dot(v, h), 0.0, 1.0);
        if (nl > 0.0) {
            let weighed = visibility(nv, nl, a2) * vh * nl / nh;
            let m = 1.0 - vh;
            let m2 = m * m;
            let fc = m2 * m2 * m;
            scale += (1.0 - fc) * weighed;
            bias += fc * weighed;
        }
    }
    return vec4<f32>(4.0 * scale / f32(p.samples), 4.0 * bias / f32(p.samples), 0.0, 1.0);
}
";

/// The uniforms every pass here reads, laid out as the shaders' `Params`.
#[derive(Clone, Copy, Debug, Default)]
struct Params {
    face: u32,
    width: u32,
    height: u32,
    kind: u32,
    grid: u32,
    lod: u32,
    yaw: f32,
    roughness: f32,
    samples: u32,
    env_size: u32,
    mapping: u32,
}

impl Params {
    fn bytes(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(48);
        for v in [self.face, self.width, self.height, self.kind, self.grid, self.lod] {
            out.extend(v.to_le_bytes());
        }
        out.extend(self.yaw.to_le_bytes());
        out.extend(self.roughness.to_le_bytes());
        for v in [self.samples, self.env_size, self.mapping, 0] {
            out.extend(v.to_le_bytes());
        }
        out
    }
}

/// One entry point of a shader here, drawing a full-screen triangle into
/// views of one format, reading the uniforms at binding 0 and a texture at
/// binding 1.
struct Pass {
    pipeline: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
}

impl Pass {
    /// Entry point `entry` of `shader`, drawing into views of `format`; the
    /// texture it reads is viewed as `input`, and it reads none for `None`.
    fn new(core: &CoreRef, shader: &str, entry: &str, input: Option<wgpu::TextureViewDimension>, format: F) -> Pass {
        let module = core.device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("Wgpu cube map"), source: wgpu::ShaderSource::Wgsl(shader.into()) });
        let mut entries = vec![wgpu::BindGroupLayoutEntry {
            binding: 0,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None },
            count: None,
        }];
        if let Some(view_dimension) = input {
            entries.push(wgpu::BindGroupLayoutEntry {
                binding: 1,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Float { filterable: false }, view_dimension, multisampled: false },
                count: None,
            });
        }
        let layout = core.device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor { label: Some("Wgpu cube map"), entries: &entries });
        let pipeline_layout = core.device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some("Wgpu cube map"), bind_group_layouts: &[Some(&layout)], immediate_size: 0 });
        let pipeline = core.device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("Wgpu cube map"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState { module: &module, entry_point: Some("vertex"), compilation_options: Default::default(), buffers: &[] },
            fragment: Some(wgpu::FragmentState {
                module: &module,
                entry_point: Some(entry),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState { format, blend: None, write_mask: wgpu::ColorWrites::ALL })],
            }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });
        Pass { pipeline, layout }
    }

    /// Draws `count` rows from row `first` of `output`, `width` texels
    /// wide, reading `input` with `params`.
    #[allow(clippy::too_many_arguments)]
    fn draw(&self, core: &CoreRef, encoder: &mut wgpu::CommandEncoder, input: Option<&wgpu::TextureView>, output: &wgpu::TextureView, params: &Params, width: u32, first: u32, count: u32) {
        let uniform = core.device.create_buffer_init(&wgpu::util::BufferInitDescriptor { label: Some("Wgpu cube map"), contents: &params.bytes(), usage: wgpu::BufferUsages::UNIFORM });
        let mut entries = vec![wgpu::BindGroupEntry { binding: 0, resource: uniform.as_entire_binding() }];
        if let Some(view) = input {
            entries.push(wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(view) });
        }
        let bind_group = core.device.create_bind_group(&wgpu::BindGroupDescriptor { label: Some("Wgpu cube map"), layout: &self.layout, entries: &entries });
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("Wgpu cube map"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: output,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations { load: wgpu::LoadOp::Load, store: wgpu::StoreOp::Store },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &bind_group, &[]);
        pass.set_scissor_rect(0, first, width, count);
        pass.draw(0..3, 0..1);
    }
}

/// A view of `texture` as `dimension`: `levels` mip levels from `level`, or
/// every one from it, and `layers` layers from `layer`.
fn view(texture: &wgpu::Texture, dimension: wgpu::TextureViewDimension, level: u32, levels: Option<u32>, layer: u32, layers: u32) -> wgpu::TextureView {
    texture.create_view(&wgpu::TextureViewDescriptor {
        label: Some("Wgpu cube map"),
        format: None,
        dimension: Some(dimension),
        usage: None,
        aspect: wgpu::TextureAspect::All,
        base_mip_level: level,
        mip_level_count: levels,
        base_array_layer: layer,
        array_layer_count: Some(layers),
    })
}

/// The format the passes here render into: Rgba32Float, or Rgba16Float on
/// a device that does not render into it.
fn rendered(core: &CoreRef) -> F {
    if convert::float_renderable(core, F::Rgba32Float) { F::Rgba32Float } else { F::Rgba16Float }
}

/// A texture the passes render into and the module reads back: `layers`
/// layers of `width` by `height` texels of `format`, with `levels` mip
/// levels.
fn target(core: &CoreRef, width: u32, height: u32, layers: u32, levels: u32, format: F) -> PsResult<wgpu::Texture> {
    core.scoped("WgpuCubeMap", "the texture a cube map pass renders into cannot be made", || {
        core.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("Wgpu cube map"),
            size: wgpu::Extent3d { width, height, depth_or_array_layers: layers },
            mip_level_count: levels,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        })
    })
}

/// The work one submission may take: texels drawn times what each costs.
/// A long pass is drawn in bands of rows, each submitted on its own, so no
/// one submission runs long enough for a driver's watchdog to end it.
const BAND: u64 = 1 << 24;

/// Entry point `entry` of `shader` as a pass drawing into views of
/// `format`, reading a texture viewed as `input`, or none.
fn pass(core: &CoreRef, shader: &str, entry: &str, input: Option<wgpu::TextureViewDimension>, format: F) -> PsResult<Pass> {
    core.scoped("WgpuCubeMap", "a cube map pass cannot be made", || Pass::new(core, shader, entry, input, format))
}

/// The view of every mip level of `texture`, `layers` layers from layer 0,
/// as `dimension`, that a pass reads.
fn read_view(core: &CoreRef, texture: &wgpu::Texture, dimension: wgpu::TextureViewDimension, layers: u32) -> PsResult<wgpu::TextureView> {
    core.scoped("WgpuCubeMap", "a cube map pass cannot read its texture", || view(texture, dimension, 0, None, 0, layers))
}

/// Draws `pass` into layer `layer` of mip level `level` of `target`,
/// reading `input` with `params`, in bands of rows each submitted on its
/// own, at `cost` a texel.
#[allow(clippy::too_many_arguments)]
fn draw_into(
    core: &CoreRef,
    pass: &Pass,
    input: Option<&wgpu::TextureView>,
    target: &wgpu::Texture,
    level: u32,
    layer: u32,
    params: &Params,
    cost: u64,
    stop: Option<&dyn Interrupt>,
) -> PsResult<()> {
    let size = target.size().mip_level_size(level, wgpu::TextureDimension::D2);
    let rows = (BAND / (u64::from(size.width) * cost.max(1))).clamp(1, u64::from(size.height)) as u32;
    let output = core.scoped("WgpuCubeMap", "a cube map pass cannot be drawn", || view(target, wgpu::TextureViewDimension::D2, level, Some(1), layer, 1))?;
    let mut first = 0;
    while first < size.height {
        let count = rows.min(size.height - first);
        let commands = core.scoped("WgpuCubeMap", "a cube map pass cannot be drawn", || {
            let mut encoder = core.device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("Wgpu cube map") });
            pass.draw(core, &mut encoder, input, &output, params, size.width, first, count);
            encoder.finish()
        })?;
        core.submit_as("WgpuCubeMap", "a cube map pass cannot be submitted", vec![commands], stop)?;
        first += count;
    }
    Ok(())
}

/// The float format `format`'s texels render in, which a resampling reads.
fn readable(format: F, what: &str) -> PsResult<F> {
    texel::render_format(format).ok_or_else(|| invalid("WgpuCubeMap", format!("{what} reads texels as floating-point colors, and {} holds integers, depth or stencil", format_name(format))))
}

/// The cube a projection, `base`, an uncompressed image, resamples into:
/// faces `size` texels across, each texel the mean of a grid of samples
/// over it weighed by the solid angle each covers, the image turned by
/// `yaw` degrees, in the format the passes render in.
fn resampled_cube(core: &CoreRef, base: &TextureFile, projection: Projection, size: u32, yaw: f64, stop: Option<&dyn Interrupt>, said: &mut Said) -> PsResult<TextureFile> {
    let render = readable(base.format, "a cube resampled from an image")?;
    let full = wgpu::Extent3d { width: base.width, height: base.height, depth_or_array_layers: 1 }.max_mips(wgpu::TextureDimension::D2);
    let (level, grid) = sampling((2.0 / f64::from(size)) / projection.finest(base.width), full);
    let work = convert::float_levels(core, base, render, level + 1, 1, MipOptions::default(), stop)?;
    let format = rendered(core);
    let made = target(core, size, size, 6, 1, format)?;
    let to_cube = pass(core, &format!("{COMMON}{TO_CUBE}"), "to_cube", Some(wgpu::TextureViewDimension::D2), format)?;
    let input = read_view(core, &work, wgpu::TextureViewDimension::D2, 1)?;
    for face in 0..6 {
        let params = Params { face, width: size, height: size, kind: projection.code(), grid, lod: level, yaw: yaw.to_radians() as f32, ..Params::default() };
        draw_into(core, &to_cube, Some(&input), &made, 0, face, &params, u64::from(grid * grid), stop)?;
    }
    let read = if level == 0 { String::new() } else { format!(", read from the image's mip level {level}, rendered by the Box filter") };
    let turn = if yaw == 0.0 { String::new() } else { format!(", the image turned {yaw} degrees about the vertical axis, +X toward -Z") };
    said.verbose.push(format!(
        "the {} was resampled on the GPU into faces {size} texels across in {}, each texel the mean of {grid} by {grid} samples over it, each weighed by the solid angle it covers{read}{turn}",
        projection.name(),
        format_name(format)
    ));
    image_io::read_file(core, &made, true, 1, stop)
}

/// The image of the projection `projection` a cube, `base`, six
/// uncompressed square faces with one level, resamples into: faces `size`
/// texels across, each texel the mean of a grid of samples over it weighed
/// by the solid angle each covers, the image turned by `yaw` degrees, in
/// the format the passes render in.
fn resampled_image(core: &CoreRef, base: &TextureFile, projection: Projection, size: u32, yaw: f64, stop: Option<&dyn Interrupt>, said: &mut Said) -> PsResult<TextureFile> {
    let render = readable(base.format, "an image resampled from a cube")?;
    let (width, height) = projection.image_size(size);
    let full = wgpu::Extent3d { width: base.width, height: base.width, depth_or_array_layers: 1 }.max_mips(wgpu::TextureDimension::D2);
    let (level, grid) = sampling(projection.coarsest(width) / (corner_angle() / f64::from(base.width)), full);
    let work = convert::float_levels(core, base, render, level + 1, 1, MipOptions::default(), stop)?;
    let format = rendered(core);
    let made = target(core, width, height, 1, 1, format)?;
    let from_cube = pass(core, &format!("{COMMON}{FROM_CUBE}"), "from_cube", Some(wgpu::TextureViewDimension::D2Array), format)?;
    let input = read_view(core, &work, wgpu::TextureViewDimension::D2Array, 6)?;
    let params = Params { width, height, kind: projection.code(), grid, lod: level, yaw: yaw.to_radians() as f32, ..Params::default() };
    draw_into(core, &from_cube, Some(&input), &made, 0, 0, &params, u64::from(grid * grid), stop)?;
    let read = if level == 0 { String::new() } else { format!(", read from the cube's mip level {level}, rendered by the Box filter") };
    let turn = if yaw == 0.0 { String::new() } else { format!(", the image turned {yaw} degrees about the vertical axis, +X toward -Z") };
    let outside = if projection == Projection::AngularMap { "; the texels outside the disc are transparent black" } else { "" };
    said.verbose.push(format!(
        "the cube was resampled on the GPU into a {width}x{height} {} in {}, each texel the mean of {grid} by {grid} samples over it, each weighed by the solid angle it covers{read}{turn}{outside}",
        projection.name(),
        format_name(format)
    ));
    image_io::read_file(core, &made, false, 1, stop)
}

/// Refuses a -Yaw that is not a finite number of degrees.
fn check_yaw(yaw: Option<f64>) -> PsResult<f64> {
    match yaw {
        None => Ok(0.0),
        Some(y) if y.is_finite() => Ok(y),
        Some(y) => Err(invalid("WgpuCubeMap", format!("-Yaw is a finite number of degrees, not {y}"))),
    }
}

/// Refuses -Yaw and -FaceSize for a layout of cells, whose faces are cut
/// and laid out as they are.
fn no_resampling(layout: CubeLayout, yaw: Option<f64>, face_size: Option<u32>) -> PsResult<()> {
    if yaw.is_none() && face_size.is_none() {
        return Ok(());
    }
    Err(invalid(
        "WgpuCubeMap",
        format!(
            "-Yaw and -FaceSize apply to a panorama, an angular map or an octahedral map, which are resampled; a {layout:?}'s faces are its cells as they are, and Resize-WgpuTexture changes their size"
        ),
    ))
}

/// The faces' size -FaceSize gives, or `default`; 0 is refused.
fn face_size_of(face_size: Option<u32>, default: u32) -> PsResult<u32> {
    match face_size {
        Some(0) => Err(invalid("WgpuCubeMap", "-FaceSize is the faces' size in texels, 1 or more")),
        Some(size) => Ok(size),
        None => Ok(default),
    }
}

/// A cube made from `file`, level 0 of one 2D image holding its faces in
/// the shape `layout` names or its aspect ratio gives. A layout's faces are
/// cut from its cells in the file's format, decoded first where a face
/// does not cover whole blocks or -Z is turned. A projection is resampled
/// on the GPU, turned by `yaw` degrees, into faces `face_size` texels
/// across, or the size its width gives, in the format the passes render
/// in.
pub(crate) fn cube_of_image(
    core: &CoreRef,
    file: TextureFile,
    layout: Option<CubeLayout>,
    yaw: Option<f64>,
    face_size: Option<u32>,
    stop: Option<&dyn Interrupt>,
    said: &mut Said,
) -> PsResult<TextureFile> {
    if file.dimension != wgpu::TextureDimension::D2 || file.layers != 1 {
        let shape = if file.dimension == wgpu::TextureDimension::D2 { format!("{} layers", file.layers) } else { format!("{:?}", file.dimension) };
        return Err(invalid("WgpuCubeMap", format!("a cube is made from one 2D image with one layer, or from six textures, its faces; this texture has {shape}")));
    }
    match shape_of(file.width, file.height, layout)? {
        Shape::Layout(layout, cells) => {
            no_resampling(layout, yaw, face_size)?;
            let size = file.width / cells.columns;
            let (bw, bh) = file.format.block_dimensions();
            let whole = size.is_multiple_of(bw) && size.is_multiple_of(bh) && (!cells.turned || (bw, bh) == (1, 1));
            let file = if whole {
                file
            } else {
                let from = file.format;
                let decoded = texfile::uncompressed(file, said)?;
                said.verbose.push(format!("the {} texels were decoded on the CPU into {}, as the faces do not cover its blocks whole", format_name(from), format_name(decoded.format)));
                decoded
            };
            let faces = cut(&file.levels[0], file.width, file.format, &cells, size)?;
            said.verbose.push(format!("the faces were cut from the {layout:?}'s cells as they are, {size} texels across"));
            Ok(TextureFile { dimension: wgpu::TextureDimension::D2, width: size, height: size, depth: 1, layers: 6, cube: true, format: file.format, levels: vec![faces] })
        }
        Shape::Projection(projection) => {
            let yaw = check_yaw(yaw)?;
            let size = face_size_of(face_size, projection.face_size(file.width))?;
            let file = TextureFile { levels: file.levels.into_iter().take(1).collect(), ..file };
            let base = texfile::uncompressed(file, said)?;
            resampled_cube(core, &base, projection, size, yaw, stop, said)
        }
    }
}

/// Level 0 of `cube`, a file of six square faces with one level, written
/// into one 2D image in `shape`: laid out in a layout's cells as they are,
/// the cells no face takes transparent black, in the faces' format or the
/// one they decode into; or resampled into a projection whose faces are
/// `face_size` texels across, the cube's size when `None`, turned by `yaw`
/// degrees, in the format the passes render in.
pub(crate) fn image_of_cube(
    core: &CoreRef,
    cube: TextureFile,
    shape: Shape,
    yaw: Option<f64>,
    face_size: Option<u32>,
    stop: Option<&dyn Interrupt>,
    said: &mut Said,
) -> PsResult<TextureFile> {
    let cube = texfile::uncompressed(cube, said)?;
    match shape {
        Shape::Layout(layout, cells) => {
            no_resampling(layout, yaw, face_size)?;
            let size = cube.width;
            let image = paste(&cube.levels[0], cube.format, &cells, size)?;
            said.verbose.push(format!("the faces were laid out in the {layout:?}'s cells as they are, {size} texels across, the cells no face takes transparent black"));
            Ok(TextureFile {
                dimension: wgpu::TextureDimension::D2,
                width: cells.columns * size,
                height: cells.rows * size,
                depth: 1,
                layers: 1,
                cube: false,
                format: cube.format,
                levels: vec![image],
            })
        }
        Shape::Projection(projection) => {
            let yaw = check_yaw(yaw)?;
            let size = face_size_of(face_size, cube.width)?;
            resampled_image(core, &cube, projection, size, yaw, stop, said)
        }
    }
}

/// The six square layers of `texture`, a cube's faces, or a refusal naming
/// what it holds instead.
pub(crate) fn check_faces(texture: &wgpu::Texture) -> PsResult<()> {
    let layers = texture.depth_or_array_layers();
    if texture.dimension() == wgpu::TextureDimension::D2 && layers == 6 && texture.width() == texture.height() {
        return Ok(());
    }
    let holds = match (texture.dimension(), layers) {
        (wgpu::TextureDimension::D2, n) if n.is_multiple_of(6) && texture.width() == texture.height() => format!("{n} layers, {} cubes; a cube map is converted one cube, six layers, at a time", n / 6),
        (wgpu::TextureDimension::D2, 1) => format!("one {}x{} layer", texture.width(), texture.height()),
        (wgpu::TextureDimension::D2, n) => format!("{n} layers of {}x{}", texture.width(), texture.height()),
        (dimension, _) => format!("{dimension:?} texels"),
    };
    Err(invalid("WgpuCubeMap", format!("a cube map is six square layers, its faces, and this texture holds {holds}")))
}

/// The cube a lighting map is made from, level 0 of its six faces:
/// `source` itself when it is six square layers, and otherwise the cube
/// made from the one image it holds, as ConvertTo-WgpuCubeMap makes it with
/// `layout` and `yaw`.
fn environment(core: &CoreRef, source: &Source, layout: Option<CubeLayout>, yaw: Option<f64>, stop: Option<&dyn Interrupt>, said: &mut Said) -> PsResult<TextureFile> {
    let texture = &source.texture;
    if texture.depth_or_array_layers() == 1 {
        let file = image_io::read_file(core, texture, false, 1, stop)?;
        return cube_of_image(core, file, layout, yaw, None, stop, said);
    }
    check_faces(texture)?;
    if layout.is_some() || yaw.is_some() {
        return Err(invalid("WgpuCubeMap", "-Layout and -Yaw read a cube from one image, and this texture is a cube's six faces already"));
    }
    image_io::read_file(core, texture, true, 1, stop)
}

/// The largest value a half float holds.
const HALF_MAX: f32 = 65504.0;

/// Whether `target` holds half floats: R16Float, Rg16Float, Rgba16Float,
/// BC6H and Basis Universal's HDR formats.
fn holds_half_floats(target: Target) -> bool {
    match target {
        Target::Gpu(format) => matches!(format, F::R16Float | F::Rg16Float | F::Rgba16Float | F::Bc6hRgbUfloat | F::Bc6hRgbFloat),
        Target::Basis(basis) => basis.hdr,
    }
}

/// `file` in `target`: kept when it is in that format already, packed on
/// the CPU from one of the float formats the passes render in, and
/// otherwise converted as Import-WgpuImage's -Format converts. A value
/// past the largest a half float holds is clamped to it first for a target
/// of half floats, which a warning counts.
pub(crate) fn stored(core: &CoreRef, file: TextureFile, target: Target, stop: Option<&dyn Interrupt>, said: &mut Said) -> PsResult<TextureFile> {
    if target == Target::Gpu(file.format) {
        return Ok(file);
    }
    let mut file = file;
    if file.format == F::Rgba32Float && holds_half_floats(target) {
        let mut clamped = 0usize;
        for level in &mut file.levels {
            for value in level.as_chunks_mut::<4>().0 {
                let v = f32::from_le_bytes(*value);
                if v.abs() > HALF_MAX {
                    *value = HALF_MAX.copysign(v).to_le_bytes();
                    clamped += 1;
                }
            }
        }
        if clamped > 0 {
            let name = match target {
                Target::Gpu(format) => format_name(format),
                Target::Basis(basis) => basis.name.to_string(),
            };
            said.warnings.push(format!("{clamped} values past 65504, the largest {name} holds, were clamped to it; -Format Rgba32Float keeps them"));
        }
    }
    match target {
        Target::Gpu(format) if matches!(file.format, F::Rgba32Float | F::Rgba16Float) && !format.is_compressed() => {
            image_io::holdable(core, format)?;
            let levels = file.levels.iter().map(|level| texel::pack(&texel::colors(level, file.format)?, format)).collect::<PsResult<Vec<_>>>()?;
            said.verbose.push(format!("the texels were packed into {} on the CPU", format_name(format)));
            Ok(TextureFile { format, levels, ..file })
        }
        other => image_io::into_target(core, file, other, stop, said),
    }
}

/// The format a call's -Format names, or `default` when it names none.
fn target_of(format: Option<&str>, default: F) -> PsResult<Target> {
    match format {
        Some(name) => parse_target(name),
        None => Ok(Target::Gpu(default)),
    }
}

/// A texture of `file` on `core`'s device, labeled `label`, its Notes what
/// making it involved.
fn finished(core: &CoreRef, file: TextureFile, label: String, said: &mut Said) -> PsResult<Texture> {
    let file = texfile::decode_for(file, core.features, said)?;
    let notes = said.verbose.iter().chain(&said.warnings).cloned().collect();
    image_io::upload(core, &file, None, label, notes)
}

/// ConvertTo-WgpuCubeMap and ToCubeMap() from one image: the cube made
/// from `source`, as `cube_of_image` makes it, in the format `format`
/// names, or the source's.
#[allow(clippy::too_many_arguments)]
pub(crate) fn cube_from_image(
    core: &CoreRef,
    source: &Source,
    layout: Option<CubeLayout>,
    yaw: Option<f64>,
    face_size: Option<u32>,
    format: Option<&str>,
    stop: Option<&dyn Interrupt>,
    said: &mut Said,
) -> PsResult<Texture> {
    let texture = &source.texture;
    let target = target_of(format, texture.format())?;
    let file = image_io::read_file(core, texture, false, 1, stop)?;
    let cube = cube_of_image(core, file, layout, yaw, face_size, stop, said)?;
    if texture.mip_level_count() > 1 {
        said.verbose.push("the cube has mip level 0 alone, as the image's own levels blend each face with its neighbors at the edges: GenerateMips(0) renders the cube's".to_string());
    }
    let cube = stored(core, cube, target, stop, said)?;
    finished(core, cube, source.label.clone(), said)
}

/// ConvertTo-WgpuCubeMap and CubeFromFaces() from six textures, the faces
/// in order, each with the device it is on, square 2D textures of one size
/// and format: the cube of their texels and the mip levels all six have,
/// in the format `format` names, or theirs, on the first face's device.
/// `names` says how a refusal names each face.
pub(crate) fn cube_from_faces(faces: &[(CoreRef, Source)], names: &[&str; 6], format: Option<&str>, stop: Option<&dyn Interrupt>, said: &mut Said) -> PsResult<Texture> {
    let [(core, first), ..] = faces else {
        return Err(invalid("WgpuCubeMap", "a cube is made from six faces, and none was given"));
    };
    if faces.len() != 6 {
        return Err(invalid("WgpuCubeMap", format!("a cube is made from six faces, and {} were given", faces.len())));
    }
    let lead = &first.texture;
    let shape = |t: &wgpu::Texture| format!("{}x{} {}", t.width(), t.height(), format_name(t.format()));
    for ((_, face), name) in faces.iter().zip(names) {
        let t = &face.texture;
        if t.dimension() != wgpu::TextureDimension::D2 || t.depth_or_array_layers() != 1 {
            return Err(invalid("WgpuCubeMap", format!("{name} is not a 2D texture with one layer, as a face is")));
        }
        if t.width() != t.height() {
            return Err(invalid("WgpuCubeMap", format!("{name} is {}x{}, and a cube's faces are square", t.width(), t.height())));
        }
        if (t.width(), t.format()) != (lead.width(), lead.format()) {
            return Err(invalid(
                "WgpuCubeMap",
                format!("{name} is {} where {} is {}: a cube's faces share one size and format, and Resize-WgpuTexture or Convert-WgpuTexture -Format brings a face in line", shape(t), names[0], shape(lead)),
            ));
        }
    }
    let target = target_of(format, lead.format())?;
    let counts: Vec<u32> = faces.iter().map(|(_, face)| face.texture.mip_level_count()).collect();
    let (fewest, most) = (counts.iter().copied().min().unwrap_or(1), counts.iter().copied().max().unwrap_or(1));
    if fewest != most {
        said.verbose.push(format!("the faces have from {fewest} to {most} mip levels, and the cube keeps the {fewest} all six have"));
    }
    let mut levels = vec![Vec::new(); fewest as usize];
    for (face_core, face) in faces {
        let file = image_io::read_file(face_core, &face.texture, false, fewest, stop)?;
        for (level, texels) in file.levels.into_iter().enumerate() {
            levels[level].extend(texels);
        }
    }
    let size = lead.width();
    let cube = TextureFile { dimension: wgpu::TextureDimension::D2, width: size, height: size, depth: 1, layers: 6, cube: true, format: lead.format(), levels };
    let cube = stored(core, cube, target, stop, said)?;
    finished(core, cube, first.label.clone(), said)
}

/// ConvertFrom-WgpuCubeMap, ToLayout() and ToPanorama(): mip level `level`
/// of the cube `source`, level 0 when `None`, written into one image in
/// `shape`, as `image_of_cube` writes it, in the format `format` names, or
/// the cube's.
#[allow(clippy::too_many_arguments)]
pub(crate) fn image_from_cube(
    core: &CoreRef,
    source: &Source,
    shape: Shape,
    yaw: Option<f64>,
    face_size: Option<u32>,
    level: Option<u32>,
    format: Option<&str>,
    stop: Option<&dyn Interrupt>,
    said: &mut Said,
) -> PsResult<Texture> {
    let texture = &source.texture;
    check_faces(texture)?;
    let level = level.unwrap_or(0);
    if level >= texture.mip_level_count() {
        return Err(invalid("WgpuMipLevels", format!("the cube has mip levels 0 to {}, not {level}", texture.mip_level_count() - 1)));
    }
    let target = target_of(format, texture.format())?;
    let cube = image_io::read_level_file(core, texture, true, level, stop)?;
    let image = image_of_cube(core, cube, shape, yaw, face_size, stop, said)?;
    let image = stored(core, image, target, stop, said)?;
    finished(core, image, source.label.clone(), said)
}

/// The texel count a side the irradiance sums read the environment at: its
/// first mip level this size or smaller.
const IRRADIANCE_SOURCE: u32 = 64;

/// The irradiance cube of `env`, six square faces with one level: faces
/// `size` texels across, each texel the cosine-weighed mean of every texel
/// of the environment brought to 64 texels a face or fewer, in the format
/// the passes render in; and with `harmonics`, its spherical harmonics.
fn irradiance(core: &CoreRef, env: TextureFile, size: u32, harmonics: bool, stop: Option<&dyn Interrupt>, said: &mut Said) -> PsResult<(TextureFile, Option<Harmonics>)> {
    let base = texfile::uncompressed(env, said)?;
    let render = readable(base.format, "an irradiance map")?;
    let mut level = 0;
    while (base.width >> level) > IRRADIANCE_SOURCE {
        level += 1;
    }
    let read = (base.width >> level).max(1);
    let work = convert::float_levels(core, &base, render, level + 1, 1, MipOptions::default(), stop)?;
    let format = rendered(core);
    let weighed = target(core, read, read, 6, 1, format)?;
    let made = target(core, size, size, 6, 1, format)?;
    let shader = format!("{COMMON}{FROM_CUBE}");
    let array = wgpu::TextureViewDimension::D2Array;
    let (weigh, sum) = (pass(core, &shader, "weigh", Some(array), format)?, pass(core, &shader, "irradiance", Some(array), format)?);
    let (env_view, weighed_view) = (read_view(core, &work, array, 6)?, read_view(core, &weighed, array, 6)?);
    for face in 0..6 {
        draw_into(core, &weigh, Some(&env_view), &weighed, 0, face, &Params { face, lod: level, ..Params::default() }, 1, stop)?;
    }
    for face in 0..6 {
        let params = Params { face, width: size, height: size, ..Params::default() };
        draw_into(core, &sum, Some(&weighed_view), &made, 0, face, &params, 6 * u64::from(read) * u64::from(read), stop)?;
    }
    let from = if level == 0 { String::new() } else { format!(" brought to {read} texels a face by the Box filter") };
    said.verbose.push(format!(
        "the irradiance was summed on the GPU into faces {size} texels across in {}: each texel the mean of every texel of the environment{from}, each weighed by the solid angle it covers and the cosine of its angle from the texel's direction",
        format_name(format)
    ));
    let harmonics = if harmonics {
        let texels = texel::colors(&image_io::read_level(core, &work, level, stop)?, render)?;
        said.verbose.push(format!("the spherical harmonics were summed on the CPU from the environment's {read} texels a face"));
        Some(spherical_harmonics(&texels, read))
    } else {
        None
    };
    Ok((image_io::read_file(core, &made, true, 1, stop)?, harmonics))
}

/// The nine basis functions at the unit direction d, as cmgen evaluates
/// them.
fn basis(d: [f64; 3]) -> [f64; 9] {
    let [x, y, z] = d;
    [
        0.282_094_791_773_878_14,
        -0.488_602_511_902_919_9 * y,
        0.488_602_511_902_919_9 * z,
        -0.488_602_511_902_919_9 * x,
        1.092_548_430_592_079_2 * x * y,
        -1.092_548_430_592_079_2 * y * z,
        0.315_391_565_252_520_05 * (3.0 * z * z - 1.0),
        -1.092_548_430_592_079_2 * x * z,
        0.546_274_215_296_039_6 * (x * x - y * y),
    ]
}

/// The irradiance of `texels`, six faces `size` texels across in face
/// order, as spherical harmonics: each color times each basis function at
/// its texel's direction times the solid angle the texel covers, summed,
/// then scaled by the clamped cosine's coefficient for its band.
fn spherical_harmonics(texels: &[[f32; 4]], size: u32) -> Harmonics {
    let cosine = [PI, 2.0 * PI / 3.0, 2.0 * PI / 3.0, 2.0 * PI / 3.0, PI / 4.0, PI / 4.0, PI / 4.0, PI / 4.0, PI / 4.0];
    let mut sums = [[0.0f64; 3]; 9];
    let at = |i: u32| (2.0 * f64::from(i) + 1.0) / f64::from(size) - 1.0;
    for face in 0..6usize {
        for y in 0..size {
            for x in 0..size {
                let functions = basis(normalized(face_direction(face, at(x), at(y))));
                let omega = texel_solid_angle(size, x, y);
                let color = texels[face * (size * size) as usize + (y * size + x) as usize];
                for (sum, b) in sums.iter_mut().zip(functions) {
                    for (channel, value) in sum.iter_mut().zip(color) {
                        *channel += f64::from(value) * b * omega;
                    }
                }
            }
        }
    }
    let channel = |c: usize| sums.iter().zip(cosine).map(|(sum, a)| sum[c] * a).collect();
    Harmonics { red: channel(0), green: channel(1), blue: channel(2) }
}

/// The prefiltered specular cube of `env`, six square faces with one
/// level: `levels` mip levels, 0 for the full chain, or as many as
/// `mapping` gives when `None`, each the environment as the GGX lobe of
/// the roughness `mapping` gives the level reflects it, by `samples`
/// samples a texel, in the format the passes render in.
fn specular(core: &CoreRef, env: TextureFile, mapping: RoughnessMapping, samples: u32, levels: Option<u32>, stop: Option<&dyn Interrupt>, said: &mut Said) -> PsResult<TextureFile> {
    let base = texfile::uncompressed(env, said)?;
    let render = readable(base.format, "a specular map")?;
    let size = base.width;
    let full = 32 - size.leading_zeros();
    let count = match levels {
        Some(0) => full,
        Some(n) if n <= full => n,
        Some(n) => return Err(invalid("WgpuMipLevels", format!("faces {size} texels across have {full} mip levels down to 1x1, not {n}"))),
        None => mapping.levels(size),
    };
    let work = convert::float_levels(core, &base, render, full, 1, MipOptions::default(), stop)?;
    let format = rendered(core);
    let made = target(core, size, size, 6, count, format)?;
    let array = wgpu::TextureViewDimension::D2Array;
    let prefilter = pass(core, &format!("{COMMON}{FROM_CUBE}"), "specular", Some(array), format)?;
    let input = read_view(core, &work, array, 6)?;
    let code = match mapping {
        RoughnessMapping::Linear => 0,
        RoughnessMapping::Quadratic => 1,
    };
    for level in 0..count {
        let roughness = mapping.roughness(level, count) as f32;
        let side = (size >> level).max(1);
        for face in 0..6 {
            let params = Params { face, width: side, height: side, roughness, samples, env_size: size, mapping: code, ..Params::default() };
            let cost = if roughness > 0.0 { 8 * u64::from(samples) } else { 1 };
            draw_into(core, &prefilter, Some(&input), &made, level, face, &params, cost, stop)?;
        }
    }
    let how = match mapping {
        RoughnessMapping::Linear => "perceptual roughness level / (levels - 1), each sample read at the mip level Khronos's sampler computes",
        RoughnessMapping::Quadratic => "perceptual roughness 1 - sqrt(1 - level / (levels - 1)), each sample read at the mip level cmgen computes",
    };
    said.verbose.push(format!(
        "the specular map was prefiltered on the GPU into {count} mip levels from {size} texels a face in {}: level 0 the environment itself, and each other level the GGX lobe of {how}, by {samples} samples a texel",
        format_name(format)
    ));
    image_io::read_file(core, &made, true, count, stop)
}

/// The BRDF table, `size` texels a side, by `samples` samples a texel, in
/// the format the passes render in.
fn brdf_table(core: &CoreRef, size: u32, samples: u32, stop: Option<&dyn Interrupt>, said: &mut Said) -> PsResult<TextureFile> {
    let format = rendered(core);
    let made = target(core, size, size, 1, 1, format)?;
    let integrate = pass(core, &format!("{COMMON}{BRDF}"), "brdf", None, format)?;
    let params = Params { width: size, height: size, samples, ..Params::default() };
    draw_into(core, &integrate, None, &made, 0, 0, &params, u64::from(samples), stop)?;
    said.verbose.push(format!(
        "the BRDF table was integrated on the GPU in {}, {size} texels a side by {samples} samples a texel, with Smith's height-correlated GGX visibility",
        format_name(format)
    ));
    image_io::read_file(core, &made, false, 1, stop)
}

/// Refuses a sample count of 0.
fn samples_of(samples: Option<u32>) -> PsResult<u32> {
    match samples {
        Some(0) => Err(invalid("WgpuSamples", "-Samples is how many samples each texel takes, 1 or more")),
        Some(n) => Ok(n),
        None => Ok(1024),
    }
}

/// ConvertTo-WgpuIrradianceMap and ToIrradianceMap(): the irradiance cube
/// of the environment `source`, faces `face_size` texels across, 32 when
/// `None`, in the format `format` names, or Rgba16Float, with its
/// spherical harmonics as its Harmonics where `harmonics` asks.
#[allow(clippy::too_many_arguments)]
pub(crate) fn irradiance_map(
    core: &CoreRef,
    source: &Source,
    layout: Option<CubeLayout>,
    yaw: Option<f64>,
    face_size: Option<u32>,
    harmonics: bool,
    format: Option<&str>,
    stop: Option<&dyn Interrupt>,
    said: &mut Said,
) -> PsResult<Texture> {
    let size = face_size_of(face_size, 32)?;
    let target = target_of(format, F::Rgba16Float)?;
    let env = environment(core, source, layout, yaw, stop, said)?;
    let (cube, harmonics) = irradiance(core, env, size, harmonics, stop, said)?;
    let cube = stored(core, cube, target, stop, said)?;
    let mut made = finished(core, cube, source.label.clone(), said)?;
    made.harmonics = harmonics;
    Ok(made)
}

/// ConvertTo-WgpuSpecularMap and ToSpecularMap(): the prefiltered specular
/// cube of the environment `source`, as `specular` makes it, in the format
/// `format` names, or Rgba16Float.
#[allow(clippy::too_many_arguments)]
pub(crate) fn specular_map(
    core: &CoreRef,
    source: &Source,
    layout: Option<CubeLayout>,
    yaw: Option<f64>,
    mapping: Option<RoughnessMapping>,
    samples: Option<u32>,
    levels: Option<u32>,
    format: Option<&str>,
    stop: Option<&dyn Interrupt>,
    said: &mut Said,
) -> PsResult<Texture> {
    let samples = samples_of(samples)?;
    let target = target_of(format, F::Rgba16Float)?;
    let env = environment(core, source, layout, yaw, stop, said)?;
    let cube = specular(core, env, mapping.unwrap_or(RoughnessMapping::Linear), samples, levels, stop, said)?;
    let cube = stored(core, cube, target, stop, said)?;
    finished(core, cube, source.label.clone(), said)
}

/// New-WgpuBrdfTable and NewBrdfTable(): the BRDF table, `size` texels a
/// side, 128 when `None`, in the format `format` names, or Rg16Float.
pub(crate) fn new_brdf_table(core: &CoreRef, size: Option<u32>, samples: Option<u32>, format: Option<&str>, stop: Option<&dyn Interrupt>, said: &mut Said) -> PsResult<Texture> {
    let size = match size {
        Some(0) => return Err(invalid("WgpuBrdfTable", "-Size is the table's side in texels, 1 or more")),
        Some(n) => n,
        None => 128,
    };
    let samples = samples_of(samples)?;
    let target = target_of(format, F::Rg16Float)?;
    let table = brdf_table(core, size, samples, stop, said)?;
    let table = stored(core, table, target, stop, said)?;
    finished(core, table, format!("BRDF table {size}x{size}"), said)
}

/// The texture a proxy holds, with its device.
fn held(texture: &PsProxy<Texture>) -> PsResult<(CoreRef, Source)> {
    texture.with(|t| (t.core.clone(), t.source()))
}

/// The textures proxies hold, each with its device.
pub(crate) fn faces_held(faces: &[PsProxy<Texture>]) -> PsResult<Vec<(CoreRef, Source)>> {
    faces.iter().map(held).collect()
}

/// The device a method's `device` names, or the default device.
pub(crate) fn device_core(device: Option<&PsProxy<Device>>) -> PsResult<CoreRef> {
    match device {
        Some(d) => d.with(|d| d.core.clone()),
        None => default_core(None),
    }
}

/// A panorama, the image ToPanorama() makes.
pub(crate) const PANORAMA: Shape = Shape::Projection(Projection::Panorama);

/// How a refusal names each face given in one array.
pub(crate) const FACE_NAMES: [&str; 6] = ["the +X face", "the -X face", "the +Y face", "the -Y face", "the +Z face", "the -Z face"];

/// Makes a cube map: from six textures, its faces, or from one image that
/// holds every face, in a layout of square cells or a projection of every
/// direction.
///
/// Six textures are the faces in the order +X, -X, +Y, -Y, +Z, -Z, given as
/// one array or by -PositiveX, -NegativeX, -PositiveY, -NegativeY,
/// -PositiveZ and -NegativeZ: square 2D textures of one size and format,
/// whose texels, and the mip levels all six have, are kept as they are.
///
/// One image is read by its shape: a 2:1 image is a panorama, 4:3 a
/// horizontal cross, 3:4 a vertical cross, and 6:1 and 1:6 strips of the
/// faces in order. -Layout names the rest: a HorizontalTee, a
/// VerticalCrossTurned, Godot's Grid2x3 and Grid3x2, and the square
/// AngularMap and Octahedral map. A layout's faces are cut from its cells
/// as they are. A panorama, an angular map or an octahedral map is
/// resampled on the GPU: each texel of the cube is the mean of a grid of
/// samples over it, each weighed by the solid angle it covers. A
/// panorama's center faces +X and its left quarter -Z, as three.js and
/// Khronos's glTF IBL sampler read it; -Yaw turns the image about the
/// vertical axis first, +X toward -Z. The faces are a quarter of a
/// panorama's or an angular map's width, and half an octahedral map's,
/// unless -FaceSize says.
///
/// The cube keeps the source's format, or takes the -Format named: a color
/// format, a BC format, encoded on the GPU, or a Basis Universal format. A
/// cube made from one image has mip level 0 alone, which GenerateMips(0)
/// fills out. What making it involved goes to the verbose stream and its
/// Notes.
///
/// # Examples
/// ConvertTo-WgpuCubeMap $sky
/// $sky | ConvertTo-WgpuCubeMap -FaceSize 512 -Format Rgba16Float
/// ConvertTo-WgpuCubeMap $cross -Layout HorizontalTee
/// $probe | ConvertTo-WgpuCubeMap -Layout AngularMap -Yaw 90
/// ConvertTo-WgpuCubeMap $px, $nx, $py, $ny, $pz, $nz
/// ConvertTo-WgpuCubeMap -PositiveX $px -NegativeX $nx -PositiveY $py -NegativeY $ny -PositiveZ $pz -NegativeZ $nz
#[cmdlet(verb = "ConvertTo", noun = "WgpuCubeMap", alias = ["ConvertTo-GpuCubeMap"], default_parameter_set = "Texture", output = ["Wgpu.Texture"])]
#[derive(Default)]
pub struct ConvertToWgpuCubeMap {
    /// One image holding the faces, or six textures, the faces in the
    /// order +X, -X, +Y, -Y, +Z, -Z.
    #[param(mandatory, position = 0, value_from_pipeline, set = "Texture")]
    pub texture: Option<Vec<PsProxy<Texture>>>,
    /// The +X face.
    #[param(mandatory, set = "Faces")]
    pub positive_x: Option<PsProxy<Texture>>,
    /// The -X face.
    #[param(mandatory, set = "Faces")]
    pub negative_x: Option<PsProxy<Texture>>,
    /// The +Y face.
    #[param(mandatory, set = "Faces")]
    pub positive_y: Option<PsProxy<Texture>>,
    /// The -Y face.
    #[param(mandatory, set = "Faces")]
    pub negative_y: Option<PsProxy<Texture>>,
    /// The +Z face.
    #[param(mandatory, set = "Faces")]
    pub positive_z: Option<PsProxy<Texture>>,
    /// The -Z face.
    #[param(mandatory, set = "Faces")]
    pub negative_z: Option<PsProxy<Texture>>,
    /// How the image holds the faces: needed for a HorizontalTee, a
    /// VerticalCrossTurned, a Grid2x3, a Grid3x2, an AngularMap or an
    /// Octahedral map, which the image's shape does not tell apart.
    #[param(set = "Texture")]
    pub layout: Option<CubeLayout>,
    /// Turns a panorama, an angular map or an octahedral map about the
    /// vertical axis by this many degrees, +X toward -Z, before it is read.
    #[param(set = "Texture")]
    pub yaw: Option<f64>,
    /// The faces' size in texels for a cube resampled from a panorama, an
    /// angular map or an octahedral map; a quarter of a panorama's or an
    /// angular map's width, and half an octahedral map's, when omitted.
    #[param(set = "Texture")]
    pub face_size: Option<u32>,
    /// The cube's format: a color format, a BC format, encoded on the GPU,
    /// or a Basis Universal format; the source's when omitted.
    #[param]
    pub format: Option<String>,
}

impl Cmdlet for ConvertToWgpuCubeMap {
    fn process(&mut self, ps: &Pipeline<'_>) -> PsResult<()> {
        let mut said = Said::default();
        let made = match &self.texture {
            Some(textures) if textures.len() == 1 => {
                let (core, source) = held(&textures[0])?;
                cube_from_image(&core, &source, self.layout, self.yaw, self.face_size, self.format.as_deref(), Some(ps), &mut said)
            }
            Some(textures) if textures.len() == 6 => {
                if self.layout.is_some() || self.yaw.is_some() || self.face_size.is_some() {
                    return Err(invalid("WgpuCubeMap", "-Layout, -Yaw and -FaceSize read a cube from one image; six textures are its faces as they are"));
                }
                let faces = textures.iter().map(held).collect::<PsResult<Vec<_>>>()?;
                cube_from_faces(&faces, &FACE_NAMES, self.format.as_deref(), Some(ps), &mut said)
            }
            Some(textures) => return Err(invalid("WgpuCubeMap", format!("a cube is made from one image holding its faces or from six textures, its faces, and {} were given", textures.len()))),
            None => {
                let named = [&self.positive_x, &self.negative_x, &self.positive_y, &self.negative_y, &self.positive_z, &self.negative_z];
                let mut faces = Vec::with_capacity(6);
                for (face, name) in named.into_iter().zip(["-PositiveX", "-NegativeX", "-PositiveY", "-NegativeY", "-PositiveZ", "-NegativeZ"]) {
                    match face {
                        Some(texture) => faces.push(held(texture)?),
                        None => return Err(invalid("WgpuCubeMap", format!("{name} was not given: a cube takes all six faces"))),
                    }
                }
                cube_from_faces(&faces, &["-PositiveX", "-NegativeX", "-PositiveY", "-NegativeY", "-PositiveZ", "-NegativeZ"], self.format.as_deref(), Some(ps), &mut said)
            }
        };
        tell(ps, &said)?;
        ps.write(made?)
    }
}

/// Writes a cube map into one image: its faces in a layout of square
/// cells, or a projection of every direction.
///
/// -Layout names the image: a HorizontalCross, the default, a
/// VerticalCross, a VerticalCrossTurned, a HorizontalTee, a
/// HorizontalStrip, a VerticalStrip, a Grid2x3, a Grid3x2, an AngularMap or
/// an Octahedral map; -Panorama makes a panorama instead. A layout's cells
/// take the faces as they are, and the cells no face takes are transparent
/// black. A panorama, an angular map or an octahedral map is resampled on
/// the GPU, each texel the mean of a grid of samples over it, each weighed
/// by the solid angle it covers; an angular map's texels outside its disc
/// are transparent black. A panorama is 4 faces wide and 2 high, an
/// angular map 4 faces across and an octahedral map 2, the faces the
/// cube's size or -FaceSize, and -Yaw turns the image about the vertical
/// axis as ConvertTo-WgpuCubeMap reads it, so the same -Yaw both ways
/// gives the image back.
///
/// The image comes from mip level 0, or -MipLevel, and keeps the cube's
/// format, or takes the -Format named: a color format, a BC format, encoded
/// on the GPU, or a Basis Universal format. What making it involved goes
/// to the verbose stream and its Notes.
///
/// # Examples
/// ConvertFrom-WgpuCubeMap $cube
/// $cube | ConvertFrom-WgpuCubeMap -Panorama -FaceSize 256
/// $cube | ConvertFrom-WgpuCubeMap -Layout VerticalCrossTurned
/// $specular | ConvertFrom-WgpuCubeMap -Layout Octahedral -MipLevel 2
#[cmdlet(verb = "ConvertFrom", noun = "WgpuCubeMap", alias = ["ConvertFrom-GpuCubeMap"], output = ["Wgpu.Texture"])]
#[derive(Default)]
pub struct ConvertFromWgpuCubeMap {
    /// The cube map: six square layers, its faces.
    #[param(mandatory, position = 0, value_from_pipeline)]
    pub texture: Option<PsProxy<Texture>>,
    /// The layout or projection of the image; HorizontalCross when
    /// omitted.
    #[param]
    pub layout: Option<CubeLayout>,
    /// Makes a panorama, center +X and left quarter -Z.
    #[param]
    pub panorama: bool,
    /// Turns a panorama, an angular map or an octahedral map about the
    /// vertical axis by this many degrees, +X toward -Z, as
    /// ConvertTo-WgpuCubeMap reads it.
    #[param]
    pub yaw: Option<f64>,
    /// The size in texels of the faces a panorama, an angular map or an
    /// octahedral map is made for; the cube's when omitted.
    #[param]
    pub face_size: Option<u32>,
    /// The cube's mip level the image comes from; 0 when omitted.
    #[param]
    pub mip_level: Option<u32>,
    /// The image's format: a color format, a BC format, encoded on the
    /// GPU, or a Basis Universal format; the cube's when omitted.
    #[param]
    pub format: Option<String>,
}

impl Cmdlet for ConvertFromWgpuCubeMap {
    fn process(&mut self, ps: &Pipeline<'_>) -> PsResult<()> {
        let Some(texture) = &self.texture else {
            return Err(invalid("WgpuNoTexture", "no texture was given"));
        };
        let shape = match (self.layout, self.panorama) {
            (Some(_), true) => return Err(invalid("WgpuCubeMap", "-Layout and -Panorama each name the image; give one")),
            (Some(layout), false) => shape(layout),
            (None, true) => Shape::Projection(Projection::Panorama),
            (None, false) => shape(CubeLayout::HorizontalCross),
        };
        let (core, source) = held(texture)?;
        let mut said = Said::default();
        let made = image_from_cube(&core, &source, shape, self.yaw, self.face_size, self.mip_level, self.format.as_deref(), Some(ps), &mut said);
        tell(ps, &said)?;
        ps.write(made?)
    }
}

/// Makes the irradiance cube of an environment: each texel the light a
/// surface facing its direction receives from every direction, weighed by
/// the cosine of the angle between them and divided by pi, the color a
/// white diffuse surface reflects, which a shader multiplies by the
/// surface's albedo.
///
/// The environment is a cube map, or a panorama or layout, converted first
/// as ConvertTo-WgpuCubeMap converts it, with -Layout and -Yaw. Each texel
/// sums every texel of the environment, brought to 64 texels a face or
/// fewer by the Box filter, by the exact solid angle each covers, and
/// divides by the sum of the weights, so an environment of one color gives
/// that color. The cube is 32 texels a face, or -FaceSize, in Rgba16Float,
/// or the -Format named, with one mip level.
///
/// -Harmonics also gives the texture a Harmonics property: the irradiance
/// as the nine spherical harmonics of each color channel cmgen's
/// --sh-irradiance writes, which a shader can evaluate in place of the
/// cube. What making it involved goes to the verbose stream and its Notes.
///
/// # Examples
/// ConvertTo-WgpuIrradianceMap $cube
/// $sky | ConvertTo-WgpuIrradianceMap -FaceSize 16 -Harmonics
/// ConvertTo-WgpuIrradianceMap $cube -Format Bc6hRgbUfloat
#[cmdlet(verb = "ConvertTo", noun = "WgpuIrradianceMap", alias = ["ConvertTo-GpuIrradianceMap"], output = ["Wgpu.Texture"])]
#[derive(Default)]
pub struct ConvertToWgpuIrradianceMap {
    /// The environment: a cube map, or a panorama or layout.
    #[param(mandatory, position = 0, value_from_pipeline)]
    pub texture: Option<PsProxy<Texture>>,
    /// The faces' size in texels; 32 when omitted.
    #[param]
    pub face_size: Option<u32>,
    /// Gives the texture the irradiance's spherical harmonics too, as its
    /// Harmonics property.
    #[param]
    pub harmonics: bool,
    /// The format: a color format, a BC format, encoded on the GPU, or a
    /// Basis Universal format; Rgba16Float when omitted.
    #[param]
    pub format: Option<String>,
    /// How an environment given as one image holds the faces, as
    /// ConvertTo-WgpuCubeMap's -Layout says.
    #[param]
    pub layout: Option<CubeLayout>,
    /// Turns an environment given as a panorama, an angular map or an
    /// octahedral map about the vertical axis by this many degrees, +X
    /// toward -Z.
    #[param]
    pub yaw: Option<f64>,
}

impl Cmdlet for ConvertToWgpuIrradianceMap {
    fn process(&mut self, ps: &Pipeline<'_>) -> PsResult<()> {
        let Some(texture) = &self.texture else {
            return Err(invalid("WgpuNoTexture", "no texture was given"));
        };
        let (core, source) = held(texture)?;
        let mut said = Said::default();
        let made = irradiance_map(&core, &source, self.layout, self.yaw, self.face_size, self.harmonics, self.format.as_deref(), Some(ps), &mut said);
        tell(ps, &said)?;
        ps.write(made?)
    }
}

/// Makes the prefiltered specular cube of an environment, which split-sum
/// image-based lighting reads beside the BRDF table: each mip level the
/// environment as the GGX lobe of one roughness reflects it, the view
/// taken along each texel's direction.
///
/// -RoughnessMapping Linear, the default, gives level n of N perceptual
/// roughness n / (N - 1), down to 2x2, as Khronos's glTF IBL sampler
/// writes the levels and glTF viewers read them; Quadratic gives it
/// 1 - sqrt(1 - n / (N - 1)), down to 16x16, as Filament's cmgen writes
/// them. -MipLevels sets N, 0 for the full chain. Level 0, roughness 0, is
/// the environment itself, its faces' size. Each texel of the other levels
/// takes 1024 samples, or -Samples, of the GGX lobe at Hammersley's
/// points, each reading the environment's mip level whose texels cover
/// about the solid angle the sample covers, as Khronos's sampler computes
/// that level for Linear and cmgen for Quadratic.
///
/// The environment is a cube map, or a panorama or layout, converted first
/// as ConvertTo-WgpuCubeMap converts it, with -Layout and -Yaw. The cube is
/// Rgba16Float, or the -Format named. What making it involved goes to the
/// verbose stream and its Notes.
///
/// # Examples
/// ConvertTo-WgpuSpecularMap $cube
/// $sky | ConvertTo-WgpuSpecularMap -RoughnessMapping Quadratic
/// ConvertTo-WgpuSpecularMap $cube -Samples 4096 -Format Bc6hRgbUfloat
#[cmdlet(verb = "ConvertTo", noun = "WgpuSpecularMap", alias = ["ConvertTo-GpuSpecularMap"], output = ["Wgpu.Texture"])]
#[derive(Default)]
pub struct ConvertToWgpuSpecularMap {
    /// The environment: a cube map, or a panorama or layout.
    #[param(mandatory, position = 0, value_from_pipeline)]
    pub texture: Option<PsProxy<Texture>>,
    /// How the mip levels step through roughness: Linear, as Khronos's
    /// sampler, when omitted, or Quadratic, as cmgen.
    #[param]
    pub roughness_mapping: Option<RoughnessMapping>,
    /// Samples a texel; 1024 when omitted.
    #[param]
    pub samples: Option<u32>,
    /// Mip levels: down to 2x2 for Linear and to 16x16 for Quadratic when
    /// omitted, and 0 for the full chain down to 1x1.
    #[param]
    pub mip_levels: Option<u32>,
    /// The format: a color format, a BC format, encoded on the GPU, or a
    /// Basis Universal format; Rgba16Float when omitted.
    #[param]
    pub format: Option<String>,
    /// How an environment given as one image holds the faces, as
    /// ConvertTo-WgpuCubeMap's -Layout says.
    #[param]
    pub layout: Option<CubeLayout>,
    /// Turns an environment given as a panorama, an angular map or an
    /// octahedral map about the vertical axis by this many degrees, +X
    /// toward -Z.
    #[param]
    pub yaw: Option<f64>,
}

impl Cmdlet for ConvertToWgpuSpecularMap {
    fn process(&mut self, ps: &Pipeline<'_>) -> PsResult<()> {
        let Some(texture) = &self.texture else {
            return Err(invalid("WgpuNoTexture", "no texture was given"));
        };
        let (core, source) = held(texture)?;
        let mut said = Said::default();
        let made = specular_map(&core, &source, self.layout, self.yaw, self.roughness_mapping, self.samples, self.mip_levels, self.format.as_deref(), Some(ps), &mut said);
        tell(ps, &said)?;
        ps.write(made?)
    }
}

/// Makes the BRDF table split-sum image-based lighting reads beside the
/// specular map: for the cosine of the view's angle across and perceptual
/// roughness down, the part of the specular light F0 scales, in red, and
/// the part it adds, in green, with Smith's height-correlated GGX
/// visibility, as Khronos's glTF IBL sampler and Filament compute them.
///
/// Texel (x, y) holds cosine (x + 0.5) / size and roughness (y + 0.5) /
/// size, so a shader reads it at (NdotV, roughness) and lights a surface
/// with the specular map's color times F0 times red plus green. The table
/// is 128 texels a side, or -Size, from 1024 samples a texel, or -Samples,
/// in Rg16Float, or the -Format named.
///
/// # Examples
/// $brdf = New-WgpuBrdfTable
/// New-WgpuBrdfTable -Size 256 -Samples 4096 -Format Rg32Float
#[cmdlet(verb = "New", noun = "WgpuBrdfTable", alias = ["New-GpuBrdfTable"], output = ["Wgpu.Texture"])]
#[derive(Default)]
pub struct NewWgpuBrdfTable {
    /// The table's side in texels; 128 when omitted.
    #[param]
    pub size: Option<u32>,
    /// Samples a texel; 1024 when omitted.
    #[param]
    pub samples: Option<u32>,
    /// The format: a color format, a BC format, encoded on the GPU, or a
    /// Basis Universal format; Rg16Float when omitted.
    #[param]
    pub format: Option<String>,
    /// The device to make it on; the default device when omitted.
    #[param]
    pub device: Option<PsProxy<Device>>,
}

impl Cmdlet for NewWgpuBrdfTable {
    fn process(&mut self, ps: &Pipeline<'_>) -> PsResult<()> {
        let core = core_for(&self.device, ps)?;
        let mut said = Said::default();
        let made = new_brdf_table(&core, self.size, self.samples, self.format.as_deref(), Some(ps), &mut said);
        tell(ps, &said)?;
        ps.write(made?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cube_reference as reference;
    use crate::device::{self, AdapterQuery, InstanceOptions, OpenOptions};

    /// The device the tests run on, chosen as gpu::tests choose it: `None`,
    /// with the reason printed, only where this host has no adapter.
    fn core() -> Option<CoreRef> {
        crate::faults::install();
        let adapters = device::adapters(&InstanceOptions::default()).expect("the adapters");
        if adapters.is_empty() {
            eprintln!("skipped: this host has no GPU adapter");
            return None;
        }
        let name = match std::env::var("WGPU_PWSH_TEST_ADAPTER") {
            Ok(name) => Some(name),
            Err(std::env::VarError::NotPresent) => None,
            Err(e) => panic!("WGPU_PWSH_TEST_ADAPTER cannot be read: {e}"),
        };
        Some(device::open(&OpenOptions { adapter: AdapterQuery { name, ..Default::default() }, ..Default::default() }, None).expect("a device opens"))
    }

    /// A 2D file of one level, `layers` layers of `width` by `height` texels
    /// of `format`.
    fn file(format: F, width: u32, height: u32, layers: u32, texels: Vec<u8>) -> TextureFile {
        TextureFile { dimension: wgpu::TextureDimension::D2, width, height, depth: 1, layers, cube: layers == 6, format, levels: vec![texels] }
    }

    /// Colors as Rgba32Float texels.
    fn floats(colors: &[[f32; 4]]) -> Vec<u8> {
        colors.iter().flatten().flat_map(|v| v.to_le_bytes()).collect()
    }

    /// An Rgba32Float file's texels as colors.
    fn colors_of(file: &TextureFile, level: usize) -> Vec<[f32; 4]> {
        texel::colors(&file.levels[level], file.format).expect("colors")
    }

    /// The direction texel (x, y) of face `face`, `size` texels across,
    /// faces at its center.
    fn texel_direction(face: usize, size: u32, x: u32, y: u32) -> [f64; 3] {
        let at = |i: u32| (2.0 * f64::from(i) + 1.0) / f64::from(size) - 1.0;
        normalized(face_direction(face, at(x), at(y)))
    }

    /// The direction the center of texel (x, y) of an image of
    /// `projection`, `width` by `height` texels, shows; `None` outside an
    /// angular map's disc.
    fn image_texel_direction(projection: Projection, width: u32, height: u32, x: u32, y: u32) -> Option<[f64; 3]> {
        let u = (f64::from(x) + 0.5) / f64::from(width);
        let v = (f64::from(y) + 0.5) / f64::from(height);
        match projection {
            Projection::Panorama => {
                let (phi, theta) = ((u - 0.5) * 2.0 * PI, v * PI);
                Some([theta.sin() * phi.cos(), theta.cos(), theta.sin() * phi.sin()])
            }
            Projection::AngularMap => {
                let (a, b) = (2.0 * u - 1.0, 1.0 - 2.0 * v);
                let r = (a * a + b * b).sqrt();
                if r > 1.0 {
                    return None;
                }
                let angle = PI * r;
                Some(if r == 0.0 { [0.0, 0.0, -1.0] } else { [angle.sin() * a / r, angle.sin() * b / r, -angle.cos()] })
            }
            Projection::Octahedral => {
                let (a, b) = (2.0 * u - 1.0, 2.0 * v - 1.0);
                let up = 1.0 - a.abs() - b.abs();
                Some(normalized(if up >= 0.0 { [a, up, b] } else { [a.signum() * (1.0 - b.abs()), up, b.signum() * (1.0 - a.abs())] }))
            }
        }
    }

    /// A color that follows the direction d: 0.5 + 0.5 d in red, green and
    /// blue, and 1 in alpha.
    fn direction_color(d: [f64; 3]) -> [f32; 4] {
        [(0.5 + 0.5 * d[0]) as f32, (0.5 + 0.5 * d[1]) as f32, (0.5 + 0.5 * d[2]) as f32, 1.0]
    }

    /// An Rgba32Float image of `projection`, `width` by `height`, each
    /// texel the direction color of its center, and 7 outside an angular
    /// map's disc, which no texel of a cube reads.
    fn projection_image(projection: Projection, width: u32, height: u32) -> TextureFile {
        let colors: Vec<[f32; 4]> = (0..height)
            .flat_map(|y| (0..width).map(move |x| (x, y)))
            .map(|(x, y)| image_texel_direction(projection, width, height, x, y).map_or([7.0; 4], direction_color))
            .collect();
        file(F::Rgba32Float, width, height, 1, floats(&colors))
    }

    /// An Rgba32Float cube, faces `size` texels across, each texel `color`
    /// of its direction.
    fn cube_of(size: u32, color: impl Fn([f64; 3]) -> [f32; 4]) -> TextureFile {
        let colors: Vec<[f32; 4]> = (0..6).flat_map(|face| (0..size * size).map(move |i| (face, i))).map(|(face, i)| color(texel_direction(face, size, i % size, i / size))).collect();
        file(F::Rgba32Float, size, size, 6, floats(&colors))
    }

    /// The largest difference between two lists of colors, red, green and
    /// blue.
    fn largest_difference(got: &[[f32; 4]], want: &[[f32; 4]]) -> f64 {
        assert_eq!(got.len(), want.len());
        got.iter().zip(want).flat_map(|(g, w)| (0..3).map(move |c| (f64::from(g[c]) - f64::from(w[c])).abs())).fold(0.0, f64::max)
    }

    /// Neighboring cells of a cross or a tee meet edge to edge: along each
    /// edge two faces share, the points on either side face the same
    /// directions. A vertical cross's -Z, below -Y, meets it only turned.
    #[test]
    fn neighboring_cells_meet_edge_to_edge() {
        use CubeLayout as L;
        for (layout, joins) in [(L::HorizontalCross, 5), (L::VerticalCross, 4), (L::VerticalCrossTurned, 5), (L::HorizontalTee, 5)] {
            let Shape::Layout(_, cells) = shape(layout) else { panic!("{layout:?} has cells") };
            let point = |face: usize, s: f64, t: f64| {
                let turned = cells.turned && face == 5;
                normalized(if turned { face_direction(face, -s, -t) } else { face_direction(face, s, t) })
            };
            let mut found = 0;
            for a in 0..6 {
                for b in 0..6 {
                    let ((ca, ra), (cb, rb)) = (cells.at[a], cells.at[b]);
                    let across = cb == ca + 1 && rb == ra;
                    let down = rb == ra + 1 && cb == ca;
                    if !(across || down) {
                        continue;
                    }
                    let meets = (0..=8).all(|i| {
                        let along = -1.0 + 0.25 * f64::from(i);
                        let (pa, pb) = if across { (point(a, 1.0, along), point(b, -1.0, along)) } else { (point(a, along, 1.0), point(b, along, -1.0)) };
                        (0..3).all(|c| (pa[c] - pb[c]).abs() < 1e-12)
                    });
                    if meets {
                        found += 1;
                    }
                }
            }
            assert_eq!(found, joins, "{layout:?}: {found} of its neighboring cells meet edge to edge");
        }
    }

    /// Every layout's faces come back as they were laid out, -Z turned and
    /// turned back, for texels of one and of eight bytes.
    #[test]
    fn faces_laid_out_and_cut_come_back() {
        for layout in [
            CubeLayout::HorizontalCross,
            CubeLayout::VerticalCross,
            CubeLayout::VerticalCrossTurned,
            CubeLayout::HorizontalTee,
            CubeLayout::HorizontalStrip,
            CubeLayout::VerticalStrip,
            CubeLayout::Grid2x3,
            CubeLayout::Grid3x2,
        ] {
            let Shape::Layout(_, cells) = shape(layout) else { panic!("{layout:?} has cells") };
            for (format, bytes) in [(F::R8Unorm, 1usize), (F::Rgba16Float, 8)] {
                let size = 5u32;
                let faces: Vec<u8> = (0..6 * size as usize * size as usize * bytes).map(|i| (i * 7 % 251) as u8).collect();
                let image = paste(&faces, format, &cells, size).expect("laid out");
                assert_eq!(image.len(), (cells.columns * cells.rows * size * size) as usize * bytes);
                let (width, height) = (cells.columns * size, cells.rows * size);
                assert_eq!(shape_of(width, height, Some(layout)).expect("fits"), Shape::Layout(layout, cells));
                assert_eq!(cut(&image, width, format, &cells, size).expect("cut"), faces, "{layout:?} {format:?}");
            }
        }
    }

    /// The turned cross holds -Z half a turn round: its first texel is the
    /// face's last.
    #[test]
    fn the_turned_cross_holds_minus_z_half_a_turn_round() {
        let Shape::Layout(_, cells) = shape(CubeLayout::VerticalCrossTurned) else { panic!("cells") };
        let size = 3u32;
        let faces: Vec<u8> = (0..6 * 9).map(|i| i as u8).collect();
        let image = paste(&faces, F::R8Unorm, &cells, size).expect("laid out");
        let at = |x: u32, y: u32| image[(y * 3 * size + x) as usize];
        assert_eq!(at(3, 9), faces[5 * 9 + 8]);
        assert_eq!(at(5, 11), faces[5 * 9]);
        assert_eq!(at(3, 3), faces[4 * 9], "+Z keeps its own way round");
    }

    /// Each aspect ratio gives its shape; a square, or a shape no layout
    /// has, is refused, and a named layout the image does not fit too.
    #[test]
    fn the_aspect_ratio_gives_the_shape() {
        let found = |w: u32, h: u32| match shape_of(w, h, None).expect("a shape") {
            Shape::Layout(layout, _) => format!("{layout:?}"),
            Shape::Projection(p) => format!("{p:?}"),
        };
        assert_eq!(found(512, 256), "Panorama");
        assert_eq!(found(256, 192), "HorizontalCross");
        assert_eq!(found(192, 256), "VerticalCross");
        assert_eq!(found(384, 64), "HorizontalStrip");
        assert_eq!(found(64, 384), "VerticalStrip");
        assert_eq!(found(128, 192), "Grid2x3");
        assert_eq!(found(192, 128), "Grid3x2");
        for (w, h, says) in [(256, 256, "-Layout AngularMap"), (300, 100, "no shape a cube comes in"), (512, 255, "no shape a cube comes in")] {
            match shape_of(w, h, None) {
                Ok(shape) => panic!("{w}x{h} gave {shape:?}"),
                Err(e) => assert!(e.error_id == "WgpuCubeMap" && e.message.contains(says), "{w}x{h}: {}", e.message),
            }
        }
        assert_eq!(shape_of(256, 256, Some(CubeLayout::Octahedral)).expect("a square fits"), Shape::Projection(Projection::Octahedral));
        assert_eq!(shape_of(256, 192, Some(CubeLayout::HorizontalTee)).expect("4:3 fits"), shape(CubeLayout::HorizontalTee));
        match shape_of(256, 192, Some(CubeLayout::Grid2x3)) {
            Ok(shape) => panic!("a 4:3 image fit {shape:?}"),
            Err(e) => assert!(e.message.contains("2 cells across and 3 down"), "{}", e.message),
        }
    }

    /// The texels' solid angles cover the sphere, 4 pi, at any size.
    #[test]
    fn the_texels_cover_the_sphere() {
        for size in [1u32, 2, 7, 64] {
            let sum: f64 = (0..size).flat_map(|y| (0..size).map(move |x| texel_solid_angle(size, x, y))).sum::<f64>() * 6.0;
            assert!((sum - 4.0 * PI).abs() < 1e-12, "{size}: {sum}");
        }
    }

    /// A panorama, an angular map and an octahedral map of colors that
    /// follow the direction give a cube whose texels follow it too, each
    /// within 3e-3 of its center's color, so each projection's directions
    /// land on the faces where the cube has them.
    #[test]
    fn each_projection_lands_where_the_cube_has_its_directions() {
        let Some(core) = core() else { return };
        let size = 16;
        let want = cube_of(size, direction_color);
        for (projection, width, height) in [(Projection::Panorama, 128, 64), (Projection::AngularMap, 128, 128), (Projection::Octahedral, 64, 64)] {
            let mut said = Said::default();
            let made = resampled_cube(&core, &projection_image(projection, width, height), projection, size, 0.0, None, &mut said).expect("resampled");
            let gap = largest_difference(&colors_of(&made, 0), &colors_of(&want, 0));
            assert!(gap < 3e-3, "{projection:?}: {gap}");
        }
    }

    /// -Yaw 90 turns +X toward -Z: the panorama's center, which faces +X,
    /// comes to the cube's -Z face.
    #[test]
    fn yaw_turns_x_toward_minus_z() {
        let Some(core) = core() else { return };
        let size = 8;
        let made = resampled_cube(&core, &projection_image(Projection::Panorama, 64, 32), Projection::Panorama, size, 90.0, None, &mut Said::default()).expect("resampled");
        let want = cube_of(size, |d| direction_color([-d[2], d[1], d[0]]));
        let gap = largest_difference(&colors_of(&made, 0), &colors_of(&want, 0));
        assert!(gap < 1e-2, "{gap}");
        let center = colors_of(&made, 0)[5 * 64 + 3 * 8 + 3];
        assert!(center[0] > 0.9, "the -Z face shows the panorama's +X: {center:?}");
    }

    /// A cube written into each projection and read back comes back within
    /// 1e-2 of itself, and -Yaw both ways gives it back too.
    #[test]
    fn a_cube_written_into_each_projection_comes_back() {
        let Some(core) = core() else { return };
        let size = 16;
        let cube = cube_of(size, direction_color);
        for projection in [Projection::Panorama, Projection::AngularMap, Projection::Octahedral] {
            for yaw in [0.0, 37.0] {
                let mut said = Said::default();
                let image = resampled_image(&core, &cube, projection, size, yaw, None, &mut said).expect("written");
                let back = resampled_cube(&core, &image, projection, size, yaw, None, &mut said).expect("read");
                let gap = largest_difference(&colors_of(&back, 0), &colors_of(&cube, 0));
                assert!(gap < 1e-2, "{projection:?} at {yaw} degrees: {gap}");
            }
        }
    }

    /// An environment of one color has that color as its irradiance and at
    /// every level of its specular map, to 1e-5, what rounding the float
    /// sums of thousands of texels and samples leaves.
    #[test]
    fn one_color_lights_as_itself() {
        let Some(core) = core() else { return };
        let gray = [0.25f32, 0.5, 0.75, 1.0];
        let mut said = Said::default();
        let (made, harmonics) = irradiance(&core, cube_of(16, |_| gray), 8, true, None, &mut said).expect("irradiance");
        for c in colors_of(&made, 0) {
            for k in 0..3 {
                assert!((c[k] - gray[k]).abs() < 1e-5, "{c:?}");
            }
        }
        let harmonics = harmonics.expect("harmonics");
        assert!((harmonics.red[0] * 0.282_094_791_773_878_14 - PI * 0.25).abs() < 1e-6, "{harmonics:?}");
        assert!(harmonics.green[1..].iter().all(|c| c.abs() < 1e-6), "{harmonics:?}");
        let specular = specular(&core, cube_of(16, |_| gray), RoughnessMapping::Linear, 64, None, None, &mut said).expect("specular");
        assert_eq!(specular.levels.len(), 4);
        for level in 0..4 {
            for c in colors_of(&specular, level) {
                for k in 0..3 {
                    assert!((c[k] - gray[k]).abs() < 1e-5, "level {level}: {c:?}");
                }
            }
        }
    }

    /// An environment that follows the direction, L = a + b . d, gives the
    /// irradiance a + 2/3 b . n at each texel's direction n, within 2e-3,
    /// and harmonics whose sum there is pi times that, within 2e-3.
    #[test]
    fn a_linear_environment_gives_its_irradiance() {
        let Some(core) = core() else { return };
        let (a, b) = ([0.5, 0.4, 0.3], [[0.2, -0.1, 0.3], [-0.25, 0.05, 0.1], [0.0, 0.3, -0.2]]);
        let light = |d: [f64; 3], c: usize| a[c] + b[c][0] * d[0] + b[c][1] * d[1] + b[c][2] * d[2];
        let env = cube_of(64, |d| [light(d, 0) as f32, light(d, 1) as f32, light(d, 2) as f32, 1.0]);
        let size = 8;
        let (made, harmonics) = irradiance(&core, env, size, true, None, &mut Said::default()).expect("irradiance");
        let harmonics = harmonics.expect("harmonics");
        for (i, got) in colors_of(&made, 0).iter().enumerate() {
            let n = texel_direction(i / 64, size, (i % 64) as u32 % size, (i % 64) as u32 / size);
            let functions = basis(n);
            for c in 0..3 {
                let want = a[c] + 2.0 / 3.0 * (b[c][0] * n[0] + b[c][1] * n[1] + b[c][2] * n[2]);
                assert!((f64::from(got[c]) - want).abs() < 2e-3, "texel {i} channel {c}: {} against {want}", got[c]);
                let coefficients = [&harmonics.red, &harmonics.green, &harmonics.blue][c];
                let sum: f64 = coefficients.iter().zip(functions).map(|(k, f)| k * f).sum();
                assert!((sum - PI * want).abs() < 2e-3, "texel {i} channel {c}: harmonics give {sum} against {}", PI * want);
            }
        }
    }

    /// Level 0 of a specular map is the environment itself, texel for
    /// texel.
    #[test]
    fn level_0_of_a_specular_map_is_the_environment() {
        let Some(core) = core() else { return };
        let env = cube_of(8, direction_color);
        let made = specular(&core, cube_of(8, direction_color), RoughnessMapping::Quadratic, 16, None, None, &mut Said::default()).expect("specular");
        assert_eq!(made.levels.len(), 2);
        assert_eq!(colors_of(&made, 0), colors_of(&env, 0));
    }

    /// Texel (x, y), channel c, of the test image of `w` by `h` texels the
    /// reference computes from.
    fn image_value(x: u32, y: u32, w: u32, h: u32, c: usize) -> f64 {
        let (x, y, w, h, c) = (f64::from(x), f64::from(y), f64::from(w), f64::from(h), c as f64);
        0.5 + 0.3 * (2.0 * PI * x / w + 0.7 * c).sin() * (PI * (y + 0.5) / h).sin() + 0.15 * (4.0 * PI * y / h + c).cos()
    }

    /// The test image as Rgba32Float, alpha 1.
    fn test_image(w: u32, h: u32) -> TextureFile {
        let colors: Vec<[f32; 4]> =
            (0..h).flat_map(|y| (0..w).map(move |x| [image_value(x, y, w, h, 0) as f32, image_value(x, y, w, h, 1) as f32, image_value(x, y, w, h, 2) as f32, 1.0])).collect();
        file(F::Rgba32Float, w, h, 1, floats(&colors))
    }

    /// The test environment's light from the unit direction d, channel c: a
    /// gentle gradient and a sun 3 times as bright, about 0.3 radians wide.
    fn env_value(d: [f64; 3], c: usize) -> f64 {
        let norm = 1.52f64.sqrt();
        let sun = [1.0 / norm, 0.6 / norm, -0.4 / norm];
        let glow = 3.0 * [1.0, 0.9, 0.7][c] * (12.0 * (d[0] * sun[0] + d[1] * sun[1] + d[2] * sun[2] - 1.0)).exp();
        0.4 + 0.2 * d[c] + 0.1 * d[(c + 1) % 3] * d[(c + 1) % 3] + glow
    }

    fn test_environment(size: u32) -> TextureFile {
        cube_of(size, |d| [env_value(d, 0) as f32, env_value(d, 1) as f32, env_value(d, 2) as f32, 1.0])
    }

    /// Asserts each of `spots`, (face, x, y, color), of a cube `size` texels
    /// across, within `tolerance` of `colors`.
    fn assert_cube(name: &str, colors: &[[f32; 4]], size: u32, spots: &[(usize, u32, u32, [f32; 3])], tolerance: f32) {
        for &(face, x, y, want) in spots {
            let got = colors[face * (size * size) as usize + (y * size + x) as usize];
            for c in 0..3 {
                assert!((got[c] - want[c]).abs() <= tolerance, "{name} face {face} texel {x},{y} channel {c}: {} against the reference's {}", got[c], want[c]);
            }
        }
    }

    /// Asserts each of `spots`, (x, y, color), of an image `width` texels
    /// wide, within `tolerance` of `colors`.
    fn assert_image(name: &str, colors: &[[f32; 4]], width: u32, spots: &[(u32, u32, [f32; 3])], tolerance: f32) {
        for &(x, y, want) in spots {
            let got = colors[(y * width + x) as usize];
            for c in 0..3 {
                assert!((got[c] - want[c]).abs() <= tolerance, "{name} texel {x},{y} channel {c}: {} against the reference's {}", got[c], want[c]);
            }
        }
    }

    /// The resamplings, the irradiance cube and its harmonics, the Linear
    /// specular levels, a Quadratic one and the BRDF table match a reference
    /// in doubles written apart from the shaders with numpy, from the same
    /// inputs: each conversion and the irradiance within 2e-5, the specular
    /// levels within 1e-4, the BRDF table within 1e-4 and 1e-3 at grazing,
    /// and the harmonics, summed in doubles on the CPU, within 1e-9.
    #[test]
    fn matches_the_reference_in_doubles() {
        let Some(core) = core() else { return };
        let mut said = Said::default();
        assert_eq!(sampling((2.0 / 16.0) / Projection::Panorama.finest(64), 7), (0, 3));
        assert_eq!(sampling((2.0 / 16.0) / Projection::AngularMap.finest(64), 7), (0, 3));
        assert_eq!(sampling((2.0 / 16.0) / Projection::Octahedral.finest(32), 6), (0, 4));
        for (projection, width) in [(Projection::Panorama, 64), (Projection::AngularMap, 64), (Projection::Octahedral, 32)] {
            assert_eq!(sampling(projection.coarsest(width) / (corner_angle() / 16.0), 5), (0, 4), "{projection:?}");
        }
        let made = resampled_cube(&core, &test_image(64, 32), Projection::Panorama, 16, 0.0, None, &mut said).expect("resampled");
        assert_cube("panorama to cube", &colors_of(&made, 0), 16, &reference::PANORAMA_TO_CUBE, 2e-5);
        let made = resampled_cube(&core, &test_image(64, 64), Projection::AngularMap, 16, 30.0, None, &mut said).expect("resampled");
        assert_cube("angular map to cube", &colors_of(&made, 0), 16, &reference::ANGULAR_TO_CUBE, 2e-5);
        let made = resampled_cube(&core, &test_image(32, 32), Projection::Octahedral, 16, 0.0, None, &mut said).expect("resampled");
        assert_cube("octahedral map to cube", &colors_of(&made, 0), 16, &reference::OCTAHEDRAL_TO_CUBE, 2e-5);
        let env = test_environment(16);
        let made = resampled_image(&core, &env, Projection::Panorama, 16, 0.0, None, &mut said).expect("resampled");
        assert_image("cube to panorama", &colors_of(&made, 0), 64, &reference::CUBE_TO_PANORAMA, 2e-5);
        let made = resampled_image(&core, &env, Projection::AngularMap, 16, -20.0, None, &mut said).expect("resampled");
        assert_image("cube to angular map", &colors_of(&made, 0), 64, &reference::CUBE_TO_ANGULAR, 2e-5);
        let made = resampled_image(&core, &env, Projection::Octahedral, 16, 0.0, None, &mut said).expect("resampled");
        assert_image("cube to octahedral map", &colors_of(&made, 0), 32, &reference::CUBE_TO_OCTAHEDRAL, 2e-5);
        let (made, harmonics) = irradiance(&core, test_environment(16), 8, true, None, &mut said).expect("irradiance");
        assert_cube("irradiance", &colors_of(&made, 0), 8, &reference::IRRADIANCE, 2e-5);
        let harmonics = harmonics.expect("harmonics");
        for (c, got) in [&harmonics.red, &harmonics.green, &harmonics.blue].into_iter().enumerate() {
            for (i, (g, w)) in got.iter().zip(reference::HARMONICS[c]).enumerate() {
                assert!((g - w).abs() <= 1e-9 * w.abs().max(1.0), "harmonic {i} channel {c}: {g} against the reference's {w}");
            }
        }
        let made = specular(&core, test_environment(16), RoughnessMapping::Linear, 64, None, None, &mut said).expect("specular");
        assert_eq!(made.levels.len(), 4);
        for (level, spots) in [(1usize, &reference::SPECULAR_LINEAR_1[..]), (2, &reference::SPECULAR_LINEAR_2[..]), (3, &reference::SPECULAR_LINEAR_3[..])] {
            assert_cube(&format!("Linear specular level {level}"), &colors_of(&made, level), 16 >> level, spots, 1e-4);
        }
        let made = specular(&core, test_environment(32), RoughnessMapping::Quadratic, 64, None, None, &mut said).expect("specular");
        assert_eq!(made.levels.len(), 2);
        assert_cube("Quadratic specular level 1", &colors_of(&made, 1), 16, &reference::SPECULAR_QUADRATIC_1, 1e-4);
        let table = brdf_table(&core, 32, 256, None, &mut said).expect("table");
        let texels = colors_of(&table, 0);
        for (x, y, want) in reference::BRDF {
            let got = texels[(y * 32 + x) as usize];
            let tolerance = if x == 0 { 1e-3 } else { 1e-4 };
            for c in 0..2 {
                assert!((got[c] - want[c]).abs() <= tolerance, "BRDF texel {x},{y} channel {c}: {} against the reference's {}", got[c], want[c]);
            }
        }
    }

    /// Filament v1.77.1's cmgen, given test_environment(64) as a horizontal
    /// cross in a .exr file of 32-bit floats, gives the same irradiance
    /// harmonics, to 2e-5 relative, and the same BRDF table, to 1e-3, what
    /// its half-float output holds, with its rows read the other way up and
    /// one row on. Its Quadratic levels read each mip level half a texel off
    /// and turn their samples by a random angle at each texel, so they stay
    /// within 0.06 of the module's, which reads at texel centers as
    /// tools/cube_reference.py does.
    #[test]
    fn matches_cmgen() {
        let Some(core) = core() else { return };
        let mut said = Said::default();
        let (_, harmonics) = irradiance(&core, test_environment(64), 8, true, None, &mut said).expect("irradiance");
        let harmonics = harmonics.expect("harmonics");
        for (c, got) in [&harmonics.red, &harmonics.green, &harmonics.blue].into_iter().enumerate() {
            for (i, (g, w)) in got.iter().zip(CMGEN_HARMONICS[c]).enumerate() {
                assert!((g - w).abs() <= 2e-5 * w.abs().max(1e-3), "harmonic {i} channel {c}: {g} against cmgen's {w}");
            }
        }
        let table = brdf_table(&core, 128, 1024, None, &mut said).expect("table");
        let texels = colors_of(&table, 0);
        for (x, y, want) in CMGEN_DFG {
            let got = texels[(y * 128 + x) as usize];
            for c in 0..2 {
                assert!((got[c] - want[c]).abs() <= 1e-3, "BRDF texel {x},{y} channel {c}: {} against cmgen's {}", got[c], want[c]);
            }
        }
        let made = specular(&core, test_environment(64), RoughnessMapping::Quadratic, 4096, None, None, &mut said).expect("specular");
        assert_eq!(made.levels.len(), 3);
        for (level, face, x, y, want) in CMGEN_QUADRATIC {
            let side = 64 >> level;
            let got = colors_of(&made, level)[face * (side * side) as usize + (y * side + x) as usize];
            for c in 0..3 {
                assert!((got[c] - want[c]).abs() <= 0.06, "Quadratic level {level} face {face} texel {x},{y} channel {c}: {} against cmgen's {}", got[c], want[c]);
            }
        }
    }

    /// cmgen --no-mirror --size=64 --sh=3 --sh-irradiance --sh-window=no:
    /// the harmonics, red, green and blue, by index.
    const CMGEN_HARMONICS: [[f64; 9]; 3] = [
        [6.218016147613525, -0.717147171497345, -0.478108495473862, -2.052467107772827, 0.410153895616531, 0.164075002074242, -0.246708869934082, 0.273439139127731, 0.14683610200882],
        [6.078803062438965, -1.502733588218689, -0.430296152830124, -1.075652003288269, 0.369138121604919, 0.147666499018669, -0.101687498390675, 0.24609549343586, 0.196844205260277],
        [5.80037784576416, -0.502001643180847, 0.522626340389252, -0.836615920066833, 0.287107706069946, 0.114851996302605, -0.185144156217575, 0.191407680511475, 0.224978923797607],
    ];

    /// cmgen --size=128 --ibl-dfg=dfg.exr: chosen texels, (x, y, scale and
    /// bias), y counted as the module's table counts it, cmgen's row 128 - y.
    const CMGEN_DFG: [(u32, u32, [f32; 2]); 8] = [
        (2, 1, [0.093933105, 0.90625]),
        (127, 1, [1.0, 0.0]),
        (64, 64, [0.83203125, 0.021514893]),
        (127, 127, [0.31201172, 3.7312508E-05]),
        (100, 20, [0.99902344, 0.00052785873]),
        (10, 90, [0.81591797, 0.066345215]),
        (40, 120, [0.5805664, 0.009719849]),
        (5, 30, [0.41918945, 0.4711914]),
    ];

    /// cmgen --no-mirror --format=exr --size=64 --ibl-samples=4096 --ibl-ld:
    /// chosen texels of Quadratic levels 1 and 2, perceptual roughness
    /// 1 - sqrt(1/2) and 1, (level, face, x, y, color); cmgen takes 4096
    /// samples a texel at level 1 and 8192 at level 2.
    const CMGEN_QUADRATIC: [(usize, usize, u32, u32, [f32; 3]); 36] = [
        (1, 0, 0, 0, [0.60595703, 0.59228516, 0.5864258]),
        (1, 0, 31, 16, [0.8544922, 0.72314453, 0.53271484]),
        (1, 0, 16, 31, [0.58984375, 0.27246094, 0.44458008]),
        (1, 1, 0, 0, [0.31982422, 0.5419922, 0.32861328]),
        (1, 1, 31, 16, [0.2680664, 0.43969727, 0.58447266]),
        (1, 1, 16, 31, [0.31225586, 0.26733398, 0.453125]),
        (1, 2, 0, 0, [0.32592773, 0.54589844, 0.3269043]),
        (1, 2, 31, 16, [1.5585938, 1.4160156, 1.1347656]),
        (1, 2, 16, 31, [0.46069336, 0.58740234, 0.54248047]),
        (1, 3, 0, 0, [0.32885742, 0.31640625, 0.5410156]),
        (1, 3, 31, 16, [0.5878906, 0.26782227, 0.44140625]),
        (1, 3, 16, 31, [0.4543457, 0.3083496, 0.26879883]),
        (1, 4, 0, 0, [0.32470703, 0.5439453, 0.5488281]),
        (1, 4, 31, 16, [0.5498047, 0.4494629, 0.5913086]),
        (1, 4, 16, 31, [0.45629883, 0.31225586, 0.5395508]),
        (1, 5, 0, 0, [1.7353516, 1.6171875, 1.1542969]),
        (1, 5, 31, 16, [0.2680664, 0.44067383, 0.31225586]),
        (1, 5, 16, 31, [0.44262695, 0.3149414, 0.26489258]),
        (2, 0, 0, 0, [0.7910156, 0.7475586, 0.7011719]),
        (2, 0, 15, 8, [0.86083984, 0.73291016, 0.58203125]),
        (2, 0, 8, 15, [0.6616211, 0.44262695, 0.5131836]),
        (2, 1, 0, 0, [0.4033203, 0.5488281, 0.4008789]),
        (2, 1, 15, 8, [0.33398438, 0.42456055, 0.53125]),
        (2, 1, 8, 15, [0.3449707, 0.32861328, 0.44384766]),
        (2, 2, 0, 0, [0.42041016, 0.56396484, 0.4074707]),
        (2, 2, 15, 8, [0.9404297, 0.8852539, 0.7294922]),
        (2, 2, 8, 15, [0.56347656, 0.63134766, 0.5996094]),
        (2, 3, 0, 0, [0.36645508, 0.34692383, 0.50634766]),
        (2, 3, 15, 8, [0.63720703, 0.4182129, 0.49731445]),
        (2, 3, 8, 15, [0.47216797, 0.36035156, 0.3503418]),
        (2, 4, 0, 0, [0.37060547, 0.51171875, 0.5239258]),
        (2, 4, 15, 8, [0.67041016, 0.55908203, 0.6352539]),
        (2, 4, 8, 15, [0.45263672, 0.34277344, 0.51904297]),
        (2, 5, 0, 0, [0.92089844, 0.87646484, 0.64208984]),
        (2, 5, 15, 8, [0.34033203, 0.43237305, 0.34887695]),
        (2, 5, 8, 15, [0.453125, 0.3635254, 0.34155273]),
    ];

    /// Each mapping's roughness by level, and the levels it gives.
    #[test]
    fn each_mapping_steps_through_roughness() {
        assert_eq!(RoughnessMapping::Linear.levels(256), 8);
        assert_eq!(RoughnessMapping::Quadratic.levels(256), 5);
        assert_eq!(RoughnessMapping::Quadratic.levels(16), 2);
        assert_eq!(RoughnessMapping::Linear.levels(1), 1);
        assert_eq!(RoughnessMapping::Linear.roughness(2, 5), 0.5);
        assert!((RoughnessMapping::Quadratic.roughness(2, 5) - (1.0 - 0.5f64.sqrt())).abs() < 1e-15);
        assert_eq!(RoughnessMapping::Quadratic.roughness(4, 5), 1.0);
        assert_eq!(RoughnessMapping::Linear.roughness(0, 1), 0.0);
    }

    /// The BRDF table's scale and bias add up to about 1 where the surface
    /// is smooth and seen head on, and its bias is about 0 there.
    #[test]
    fn a_smooth_surface_seen_head_on_reflects_f0() {
        let Some(core) = core() else { return };
        let size = 32;
        let table = brdf_table(&core, size, 256, None, &mut Said::default()).expect("table");
        let texels = colors_of(&table, 0);
        let head_on = texels[(size - 1) as usize];
        assert!((head_on[0] + head_on[1] - 1.0).abs() < 2e-2 && head_on[1] < 1e-3, "{head_on:?}");
        assert!(texels.iter().all(|t| t[0] >= 0.0 && t[1] >= 0.0 && t[0] + t[1] <= 1.0 + 1e-3), "every texel reflects no more than it receives");
    }
}
