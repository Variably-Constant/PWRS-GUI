//! Normal maps: a height map's slopes made into normals by a render pass,
//! and how every step that reads or writes normals stores them. A format
//! that holds no negative values stores a normal n as n / 2 + 1 / 2, and
//! any other stores n as it is. Green points up the image, as glTF reads
//! it, unless a step flips it.

use pwrs::prelude::*;
use wgpu::util::DeviceExt;
use wgpu::TextureFormat as F;

use crate::convert::{MipEdge, MipFilter, MipOptions};
use crate::device::CoreRef;
use crate::error::invalid;
use crate::image_io;
use crate::tasks::Interrupt;
use crate::texel;
use crate::texfile::Said;
use crate::texture::{tell, Texture};

/// The channel of a height map its heights are read from.
#[psenum(name = "Wgpu.HeightChannel")]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum HeightChannel {
    /// Red, which a grayscale height map stores its heights in, as it does
    /// in every channel.
    #[default]
    Red,
    /// Green.
    Green,
    /// Blue.
    Blue,
    /// Alpha.
    Alpha,
    /// The luminance of red, green and blue by texconv's weights: 0.2125,
    /// 0.7154 and 0.0721.
    Luminance,
}

impl HeightChannel {
    /// The number the shader knows the channel by.
    fn code(self) -> u32 {
        match self {
            HeightChannel::Red => 0,
            HeightChannel::Green => 1,
            HeightChannel::Blue => 2,
            HeightChannel::Alpha => 3,
            HeightChannel::Luminance => 4,
        }
    }

    /// The height `color` holds, read as the shader reads it.
    fn of(self, color: &[f32; 4]) -> f32 {
        match self {
            HeightChannel::Red => color[0],
            HeightChannel::Green => color[1],
            HeightChannel::Blue => color[2],
            HeightChannel::Alpha => color[3],
            HeightChannel::Luminance => 0.2125 * color[0] + 0.7154 * color[1] + 0.0721 * color[2],
        }
    }

    fn name(self) -> &'static str {
        match self {
            HeightChannel::Red => "red",
            HeightChannel::Green => "green",
            HeightChannel::Blue => "blue",
            HeightChannel::Alpha => "alpha",
            HeightChannel::Luminance => "the luminance of red, green and blue",
        }
    }
}

/// How a height map's slopes are taken from the heights around each
/// texel; each kernel is scaled so a steady ramp gives its own slope.
#[psenum(name = "Wgpu.NormalKernel")]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum NormalKernel {
    /// texconv's: across, the left-right differences in the texel's row and
    /// the rows above and below it, averaged; down, the up-down differences
    /// in its column and the columns beside it, averaged.
    #[default]
    Prewitt,
    /// The difference of the two neighbors alone.
    Central,
    /// The three differences of Prewitt weighted 1, 2 and 1.
    Sobel,
    /// NVIDIA Texture Tools' 5 by 5 kernel, which reaches two texels each
    /// way.
    Sobel5x5,
}

impl NormalKernel {
    fn code(self) -> u32 {
        match self {
            NormalKernel::Prewitt => 0,
            NormalKernel::Central => 1,
            NormalKernel::Sobel => 2,
            NormalKernel::Sobel5x5 => 3,
        }
    }
}

/// What a height map is read as past its edges.
#[psenum(name = "Wgpu.HeightEdge")]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum HeightEdge {
    /// The slope at the edge carried on: a texel past the edge is twice
    /// the edge texel's height less the height of the texel as far inside.
    #[default]
    Continue,
    /// The other side, for a height map that tiles.
    Wrap,
    /// The edge texel repeated.
    Clamp,
    /// The map mirrored at its edge, the edge texel repeated, as a
    /// MirrorRepeat sampler reads it.
    Mirror,
}

impl HeightEdge {
    fn code(self) -> u32 {
        match self {
            HeightEdge::Continue => 0,
            HeightEdge::Wrap => 1,
            HeightEdge::Clamp => 2,
            HeightEdge::Mirror => 3,
        }
    }

    fn said(self) -> &'static str {
        match self {
            HeightEdge::Continue => "the slope at each edge carried on past it",
            HeightEdge::Wrap => "the edges wrapped",
            HeightEdge::Clamp => "the edge texels repeated past the edges",
            HeightEdge::Mirror => "the map mirrored at its edges",
        }
    }
}

/// What a normal map's alpha holds.
#[psenum(name = "Wgpu.NormalAlpha")]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum NormalAlpha {
    /// 1.
    #[default]
    One,
    /// The height the texel's normal came from, which parallax mapping
    /// reads.
    Height,
    /// texconv's occlusion term: darker where the eight neighbors rise
    /// above the texel.
    Occlusion,
}

impl NormalAlpha {
    fn code(self) -> u32 {
        match self {
            NormalAlpha::One => 0,
            NormalAlpha::Height => 1,
            NormalAlpha::Occlusion => 2,
        }
    }

    fn said(self) -> &'static str {
        match self {
            NormalAlpha::One => "1",
            NormalAlpha::Height => "the height",
            NormalAlpha::Occlusion => "texconv's occlusion term",
        }
    }
}

/// How the mip levels of a texture of normals keep them unit length.
#[psenum(name = "Wgpu.NormalMips")]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum NormalMips {
    /// Each level filtered as -MipFilter says, then each texel's normal
    /// scaled back to length 1.
    #[default]
    Renormalize,
    /// Each normal's slopes, x / z and y / z, filtered as -MipFilter says
    /// and made a unit normal again, as NVIDIA's exporter filters normal
    /// maps; a z under 1/255 counts as 1/255, so no slope passes 255.
    SlopeSpace,
}

/// Whether `format` holds negative values: the snorm formats and the
/// float formats with a sign, which store a normal as it is.
pub(crate) fn signed(format: F) -> bool {
    matches!(
        format,
        F::R8Snorm
            | F::Rg8Snorm
            | F::Rgba8Snorm
            | F::R16Snorm
            | F::Rg16Snorm
            | F::Rgba16Snorm
            | F::R16Float
            | F::Rg16Float
            | F::Rgba16Float
            | F::R32Float
            | F::Rg32Float
            | F::Rgba32Float
            | F::Bc4RSnorm
            | F::Bc5RgSnorm
            | F::Bc6hRgbFloat
            | F::EacR11Snorm
            | F::EacRg11Snorm
    )
}

/// The format a texture of normals is kept in between a transform and
/// the conversion into `target`'s format: Rgba32Float where the target
/// holds negative values, and otherwise Rgba8Unorm for texels of 8 bits a
/// channel, `decoded`, or Rgba16Unorm for wider ones.
pub(crate) fn kept_in(target: F, decoded: F) -> F {
    if signed(target) {
        return F::Rgba32Float;
    }
    match decoded {
        F::R8Unorm | F::R8Snorm | F::Rg8Unorm | F::Rg8Snorm | F::Rgba8Unorm | F::Rgba8UnormSrgb | F::Rgba8Snorm | F::Bgra8Unorm | F::Bgra8UnormSrgb => F::Rgba8Unorm,
        _ => F::Rgba16Unorm,
    }
}

/// Heights made into normals: how steep, where the heights are read, the
/// kernel, what is read past the edges, what alpha holds, and whether
/// green points down.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct HeightToNormal {
    pub(crate) strength: f32,
    pub(crate) channel: HeightChannel,
    pub(crate) kernel: NormalKernel,
    pub(crate) edge: HeightEdge,
    pub(crate) alpha: NormalAlpha,
    pub(crate) flip_green: bool,
}

impl HeightToNormal {
    /// The step a call's strength and settings ask for, each setting it
    /// omits at its default; `parameter` names the strength in a refusal.
    pub(crate) fn new(
        strength: f64,
        parameter: &str,
        channel: Option<HeightChannel>,
        kernel: Option<NormalKernel>,
        edge: Option<HeightEdge>,
        alpha: Option<NormalAlpha>,
        flip_green: bool,
    ) -> PsResult<HeightToNormal> {
        if !strength.is_finite() {
            return Err(invalid("WgpuNormalMap", format!("{parameter} is a finite number, the slope a height step of 1 between neighboring texels gives, not {strength}")));
        }
        Ok(HeightToNormal {
            strength: strength as f32,
            channel: channel.unwrap_or(HeightChannel::Red),
            kernel: kernel.unwrap_or(NormalKernel::Prewitt),
            edge: edge.unwrap_or(HeightEdge::Continue),
            alpha: alpha.unwrap_or(NormalAlpha::One),
            flip_green,
        })
    }

    /// The pass's uniforms, laid out as the shader's `Heights`, with the
    /// normals stored as they are when `signed`.
    fn bytes(&self, signed: bool) -> Vec<u8> {
        let mut out = Vec::with_capacity(32);
        out.extend(self.strength.to_le_bytes());
        for v in [self.channel.code(), self.kernel.code(), self.edge.code(), self.alpha.code(), u32::from(self.flip_green), u32::from(signed), 0] {
            out.extend(v.to_le_bytes());
        }
        out
    }

    /// What a note says the pass did.
    pub(crate) fn said(&self) -> String {
        format!(
            "the heights, read from {}, were made into normals on the GPU by the {:?} kernel at strength {}, {}; green points {}, and alpha holds {}",
            self.channel.name(),
            self.kernel,
            self.strength,
            self.edge.said(),
            if self.flip_green { "down, the convention named after DirectX" } else { "up, as glTF reads it" },
            self.alpha.said()
        )
    }
}

/// The pass that makes heights into normals. Every height is read through
/// `at`, which brings a texel past an edge inside by the pass's edge.
const HEIGHTS: &str = "
struct Heights {
    strength: f32,
    channel: u32,
    kernel: u32,
    edge: u32,
    alpha: u32,
    flip: u32,
    signed: u32,
    unused: u32,
}

@group(0) @binding(0) var source: texture_2d<f32>;
@group(0) @binding(1) var<uniform> h: Heights;

@vertex fn vertex(@builtin(vertex_index) index: u32) -> @builtin(position) vec4<f32> {
    let corner = vec2<f32>(f32((index << 1u) & 2u), f32(index & 2u));
    return vec4<f32>(corner.x * 2.0 - 1.0, 1.0 - corner.y * 2.0, 0.0, 1.0);
}

// The height a texel holds: one channel, or the luminance of red, green
// and blue by texconv's weights.
fn height_of(t: vec4<f32>) -> f32 {
    var v = dot(t.rgb, vec3<f32>(0.2125, 0.7154, 0.0721));
    if (h.channel < 4u) {
        v = t[h.channel];
    }
    return v;
}

fn raw(x: i32, y: i32) -> f32 {
    return height_of(textureLoad(source, vec2<i32>(x, y), 0));
}

// Coordinate `i` of a row `n` texels long brought inside it: the other
// side for Wrap, the edge texel for Clamp, the row mirrored with its edge
// texel repeated for Mirror, and for Continue the texel as far inside the
// edge as `i` is past it. It returns once, after the switch, as FXC
// refuses a function whose returns are all inside one.
fn inside(i: i32, n: i32) -> i32 {
    var k = clamp(i, 0, n - 1);
    switch h.edge {
        case 1u: {
            k = ((i % n) + n) % n;
        }
        case 3u: {
            let period = 2 * n;
            let r = ((i % period) + period) % period;
            k = select(r, period - 1 - r, r >= n);
        }
        case 0u: {
            k = i;
            if (i < 0) {
                k = -i;
            } else if (i >= n) {
                k = 2 * (n - 1) - i;
            }
            k = clamp(k, 0, n - 1);
        }
        default: {}
    }
    return k;
}

// The height at column x of row y, a row inside the map, for Continue:
// past the left or right edge, the height of the texel `inside` names,
// ix, turned upside down about the edge texel's, 2 h(ex) - h(ix).
fn along(x: i32, ex: i32, ix: i32, y: i32) -> f32 {
    var v = raw(ix, y);
    if (x != ex) {
        v = 2.0 * raw(ex, y) - v;
    }
    return v;
}

// The height at (x, y), inside the map or past an edge. Continue turns a
// height past an edge upside down about the edge texel's, across first
// and then down. Where heights match from row to row, or from column to
// column, one of the two turns is 2 v - v of equal heights, which is
// exact, so they still match past the edges.
fn at(x: i32, y: i32) -> f32 {
    let size = vec2<i32>(textureDimensions(source));
    let ix = inside(x, size.x);
    let iy = inside(y, size.y);
    var v = 0.0;
    if (h.edge == 0u) {
        let ex = clamp(x, 0, size.x - 1);
        let ey = clamp(y, 0, size.y - 1);
        v = along(x, ex, ix, iy);
        if (y != ey) {
            v = 2.0 * along(x, ex, ix, ey) - v;
        }
    } else {
        v = raw(ix, iy);
    }
    return v;
}

// The weight of the height i texels across and j down from a texel in its
// slope across, by the pass's kernel; the slope down takes the same
// weights turned. Each kernel is scaled so a steady ramp gives its own
// slope: Prewitt, texconv's, the three differences of the rows around the
// texel over 6; Central the one difference over 2; Sobel the three
// weighted 1, 2 and 1 over 8; and NVIDIA Texture Tools' 5 by 5 kernel
// sign(i) (5 - |i| - |j|) over 64.
fn weight(i: i32, j: i32) -> f32 {
    var w = 0.0;
    if (h.kernel == 1u) {
        w = select(0.0, f32(i) / 2.0, j == 0 && abs(i) == 1);
    } else if (h.kernel == 2u) {
        w = select(0.0, f32(i * (2 - abs(j))) / 8.0, abs(i) == 1 && abs(j) <= 1);
    } else if (h.kernel == 3u) {
        w = f32(sign(i) * (5 - abs(i) - abs(j))) / 64.0;
    } else {
        w = select(0.0, f32(i) / 6.0, abs(i) == 1 && abs(j) <= 1);
    }
    return w;
}

// A texel's normal: where the height rises to the right the surface faces
// left, and where it rises down the image the surface faces up it, which
// green points to unless the pass flips it. The heights 2 texels each way
// are read once. Each slope weighs the difference of each pair of heights
// on opposite sides, so heights that match across the texel give a slope
// of exactly 0. Alpha is 1, the texel's height, or texconv's occlusion
// term: d is the strength over 8 times how far the eight neighbors rise
// above the texel, summed, and the term is (r - d) / r with
// r = sqrt(1 + d^2), or 1 where no neighbor rises.
@fragment fn normals(@builtin(position) position: vec4<f32>) -> @location(0) vec4<f32> {
    let p = vec2<i32>(position.xy);
    var near: array<f32, 25>;
    for (var k = 0; k < 25; k++) {
        near[k] = at(p.x + k % 5 - 2, p.y + k / 5 - 2);
    }
    var across = 0.0;
    var down = 0.0;
    for (var j = -2; j <= 2; j++) {
        for (var i = 1; i <= 2; i++) {
            let w = weight(i, j);
            across += w * (near[(j + 2) * 5 + 2 + i] - near[(j + 2) * 5 + 2 - i]);
            down += w * (near[(2 + i) * 5 + j + 2] - near[(2 - i) * 5 + j + 2]);
        }
    }
    var rise = 0.0;
    for (var j = -1; j <= 1; j++) {
        for (var i = -1; i <= 1; i++) {
            rise += max(0.0, near[(j + 2) * 5 + i + 2] - near[12]);
        }
    }
    let s = vec2<f32>(across, down) * h.strength;
    var n = normalize(vec3<f32>(-s.x, s.y, 1.0));
    if (h.flip != 0u) {
        n.y = -n.y;
    }
    var a = 1.0;
    if (h.alpha == 1u) {
        a = near[12];
    } else if (h.alpha == 2u) {
        let d = rise * h.strength * 0.125;
        if (d > 0.0) {
            let r = sqrt(1.0 + d * d);
            a = (r - d) / r;
        }
    }
    return vec4<f32>(select(n * 0.5 + 0.5, n, h.signed != 0u), a);
}
";

/// Level 0 of each layer of `heights`, a 2D texture of a float format,
/// made into normals as `step` says, in a new 2D texture of `format`, a
/// float format with four channels, of the same size and layers, with
/// `levels` mip levels for the caller to render the levels below level 0
/// into. The normals are stored as they are when `signed`, and as
/// n / 2 + 1 / 2 otherwise.
pub(crate) fn from_heights(core: &CoreRef, heights: &wgpu::Texture, step: &HeightToNormal, format: F, levels: u32, signed: bool, stop: Option<&dyn Interrupt>) -> PsResult<wgpu::Texture> {
    let size = heights.size();
    let target = core.scoped("WgpuNormalMap", "the normal map cannot be made", || {
        core.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("Wgpu normal map"),
            size,
            mip_level_count: levels,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        })
    })?;
    let commands = core.scoped("WgpuNormalMap", "the normal map cannot be made", || {
        let module = core.device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("Wgpu normal map"), source: wgpu::ShaderSource::Wgsl(HEIGHTS.into()) });
        let layout = core.device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Wgpu normal map"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Float { filterable: false }, view_dimension: wgpu::TextureViewDimension::D2, multisampled: false },
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
        let pipeline_layout = core.device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some("Wgpu normal map"), bind_group_layouts: &[Some(&layout)], immediate_size: 0 });
        let pipeline = core.device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("Wgpu normal map"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState { module: &module, entry_point: Some("vertex"), compilation_options: Default::default(), buffers: &[] },
            fragment: Some(wgpu::FragmentState {
                module: &module,
                entry_point: Some("normals"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState { format, blend: None, write_mask: wgpu::ColorWrites::ALL })],
            }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });
        let uniform = core.device.create_buffer_init(&wgpu::util::BufferInitDescriptor { label: Some("Wgpu normal map"), contents: &step.bytes(signed), usage: wgpu::BufferUsages::UNIFORM });
        let view = |texture: &wgpu::Texture, layer: u32| {
            texture.create_view(&wgpu::TextureViewDescriptor {
                label: Some("Wgpu normal map"),
                format: None,
                dimension: Some(wgpu::TextureViewDimension::D2),
                usage: None,
                aspect: wgpu::TextureAspect::All,
                base_mip_level: 0,
                mip_level_count: Some(1),
                base_array_layer: layer,
                array_layer_count: Some(1),
            })
        };
        let mut encoder = core.device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("Wgpu normal map") });
        for layer in 0..size.depth_or_array_layers {
            let (input, output) = (view(heights, layer), view(&target, layer));
            let bind_group = core.device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("Wgpu normal map"),
                layout: &layout,
                entries: &[wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&input) }, wgpu::BindGroupEntry { binding: 1, resource: uniform.as_entire_binding() }],
            });
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("Wgpu normal map"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &output,
                    depth_slice: None,
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
        encoder.finish()
    })?;
    core.submit(vec![commands], stop)?;
    Ok(target)
}

/// Whether the opposite edges of `grid`, `width` by `height` heights,
/// join along one axis as neighbors inside it do: across compares the
/// left and right edges, and down the top and bottom. The mean step across
/// the seam between them is at most the largest mean step between
/// neighboring columns, or rows, inside the map, and at most three times
/// their median, each with half a step of 8 bits to spare.
fn joins(grid: &[f64], width: usize, height: usize, across: bool) -> bool {
    let (lines, length) = if across { (width, height) } else { (height, width) };
    if lines < 3 {
        return false;
    }
    let at = |line: usize, i: usize| if across { grid[i * width + line] } else { grid[line * width + i] };
    let step = |a: usize, b: usize| (0..length).map(|i| (at(a, i) - at(b, i)).abs()).sum::<f64>() / length as f64;
    let mut inner: Vec<f64> = (0..lines - 1).map(|line| step(line + 1, line)).collect();
    inner.sort_by(f64::total_cmp);
    let seam = step(0, lines - 1);
    let spare = 0.5 / 255.0;
    seam <= inner[inner.len() - 1] + spare && seam <= 3.0 * inner[inner.len() / 2] + spare
}

/// The note that says the first layer of `heights`, a texture of a float
/// format, read in `channel`, looks like it tiles: which pairs of
/// opposite edges join as neighboring texels do, so the Wrap edge would
/// read across them. `None` when neither pair does.
pub(crate) fn tiling_note(core: &CoreRef, heights: &wgpu::Texture, channel: HeightChannel, stop: Option<&dyn Interrupt>) -> PsResult<Option<String>> {
    let size = heights.size();
    let (width, height) = (size.width as usize, size.height as usize);
    let colors = texel::colors(&image_io::read_level(core, heights, 0, stop)?, heights.format())?;
    let grid: Vec<f64> = colors[..width * height].iter().map(|c| f64::from(channel.of(c))).collect();
    let pairs = match (joins(&grid, width, height, true), joins(&grid, width, height, false)) {
        (true, true) => "its left and right edges, and its top and bottom, join as neighboring texels do",
        (true, false) => "its left and right edges join as neighboring columns do",
        (false, true) => "its top and bottom edges join as neighboring rows do",
        (false, false) => return Ok(None),
    };
    Ok(Some(format!("the height map looks like it tiles, as {pairs}: the Wrap edge reads across them")))
}

/// Makes a normal map from a height map: each texel's normal from the
/// slopes of the heights around it, by a kernel, at a strength.
///
/// -Strength scales the slopes: a height step of 1 between neighboring
/// texels, heights running from 0 to 1, gives a slope of the strength, as
/// texconv's -nmapamp does; 1 when omitted. -HeightChannel reads the
/// heights from red, green, blue, alpha or the luminance of red, green and
/// blue; red when omitted. -NormalKernel takes the slopes by texconv's 3
/// by 3 Prewitt kernel when omitted, or by central differences, Sobel's 3
/// by 3 or NVIDIA Texture Tools' 5 by 5, each scaled so a steady ramp
/// gives its own slope.
///
/// Past the edges the slope at each edge carries on, unless -HeightEdge
/// says Wrap, for a height map that tiles, Clamp or Mirror; when the edges
/// join as neighboring texels do, the verbose stream and the Notes say the
/// map looks like it tiles. Green points up the image, as glTF reads it,
/// and down, the convention named after DirectX, with -FlipGreen. Alpha
/// is 1, or the height or texconv's occlusion term as -NormalAlpha says.
///
/// The new texture is Rgba8Unorm, or the -Format named: a color format, a
/// BC format, encoded on the GPU, or a Basis Universal format. A format
/// that holds no negative values stores each normal n as n / 2 + 1 / 2,
/// and any other stores n. -TwoChannel keeps X and Y alone, where the
/// format keeps them apart: in red and green for a two-channel format
/// such as Bc5RgUnorm, and X in red, green and blue with Y in alpha for a
/// Basis Universal LDR format, for a shader to rebuild Z. The new texture
/// has the height map's mip levels, rendered from level 0 as -MipFilter
/// and -MipEdge say, each texel scaled back to length 1, or filtered in
/// slope space as -NormalMips says. What making it involved goes to the
/// verbose stream and its Notes.
///
/// # Examples
/// ConvertTo-WgpuNormalMap $height
/// $height | ConvertTo-WgpuNormalMap -Strength 4
/// $height | ConvertTo-WgpuNormalMap -Kernel Sobel5x5 -Edge Wrap
/// $height | ConvertTo-WgpuNormalMap -Strength 2 -FlipGreen -Format Bc5RgUnorm -TwoChannel
/// $height | ConvertTo-WgpuNormalMap -Alpha Height
#[cmdlet(verb = "ConvertTo", noun = "WgpuNormalMap", alias = ["ConvertTo-GpuNormalMap"], output = ["Wgpu.Texture"])]
#[derive(Default)]
pub struct ConvertToWgpuNormalMap {
    /// The height map; it is left as it is.
    #[param(mandatory, position = 0, value_from_pipeline)]
    pub texture: Option<PsProxy<Texture>>,
    /// How steep the slopes are: a height step of 1 between neighboring
    /// texels gives a slope of this; 1 when omitted.
    #[param]
    pub strength: Option<f64>,
    /// Where the heights are read: Red, Green, Blue, Alpha or Luminance;
    /// Red when omitted.
    #[param]
    pub height_channel: Option<HeightChannel>,
    /// The kernel the slopes are taken by: Prewitt, texconv's, when
    /// omitted, Central, Sobel or Sobel5x5.
    #[param(alias = ["Kernel"])]
    pub normal_kernel: Option<NormalKernel>,
    /// What is read past the edges: Continue, the slope at the edge
    /// carried on, when omitted; Wrap, the other side; Clamp or Mirror.
    #[param(alias = ["Edge"])]
    pub height_edge: Option<HeightEdge>,
    /// What alpha holds: One when omitted, Height or Occlusion.
    #[param(alias = ["Alpha"])]
    pub normal_alpha: Option<NormalAlpha>,
    /// Points green down the image, the convention named after DirectX;
    /// up, as glTF reads normal maps, when omitted.
    #[param]
    pub flip_green: bool,
    /// Keeps X and Y alone where -Format keeps them apart: in red and green
    /// for a two-channel format, and X in red, green and blue with Y in
    /// alpha for a Basis Universal LDR format.
    #[param]
    pub two_channel: bool,
    /// The new texture's format: a color format, a BC format, encoded on
    /// the GPU, or a Basis Universal format; Rgba8Unorm when omitted.
    #[param]
    pub format: Option<String>,
    /// How the mip levels keep their normals unit length: Renormalize when
    /// omitted, or SlopeSpace.
    #[param]
    pub normal_mips: Option<NormalMips>,
    /// How the mip levels are filtered: Box, the default, each from the
    /// level above; Triangle, Kaiser, Mitchell or Lanczos from level 0.
    #[param]
    pub mip_filter: Option<MipFilter>,
    /// What Triangle, Kaiser, Mitchell and Lanczos read past an edge:
    /// Clamp, the default; Wrap; or Mirror.
    #[param]
    pub mip_edge: Option<MipEdge>,
}

impl Cmdlet for ConvertToWgpuNormalMap {
    fn process(&mut self, ps: &Pipeline<'_>) -> PsResult<()> {
        let Some(texture) = &self.texture else {
            return Err(invalid("WgpuNoTexture", "no texture was given"));
        };
        let step = HeightToNormal::new(self.strength.unwrap_or(1.0), "-Strength", self.height_channel, self.normal_kernel, self.height_edge, self.normal_alpha, self.flip_green)?;
        let mips = MipOptions::given(self.mip_filter, self.mip_edge).with_normals(Some(self.normal_mips.unwrap_or(NormalMips::Renormalize)), self.normal_mips.is_some());
        let (core, source) = texture.with(|t| (t.core.clone(), t.source()))?;
        let mut said = Said::default();
        let made = image_io::normal_map(&core, &source, &step, self.format.as_deref(), self.two_channel, mips, Some(ps), &mut said);
        tell(ps, &said)?;
        ps.write(made?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::color::Color;
    use crate::device::{self, AdapterQuery, InstanceOptions, OpenOptions};
    use crate::image_io::Source;
    use crate::texfile::{self, TextureFile};
    use crate::texture;
    use crate::transform::{self, Asked, Packed, Transform};

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

    /// A 2D file of `format`, `width` by `height`, with level 0 alone.
    fn file(format: F, width: u32, height: u32, texels: Vec<u8>) -> TextureFile {
        TextureFile { dimension: wgpu::TextureDimension::D2, width, height, depth: 1, layers: 1, cube: false, format, levels: vec![texels] }
    }

    /// An R32Float height map, `width` by `height`, with `levels` mip
    /// levels, the levels below level 0 left at 0, on `core`.
    fn heights(core: &CoreRef, width: u32, height: u32, levels: u32, grid: &[f64]) -> Source {
        let mut file = file(F::R32Float, width, height, grid.iter().flat_map(|&h| (h as f32).to_le_bytes()).collect());
        for level in 1..levels {
            file.levels.push(vec![0; ((width >> level).max(1) * (height >> level).max(1) * 4) as usize]);
        }
        image_io::upload(core, &file, None, "heights".to_string(), Vec::new()).expect("uploaded").source()
    }

    /// Heights from 0 to 1 with no pattern a kernel or an edge could hide
    /// behind: a wave across and down, and a splitmix64 hash of each texel.
    fn bumpy(width: usize, height: usize) -> Vec<f64> {
        let hash = |x: usize, y: usize| {
            let mut z = (y * 7919 + x) as u64 + 0x9E37_79B9_7F4A_7C15;
            z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
            z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
            (z ^ (z >> 31)) % 1000
        };
        (0..height).flat_map(|y| (0..width).map(move |x| 0.5 + 0.3 * (0.7 * x as f64 + 0.3 * y as f64).sin() + 0.0002 * hash(x, y) as f64 - 0.1)).collect()
    }

    fn step(strength: f64, kernel: NormalKernel, edge: HeightEdge, alpha: NormalAlpha, flip_green: bool) -> HeightToNormal {
        HeightToNormal::new(strength, "-Strength", None, Some(kernel), Some(edge), Some(alpha), flip_green).expect("a step")
    }

    /// The normal map `step` makes of `source`, in Rgba32Float, which
    /// stores normals as they are, and what making it said.
    fn made(core: &CoreRef, source: &Source, step: &HeightToNormal, mips: MipOptions) -> (Texture, Said) {
        let mut said = Said::default();
        let texture = image_io::normal_map(core, source, step, Some("Rgba32Float"), false, mips, None, &mut said).expect("made");
        (texture, said)
    }

    /// Mip level `level` of an Rgba32Float texture.
    fn texels(core: &CoreRef, texture: &Texture, level: u32) -> Vec<[f32; 4]> {
        texel::colors(&image_io::read_level(core, &texture.texture, level, None).expect("read"), F::Rgba32Float).expect("colors")
    }

    /// The height at (x, y) of `grid`, `width` by `height`, inside it or
    /// past an edge, as `edge` reads it: past an edge, the other side for
    /// Wrap, the edge texel for Clamp, the map mirrored with its edge texel
    /// repeated for Mirror, and for Continue the height of the texel as far
    /// inside turned upside down about the edge's, along each axis.
    fn height_at(grid: &[f64], width: usize, height: usize, edge: HeightEdge, x: i64, y: i64) -> f64 {
        let fold = |i: i64, n: i64| match edge {
            HeightEdge::Wrap => i.rem_euclid(n),
            HeightEdge::Clamp => i.clamp(0, n - 1),
            HeightEdge::Mirror => {
                let r = i.rem_euclid(2 * n);
                if r >= n { 2 * n - 1 - r } else { r }
            }
            HeightEdge::Continue => (if i < 0 { -i } else if i >= n { 2 * (n - 1) - i } else { i }).clamp(0, n - 1),
        };
        let (n, m) = (width as i64, height as i64);
        let raw = |x: i64, y: i64| grid[y as usize * width + x as usize];
        let (ix, iy) = (fold(x, n), fold(y, m));
        if edge != HeightEdge::Continue {
            return raw(ix, iy);
        }
        let (ex, ey) = (x.clamp(0, n - 1), y.clamp(0, m - 1));
        match (x != ex, y != ey) {
            (true, true) => 4.0 * raw(ex, ey) - 2.0 * raw(ex, iy) - 2.0 * raw(ix, ey) + raw(ix, iy),
            (true, false) => 2.0 * raw(ex, iy) - raw(ix, iy),
            (false, true) => 2.0 * raw(ix, ey) - raw(ix, iy),
            (false, false) => raw(ix, iy),
        }
    }

    /// Texel (x, y)'s normal and alpha by `step`, in doubles: each kernel
    /// as a weight of the texel i across and j down, the slope down by the
    /// same weights turned, and texconv's occlusion term.
    fn expected(grid: &[f64], width: usize, height: usize, step: &HeightToNormal, x: i64, y: i64) -> [f64; 4] {
        let h = |i: i64, j: i64| height_at(grid, width, height, step.edge, x + i, y + j);
        let weight = |i: i64, j: i64| -> f64 {
            match step.kernel {
                NormalKernel::Central if j == 0 && i.abs() == 1 => i as f64 / 2.0,
                NormalKernel::Prewitt if i.abs() == 1 && j.abs() <= 1 => i as f64 / 6.0,
                NormalKernel::Sobel if i.abs() == 1 && j.abs() <= 1 => (i * (2 - j.abs())) as f64 / 8.0,
                NormalKernel::Sobel5x5 if i != 0 && i.abs() <= 2 && j.abs() <= 2 => (i.signum() * (5 - i.abs() - j.abs())) as f64 / 64.0,
                _ => 0.0,
            }
        };
        let (mut across, mut down) = (0.0, 0.0);
        for j in -2..=2 {
            for i in -2..=2 {
                across += weight(i, j) * h(i, j);
                down += weight(i, j) * h(j, i);
            }
        }
        let s = f64::from(step.strength);
        let n = [-s * across, if step.flip_green { -s * down } else { s * down }, 1.0];
        let length = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
        let alpha = match step.alpha {
            NormalAlpha::One => 1.0,
            NormalAlpha::Height => h(0, 0),
            NormalAlpha::Occlusion => {
                let rise: f64 = (-1..=1).flat_map(|j| (-1..=1).map(move |i| (i, j))).filter(|&(i, j)| i != 0 || j != 0).map(|(i, j)| (h(i, j) - h(0, 0)).max(0.0)).sum();
                let d = rise * s * 0.125;
                if d > 0.0 {
                    let r = (1.0 + d * d).sqrt();
                    (r - d) / r
                } else {
                    1.0
                }
            }
        };
        [n[0] / length, n[1] / length, n[2] / length, alpha]
    }

    /// Every texel of the pass matches the reference by every kernel, edge
    /// and alpha, green up and down, on a map whose sides are odd and
    /// differ, to 1e-5.
    #[test]
    fn makes_the_normals_the_reference_makes() {
        let Some(core) = core() else { return };
        let (width, height) = (13, 11);
        let grid = bumpy(width, height);
        let source = heights(&core, width as u32, height as u32, 1, &grid);
        for kernel in [NormalKernel::Prewitt, NormalKernel::Central, NormalKernel::Sobel, NormalKernel::Sobel5x5] {
            for edge in [HeightEdge::Continue, HeightEdge::Wrap, HeightEdge::Clamp, HeightEdge::Mirror] {
                for (alpha, flip_green) in [(NormalAlpha::One, false), (NormalAlpha::Height, true), (NormalAlpha::Occlusion, false)] {
                    let step = step(3.0, kernel, edge, alpha, flip_green);
                    let (texture, _) = made(&core, &source, &step, MipOptions::default());
                    for (at, got) in texels(&core, &texture, 0).iter().enumerate() {
                        let want = expected(&grid, width, height, &step, (at % width) as i64, (at / width) as i64);
                        for c in 0..4 {
                            assert!((f64::from(got[c]) - want[c]).abs() < 1e-5, "{kernel:?} {edge:?} {alpha:?} texel {at} channel {c}: {got:?} against {want:?}");
                        }
                    }
                }
            }
        }
    }

    /// texconv's normals of the 8-bit height map `texconv_heights` makes, by
    /// texconv -nmap r -nmapamp 3 -f R32G32B32A32_FLOAT from DirectXTex's
    /// May2026 release, at chosen texels: corners, edges and inside. Its
    /// edges wrap, and its green points down.
    const TEXCONV: [(usize, usize, [f32; 4]); 10] = [
        (0, 0, [0.25481153, 0.18785377, 0.9485684, 1.0]),
        (15, 0, [0.25691995, -0.13877428, 0.9564172, 1.0]),
        (0, 15, [-0.060927413, 0.572397, 0.8177099, 1.0]),
        (15, 15, [-0.211501, 0.54545003, 0.8110188, 1.0]),
        (7, 0, [0.24546179, 0.20083235, 0.94837487, 1.0]),
        (0, 7, [-0.21290316, 0.12165895, 0.9694696, 1.0]),
        (8, 8, [0.039013397, -0.0936322, 0.99484223, 1.0]),
        (3, 11, [0.10546527, -0.18024972, 0.9779505, 1.0]),
        (12, 5, [0.27375168, -0.103125624, 0.95625585, 1.0]),
        (15, 9, [-0.013675698, 0.0840078, 0.99637127, 1.0]),
    ];

    /// texconv's occlusion term at the same texels, by -nmap ro.
    const TEXCONV_OCCLUSION: [f32; 10] = [0.14707893, 0.273941, 0.39254987, 1.0, 0.12927559, 0.6239153, 0.6368926, 0.8276196, 0.97941613, 1.0];

    /// The height map TEXCONV was made from: 16 by 16 bytes, each
    /// (x * 37 + y * 91 + (x * y) % 23 * 7) % 256, as R8Unorm.
    fn texconv_heights(core: &CoreRef) -> Source {
        let bytes: Vec<u8> = (0..16usize).flat_map(|y| (0..16usize).map(move |x| ((x * 37 + y * 91 + (x * y) % 23 * 7) % 256) as u8)).collect();
        image_io::upload(core, &file(F::R8Unorm, 16, 16, bytes), None, "heights".to_string(), Vec::new()).expect("uploaded").source()
    }

    /// The Prewitt kernel with wrapped edges and green down gives texconv's
    /// normals, and the Occlusion alpha its occlusion term, to 1e-5.
    #[test]
    fn matches_texconv() {
        let Some(core) = core() else { return };
        let source = texconv_heights(&core);
        let (normals, _) = made(&core, &source, &step(3.0, NormalKernel::Prewitt, HeightEdge::Wrap, NormalAlpha::One, true), MipOptions::default());
        let (occluded, _) = made(&core, &source, &step(3.0, NormalKernel::Prewitt, HeightEdge::Wrap, NormalAlpha::Occlusion, true), MipOptions::default());
        let (normals, occluded) = (texels(&core, &normals, 0), texels(&core, &occluded, 0));
        for (&(x, y, want), &alpha) in TEXCONV.iter().zip(&TEXCONV_OCCLUSION) {
            let got = normals[y * 16 + x];
            for c in 0..4 {
                assert!((got[c] - want[c]).abs() < 1e-5, "texel {x},{y}: {got:?} against texconv's {want:?}");
            }
            assert!((occluded[y * 16 + x][3] - alpha).abs() < 1e-5, "texel {x},{y}: occlusion {} against texconv's {alpha}", occluded[y * 16 + x][3]);
        }
    }

    /// A steady ramp gives its own slope times the strength by every
    /// kernel, at every texel: the slope carried on past the edges keeps it
    /// at the border and the corners too.
    #[test]
    fn a_ramp_gives_its_own_slope_by_every_kernel() {
        let Some(core) = core() else { return };
        let (width, height) = (9, 7);
        let grid: Vec<f64> = (0..height).flat_map(|y| (0..width).map(move |x| 0.25 + 0.02 * x as f64 - 0.03 * y as f64)).collect();
        let source = heights(&core, width as u32, height as u32, 1, &grid);
        let want = {
            let n = [-5.0 * 0.02, 5.0 * -0.03, 1.0f64];
            let length = (n[0] * n[0] + n[1] * n[1] + 1.0).sqrt();
            [n[0] / length, n[1] / length, n[2] / length]
        };
        for kernel in [NormalKernel::Prewitt, NormalKernel::Central, NormalKernel::Sobel, NormalKernel::Sobel5x5] {
            let (texture, _) = made(&core, &source, &step(5.0, kernel, HeightEdge::Continue, NormalAlpha::One, false), MipOptions::default());
            for (at, got) in texels(&core, &texture, 0).iter().enumerate() {
                for c in 0..3 {
                    assert!((f64::from(got[c]) - want[c]).abs() < 1e-5, "{kernel:?} texel {at}: {got:?} against {want:?}");
                }
            }
        }
    }

    /// Heights that match from row to row give a slope down of exactly 0,
    /// and heights that match from column to column a slope across of
    /// exactly 0, at every texel, by every kernel and past every edge.
    #[test]
    fn matching_heights_give_a_slope_of_exactly_0() {
        let Some(core) = core() else { return };
        let (width, height) = (7, 6);
        let across: Vec<f64> = (0..height).flat_map(|_| (0..width).map(|x| 0.1 + 0.07 * x as f64)).collect();
        let down: Vec<f64> = (0..height).flat_map(|y| (0..width).map(move |_| 0.9 - 0.11 * y as f64)).collect();
        for (grid, flat) in [(&across, 1), (&down, 0)] {
            let source = heights(&core, width as u32, height as u32, 1, grid);
            for kernel in [NormalKernel::Prewitt, NormalKernel::Central, NormalKernel::Sobel, NormalKernel::Sobel5x5] {
                for edge in [HeightEdge::Continue, HeightEdge::Wrap, HeightEdge::Clamp, HeightEdge::Mirror] {
                    let (texture, _) = made(&core, &source, &step(3.0, kernel, edge, NormalAlpha::One, false), MipOptions::default());
                    for (at, got) in texels(&core, &texture, 0).iter().enumerate() {
                        assert_eq!(got[flat], 0.0, "{kernel:?} {edge:?} texel {at}: {got:?}");
                    }
                }
            }
        }
    }

    /// Green points up the image unless flipped: a height rising down the
    /// image faces up it, and one rising to the right faces left.
    #[test]
    fn green_points_up_unless_flipped() {
        let Some(core) = core() else { return };
        let down: Vec<f64> = (0..8).flat_map(|y| (0..8).map(move |_| 0.1 * y as f64)).collect();
        let right: Vec<f64> = (0..8).flat_map(|_| (0..8).map(|x| 0.1 * x as f64)).collect();
        let at = |grid: &[f64], flip_green: bool| {
            let (texture, _) = made(&core, &heights(&core, 8, 8, 1, grid), &step(1.0, NormalKernel::Prewitt, HeightEdge::Continue, NormalAlpha::One, flip_green), MipOptions::default());
            texels(&core, &texture, 0)[27]
        };
        assert!(at(&down, false)[1] > 0.05 && at(&down, true)[1] < -0.05);
        assert!(at(&right, false)[0] < -0.05);
    }

    /// A format that holds no negative values stores n / 2 + 1 / 2, and a
    /// snorm format stores n, each rounded to its own precision.
    #[test]
    fn stores_normals_as_the_format_does() {
        let Some(core) = core() else { return };
        let grid = bumpy(8, 8);
        let source = heights(&core, 8, 8, 1, &grid);
        let step = step(2.0, NormalKernel::Prewitt, HeightEdge::Continue, NormalAlpha::One, false);
        let mut said = Said::default();
        let unorm = image_io::normal_map(&core, &source, &step, None, false, MipOptions::default(), None, &mut said).expect("made");
        assert_eq!(unorm.format, "Rgba8Unorm");
        let snorm = image_io::normal_map(&core, &source, &step, Some("Rg8Snorm"), false, MipOptions::default(), None, &mut said).expect("made");
        let unorm = texture::read_texels(&core, &unorm.texture, 0, 0, wgpu::TextureAspect::All, None).expect("read");
        let snorm = texture::read_texels(&core, &snorm.texture, 0, 0, wgpu::TextureAspect::All, None).expect("read");
        for at in 0..64usize {
            let want = expected(&grid, 8, 8, &step, (at % 8) as i64, (at / 8) as i64);
            for c in 0..3 {
                assert!((f64::from(unorm[at * 4 + c]) / 255.0 - (want[c] / 2.0 + 0.5)).abs() <= 0.5 / 255.0 + 1e-6, "texel {at} channel {c}");
            }
            assert_eq!(unorm[at * 4 + 3], 255);
            for c in 0..2 {
                assert!((f64::from(snorm[at * 2 + c] as i8) / 127.0 - want[c]).abs() <= 0.5 / 127.0 + 1e-6, "texel {at} channel {c}");
            }
        }
    }

    /// Renormalize leaves every texel of every level at length 1, the Box
    /// level the normalized sum of the four normals under it; SlopeSpace
    /// makes each level the unit normal of the mean slopes of the level-0
    /// texels under it.
    #[test]
    fn keeps_mip_levels_unit_length() {
        let Some(core) = core() else { return };
        let grid = bumpy(16, 16);
        let source = heights(&core, 16, 16, 5, &grid);
        let step = step(4.0, NormalKernel::Sobel, HeightEdge::Wrap, NormalAlpha::One, false);
        let renormalized = MipOptions::default().with_normals(Some(NormalMips::Renormalize), true);
        let (texture, said) = made(&core, &source, &step, renormalized);
        assert!(said.verbose.iter().any(|l| l.contains("each texel's normal scaled back to length 1")), "{said:?}");
        let top = texels(&core, &texture, 0);
        for level in 1..5 {
            for got in texels(&core, &texture, level) {
                let length = (got[0] * got[0] + got[1] * got[1] + got[2] * got[2]).sqrt();
                assert!((length - 1.0).abs() < 1e-5, "level {level}: {got:?}");
            }
        }
        for (at, got) in texels(&core, &texture, 1).iter().enumerate() {
            let (x, y) = (at % 8 * 2, at / 8 * 2);
            let sum: Vec<f32> = (0..3).map(|c| top[y * 16 + x][c] + top[y * 16 + x + 1][c] + top[(y + 1) * 16 + x][c] + top[(y + 1) * 16 + x + 1][c]).collect();
            let length = (sum[0] * sum[0] + sum[1] * sum[1] + sum[2] * sum[2]).sqrt();
            for c in 0..3 {
                assert!((got[c] - sum[c] / length).abs() < 1e-5, "level 1 texel {at}");
            }
        }
        let (texture, _) = made(&core, &source, &step, MipOptions::default().with_normals(Some(NormalMips::SlopeSpace), true));
        let top = texels(&core, &texture, 0);
        for (level, span) in [(1u32, 2usize), (2, 4)] {
            let size = 16 >> level;
            for (at, got) in texels(&core, &texture, level).iter().enumerate() {
                let (x0, y0) = (at % size * span, at / size * span);
                let mut slopes = [0.0f64; 2];
                for y in y0..y0 + span {
                    for x in x0..x0 + span {
                        let n = top[y * 16 + x];
                        slopes[0] += f64::from(n[0] / n[2]);
                        slopes[1] += f64::from(n[1] / n[2]);
                    }
                }
                let count = (span * span) as f64;
                let n = [slopes[0] / count, slopes[1] / count, 1.0];
                let length = (n[0] * n[0] + n[1] * n[1] + 1.0).sqrt();
                for c in 0..3 {
                    assert!((f64::from(got[c]) - n[c] / length).abs() < 1e-5, "level {level} texel {at}: {got:?}");
                }
            }
        }
        let (texture, _) = made(&core, &source, &step, MipOptions::given(Some(MipFilter::Kaiser), Some(MipEdge::Wrap)).with_normals(Some(NormalMips::Renormalize), true));
        for level in 1..5 {
            for got in texels(&core, &texture, level) {
                let length = (got[0] * got[0] + got[1] * got[1] + got[2] * got[2]).sqrt();
                assert!((length - 1.0).abs() < 1e-5, "Kaiser level {level}: {got:?}");
            }
        }
    }

    /// Rgba8Unorm texels as colors from 0 to 1.
    fn unorm_colors(bytes: &[u8]) -> Vec<[f32; 4]> {
        bytes.as_chunks::<4>().0.iter().map(|t| t.map(|v| f32::from(v) / 255.0)).collect()
    }

    /// Convert's steps on the normals a texture holds: Z rebuilt from X and
    /// Y, each normal scaled to length 1, and green flipped, read as
    /// n / 2 + 1 / 2 from Rgba8Unorm and as n from Rgba16Float.
    #[test]
    fn changes_the_normals_a_texture_holds() {
        let Some(core) = core() else { return };
        let normals: [[f32; 3]; 4] = [[0.3, 0.4, 0.2], [-0.6, 0.0, 0.5], [0.0, -0.8, 0.1], [0.1, 0.2, 0.9]];
        let run = |format: F, color: Color| {
            let stores_n = signed(format);
            let colors: Vec<[f32; 4]> = normals.iter().map(|n| if stores_n { [n[0], n[1], n[2], 1.0] } else { [n[0] / 2.0 + 0.5, n[1] / 2.0 + 0.5, n[2] / 2.0 + 0.5, 1.0] }).collect();
            let texture = image_io::upload(&core, &file(format, 4, 1, texel::pack(&colors, format).expect("packed")), None, "normals".to_string(), Vec::new()).expect("uploaded");
            let made = image_io::transformed(&core, &texture.source(), &Transform { color, ..Default::default() }, MipOptions::default(), None, None, &mut Said::default()).expect("changed");
            let back = image_io::read_level(&core, &made.texture, 0, None).expect("read");
            let colors = if format == F::Rgba8Unorm { unorm_colors(&back) } else { texel::colors(&back, format).expect("colors") };
            colors.iter().map(|c| if stores_n { [c[0], c[1], c[2]] } else { [c[0] * 2.0 - 1.0, c[1] * 2.0 - 1.0, c[2] * 2.0 - 1.0] }).collect::<Vec<_>>()
        };
        for format in [F::Rgba8Unorm, F::Rgba16Float] {
            let within = if format == F::Rgba8Unorm { 2.5 / 255.0 } else { 2e-3 };
            let flipped = run(format, Color { flip_green: true, ..Default::default() });
            let rebuilt = run(format, Color { reconstruct_z: true, ..Default::default() });
            let unit = run(format, Color { normalize: true, ..Default::default() });
            for (i, n) in normals.iter().enumerate() {
                assert!((flipped[i][1] + n[1]).abs() < within && (flipped[i][0] - n[0]).abs() < within, "{format:?} flip {i}");
                let z = (1.0 - n[0] * n[0] - n[1] * n[1]).max(0.0).sqrt();
                assert!((rebuilt[i][2] - z).abs() < within, "{format:?} z {i}: {:?}", rebuilt[i]);
                let length = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
                for c in 0..3 {
                    assert!((unit[i][c] - n[c] / length).abs() < within, "{format:?} normalize {i}");
                }
            }
        }
    }

    /// -Normalize writing for a conversion to a format that stores normals
    /// as they are keeps them as n, read from n / 2 + 1 / 2.
    #[test]
    fn writes_normals_as_the_target_stores_them() {
        let Some(core) = core() else { return };
        let colors = [[0.3f32 / 2.0 + 0.5, 0.4 / 2.0 + 0.5, 0.2 / 2.0 + 0.5, 1.0]];
        let source = file(F::Rgba8Unorm, 1, 1, texel::pack(&colors, F::Rgba8Unorm).expect("packed"));
        let transform = Transform { color: Color { normalize: true, ..Default::default() }, target: Some(F::Rgba16Float), ..Default::default() };
        let mut said = Said::default();
        let out = transform::apply(&core, source, 1, MipOptions::default(), &transform, Packed::Decoded, None, None, None, &mut said).expect("written");
        assert_eq!(out.format, F::Rgba32Float);
        let n = texel::colors(&out.levels[0], F::Rgba32Float).expect("colors")[0];
        let read = [0.3f32, 0.4, 0.2].map(|v| ((v / 2.0 + 0.5) * 255.0).round() / 255.0 * 2.0 - 1.0);
        let length = (read[0] * read[0] + read[1] * read[1] + read[2] * read[2]).sqrt();
        for c in 0..3 {
            assert!((n[c] - read[c] / length).abs() < 1e-5, "{n:?}");
        }
        assert!(said.verbose.iter().any(|l| l.contains("written with them stored as they are")), "{said:?}");
    }

    /// -TwoChannel keeps X and Y in red and green for BC5, and puts X in
    /// red, green and blue with Y in alpha for a Basis Universal format,
    /// each within its encoder's error on a smooth map.
    #[test]
    fn lays_two_channels_out_for_the_format() {
        let Some(core) = core() else { return };
        let grid: Vec<f64> = (0..16).flat_map(|y| (0..16).map(move |x| 0.5 + 0.3 * (0.4 * x as f64 + 0.2 * y as f64).sin())).collect();
        let source = heights(&core, 16, 16, 1, &grid);
        let step = step(2.0, NormalKernel::Prewitt, HeightEdge::Continue, NormalAlpha::One, false);
        let decoded = |texture: &Texture| {
            let file = image_io::read_file(&core, &texture.texture, false, 1, None).expect("read");
            let file = texfile::uncompressed(file, &mut Said::default()).expect("decoded");
            (file.format, file.levels[0].clone())
        };
        let mut said = Said::default();
        let bc5 = image_io::normal_map(&core, &source, &step, Some("Bc5RgUnorm"), true, MipOptions::default(), None, &mut said).expect("made");
        let (format, bytes) = decoded(&bc5);
        assert_eq!(format, F::Rg8Unorm);
        for at in 0..256usize {
            let want = expected(&grid, 16, 16, &step, (at % 16) as i64, (at / 16) as i64);
            for c in 0..2 {
                let got = f64::from(bytes[at * 2 + c]) / 255.0;
                assert!((got - (want[c] / 2.0 + 0.5)).abs() < 4.0 / 255.0, "BC5 texel {at} channel {c}: {got}");
            }
        }
        let basis = image_io::normal_map(&core, &source, &step, Some("BasisUastc4x4"), true, MipOptions::default(), None, &mut said).expect("made");
        assert!(said.verbose.iter().any(|l| l.contains("X went into red, green and blue and its Y into alpha")), "{said:?}");
        let (format, bytes) = decoded(&basis);
        assert!(matches!(format, F::Rgba8Unorm | F::Rgba8UnormSrgb), "{format:?}");
        for at in 0..256usize {
            let want = expected(&grid, 16, 16, &step, (at % 16) as i64, (at / 16) as i64);
            let t = &bytes[at * 4..at * 4 + 4];
            assert!((f64::from(t[0]) / 255.0 - (want[0] / 2.0 + 0.5)).abs() < 6.0 / 255.0, "Basis texel {at} X: {t:?}");
            assert!((f64::from(t[3]) / 255.0 - (want[1] / 2.0 + 0.5)).abs() < 6.0 / 255.0, "Basis texel {at} Y: {t:?}");
            assert!(t[0].abs_diff(t[1]) <= 6 && t[0].abs_diff(t[2]) <= 6, "Basis texel {at}: {t:?}");
        }
    }

    /// A height map whose opposite edges join as neighboring texels do is
    /// said to look like it tiles, unless the Wrap edge already reads
    /// across them; a ramp across and down is not.
    #[test]
    fn says_when_a_height_map_looks_like_it_tiles() {
        let Some(core) = core() else { return };
        let tiles: Vec<f64> = (0..16).flat_map(|y| (0..16).map(move |x| 0.5 + 0.25 * (std::f64::consts::TAU * x as f64 / 16.0).sin() * (std::f64::consts::TAU * y as f64 / 16.0).cos())).collect();
        let ramp: Vec<f64> = (0..16).flat_map(|y| (0..16).map(move |x| (x + y) as f64 / 32.0)).collect();
        let note = |grid: &[f64], edge: HeightEdge| {
            let (_, said) = made(&core, &heights(&core, 16, 16, 1, grid), &step(1.0, NormalKernel::Prewitt, edge, NormalAlpha::One, false), MipOptions::default());
            said.verbose.iter().any(|l| l.contains("looks like it tiles"))
        };
        assert!(note(&tiles, HeightEdge::Continue));
        assert!(!note(&tiles, HeightEdge::Wrap));
        assert!(!note(&ramp, HeightEdge::Continue));
    }

    /// A refusal's error.
    fn refused<T>(result: PsResult<T>) -> PsError {
        match result {
            Ok(_) => panic!("expected a refusal"),
            Err(e) => e,
        }
    }

    /// What makes no normal map, or no normal, is refused.
    #[test]
    fn refuses_what_makes_no_normal_map() {
        assert_eq!(refused(HeightToNormal::new(f64::INFINITY, "-Strength", None, None, None, None, false)).error_id, "WgpuNormalMap");
        let minimum = MipOptions::given(Some(MipFilter::Min), None).with_normals(Some(NormalMips::Renormalize), true);
        assert_eq!(refused(minimum.check()).error_id, "WgpuNormalMap");
        let settings_alone = Asked { normal_kernel: Some(NormalKernel::Sobel), ..Default::default() };
        assert_eq!(refused(settings_alone.transform()).error_id, "WgpuNormalMap");
        let both = Asked { height_to_normal: Some(1.0), normalize: true, ..Default::default() };
        assert_eq!(refused(both.transform()).error_id, "WgpuNormalMap");
        assert_eq!(refused(Color { reconstruct_z: true, ..Default::default() }.prepare(F::Rg8Unorm, F::Rg8Unorm, false, false)).error_id, "WgpuNormalMap");
        assert_eq!(refused(image_io::two_channel_layout(Some(image_io::parse_target("Rgba8Unorm").expect("a format")))).error_id, "WgpuNormalMap");
        assert_eq!(refused(image_io::two_channel_layout(None)).error_id, "WgpuNormalMap");
        let Some(core) = core() else { return };
        let source = heights(&core, 8, 8, 1, &bumpy(8, 8));
        let step = step(1.0, NormalKernel::Prewitt, HeightEdge::Continue, NormalAlpha::One, false);
        let mut said = Said::default();
        assert_eq!(refused(image_io::normal_map(&core, &source, &step, Some("Rgba8UnormSrgb"), false, MipOptions::default(), None, &mut said)).error_id, "WgpuNormalMap");
        let cube = TextureFile { layers: 6, cube: true, ..file(F::R32Float, 4, 4, vec![0; 4 * 4 * 4 * 6]) };
        let transform = Transform { normal: Some(step), ..Default::default() };
        assert_eq!(refused(transform::apply(&core, cube, 1, MipOptions::default(), &transform, Packed::Decoded, None, None, None, &mut said)).error_id, "WgpuNormalMap");
    }
}
