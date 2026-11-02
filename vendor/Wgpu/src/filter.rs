//! Image filters over level 0 of a texture the transform drew, in place: a
//! Gaussian blur, an unsharp mask, and edges by the Sobel, Scharr and
//! Prewitt gradients, the Laplacian, or Canny's method as scikit-image
//! follows it. Each layer of an array or a cube map, and each depth slice
//! of a 3D texture, is filtered on its own, with the texels past an edge
//! read as a FilterEdge says. The blur and the unsharp mask work on linear
//! values weighed by alpha; the gradients and the Laplacian on the values
//! the texture stores; Canny on the luminance, Rec. 709 in linear light, or
//! the red channel of a texture of one or two channels, which holds gray.
//! The passes are compute shaders over 32-bit float copies, and a render
//! pass writes the result back in the texture's own format.

use crate::buffer;
use crate::device::CoreRef;
use crate::error::invalid;
use crate::tasks::Interrupt;

use pwrs::prelude::*;
use wgpu::util::DeviceExt;
use wgpu::TextureFormat as F;

/// What a filter reads past the edge of the texture.
#[psenum(name = "Wgpu.FilterEdge")]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum FilterEdge {
    /// The edge texel repeated, as scikit-image's gaussian and ImageMagick's
    /// Edge virtual pixels read past it.
    #[default]
    Clamp,
    /// The other side, for a texture that tiles.
    Wrap,
    /// The texture mirrored at its edge, the edge texel repeated, as a
    /// MirrorRepeat sampler reads it.
    Mirror,
}

impl FilterEdge {
    /// The number the shaders know the edge by.
    fn code(self) -> u32 {
        match self {
            FilterEdge::Clamp => 0,
            FilterEdge::Wrap => 1,
            FilterEdge::Mirror => 2,
        }
    }
}

/// How -Edges finds edges.
#[psenum(name = "Wgpu.EdgeOperator")]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum EdgeOperator {
    /// The size of the gradient by Sobel's kernel, which weighs the three
    /// rows or columns across it 1, 2 and 1.
    #[default]
    Sobel,
    /// The size of the gradient by Scharr's kernel, which weighs them 3, 10
    /// and 3.
    Scharr,
    /// The size of the gradient by Prewitt's kernel, which weighs them
    /// alike.
    Prewitt,
    /// The size of the second derivative, by the Laplacian of the four
    /// nearest texels.
    Laplacian,
    /// Canny's thin, linked edges in the luminance: 1 on an edge and 0
    /// elsewhere.
    Canny,
}

impl EdgeOperator {
    /// The number the gradient shader knows the operator by.
    fn code(self) -> u32 {
        match self {
            EdgeOperator::Sobel => 0,
            EdgeOperator::Scharr => 1,
            EdgeOperator::Prewitt => 2,
            EdgeOperator::Laplacian => 3,
            EdgeOperator::Canny => 4,
        }
    }
}

/// An unsharp mask: `amount` times the difference between a texel and a
/// Gaussian blur of standard deviation `radius` added back, where that
/// difference is at least `threshold`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Sharpen {
    pub(crate) amount: f64,
    pub(crate) radius: f64,
    pub(crate) threshold: f64,
}

/// The edges -Edges finds: Canny's take a smoothing of standard deviation
/// `sigma` and the thresholds `low` and `high`, in the scale where a step
/// from 0 to 1 reads 1.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Edges {
    pub(crate) operator: EdgeOperator,
    pub(crate) low: f64,
    pub(crate) high: f64,
    pub(crate) sigma: f64,
}

/// The filters one call runs, in this order: the blur, the unsharp mask,
/// then the edges, each reading past the edge as `edge` says.
#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub(crate) struct Filters {
    /// The blur's standard deviations across and down, in texels.
    pub(crate) blur: Option<[f64; 2]>,
    pub(crate) sharpen: Option<Sharpen>,
    pub(crate) edges: Option<Edges>,
    pub(crate) edge: FilterEdge,
}

impl Filters {
    /// What the notes say the filters did.
    pub(crate) fn said(&self) -> Vec<String> {
        let past = match self.edge {
            FilterEdge::Clamp => "the edge texels repeated past the edges",
            FilterEdge::Wrap => "the other side read past the edges",
            FilterEdge::Mirror => "the texture mirrored past the edges",
        };
        let mut said = Vec::new();
        if let Some([across, down]) = self.blur {
            let spread = if across == down { format!("a standard deviation of {}", texels(across)) } else { format!("standard deviations of {} across and {} down", texels(across), texels(down)) };
            said.push(format!("the texels were blurred on the GPU by a Gaussian of {spread}, in linear light weighed by alpha, {past}"));
        }
        if let Some(s) = self.sharpen {
            said.push(format!(
                "the texels were sharpened on the GPU by an unsharp mask, {} times the difference from a Gaussian of a standard deviation of {} added where it is at least {}, {past}",
                s.amount,
                texels(s.radius),
                s.threshold
            ));
        }
        if let Some(e) = self.edges {
            said.push(match e.operator {
                EdgeOperator::Canny => format!(
                    "Canny's edges were found on the GPU in the luminance smoothed by a Gaussian of a standard deviation of {}, kept above {} where they link to one above {}, {past}",
                    texels(e.sigma),
                    e.low,
                    e.high
                ),
                EdgeOperator::Laplacian => format!("the edges were found on the GPU as the size of the Laplacian of the stored values, {past}"),
                other => format!("the edges were found on the GPU as the size of the {other:?} gradient of the stored values, {past}"),
            });
        }
        said
    }

    fn canny(&self) -> bool {
        self.edges.is_some_and(|e| e.operator == EdgeOperator::Canny)
    }

    /// The entry points of the compute shader the filters run after the
    /// load.
    fn entries(&self) -> Vec<&'static str> {
        let mut entries = Vec::new();
        if self.blur.is_some() || self.sharpen.is_some() || self.canny() {
            entries.push("blur");
        }
        if self.sharpen.is_some() {
            entries.push("sharpen");
        }
        match self.edges.map(|e| e.operator) {
            Some(EdgeOperator::Canny) => entries.extend(["luma", "sobel", "suppress", "link", "edges"]),
            Some(_) => entries.push("gradient"),
            None => {}
        }
        entries
    }
}

/// The filter parameters, as they were given.
#[derive(Clone, Copy, Default)]
pub(crate) struct Asked<'a> {
    pub(crate) blur: Option<&'a [f64]>,
    pub(crate) sharpen: Option<f64>,
    pub(crate) sharpen_radius: Option<f64>,
    pub(crate) sharpen_threshold: Option<f64>,
    pub(crate) edges: Option<EdgeOperator>,
    pub(crate) edge_threshold: Option<&'a [f64]>,
    pub(crate) edge_sigma: Option<f64>,
    pub(crate) edge: Option<FilterEdge>,
}

/// `value` texels, as a note says it.
fn texels(value: f64) -> String {
    if value == 1.0 { "1 texel".to_string() } else { format!("{value} texels") }
}

/// Whether `value` is a number above 0.
fn positive(value: f64) -> bool {
    value.is_finite() && value > 0.0
}

impl Asked<'_> {
    /// The filters the parameters name, or `None` for none. A value out of
    /// range, and a setting given without the filter it belongs to, are
    /// refused.
    pub(crate) fn filters(&self) -> PsResult<Option<Filters>> {
        let blur = match self.blur {
            None => None,
            Some(sigmas) if sigmas.len() == 1 || sigmas.len() == 2 => {
                if let Some(sigma) = sigmas.iter().copied().find(|s| !positive(*s)) {
                    return Err(invalid("WgpuBlur", format!("-Blur takes standard deviations above 0, and {sigma} is not")));
                }
                Some([sigmas[0], sigmas[sigmas.len() - 1]])
            }
            Some(sigmas) => return Err(invalid("WgpuBlur", format!("-Blur takes one standard deviation, or two, across and down, and was given {}", sigmas.len()))),
        };
        let sharpen = match self.sharpen {
            Some(amount) => {
                if !positive(amount) {
                    return Err(invalid("WgpuSharpen", format!("-Sharpen is how much of the detail to add, above 0, and {amount} is not")));
                }
                let radius = self.sharpen_radius.unwrap_or(1.0);
                if !positive(radius) {
                    return Err(invalid("WgpuSharpen", format!("-SharpenRadius is the standard deviation of the blur -Sharpen compares with, above 0, and {radius} is not")));
                }
                let threshold = self.sharpen_threshold.unwrap_or(0.0);
                if !(0.0..=1.0).contains(&threshold) {
                    return Err(invalid("WgpuSharpen", format!("-SharpenThreshold is a difference from 0 to 1, and {threshold} is not")));
                }
                Some(Sharpen { amount, radius, threshold })
            }
            None if self.sharpen_radius.is_some() || self.sharpen_threshold.is_some() => {
                return Err(invalid("WgpuSharpen", "-SharpenRadius and -SharpenThreshold say how -Sharpen sharpens: give -Sharpen"));
            }
            None => None,
        };
        if self.edges != Some(EdgeOperator::Canny) && (self.edge_threshold.is_some() || self.edge_sigma.is_some()) {
            return Err(invalid("WgpuEdges", "-EdgeThreshold and -EdgeSigma say how Canny finds edges: give -Edges Canny"));
        }
        let (low, high) = match self.edge_threshold {
            None => (0.1, 0.2),
            Some(&[low, high]) if low.is_finite() && high.is_finite() && low >= 0.0 && low <= high => (low, high),
            Some(&[low, high]) => {
                return Err(invalid("WgpuEdges", format!("-EdgeThreshold is Canny's low threshold, 0 or above, and its high one, no lower, and {low} and {high} are not")));
            }
            Some(values) => return Err(invalid("WgpuEdges", format!("-EdgeThreshold takes two numbers, Canny's low and high thresholds, and was given {}", values.len()))),
        };
        let sigma = self.edge_sigma.unwrap_or(1.0);
        if !positive(sigma) {
            return Err(invalid("WgpuEdges", format!("-EdgeSigma is the standard deviation of the smoothing before Canny's gradient, above 0, and {sigma} is not")));
        }
        let edges = self.edges.map(|operator| Edges { operator, low, high, sigma });
        if blur.is_none() && sharpen.is_none() && edges.is_none() {
            if self.edge.is_some() {
                return Err(invalid("WgpuFilterEdge", "-FilterEdge says what a filter reads past the edge of the texture, and no filter was given"));
            }
            return Ok(None);
        }
        Ok(Some(Filters { blur, sharpen, edges, edge: self.edge.unwrap_or(FilterEdge::Clamp) }))
    }
}

/// A Gaussian's weights from -r to r texels, summing to 1, where r is 4
/// standard deviations rounded as scipy's gaussian_filter rounds its
/// truncate of 4.
pub(crate) fn gaussian_weights(sigma: f64) -> Vec<f32> {
    let radius = (4.0 * sigma + 0.5).floor() as i64;
    let raw: Vec<f64> = (-radius..=radius).map(|x| (-0.5 * (x as f64 / sigma).powi(2)).exp()).collect();
    let total: f64 = raw.iter().sum();
    raw.iter().map(|w| (w / total) as f32).collect()
}

/// The shaders' shared values: the texture's size and channels, the edge,
/// a blur's radius and axis, the gradient operator, whether the texture
/// stores sRGB, the first row a submission covers, and the unsharp mask's
/// amount and threshold and Canny's thresholds.
#[derive(Clone, Copy, Default)]
struct Params {
    width: u32,
    height: u32,
    edge: u32,
    radius: u32,
    axis: u32,
    kernel: u32,
    srgb: u32,
    channels: u32,
    row: u32,
    amount: f32,
    threshold: f32,
    low: f32,
    high: f32,
}

impl Params {
    fn bytes(&self) -> Vec<u8> {
        let words = [
            self.width,
            self.height,
            self.edge,
            self.radius,
            self.axis,
            self.kernel,
            self.srgb,
            self.channels,
            self.row,
            self.amount.to_bits(),
            self.threshold.to_bits(),
            self.low.to_bits(),
            self.high.to_bits(),
            0,
            0,
            0,
        ];
        bytemuck::cast_slice(&words).to_vec()
    }
}

const COMPUTE: &str = r"
struct Params {
    width: u32,
    height: u32,
    edge: u32,
    radius: u32,
    axis: u32,
    kernel: u32,
    srgb: u32,
    channels: u32,
    row: u32,
    amount: f32,
    threshold: f32,
    low: f32,
    high: f32,
    unused0: u32,
    unused1: u32,
    unused2: u32,
}

@group(0) @binding(0) var source: texture_2d<f32>;
@group(0) @binding(1) var written: texture_storage_2d<rgba32float, write>;
@group(0) @binding(2) var<uniform> params: Params;
@group(0) @binding(3) var<storage, read> weights: array<f32>;
@group(0) @binding(4) var other: texture_2d<f32>;
@group(0) @binding(5) var<storage, read_write> state: array<atomic<u32>>;
@group(0) @binding(6) var<storage, read_write> changed: atomic<u32>;

// The texel `i` of `n` reads past an edge: the edge texel, the other side,
// or the texture mirrored with the edge texel repeated.
fn edge_index(i: i32, n: i32) -> i32 {
    if (i >= 0 && i < n) {
        return i;
    }
    switch params.edge {
        case 1u: {
            return ((i % n) + n) % n;
        }
        case 2u: {
            let period = 2 * n;
            var m = ((i % period) + period) % period;
            if (m >= n) {
                m = period - 1 - m;
            }
            return m;
        }
        default: {
            return clamp(i, 0, n - 1);
        }
    }
}

fn at(t: texture_2d<f32>, x: i32, y: i32) -> vec4<f32> {
    let w = i32(params.width);
    let h = i32(params.height);
    return textureLoad(t, vec2<i32>(edge_index(x, w), edge_index(y, h)), 0);
}

fn straight(c: vec4<f32>) -> vec3<f32> {
    if (c.a > 0.0) {
        return c.rgb / c.a;
    }
    return vec3<f32>(0.0);
}

fn to_srgb(v: vec3<f32>) -> vec3<f32> {
    let low = v * 12.92;
    let high = 1.055 * pow(max(v, vec3<f32>(0.0)), vec3<f32>(1.0 / 2.4)) - 0.055;
    return select(high, low, v <= vec3<f32>(0.0031308));
}

fn to_linear(v: vec3<f32>) -> vec3<f32> {
    let low = v / 12.92;
    let high = pow(max((v + 0.055) / 1.055, vec3<f32>(0.0)), vec3<f32>(2.4));
    return select(high, low, v <= vec3<f32>(0.04045));
}

// The texel this invocation writes, which may be past the texture.
fn texel(id: vec3<u32>) -> vec2<i32> {
    return vec2<i32>(i32(id.x), i32(id.y + params.row));
}

fn outside(p: vec2<i32>) -> bool {
    return p.x >= i32(params.width) || p.y >= i32(params.height);
}

fn state_at(p: vec2<i32>) -> u32 {
    return u32(p.y) * params.width + u32(p.x);
}

// The texture's texels, linear and weighed by alpha.
@compute @workgroup_size(8, 8)
fn load(@builtin(global_invocation_id) id: vec3<u32>) {
    let p = texel(id);
    if (outside(p)) {
        return;
    }
    let c = textureLoad(source, p, 0);
    textureStore(written,p, vec4<f32>(c.rgb * c.a, c.a));
}

// One axis of a Gaussian, its weights from -radius to radius.
@compute @workgroup_size(8, 8)
fn blur(@builtin(global_invocation_id) id: vec3<u32>) {
    let p = texel(id);
    if (outside(p)) {
        return;
    }
    let along = select(vec2<i32>(1, 0), vec2<i32>(0, 1), params.axis == 1u);
    let r = i32(params.radius);
    var sum = vec4<f32>(0.0);
    for (var k = -r; k <= r; k++) {
        let q = p + along * k;
        sum += weights[u32(k + r)] * at(source, q.x, q.y);
    }
    textureStore(written,p, sum);
}

// The unsharp mask of the texels in source, weighed by alpha, against their
// blur in other: amount times the difference added where it is at least
// the threshold, alpha kept.
@compute @workgroup_size(8, 8)
fn sharpen(@builtin(global_invocation_id) id: vec3<u32>) {
    let p = texel(id);
    if (outside(p)) {
        return;
    }
    let c = textureLoad(source, p, 0);
    let b = textureLoad(other, p, 0);
    let color = straight(c);
    var blurred = color;
    if (b.a > 0.0) {
        blurred = b.rgb / b.a;
    }
    let detail = color - blurred;
    let sharp = select(color + params.amount * detail, color, abs(detail) < vec3<f32>(params.threshold));
    textureStore(written,p, vec4<f32>(sharp * c.a, c.a));
}

// The color of a texel as the texture stores it.
fn stored(x: i32, y: i32) -> vec3<f32> {
    let c = straight(at(source, x, y));
    if (params.srgb == 1u) {
        return to_srgb(c);
    }
    return c;
}

// The size of each channel's gradient by Sobel's, Scharr's or Prewitt's
// kernel, a step from 0 to 1 reading 1, or of its Laplacian, in the values
// the texture stores; alpha kept.
@compute @workgroup_size(8, 8)
fn gradient(@builtin(global_invocation_id) id: vec3<u32>) {
    let p = texel(id);
    if (outside(p)) {
        return;
    }
    var size: vec3<f32>;
    if (params.kernel == 3u) {
        let around = stored(p.x - 1, p.y) + stored(p.x + 1, p.y) + stored(p.x, p.y - 1) + stored(p.x, p.y + 1);
        size = abs(around - 4.0 * stored(p.x, p.y));
    } else {
        var w = vec3<f32>(1.0, 2.0, 1.0);
        if (params.kernel == 1u) {
            w = vec3<f32>(3.0, 10.0, 3.0);
        } else if (params.kernel == 2u) {
            w = vec3<f32>(1.0, 1.0, 1.0);
        }
        var gx = vec3<f32>(0.0);
        var gy = vec3<f32>(0.0);
        for (var j = -1; j <= 1; j++) {
            let wj = w[j + 1];
            gx += wj * (stored(p.x + 1, p.y + j) - stored(p.x - 1, p.y + j));
            gy += wj * (stored(p.x + j, p.y + 1) - stored(p.x + j, p.y - 1));
        }
        size = sqrt(gx * gx + gy * gy) / (w.x + w.y + w.z);
    }
    if (params.srgb == 1u) {
        size = to_linear(size);
    }
    let a = textureLoad(source, p, 0).a;
    textureStore(written,p, vec4<f32>(size * a, a));
}

// Each texel's luminance, Rec. 709 in linear light, or the red channel of a
// texture of one or two channels, which holds gray.
@compute @workgroup_size(8, 8)
fn luma(@builtin(global_invocation_id) id: vec3<u32>) {
    let p = texel(id);
    if (outside(p)) {
        return;
    }
    let color = straight(textureLoad(source, p, 0));
    var y = dot(color, vec3<f32>(0.2126, 0.7152, 0.0722));
    if (params.channels < 3u) {
        y = color.r;
    }
    textureStore(written,p, vec4<f32>(y, y, y, 1.0));
}

// A texel of `t`, the edge texel one past the edge, as scikit-image's Sobel
// reads the smoothed luminance whatever its smoothing reads.
fn clamped(t: texture_2d<f32>, x: i32, y: i32) -> vec4<f32> {
    let w = i32(params.width);
    let h = i32(params.height);
    return textureLoad(t, vec2<i32>(clamp(x, 0, w - 1), clamp(y, 0, h - 1)), 0);
}

// Sobel's gradient of the smoothed luminance, across and down, a step from 0
// to 1 reading 1, and its size.
@compute @workgroup_size(8, 8)
fn sobel(@builtin(global_invocation_id) id: vec3<u32>) {
    let p = texel(id);
    if (outside(p)) {
        return;
    }
    var gx = 0.0;
    var gy = 0.0;
    for (var j = -1; j <= 1; j++) {
        let wj = select(1.0, 2.0, j == 0);
        gx += wj * (clamped(source, p.x + 1, p.y + j).r - clamped(source, p.x - 1, p.y + j).r);
        gy += wj * (clamped(source, p.x + j, p.y + 1).r - clamped(source, p.x + j, p.y - 1).r);
    }
    gx = gx / 4.0;
    gy = gy / 4.0;
    textureStore(written,p, vec4<f32>(gx, gy, sqrt(gx * gx + gy * gy), 0.0));
}

fn size_at(q: vec2<i32>) -> f32 {
    return textureLoad(source, q, 0).b;
}

// Non-maximum suppression as scikit-image's bilinear one: a texel inside
// the outermost ring whose gradient is at least the low threshold is kept
// where the sizes interpolated at the two texels along its gradient are no
// larger, as 1, or as 2 where it is at least the high threshold. The
// branches pick only the offsets of the neighbors and the weight between
// them, and the neighbors are read after: AMD's shader compiler ends the
// process, through Vulkan and Direct3D 12 alike, on this pass with the
// reads inside the branches.
@compute @workgroup_size(8, 8)
fn suppress(@builtin(global_invocation_id) id: vec3<u32>) {
    let p = texel(id);
    if (outside(p)) {
        return;
    }
    let w = i32(params.width);
    let h = i32(params.height);
    let g = textureLoad(source, p, 0);
    // As scikit-image names them: isobel along the rows, down, and jsobel
    // along the columns, across.
    let isobel = g.y;
    let jsobel = g.x;
    let m = g.z;
    var kept = 0u;
    if (p.x > 0 && p.y > 0 && p.x < w - 1 && p.y < h - 1 && m >= max(params.low, 1e-14)) {
        let ai = abs(isobel);
        let aj = abs(jsobel);
        let both = (isobel >= 0.0 && jsobel >= 0.0) || (isobel <= 0.0 && jsobel <= 0.0);
        // The neighbor along the nearer axis, the diagonal one beside it
        // and the weight between them, on one side; the other side is the
        // opposite pair.
        var near = vec2<i32>(0, 1);
        var far = vec2<i32>(1, 1);
        var t = aj / ai;
        if (both && ai <= aj) {
            near = vec2<i32>(1, 0);
            t = ai / aj;
        } else if (!both && ai < aj) {
            near = vec2<i32>(1, 0);
            far = vec2<i32>(1, -1);
            t = ai / aj;
        } else if (!both) {
            near = vec2<i32>(0, -1);
            far = vec2<i32>(1, -1);
        }
        let n11 = size_at(p + near);
        let n12 = size_at(p + far);
        let n21 = size_at(p - near);
        let n22 = size_at(p - far);
        if (n12 * t + n11 * (1.0 - t) <= m && n22 * t + n21 * (1.0 - t) <= m) {
            kept = select(1u, 2u, m >= params.high);
        }
    }
    atomicStore(&state[state_at(p)], kept);
}

var<workgroup> tile: array<atomic<u32>, 324>;
var<workgroup> grown: atomic<u32>;
var<workgroup> again: u32;

fn tile_state(x: i32, y: i32) -> u32 {
    if (x < 0 || y < 0 || x >= i32(params.width) || y >= i32(params.height)) {
        return 0u;
    }
    return atomicLoad(&state[state_at(vec2<i32>(x, y))]);
}

// Hysteresis in a 16 by 16 tile and the texels around it: a kept texel
// 8-connected to a strong one becomes strong, round after round until the
// tile settles; `changed` is set when a texel of the tile changed, so a
// later sweep carries the change into the tiles beside it.
@compute @workgroup_size(16, 16)
fn link(@builtin(local_invocation_id) local: vec3<u32>, @builtin(workgroup_id) group: vec3<u32>) {
    let origin = vec2<i32>(i32(group.x * 16u), i32(group.y * 16u + params.row)) - vec2<i32>(1);
    for (var i = local.y * 16u + local.x; i < 324u; i += 256u) {
        atomicStore(&tile[i], tile_state(origin.x + i32(i % 18u), origin.y + i32(i / 18u)));
    }
    workgroupBarrier();
    let me = (local.y + 1u) * 18u + local.x + 1u;
    let first = atomicLoad(&tile[me]);
    loop {
        if (local.x == 0u && local.y == 0u) {
            atomicStore(&grown, 0u);
        }
        workgroupBarrier();
        if (atomicLoad(&tile[me]) == 1u) {
            var strong = false;
            for (var dy = 0u; dy < 3u; dy++) {
                for (var dx = 0u; dx < 3u; dx++) {
                    if (atomicLoad(&tile[me + dy * 18u + dx - 19u]) == 2u) {
                        strong = true;
                    }
                }
            }
            if (strong) {
                atomicStore(&tile[me], 2u);
                atomicStore(&grown, 1u);
            }
        }
        workgroupBarrier();
        if (local.x == 0u && local.y == 0u) {
            again = atomicLoad(&grown);
        }
        if (workgroupUniformLoad(&again) == 0u) {
            break;
        }
    }
    let p = origin + vec2<i32>(i32(local.x) + 1, i32(local.y) + 1);
    if (!outside(p)) {
        let last = atomicLoad(&tile[me]);
        atomicStore(&state[state_at(p)], last);
        if (last != first) {
            atomicStore(&changed, 1u);
        }
    }
}

// Canny's edges: 1 on a strong texel and 0 elsewhere, with the alpha of the
// texels in other.
@compute @workgroup_size(8, 8)
fn edges(@builtin(global_invocation_id) id: vec3<u32>) {
    let p = texel(id);
    if (outside(p)) {
        return;
    }
    let a = textureLoad(other, p, 0).a;
    let v = select(0.0, 1.0, atomicLoad(&state[state_at(p)]) == 2u);
    textureStore(written,p, vec4<f32>(vec3<f32>(v) * a, a));
}
";

const VOLUME_LOAD: &str = r"
struct Params {
    width: u32,
    height: u32,
    slice: u32,
    row: u32,
}

@group(0) @binding(0) var source: texture_3d<f32>;
@group(0) @binding(1) var written: texture_storage_2d<rgba32float, write>;
@group(0) @binding(2) var<uniform> params: Params;

// One depth slice's texels, linear and weighed by alpha.
@compute @workgroup_size(8, 8)
fn load(@builtin(global_invocation_id) id: vec3<u32>) {
    let y = id.y + params.row;
    if (id.x >= params.width || y >= params.height) {
        return;
    }
    let c = textureLoad(source, vec3<i32>(i32(id.x), i32(y), i32(params.slice)), 0);
    textureStore(written,vec2<i32>(i32(id.x), i32(y)), vec4<f32>(c.rgb * c.a, c.a));
}
";

const STORE: &str = r"
@group(0) @binding(0) var work: texture_2d<f32>;

@vertex
fn vertex(@builtin(vertex_index) i: u32) -> @builtin(position) vec4<f32> {
    var corners = array<vec2<f32>, 3>(vec2<f32>(-1.0, -1.0), vec2<f32>(3.0, -1.0), vec2<f32>(-1.0, 3.0));
    return vec4<f32>(corners[i], 0.0, 1.0);
}

// The filtered texels, no longer weighed by alpha.
@fragment
fn store(@builtin(position) at: vec4<f32>) -> @location(0) vec4<f32> {
    let c = textureLoad(work, vec2<i32>(at.xy), 0);
    var color = vec3<f32>(0.0);
    if (c.a > 0.0) {
        color = c.rgb / c.a;
    }
    return vec4<f32>(color, c.a);
}
";

/// The work one submission may take: texels times the texels each reads.
/// A long pass runs in bands of rows, each submitted on its own, so no one
/// submission runs long enough for a driver's watchdog to end it.
const BAND: u64 = 1 << 24;

/// The rounds Canny's hysteresis is counted as running in a tile, for the
/// work a sweep of it takes: each round reads a texel's nine neighbors.
const LINK_ROUNDS: u64 = 16;

/// The layouts and pipelines a run of filters uses: the load for the
/// texture's dimension, the compute passes the filters run, and the render
/// pipeline for the texture's format.
struct Kit {
    layout: wgpu::BindGroupLayout,
    volume_layout: wgpu::BindGroupLayout,
    store_layout: wgpu::BindGroupLayout,
    load: wgpu::ComputePipeline,
    passes: Vec<(&'static str, wgpu::ComputePipeline)>,
    store: wgpu::RenderPipeline,
}

impl Kit {
    fn new(core: &CoreRef, format: F, volume: bool, entries: &[&'static str]) -> Kit {
        let entry = |binding: u32, ty: wgpu::BindingType| wgpu::BindGroupLayoutEntry { binding, visibility: wgpu::ShaderStages::COMPUTE, ty, count: None };
        let sampled = |binding: u32, dimension: wgpu::TextureViewDimension| {
            entry(binding, wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Float { filterable: false }, view_dimension: dimension, multisampled: false })
        };
        let written = entry(1, wgpu::BindingType::StorageTexture { access: wgpu::StorageTextureAccess::WriteOnly, format: F::Rgba32Float, view_dimension: wgpu::TextureViewDimension::D2 });
        let uniform = entry(2, wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None });
        let storage = |binding: u32, read_only: bool| entry(binding, wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Storage { read_only }, has_dynamic_offset: false, min_binding_size: None });
        let layout = core.device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Wgpu filter"),
            entries: &[sampled(0, wgpu::TextureViewDimension::D2), written, uniform, storage(3, true), sampled(4, wgpu::TextureViewDimension::D2), storage(5, false), storage(6, false)],
        });
        let volume_layout = core.device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor { label: Some("Wgpu filter"), entries: &[sampled(0, wgpu::TextureViewDimension::D3), written, uniform] });
        let store_layout = core.device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Wgpu filter"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Float { filterable: false }, view_dimension: wgpu::TextureViewDimension::D2, multisampled: false },
                count: None,
            }],
        });
        let shader = |text: &str| core.device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("Wgpu filter"), source: wgpu::ShaderSource::Wgsl(text.into()) });
        let pipeline_layout = |layout: &wgpu::BindGroupLayout| core.device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some("Wgpu filter"), bind_group_layouts: &[Some(layout)], immediate_size: 0 });
        let compute = |layout: &wgpu::PipelineLayout, module: &wgpu::ShaderModule, entry: &str| {
            core.device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some("Wgpu filter"),
                layout: Some(layout),
                module,
                entry_point: Some(entry),
                compilation_options: Default::default(),
                cache: None,
            })
        };
        let module = shader(COMPUTE);
        let flat = pipeline_layout(&layout);
        let load = if volume { compute(&pipeline_layout(&volume_layout), &shader(VOLUME_LOAD), "load") } else { compute(&flat, &module, "load") };
        let passes = entries.iter().map(|&entry| (entry, compute(&flat, &module, entry))).collect();
        let store_module = shader(STORE);
        let store = core.device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("Wgpu filter"),
            layout: Some(&pipeline_layout(&store_layout)),
            vertex: wgpu::VertexState { module: &store_module, entry_point: Some("vertex"), compilation_options: Default::default(), buffers: &[] },
            fragment: Some(wgpu::FragmentState {
                module: &store_module,
                entry_point: Some("store"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState { format, blend: None, write_mask: wgpu::ColorWrites::ALL })],
            }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });
        Kit { layout, volume_layout, store_layout, load, passes, store }
    }

    /// The pipeline of `entry`, one the kit was made with.
    fn pipeline(&self, entry: &str) -> &wgpu::ComputePipeline {
        &self.passes.iter().find(|(e, _)| *e == entry).expect("a pass runs an entry the kit was made with").1
    }
}

/// The two work textures besides `at`, of three.
fn others(at: usize) -> (usize, usize) {
    ((at + 1) % 3, (at + 2) % 3)
}

/// The 32-bit float textures and the buffers a run of filters reads and
/// writes, and the values every pass shares.
struct Work<'a> {
    core: &'a CoreRef,
    kit: Kit,
    views: Vec<wgpu::TextureView>,
    state: wgpu::Buffer,
    changed: wgpu::Buffer,
    none: wgpu::Buffer,
    params: Params,
    /// The work one submission may take, as BAND says.
    band: u64,
}

impl<'a> Work<'a> {
    /// The work for `filters` over `texture`, whose texels the gradients
    /// read as sRGB when `srgb`, `band` the work a submission may take:
    /// two work textures, or three for the unsharp mask and for Canny,
    /// which keep the texels they started from.
    fn new(core: &'a CoreRef, texture: &wgpu::Texture, filters: &Filters, srgb: bool, band: u64) -> Work<'a> {
        let (width, height) = (texture.width(), texture.height());
        let format = texture.format();
        let count = if filters.sharpen.is_some() || filters.canny() { 3 } else { 2 };
        let views = (0..count)
            .map(|_| {
                core.device
                    .create_texture(&wgpu::TextureDescriptor {
                        label: Some("Wgpu filter"),
                        size: wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
                        mip_level_count: 1,
                        sample_count: 1,
                        dimension: wgpu::TextureDimension::D2,
                        format: F::Rgba32Float,
                        usage: wgpu::TextureUsages::STORAGE_BINDING | wgpu::TextureUsages::TEXTURE_BINDING,
                        view_formats: &[],
                    })
                    .create_view(&wgpu::TextureViewDescriptor::default())
            })
            .collect();
        let buffer = |size: u64, usage: wgpu::BufferUsages| core.device.create_buffer(&wgpu::BufferDescriptor { label: Some("Wgpu filter"), size, usage, mapped_at_creation: false });
        let state_size = if filters.canny() { u64::from(width) * u64::from(height) * 4 } else { 4 };
        Work {
            core,
            kit: Kit::new(core, format, texture.dimension() == wgpu::TextureDimension::D3, &filters.entries()),
            views,
            state: buffer(state_size, wgpu::BufferUsages::STORAGE),
            changed: buffer(4, wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC | wgpu::BufferUsages::COPY_DST),
            none: buffer(4, wgpu::BufferUsages::STORAGE),
            params: Params {
                width,
                height,
                edge: filters.edge.code(),
                kernel: filters.edges.map_or(0, |e| e.operator.code()),
                srgb: u32::from(srgb),
                channels: u32::from(format.components()),
                ..Params::default()
            },
            band,
        }
    }

    /// The rows a band takes at `taps` texels read a texel: as many as the
    /// band allows, in whole tiles of Canny's hysteresis, and at least one
    /// tile.
    fn rows(&self, taps: u64) -> u32 {
        let tiles = (self.band / (u64::from(self.params.width) * taps.max(1)) / 16).max(1);
        (tiles * 16).min(u64::from(self.params.height)) as u32
    }

    fn uniform(&self, contents: &[u8]) -> wgpu::Buffer {
        self.core.device.create_buffer_init(&wgpu::util::BufferInitDescriptor { label: Some("Wgpu filter"), contents, usage: wgpu::BufferUsages::UNIFORM })
    }

    /// The bind group of a pass reading `from` and `other` and writing `to`,
    /// with `uniform` and `weights`.
    fn group(&self, from: &wgpu::TextureView, other: &wgpu::TextureView, to: &wgpu::TextureView, uniform: &wgpu::Buffer, weights: &wgpu::Buffer) -> wgpu::BindGroup {
        self.core.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Wgpu filter"),
            layout: &self.kit.layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(from) },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(to) },
                wgpu::BindGroupEntry { binding: 2, resource: uniform.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 3, resource: weights.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 4, resource: wgpu::BindingResource::TextureView(other) },
                wgpu::BindGroupEntry { binding: 5, resource: self.state.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 6, resource: self.changed.as_entire_binding() },
            ],
        })
    }

    /// Submits what `record` records into a new encoder, with any error it
    /// raises reported.
    fn submit(&self, stop: Option<&dyn Interrupt>, record: impl FnOnce(&mut wgpu::CommandEncoder)) -> PsResult<()> {
        let commands = self.core.scoped("WgpuFilter", "the texture cannot be filtered", || {
            let mut encoder = self.core.device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("Wgpu filter") });
            record(&mut encoder);
            encoder.finish()
        })?;
        self.core.submit(vec![commands], stop)?;
        Ok(())
    }

    /// Dispatches `pipeline` with `group` over `width` texels and `rows`
    /// rows, in workgroups of `tile` by `tile` texels.
    fn dispatch(encoder: &mut wgpu::CommandEncoder, pipeline: &wgpu::ComputePipeline, group: &wgpu::BindGroup, width: u32, rows: u32, tile: u32) {
        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor { label: Some("Wgpu filter"), timestamp_writes: None });
        pass.set_pipeline(pipeline);
        pass.set_bind_group(0, group, &[]);
        pass.dispatch_workgroups(width.div_ceil(tile), rows.div_ceil(tile), 1);
    }

    /// Runs `entry` reading work textures `from` and `other` and writing
    /// `to`, in bands of rows each submitted on its own, at `taps` texels
    /// read a texel.
    #[allow(clippy::too_many_arguments)]
    fn run(&self, entry: &str, params: Params, from: usize, other: usize, to: usize, weights: Option<&wgpu::Buffer>, taps: u64, stop: Option<&dyn Interrupt>) -> PsResult<()> {
        // Canny's hysteresis runs in workgroups of 16 by 16 texels, the
        // other passes in 8 by 8.
        let tile = if entry == "link" { 16 } else { 8 };
        let rows = self.rows(taps);
        let mut row = 0;
        while row < params.height {
            let count = rows.min(params.height - row);
            self.submit(stop, |encoder| {
                let uniform = self.uniform(&Params { row, ..params }.bytes());
                let group = self.group(&self.views[from], &self.views[other], &self.views[to], &uniform, weights.unwrap_or(&self.none));
                Work::dispatch(encoder, self.kit.pipeline(entry), &group, params.width, count, tile);
            })?;
            row += count;
        }
        Ok(())
    }

    /// Loads `plane`, one layer of a 2D texture, or a 3D texture viewed whole
    /// at depth slice `layer`, into work texture 0, linear and weighed by
    /// alpha.
    fn load(&self, plane: &wgpu::TextureView, volume: bool, layer: u32, stop: Option<&dyn Interrupt>) -> PsResult<()> {
        let (width, height) = (self.params.width, self.params.height);
        let rows = self.rows(1);
        let mut row = 0;
        while row < height {
            let count = rows.min(height - row);
            self.submit(stop, |encoder| {
                let group = if volume {
                    let uniform = self.uniform(bytemuck::cast_slice(&[width, height, layer, row]));
                    self.core.device.create_bind_group(&wgpu::BindGroupDescriptor {
                        label: Some("Wgpu filter"),
                        layout: &self.kit.volume_layout,
                        entries: &[
                            wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(plane) },
                            wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(&self.views[0]) },
                            wgpu::BindGroupEntry { binding: 2, resource: uniform.as_entire_binding() },
                        ],
                    })
                } else {
                    let uniform = self.uniform(&Params { row, ..self.params }.bytes());
                    self.group(plane, plane, &self.views[0], &uniform, &self.none)
                };
                Work::dispatch(encoder, &self.kit.load, &group, width, count, 8);
            })?;
            row += count;
        }
        Ok(())
    }

    /// Blurs work texture `from` by standard deviations `sigmas` across and
    /// down, through `spare`, into `to`, which may be `from`.
    fn blur(&self, from: usize, spare: usize, to: usize, sigmas: [f64; 2], stop: Option<&dyn Interrupt>) -> PsResult<()> {
        for (axis, sigma) in sigmas.into_iter().enumerate() {
            let weights = gaussian_weights(sigma);
            let buffer = self.core.scoped("WgpuFilter", "the texture cannot be filtered", || {
                self.core.device.create_buffer_init(&wgpu::util::BufferInitDescriptor { label: Some("Wgpu filter"), contents: bytemuck::cast_slice(&weights), usage: wgpu::BufferUsages::STORAGE })
            })?;
            let params = Params { radius: (weights.len() / 2) as u32, axis: axis as u32, ..self.params };
            let (read, write) = if axis == 0 { (from, spare) } else { (spare, to) };
            self.run("blur", params, read, read, write, Some(&buffer), weights.len() as u64, stop)?;
        }
        Ok(())
    }

    /// Runs Canny's hysteresis until a sweep changes nothing, four sweeps at
    /// a time, reading back whether the last of the four changed a texel.
    /// The sweeps bind work textures `from` and `to`, which they leave alone.
    fn link(&self, from: usize, to: usize, stop: Option<&dyn Interrupt>) -> PsResult<()> {
        loop {
            for sweep in 0..4 {
                if sweep == 3 {
                    self.submit(stop, |encoder| encoder.clear_buffer(&self.changed, 0, None))?;
                }
                self.run("link", self.params, from, from, to, None, 9 * LINK_ROUNDS, stop)?;
            }
            if buffer::read_bytes(self.core, &self.changed, 0, 4, stop)?.iter().all(|b| *b == 0) {
                return Ok(());
            }
        }
    }

    /// Writes work texture `at` into `plane`, at depth slice `slice` of a 3D
    /// texture, no longer weighed by alpha, in bands of rows each submitted
    /// on its own.
    fn store(&self, plane: &wgpu::TextureView, at: usize, slice: Option<u32>, stop: Option<&dyn Interrupt>) -> PsResult<()> {
        let (width, height) = (self.params.width, self.params.height);
        let group = self.core.scoped("WgpuFilter", "the texture cannot be filtered", || {
            self.core.device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("Wgpu filter"),
                layout: &self.kit.store_layout,
                entries: &[wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&self.views[at]) }],
            })
        })?;
        let rows = self.rows(1);
        let mut row = 0;
        while row < height {
            let count = rows.min(height - row);
            self.submit(stop, |encoder| {
                let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("Wgpu filter"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: plane,
                        depth_slice: slice,
                        resolve_target: None,
                        ops: wgpu::Operations { load: wgpu::LoadOp::Load, store: wgpu::StoreOp::Store },
                    })],
                    depth_stencil_attachment: None,
                    timestamp_writes: None,
                    occlusion_query_set: None,
                    multiview_mask: None,
                });
                pass.set_pipeline(&self.kit.store);
                pass.set_bind_group(0, &group, &[]);
                pass.set_scissor_rect(0, row, width, count);
                pass.draw(0..3, 0..1);
            })?;
            row += count;
        }
        Ok(())
    }
}

/// Runs `filters` over level 0 of `texture`, a texture the transform drew
/// in a float format, in place: each layer of an array or a cube map, and
/// each depth slice of a 3D texture, on its own. `srgb` says the texture's
/// texels are kept as sRGB, whose encoded values the gradients and the
/// Laplacian read.
pub(crate) fn apply(core: &CoreRef, texture: &wgpu::Texture, filters: &Filters, srgb: bool, stop: Option<&dyn Interrupt>) -> PsResult<()> {
    apply_in_bands(core, texture, filters, srgb, BAND, stop)
}

/// `apply`, each submission taking at most `band` work as BAND counts it.
fn apply_in_bands(core: &CoreRef, texture: &wgpu::Texture, filters: &Filters, srgb: bool, band: u64, stop: Option<&dyn Interrupt>) -> PsResult<()> {
    let volume = texture.dimension() == wgpu::TextureDimension::D3;
    let work = core.scoped("WgpuFilter", "the texture cannot be filtered", || Work::new(core, texture, filters, srgb, band))?;
    // The work texture holding the texels after the blur and the unsharp
    // mask, and the two the edges are found through.
    let at = if filters.sharpen.is_some() { others(0).0 } else { 0 };
    let (first, second) = others(at);
    for layer in 0..texture.depth_or_array_layers() {
        let plane = core.scoped("WgpuFilter", "the texture cannot be filtered", || {
            texture.create_view(&wgpu::TextureViewDescriptor {
                label: Some("Wgpu filter"),
                dimension: Some(if volume { wgpu::TextureViewDimension::D3 } else { wgpu::TextureViewDimension::D2 }),
                base_mip_level: 0,
                mip_level_count: Some(1),
                base_array_layer: if volume { 0 } else { layer },
                array_layer_count: Some(1),
                ..Default::default()
            })
        })?;
        work.load(&plane, volume, layer, stop)?;
        if let Some(sigmas) = filters.blur {
            work.blur(0, others(0).0, 0, sigmas, stop)?;
        }
        if let Some(s) = filters.sharpen {
            let (spare, blurred) = others(0);
            work.blur(0, spare, blurred, [s.radius; 2], stop)?;
            let mask = Params { amount: s.amount as f32, threshold: s.threshold as f32, ..work.params };
            work.run("sharpen", mask, 0, blurred, spare, None, 1, stop)?;
        }
        let last = match filters.edges {
            None => at,
            Some(e) if e.operator == EdgeOperator::Canny => {
                work.run("luma", work.params, at, at, first, None, 1, stop)?;
                work.blur(first, second, first, [e.sigma; 2], stop)?;
                work.run("sobel", work.params, first, first, second, None, 9, stop)?;
                let thresholds = Params { low: e.low as f32, high: e.high as f32, ..work.params };
                work.run("suppress", thresholds, second, second, first, None, 9, stop)?;
                work.link(second, first, stop)?;
                work.run("edges", work.params, at, at, first, None, 1, stop)?;
                first
            }
            Some(_) => {
                work.run("gradient", work.params, at, at, first, None, 9, stop)?;
                first
            }
        };
        work.store(&plane, last, volume.then_some(layer), stop)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::convert::MipOptions;
    use crate::device::{self, AdapterQuery, InstanceOptions, OpenOptions};
    use crate::filter_reference as reference;
    use crate::texfile::{Said, TextureFile};
    use crate::transform::{self, Packed, Transform};
    use wgpu::TextureDimension::{D2, D3};

    const W: u32 = reference::WIDTH;
    const H: u32 = reference::HEIGHT;

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

    /// The color test image tools/filter_reference.py builds, rows of
    /// float32 red, green, blue and alpha: alpha from 0 to 1, or 1 where
    /// `opaque`.
    fn color_image(opaque: bool) -> Vec<u8> {
        let mut texels = Vec::new();
        for y in 0..H {
            for x in 0..W {
                let (fx, fy) = (f64::from(x), f64::from(y));
                let r = f64::from((x * 5 + y * 3) % 17) / 16.0;
                let g = f64::from((x * x + 2 * y) % 13) / 16.0;
                let b = ((101.25 - (fx - 19.375).powi(2) - (fy - 15.625).powi(2)) / 8.0).clamp(0.0, 1.0);
                let a = if opaque { 1.0 } else { f64::from((x * 3 + y) % 5) / 4.0 };
                texels.extend([r as f32, g as f32, b as f32, a as f32]);
            }
        }
        bytemuck::cast_slice(&texels).to_vec()
    }

    /// The gray test image tools/filter_reference.py builds, float32: a
    /// disc, a faint disc and a step whose contrast fades along it.
    fn gray_image() -> Vec<u8> {
        let mut texels = Vec::new();
        for y in 0..H {
            for x in 0..W {
                let (fx, fy) = (f64::from(x), f64::from(y));
                let disc = ((49.5 - (fx - 12.375).powi(2) - (fy - 11.625).powi(2)) / 4.0).clamp(0.0, 1.0);
                let faint = 0.15625 * ((20.25 - (fx - 30.625).powi(2) - (fy - 9.375).powi(2)) / 4.0).clamp(0.0, 1.0);
                let step = (1.0 - fx * 0.0234375) * ((fy - 22.0 - fx * 0.3125) * 2.0 + 0.5).clamp(0.0, 1.0);
                texels.push((disc + faint + step).clamp(0.0, 1.0) as f32);
            }
        }
        bytemuck::cast_slice(&texels).to_vec()
    }

    /// The sRGB test texture's stored bytes, as tools/filter_reference.py
    /// builds them.
    fn stored_bytes() -> Vec<u8> {
        let mut bytes = Vec::new();
        for y in 0..H {
            for x in 0..W {
                bytes.extend([(x * 37 + y * 91) % 256, (x * x * 3 + y * 5) % 256, (x * y * 7) % 256, 255].map(|v| u8::try_from(v).expect("a byte")));
            }
        }
        bytes
    }

    /// `texels` of `format`, `planes` layers or, for a 3D texture, depth
    /// slices of `width` by `height`, through the transform with `filters`,
    /// and the notes it writes.
    fn filtered(core: &CoreRef, format: F, dimension: wgpu::TextureDimension, (width, height): (u32, u32), planes: u32, texels: Vec<u8>, filters: Filters) -> (Vec<u8>, Said) {
        let (depth, layers) = if dimension == D3 { (planes, 1) } else { (1, planes) };
        let file = TextureFile { dimension, width, height, depth, layers, cube: false, format, levels: vec![texels] };
        let transform = Transform { filters: Some(filters), ..Default::default() };
        let mut said = Said::default();
        let out = transform::apply(core, file, 1, MipOptions::default(), &transform, Packed::Own, None, None, None, &mut said).expect("filtered");
        assert_eq!((out.format, out.width, out.height), (format, width, height));
        (out.levels.into_iter().next().expect("level 0"), said)
    }

    /// Where plane `plane` of `got`, rows of float32 red, green, blue and
    /// alpha of the test images' size, differs at reference::SPOTS from
    /// `want` by more than `tolerance`.
    fn differences(what: &str, got: &[u8], plane: usize, want: &[[f32; 4]], tolerance: f32) -> Vec<String> {
        let floats: Vec<f32> = bytemuck::pod_collect_to_vec(got);
        let base = plane * (W * H * 4) as usize;
        let mut wrong = Vec::new();
        for (&(x, y), want) in reference::SPOTS.iter().zip(want) {
            let at = base + ((y * W + x) * 4) as usize;
            let texel = &floats[at..at + 4];
            if texel.iter().zip(want).any(|(g, w)| (g - w).abs() > tolerance) {
                wrong.push(format!("{what}: texel ({x}, {y}) is {texel:?}, and the reference {want:?}"));
            }
        }
        wrong
    }

    /// A row of 0s and 1s for each row of an edge map of float32 texels of
    /// `channels` channels, with a ? for a texel that is neither.
    fn edge_rows(got: &[u8], width: u32, channels: usize) -> Vec<String> {
        let floats: Vec<f32> = bytemuck::pod_collect_to_vec(got);
        let row = width as usize * channels;
        floats
            .chunks(row)
            .map(|texels| {
                texels
                    .chunks(channels)
                    .map(|t| if t[0] == 1.0 {
                        '1'
                    } else if t[0] == 0.0 {
                        '0'
                    } else {
                        '?'
                    })
                    .collect()
            })
            .collect()
    }

    /// Canny with the standard deviation `sigma` and thresholds `low` and
    /// `high`, reading past the edge as `edge` says.
    fn canny(sigma: f64, low: f64, high: f64, edge: FilterEdge) -> Filters {
        Filters { edges: Some(Edges { operator: EdgeOperator::Canny, low, high, sigma }), edge, ..Filters::default() }
    }

    /// The weights reach 4 standard deviations, rounded as scipy rounds
    /// them, are symmetric, and sum to 1.
    #[test]
    fn weighs_a_gaussian_as_scipy_cuts_it() {
        for (sigma, taps) in [(0.1, 1), (0.5, 5), (1.0, 9), (1.6, 13), (2.0, 17)] {
            let weights = gaussian_weights(sigma);
            assert_eq!(weights.len(), taps, "sigma {sigma}");
            let total: f32 = weights.iter().sum();
            assert!((total - 1.0).abs() < 1e-6, "sigma {sigma}: {total}");
            assert!(weights.iter().zip(weights.iter().rev()).all(|(a, b)| a == b), "sigma {sigma}");
        }
        let one = gaussian_weights(1.0);
        let expected = (-0.5f64).exp() / (0..9).map(|x: i32| (-0.5 * f64::from(x - 4).powi(2)).exp()).sum::<f64>();
        assert!((f64::from(one[5]) - expected).abs() < 1e-7, "{one:?}");
    }

    /// The parameters name the filters they give, with Canny's and the
    /// unsharp mask's defaults, and refuse a value out of range and a
    /// setting without its filter.
    #[test]
    fn names_the_filters_the_parameters_give_and_refuses_the_rest() {
        assert_eq!(Asked::default().filters().expect("none"), None);
        let blur = Asked { blur: Some(&[2.0]), ..Asked::default() }.filters().expect("a blur").expect("filters");
        assert_eq!((blur.blur, blur.edge), (Some([2.0, 2.0]), FilterEdge::Clamp));
        let both = Asked { blur: Some(&[4.0, 1.0]), edge: Some(FilterEdge::Wrap), ..Asked::default() }.filters().expect("a blur").expect("filters");
        assert_eq!((both.blur, both.edge), (Some([4.0, 1.0]), FilterEdge::Wrap));
        let sharpen = Asked { sharpen: Some(1.5), ..Asked::default() }.filters().expect("a mask").expect("filters");
        assert_eq!(sharpen.sharpen, Some(Sharpen { amount: 1.5, radius: 1.0, threshold: 0.0 }));
        let canny = Asked { edges: Some(EdgeOperator::Canny), ..Asked::default() }.filters().expect("edges").expect("filters");
        assert_eq!(canny.edges, Some(Edges { operator: EdgeOperator::Canny, low: 0.1, high: 0.2, sigma: 1.0 }));
        let check = |asked: Asked<'_>, id: &str, message: &str| {
            let e = asked.filters().expect_err("refused");
            assert_eq!((e.error_id.as_str(), e.message.as_str()), (id, message));
        };
        check(Asked { blur: Some(&[0.0]), ..Asked::default() }, "WgpuBlur", "-Blur takes standard deviations above 0, and 0 is not");
        check(Asked { blur: Some(&[1.0, 2.0, 3.0]), ..Asked::default() }, "WgpuBlur", "-Blur takes one standard deviation, or two, across and down, and was given 3");
        check(Asked { edges: Some(EdgeOperator::Canny), edge_threshold: Some(&[0.1]), ..Asked::default() }, "WgpuEdges", "-EdgeThreshold takes two numbers, Canny's low and high thresholds, and was given 1");
        check(Asked { sharpen: Some(-1.0), ..Asked::default() }, "WgpuSharpen", "-Sharpen is how much of the detail to add, above 0, and -1 is not");
        check(Asked { sharpen: Some(1.0), sharpen_threshold: Some(1.5), ..Asked::default() }, "WgpuSharpen", "-SharpenThreshold is a difference from 0 to 1, and 1.5 is not");
        check(Asked { sharpen_radius: Some(2.0), ..Asked::default() }, "WgpuSharpen", "-SharpenRadius and -SharpenThreshold say how -Sharpen sharpens: give -Sharpen");
        check(Asked { edges: Some(EdgeOperator::Sobel), edge_sigma: Some(2.0), ..Asked::default() }, "WgpuEdges", "-EdgeThreshold and -EdgeSigma say how Canny finds edges: give -Edges Canny");
        check(
            Asked { edges: Some(EdgeOperator::Canny), edge_threshold: Some(&[0.3, 0.2]), ..Asked::default() },
            "WgpuEdges",
            "-EdgeThreshold is Canny's low threshold, 0 or above, and its high one, no lower, and 0.3 and 0.2 are not",
        );
        check(Asked { edge: Some(FilterEdge::Mirror), ..Asked::default() }, "WgpuFilterEdge", "-FilterEdge says what a filter reads past the edge of the texture, and no filter was given");
    }

    /// Each blur is scipy's gaussian_filter1d across, then down, truncated
    /// at 4 standard deviations, of color weighed by alpha, with the mode
    /// of each FilterEdge, including blurs reaching past the far side.
    #[test]
    fn blurs_as_scipy_with_each_edge() {
        let Some(core) = core() else { return };
        let cases = [
            ("Clamp", [1.5, 0.75], FilterEdge::Clamp, &reference::BLUR_CLAMP),
            ("Wrap", [2.0, 2.0], FilterEdge::Wrap, &reference::BLUR_WRAP),
            ("Mirror", [0.625, 1.75], FilterEdge::Mirror, &reference::BLUR_MIRROR),
            ("Wrap past the far side", [12.0, 10.0], FilterEdge::Wrap, &reference::BLUR_WRAP_WIDE),
            ("Mirror past the far side", [12.0, 10.0], FilterEdge::Mirror, &reference::BLUR_MIRROR_WIDE),
        ];
        let mut wrong = Vec::new();
        for (what, sigmas, edge, want) in cases {
            let filters = Filters { blur: Some(sigmas), edge, ..Filters::default() };
            let (got, said) = filtered(&core, F::Rgba32Float, D2, (W, H), 1, color_image(false), filters);
            assert_eq!(said.verbose, filters.said(), "{what}");
            wrong.extend(differences(what, &got, 0, want, 5e-5));
        }
        assert!(wrong.is_empty(), "{}", wrong.join("\n"));
    }

    /// The unsharp mask adds amount times each channel's difference from
    /// its blur where that is at least the threshold, as the formula over
    /// scipy's blur gives, and leaves a transparent texel transparent black.
    #[test]
    fn sharpens_as_the_unsharp_formula_gives() {
        let Some(core) = core() else { return };
        let cases = [
            ("Clamp", Sharpen { amount: 1.5, radius: 1.25, threshold: 0.046875 }, FilterEdge::Clamp, &reference::SHARPEN_CLAMP),
            ("Wrap", Sharpen { amount: 0.75, radius: 2.0, threshold: 0.0 }, FilterEdge::Wrap, &reference::SHARPEN_WRAP),
        ];
        let mut wrong = Vec::new();
        for (what, sharpen, edge, want) in cases {
            let filters = Filters { sharpen: Some(sharpen), edge, ..Filters::default() };
            let (got, said) = filtered(&core, F::Rgba32Float, D2, (W, H), 1, color_image(false), filters);
            assert_eq!(said.verbose, filters.said(), "{what}");
            wrong.extend(differences(what, &got, 0, want, 1e-4));
        }
        assert!(wrong.is_empty(), "{}", wrong.join("\n"));
    }

    /// Sobel's, Scharr's and Prewitt's gradients are scipy's correlate1d
    /// of [-1, 0, 1] and each one's weights, divided by their sum, and the
    /// Laplacian is scipy's laplace, of the values the texture stores.
    #[test]
    fn finds_gradients_as_scipy_correlates() {
        let Some(core) = core() else { return };
        let cases = [
            ("Sobel, Clamp", EdgeOperator::Sobel, FilterEdge::Clamp, &reference::SOBEL_CLAMP),
            ("Scharr, Clamp", EdgeOperator::Scharr, FilterEdge::Clamp, &reference::SCHARR_CLAMP),
            ("Prewitt, Clamp", EdgeOperator::Prewitt, FilterEdge::Clamp, &reference::PREWITT_CLAMP),
            ("Sobel, Wrap", EdgeOperator::Sobel, FilterEdge::Wrap, &reference::SOBEL_WRAP),
            ("Sobel, Mirror", EdgeOperator::Sobel, FilterEdge::Mirror, &reference::SOBEL_MIRROR),
            ("Laplacian, Clamp", EdgeOperator::Laplacian, FilterEdge::Clamp, &reference::LAPLACIAN_CLAMP),
        ];
        let mut wrong = Vec::new();
        for (what, operator, edge, want) in cases {
            let filters = Filters { edges: Some(Edges { operator, low: 0.1, high: 0.2, sigma: 1.0 }), edge, ..Filters::default() };
            let (got, said) = filtered(&core, F::Rgba32Float, D2, (W, H), 1, color_image(true), filters);
            assert_eq!(said.verbose, filters.said(), "{what}");
            wrong.extend(differences(what, &got, 0, want, 2e-5));
        }
        assert!(wrong.is_empty(), "{}", wrong.join("\n"));
    }

    /// An sRGB texture's gradient is of the values it stores: what it
    /// stores after is within a step of scipy's gradient of the bytes.
    #[test]
    fn finds_the_gradient_of_what_an_srgb_texture_stores() {
        let Some(core) = core() else { return };
        let filters = Filters { edges: Some(Edges { operator: EdgeOperator::Sobel, low: 0.1, high: 0.2, sigma: 1.0 }), ..Filters::default() };
        let (got, said) = filtered(&core, F::Rgba8UnormSrgb, D2, (W, H), 1, stored_bytes(), filters);
        assert!(said.verbose.starts_with(&filters.said()), "{said:?}");
        let mut wrong = Vec::new();
        for (&(x, y), want) in reference::SPOTS.iter().zip(&reference::SOBEL_SRGB_STORED) {
            let at = ((y * W + x) * 4) as usize;
            let texel = &got[at..at + 4];
            if texel.iter().zip(want).any(|(g, w)| (f32::from(*g) - w).abs() > 1.0) {
                wrong.push(format!("texel ({x}, {y}) stores {texel:?}, and the reference {want:?}"));
            }
        }
        assert!(wrong.is_empty(), "{}", wrong.join("\n"));
    }

    /// Canny's edges are scikit-image's, texel for texel, with each
    /// FilterEdge for its smoothing: the faint disc's weak edge is dropped,
    /// and the step's weak end is kept where it links to its strong part.
    #[test]
    fn finds_canny_edges_as_scikit_image_does() {
        let Some(core) = core() else { return };
        let cases = [
            ("Clamp", canny(1.0, 0.1, 0.2, FilterEdge::Clamp), &reference::CANNY_CLAMP),
            ("Wrap", canny(1.0, 0.09375, 0.1875, FilterEdge::Wrap), &reference::CANNY_WRAP),
            ("Mirror", canny(1.0, 0.1, 0.2, FilterEdge::Mirror), &reference::CANNY_MIRROR),
            ("Clamp, wider", canny(1.375, 0.0625, 0.15625, FilterEdge::Clamp), &reference::CANNY_WIDE),
        ];
        for (what, filters, want) in cases {
            let (got, said) = filtered(&core, F::R32Float, D2, (W, H), 1, gray_image(), filters);
            assert_eq!(said.verbose, filters.said(), "{what}");
            let rows = edge_rows(&got, W, 1);
            assert_eq!(rows, want, "{what}:\n{}", rows.iter().zip(want).map(|(g, w)| format!("{g}  {w}")).collect::<Vec<_>>().join("\n"));
        }
    }

    /// With room for no more than a tile of rows a submission, every pass
    /// runs in bands of 16 rows and gives what one band gives.
    #[test]
    fn filters_in_bands_of_rows_as_in_one() {
        let Some(core) = core() else { return };
        let texture = |format: F, texels: &[u8]| {
            let made = core.device.create_texture(&wgpu::TextureDescriptor {
                label: Some("Wgpu filter test"),
                size: wgpu::Extent3d { width: W, height: H, depth_or_array_layers: 1 },
                mip_level_count: 1,
                sample_count: 1,
                dimension: D2,
                format,
                usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC | wgpu::TextureUsages::COPY_DST,
                view_formats: &[],
            });
            crate::texture::write_texels(&core, &made, 0, 0, wgpu::TextureAspect::All, texels).expect("written");
            made
        };
        let banded = |format: F, texels: &[u8], filters: Filters| {
            let made = texture(format, texels);
            apply_in_bands(&core, &made, &filters, false, 1, None).expect("filtered in bands");
            crate::image_io::read_level(&core, &made, 0, None).expect("read back")
        };
        let blur = Filters { blur: Some([1.5, 0.75]), ..Filters::default() };
        let mut wrong = differences("Blur", &banded(F::Rgba32Float, &color_image(false), blur), 0, &reference::BLUR_CLAMP, 5e-5);
        let sharpen = Filters { sharpen: Some(Sharpen { amount: 1.5, radius: 1.25, threshold: 0.046875 }), ..Filters::default() };
        wrong.extend(differences("Sharpen", &banded(F::Rgba32Float, &color_image(false), sharpen), 0, &reference::SHARPEN_CLAMP, 1e-4));
        assert!(wrong.is_empty(), "{}", wrong.join("\n"));
        let rows = edge_rows(&banded(F::R32Float, &gray_image(), canny(1.0, 0.1, 0.2, FilterEdge::Clamp)), W, 1);
        assert_eq!(rows, reference::CANNY_CLAMP);
    }

    /// Each layer of an array and each depth slice of a 3D texture is
    /// filtered on its own.
    #[test]
    fn filters_each_layer_and_depth_slice_on_its_own() {
        let Some(core) = core() else { return };
        let filters = Filters { blur: Some([1.5, 0.75]), ..Filters::default() };
        let planes = [color_image(false), color_image(true)].concat();
        let mut wrong = Vec::new();
        for dimension in [D2, D3] {
            let (got, _) = filtered(&core, F::Rgba32Float, dimension, (W, H), 2, planes.clone(), filters);
            wrong.extend(differences(&format!("{dimension:?} plane 0"), &got, 0, &reference::BLUR_CLAMP, 5e-5));
            wrong.extend(differences(&format!("{dimension:?} plane 1"), &got, 1, &reference::BLUR_CLAMP_OPAQUE, 5e-5));
        }
        assert!(wrong.is_empty(), "{}", wrong.join("\n"));
    }

    /// A texture smaller than a workgroup, a tile or Canny's ring is
    /// filtered whole: Canny finds no edge in it, and a blur of one texel
    /// gives that texel.
    #[test]
    fn filters_textures_of_a_texel_or_two() {
        let Some(core) = core() else { return };
        for (width, height) in [(1, 1), (2, 3), (17, 1), (3, 18)] {
            let texels: Vec<f32> = (0..width * height).map(|i| if i % 2 == 0 { 0.0 } else { 1.0 }).collect();
            let (got, _) = filtered(&core, F::R32Float, D2, (width, height), 1, bytemuck::cast_slice(&texels).to_vec(), canny(1.0, 0.1, 0.2, FilterEdge::Clamp));
            assert!(edge_rows(&got, width, 1).iter().all(|row| row.chars().all(|c| c == '0')), "{width}x{height}: {:?}", edge_rows(&got, width, 1));
        }
        let one = [0.25f32, 0.5, 0.75, 0.5];
        let (got, _) = filtered(&core, F::Rgba32Float, D2, (1, 1), 1, bytemuck::cast_slice(&one).to_vec(), Filters { blur: Some([3.0, 3.0]), ..Filters::default() });
        let back: Vec<f32> = bytemuck::pod_collect_to_vec(&got);
        assert!(back.iter().zip(one).all(|(g, w)| (g - w).abs() < 1e-6), "{back:?}");
    }
}
