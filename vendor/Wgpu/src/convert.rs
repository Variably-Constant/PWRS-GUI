//! Conversions of a texture on its device: mip levels rendered by a
//! filter, each from the level above it or from level 0, and every level
//! and layer encoded in a BC format by compute shaders, block_compression's
//! for BC1 to BC7 and BC6H and the module's own for signed BC4 and BC5.

use std::sync::Arc;

use block_compression::{BC6HSettings, BC7Settings};
use pwrs::prelude::*;
use wgpu::TextureFormat as F;
use wgpu::util::DeviceExt;

use crate::buffer;
use crate::device::CoreRef;
use crate::error::invalid;
use crate::normal::{self, NormalMips};
use crate::tasks::Interrupt;
use crate::texfile::TextureFile;
use crate::texture::{format_features, format_name, write_texels};
use crate::transform::KERNELS;

/// How each mip level is filtered from the image above it.
#[psenum(name = "Wgpu.MipFilter")]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum MipFilter {
    /// The average of the texels each new texel covers, each weighed by how
    /// much of it the new texel covers, from the level above.
    #[default]
    Box,
    /// The triangle, from level 0.
    Triangle,
    /// A sinc under Kaiser's window 3 texels wide with alpha 4, from level 0.
    Kaiser,
    /// Mitchell and Netravali's cubic with B and C a third, from level 0.
    Mitchell,
    /// Lanczos with three lobes, as Resize's, from level 0.
    Lanczos,
    /// The smallest value of each channel among the texels each new texel
    /// covers, from the level above.
    Min,
    /// The largest value of each channel among the texels each new texel
    /// covers, from the level above.
    Max,
}

impl MipFilter {
    /// The kernel the resample shader knows a filter of level 0 by, and how
    /// far it reaches each way in texels of the level; `None` for a filter
    /// of the level above.
    fn kernel(self) -> Option<(u32, f32)> {
        match self {
            MipFilter::Triangle => Some((2, 1.0)),
            MipFilter::Lanczos => Some((4, 3.0)),
            MipFilter::Kaiser => Some((5, 3.0)),
            MipFilter::Mitchell => Some((6, 2.0)),
            MipFilter::Box | MipFilter::Min | MipFilter::Max => None,
        }
    }

    /// The pass that makes a level from the level above with this filter.
    fn reduce(self) -> &'static str {
        match self {
            MipFilter::Min => "reduce_min",
            MipFilter::Max => "reduce_max",
            _ => "reduce",
        }
    }
}

/// What the filters of level 0 read past an edge of the image.
#[psenum(name = "Wgpu.MipEdge")]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum MipEdge {
    /// Nothing: a texel past the edge is left out and the rest weigh the
    /// new texel, as Resize does.
    #[default]
    Clamp,
    /// The other side of the image, for a texture that tiles.
    Wrap,
    /// The image mirrored at its edge, the edge texel repeated, as a
    /// MirrorRepeat sampler reads it.
    Mirror,
}

impl MipEdge {
    /// The number the resample shader knows the edge by.
    fn code(self) -> u32 {
        match self {
            MipEdge::Clamp => 0,
            MipEdge::Wrap => 1,
            MipEdge::Mirror => 2,
        }
    }

    /// The edge a name gives, in any case: Clamp, Wrap or Mirror, or the
    /// sampler address mode that names it, ClampToEdge, Repeat or
    /// MirrorRepeat; `None` for any other name. ClampToBorder is refused,
    /// as a mip level has no border color.
    pub(crate) fn named(name: &str) -> PsResult<Option<MipEdge>> {
        let edge = match name.trim().to_ascii_lowercase().as_str() {
            "clamp" | "clamptoedge" => MipEdge::Clamp,
            "wrap" | "repeat" => MipEdge::Wrap,
            "mirror" | "mirrorrepeat" => MipEdge::Mirror,
            "clamptoborder" => return Err(invalid("WgpuMipEdge", "a mip level has no border color: the edge is Clamp, Wrap or Mirror, or ClampToEdge, Repeat or MirrorRepeat")),
            _other => return Ok(None),
        };
        Ok(Some(edge))
    }
}

/// How the mip levels a call renders are made: Box and Clamp unless the
/// call names others, with their alpha scaled to keep level 0's alpha
/// coverage when the call names a reference for it, and, for a texture of
/// normals, each normal kept unit length as `normals` says.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct MipOptions {
    pub(crate) filter: MipFilter,
    pub(crate) edge: MipEdge,
    /// Whether the call named the filter.
    pub(crate) filter_named: bool,
    /// Whether the call named the edge.
    pub(crate) edge_named: bool,
    /// -KeepCoverage: the alpha an alpha test passes at, whose share of
    /// texels each rendered level keeps from level 0.
    pub(crate) coverage: Option<f32>,
    /// How the levels keep the normals a texture holds unit length; `None`
    /// filters its texels as colors.
    pub(crate) normals: Option<NormalMips>,
    /// Whether the call named how the levels keep normals unit length.
    pub(crate) normals_named: bool,
    /// Whether the normals are stored as they are, as a format that holds
    /// negative values stores them, rather than as n / 2 + 1 / 2.
    pub(crate) signed: bool,
}

impl MipOptions {
    /// The options a call's filter and edge give.
    pub(crate) fn given(filter: Option<MipFilter>, edge: Option<MipEdge>) -> MipOptions {
        let (filter_named, edge_named) = (filter.is_some(), edge.is_some());
        MipOptions { filter: filter.unwrap_or(MipFilter::Box), edge: edge.unwrap_or(MipEdge::Clamp), filter_named, edge_named, ..MipOptions::default() }
    }

    /// The options with -KeepCoverage's reference, above 0 and at most 1.
    pub(crate) fn with_coverage(self, reference: Option<f64>) -> PsResult<MipOptions> {
        match reference {
            None => Ok(self),
            Some(r) if r > 0.0 && r <= 1.0 => Ok(MipOptions { coverage: Some(r as f32), ..self }),
            Some(r) => Err(invalid("WgpuKeepCoverage", format!("-KeepCoverage is the alpha an alpha test passes at, above 0 and at most 1, not {r}"))),
        }
    }

    /// The options with `normals`, how the levels keep normals unit
    /// length, which the call named when `named`.
    pub(crate) fn with_normals(self, normals: Option<NormalMips>, named: bool) -> MipOptions {
        MipOptions { normals, normals_named: named && normals.is_some(), ..self }
    }

    /// The options with a call's -NormalMips, and Renormalize, not named,
    /// where it names none and `makes_normals` from heights.
    pub(crate) fn for_normals(self, named: Option<NormalMips>, makes_normals: bool) -> MipOptions {
        match named {
            Some(normals) => self.with_normals(Some(normals), true),
            None if makes_normals => self.with_normals(Some(NormalMips::Renormalize), false),
            None => self,
        }
    }

    /// The options for normals stored as a texture of `format` stores them.
    pub(crate) fn stored_as(self, format: F) -> MipOptions {
        MipOptions { signed: normal::signed(format), ..self }
    }

    /// Whether the call named the filter, the edge, a coverage to keep or
    /// how normals stay unit length.
    pub(crate) fn named(self) -> bool {
        self.filter_named || self.edge_named || self.coverage.is_some() || self.normals_named
    }

    /// Whether the call named how normals stay unit length and nothing
    /// else: beside a format to convert into, that marks the texels as
    /// normals, written again where the format stores them the other way.
    pub(crate) fn names_normals_alone(self) -> bool {
        self.normals_named && !self.filter_named && !self.edge_named && self.coverage.is_none()
    }

    /// What the call named, for a warning that it changed nothing.
    pub(crate) fn what(self) -> String {
        let mut named = match (self.filter_named, self.edge_named) {
            (true, true) => vec!["the mip filter and edge"],
            (true, false) => vec!["the mip filter"],
            (false, true) => vec!["the mip edge"],
            (false, false) => vec![],
        };
        if self.coverage.is_some() {
            named.push("-KeepCoverage");
        }
        if self.normals_named {
            named.push("-NormalMips");
        }
        named.join(" and ")
    }

    /// Refuses a Wrap or Mirror edge with Box, Min or Max, which read only
    /// the texels each new texel covers, and normals with Min or Max,
    /// which take each channel apart.
    pub(crate) fn check(self) -> PsResult<()> {
        if self.normals.is_some() && matches!(self.filter, MipFilter::Min | MipFilter::Max) {
            let which = if self.filter == MipFilter::Min { "smallest" } else { "largest" };
            return Err(invalid(
                "WgpuNormalMap",
                format!("{:?} takes each channel's {which} value on its own, which makes no normal of a normal's channels: give -NormalMips with Box, Triangle, Kaiser, Mitchell or Lanczos", self.filter),
            ));
        }
        if self.filter.kernel().is_some() || self.edge == MipEdge::Clamp {
            return Ok(());
        }
        let filter = if self.filter_named { format!("{:?}", self.filter) } else { "Box, the filter when none is given,".to_string() };
        Err(invalid(
            "WgpuMipEdge",
            format!("{filter} reads only the texels each new texel covers and never past an edge, so the {:?} edge changes nothing; Triangle, Kaiser, Mitchell and Lanczos read past one", self.edge),
        ))
    }

    /// How the levels were made, for a texture's Notes; Box says "each from
    /// the level above it".
    pub(crate) fn said(self) -> String {
        let edge = match self.edge {
            MipEdge::Clamp => "",
            MipEdge::Wrap => ", its edges wrapped",
            MipEdge::Mirror => ", its edges mirrored",
        };
        let made = match self.filter {
            MipFilter::Box => "each from the level above it".to_string(),
            MipFilter::Min => "each the smallest of the texels it covers in the level above it".to_string(),
            MipFilter::Max => "each the largest of the texels it covers in the level above it".to_string(),
            other => format!("each from level 0 by the {other:?} filter{edge}"),
        };
        let made = match self.normals {
            None => made,
            Some(NormalMips::Renormalize) => format!("{made}, each texel's normal scaled back to length 1"),
            Some(NormalMips::SlopeSpace) => format!("{made}, the normals' slopes filtered and made unit normals again"),
        };
        match self.coverage {
            None => made,
            Some(reference) => format!("{made}, its alpha scaled to keep level 0's share of texels with alpha at or above {reference}"),
        }
    }

    /// How the passes read and write each texel: 0 as stored, 1 as a
    /// normal, scaled back to length 1 when written, and 2 as a normal's
    /// slopes, made a unit normal when written.
    fn mode(self) -> u32 {
        match self.normals {
            None => 0,
            Some(NormalMips::Renormalize) => 1,
            Some(NormalMips::SlopeSpace) => 2,
        }
    }
}

/// The functions the passes read and write normals by, after the
/// constants `normal_constants` writes: whether the texture stores normals
/// as they are, and whether it stores X and Y alone.
const NORMALS: &str = "
// A normal from the values a texel stores: a format that holds no
// negative values stores n / 2 + 1 / 2, and any other n itself. One that
// stores X and Y alone has its Z rebuilt.
fn normal_of(s: vec3<f32>) -> vec3<f32> {
    var n = select(s * 2.0 - 1.0, s, SIGNED);
    if (XY_ONLY) {
        n.z = sqrt(max(0.0, 1.0 - n.x * n.x - n.y * n.y));
    }
    return n;
}

fn stored_normal(n: vec3<f32>) -> vec3<f32> {
    return select(n * 0.5 + 0.5, n, SIGNED);
}

// A texel as a filter reads it: by `mode` 1 its normal, and by 2 its
// normal's slopes, x / z and y / z, its z held at 1/255 or above so no
// slope passes 255; by 0 as stored. Alpha is kept.
fn read_as(t: vec4<f32>, mode: u32) -> vec4<f32> {
    let n = normal_of(t.xyz);
    var v = t;
    if (mode == 1u) {
        v = vec4<f32>(n, t.a);
    } else if (mode == 2u) {
        v = vec4<f32>(n.xy / max(n.z, 1.0 / 255.0), 0.0, t.a);
    }
    return v;
}

// A filtered texel stored again: by `mode` 1 a normal scaled back to
// length 1, straight up where it has none, and by 2 slopes made a unit
// normal; by 0 as filtered. Alpha is kept.
fn written_as(v: vec4<f32>, mode: u32) -> vec4<f32> {
    var t = v;
    if (mode == 1u) {
        let size = length(v.xyz);
        t = vec4<f32>(stored_normal(select(vec3<f32>(0.0, 0.0, 1.0), v.xyz / size, size > 0.0)), v.a);
    } else if (mode == 2u) {
        t = vec4<f32>(stored_normal(normalize(vec3<f32>(v.xy, 1.0))), v.a);
    }
    return t;
}
";

/// The constants and functions a pass over a texture of `format` reads
/// normals by: stored as they are when `signed`, and X and Y alone in a
/// format of fewer than three channels.
fn normal_constants(signed: bool, format: F) -> String {
    format!("const SIGNED: bool = {signed};\nconst XY_ONLY: bool = {};\n{NORMALS}", format.components() < 3)
}

/// The fullscreen triangle every pass here draws, and the passes that read
/// a 2D level: the next level down, and a copy padded to whole blocks.
const SHADER_2D: &str = "
@group(0) @binding(0) var source: texture_2d<f32>;

@vertex fn vertex(@builtin(vertex_index) index: u32) -> @builtin(position) vec4<f32> {
    let corner = vec2<f32>(f32((index << 1u) & 2u), f32(index & 2u));
    return vec4<f32>(corner.x * 2.0 - 1.0, 1.0 - corner.y * 2.0, 0.0, 1.0);
}

// A texel of the next level down: the texels under it, each weighed by
// how much of it the texel covers, so an odd size loses no texel, each
// read as `read_as` reads it by `mode` and the average written as
// `written_as` writes it.
fn boxed(position: vec2<f32>, mode: u32) -> vec4<f32> {
    let size = textureDimensions(source);
    let smaller = max(size >> vec2<u32>(1u), vec2<u32>(1u));
    let scale = vec2<f32>(size) / vec2<f32>(smaller);
    let low = floor(position) * scale;
    let high = low + scale;
    var sum = vec4<f32>(0.0);
    var area = 0.0;
    for (var y = u32(low.y); f32(y) < high.y && y < size.y; y++) {
        let wy = min(high.y, f32(y + 1u)) - max(low.y, f32(y));
        for (var x = u32(low.x); f32(x) < high.x && x < size.x; x++) {
            let wx = min(high.x, f32(x + 1u)) - max(low.x, f32(x));
            sum += read_as(textureLoad(source, vec2<u32>(x, y), 0), mode) * (wx * wy);
            area += wx * wy;
        }
    }
    return written_as(sum / area, mode);
}

@fragment fn reduce(@builtin(position) position: vec4<f32>) -> @location(0) vec4<f32> {
    return boxed(position.xy, 0u);
}

@fragment fn reduce_normals(@builtin(position) position: vec4<f32>) -> @location(0) vec4<f32> {
    return boxed(position.xy, 1u);
}

@fragment fn reduce_slopes(@builtin(position) position: vec4<f32>) -> @location(0) vec4<f32> {
    return boxed(position.xy, 2u);
}

// A texel of the next level down that is the smallest, channel by channel,
// of the texels under it, or the largest when `largest`.
fn extreme(position: vec2<f32>, largest: bool) -> vec4<f32> {
    let size = textureDimensions(source);
    let smaller = max(size >> vec2<u32>(1u), vec2<u32>(1u));
    let scale = vec2<f32>(size) / vec2<f32>(smaller);
    let low = floor(position) * scale;
    let high = low + scale;
    var found = textureLoad(source, vec2<u32>(low), 0);
    for (var y = u32(low.y); f32(y) < high.y && y < size.y; y++) {
        for (var x = u32(low.x); f32(x) < high.x && x < size.x; x++) {
            let texel = textureLoad(source, vec2<u32>(x, y), 0);
            found = select(min(found, texel), max(found, texel), largest);
        }
    }
    return found;
}

@fragment fn reduce_min(@builtin(position) position: vec4<f32>) -> @location(0) vec4<f32> {
    return extreme(position.xy, false);
}

@fragment fn reduce_max(@builtin(position) position: vec4<f32>) -> @location(0) vec4<f32> {
    return extreme(position.xy, true);
}

// A texel of a copy padded to whole blocks: the source texel, or past an
// edge the texel at that edge.
@fragment fn pad(@builtin(position) position: vec4<f32>) -> @location(0) vec4<f32> {
    let size = textureDimensions(source);
    return textureLoad(source, min(vec2<u32>(position.xy), size - 1u), 0);
}
";

/// The same two passes for one depth slice of a 3D level.
const SHADER_3D: &str = "
@group(0) @binding(0) var source: texture_3d<f32>;
@group(0) @binding(1) var<uniform> slice: vec4<u32>;

@vertex fn vertex(@builtin(vertex_index) index: u32) -> @builtin(position) vec4<f32> {
    let corner = vec2<f32>(f32((index << 1u) & 2u), f32(index & 2u));
    return vec4<f32>(corner.x * 2.0 - 1.0, 1.0 - corner.y * 2.0, 0.0, 1.0);
}

fn boxed(position: vec2<f32>, mode: u32) -> vec4<f32> {
    let size = textureDimensions(source);
    let smaller = max(size >> vec3<u32>(1u), vec3<u32>(1u));
    let scale = vec3<f32>(size) / vec3<f32>(smaller);
    let low = vec3<f32>(floor(position), f32(slice.x)) * scale;
    let high = low + scale;
    var sum = vec4<f32>(0.0);
    var volume = 0.0;
    for (var z = u32(low.z); f32(z) < high.z && z < size.z; z++) {
        let wz = min(high.z, f32(z + 1u)) - max(low.z, f32(z));
        for (var y = u32(low.y); f32(y) < high.y && y < size.y; y++) {
            let wy = min(high.y, f32(y + 1u)) - max(low.y, f32(y));
            for (var x = u32(low.x); f32(x) < high.x && x < size.x; x++) {
                let wx = min(high.x, f32(x + 1u)) - max(low.x, f32(x));
                sum += read_as(textureLoad(source, vec3<u32>(x, y, z), 0), mode) * (wx * wy * wz);
                volume += wx * wy * wz;
            }
        }
    }
    return written_as(sum / volume, mode);
}

@fragment fn reduce(@builtin(position) position: vec4<f32>) -> @location(0) vec4<f32> {
    return boxed(position.xy, 0u);
}

@fragment fn reduce_normals(@builtin(position) position: vec4<f32>) -> @location(0) vec4<f32> {
    return boxed(position.xy, 1u);
}

@fragment fn reduce_slopes(@builtin(position) position: vec4<f32>) -> @location(0) vec4<f32> {
    return boxed(position.xy, 2u);
}

fn extreme(position: vec2<f32>, largest: bool) -> vec4<f32> {
    let size = textureDimensions(source);
    let smaller = max(size >> vec3<u32>(1u), vec3<u32>(1u));
    let scale = vec3<f32>(size) / vec3<f32>(smaller);
    let low = vec3<f32>(floor(position), f32(slice.x)) * scale;
    let high = low + scale;
    var found = textureLoad(source, vec3<u32>(low), 0);
    for (var z = u32(low.z); f32(z) < high.z && z < size.z; z++) {
        for (var y = u32(low.y); f32(y) < high.y && y < size.y; y++) {
            for (var x = u32(low.x); f32(x) < high.x && x < size.x; x++) {
                let texel = textureLoad(source, vec3<u32>(x, y, z), 0);
                found = select(min(found, texel), max(found, texel), largest);
            }
        }
    }
    return found;
}

@fragment fn reduce_min(@builtin(position) position: vec4<f32>) -> @location(0) vec4<f32> {
    return extreme(position.xy, false);
}

@fragment fn reduce_max(@builtin(position) position: vec4<f32>) -> @location(0) vec4<f32> {
    return extreme(position.xy, true);
}

@fragment fn pad(@builtin(position) position: vec4<f32>) -> @location(0) vec4<f32> {
    let size = textureDimensions(source);
    return textureLoad(source, min(vec3<u32>(vec2<u32>(position.xy), slice.x), size - 1u), 0);
}
";

/// The uniforms, the fullscreen triangle and the edges of the passes that
/// filter a level from level 0, one axis at a time: 0 across, 1 down and 2
/// through the depth. `mode` holds how the pass reads each texel in its
/// low two bits and how it writes its result in the two above them, by
/// `read_as` and `written_as`.
const RESAMPLE_COMMON: &str = "
struct Pass {
    axis: u32,
    kernel: u32,
    edge: u32,
    slice: u32,
    ratio: f32,
    support: f32,
    scale: f32,
    mode: u32,
}

@group(0) @binding(1) var<uniform> m: Pass;

@vertex fn vertex(@builtin(vertex_index) index: u32) -> @builtin(position) vec4<f32> {
    let corner = vec2<f32>(f32((index << 1u) & 2u), f32(index & 2u));
    return vec4<f32>(corner.x * 2.0 - 1.0, 1.0 - corner.y * 2.0, 0.0, 1.0);
}

// Texel `j` of a row `n` texels long by the pass's edge: past the edge, -1
// for Clamp, which leaves it out; the other side for Wrap; for Mirror the
// image mirrored, its edge texel repeated. It returns once, after the
// switch, as FXC refuses a function whose returns are all inside one.
fn edged(j: i32, n: i32) -> i32 {
    var k = select(-1, j, j >= 0 && j < n);
    switch m.edge {
        case 1u: {
            k = ((j % n) + n) % n;
        }
        case 2u: {
            let period = 2 * n;
            let r = ((j % period) + period) % period;
            k = select(r, period - 1 - r, r >= n);
        }
        default: {}
    }
    return k;
}
";

/// A pass over a 2D image: each texel filtered along the pass's axis from
/// the texels within the kernel's reach of where its center lands, as a
/// resize computes it, the weights scaled to sum to 1.
const RESAMPLE_2D: &str = "
@group(0) @binding(0) var source: texture_2d<f32>;

@fragment fn resample(@builtin(position) position: vec4<f32>) -> @location(0) vec4<f32> {
    let at = vec2<i32>(position.xy);
    let dims = vec2<i32>(textureDimensions(source));
    let n = select(dims.y, dims.x, m.axis == 0u);
    let i = select(at.y, at.x, m.axis == 0u);
    let center = (f32(i) + 0.5) * m.ratio;
    var sum = vec4<f32>(0.0);
    var total = 0.0;
    for (var j = i32(floor(center - m.support)); j < i32(ceil(center + m.support)); j++) {
        let k = edged(j, n);
        if (k < 0) {
            continue;
        }
        let w = kernel_weight(m.kernel, (f32(j) + 0.5 - center) / m.scale);
        sum += read_as(textureLoad(source, select(vec2<i32>(at.x, k), vec2<i32>(k, at.y), m.axis == 0u), 0), m.mode & 3u) * w;
        total += w;
    }
    return written_as(sum / total, m.mode >> 2u);
}
";

/// The same pass over a 3D image, for depth slice `m.slice` of the new
/// image: across or down within that slice of the source, or through the
/// source's depth.
const RESAMPLE_3D: &str = "
@group(0) @binding(0) var source: texture_3d<f32>;

@fragment fn resample(@builtin(position) position: vec4<f32>) -> @location(0) vec4<f32> {
    let at = vec2<i32>(position.xy);
    let dims = vec3<i32>(textureDimensions(source));
    var n = dims.z;
    var i = i32(m.slice);
    if (m.axis == 0u) {
        n = dims.x;
        i = at.x;
    } else if (m.axis == 1u) {
        n = dims.y;
        i = at.y;
    }
    let center = (f32(i) + 0.5) * m.ratio;
    var sum = vec4<f32>(0.0);
    var total = 0.0;
    for (var j = i32(floor(center - m.support)); j < i32(ceil(center + m.support)); j++) {
        let k = edged(j, n);
        if (k < 0) {
            continue;
        }
        let w = kernel_weight(m.kernel, (f32(j) + 0.5 - center) / m.scale);
        var texel = vec3<i32>(at, i32(m.slice));
        if (m.axis == 0u) {
            texel.x = k;
        } else if (m.axis == 1u) {
            texel.y = k;
        } else {
            texel.z = k;
        }
        sum += read_as(textureLoad(source, texel, 0), m.mode & 3u) * w;
        total += w;
    }
    return written_as(sum / total, m.mode >> 2u);
}
";

/// Signed BC4 and BC5: for each 4x4 block of each channel, endpoints from
/// the channel's range tried in both of BC4's palettes (eight values, or
/// six with -1 and +1), and each texel given its nearest palette value.
const SIGNED_BC: &str = "
struct Place {
    // Where this layer's blocks start in `blocks`, in words.
    offset: u32,
    // 1 for BC4, 2 for BC5.
    channels: u32,
}

@group(0) @binding(0) var source: texture_2d<f32>;
@group(0) @binding(1) var<storage, read_write> blocks: array<u32>;
@group(0) @binding(2) var<uniform> place: Place;

struct Fit {
    low: u32,
    high: u32,
    error: f32,
}

// The values endpoints r0 and r1, signed bytes, decode to.
fn palette(r0: i32, r1: i32) -> array<f32, 8> {
    let a = f32(r0) / 127.0;
    let b = f32(r1) / 127.0;
    var p: array<f32, 8>;
    p[0] = a;
    p[1] = b;
    if (r0 > r1) {
        for (var i = 1u; i < 7u; i++) {
            p[i + 1u] = (f32(7u - i) * a + f32(i) * b) / 7.0;
        }
    } else {
        for (var i = 1u; i < 5u; i++) {
            p[i + 1u] = (f32(5u - i) * a + f32(i) * b) / 5.0;
        }
        p[6] = -1.0;
        p[7] = 1.0;
    }
    return p;
}

// The block for endpoints r0 and r1 with each value at its nearest
// palette value, and the squared error of it.
fn fit(values: array<f32, 16>, r0: i32, r1: i32) -> Fit {
    var v = values;
    var p = palette(r0, r1);
    var low = (bitcast<u32>(r0) & 0xFFu) | ((bitcast<u32>(r1) & 0xFFu) << 8u);
    var high = 0u;
    var error = 0.0;
    for (var i = 0u; i < 16u; i++) {
        var best = 0u;
        var best_error = 1e9;
        for (var j = 0u; j < 8u; j++) {
            let d = v[i] - p[j];
            if (d * d < best_error) {
                best_error = d * d;
                best = j;
            }
        }
        error += best_error;
        // Texel i's index is bits 16 + 3i to 18 + 3i of the 64-bit block.
        let bit = 16u + 3u * i;
        if (bit + 3u <= 32u) {
            low |= best << bit;
        } else if (bit >= 32u) {
            high |= best << (bit - 32u);
        } else {
            low |= best << bit;
            high |= best >> (32u - bit);
        }
    }
    return Fit(low, high, error);
}

fn encode(values: array<f32, 16>) -> vec2<u32> {
    var v = values;
    var low = 1.0;
    var high = -1.0;
    // The six-value palette holds -1 and +1 itself: its endpoints span the rest.
    var inner_low = 1.0;
    var inner_high = -1.0;
    for (var i = 0u; i < 16u; i++) {
        low = min(low, v[i]);
        high = max(high, v[i]);
        if (v[i] > -1.0 && v[i] < 1.0) {
            inner_low = min(inner_low, v[i]);
            inner_high = max(inner_high, v[i]);
        }
    }
    var best = fit(v, i32(round(high * 127.0)), i32(round(low * 127.0)));
    if (inner_low <= inner_high) {
        let six = fit(v, i32(round(inner_low * 127.0)), i32(round(inner_high * 127.0)));
        if (six.error < best.error) {
            best = six;
        }
    }
    return vec2<u32>(best.low, best.high);
}

@compute @workgroup_size(8, 8)
fn main(@builtin(global_invocation_id) id: vec3<u32>) {
    let across = textureDimensions(source).x / 4u;
    let down = textureDimensions(source).y / 4u;
    if (id.x >= across || id.y >= down) {
        return;
    }
    let block = id.y * across + id.x;
    for (var c = 0u; c < place.channels; c++) {
        var values: array<f32, 16>;
        for (var i = 0u; i < 16u; i++) {
            let texel = textureLoad(source, vec2<u32>(id.x * 4u + i % 4u, id.y * 4u + i / 4u), 0);
            values[i] = clamp(texel[c], -1.0, 1.0);
        }
        let words = encode(values);
        let at = place.offset + (block * place.channels + c) * 2u;
        blocks[at] = words.x;
        blocks[at + 1u] = words.y;
    }
}
";

/// Whether the passes here can read `format`, and write it.
pub(crate) fn float_renderable(core: &CoreRef, format: F) -> bool {
    let float = matches!(format.sample_type(None, Some(core.features)), Some(wgpu::TextureSampleType::Float { .. }));
    float && format_features(core, format).allowed_usages.contains(wgpu::TextureUsages::RENDER_ATTACHMENT)
}

/// A pass's source and target bound and drawn: `entry`, a reduce or `pad`,
/// from the 2D or 3D shader after `prelude`, the constants and functions
/// `normal_constants` writes, into a view of `format`.
struct Passes {
    pipeline: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
    volume: bool,
}

impl Passes {
    fn new(core: &CoreRef, volume: bool, entry: &str, format: F, prelude: &str) -> Passes {
        let module = core.device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Wgpu texture conversion"),
            source: wgpu::ShaderSource::Wgsl(format!("{prelude}{}", if volume { SHADER_3D } else { SHADER_2D }).into()),
        });
        let mut entries = vec![wgpu::BindGroupLayoutEntry {
            binding: 0,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Texture {
                sample_type: wgpu::TextureSampleType::Float { filterable: false },
                view_dimension: if volume { wgpu::TextureViewDimension::D3 } else { wgpu::TextureViewDimension::D2 },
                multisampled: false,
            },
            count: None,
        }];
        if volume {
            entries.push(wgpu::BindGroupLayoutEntry {
                binding: 1,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None },
                count: None,
            });
        }
        let layout = core.device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor { label: Some("Wgpu texture conversion"), entries: &entries });
        let pipeline_layout =
            core.device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some("Wgpu texture conversion"), bind_group_layouts: &[Some(&layout)], immediate_size: 0 });
        let pipeline = core.device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("Wgpu texture conversion"),
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
        Passes { pipeline, layout, volume }
    }

    /// Draws `source` through the pipeline into `target`, at depth slice
    /// `slice` of a 3D target.
    fn draw(&self, core: &CoreRef, encoder: &mut wgpu::CommandEncoder, source: &wgpu::TextureView, target: &wgpu::TextureView, slice: Option<u32>) {
        let uniform = slice.map(|z| uniform(core, &[z, 0, 0, 0]));
        let mut entries = vec![wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(source) }];
        if let Some(buffer) = &uniform {
            entries.push(wgpu::BindGroupEntry { binding: 1, resource: buffer.as_entire_binding() });
        }
        let bind_group = core.device.create_bind_group(&wgpu::BindGroupDescriptor { label: Some("Wgpu texture conversion"), layout: &self.layout, entries: &entries });
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("Wgpu texture conversion"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: target,
                depth_slice: if self.volume { slice } else { None },
                resolve_target: None,
                ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT), store: wgpu::StoreOp::Store },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &bind_group, &[]);
        pass.draw(0..3, 0..1);
    }
}

/// A uniform buffer holding `words`.
fn uniform(core: &CoreRef, words: &[u32; 4]) -> wgpu::Buffer {
    core.device.create_buffer_init(&wgpu::util::BufferInitDescriptor { label: Some("Wgpu texture conversion"), contents: bytemuck::cast_slice(words), usage: wgpu::BufferUsages::UNIFORM })
}

/// A view of one mip level and one array layer, or of one whole level of
/// a 3D texture.
fn level_view(texture: &wgpu::Texture, level: u32, layer: u32, format: Option<F>) -> wgpu::TextureView {
    let volume = texture.dimension() == wgpu::TextureDimension::D3;
    texture.create_view(&wgpu::TextureViewDescriptor {
        label: Some("Wgpu texture conversion"),
        format,
        dimension: Some(if volume { wgpu::TextureViewDimension::D3 } else { wgpu::TextureViewDimension::D2 }),
        usage: None,
        aspect: wgpu::TextureAspect::All,
        base_mip_level: level,
        mip_level_count: Some(1),
        base_array_layer: if volume { 0 } else { layer },
        array_layer_count: Some(1),
    })
}

/// The layers of a texture, or the depth slices of mip level `level` of a
/// 3D one.
fn planes(texture: &wgpu::Texture, level: u32) -> u32 {
    let size = texture.size().mip_level_size(level, texture.dimension());
    size.depth_or_array_layers
}

/// Renders mip levels `first` and on of `texture`, a 2D or 3D texture of
/// a float format with the TextureBinding, RenderAttachment and CopySrc
/// usages, in every layer: by Box, Min or Max each from the level above
/// it, and by the other filters each from level 0, its normals kept unit
/// length as `mips` says; then, with a coverage to keep, scales their
/// alpha as alpha::keep_coverage does.
pub(crate) fn render_mips(core: &CoreRef, texture: &wgpu::Texture, first: u32, mips: MipOptions, stop: Option<&dyn Interrupt>) -> PsResult<()> {
    if !float_renderable(core, texture.format()) {
        return Err(invalid("WgpuMipLevels", format!("mip levels are rendered in {}, which this device does not render into", format_name(texture.format()))));
    }
    let work = if float_renderable(core, F::Rgba32Float) { F::Rgba32Float } else { F::Rgba16Float };
    let prelude = normal_constants(mips.signed, texture.format());
    let commands = core.scoped("WgpuMipLevels", "the mip levels cannot be rendered", || match mips.filter.kernel() {
        Some((code, support)) => filter_levels(core, texture, first, Kernel { code, support, edge: mips.edge.code(), normals: mips.mode() }, work, &prelude),
        None => {
            let entry = match mips.normals {
                None => mips.filter.reduce(),
                Some(NormalMips::Renormalize) => "reduce_normals",
                Some(NormalMips::SlopeSpace) => "reduce_slopes",
            };
            reduce_levels(core, texture, first, entry, &prelude)
        }
    })?;
    core.submit_and_wait(vec![commands], stop)?;
    if let Some(reference) = mips.coverage {
        crate::alpha::keep_coverage(core, texture, first, reference, stop)?;
    }
    Ok(())
}

/// Records mip levels `first` and on of `texture`, each made from the level
/// above it by the pass `entry`, whose shader starts with `prelude`.
fn reduce_levels(core: &CoreRef, texture: &wgpu::Texture, first: u32, entry: &str, prelude: &str) -> wgpu::CommandBuffer {
    let volume = texture.dimension() == wgpu::TextureDimension::D3;
    let passes = Passes::new(core, volume, entry, texture.format(), prelude);
    let mut encoder = core.device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("Wgpu mip levels") });
    for level in first.max(1)..texture.mip_level_count() {
        if volume {
            let source = level_view(texture, level - 1, 0, None);
            let target = level_view(texture, level, 0, None);
            for z in 0..planes(texture, level) {
                passes.draw(core, &mut encoder, &source, &target, Some(z));
            }
        } else {
            for layer in 0..texture.depth_or_array_layers() {
                passes.draw(core, &mut encoder, &level_view(texture, level - 1, layer, None), &level_view(texture, level, layer, None), None);
            }
        }
    }
    encoder.finish()
}

/// A filter of level 0 as the resample passes take it: its kernel, how far
/// it reaches each way in texels of the level, the edge, and how the
/// first pass reads normals and the last writes them, as
/// `MipOptions::mode` numbers it.
#[derive(Clone, Copy)]
struct Kernel {
    code: u32,
    support: f32,
    edge: u32,
    normals: u32,
}

impl Kernel {
    /// The mode of a pass that reads level 0 when `first` and writes the
    /// level when `last`; a pass between them reads and writes the
    /// normals or slopes the first pass made as they are.
    fn mode(self, first: bool, last: bool) -> u32 {
        let read = if first { self.normals } else { 0 };
        let write = if last { self.normals } else { 0 };
        read | (write << 2)
    }
}

/// The axis a resample pass filters along, 0 across, 1 down and 2 through
/// the depth, the source's size along it and the drawn image's, and the
/// depth slice of a 3D image the pass reads or draws.
#[derive(Clone, Copy)]
struct Along {
    axis: u32,
    from: u32,
    to: u32,
    slice: u32,
}

/// What a resample pass reads: a 2D view, or a 3D one.
struct Input<'a> {
    view: &'a wgpu::TextureView,
    volume: bool,
}

/// What a resample pass draws into: a view of `format`, at a depth slice
/// of a 3D one.
struct Output<'a> {
    view: &'a wgpu::TextureView,
    slice: Option<u32>,
    format: F,
}

/// The resample passes' 2D and 3D shaders and layouts, and their pipelines,
/// each built the first time a target format needs it.
struct Resample {
    flat: (wgpu::ShaderModule, wgpu::BindGroupLayout),
    volume: (wgpu::ShaderModule, wgpu::BindGroupLayout),
    built: Vec<((bool, F), wgpu::RenderPipeline)>,
}

impl Resample {
    /// The passes, their shaders after `prelude`, the constants and
    /// functions `normal_constants` writes.
    fn new(core: &CoreRef, prelude: &str) -> Resample {
        let make = |code: &str, dimension: wgpu::TextureViewDimension| {
            let module = core.device.create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("Wgpu mip levels"),
                source: wgpu::ShaderSource::Wgsl(format!("{prelude}{RESAMPLE_COMMON}{KERNELS}{code}").into()),
            });
            let layout = core.device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("Wgpu mip levels"),
                entries: &[
                    wgpu::BindGroupLayoutEntry {
                        binding: 0,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Float { filterable: false }, view_dimension: dimension, multisampled: false },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 1,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None },
                        count: None,
                    },
                ],
            });
            (module, layout)
        };
        Resample { flat: make(RESAMPLE_2D, wgpu::TextureViewDimension::D2), volume: make(RESAMPLE_3D, wgpu::TextureViewDimension::D3), built: Vec::new() }
    }

    fn pipeline(&mut self, core: &CoreRef, volume: bool, format: F) -> wgpu::RenderPipeline {
        if let Some((_, built)) = self.built.iter().find(|(key, _)| *key == (volume, format)) {
            return built.clone();
        }
        let (module, layout) = if volume { &self.volume } else { &self.flat };
        let pipeline_layout = core.device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some("Wgpu mip levels"), bind_group_layouts: &[Some(layout)], immediate_size: 0 });
        let pipeline = core.device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("Wgpu mip levels"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState { module, entry_point: Some("vertex"), compilation_options: Default::default(), buffers: &[] },
            fragment: Some(wgpu::FragmentState {
                module,
                entry_point: Some("resample"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState { format, blend: None, write_mask: wgpu::ColorWrites::ALL })],
            }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });
        self.built.push(((volume, format), pipeline.clone()));
        pipeline
    }

    /// Records a pass that filters `input` by `kernel` `along` an axis into
    /// `output`, as a resize computes each texel, reading and writing each
    /// texel as `mode` says.
    #[allow(clippy::too_many_arguments)]
    fn draw(&mut self, core: &CoreRef, encoder: &mut wgpu::CommandEncoder, input: &Input<'_>, output: &Output<'_>, kernel: Kernel, along: Along, mode: u32) {
        let ratio = along.from as f32 / along.to as f32;
        let scale = ratio.max(1.0);
        let mut bytes = Vec::with_capacity(32);
        for v in [along.axis, kernel.code, kernel.edge, along.slice] {
            bytes.extend(v.to_le_bytes());
        }
        for v in [ratio, kernel.support * scale, scale] {
            bytes.extend(v.to_le_bytes());
        }
        bytes.extend(mode.to_le_bytes());
        let uniform = core.device.create_buffer_init(&wgpu::util::BufferInitDescriptor { label: Some("Wgpu mip levels"), contents: &bytes, usage: wgpu::BufferUsages::UNIFORM });
        let pipeline = self.pipeline(core, input.volume, output.format);
        let layout = if input.volume { &self.volume.1 } else { &self.flat.1 };
        let bind_group = core.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Wgpu mip levels"),
            layout,
            entries: &[wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(input.view) }, wgpu::BindGroupEntry { binding: 1, resource: uniform.as_entire_binding() }],
        });
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("Wgpu mip levels"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: output.view,
                depth_slice: output.slice,
                resolve_target: None,
                ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT), store: wgpu::StoreOp::Store },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        pass.set_pipeline(&pipeline);
        pass.set_bind_group(0, &bind_group, &[]);
        pass.draw(0..3, 0..1);
    }
}

/// An image between two resample passes, of `work` and `size`: 3D when
/// `volume`, and 2D otherwise.
fn between(core: &CoreRef, size: wgpu::Extent3d, volume: bool, work: F) -> wgpu::Texture {
    core.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("Wgpu mip levels"),
        size,
        mip_level_count: 1,
        sample_count: 1,
        dimension: if volume { wgpu::TextureDimension::D3 } else { wgpu::TextureDimension::D2 },
        format: work,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
        view_formats: &[],
    })
}

/// The passes one filtering of mip levels from level 0 records, by
/// `kernel`, through images of `work`.
struct Filtering<'a> {
    core: &'a CoreRef,
    passes: Resample,
    encoder: wgpu::CommandEncoder,
    kernel: Kernel,
    work: F,
}

impl Filtering<'_> {
    /// Records one plane of `input`, `from` texels across and down, filtered
    /// into `output`, `to` texels across and down: down, then across through
    /// an image of `work` when both sizes change. `slice` is the depth slice
    /// of a 3D input the passes read; `first` says the input is level 0,
    /// as the texture stores it, and not an image an earlier pass drew.
    fn plane(&mut self, input: &Input<'_>, from: (u32, u32), to: (u32, u32), output: &Output<'_>, slice: u32, first: bool) {
        let down = Along { axis: 1, from: from.1, to: to.1, slice };
        let across = Along { axis: 0, from: from.0, to: to.0, slice };
        match (from.1 != to.1, from.0 != to.0) {
            (true, true) => {
                let image = between(self.core, wgpu::Extent3d { width: from.0, height: to.1, depth_or_array_layers: 1 }, false, self.work);
                let view = image.create_view(&wgpu::TextureViewDescriptor::default());
                self.passes.draw(self.core, &mut self.encoder, input, &Output { view: &view, slice: None, format: self.work }, self.kernel, down, self.kernel.mode(first, false));
                self.passes.draw(self.core, &mut self.encoder, &Input { view: &view, volume: false }, output, self.kernel, Along { slice: 0, ..across }, self.kernel.mode(false, true));
            }
            (true, false) => self.passes.draw(self.core, &mut self.encoder, input, output, self.kernel, down, self.kernel.mode(first, true)),
            (false, true) => self.passes.draw(self.core, &mut self.encoder, input, output, self.kernel, across, self.kernel.mode(first, true)),
            (false, false) => {}
        }
    }
}

/// Records mip levels `first` and on of `texture`, each filtered from level
/// 0 by `kernel` one axis at a time, through images of `work`: down, then
/// across, in each layer of a 2D texture; through the depth, then down and
/// across in each depth slice, for a 3D one. An axis whose size a level
/// keeps is not filtered. The passes' shaders start with `prelude`.
fn filter_levels(core: &CoreRef, texture: &wgpu::Texture, first: u32, kernel: Kernel, work: F, prelude: &str) -> wgpu::CommandBuffer {
    let encoder = core.device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("Wgpu mip levels") });
    let mut filtering = Filtering { core, passes: Resample::new(core, prelude), encoder, kernel, work };
    let volume = texture.dimension() == wgpu::TextureDimension::D3;
    let top = texture.size().mip_level_size(0, texture.dimension());
    let from = (top.width, top.height);
    for level in first.max(1)..texture.mip_level_count() {
        let size = texture.size().mip_level_size(level, texture.dimension());
        let to = (size.width, size.height);
        if !volume {
            for layer in 0..texture.depth_or_array_layers() {
                let source = level_view(texture, 0, layer, None);
                let target = level_view(texture, level, layer, None);
                filtering.plane(&Input { view: &source, volume: false }, from, to, &Output { view: &target, slice: None, format: texture.format() }, 0, true);
            }
            continue;
        }
        let level_zero = level_view(texture, 0, 0, None);
        let target = level_view(texture, level, 0, None);
        let planar = from != to;
        // The depth filtered first, into an image the planes are then
        // filtered from, or into the level itself when its planes keep
        // their size.
        let deep = (top.depth_or_array_layers != size.depth_or_array_layers).then(|| {
            let image = between(core, wgpu::Extent3d { width: top.width, height: top.height, depth_or_array_layers: size.depth_or_array_layers }, true, work);
            let view = image.create_view(&wgpu::TextureViewDescriptor::default());
            for z in 0..size.depth_or_array_layers {
                let along = Along { axis: 2, from: top.depth_or_array_layers, to: size.depth_or_array_layers, slice: z };
                let output = if planar { Output { view: &view, slice: Some(z), format: work } } else { Output { view: &target, slice: Some(z), format: texture.format() } };
                filtering.passes.draw(core, &mut filtering.encoder, &Input { view: &level_zero, volume: true }, &output, kernel, along, kernel.mode(true, !planar));
            }
            view
        });
        if planar {
            let source = deep.as_ref().unwrap_or(&level_zero);
            for z in 0..size.depth_or_array_layers {
                filtering.plane(&Input { view: source, volume: true }, from, to, &Output { view: &target, slice: Some(z), format: texture.format() }, z, deep.is_none());
            }
        }
    }
    filtering.encoder.finish()
}

/// A texture in `render`, the float format `file`'s texels render in, with
/// `count` mip levels: the first `have` drawn from `file`'s own levels,
/// which converts them, and the rest rendered as `mips` says. A 1D file
/// renders as a 2D texture one texel high, as a 1D texture cannot be
/// rendered into.
pub(crate) fn float_levels(core: &CoreRef, file: &TextureFile, render: F, count: u32, have: u32, mips: MipOptions, stop: Option<&dyn Interrupt>) -> PsResult<wgpu::Texture> {
    let volume = file.dimension == wgpu::TextureDimension::D3;
    let dimension = if volume { wgpu::TextureDimension::D3 } else { wgpu::TextureDimension::D2 };
    let size = wgpu::Extent3d { width: file.width, height: file.height, depth_or_array_layers: if volume { file.depth } else { file.layers } };
    let made = |format: F, levels: u32, usage: wgpu::TextureUsages| {
        core.scoped("WgpuConvert", "the texture cannot be converted", || {
            core.device.create_texture(&wgpu::TextureDescriptor { label: Some("Wgpu conversion"), size, mip_level_count: levels, sample_count: 1, dimension, format, usage, view_formats: &[] })
        })
    };
    let source = made(file.format, have, wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST)?;
    for mip in 0..have as usize {
        let len = file.layer_len(mip);
        for layer in 0..file.layers as usize {
            write_texels(core, &source, mip as u32, layer as u32, wgpu::TextureAspect::All, &file.levels[mip][layer * len..(layer + 1) * len])?;
        }
    }
    let target = made(render, count, wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC)?;
    let commands = core.scoped("WgpuConvert", "the texture cannot be converted", || {
        let passes = Passes::new(core, volume, "pad", render, &normal_constants(false, render));
        let mut encoder = core.device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("Wgpu conversion") });
        for level in 0..have {
            if volume {
                let (from, to) = (level_view(&source, level, 0, None), level_view(&target, level, 0, None));
                for z in 0..planes(&target, level) {
                    passes.draw(core, &mut encoder, &from, &to, Some(z));
                }
            } else {
                for layer in 0..file.layers {
                    passes.draw(core, &mut encoder, &level_view(&source, level, layer, None), &level_view(&target, level, layer, None), None);
                }
            }
        }
        encoder.finish()
    })?;
    core.submit(vec![commands], stop)?;
    render_mips(core, &target, have, mips, stop)?;
    Ok(target)
}

/// The format block_compression and the signed encoder read for `target`,
/// and the format a padded copy is rendered through, which encodes sRGB
/// where `target` stores sRGB.
fn work_formats(target: F) -> (F, F) {
    match target {
        F::Bc6hRgbUfloat | F::Bc4RSnorm | F::Bc5RgSnorm => (F::Rgba16Float, F::Rgba16Float),
        _ if target.is_srgb() => (F::Rgba8Unorm, F::Rgba8UnormSrgb),
        _ => (F::Rgba8Unorm, F::Rgba8Unorm),
    }
}

/// block_compression 0.11.0's compute shaders, kept with its license in
/// vendor/block_compression with the changes its CHANGES.md describes.
const BC1_TO_5: &str = include_str!("../vendor/block_compression/bc1_to_5.wgsl");
const BC6H: &str = include_str!("../vendor/block_compression/bc6h.wgsl");
const BC7: &str = include_str!("../vendor/block_compression/bc7.wgsl");

/// Each encoder `encode_bc` dispatches: block_compression's BC1 to BC7 and
/// BC6H, and the module's signed BC4 and BC5. Each has a compute pipeline
/// of its own on a device, built the first time it is used.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Encoder {
    Bc1,
    Bc2,
    Bc3,
    Bc4,
    Bc5,
    Bc6h,
    Bc7,
    Signed,
}

/// The number of encoders, the slots a device keeps their pipelines in.
pub(crate) const ENCODERS: usize = 8;

impl Encoder {
    fn source(self) -> &'static str {
        match self {
            Encoder::Bc1 | Encoder::Bc2 | Encoder::Bc3 | Encoder::Bc4 | Encoder::Bc5 => BC1_TO_5,
            Encoder::Bc6h => BC6H,
            Encoder::Bc7 => BC7,
            Encoder::Signed => SIGNED_BC,
        }
    }

    /// The entry points of the encoder's passes, dispatched in this order
    /// over the same blocks: one for each encoder but BC7, which runs in
    /// four, so Direct3D 12's software adapter on Arm64 can compile it.
    fn entry_points(self) -> &'static [&'static str] {
        match self {
            Encoder::Bc1 => &["compress_bc1"],
            Encoder::Bc2 => &["compress_bc2"],
            Encoder::Bc3 => &["compress_bc3"],
            Encoder::Bc4 => &["compress_bc4"],
            Encoder::Bc5 => &["compress_bc5"],
            Encoder::Bc6h => &["compress_bc6h"],
            Encoder::Bc7 => &["compress_bc7_mode456", "compress_bc7_rank", "compress_bc7_search", "compress_bc7_finish"],
            Encoder::Signed => &["main"],
        }
    }

    /// The bytes of the settings the encoder reads at binding 3, for BC6H
    /// and BC7.
    fn settings_size(self) -> Option<u64> {
        match self {
            Encoder::Bc6h => Some(size_of::<BC6HSettings>() as u64),
            Encoder::Bc7 => Some(size_of::<BC7Settings>() as u64),
            _ => None,
        }
    }
}

/// An encoder's compute pipelines, one for each of its passes, and the
/// layout of the bind group they share: the source texture, the blocks it
/// writes, its uniforms, for BC6H and BC7 their settings, and for BC7 the
/// state its passes keep for each block.
pub(crate) struct EncoderPipeline {
    pipelines: Vec<wgpu::ComputePipeline>,
    layout: wgpu::BindGroupLayout,
}

/// The 4-byte words of state BC7's passes keep for each block, laid out as
/// bc7.wgsl reads them: the partitions ranked for modes 1 and 3 and for
/// mode 7, 64 each; five partitioned searches of 29 words; and modes 4 to
/// 6's error and block.
const BC7_RANKED_WORDS: u64 = 2 * 64;
const BC7_FOUND_WORDS: u64 = 5 * 29;
const BC7_CHOSEN_WORDS: u64 = 5;

/// The most bytes one of BC7's state buffers takes, so a large level is
/// encoded in slices of rows instead of holding state for every block.
const BC7_STATE_BUDGET: u64 = 32 << 20;

/// The buffers BC7's passes keep each block's state in, and the rows of
/// blocks one dispatch of them covers.
struct Bc7State {
    ranked: wgpu::Buffer,
    found: wgpu::Buffer,
    chosen: wgpu::Buffer,
    rows: u32,
}

impl Bc7State {
    /// State for slices of rows `blocks_across` blocks wide: the `rows` a
    /// level has, or as many as fit `budget` bytes a buffer and the
    /// device's largest storage binding.
    fn new(core: &CoreRef, blocks_across: u32, rows: u32, budget: u64) -> PsResult<Self> {
        let budget = budget.min(core.limits.max_storage_buffer_binding_size).min(core.limits.max_buffer_size);
        let per_row = blocks_across as u64 * BC7_FOUND_WORDS * 4;
        let fit = u32::try_from((budget / per_row).max(1)).map_err(|e| invalid("WgpuCompress", format!("a level is too large to encode: {e}")))?;
        let rows = rows.min(fit).max(1);
        let blocks = rows as u64 * blocks_across as u64;
        let state = |words: u64, label: &str| buffer::create_raw(core, blocks * words * 4, wgpu::BufferUsages::STORAGE, label, None);
        Ok(Bc7State {
            ranked: state(BC7_RANKED_WORDS, "Wgpu BC7 ranked partitions")?,
            found: state(BC7_FOUND_WORDS, "Wgpu BC7 partition searches")?,
            chosen: state(BC7_CHOSEN_WORDS, "Wgpu BC7 modes 4 to 6")?,
            rows,
        })
    }
}

/// `encoder`'s pipeline on `core`'s device: the one a call before built, or
/// one built now and kept for the calls after. Two calls that both find
/// none build one each, and the later one is kept.
fn encoder_pipeline(core: &CoreRef, encoder: Encoder) -> PsResult<Arc<EncoderPipeline>> {
    let slot = &core.bc_encoders[encoder as usize];
    if let Some(built) = slot.load_full() {
        return Ok(built);
    }
    let built = Arc::new(core.scoped("WgpuCompress", "the encoder cannot be built", || {
        let module = core.device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("Wgpu block compression"), source: wgpu::ShaderSource::Wgsl(encoder.source().into()) });
        let buffer = |binding: u32, ty: wgpu::BufferBindingType| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::COMPUTE,
            ty: wgpu::BindingType::Buffer { ty, has_dynamic_offset: false, min_binding_size: None },
            count: None,
        };
        let mut entries = vec![
            wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::COMPUTE,
                ty: wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Float { filterable: true }, view_dimension: wgpu::TextureViewDimension::D2, multisampled: false },
                count: None,
            },
            buffer(1, wgpu::BufferBindingType::Storage { read_only: false }),
            buffer(2, wgpu::BufferBindingType::Uniform),
        ];
        if encoder.settings_size().is_some() {
            entries.push(buffer(3, wgpu::BufferBindingType::Storage { read_only: true }));
        }
        if encoder == Encoder::Bc7 {
            for binding in 4..=6 {
                entries.push(buffer(binding, wgpu::BufferBindingType::Storage { read_only: false }));
            }
        }
        let layout = core.device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor { label: Some("Wgpu block compression"), entries: &entries });
        let pipeline_layout =
            core.device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some("Wgpu block compression"), bind_group_layouts: &[Some(&layout)], immediate_size: 0 });
        let pipelines = encoder
            .entry_points()
            .iter()
            .map(|entry| {
                core.device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                    label: Some("Wgpu block compression"),
                    layout: Some(&pipeline_layout),
                    module: &module,
                    entry_point: Some(entry),
                    compilation_options: Default::default(),
                    cache: None,
                })
            })
            .collect();
        EncoderPipeline { pipelines, layout }
    })?);
    slot.store(Some(built.clone()));
    Ok(built)
}

/// One of block_compression's presets of settings `S`, and its name.
type Preset<S> = (fn() -> S, &'static str);

/// An encoder, the bytes of the settings it reads, and the name of their
/// preset.
type Choice = (Encoder, Option<Vec<u8>>, Option<&'static str>);

/// The encoder `target` takes, the bytes of the settings it reads, and
/// the name of their preset. `effort`, 0 to 10, picks the preset: for BC7
/// block_compression's opaque presets when every texel of the source has
/// full alpha and its alpha presets otherwise, from ultra fast to slow,
/// and for BC6H from very fast to very slow; basic when omitted. BC1 to
/// BC5 and signed BC4 and BC5 have a single setting.
fn variant(target: F, opaque: bool, effort: Option<u32>) -> PsResult<Choice> {
    // The preset `effort` reaches past `cuts`, or `basic` when it is omitted.
    let tier = |cuts: [u32; 4], basic: usize| effort.map_or(basic, |e| cuts.iter().filter(|&&cut| e >= cut).count());
    Ok(match target {
        F::Bc1RgbaUnorm | F::Bc1RgbaUnormSrgb => (Encoder::Bc1, None, None),
        F::Bc2RgbaUnorm | F::Bc2RgbaUnormSrgb => (Encoder::Bc2, None, None),
        F::Bc3RgbaUnorm | F::Bc3RgbaUnormSrgb => (Encoder::Bc3, None, None),
        F::Bc4RUnorm => (Encoder::Bc4, None, None),
        F::Bc5RgUnorm => (Encoder::Bc5, None, None),
        F::Bc7RgbaUnorm | F::Bc7RgbaUnormSrgb => {
            let presets: [Preset<BC7Settings>; 5] = if opaque {
                [
                    (BC7Settings::opaque_ultra_fast, "BC7 opaque ultra fast"),
                    (BC7Settings::opaque_very_fast, "BC7 opaque very fast"),
                    (BC7Settings::opaque_fast, "BC7 opaque fast"),
                    (BC7Settings::opaque_basic, "BC7 opaque basic"),
                    (BC7Settings::opaque_slow, "BC7 opaque slow"),
                ]
            } else {
                [
                    (BC7Settings::alpha_ultrafast, "BC7 alpha ultra fast"),
                    (BC7Settings::alpha_very_fast, "BC7 alpha very fast"),
                    (BC7Settings::alpha_fast, "BC7 alpha fast"),
                    (BC7Settings::alpha_basic, "BC7 alpha basic"),
                    (BC7Settings::alpha_slow, "BC7 alpha slow"),
                ]
            };
            let (settings, name) = presets[tier([2, 4, 6, 8], 3)];
            (Encoder::Bc7, Some(bytemuck::bytes_of(&settings()).to_vec()), Some(name))
        }
        F::Bc6hRgbUfloat => {
            let presets: [Preset<BC6HSettings>; 5] = [
                (BC6HSettings::very_fast, "BC6H very fast"),
                (BC6HSettings::fast, "BC6H fast"),
                (BC6HSettings::basic, "BC6H basic"),
                (BC6HSettings::slow, "BC6H slow"),
                (BC6HSettings::very_slow, "BC6H very slow"),
            ];
            let (settings, name) = presets[tier([2, 4, 7, 9], 2)];
            (Encoder::Bc6h, Some(bytemuck::bytes_of(&settings()).to_vec()), Some(name))
        }
        F::Bc4RSnorm | F::Bc5RgSnorm => (Encoder::Signed, None, None),
        F::Bc6hRgbFloat => {
            return Err(invalid("WgpuCompressFormat", "the module encodes BC6H unsigned only: give Bc6hRgbUfloat, for colors of zero and above"));
        }
        other => {
            return Err(invalid(
                "WgpuCompressFormat",
                format!(
                    "{} is not a format the module encodes on the GPU; it encodes Bc1RgbaUnorm, Bc2RgbaUnorm, Bc3RgbaUnorm and Bc7RgbaUnorm and their Srgb forms, Bc4RUnorm, Bc4RSnorm, Bc5RgUnorm, Bc5RgSnorm and Bc6hRgbUfloat, and ASTC and ETC2 textures come from the Basis Universal formats, such as BasisAstc4x4 and BasisEtc1s",
                    format_name(other)
                ),
            ));
        }
    })
}

/// Refuses a block-compressed `target` the module does not encode.
pub(crate) fn check_encodable(target: F) -> PsResult<()> {
    variant(target, true, None).map(|_| ())
}

/// Refuses `effort` for a BC format with a single setting.
pub(crate) fn check_effort(target: F, effort: Option<u32>) -> PsResult<()> {
    let presets = matches!(target, F::Bc7RgbaUnorm | F::Bc7RgbaUnormSrgb | F::Bc6hRgbUfloat);
    match effort {
        Some(_) if !presets => Err(invalid("WgpuEffort", format!("-Effort applies to Bc7RgbaUnorm, Bc7RgbaUnormSrgb and Bc6hRgbUfloat, and {} has a single setting", format_name(target)))),
        Some(effort) if effort > 10 => Err(invalid("WgpuEffort", format!("-Effort runs from 0 to 10, not {effort}"))),
        _ => Ok(()),
    }
}

/// A copy of mip level `level` of `texture` with one 2D layer for each of
/// its layers (or depth slices), in `work` and rendered through
/// `render`, its size rounded up to whole 4x4 blocks with the edge texels
/// repeated past the edges.
fn padded(core: &CoreRef, texture: &wgpu::Texture, level: u32, work: F, render: F, stop: Option<&dyn Interrupt>) -> PsResult<wgpu::Texture> {
    let size = texture.size().mip_level_size(level, texture.dimension());
    let layers = planes(texture, level);
    let volume = texture.dimension() == wgpu::TextureDimension::D3;
    let copy = core.scoped("WgpuCompress", "the texture cannot be prepared for encoding", || {
        core.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("Wgpu encoding source"),
            size: wgpu::Extent3d { width: size.width.next_multiple_of(4), height: size.height.next_multiple_of(4), depth_or_array_layers: layers },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: work,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: if render == work { &[] } else { std::slice::from_ref(&render) },
        })
    })?;
    let commands = core.scoped("WgpuCompress", "the texture cannot be prepared for encoding", || {
        let passes = Passes::new(core, volume, "pad", render, &normal_constants(false, render));
        let mut encoder = core.device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("Wgpu encoding source") });
        let whole = level_view(texture, level, 0, None);
        for layer in 0..layers {
            let target = level_view(&copy, 0, layer, Some(render));
            if volume {
                passes.draw(core, &mut encoder, &whole, &target, Some(layer));
            } else {
                passes.draw(core, &mut encoder, &level_view(texture, level, layer, None), &target, None);
            }
        }
        encoder.finish()
    })?;
    core.submit(vec![commands], stop)?;
    Ok(copy)
}

/// Mip levels `first` and on of `texture`, every layer, encoded in
/// `target`, a BC format, on the GPU, and what encoded them. `effort`
/// picks the preset of BC6H and BC7, and `opaque`, which says every texel
/// of the source has full alpha, picks BC7's opaque presets.
pub(crate) fn encode_bc(
    core: &CoreRef,
    texture: &wgpu::Texture,
    target: F,
    effort: Option<u32>,
    opaque: bool,
    first: u32,
    stop: Option<&dyn Interrupt>,
) -> PsResult<(Vec<Vec<u8>>, String)> {
    encode_bc_within(core, texture, target, effort, opaque, first, stop, BC7_STATE_BUDGET)
}

/// `encode_bc` with BC7's state held to `state_budget` bytes a buffer,
/// which sets how many rows of blocks one dispatch of its passes covers.
#[allow(clippy::too_many_arguments)]
fn encode_bc_within(
    core: &CoreRef,
    texture: &wgpu::Texture,
    target: F,
    effort: Option<u32>,
    opaque: bool,
    first: u32,
    stop: Option<&dyn Interrupt>,
    state_budget: u64,
) -> PsResult<(Vec<Vec<u8>>, String)> {
    check_effort(target, effort)?;
    let (encoder, settings, preset) = variant(target, opaque, effort)?;
    let by = match (encoder, preset) {
        (Encoder::Signed, _) => "the module's signed BC4 and BC5 encoder".to_string(),
        (_, Some(preset)) => format!("block_compression's {preset} settings"),
        (_, None) => "block_compression".to_string(),
    };
    if !float_renderable(core, texture.format()) || texture.format().is_compressed() {
        return Err(invalid("WgpuCompress", format!("a {} texture is read as floating-point colors to encode it, which this device cannot do", format_name(texture.format()))));
    }
    if !texture.usage().contains(wgpu::TextureUsages::TEXTURE_BINDING) {
        return Err(invalid("WgpuCompress", "encoding a texture reads it in a shader: it needs TextureBinding among its usages"));
    }
    let block = target.block_copy_size(None).ok_or_else(|| invalid("WgpuCompress", format!("{} has no block layout", format_name(target))))? as u64;
    let channels = if target == F::Bc5RgSnorm { 2 } else { 1 };
    let (work, render) = work_formats(target);
    let dimension = texture.dimension();
    let size = texture.size();
    let pipeline = encoder_pipeline(core, encoder)?;
    let settings = settings.map(|bytes| {
        core.device.create_buffer_init(&wgpu::util::BufferInitDescriptor { label: Some("Wgpu block compression"), contents: &bytes, usage: wgpu::BufferUsages::STORAGE })
    });
    // BC7's passes keep each block's state in buffers sized for slices of
    // rows of the largest level, the first one encoded.
    let largest = size.mip_level_size(first, dimension);
    let state = if encoder == Encoder::Bc7 { Some(Bc7State::new(core, largest.width.div_ceil(4), largest.height.div_ceil(4), state_budget)?) } else { None };
    let mut levels = Vec::with_capacity(texture.mip_level_count().saturating_sub(first) as usize);
    for level in first..texture.mip_level_count() {
        let extent = size.mip_level_size(level, dimension);
        let layers = planes(texture, level);
        let layer_bytes = extent.width.div_ceil(4) as u64 * extent.height.div_ceil(4) as u64 * block;
        let total = layer_bytes * layers as u64;
        let copy = padded(core, texture, level, work, render, stop)?;
        let out = buffer::create_raw(core, total, wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC, "Wgpu encoded blocks", None)?;
        let (pw, ph) = (extent.width.next_multiple_of(4), extent.height.next_multiple_of(4));
        let commands = core.scoped("WgpuCompress", "the texture cannot be encoded", || -> PsResult<wgpu::CommandBuffer> {
            let (across, rows) = (pw / 4, ph / 4);
            let slice = state.as_ref().map_or(rows, |s| s.rows);
            let mut groups = Vec::with_capacity(layers as usize);
            for layer in 0..layers {
                let mut row = 0;
                while row < rows {
                    let count = slice.min(rows - row);
                    let first_block = layer_bytes * layer as u64 + row as u64 * across as u64 * block;
                    let words = u32::try_from(first_block / 4).map_err(|e| invalid("WgpuCompress", format!("a level is too large to encode in one buffer: {e}")))?;
                    // block_compression's uniforms are the width and height of
                    // the rows a dispatch encodes, its first row of texels and
                    // where its blocks start in words; the signed encoder's
                    // are where they start and how many channels it encodes.
                    let uniforms = if encoder == Encoder::Signed { [words, channels, 0, 0] } else { [pw, count * 4, row * 4, words] };
                    let uniforms = uniform(core, &uniforms);
                    let view = level_view(&copy, 0, layer, Some(work));
                    let mut entries = vec![
                        wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&view) },
                        wgpu::BindGroupEntry { binding: 1, resource: out.as_entire_binding() },
                        wgpu::BindGroupEntry { binding: 2, resource: uniforms.as_entire_binding() },
                    ];
                    if let Some(settings) = &settings {
                        entries.push(wgpu::BindGroupEntry { binding: 3, resource: settings.as_entire_binding() });
                    }
                    if let Some(state) = &state {
                        entries.push(wgpu::BindGroupEntry { binding: 4, resource: state.ranked.as_entire_binding() });
                        entries.push(wgpu::BindGroupEntry { binding: 5, resource: state.found.as_entire_binding() });
                        entries.push(wgpu::BindGroupEntry { binding: 6, resource: state.chosen.as_entire_binding() });
                    }
                    let group = core.device.create_bind_group(&wgpu::BindGroupDescriptor { label: Some("Wgpu block compression"), layout: &pipeline.layout, entries: &entries });
                    groups.push((group, count));
                    row += count;
                }
            }
            let mut commands = core.device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("Wgpu block compression") });
            {
                let mut pass = commands.begin_compute_pass(&wgpu::ComputePassDescriptor { label: Some("Wgpu block compression"), timestamp_writes: None });
                for (group, count) in &groups {
                    pass.set_bind_group(0, group, &[]);
                    // Each pass runs over the rows before the next starts.
                    for each in &pipeline.pipelines {
                        pass.set_pipeline(each);
                        pass.dispatch_workgroups(across.div_ceil(8), count.div_ceil(8), 1);
                    }
                }
            }
            Ok(commands.finish())
        })??;
        core.submit_and_wait(vec![commands], stop)?;
        levels.push(buffer::read_bytes(core, &out, 0, total, stop)?);
    }
    Ok((levels, by))
}

#[cfg(test)]
mod tests {
    use super::*;
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

    fn texture(core: &CoreRef, width: u32, height: u32, levels: u32, format: F, texels: &[u8]) -> wgpu::Texture {
        let texture = core.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("test"),
            size: wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
            mip_level_count: levels,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        crate::texture::write_texels(core, &texture, 0, 0, wgpu::TextureAspect::All, texels).expect("written");
        texture
    }

    /// A 3x3 level's 1x1 mip covers all nine texels, a third of each
    /// across and down, so it is their average: a box filter loses no
    /// texel of an odd size.
    #[test]
    fn renders_a_mip_level_as_the_average_of_the_texels_under_it() {
        let Some(core) = core() else { return };
        let values: Vec<f32> = (1..=9).map(|v| v as f32).collect();
        let texels: Vec<u8> = values.iter().flat_map(|v| bytemuck::bytes_of(&[*v, 0.0f32, 0.0, 1.0]).to_vec()).collect();
        let texture = texture(&core, 3, 3, 2, F::Rgba32Float, &texels);
        render_mips(&core, &texture, 1, MipOptions::default(), None).expect("rendered");
        let level: Vec<f32> = bytemuck::pod_collect_to_vec(&crate::texture::read_texels(&core, &texture, 1, 0, wgpu::TextureAspect::All, None).expect("read"));
        assert!((level[0] - 5.0).abs() < 1e-4 && level[3] == 1.0, "{level:?}");
    }

    /// Signed BC5 of a ramp from -1 to +1 decodes, by bcdec_rs, within half
    /// a step of either palette across each block's range (the six-value
    /// palette's steps are the wider, a fifth of it), plus the rounding of
    /// the endpoints to signed bytes.
    #[test]
    fn encodes_signed_bc5_that_decodes_close_to_its_source() {
        let Some(core) = core() else { return };
        let (width, height) = (8u32, 4u32);
        let ramp = |i: usize| (i as f32 / 31.0) * 2.0 - 1.0;
        let texels: Vec<u8> = (0..(width * height) as usize)
            .flat_map(|i| [ramp(i), -ramp(i), 0.0, 1.0].map(half::f16::from_f32).iter().flat_map(|h| h.to_le_bytes()).collect::<Vec<u8>>())
            .collect();
        let source = texture(&core, width, height, 1, F::Rgba16Float, &texels);
        let (levels, by) = encode_bc(&core, &source, F::Bc5RgSnorm, None, true, 0, None).expect("encoded");
        assert_eq!(by, "the module's signed BC4 and BC5 encoder");
        let blocks = &levels[0];
        assert_eq!(blocks.len(), 2 * 16);
        for (b, block) in blocks.as_chunks::<16>().0.iter().enumerate() {
            let mut rg = [0u8; 32];
            bcdec_rs::bc5(block, &mut rg, 8, true);
            let at = |x: usize, y: usize| y * width as usize + b * 4 + x;
            let range = ramp(at(3, 3)) - ramp(at(0, 0));
            let bound = range / 10.0 + 2.0 / 127.0;
            for y in 0..4 {
                for x in 0..4 {
                    let expected = ramp(at(x, y));
                    let r = rg[(y * 4 + x) * 2] as i8 as f32 / 127.0;
                    let g = rg[(y * 4 + x) * 2 + 1] as i8 as f32 / 127.0;
                    assert!((r - expected).abs() <= bound && (g + expected).abs() <= bound, "block {b} texel ({x}, {y}): {r} {g} for {expected}, bound {bound}");
                }
            }
        }
    }

    /// BC7's passes keep each block's state in buffers that hold a slice of
    /// rows at a time. Two levels encoded in slices of three rows, the last
    /// of each level shorter, write the same blocks as one dispatch over
    /// every row, with the alpha basic settings that try every mode.
    #[test]
    fn encodes_bc7_in_slices_of_rows_as_in_one() {
        let Some(core) = core() else { return };
        let texels: Vec<u8> = (0..48u32).flat_map(|y| (0..64u32).flat_map(move |x| [(x * 4) as u8, (y * 5) as u8, ((x * y) % 256) as u8, (255 - (x + y) % 64) as u8])).collect();
        let source = texture(&core, 64, 48, 2, F::Rgba8Unorm, &texels);
        let (whole, by) = encode_bc(&core, &source, F::Bc7RgbaUnorm, Some(6), false, 0, None).expect("encoded in one dispatch a level");
        assert_eq!(by, "block_compression's BC7 alpha basic settings");
        assert_eq!(whole[0].len(), 16 * 12 * 16);
        assert!(whole[0].iter().any(|&b| b != 0), "the blocks are written");
        let three_rows = 3 * 16 * BC7_FOUND_WORDS * 4;
        let (sliced, _) = encode_bc_within(&core, &source, F::Bc7RgbaUnorm, Some(6), false, 0, None, three_rows).expect("encoded in slices");
        assert_eq!(sliced, whole);
    }

    /// An opaque picture's BC7 copy decodes with alpha 255 in every texel,
    /// with each of the five opaque presets: mode 6, which shares one p-bit
    /// among an endpoint's four channels, keeps the p-bit with which alpha
    /// 255 decodes as 255. The picture is a sky's smooth gradient, which
    /// the encoder gives mode 6.
    #[test]
    fn keeps_an_opaque_pictures_alpha_in_bc7() {
        let Some(core) = core() else { return };
        let (width, height) = (64u32, 32u32);
        let texels: Vec<u8> = (0..height).flat_map(|y| (0..width).flat_map(move |x| [(60 + y * 3 + x / 8) as u8, (120 + y * 2) as u8, (230 - y / 2) as u8, 255])).collect();
        let source = texture(&core, width, height, 1, F::Rgba8Unorm, &texels);
        for effort in [0, 2, 4, 6, 8] {
            let (levels, by) = encode_bc(&core, &source, F::Bc7RgbaUnorm, Some(effort), true, 0, None).expect("encoded");
            assert!(by.contains("BC7 opaque"), "{by}");
            let file = TextureFile { dimension: wgpu::TextureDimension::D2, width, height, depth: 1, layers: 1, cube: false, format: F::Bc7RgbaUnorm, levels };
            let decoded = crate::texfile::decode_for(file, wgpu::Features::empty(), &mut crate::texfile::Said::default()).expect("decoded");
            let below = decoded.levels[0].as_chunks::<4>().0.iter().filter(|t| t[3] != 255).count();
            assert_eq!(below, 0, "{by}: {below} of {} texels decode with alpha below 255", width * height);
        }
    }

    /// A 6x6 level encodes in BC7 block for block as the 8x8 level its edge
    /// texels pad it to, and decodes far nearer its source than a flat
    /// level of its source's mean color is. How near BC7 comes depends on
    /// the GPU running the encoder, so the test compares rather than
    /// bounds.
    #[test]
    fn encodes_bc7_from_a_level_that_is_not_whole_blocks() {
        let Some(core) = core() else { return };
        let texel = |x: u32, y: u32| {
            let i = y * 6 + x;
            [(i * 7) as u8, 200, (255 - i * 7) as u8, 255]
        };
        let texels: Vec<u8> = (0..6).flat_map(|y| (0..6).flat_map(move |x| texel(x, y))).collect();
        let padded: Vec<u8> = (0..8).flat_map(|y: u32| (0..8).flat_map(move |x: u32| texel(x.min(5), y.min(5)))).collect();
        let (levels, by) = encode_bc(&core, &texture(&core, 6, 6, 1, F::Rgba8Unorm, &texels), F::Bc7RgbaUnorm, Some(0), true, 0, None).expect("encoded");
        assert_eq!(by, "block_compression's BC7 opaque ultra fast settings");
        assert_eq!(levels[0].len(), 4 * 16);
        let (whole, _) = encode_bc(&core, &texture(&core, 8, 8, 1, F::Rgba8Unorm, &padded), F::Bc7RgbaUnorm, Some(0), true, 0, None).expect("encoded");
        assert_eq!(levels, whole, "the 6x6 level encodes as its edge texels pad it");
        let blocks = levels[0].clone();
        let file = TextureFile { dimension: wgpu::TextureDimension::D2, width: 6, height: 6, depth: 1, layers: 1, cube: false, format: F::Bc7RgbaUnorm, levels };
        let decoded = crate::texfile::decode_for(file, wgpu::Features::empty(), &mut crate::texfile::Said::default()).expect("decoded");
        let distance = |a: &[u8], b: &[u8]| a.iter().zip(b).map(|(x, y)| f64::from(x.abs_diff(*y))).sum::<f64>();
        let mean: Vec<u8> = (0..4).map(|c| (texels.iter().skip(c).step_by(4).map(|&v| f64::from(v)).sum::<f64>() / 36.0).round() as u8).collect();
        let flat = mean.repeat(36);
        let (encoded, plain) = (distance(&decoded.levels[0], &texels), distance(&flat, &texels));
        // Each block's mode, its bytes, how far it decodes from the texels it
        // covers, and its farthest texel, decoded and source.
        let texel_at = |pixels: &[u8], x: u32, y: u32| -> [u8; 4] {
            let at = ((y * 6 + x) * 4) as usize;
            [pixels[at], pixels[at + 1], pixels[at + 2], pixels[at + 3]]
        };
        let described: Vec<String> = blocks
            .as_chunks::<16>()
            .0
            .iter()
            .enumerate()
            .map(|(b, block)| {
                let (left, top) = (b as u32 % 2 * 4, b as u32 / 2 * 4);
                let covered: Vec<(u32, u32)> = (top..(top + 4).min(6)).flat_map(|y| (left..(left + 4).min(6)).map(move |x| (x, y))).collect();
                let gap = |&(x, y): &(u32, u32)| distance(&texel_at(&decoded.levels[0], x, y), &texel_at(&texels, x, y));
                let (x, y) = *covered.iter().max_by(|a, c| gap(a).total_cmp(&gap(c))).expect("a block covers texels");
                let bytes: String = block.iter().map(|v| format!("{v:02x}")).collect();
                let off: f64 = covered.iter().map(gap).sum();
                format!("block {b}: mode {}, bytes {bytes}, {off} off, farthest texel ({x}, {y}) {:?} for {:?}", block[0].trailing_zeros(), texel_at(&decoded.levels[0], x, y), texel_at(&texels, x, y))
            })
            .collect();
        assert!(encoded * 4.0 < plain, "BC7 on {} is {encoded} from its source, and a flat level of its mean {plain}; {}", core.info.name, described.join("; "));
    }

    /// An encoder's pipeline is built the first time it encodes and kept for
    /// the next encode; no other encoder's pipeline is built.
    #[test]
    fn builds_an_encoders_pipeline_on_first_use_and_keeps_it() {
        let Some(core) = core() else { return };
        let source = texture(&core, 4, 4, 1, F::Rgba8Unorm, &[128; 64]);
        assert!(core.bc_encoders.iter().all(|slot| slot.load().is_none()));
        encode_bc(&core, &source, F::Bc1RgbaUnorm, None, true, 0, None).expect("encoded");
        let built = core.bc_encoders[Encoder::Bc1 as usize].load_full().expect("kept");
        encode_bc(&core, &source, F::Bc1RgbaUnormSrgb, None, true, 0, None).expect("encoded again");
        assert!(Arc::ptr_eq(&built, &core.bc_encoders[Encoder::Bc1 as usize].load_full().expect("kept")));
        let others: Vec<usize> = (0..ENCODERS).filter(|&i| i != Encoder::Bc1 as usize && core.bc_encoders[i].load().is_some()).collect();
        assert!(others.is_empty(), "pipelines built for encoders {others:?}");
    }

    /// An R8Snorm level, a format WebGPU does not render into, draws into
    /// R16Float unchanged, and its next level is the average of the
    /// texels under it.
    #[test]
    fn renders_levels_of_a_format_through_its_float_format() {
        let Some(core) = core() else { return };
        let texels: Vec<u8> = [-127i8, 127, 64, -64].iter().map(|v| *v as u8).collect();
        let file = TextureFile { dimension: wgpu::TextureDimension::D2, width: 2, height: 2, depth: 1, layers: 1, cube: false, format: F::R8Snorm, levels: vec![texels] };
        let render = crate::texel::render_format(F::R8Snorm).expect("a float format");
        let texture = float_levels(&core, &file, render, 2, 1, MipOptions::default(), None).expect("rendered");
        let read = |level| crate::texel::colors(&crate::texture::read_texels(&core, &texture, level, 0, wgpu::TextureAspect::All, None).expect("read"), render).expect("colors");
        let top: Vec<f32> = read(0).iter().map(|c| c[0]).collect();
        let expected = [-1.0, 1.0, 64.0 / 127.0, -64.0 / 127.0];
        assert!(top.iter().zip(expected).all(|(a, b)| (a - b).abs() < 1e-3), "{top:?}");
        assert!(read(1)[0][0].abs() < 1e-3, "{:?}", read(1));
    }

    /// An Rgba32Float texture of `image` with the full mip chain, its
    /// levels below level 0 rendered as `mips` says.
    fn mipped(core: &CoreRef, image: &image::Rgba32FImage, mips: MipOptions) -> wgpu::Texture {
        let (width, height) = image.dimensions();
        let levels = wgpu::Extent3d { width, height, depth_or_array_layers: 1 }.max_mips(wgpu::TextureDimension::D2);
        let texture = texture(core, width, height, levels, F::Rgba32Float, bytemuck::cast_slice(image.as_raw()));
        render_mips(core, &texture, 1, mips, None).expect("rendered");
        texture
    }

    /// Mip level `level` of an Rgba32Float texture as an image.
    fn level_image(core: &CoreRef, texture: &wgpu::Texture, level: u32) -> image::Rgba32FImage {
        let size = texture.size().mip_level_size(level, wgpu::TextureDimension::D2);
        let values: Vec<f32> = bytemuck::pod_collect_to_vec(&crate::texture::read_texels(core, texture, level, 0, wgpu::TextureAspect::All, None).expect("read"));
        image::Rgba32FImage::from_raw(size.width, size.height, values).expect("an image")
    }

    /// The share of `alphas` at or above `reference`.
    fn passing(alphas: &[f32], reference: f32) -> f64 {
        alphas.iter().filter(|&&a| a >= reference).count() as f64 / alphas.len() as f64
    }

    /// The edge -KeepCoverage takes for a level of `alphas`, counted on the
    /// CPU as the counting pass counts them on the GPU.
    fn edge_of(alphas: &[f32], share: f64, reference: f32) -> f32 {
        let mut bins = vec![0u32; 4096];
        for &a in alphas {
            bins[((a.clamp(0.0, 1.0) * 4096.0) as usize).min(4095)] += 1;
        }
        crate::alpha::coverage_edge(&bins, share, reference)
    }

    /// -KeepCoverage against the same arithmetic on the CPU: the scales the
    /// GPU's counts give equal those the CPU's counts of each level, read
    /// back unscaled, give; each texel keeps its color and takes its
    /// unscaled alpha times the scale, at most 1, on the side of the
    /// reference its bin put it; and each level's share of texels at or
    /// above the reference comes at least as near level 0's as it was, and
    /// nearer at the levels the filter moved it at.
    #[test]
    fn keeps_alpha_coverage_across_levels() {
        let Some(core) = core() else { return };
        let reference = 0.5f32;
        let below = f32::from_bits(reference.to_bits() - 1);
        // Thin strands, which a box filter thins below the reference.
        let image = image::Rgba32FImage::from_fn(64, 64, |x, y| {
            let strand = (x * 7 + y * 3) % 11 < 2 || (x ^ y) % 13 == 0;
            image::Rgba([x as f32 / 64.0, y as f32 / 64.0, 0.5, if strand { 1.0 } else { 0.0 }])
        });
        let texture = mipped(&core, &image, MipOptions::default());
        let levels = texture.mip_level_count();
        let unscaled: Vec<image::Rgba32FImage> = (0..levels).map(|level| level_image(&core, &texture, level)).collect();
        let alphas = |img: &image::Rgba32FImage| img.pixels().map(|p| p[3]).collect::<Vec<f32>>();
        let level0 = passing(&alphas(&unscaled[0]), reference);
        let scales = crate::alpha::keep_coverage(&core, &texture, 1, reference, None).expect("kept");
        assert_eq!(scales.len(), levels as usize - 1);
        let mut nearer = 0;
        for level in 1..levels {
            let before = &unscaled[level as usize];
            let edge = edge_of(&alphas(before), level0, reference);
            let scale = reference / edge;
            assert_eq!(scales[level as usize - 1], scale, "level {level}");
            let after = level_image(&core, &texture, level);
            for (x, y, b) in before.enumerate_pixels() {
                let a = (b[3] * scale).min(1.0);
                let want = if scale == 1.0 { b[3] } else if b[3] >= edge { a.max(reference) } else { a.min(below) };
                assert_eq!(after.get_pixel(x, y).0, [b[0], b[1], b[2], want], "level {level} texel ({x}, {y})");
            }
            let was = (passing(&alphas(before), reference) - level0).abs();
            let now = (passing(&alphas(&after), reference) - level0).abs();
            assert!(now <= was, "level {level} is {now} from level 0's share of {level0}, and was {was}");
            if now < was {
                nearer += 1;
            }
        }
        assert!(nearer > 0, "no level came nearer level 0's share of {level0}");
    }

    /// Each layer of an array keeps its own level 0's share, and every
    /// depth slice of a 3D level is counted together: the scales are those
    /// the CPU's counts of exactly those texels give.
    #[test]
    fn keeps_coverage_for_each_layer_and_for_a_whole_volume() {
        let Some(core) = core() else { return };
        let reference = 0.5f32;
        let patterns: [fn(u32, u32, u32) -> bool; 2] = [|x, y, z| (x * 7 + y * 3 + z * 5) % 11 < 2, |x, y, z| (x + 2 * y + z) % 5 < 2];
        for (dimension, planes) in [(wgpu::TextureDimension::D2, 2u32), (wgpu::TextureDimension::D3, 4)] {
            let size = wgpu::Extent3d { width: 32, height: 32, depth_or_array_layers: planes };
            let levels = size.max_mips(dimension);
            let texture = core.device.create_texture(&wgpu::TextureDescriptor {
                label: Some("test"),
                size,
                mip_level_count: levels,
                sample_count: 1,
                dimension,
                format: F::Rgba32Float,
                usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC | wgpu::TextureUsages::COPY_DST,
                view_formats: &[],
            });
            let plane = |pattern: fn(u32, u32, u32) -> bool, z: u32| -> Vec<f32> {
                (0..32 * 32).flat_map(|i| [0.25, 0.5, 0.75, if pattern(i % 32, i / 32, z) { 1.0 } else { 0.0 }]).collect()
            };
            let volume = dimension == wgpu::TextureDimension::D3;
            if volume {
                let texels: Vec<f32> = (0..planes).flat_map(|z| plane(patterns[0], z)).collect();
                crate::texture::write_texels(&core, &texture, 0, 0, wgpu::TextureAspect::All, bytemuck::cast_slice(&texels)).expect("written");
            } else {
                for layer in 0..planes {
                    crate::texture::write_texels(&core, &texture, 0, layer, wgpu::TextureAspect::All, bytemuck::cast_slice(&plane(patterns[layer as usize], 0))).expect("written");
                }
            }
            render_mips(&core, &texture, 1, MipOptions::default(), None).expect("rendered");
            let groups = if volume { 1 } else { planes };
            let alphas = |level: u32, group: u32| -> Vec<f32> {
                let values: Vec<f32> = bytemuck::pod_collect_to_vec(&crate::texture::read_texels(&core, &texture, level, group, wgpu::TextureAspect::All, None).expect("read"));
                values.as_chunks::<4>().0.iter().map(|t| t[3]).collect()
            };
            let mut want = Vec::new();
            for group in 0..groups {
                let level0 = passing(&alphas(0, group), reference);
                for level in 1..levels {
                    want.push(reference / edge_of(&alphas(level, group), level0, reference));
                }
            }
            let scales = crate::alpha::keep_coverage(&core, &texture, 1, reference, None).expect("kept");
            assert_eq!(scales, want, "{dimension:?} with {planes} layers or slices");
        }
    }

    /// The largest difference of any channel between two images of one
    /// size, and where it is: the texel's x and y, the channel, and the two
    /// values.
    fn worst(a: &image::Rgba32FImage, b: &image::Rgba32FImage) -> (f32, String) {
        assert_eq!(a.dimensions(), b.dimensions());
        let (at, gap) = a.as_raw().iter().zip(b.as_raw()).map(|(x, y)| (x - y).abs()).enumerate().fold((0, 0.0), |most, (i, d)| if d > most.1 { (i, d) } else { most });
        let (texel, channel) = (at / 4, at % 4);
        let (x, y) = (texel as u32 % a.width(), texel as u32 / a.width());
        (gap, format!("{gap} at texel ({x}, {y}) channel {channel}: {} against {}", a.as_raw()[at], b.as_raw()[at]))
    }

    /// A 23x17 picture whose texels all differ, every channel from 0.3 to
    /// 0.7, which no filter here carries past 0 or 1, where the image
    /// crate clamps.
    fn picture() -> image::Rgba32FImage {
        let value = |v: u32| 0.3 + 0.4 * (v % 256) as f32 / 255.0;
        image::Rgba32FImage::from_fn(23, 17, |x, y| image::Rgba([value(x * 37 + y * 11), value(x * 5 + y * 41), value((x ^ y) * 23), value(255 - x - y)]))
    }

    /// Triangle and Lanczos make each level from level 0 as the image
    /// crate's resize of level 0 to that size does: the same taps, the
    /// ones past an edge left out and the rest weighed again. Lanczos takes
    /// the wider bound, as a GPU's sin may be less exact than the CPU's.
    #[test]
    fn filters_each_level_from_level_0_as_the_image_crate_resizes_it() {
        let Some(core) = core() else { return };
        let source = picture();
        for (filter, theirs, bound) in [(MipFilter::Triangle, image::imageops::FilterType::Triangle, 1e-4), (MipFilter::Lanczos, image::imageops::FilterType::Lanczos3, 2e-3)] {
            let texture = mipped(&core, &source, MipOptions::given(Some(filter), None));
            for level in 1..texture.mip_level_count() {
                let got = level_image(&core, &texture, level);
                let expected = image::imageops::resize(&source, got.width(), got.height(), theirs);
                let (gap, said) = worst(&got, &expected);
                assert!(gap <= bound, "{filter:?} level {level} differs by {said}");
            }
        }
    }

    /// A Wrap edge reads the other side of the image and a Mirror edge the
    /// image mirrored, so a level equals the middle of the image crate's
    /// resize of the image tiled three by three, plain or mirrored.
    #[test]
    fn wraps_and_mirrors_an_edge_as_a_resize_of_tiles() {
        let Some(core) = core() else { return };
        let source = picture();
        let (w, h) = source.dimensions();
        for edge in [MipEdge::Wrap, MipEdge::Mirror] {
            let tiles = image::Rgba32FImage::from_fn(3 * w, 3 * h, |x, y| {
                let (tx, ty, sx, sy) = (x / w, y / h, x % w, y % h);
                let mirrored = edge == MipEdge::Mirror;
                let sx = if mirrored && tx != 1 { w - 1 - sx } else { sx };
                let sy = if mirrored && ty != 1 { h - 1 - sy } else { sy };
                *source.get_pixel(sx, sy)
            });
            let texture = mipped(&core, &source, MipOptions::given(Some(MipFilter::Triangle), Some(edge)));
            for level in 1..=2 {
                let got = level_image(&core, &texture, level);
                let (lw, lh) = got.dimensions();
                let resized = image::imageops::resize(&tiles, 3 * lw, 3 * lh, image::imageops::FilterType::Triangle);
                let middle = image::imageops::crop_imm(&resized, lw, lh, lw, lh).to_image();
                let (gap, said) = worst(&got, &middle);
                assert!(gap <= 1e-4, "{edge:?} level {level} differs by {said}");
            }
        }
    }

    // Levels 1 and 3 of a 64-texel row holding a single 1 at texel 20,
    // filtered with a Wrap edge, computed outside the module in f64 and
    // rounded to f32: each texel is the kernel's weight at texel 20 over
    // the sum of its weights at every texel within its reach, the kernel
    // stretched by the level's ratio, Kaiser from numpy's sinc and i0 with
    // NVTT's width 3 and alpha 4, and Mitchell from Mitchell and
    // Netravali's B, C cubic in exact fractions with B = C = 1/3.
    const KAISER_LEVEL_1: [f32; 32] = [
        0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0063068084, -0.03391876, 0.13399173, 0.44307706, -0.06568918, 0.016232356, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0,
        0.0, 0.0, 0.0, 0.0,
    ];
    const KAISER_LEVEL_3: [f32; 8] = [0.0015001849, -0.0058289645, 0.12369628, 0.006947226, -0.0018016481, 0.00027309055, 0.0, 0.0];
    const MITCHELL_LEVEL_1: [f32; 32] = [
        0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, -0.007378472, 0.1280382, 0.39105904, -0.01171875, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0,
        0.0, 0.0,
    ];
    const MITCHELL_LEVEL_3: [f32; 8] = [0.0, 0.0034332275, 0.110170156, 0.011547512, -0.00015089248, 0.0, 0.0, 0.0];

    /// A single bright texel spreads over a level as the Kaiser and
    /// Mitchell kernels weigh it, and alpha, 1 everywhere, stays 1, as the
    /// weights sum to 1.
    #[test]
    fn spreads_one_texel_by_kaisers_and_mitchells_kernels() {
        let Some(core) = core() else { return };
        let row = image::Rgba32FImage::from_fn(64, 1, |x, _| image::Rgba([if x == 20 { 1.0 } else { 0.0 }, 0.0, 0.0, 1.0]));
        for (filter, level_1, level_3) in [(MipFilter::Kaiser, &KAISER_LEVEL_1[..], &KAISER_LEVEL_3[..]), (MipFilter::Mitchell, &MITCHELL_LEVEL_1[..], &MITCHELL_LEVEL_3[..])] {
            let texture = mipped(&core, &row, MipOptions::given(Some(filter), Some(MipEdge::Wrap)));
            for (level, expected) in [(1, level_1), (3, level_3)] {
                let got = level_image(&core, &texture, level);
                for (i, (texel, want)) in got.pixels().zip(expected).enumerate() {
                    assert!((texel[0] - want).abs() <= 1e-3, "{filter:?} level {level} texel {i}: {} for {want}", texel[0]);
                    assert!((texel[3] - 1.0).abs() <= 1e-5, "{filter:?} level {level} texel {i}: alpha {}", texel[3]);
                }
            }
        }
    }

    /// Min and Max make each texel the smallest or the largest value of each
    /// channel among the texels of the level above it covers, a texel it
    /// covers in part among them.
    #[test]
    fn takes_the_smallest_or_largest_of_the_texels_each_new_texel_covers() {
        let Some(core) = core() else { return };
        let source = image::Rgba32FImage::from_fn(5, 3, |x, y| image::Rgba([0, 1, 2, 3].map(|c| ((x * 7 + y * 3 + c * 5) % 11) as f32 / 10.0)));
        // The texels of a row `n` long that texel `i` of a row half as long,
        // and at least 1, covers.
        let covered = |i: u32, n: u32| {
            let scale = n as f32 / (n >> 1).max(1) as f32;
            (0..n).filter(move |&j| (j + 1) as f32 > i as f32 * scale && (j as f32) < (i + 1) as f32 * scale)
        };
        for (filter, largest) in [(MipFilter::Min, false), (MipFilter::Max, true)] {
            let texture = mipped(&core, &source, MipOptions::given(Some(filter), None));
            let mut above = source.clone();
            for level in 1..texture.mip_level_count() {
                let got = level_image(&core, &texture, level);
                let (n, m) = above.dimensions();
                let expected = image::Rgba32FImage::from_fn(got.width(), got.height(), |x, y| {
                    let texels: Vec<image::Rgba<f32>> = covered(y, m).flat_map(|sy| covered(x, n).map(move |sx| (sx, sy))).map(|(sx, sy)| *above.get_pixel(sx, sy)).collect();
                    image::Rgba([0, 1, 2, 3].map(|c| texels.iter().map(|t| t[c]).fold(if largest { f32::MIN } else { f32::MAX }, if largest { f32::max } else { f32::min })))
                });
                assert_eq!(got, expected, "{filter:?} level {level}");
                above = got;
            }
        }
    }

    /// The edge names and the sampler names that mean the same edge, any
    /// case; ClampToBorder is refused and any other name is no edge.
    #[test]
    fn reads_an_edge_from_its_name_or_the_sampler_names() {
        let named = |name: &str| MipEdge::named(name).expect("not refused");
        assert_eq!(named("Wrap"), Some(MipEdge::Wrap));
        assert_eq!(named("repeat"), Some(MipEdge::Wrap));
        assert_eq!(named("MIRRORREPEAT"), Some(MipEdge::Mirror));
        assert_eq!(named("ClampToEdge"), Some(MipEdge::Clamp));
        assert_eq!(named("Sideways"), None);
        match MipEdge::named("ClampToBorder") {
            Ok(edge) => panic!("ClampToBorder read as {edge:?}"),
            Err(e) => assert_eq!(e.error_id, "WgpuMipEdge"),
        }
    }

    /// Wrap and Mirror are refused with Box, Min and Max, which read no
    /// texel past an edge, and taken with the filters that do; Clamp goes
    /// with any filter.
    #[test]
    fn refuses_an_edge_the_filter_never_reads_past() {
        let refusal = |filter: Option<MipFilter>, edge: MipEdge| match MipOptions::given(filter, Some(edge)).check() {
            Ok(()) => None,
            Err(e) => Some((e.error_id, e.message)),
        };
        let (id, message) = refusal(None, MipEdge::Wrap).expect("refused");
        assert_eq!(id, "WgpuMipEdge");
        assert!(message.starts_with("Box, the filter when none is given, reads only the texels each new texel covers"), "{message}");
        let (_, message) = refusal(Some(MipFilter::Max), MipEdge::Mirror).expect("refused");
        assert!(message.starts_with("Max reads only"), "{message}");
        assert_eq!(refusal(Some(MipFilter::Kaiser), MipEdge::Wrap), None);
        assert_eq!(refusal(Some(MipFilter::Min), MipEdge::Clamp), None);
    }
}
