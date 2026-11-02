//! Comparing a texture with a reference, and measuring one.
//! Compare-WgpuTexture writes the mean squared error, PSNR and SSIM of each
//! level, layer and channel both textures hold, or a picture of the error
//! on a log scale; Measure-WgpuTexture writes each channel's minimum,
//! maximum, mean, standard deviation and histogram.
//!
//! Both read the texels back from the GPU as the values they store: a
//! block-compressed format decoded on the CPU, an integer format through
//! the norm format with its bits, depth from 0 to 1 and stencil over 255,
//! and a multisampled texture's samples averaged in a shader. SSIM runs on
//! the GPU; the rest is added up on the CPU in double precision.

use std::num::NonZeroU64;

use half::f16;
use pwrs::prelude::*;
use wgpu::util::DeviceExt;
use wgpu::TextureFormat as F;

use crate::buffer;
use crate::device::CoreRef;
use crate::error::invalid;
use crate::image_io::{self, Source};
use crate::tasks::Interrupt;
use crate::texel;
use crate::texfile::{self, Said, TextureFile};
use crate::texture::{self, format_name, tell, Texture};

/// The side of SSIM's window: a level narrower or shorter than this has no
/// SSIM.
const WINDOW: usize = 11;

/// The most memory one band of SSIM's input takes: the rows of a plane
/// are run in bands whose texels fit in this and in the device's largest
/// storage binding.
const BAND_BYTES: u64 = 64 << 20;

/// The error at or below which the difference picture is black: 2^-12.
const FLOOR: f64 = 1.0 / 4096.0;

/// A channel of a texture's texels.
#[psenum(name = "Wgpu.TextureChannel")]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum TextureChannel {
    /// The first channel: red, the depth of a texture with depth, or the stencil of Stencil8.
    #[default]
    Red,
    /// The second channel: green, or the stencil of a texture with depth and stencil.
    Green,
    /// The third channel: blue.
    Blue,
    /// The fourth channel: alpha.
    Alpha,
}

impl TextureChannel {
    const ALL: [TextureChannel; 4] = [TextureChannel::Red, TextureChannel::Green, TextureChannel::Blue, TextureChannel::Alpha];

    fn index(self) -> usize {
        match self {
            TextureChannel::Red => 0,
            TextureChannel::Green => 1,
            TextureChannel::Blue => 2,
            TextureChannel::Alpha => 3,
        }
    }

    fn name(self) -> &'static str {
        ["red", "green", "blue", "alpha"][self.index()]
    }
}

/// What Compare-WgpuTexture writes: how one channel of one mip level and
/// layer of a texture differs from the reference's.
#[psclass(name = "Wgpu.TextureComparison", columns = ["Level", "Layer", "Channel", "MSE", "PSNR", "SSIM"])]
#[derive(Default, Clone, Debug)]
pub struct TextureComparison {
    /// The mip level.
    pub level: u32,
    /// The array layer or cube face, or the depth slice of a 3D texture.
    pub layer: u32,
    /// The channel.
    pub channel: TextureChannel,
    /// The mean squared error: the mean of the squared differences between the texture's values and the reference's.
    pub m_s_e: f64,
    /// The peak signal-to-noise ratio in decibels, 10 log10(peak^2 / MSE): the peak is 1 for values from 0 to 1, 2 for values from -1 to 1, and for a float texture the reference channel's largest value, or 1 when that is 0 or below; +Infinity when the values match.
    pub p_s_n_r: f64,
    /// The structural similarity, Wang et al.'s SSIM with a Gaussian window of sigma 1.5 and the peak as its data range: 1 when the values match; $null for a level under 11 texels on a side.
    pub s_s_i_m: Option<f64>,
}

/// What Measure-WgpuTexture writes: the values of one channel of one mip
/// level and layer of a texture.
#[psclass(name = "Wgpu.TextureMeasurement", columns = ["Level", "Layer", "Channel", "Minimum", "Maximum", "Average", "StandardDeviation", "Histogram"])]
#[derive(Default, Clone, Debug)]
pub struct TextureMeasurement {
    /// The mip level.
    pub level: u32,
    /// The array layer or cube face, or the depth slice of a 3D texture.
    pub layer: u32,
    /// The channel.
    pub channel: TextureChannel,
    /// The smallest value; NaN when a value is NaN.
    pub minimum: f64,
    /// The largest value; NaN when a value is NaN.
    pub maximum: f64,
    /// The mean; NaN when a value is NaN.
    pub average: f64,
    /// The population standard deviation, the square root of the mean squared distance from the mean; NaN when a value is NaN.
    pub standard_deviation: f64,
    /// How many values each bin counts, the bins splitting the range evenly; values below it count in the first bin, values above it in the last, and NaN in none.
    pub histogram: Vec<u64>,
}

/// How far a texture's values span, which sets PSNR's peak and SSIM's
/// data range.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Span {
    /// From 0 to 1: unorm, sRGB and unsigned integer values, depth and
    /// stencil.
    Unit,
    /// From -1 to 1: snorm and signed integer values.
    Signed,
    /// Floats.
    Float,
}

impl Span {
    /// The peak of values that span so, `largest` being the reference
    /// channel's largest value: 1, 2, or for floats that value, 1 when it
    /// is 0 or below.
    fn peak(self, largest: f64) -> f64 {
        match self {
            Span::Unit => 1.0,
            Span::Signed => 2.0,
            Span::Float if largest > 0.0 => largest,
            Span::Float => 1.0,
        }
    }
}

/// Reads channel `c` of one texel's bytes as a value.
type Unpack = fn(&[u8], usize) -> f64;

fn unit8(t: &[u8], c: usize) -> f64 {
    f64::from(t[c]) / 255.0
}

fn bgra8(t: &[u8], c: usize) -> f64 {
    f64::from(t[[2, 1, 0, 3][c]]) / 255.0
}

fn signed8(t: &[u8], c: usize) -> f64 {
    (f64::from(t[c] as i8) / 127.0).max(-1.0)
}

fn word16(t: &[u8], c: usize) -> [u8; 2] {
    [t[2 * c], t[2 * c + 1]]
}

fn unit16(t: &[u8], c: usize) -> f64 {
    f64::from(u16::from_le_bytes(word16(t, c))) / 65535.0
}

fn signed16(t: &[u8], c: usize) -> f64 {
    (f64::from(i16::from_le_bytes(word16(t, c))) / 32767.0).max(-1.0)
}

fn half(t: &[u8], c: usize) -> f64 {
    f16::from_le_bytes(word16(t, c)).to_f64()
}

fn word32(t: &[u8], c: usize) -> [u8; 4] {
    [t[4 * c], t[4 * c + 1], t[4 * c + 2], t[4 * c + 3]]
}

fn unit32(t: &[u8], c: usize) -> f64 {
    f64::from(u32::from_le_bytes(word32(t, c))) / f64::from(u32::MAX)
}

fn signed32(t: &[u8], c: usize) -> f64 {
    (f64::from(i32::from_le_bytes(word32(t, c))) / f64::from(i32::MAX)).max(-1.0)
}

fn single(t: &[u8], c: usize) -> f64 {
    f64::from(f32::from_le_bytes(word32(t, c)))
}

fn word64(t: &[u8]) -> u64 {
    u64::from_le_bytes([t[0], t[1], t[2], t[3], t[4], t[5], t[6], t[7]])
}

fn unit64(t: &[u8], _: usize) -> f64 {
    word64(t) as f64 / u64::MAX as f64
}

fn rgb10a2(t: &[u8], c: usize) -> f64 {
    let bits = u32::from_le_bytes(word32(t, 0));
    if c == 3 { f64::from(bits >> 30) / 3.0 } else { f64::from((bits >> (10 * c)) & 0x3FF) / 1023.0 }
}

/// An unsigned float of a 5-bit exponent biased by 15 above `mantissa`
/// bits, as the Vulkan specification decodes it: subnormal at exponent 0,
/// infinite or NaN at 31.
fn small_float(bits: u32, mantissa: u32) -> f64 {
    let fraction = f64::from(bits & ((1 << mantissa) - 1)) / f64::from(1u32 << mantissa);
    match bits >> mantissa {
        0 => fraction * 2f64.powi(-14),
        31 if fraction == 0.0 => f64::INFINITY,
        31 => f64::NAN,
        exponent => (1.0 + fraction) * 2f64.powi(exponent as i32 - 15),
    }
}

fn rg11b10(t: &[u8], c: usize) -> f64 {
    let bits = u32::from_le_bytes(word32(t, 0));
    match c {
        0 => small_float(bits & 0x7FF, 6),
        1 => small_float((bits >> 11) & 0x7FF, 6),
        _ => small_float(bits >> 22, 5),
    }
}

fn rgb9e5(t: &[u8], c: usize) -> f64 {
    let bits = u32::from_le_bytes(word32(t, 0));
    f64::from((bits >> (9 * c)) & 0x1FF) * 2f64.powi((bits >> 27) as i32 - 24)
}

/// A depth-stencil texel as Compare and Measure keep it: the depth as a
/// 32-bit float, then the stencil byte, read over 255.
fn depth_stencil(t: &[u8], c: usize) -> f64 {
    if c == 0 { single(t, 0) } else { f64::from(t[4]) / 255.0 }
}

/// How `format`'s texels read as values, and how far they span: an
/// integer format through the norm format with its bits. None for a
/// format whose texels are not read on the CPU: depth, stencil, planar and
/// block-compressed formats.
fn reading(format: F) -> Option<(Unpack, Span)> {
    let unpack: (Unpack, Span) = match format {
        F::R8Unorm | F::Rg8Unorm | F::Rgba8Unorm | F::Rgba8UnormSrgb | F::R8Uint | F::Rg8Uint | F::Rgba8Uint => (unit8, Span::Unit),
        F::Bgra8Unorm | F::Bgra8UnormSrgb => (bgra8, Span::Unit),
        F::R8Snorm | F::Rg8Snorm | F::Rgba8Snorm | F::R8Sint | F::Rg8Sint | F::Rgba8Sint => (signed8, Span::Signed),
        F::R16Unorm | F::Rg16Unorm | F::Rgba16Unorm | F::R16Uint | F::Rg16Uint | F::Rgba16Uint => (unit16, Span::Unit),
        F::R16Snorm | F::Rg16Snorm | F::Rgba16Snorm | F::R16Sint | F::Rg16Sint | F::Rgba16Sint => (signed16, Span::Signed),
        F::R16Float | F::Rg16Float | F::Rgba16Float => (half, Span::Float),
        F::R32Uint | F::Rg32Uint | F::Rgba32Uint => (unit32, Span::Unit),
        F::R32Sint | F::Rg32Sint | F::Rgba32Sint => (signed32, Span::Signed),
        F::R32Float | F::Rg32Float | F::Rgba32Float => (single, Span::Float),
        F::R64Uint => (unit64, Span::Unit),
        F::Rgb10a2Unorm | F::Rgb10a2Uint => (rgb10a2, Span::Unit),
        F::Rg11b10Ufloat => (rg11b10, Span::Float),
        F::Rgb9e5Ufloat => (rgb9e5, Span::Float),
        _ => return None,
    };
    Some(unpack)
}

/// A texture's texels, or one plane's of a planar texture, as Compare and
/// Measure read them: the levels read and every layer, packed as a file
/// holds them, and how each texel's bytes give its values.
struct Texels {
    dimension: wgpu::TextureDimension,
    /// The width and height of the texels: a plane's, which for the
    /// chroma of a planar texture is half the texture's.
    width: u32,
    height: u32,
    /// The depth slices of a 3D texture; 1 otherwise.
    depth: u32,
    /// The array layers; 1 for a 3D texture.
    layers: u32,
    cube: bool,
    unpack: Unpack,
    /// The bytes of one texel.
    size: usize,
    channels: usize,
    /// The channel the first of a texel's values is: 0, or 1 for the
    /// chroma of a planar texture, whose two values are Green and Blue.
    first: usize,
    span: Span,
    /// Whether the texels are R64Uint, whose differences are taken in
    /// integers, as a double cannot tell apart values that close.
    wide: bool,
    levels: Vec<Vec<u8>>,
}

impl Texels {
    /// `levels` of `texture`'s shape, each texel `size` bytes of
    /// `channels` values `unpack` reads.
    fn of(texture: &wgpu::Texture, cube: bool, unpack: Unpack, size: usize, channels: usize, span: Span, levels: Vec<Vec<u8>>) -> Texels {
        let volume = texture.dimension() == wgpu::TextureDimension::D3;
        Texels {
            dimension: texture.dimension(),
            width: texture.width(),
            height: texture.height(),
            depth: if volume { texture.depth_or_array_layers() } else { 1 },
            layers: if volume { 1 } else { texture.depth_or_array_layers() },
            cube,
            unpack,
            size,
            channels,
            first: 0,
            span,
            wide: false,
            levels,
        }
    }

    /// The width and height of mip level `level`.
    fn extent(&self, level: usize) -> (usize, usize) {
        ((self.width >> level).max(1) as usize, (self.height >> level).max(1) as usize)
    }

    /// The slices of mip level `level`: its layers, or the depth slices of
    /// a 3D level.
    fn slices(&self, level: usize) -> usize {
        let slices = if self.dimension == wgpu::TextureDimension::D3 { (self.depth >> level).max(1) } else { 1 };
        self.layers as usize * slices as usize
    }

    /// The texels of slice `slice` of mip level `level`.
    fn slice(&self, level: usize, slice: usize) -> &[u8] {
        let (width, height) = self.extent(level);
        let len = width * height * self.size;
        &self.levels[level][slice * len..(slice + 1) * len]
    }

    /// Channel `channel` of texel `texel` of `plane`.
    fn value(&self, plane: &[u8], texel: usize, channel: usize) -> f64 {
        (self.unpack)(&plane[texel * self.size..(texel + 1) * self.size], channel)
    }

    /// Channel `channel` of a plane's `n` texels as 32-bit floats.
    fn floats(&self, plane: &[u8], channel: usize, n: usize) -> Vec<f32> {
        (0..n).map(|texel| self.value(plane, texel, channel) as f32).collect()
    }
}

/// Refuses `texture` unless it has `usage`, naming it as `role` and
/// saying why it needs it.
fn needs(texture: &wgpu::Texture, usage: wgpu::TextureUsages, role: &str, why: &str) -> PsResult<()> {
    if texture.usage().contains(usage) {
        return Ok(());
    }
    let names: Vec<String> = texture::usage_list(usage).iter().map(|u| format!("{u:?}")).collect();
    Err(invalid("WgpuTextureUsage", format!("{role} needs {} among its usages {why}", names.join(" and "))))
}

/// The first `size` bytes of `buffer`, bound with that size.
fn bound(buffer: &wgpu::Buffer, size: u64) -> wgpu::BindingResource<'_> {
    wgpu::BindingResource::Buffer(wgpu::BufferBinding { buffer, offset: 0, size: NonZeroU64::new(size) })
}

/// How a shader averages a texel's samples.
enum Mean {
    /// In floats: the WGSL expression giving sample `s` at `at` as four
    /// values.
    Float(String),
    /// In unsigned integers, rounded to the nearest, ties to even.
    Unsigned,
    /// In signed integers, rounded to the nearest, ties to even.
    Signed,
}

/// What a shader reads of one level and layer of a texture that no copy
/// reads: the depth of Depth24Plus, or the samples of a multisampled
/// texture.
struct ShaderRead {
    /// The WGSL type the texture binds as.
    ty: &'static str,
    binding: wgpu::BindingType,
    aspect: wgpu::TextureAspect,
    mean: Mean,
    /// How many of the four values a texel has.
    channels: u32,
}

/// The pass that reads a texture in a shader: each texel's samples,
/// averaged as MEAN says, written as CHANNELS values of type ELEMENT.
const READ: &str = "
struct Shape {
    width: u32,
    height: u32,
    samples: u32,
    unused: u32,
}

@group(0) @binding(0) var source: SOURCE;
@group(0) @binding(1) var<storage, read_write> values: array<ELEMENT>;
@group(0) @binding(2) var<uniform> shape: Shape;

@compute @workgroup_size(8, 8, 1) fn main(@builtin(global_invocation_id) id: vec3<u32>) {
    if (id.x >= shape.width || id.y >= shape.height) {
        return;
    }
    let at = vec2<i32>(id.xy);
MEAN    let first = (id.y * shape.width + id.x) * CHANNELSu;
    for (var c = 0u; c < CHANNELSu; c++) {
        values[first + c] = mean[c];
    }
}
";

/// The float mean of READ: the samples added and divided by their count.
const FLOAT_MEAN: &str = "
    var sum = vec4<f32>(0.0);
    for (var s = 0; s < i32(shape.samples); s++) {
        sum += LOAD;
    }
    let mean = sum / f32(shape.samples);
";

/// The integer mean of READ: each sample's quotient by the count, which
/// cannot overflow when added, and its remainder, added and divided by the
/// count once, so the mean is exact before it is rounded.
const INTEGER_MEAN: &str = "
    let n = INTEGER(shape.samples);
    var whole = vec4<INTEGER>(0);
    var part = vec4<INTEGER>(0);
    for (var s = 0; s < i32(shape.samples); s++) {
        let v = textureLoad(source, at, s);
        whole += v / n;
        part += v % n;
    }
    let mean = whole + vec4<INTEGER>(round(vec4<f32>(part) / f32(n)));
";

/// Mip level `level` of layer `layer` of `texture`, read in a shader as
/// `read` says: `read.channels` values a texel, each the four bytes of an
/// f32 for a float mean and of a u32 or i32 for an integer one.
fn shader_read(core: &CoreRef, texture: &wgpu::Texture, read: &ShaderRead, level: u32, layer: u32, stop: Option<&dyn Interrupt>) -> PsResult<Vec<u8>> {
    let size = texture.size().mip_level_size(level, texture.dimension());
    let (width, height) = (size.width, size.height);
    let bytes = u64::from(width) * u64::from(height) * u64::from(read.channels) * 4;
    let values = buffer::create_raw(core, bytes, wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC, "Wgpu texture read", None)?;
    let commands = core.scoped("WgpuTextureRead", "the texture cannot be read in a shader", || {
        let (element, mean) = match &read.mean {
            Mean::Float(load) => ("f32", FLOAT_MEAN.replace("LOAD", load)),
            Mean::Unsigned => ("u32", INTEGER_MEAN.replace("INTEGER", "u32")),
            Mean::Signed => ("i32", INTEGER_MEAN.replace("INTEGER", "i32")),
        };
        let source = READ.replace("SOURCE", read.ty).replace("ELEMENT", element).replace("MEAN", &mean).replace("CHANNELS", &read.channels.to_string());
        let module = core.device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("Wgpu texture read"), source: wgpu::ShaderSource::Wgsl(source.into()) });
        let entry = |binding: u32, ty: wgpu::BindingType| wgpu::BindGroupLayoutEntry { binding, visibility: wgpu::ShaderStages::COMPUTE, ty, count: None };
        let layout = core.device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Wgpu texture read"),
            entries: &[
                entry(0, read.binding),
                entry(1, wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Storage { read_only: false }, has_dynamic_offset: false, min_binding_size: None }),
                entry(2, wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None }),
            ],
        });
        let pipeline_layout = core.device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some("Wgpu texture read"), bind_group_layouts: &[Some(&layout)], immediate_size: 0 });
        let pipeline = core.device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("Wgpu texture read"),
            layout: Some(&pipeline_layout),
            module: &module,
            entry_point: Some("main"),
            compilation_options: Default::default(),
            cache: None,
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor {
            label: Some("Wgpu texture read"),
            dimension: Some(wgpu::TextureViewDimension::D2),
            aspect: read.aspect,
            base_mip_level: level,
            mip_level_count: Some(1),
            base_array_layer: layer,
            array_layer_count: Some(1),
            ..Default::default()
        });
        let mut shape = [0u8; 16];
        for (at, v) in [width, height, texture.sample_count(), 0].into_iter().enumerate() {
            shape[at * 4..at * 4 + 4].copy_from_slice(&v.to_le_bytes());
        }
        let shape = core.device.create_buffer_init(&wgpu::util::BufferInitDescriptor { label: Some("Wgpu texture read"), contents: &shape, usage: wgpu::BufferUsages::UNIFORM });
        let group = core.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Wgpu texture read"),
            layout: &layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&view) },
                wgpu::BindGroupEntry { binding: 1, resource: bound(&values, bytes) },
                wgpu::BindGroupEntry { binding: 2, resource: bound(&shape, 16) },
            ],
        });
        let mut encoder = core.device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("Wgpu texture read") });
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor { label: Some("Wgpu texture read"), timestamp_writes: None });
            pass.set_pipeline(&pipeline);
            pass.set_bind_group(0, &group, &[]);
            pass.dispatch_workgroups(width.div_ceil(8), height.div_ceil(8), 1);
        }
        encoder.finish()
    })?;
    core.submit_and_wait(vec![commands], stop)?;
    buffer::read_bytes(core, &values, 0, bytes, stop)
}

/// Every layer of each of `levels` mip levels of `texture`, read in a
/// shader as `read` says.
fn shader_levels(core: &CoreRef, texture: &wgpu::Texture, read: &ShaderRead, levels: u32, stop: Option<&dyn Interrupt>) -> PsResult<Vec<Vec<u8>>> {
    let mut out = Vec::with_capacity(levels as usize);
    for level in 0..levels {
        let mut bytes = Vec::new();
        for layer in 0..texture.depth_or_array_layers() {
            bytes.extend(shader_read(core, texture, read, level, layer, stop)?);
        }
        out.push(bytes);
    }
    Ok(out)
}

/// Every layer of each of `levels` mip levels of `aspect` of `texture`,
/// copied back from the GPU.
fn copied(core: &CoreRef, texture: &wgpu::Texture, aspect: wgpu::TextureAspect, levels: u32, stop: Option<&dyn Interrupt>) -> PsResult<Vec<Vec<u8>>> {
    let mut out = Vec::with_capacity(levels as usize);
    for level in 0..levels {
        let mut bytes = Vec::new();
        for layer in 0..texture.depth_or_array_layers() {
            bytes.extend(texture::read_texels(core, texture, level, layer, aspect, stop)?);
        }
        out.push(bytes);
    }
    Ok(out)
}

/// The depth and the stencil of `source`'s first `levels` mip levels: the
/// depth copied where its format allows, Depth16Unorm as 16-bit values,
/// and read in a shader for Depth24Plus and Depth24PlusStencil8; the
/// stencil copied and read over 255; a texture with both, the depth as
/// channel 0 and the stencil as channel 1.
fn depth_and_stencil(core: &CoreRef, source: &Source, role: &str, levels: u32, stop: Option<&dyn Interrupt>, said: &mut Said) -> PsResult<Texels> {
    let texture = &source.texture;
    let format = texture.format();
    let depth = if !format.has_depth_aspect() {
        None
    } else if let Some(size) = format.block_copy_size(Some(wgpu::TextureAspect::DepthOnly)) {
        needs(texture, wgpu::TextureUsages::COPY_SRC, role, "to be read back from the GPU")?;
        Some((copied(core, texture, wgpu::TextureAspect::DepthOnly, levels, stop)?, size as usize))
    } else {
        needs(texture, wgpu::TextureUsages::TEXTURE_BINDING, role, &format!("for a shader to read its depth, as no copy reads the depth of {}", format_name(format)))?;
        let read = ShaderRead {
            ty: "texture_depth_2d",
            binding: wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Depth, view_dimension: wgpu::TextureViewDimension::D2, multisampled: false },
            aspect: wgpu::TextureAspect::DepthOnly,
            mean: Mean::Float("vec4<f32>(textureLoad(source, at, 0), 0.0, 0.0, 0.0)".to_string()),
            channels: 1,
        };
        said.verbose.push(format!("the depth of {role} was read in a shader"));
        Some((shader_levels(core, texture, &read, levels, stop)?, 4))
    };
    let stencil = if format.has_stencil_aspect() {
        needs(texture, wgpu::TextureUsages::COPY_SRC, role, "to be read back from the GPU")?;
        Some(copied(core, texture, wgpu::TextureAspect::StencilOnly, levels, stop)?)
    } else {
        None
    };
    let (unpack, size, channels, read): (Unpack, usize, usize, Vec<Vec<u8>>) = match (depth, stencil) {
        (Some((depth, 2)), None) => (unit16, 2, 1, depth),
        (Some((depth, _)), None) => (single, 4, 1, depth),
        (None, Some(stencil)) => (unit8, 1, 1, stencil),
        (Some((depth, _)), Some(stencil)) => {
            let joined = depth
                .iter()
                .zip(&stencil)
                .map(|(d, s)| d.as_chunks::<4>().0.iter().zip(s).flat_map(|(d, s)| [d[0], d[1], d[2], d[3], *s]).collect())
                .collect();
            (depth_stencil, 5, 2, joined)
        }
        (None, None) => return Err(invalid("WgpuTextureRead", format!("{} has neither depth nor stencil to read", format_name(format)))),
    };
    Ok(Texels::of(texture, source.cube, unpack, size, channels, Span::Unit, read))
}

/// Rounded integer channel values, the four bytes of a u32 or i32 each,
/// packed into texels of the integer format `format`.
fn integers(words: &[u8], format: F) -> PsResult<Vec<u8>> {
    let values: Vec<u32> = words.as_chunks::<4>().0.iter().map(|w| u32::from_le_bytes(*w)).collect();
    if format == F::Rgb10a2Uint {
        return Ok(values.as_chunks::<4>().0.iter().flat_map(|t| (t[0] | (t[1] << 10) | (t[2] << 20) | (t[3] << 30)).to_le_bytes()).collect());
    }
    let size = format.block_copy_size(None).ok_or_else(|| invalid("WgpuTextureRead", format!("{} has no texel layout", format_name(format))))? as usize;
    let width = size / usize::from(format.components());
    Ok(values.iter().flat_map(|v| v.to_le_bytes().into_iter().take(width)).collect())
}

/// A multisampled texture as a resolve would store it: each texel's
/// samples averaged in a shader and rounded to the format. Colors are
/// averaged as a shader reads them, an sRGB texture's in linear light,
/// and packed into the format on the CPU; integers and stencil are
/// averaged exactly in integers; depth is averaged as floats, rounded to
/// 16 bits for Depth16Unorm. The texels then read as a single-sample
/// texture's do.
fn resolved(core: &CoreRef, source: &Source, role: &str, stop: Option<&dyn Interrupt>, said: &mut Said) -> PsResult<Texels> {
    let texture = &source.texture;
    let format = texture.format();
    needs(texture, wgpu::TextureUsages::TEXTURE_BINDING, role, "for a shader to average its samples, as no copy reads a multisampled texture")?;
    said.verbose.push(format!("the {} samples of each texel of {role} were averaged in a shader and rounded to {}", texture.sample_count(), format_name(format)));
    let multisampled = |sample_type: wgpu::TextureSampleType| wgpu::BindingType::Texture { sample_type, view_dimension: wgpu::TextureViewDimension::D2, multisampled: true };
    let mut level = Vec::new();
    if format.is_depth_stencil_format() {
        let depth = ShaderRead {
            ty: "texture_depth_multisampled_2d",
            binding: multisampled(wgpu::TextureSampleType::Depth),
            aspect: wgpu::TextureAspect::DepthOnly,
            mean: Mean::Float("vec4<f32>(textureLoad(source, at, s), 0.0, 0.0, 0.0)".to_string()),
            channels: 1,
        };
        let stencil = ShaderRead {
            ty: "texture_multisampled_2d<u32>",
            binding: multisampled(wgpu::TextureSampleType::Uint),
            aspect: wgpu::TextureAspect::StencilOnly,
            mean: Mean::Unsigned,
            channels: 1,
        };
        for layer in 0..texture.depth_or_array_layers() {
            let depths = if format.has_depth_aspect() { Some(shader_read(core, texture, &depth, 0, layer, stop)?) } else { None };
            let stencils = if format.has_stencil_aspect() { Some(shader_read(core, texture, &stencil, 0, layer, stop)?) } else { None };
            match (depths, stencils) {
                (Some(d), None) if format == F::Depth16Unorm => {
                    level.extend(d.as_chunks::<4>().0.iter().flat_map(|w| ((f64::from(f32::from_le_bytes(*w)) * 65535.0).round() as u16).to_le_bytes()));
                }
                (Some(d), None) => level.extend(d),
                (None, Some(s)) => level.extend(s.as_chunks::<4>().0.iter().map(|w| w[0])),
                (Some(d), Some(s)) => level.extend(d.as_chunks::<4>().0.iter().zip(s.as_chunks::<4>().0).flat_map(|(d, s)| [d[0], d[1], d[2], d[3], s[0]])),
                (None, None) => return Err(invalid("WgpuTextureRead", format!("{} has neither depth nor stencil to read", format_name(format)))),
            }
        }
        let (unpack, size, channels): (Unpack, usize, usize) = match (format.has_depth_aspect(), format.has_stencil_aspect()) {
            (true, true) => (depth_stencil, 5, 2),
            (false, _) => (unit8, 1, 1),
            (true, false) if format == F::Depth16Unorm => (unit16, 2, 1),
            (true, false) => (single, 4, 1),
        };
        return Ok(Texels::of(texture, source.cube, unpack, size, channels, Span::Unit, vec![level]));
    }
    let (unpack, span) = reading(format).ok_or_else(|| invalid("WgpuTextureRead", format!("the module reads no multisampled {} texels", format_name(format))))?;
    let size = format.block_copy_size(None).ok_or_else(|| invalid("WgpuTextureRead", format!("{} has no texel layout", format_name(format))))? as usize;
    let channels = usize::from(format.components());
    let (ty, sample_type, mean) = match format.sample_type(None, None) {
        Some(wgpu::TextureSampleType::Uint) => ("texture_multisampled_2d<u32>", wgpu::TextureSampleType::Uint, Mean::Unsigned),
        Some(wgpu::TextureSampleType::Sint) => ("texture_multisampled_2d<i32>", wgpu::TextureSampleType::Sint, Mean::Signed),
        _ => ("texture_multisampled_2d<f32>", wgpu::TextureSampleType::Float { filterable: false }, Mean::Float("textureLoad(source, at, s)".to_string())),
    };
    let read = ShaderRead { ty, binding: multisampled(sample_type), aspect: wgpu::TextureAspect::All, mean, channels: channels as u32 };
    for layer in 0..texture.depth_or_array_layers() {
        let words = shader_read(core, texture, &read, 0, layer, stop)?;
        if let Mean::Float(_) = read.mean {
            let colors: Vec<[f32; 4]> = words
                .as_chunks::<4>()
                .0
                .chunks_exact(channels)
                .map(|texel| {
                    let mut color = [0.0, 0.0, 0.0, 1.0];
                    for (c, w) in texel.iter().enumerate() {
                        color[c] = f32::from_le_bytes(*w);
                    }
                    color
                })
                .collect();
            level.extend(texel::pack(&colors, format)?);
        } else {
            level.extend(integers(&words, format)?);
        }
    }
    Ok(Texels::of(texture, source.cube, unpack, size, channels, span, vec![level]))
}

/// The first `levels` mip levels of planar `source`, every layer, as its
/// planes: its luma, one channel at the texture's size, then its chroma,
/// two channels at the chroma plane's size, which read as Green and Blue.
/// `role` names it in what the read involved, which goes to `said`.
fn planar(core: &CoreRef, source: &Source, role: &str, levels: u32, stop: Option<&dyn Interrupt>, said: &mut Said) -> PsResult<Vec<Texels>> {
    let texture = &source.texture;
    let format = texture.format();
    let layers = if texture.dimension() == wgpu::TextureDimension::D3 { 1 } else { texture.depth_or_array_layers() };
    let mut planes: Vec<Texels> = Vec::with_capacity(2);
    for aspect in [wgpu::TextureAspect::Plane0, wgpu::TextureAspect::Plane1] {
        let read_as = format.aspect_specific_format(aspect).ok_or_else(|| invalid("WgpuTextureRead", format!("the module reads no {} planes", format_name(format))))?;
        let (unpack, span) = reading(read_as).ok_or_else(|| invalid("WgpuTextureRead", format!("the module reads no {} texels", format_name(read_as))))?;
        let size = read_as.block_copy_size(None).ok_or_else(|| invalid("WgpuTextureRead", format!("{} has no texel layout", format_name(read_as))))? as usize;
        let mut read = Vec::with_capacity(levels as usize);
        for level in 0..levels {
            let mut bytes = Vec::new();
            for layer in 0..layers {
                bytes.extend(texture::read_texels(core, texture, level, layer, aspect, stop)?);
            }
            read.push(bytes);
        }
        let (across, down) = format.subsampling_factors(aspect.to_plane());
        let mut texels = Texels::of(texture, source.cube, unpack, size, usize::from(read_as.components()), span, read);
        texels.width /= across;
        texels.height /= down;
        texels.first = planes.iter().map(|plane| plane.channels).sum();
        planes.push(texels);
    }
    if let [luma, chroma] = planes.as_slice() {
        said.verbose.push(format!(
            "{role} is {}, whose luma was read as red at {}x{} and whose chroma as green and blue at {}x{}",
            format_name(format),
            luma.width,
            luma.height,
            chroma.width,
            chroma.height
        ));
    }
    Ok(planes)
}

/// `source`'s first `levels` mip levels, every layer, read back as
/// values: one set of texels, or a planar texture's planes, its luma as
/// Red and its chroma as Green and Blue. `role` names it in a refusal.
/// What the read involved goes to `said`.
fn read(core: &CoreRef, source: &Source, role: &str, levels: u32, stop: Option<&dyn Interrupt>, said: &mut Said) -> PsResult<Vec<Texels>> {
    let texture = &source.texture;
    let format = texture.format();
    if format.is_multi_planar_format() {
        needs(texture, wgpu::TextureUsages::COPY_SRC, role, "to be read back from the GPU")?;
        return planar(core, source, role, levels, stop, said);
    }
    if texture.sample_count() > 1 {
        return Ok(vec![resolved(core, source, role, stop, said)?]);
    }
    if format.is_depth_stencil_format() {
        return Ok(vec![depth_and_stencil(core, source, role, levels, stop, said)?]);
    }
    needs(texture, wgpu::TextureUsages::COPY_SRC, role, "to be read back from the GPU")?;
    let file = image_io::read_file(core, texture, source.cube, levels, stop)?;
    let file = texfile::uncompressed(file, said)?;
    if format.is_compressed() {
        said.verbose.push(format!("the {} texels of {role} were decoded on the CPU into {}", format_name(format), format_name(file.format)));
    }
    let (unpack, span) = reading(file.format).ok_or_else(|| invalid("WgpuTextureRead", format!("the module reads no {} texels", format_name(file.format))))?;
    let size = file.format.block_copy_size(None).ok_or_else(|| invalid("WgpuTextureRead", format!("{} has no texel layout", format_name(file.format))))? as usize;
    let mut texels = Texels::of(texture, source.cube, unpack, size, usize::from(file.format.components()), span, file.levels);
    texels.wide = file.format == F::R64Uint;
    Ok(vec![texels])
}

/// How a refusal names `texture`'s dimension.
fn dimension_name(texture: &wgpu::Texture) -> &'static str {
    match texture.dimension() {
        wgpu::TextureDimension::D1 => "1D",
        wgpu::TextureDimension::D2 => "2D",
        wgpu::TextureDimension::D3 => "3D",
    }
}

/// How a refusal names `texture`'s layers: its depth for a 3D texture.
fn layer_count(texture: &wgpu::Texture) -> String {
    match (texture.dimension(), texture.depth_or_array_layers()) {
        (wgpu::TextureDimension::D3, n) => format!("a depth of {n}"),
        (_, 1) => "1 layer".to_string(),
        (_, n) => format!("{n} layers"),
    }
}

/// A texture as Compare and Measure take it: its device, and its texels.
pub(crate) type Operand = (CoreRef, Source);

/// The texels of `reference` and `texture` over the mip levels both have,
/// each set with the other's and the number of channels both store: one
/// pair, or a pair for each plane of two planar textures. Refused unless
/// they have the same dimension, width, height and layers, and both keep
/// their texels in planes or neither does.
fn pair(reference: &Operand, texture: &Operand, stop: Option<&dyn Interrupt>, said: &mut Said) -> PsResult<Vec<(Texels, Texels, usize)>> {
    let (r, t) = (&reference.1.texture, &texture.1.texture);
    if r.dimension() != t.dimension() {
        let (reference_dimension, texture_dimension) = (dimension_name(r), dimension_name(t));
        return Err(invalid(
            "WgpuCompare",
            format!("the reference is a {reference_dimension} texture and the texture a {texture_dimension} one: textures compare only with the same dimension"),
        ));
    }
    if (r.width(), r.height()) != (t.width(), t.height()) {
        return Err(invalid(
            "WgpuCompare",
            format!("the reference is {}x{} and the texture {}x{}: textures compare only at the same width and height", r.width(), r.height(), t.width(), t.height()),
        ));
    }
    if r.depth_or_array_layers() != t.depth_or_array_layers() {
        let what = if r.dimension() == wgpu::TextureDimension::D3 { "depth" } else { "number of layers" };
        let (reference_layers, texture_layers) = (layer_count(r), layer_count(t));
        return Err(invalid("WgpuCompare", format!("the reference has {reference_layers} and the texture {texture_layers}: textures compare only with the same {what}")));
    }
    if r.format().is_multi_planar_format() != t.format().is_multi_planar_format() {
        let (reference_format, texture_format) = (format_name(r.format()), format_name(t.format()));
        return Err(invalid(
            "WgpuCompare",
            format!("the reference is {reference_format} and the texture {texture_format}: textures compare only when both keep their texels in planes or neither does"),
        ));
    }
    let levels = r.mip_level_count().min(t.mip_level_count());
    let a = read(&reference.0, &reference.1, "the reference", levels, stop, said)?;
    let b = read(&texture.0, &texture.1, "the texture", levels, stop, said)?;
    Ok(a.into_iter()
        .zip(b)
        .map(|(a, b)| {
            let channels = a.channels.min(b.channels);
            (a, b, channels)
        })
        .collect())
}

/// A sum of doubles with Neumaier's compensation, which carries the low
/// bits each addition rounds away, so a sum of millions of texels keeps
/// its last digits.
#[derive(Default)]
struct Sum {
    sum: f64,
    compensation: f64,
}

impl Sum {
    fn add(&mut self, v: f64) {
        let t = self.sum + v;
        // An infinite or NaN sum has no low bits to carry.
        if t.is_finite() {
            self.compensation += if self.sum.abs() >= v.abs() { (self.sum - t) + v } else { (v - t) + self.sum };
        }
        self.sum = t;
    }

    fn total(&self) -> f64 {
        self.sum + self.compensation
    }
}

/// The difference of channel `channel` of texel `texel` between two
/// planes: taken in integers for two R64Uint textures.
fn difference_at(a: &Texels, pa: &[u8], b: &Texels, pb: &[u8], texel: usize, channel: usize) -> f64 {
    if a.wide && b.wide {
        let at = texel * 8;
        let wide = i128::from(word64(&pa[at..at + 8])) - i128::from(word64(&pb[at..at + 8]));
        return wide as f64 / u64::MAX as f64;
    }
    a.value(pa, texel, channel) - b.value(pb, texel, channel)
}

/// The mean squared difference of channel `channel` between two planes of
/// `n` texels, and the reference's largest value in it.
fn squared_error(a: &Texels, pa: &[u8], b: &Texels, pb: &[u8], channel: usize, n: usize) -> (f64, f64) {
    let mut sum = Sum::default();
    let mut largest = f64::NEG_INFINITY;
    for texel in 0..n {
        let d = difference_at(a, pa, b, pb, texel, channel);
        sum.add(d * d);
        largest = largest.max(a.value(pa, texel, channel));
    }
    (sum.total() / n as f64, largest)
}

/// The weights of SSIM's window along one side: a Gaussian of sigma 1.5
/// truncated at 3.5 sigma, 11 taps summing to 1, as scipy's gaussian_filter
/// makes them for scikit-image.
fn window_weights() -> [f64; WINDOW] {
    let mut weights = [0.0; WINDOW];
    for (at, w) in weights.iter_mut().enumerate() {
        let x = at as f64 - 5.0;
        *w = (-0.5 * x * x / 2.25).exp();
    }
    let sum: f64 = weights.iter().sum();
    weights.map(|w| w / sum)
}

/// SSIM's pass over one band of a plane's rows. Each workgroup loads the
/// 26 by 26 texels the windows of its 16 by 16 texels cover, finds each
/// window's weighted means, then the weighted variances and covariance
/// about them, so no large squares cancel in single precision, and adds
/// its texels' SSIM into one sum.
const SSIM_PASS: &str = "
struct Band {
    width: u32,
    columns: u32,
    rows: u32,
    unused: u32,
    c1: f32,
    c2: f32,
    unused1: f32,
    unused2: f32,
}

@group(0) @binding(0) var<storage, read> xs: array<f32>;
@group(0) @binding(1) var<storage, read> ys: array<f32>;
@group(0) @binding(2) var<storage, read_write> sums: array<f32>;
@group(0) @binding(3) var<uniform> band: Band;

var<private> weights: array<f32, 11> = array<f32, 11>(WEIGHTS);
var<workgroup> tile_x: array<f32, 676>;
var<workgroup> tile_y: array<f32, 676>;
var<workgroup> partial: array<f32, 256>;

@compute @workgroup_size(16, 16, 1)
fn main(@builtin(local_invocation_id) local: vec3<u32>, @builtin(local_invocation_index) index: u32, @builtin(workgroup_id) group: vec3<u32>, @builtin(num_workgroups) groups: vec3<u32>) {
    let corner = group.xy * 16u;
    for (var i = index; i < 676u; i += 256u) {
        let at = corner + vec2<u32>(i % 26u, i / 26u);
        var x = 0.0;
        var y = 0.0;
        if (at.x < band.width && at.y < band.rows + 10u) {
            x = xs[at.y * band.width + at.x];
            y = ys[at.y * band.width + at.x];
        }
        tile_x[i] = x;
        tile_y[i] = y;
    }
    workgroupBarrier();
    var s = 0.0;
    let texel = corner + local.xy;
    if (texel.x < band.columns && texel.y < band.rows) {
        var mean = vec2<f32>(0.0);
        for (var j = 0u; j < 11u; j++) {
            for (var i = 0u; i < 11u; i++) {
                let at = (local.y + j) * 26u + local.x + i;
                mean += weights[j] * weights[i] * vec2<f32>(tile_x[at], tile_y[at]);
            }
        }
        var moments = vec3<f32>(0.0);
        for (var j = 0u; j < 11u; j++) {
            for (var i = 0u; i < 11u; i++) {
                let at = (local.y + j) * 26u + local.x + i;
                let d = vec2<f32>(tile_x[at], tile_y[at]) - mean;
                moments += weights[j] * weights[i] * vec3<f32>(d.x * d.x, d.y * d.y, d.x * d.y);
            }
        }
        s = ((2.0 * mean.x * mean.y + band.c1) * (2.0 * moments.z + band.c2)) / ((mean.x * mean.x + mean.y * mean.y + band.c1) * (moments.x + moments.y + band.c2));
    }
    partial[index] = s;
    workgroupBarrier();
STEPS    if (index == 0u) {
        sums[group.y * groups.x + group.x] = partial[0];
    }
}
";

/// SSIM's compute pass, built once for a comparison.
struct Ssim {
    layout: wgpu::BindGroupLayout,
    pipeline: wgpu::ComputePipeline,
}

impl Ssim {
    fn new(core: &CoreRef) -> PsResult<Ssim> {
        let weights: Vec<String> = window_weights().iter().map(|w| format!("{:?}", *w as f32)).collect();
        let steps: String = [128u32, 64, 32, 16, 8, 4, 2, 1].iter().map(|w| format!("    if (index < {w}u) {{\n        partial[index] += partial[index + {w}u];\n    }}\n    workgroupBarrier();\n")).collect();
        let source = SSIM_PASS.replace("WEIGHTS", &weights.join(", ")).replace("STEPS", &steps);
        core.scoped("WgpuSsim", "SSIM's pass cannot be built", || {
            let module = core.device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("Wgpu SSIM"), source: wgpu::ShaderSource::Wgsl(source.into()) });
            let entry = |binding: u32, ty: wgpu::BufferBindingType| wgpu::BindGroupLayoutEntry {
                binding,
                visibility: wgpu::ShaderStages::COMPUTE,
                ty: wgpu::BindingType::Buffer { ty, has_dynamic_offset: false, min_binding_size: None },
                count: None,
            };
            let read = wgpu::BufferBindingType::Storage { read_only: true };
            let layout = core.device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("Wgpu SSIM"),
                entries: &[entry(0, read), entry(1, read), entry(2, wgpu::BufferBindingType::Storage { read_only: false }), entry(3, wgpu::BufferBindingType::Uniform)],
            });
            let pipeline_layout = core.device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some("Wgpu SSIM"), bind_group_layouts: &[Some(&layout)], immediate_size: 0 });
            let pipeline = core.device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some("Wgpu SSIM"),
                layout: Some(&pipeline_layout),
                module: &module,
                entry_point: Some("main"),
                // The pass writes each value of its workgroup memory before
                // it reads it, so it leaves out the zero fill wgpu adds by
                // default: WARP takes seconds to compile that fill of 1608
                // values on x64, and compiling it ends the process on Arm64.
                compilation_options: wgpu::PipelineCompilationOptions { constants: &[], zero_initialize_workgroup_memory: false },
                cache: None,
            });
            Ssim { layout, pipeline }
        })
    }

    /// The mean SSIM of `y` against `x`, planes of `width` by `height`
    /// values, over the texels whose 11 by 11 windows are inside them,
    /// with data range `range`: K1 0.01 and K2 0.03.
    // clippy::too_many_arguments: the planes, their shape, the range, the
    // device and the interrupt are each what one band of the pass needs.
    #[allow(clippy::too_many_arguments)]
    fn mean(&self, core: &CoreRef, x: &[f32], y: &[f32], width: usize, height: usize, range: f64, stop: Option<&dyn Interrupt>) -> PsResult<f64> {
        let (columns, rows) = (width - (WINDOW - 1), height - (WINDOW - 1));
        let limit = core.limits.max_storage_buffer_binding_size.min(BAND_BYTES);
        let most_groups = u64::from(core.limits.max_compute_workgroups_per_dimension);
        let band_rows = (limit / (width as u64 * 4)).saturating_sub(WINDOW as u64 - 1).min(most_groups * 16).min(rows as u64) as usize;
        if band_rows == 0 || (columns as u64).div_ceil(16) > most_groups {
            return Err(invalid("WgpuSsim", format!("a level {width} texels wide is more than SSIM's pass takes on {}, whose storage bindings hold {limit} bytes", core.info.name)));
        }
        let c1 = (0.01 * range).powi(2) as f32;
        let c2 = (0.03 * range).powi(2) as f32;
        let mut total = 0.0;
        let mut first = 0;
        while first < rows {
            let count = band_rows.min(rows - first);
            let span = first * width..(first + count + WINDOW - 1) * width;
            let groups = (columns.div_ceil(16) as u32, count.div_ceil(16) as u32);
            let sums_bytes = u64::from(groups.0) * u64::from(groups.1) * 4;
            let xs: &[u8] = bytemuck::cast_slice(&x[span.clone()]);
            let ys: &[u8] = bytemuck::cast_slice(&y[span]);
            let mut constants = [0u8; 32];
            for (at, word) in [width as u32, columns as u32, count as u32, 0].into_iter().enumerate() {
                constants[at * 4..at * 4 + 4].copy_from_slice(&word.to_le_bytes());
            }
            constants[16..20].copy_from_slice(&c1.to_le_bytes());
            constants[20..24].copy_from_slice(&c2.to_le_bytes());
            let x_buffer = buffer::create_raw(core, xs.len() as u64, wgpu::BufferUsages::STORAGE, "Wgpu SSIM", Some(xs))?;
            let y_buffer = buffer::create_raw(core, ys.len() as u64, wgpu::BufferUsages::STORAGE, "Wgpu SSIM", Some(ys))?;
            let sums = buffer::create_raw(core, sums_bytes, wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC, "Wgpu SSIM", None)?;
            let band = buffer::create_raw(core, 32, wgpu::BufferUsages::UNIFORM, "Wgpu SSIM", Some(&constants))?;
            let commands = core.scoped("WgpuSsim", "SSIM cannot be computed", || {
                let group = core.device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("Wgpu SSIM"),
                    layout: &self.layout,
                    entries: &[
                        wgpu::BindGroupEntry { binding: 0, resource: bound(&x_buffer, xs.len() as u64) },
                        wgpu::BindGroupEntry { binding: 1, resource: bound(&y_buffer, ys.len() as u64) },
                        wgpu::BindGroupEntry { binding: 2, resource: bound(&sums, sums_bytes) },
                        wgpu::BindGroupEntry { binding: 3, resource: bound(&band, 32) },
                    ],
                });
                let mut encoder = core.device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("Wgpu SSIM") });
                {
                    let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor { label: Some("Wgpu SSIM"), timestamp_writes: None });
                    pass.set_pipeline(&self.pipeline);
                    pass.set_bind_group(0, &group, &[]);
                    pass.dispatch_workgroups(groups.0, groups.1, 1);
                }
                encoder.finish()
            })?;
            core.submit_and_wait(vec![commands], stop)?;
            let bytes = buffer::read_bytes(core, &sums, 0, sums_bytes, stop)?;
            total += bytes.as_chunks::<4>().0.iter().map(|w| f64::from(f32::from_le_bytes(*w))).sum::<f64>();
            first += count;
        }
        Ok(total / (columns * rows) as f64)
    }
}

/// Compare(reference): a row for each mip level both textures have, each
/// layer and each channel both store, with the mean squared error, PSNR
/// and SSIM of `texture`'s values against `reference`'s. SSIM runs on
/// `texture`'s device. What reading them involved goes to `said`.
pub(crate) fn compared(reference: &Operand, texture: &Operand, stop: Option<&dyn Interrupt>, said: &mut Said) -> PsResult<Vec<TextureComparison>> {
    let pairs = pair(reference, texture, stop, said)?;
    let core = &texture.0;
    // The first set is the texture's size; a chroma plane after it is
    // smaller.
    let full = &pairs[0].0;
    let ssim = if full.width as usize >= WINDOW && full.height as usize >= WINDOW { Some(Ssim::new(core)?) } else { None };
    let mut rows = Vec::new();
    for level in 0..full.levels.len() {
        for slice in 0..full.slices(level) {
            for (a, b, channels) in &pairs {
                let (width, height) = a.extent(level);
                let n = width * height;
                let (pa, pb) = (a.slice(level, slice), b.slice(level, slice));
                for channel in 0..*channels {
                    let (mse, largest) = squared_error(a, pa, b, pb, channel, n);
                    let peak = a.span.peak(largest);
                    let psnr = if mse == 0.0 { f64::INFINITY } else { 10.0 * (peak * peak / mse).log10() };
                    let similarity = match &ssim {
                        Some(pass) if width >= WINDOW && height >= WINDOW => Some(pass.mean(core, &a.floats(pa, channel, n), &b.floats(pb, channel, n), width, height, peak, stop)?),
                        _ => None,
                    };
                    rows.push(TextureComparison {
                        level: level as u32,
                        layer: slice as u32,
                        channel: TextureChannel::ALL[a.first + channel],
                        m_s_e: mse,
                        p_s_n_r: psnr,
                        s_s_i_m: similarity,
                    });
                }
            }
        }
    }
    Ok(rows)
}

/// An error's brightness on the difference picture's scale, from 0 to 1:
/// 0 at 2^-12 or less, 1 at 1 or more, each twelfth doubling the error; a
/// NaN error is 1.
fn brightness(error: f64) -> f64 {
    if error.is_nan() {
        1.0
    } else if error <= FLOOR {
        0.0
    } else {
        ((error.log2() + 12.0) / 12.0).clamp(0.0, 1.0)
    }
}

fn shade(brightness: f64) -> u8 {
    (brightness * 255.0).round() as u8
}

/// The channels a refusal lists: "red", "red and green", "red, green and
/// blue".
fn channel_list(channels: usize) -> String {
    let names: Vec<&str> = TextureChannel::ALL[..channels].iter().map(|c| c.name()).collect();
    match names.as_slice() {
        [] => "none".to_string(),
        [only] => (*only).to_string(),
        [rest @ .., last] => format!("{} and {last}", rest.join(", ")),
    }
}

/// Difference(reference, channel): an Rgba8Unorm texture of the
/// textures' size, layers and the mip levels both have, each texel the
/// error of `texture`'s values against `reference`'s on a fixed log scale,
/// red, green and blue in their own colors, or `channel` as gray, with
/// alpha opaque. It is made on `texture`'s device.
pub(crate) fn difference(reference: &Operand, texture: &Operand, channel: Option<TextureChannel>, stop: Option<&dyn Interrupt>, said: &mut Said) -> PsResult<Texture> {
    let pairs = pair(reference, texture, stop, said)?;
    let channels = pairs.iter().fold(0, |most, (a, _, shared)| most.max(a.first + shared));
    if let Some(c) = channel
        && c.index() >= channels
    {
        let (channel, shared) = (c.name(), channel_list(channels));
        return Err(invalid("WgpuCompare", format!("the textures do not both store {channel}: they share {shared}")));
    }
    let full = &pairs[0].0;
    let mut levels = Vec::with_capacity(full.levels.len());
    for level in 0..full.levels.len() {
        let (width, height) = full.extent(level);
        let n = width * height;
        let mut out = Vec::with_capacity(full.slices(level) * n * 4);
        for slice in 0..full.slices(level) {
            // For each channel both store, in order: the slices holding
            // it, the channel among theirs, the width of their rows, and
            // the texels of the picture each of theirs covers across and
            // down, 2 for the chroma of a planar texture.
            let mut held = Vec::with_capacity(channels);
            for (a, b, shared) in &pairs {
                let (w, h) = a.extent(level);
                for k in 0..*shared {
                    held.push((a, a.slice(level, slice), b, b.slice(level, slice), k, w, width / w, height / h));
                }
            }
            let error = |texel: usize, c: usize| {
                let (a, pa, b, pb, k, w, across, down) = held[c];
                let (x, y) = (texel % width, texel / width);
                shade(brightness(difference_at(a, pa, b, pb, (y / down) * w + x / across, k).abs()))
            };
            for texel in 0..n {
                match channel {
                    Some(c) => {
                        let gray = error(texel, c.index());
                        out.extend([gray, gray, gray, 255]);
                    }
                    None => {
                        let mut rgba = [0, 0, 0, 255];
                        for (c, v) in rgba.iter_mut().enumerate().take(channels.min(3)) {
                            *v = error(texel, c);
                        }
                        out.extend(rgba);
                    }
                }
            }
        }
        levels.push(out);
    }
    let file = TextureFile { dimension: full.dimension, width: full.width, height: full.height, depth: full.depth, layers: full.layers, cube: full.cube, format: F::Rgba8Unorm, levels };
    let core = &texture.0;
    let mut usage = texture::usable(core, F::Rgba8Unorm);
    if full.dimension == wgpu::TextureDimension::D1 {
        usage -= wgpu::TextureUsages::RENDER_ATTACHMENT;
    }
    let shown = match channel {
        Some(c) => format!("its {} channel as gray", c.name()),
        None => "red, green and blue in their own colors".to_string(),
    };
    said.verbose.push(format!(
        "each texel is the error of {} against {} on a log scale, black at 2^-12 or less and full at 1 or more, each twelfth of brightness doubling the error: {shown}",
        texture.1.label, reference.1.label
    ));
    if let [_, (chroma, _, _)] = pairs.as_slice()
        && channel.is_none_or(|c| c.index() >= chroma.first)
    {
        let (across, down) = (full.width / chroma.width, full.height / chroma.height);
        said.verbose.push(format!("each chroma texel's error fills the {across} by {down} texels that share it"));
    }
    let notes = said.verbose.iter().chain(&said.warnings).cloned().collect();
    image_io::upload(core, &file, Some(usage), format!("{}x{} {}", full.width, full.height, format_name(F::Rgba8Unorm)), notes)
}

/// The low and high end of a histogram's range: two finite numbers, the
/// first below the second.
fn range_of(range: &[f64]) -> PsResult<(f64, f64)> {
    match *range {
        [low, high] if low.is_finite() && high.is_finite() && low < high && (high - low).is_finite() => Ok((low, high)),
        _ => Err(invalid("WgpuMeasure", "-Range takes two numbers, the low end and then the high end, finite, the first below the second")),
    }
}

/// The minimum, maximum, mean and population standard deviation of
/// channel `channel` of a plane's `n` texels, all NaN when a value is NaN,
/// with each value but NaN counted into `histogram`, whose bins split
/// `low` to `high` evenly: values below it count in the first bin, values
/// above it in the last.
fn statistics(t: &Texels, plane: &[u8], channel: usize, n: usize, (low, high): (f64, f64), histogram: &mut [u64]) -> (f64, f64, f64, f64) {
    let mut minimum = f64::INFINITY;
    let mut maximum = f64::NEG_INFINITY;
    let mut sum = Sum::default();
    let mut nan = false;
    for texel in 0..n {
        let v = t.value(plane, texel, channel);
        if v.is_nan() {
            nan = true;
            continue;
        }
        minimum = minimum.min(v);
        maximum = maximum.max(v);
        sum.add(v);
    }
    let mean = sum.total() / n as f64;
    let bins = histogram.len();
    let mut squares = Sum::default();
    for texel in 0..n {
        let v = t.value(plane, texel, channel);
        if v.is_nan() {
            continue;
        }
        squares.add((v - mean) * (v - mean));
        let at = (v - low) / (high - low) * bins as f64;
        let bin = if at >= bins as f64 {
            bins - 1
        } else if at > 0.0 {
            (at as usize).min(bins - 1)
        } else {
            0
        };
        histogram[bin] += 1;
    }
    if nan {
        return (f64::NAN, f64::NAN, f64::NAN, f64::NAN);
    }
    (minimum, maximum, mean, (squares.total() / n as f64).sqrt())
}

/// Measure(bins, range): a row for each mip level, layer and channel of
/// `texture`, with its values' minimum, maximum, mean, standard deviation
/// and a histogram of `bins` bins over `range`. What reading it involved
/// goes to `said`.
pub(crate) fn measured(texture: &Operand, bins: u32, range: &[f64], stop: Option<&dyn Interrupt>, said: &mut Said) -> PsResult<Vec<TextureMeasurement>> {
    let range = range_of(range)?;
    if bins == 0 {
        return Err(invalid("WgpuMeasure", "-Bins is at least 1"));
    }
    let planes = read(&texture.0, &texture.1, "the texture", texture.1.texture.mip_level_count(), stop, said)?;
    let full = &planes[0];
    let mut rows = Vec::new();
    for level in 0..full.levels.len() {
        for slice in 0..full.slices(level) {
            for t in &planes {
                let (width, height) = t.extent(level);
                let texels = t.slice(level, slice);
                for channel in 0..t.channels {
                    let mut histogram = Vec::new();
                    histogram.try_reserve_exact(bins as usize).map_err(|e| invalid("WgpuMeasure", format!("a histogram of {bins} bins cannot be held in memory: {e}")))?;
                    histogram.resize(bins as usize, 0);
                    let (minimum, maximum, average, standard_deviation) = statistics(t, texels, channel, width * height, range, &mut histogram);
                    rows.push(TextureMeasurement {
                        level: level as u32,
                        layer: slice as u32,
                        channel: TextureChannel::ALL[t.first + channel],
                        minimum,
                        maximum,
                        average,
                        standard_deviation,
                        histogram,
                    });
                }
            }
        }
    }
    Ok(rows)
}

/// The bins Measure-WgpuTexture and Measure(bins, range) take when none
/// are given.
pub(crate) const DEFAULT_BINS: u32 = 256;

/// The range Measure-WgpuTexture and Measure(bins, range) take when none
/// is given.
pub(crate) const DEFAULT_RANGE: [f64; 2] = [0.0, 1.0];

/// Compares a texture with a reference, such as a block-compressed copy
/// with its original: for each mip level both have, each layer and each
/// channel both store, a row with the mean squared error (MSE), the peak
/// signal-to-noise ratio (PSNR) and the structural similarity (SSIM) of
/// the texture's values against the reference's. With -Difference it
/// writes a picture of the error instead.
///
/// The textures need the same dimension, width, height and layers, and an
/// NV12 or P010 texture compares only with another NV12 or P010 texture;
/// their formats and mip levels may differ. Each value is read as the
/// texture stores it: from 0 to 1, an sRGB texture in its encoded form, a
/// block-compressed one decoded on the CPU first; snorm values from -1 to
/// 1; an integer format through the norm format with its bits, as R8Uint
/// reads as R8Unorm; depth from 0 to 1 and stencil over 255, a
/// depth-stencil texture's depth as Red and its stencil as Green; a
/// multisampled texture as a resolve would store it, each texel's samples
/// averaged and rounded to the format; and an NV12 or P010 texture's luma
/// as Red at the texture's size and its chroma as Green and Blue at half
/// its width and height.
///
/// PSNR's peak is 1, 2 for values from -1 to 1, and for a float texture
/// the reference channel's largest value in that level and layer, or 1
/// when that is 0 or below. SSIM is Wang et al.'s, as scikit-image computes
/// it with a Gaussian window of sigma 1.5 and the peak as its data range,
/// averaged over the level less a 5-texel border; a level under 11 texels
/// on a side has none.
///
/// The difference picture is an Rgba8Unorm texture of the textures' size,
/// layers and shared mip levels. Each texel shows the error on a fixed log
/// scale: black at 2^-12 or less, full at 1 or more, each twelfth of
/// brightness doubling the error, so the same brightness means the same
/// error in every picture. Red, green and blue show in their own colors
/// with alpha opaque, or -Channel shows one channel, alpha included, as
/// gray. The chroma of an NV12 or P010 texture shows each chroma texel's
/// error over the 2 by 2 texels that share it.
///
/// # Examples
/// Compare-WgpuTexture $original $bc7
/// $bc7 | Compare-WgpuTexture $original
/// $bc7 | Compare-WgpuTexture $original | Where-Object Level -EQ 0
/// $bc7 | Compare-WgpuTexture $original -Difference | Export-WgpuImage -Path .\error.png
/// $bc7 | Compare-WgpuTexture $original -Difference -Channel Alpha
#[cmdlet(verb = "Compare", noun = "WgpuTexture", alias = ["Compare-GpuTexture"], default_parameter_set = "Metrics", output = ["Wgpu.TextureComparison", "Wgpu.Texture"])]
#[derive(Default)]
pub struct CompareWgpuTexture {
    /// The texture to compare with, such as the original a copy was made from.
    #[param(mandatory, position = 0)]
    pub reference: Option<PsProxy<Texture>>,
    /// The texture to check against the reference, such as a block-compressed copy of it.
    #[param(mandatory, position = 1, value_from_pipeline)]
    pub texture: Option<PsProxy<Texture>>,
    /// Writes a picture of the error, an Rgba8Unorm texture, instead of the rows.
    #[param(mandatory, set = "Difference")]
    pub difference: bool,
    /// The channel the picture shows, as gray: Red, Green, Blue or Alpha; red, green and blue in their own colors when omitted.
    #[param(set = "Difference")]
    pub channel: Option<TextureChannel>,
}

impl Cmdlet for CompareWgpuTexture {
    fn process(&mut self, ps: &Pipeline<'_>) -> PsResult<()> {
        let (Some(reference), Some(texture)) = (&self.reference, &self.texture) else {
            return Err(invalid("WgpuNoTexture", "no texture was given"));
        };
        let reference = reference.with(|t| (t.core.clone(), t.source()))?;
        let texture = texture.with(|t| (t.core.clone(), t.source()))?;
        let mut said = Said::default();
        if self.difference {
            let made = difference(&reference, &texture, self.channel, Some(ps), &mut said);
            tell(ps, &said)?;
            return ps.write(made?);
        }
        let rows = compared(&reference, &texture, Some(ps), &mut said);
        tell(ps, &said)?;
        for row in rows? {
            ps.write(row)?;
        }
        Ok(())
    }
}

/// Measures a texture's values: for each mip level, layer and channel, a
/// row with the minimum, maximum, mean, population standard deviation and
/// a histogram of the values.
///
/// Each value is read as Compare-WgpuTexture reads it: from 0 to 1 as the
/// texture stores it, an sRGB texture in its encoded form, a
/// block-compressed one decoded on the CPU first; snorm values from -1 to
/// 1; an integer format through the norm format with its bits; depth from
/// 0 to 1 and stencil over 255; a multisampled texture as a resolve would
/// store it; and an NV12 or P010 texture's luma as Red and its chroma as
/// Green and Blue, measured at the chroma plane's size.
///
/// The histogram's bins split -Range evenly, 256 bins from 0 to 1 when
/// omitted, so each 8-bit value has a bin of its own. Values below the
/// range count in the first bin and values above it in the last. A NaN
/// value counts in no bin and makes the row's minimum, maximum, mean and
/// standard deviation NaN.
///
/// # Examples
/// Measure-WgpuTexture $tex
/// $tex | Measure-WgpuTexture | Where-Object Level -EQ 0
/// $hdr | Measure-WgpuTexture -Range 0, 16 -Bins 64
/// (Measure-WgpuTexture $tex | Select-Object -First 1).Histogram
#[cmdlet(verb = "Measure", noun = "WgpuTexture", alias = ["Measure-GpuTexture"], output = ["Wgpu.TextureMeasurement"])]
#[derive(Default)]
pub struct MeasureWgpuTexture {
    /// The texture to measure.
    #[param(mandatory, position = 0, value_from_pipeline)]
    pub texture: Option<PsProxy<Texture>>,
    /// The histogram's bins: 256 when omitted.
    #[param]
    pub bins: Option<u32>,
    /// The values the histogram's bins split: the low end and the high end, 0 and 1 when omitted.
    #[param]
    pub range: Option<Vec<f64>>,
}

impl Cmdlet for MeasureWgpuTexture {
    fn process(&mut self, ps: &Pipeline<'_>) -> PsResult<()> {
        let Some(texture) = &self.texture else {
            return Err(invalid("WgpuNoTexture", "no texture was given"));
        };
        let texture = texture.with(|t| (t.core.clone(), t.source()))?;
        let mut said = Said::default();
        let rows = measured(&texture, self.bins.unwrap_or(DEFAULT_BINS), self.range.as_deref().unwrap_or(&DEFAULT_RANGE), Some(ps), &mut said);
        tell(ps, &said)?;
        for row in rows? {
            ps.write(row)?;
        }
        Ok(())
    }
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

    /// The error `result` holds; the test fails when it holds a value.
    fn refused<T>(result: PsResult<T>) -> PsError {
        match result {
            Ok(_) => panic!("expected a refusal"),
            Err(e) => e,
        }
    }

    /// A 2D file of `layers` layers and the given levels.
    fn file(format: F, width: u32, height: u32, layers: u32, levels: Vec<Vec<u8>>) -> TextureFile {
        TextureFile { dimension: wgpu::TextureDimension::D2, width, height, depth: 1, layers, cube: false, format, levels }
    }

    /// `file` on `core`, with every usage its format allows.
    fn upload(core: &CoreRef, file: &TextureFile) -> Operand {
        let texture = image_io::upload(core, file, None, "test".to_string(), Vec::new()).expect("uploaded");
        (core.clone(), texture.source())
    }

    fn floats(values: &[f32]) -> Vec<u8> {
        values.iter().flat_map(|v| v.to_le_bytes()).collect()
    }

    /// A 16 by 16 texture of `format` with `samples` samples a texel,
    /// cleared by a render pass: color to `color`, depth to `depth` and
    /// stencil to `stencil`.
    fn cleared(core: &CoreRef, format: F, samples: u32, usage: wgpu::TextureUsages, color: [f64; 4], depth: f32, stencil: u32) -> Operand {
        let texture = core
            .scoped("WgpuTexture", "the test texture cannot be made", || {
                core.device.create_texture(&wgpu::TextureDescriptor {
                    label: Some("cleared"),
                    size: wgpu::Extent3d { width: 16, height: 16, depth_or_array_layers: 1 },
                    mip_level_count: 1,
                    sample_count: samples,
                    dimension: wgpu::TextureDimension::D2,
                    format,
                    usage: usage | wgpu::TextureUsages::RENDER_ATTACHMENT,
                    view_formats: &[],
                })
            })
            .expect("made");
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        let commands = core
            .scoped("WgpuTexture", "the test texture cannot be cleared", || {
                let mut encoder = core.device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("cleared") });
                let targets: Vec<Option<wgpu::RenderPassColorAttachment>> = if format.is_depth_stencil_format() {
                    Vec::new()
                } else {
                    vec![Some(wgpu::RenderPassColorAttachment {
                        view: &view,
                        depth_slice: None,
                        resolve_target: None,
                        ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color { r: color[0], g: color[1], b: color[2], a: color[3] }), store: wgpu::StoreOp::Store },
                    })]
                };
                let depth_stencil = format.is_depth_stencil_format().then(|| wgpu::RenderPassDepthStencilAttachment {
                    view: &view,
                    depth_ops: format.has_depth_aspect().then_some(wgpu::Operations { load: wgpu::LoadOp::Clear(depth), store: wgpu::StoreOp::Store }),
                    stencil_ops: format.has_stencil_aspect().then_some(wgpu::Operations { load: wgpu::LoadOp::Clear(stencil), store: wgpu::StoreOp::Store }),
                });
                let pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("cleared"),
                    color_attachments: &targets,
                    depth_stencil_attachment: depth_stencil,
                    timestamp_writes: None,
                    occlusion_query_set: None,
                    multiview_mask: None,
                });
                drop(pass);
                encoder.finish()
            })
            .expect("recorded");
        core.submit_and_wait(vec![commands], None).expect("cleared");
        (core.clone(), Source { texture, cube: false, label: "cleared".to_string() })
    }

    fn splitmix(z: u64) -> u64 {
        let mut z = z.wrapping_add(0x9E37_79B9_7F4A_7C15);
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    fn hashed(salt: u64, x: u64, y: u64, c: u64) -> u64 {
        splitmix(((salt * 1_000_003 + y) * 1_000_003 + x) * 8 + c)
    }

    /// Level `level` of layer `layer` of the pair scikit-image and numpy
    /// were given for SKIMAGE and NUMPY, built the same way there: the
    /// reference's Rgba8Unorm texels, each a mix of its position and a
    /// splitmix64 hash of it, and the texture's, which add to each value an
    /// error from -12 to 12.
    fn oracle_level(level: u32, layer: u32) -> (Vec<u8>, Vec<u8>) {
        let (width, height) = ((40u64 >> level).max(1), (33u64 >> level).max(1));
        let (l, k) = (u64::from(level), u64::from(layer));
        let (mut reference, mut texture) = (Vec::new(), Vec::new());
        for y in 0..height {
            for x in 0..width {
                for c in 0..4 {
                    let v = (x * 6 + y * 4 + c * 40) % 200 + ((x / 4 + y / 4) % 2) * 40 + hashed(1 + 10 * l + 100 * k, x, y, c) % 13;
                    reference.push(v as u8);
                    texture.push((v as i64 + (hashed(2 + 10 * l + 100 * k, x, y, c) % 25) as i64 - 12).clamp(0, 255) as u8);
                }
            }
        }
        (reference, texture)
    }

    /// The oracle's Rgba8Unorm textures: 40x33, three levels, two layers.
    fn oracle_pair(core: &CoreRef) -> (Operand, Operand) {
        let (mut reference, mut texture) = (Vec::new(), Vec::new());
        for level in 0..3 {
            let (mut r, mut t) = (Vec::new(), Vec::new());
            for layer in 0..2 {
                let (a, b) = oracle_level(level, layer);
                r.extend(a);
                t.extend(b);
            }
            reference.push(r);
            texture.push(t);
        }
        (upload(core, &file(F::Rgba8Unorm, 40, 33, 2, reference)), upload(core, &file(F::Rgba8Unorm, 40, 33, 2, texture)))
    }

    /// scikit-image 0.26.0's PSNR and SSIM of the oracle's pair, a row for
    /// each level, layer and channel in that order: gaussian_weights=True,
    /// sigma=1.5, use_sample_covariance=False, data_range=1.
    const SKIMAGE: [(f64, Option<f64>); 24] = [
        (30.94081979717409, Some(0.9634966264184766)),
        (30.89979344479888, Some(0.9625533831882912)),
        (31.145185218801608, Some(0.9630507637034453)),
        (30.85699325998383, Some(0.9619815903080738)),
        (31.0658887291947, Some(0.9643916822537327)),
        (31.068605819231475, Some(0.9651398639407948)),
        (31.06472478221262, Some(0.9645263296649127)),
        (30.88239952717748, Some(0.9623646868115077)),
        (31.219712286824407, Some(0.9519499365162946)),
        (31.177902627513852, Some(0.9627593357081013)),
        (31.08983490434443, Some(0.9701584939305542)),
        (31.371875051570633, Some(0.9871647089723681)),
        (31.078850614377966, Some(0.9556278052551548)),
        (30.8688820128392, Some(0.9545731934101854)),
        (30.401630082991538, Some(0.958482704424965)),
        (31.213911933547042, Some(0.982830328163606)),
        (30.101356871455035, None),
        (31.032805785723696, None),
        (30.218284374956728, None),
        (30.82600905305762, None),
        (30.750930345344795, None),
        (30.289982432650543, None),
        (31.43298745659374, None),
        (31.361582353304293, None),
    ];

    /// numpy 2.5.4's min, max, mean and std of the oracle's reference, a
    /// row for each level, layer and channel in that order.
    const NUMPY: [(f64, f64, f64, f64); 24] = [
        (0.0, 0.9725490196078431, 0.4780659536541889, 0.23271080240134584),
        (0.0, 0.9725490196078431, 0.5036601307189543, 0.23135376957669482),
        (0.00392156862745098, 0.9803921568627451, 0.5018062982768866, 0.24170813079957623),
        (0.0, 0.9764705882352941, 0.4894711824123588, 0.24778879269887202),
        (0.0, 0.9725490196078431, 0.47854426619132495, 0.23170939622873293),
        (0.00784313725490196, 0.9803921568627451, 0.5033184789067143, 0.2310212843892041),
        (0.00392156862745098, 0.9803921568627451, 0.5025282234105763, 0.24190459971844017),
        (0.0, 0.9803921568627451, 0.48968508615567446, 0.24774630103626102),
        (0.03529411764705882, 0.8745098039215686, 0.4426225490196078, 0.1759235103193528),
        (0.1568627450980392, 0.9607843137254902, 0.5800367647058824, 0.17566132141198781),
        (0.011764705882352941, 0.9764705882352941, 0.5746813725490196, 0.25490586229103185),
        (0.0, 0.9803921568627451, 0.46984068627450987, 0.29147516961792236),
        (0.0, 0.8705882352941177, 0.44281862745098033, 0.17498858746496432),
        (0.1607843137254902, 0.9686274509803922, 0.5795955882352941, 0.17649643014424413),
        (0.0196078431372549, 0.9725490196078431, 0.5745710784313726, 0.2544840014069215),
        (0.00392156862745098, 0.9686274509803922, 0.4711764705882352, 0.29165165557021505),
        (0.00392156862745098, 0.5058823529411764, 0.265, 0.1155062990205206),
        (0.16862745098039217, 0.6509803921568628, 0.4224509803921569, 0.11501629692459599),
        (0.32941176470588235, 0.8392156862745098, 0.5769117647058823, 0.11589710557698431),
        (0.17647058823529413, 0.9568627450980393, 0.7239705882352941, 0.12903315643445554),
        (0.0196078431372549, 0.5215686274509804, 0.25985294117647056, 0.11791431042106859),
        (0.1843137254901961, 0.6745098039215687, 0.41808823529411765, 0.11478126189194435),
        (0.34901960784313724, 0.8, 0.5782843137254903, 0.11268445726399537),
        (0.19607843137254902, 0.9607843137254902, 0.7245098039215687, 0.12366239340742437),
    ];

    #[test]
    fn reads_each_layout_as_values() {
        assert_eq!(unit8(&[0, 255, 51, 0], 2), 0.2);
        assert_eq!(bgra8(&[10, 20, 51, 40], 0), 0.2);
        assert_eq!(bgra8(&[51, 20, 30, 40], 2), 0.2);
        assert_eq!(signed8(&[0x80], 0), -1.0);
        assert_eq!(signed8(&[0x81], 0), -1.0);
        assert_eq!(signed8(&[127], 0), 1.0);
        assert_eq!(unit16(&u16::MAX.to_le_bytes(), 0), 1.0);
        assert_eq!(signed16(&i16::MIN.to_le_bytes(), 0), -1.0);
        assert_eq!(unit32(&u32::MAX.to_le_bytes(), 0), 1.0);
        assert_eq!(signed32(&i32::MIN.to_le_bytes(), 0), -1.0);
        assert_eq!(unit64(&u64::MAX.to_le_bytes(), 0), 1.0);
        assert_eq!(half(&[0, 0, 0x00, 0x38], 1), 0.5);
        assert_eq!(single(&[0, 0, 0, 0, 0, 0, 0x80, 0x3E], 1), 0.25);
        let packed = (1023u32 | (341 << 10) | (1 << 30)).to_le_bytes();
        assert_eq!([0, 1, 2, 3].map(|c| rgb10a2(&packed, c)), [1.0, 1.0 / 3.0, 0.0, 1.0 / 3.0]);
        // 1.0 is exponent 15 with no mantissa in 11 bits and in 10.
        let one = (960u32 | (960 << 11) | (480 << 22)).to_le_bytes();
        assert_eq!([0, 1, 2].map(|c| rg11b10(&one, c)), [1.0, 1.0, 1.0]);
        assert_eq!(small_float(1, 6), 2f64.powi(-20));
        assert_eq!(small_float(31 << 6, 6), f64::INFINITY);
        assert!(small_float((31 << 6) | 1, 6).is_nan());
        // A shared exponent of 16 makes each unit 2^-8.
        let shared = (256u32 | (128 << 9) | (64 << 18) | (16 << 27)).to_le_bytes();
        assert_eq!([0, 1, 2].map(|c| rgb9e5(&shared, c)), [1.0, 0.5, 0.25]);
        let depth: Vec<u8> = 0.25f32.to_le_bytes().into_iter().chain([51]).collect();
        assert_eq!([0, 1].map(|c| depth_stencil(&depth, c)), [0.25, 0.2]);
        for (format, texel, value, span) in [
            (F::R8Uint, vec![51u8], 0.2, Span::Unit),
            (F::Rg8Sint, vec![0x80, 0], -1.0, Span::Signed),
            (F::Rgb10a2Uint, (1023u32).to_le_bytes().to_vec(), 1.0, Span::Unit),
            (F::Rgba16Uint, vec![0xFF, 0xFF, 0, 0, 0, 0, 0, 0], 1.0, Span::Unit),
            (F::R32Sint, i32::MIN.to_le_bytes().to_vec(), -1.0, Span::Signed),
            (F::Rgba8UnormSrgb, vec![51, 0, 0, 0], 0.2, Span::Unit),
            (F::R16Float, vec![0x00, 0x38], 0.5, Span::Float),
        ] {
            let (unpack, got) = reading(format).expect("read");
            assert_eq!((unpack(&texel, 0), got), (value, span), "{format:?}");
        }
        for format in [F::Depth24Plus, F::Stencil8, F::Bc7RgbaUnorm, F::NV12] {
            assert!(reading(format).is_none(), "{format:?}");
        }
    }

    #[test]
    fn peaks_are_one_two_or_the_references_largest_float() {
        assert_eq!(Span::Unit.peak(5.0), 1.0);
        assert_eq!(Span::Signed.peak(5.0), 2.0);
        assert_eq!(Span::Float.peak(8.0), 8.0);
        assert_eq!(Span::Float.peak(0.0), 1.0);
        assert_eq!(Span::Float.peak(-3.0), 1.0);
        assert_eq!(Span::Float.peak(f64::NAN), 1.0);
        assert_eq!(Span::Float.peak(f64::NEG_INFINITY), 1.0);
    }

    #[test]
    fn brightness_doubles_the_error_each_twelfth() {
        assert_eq!(brightness(0.0), 0.0);
        assert_eq!(brightness(2f64.powi(-13)), 0.0);
        assert_eq!(brightness(2f64.powi(-12)), 0.0);
        assert_eq!(brightness(2f64.powi(-6)), 0.5);
        assert_eq!(brightness(0.5), 11.0 / 12.0);
        assert_eq!(brightness(1.0), 1.0);
        assert_eq!(brightness(4.0), 1.0);
        assert_eq!(brightness(f64::INFINITY), 1.0);
        assert_eq!(brightness(f64::NAN), 1.0);
        assert_eq!(shade(0.5), 128);
        assert_eq!(channel_list(1), "red");
        assert_eq!(channel_list(2), "red and green");
        assert_eq!(channel_list(4), "red, green, blue and alpha");
    }

    #[test]
    fn window_weights_are_a_normalized_gaussian_of_sigma_one_and_a_half() {
        let weights = window_weights();
        assert!((weights.iter().sum::<f64>() - 1.0).abs() < 1e-15);
        assert!(weights.iter().eq(weights.iter().rev()));
        // Each step out from the middle multiplies by exp(-(2k+1)/4.5).
        assert!((weights[6] / weights[5] - (-1.0f64 / 4.5).exp()).abs() < 1e-15);
        assert!((weights[10] / weights[9] - (-9.0f64 / 4.5).exp()).abs() < 1e-15);
    }

    /// The compensated sum keeps what a plain one rounds away, and an
    /// infinite or NaN value still makes the sum infinite or NaN.
    #[test]
    fn sums_carry_the_bits_each_addition_rounds_away() {
        let total = |values: &[f64]| {
            let mut sum = Sum::default();
            for &v in values {
                sum.add(v);
            }
            sum.total()
        };
        assert_eq!(total(&[1e16, 1.0, -1e16]), 1.0);
        assert_eq!(total(&[0.1; 10]), 1.0);
        let ramp: Vec<f64> = (0..256).map(|v| f64::from(v) / 255.0).collect();
        assert_eq!(total(&ramp) / 256.0, 0.5);
        assert_eq!(total(&[1.0, f64::INFINITY, 2.0]), f64::INFINITY);
        assert!(total(&[1.0, f64::NAN]).is_nan());
        assert!(total(&[f64::INFINITY, f64::NEG_INFINITY]).is_nan());
    }

    /// Two R64Uint values a double cannot tell apart differ by one in
    /// their integers.
    #[test]
    fn r64uint_differences_are_taken_in_integers() {
        let texels = |values: &[u64]| Texels {
            dimension: wgpu::TextureDimension::D2,
            width: values.len() as u32,
            height: 1,
            depth: 1,
            layers: 1,
            cube: false,
            unpack: unit64,
            size: 8,
            channels: 1,
            first: 0,
            span: Span::Unit,
            wide: true,
            levels: vec![values.iter().flat_map(|v| v.to_le_bytes()).collect()],
        };
        let (a, b) = (texels(&[u64::MAX, 7]), texels(&[u64::MAX - 1, 7]));
        assert_eq!(a.value(a.slice(0, 0), 0, 0), b.value(b.slice(0, 0), 0, 0));
        assert_eq!(difference_at(&a, a.slice(0, 0), &b, b.slice(0, 0), 0, 0), 1.0 / u64::MAX as f64);
        assert_eq!(difference_at(&a, a.slice(0, 0), &b, b.slice(0, 0), 1, 0), 0.0);
        let (mse, _) = squared_error(&a, a.slice(0, 0), &b, b.slice(0, 0), 0, 2);
        assert!(mse > 0.0);
    }

    /// The PSNR and SSIM of each level, layer and channel match
    /// scikit-image's on the same texels: PSNR, added on the CPU in
    /// doubles, to 1e-9 dB, and SSIM, from the GPU's single precision, to
    /// 1e-5; the levels under 11 texels on a side have none.
    #[test]
    fn matches_scikit_image() {
        let Some(core) = core() else { return };
        let (reference, texture) = oracle_pair(&core);
        let rows = compared(&reference, &texture, None, &mut Said::default()).expect("compared");
        assert_eq!(rows.len(), SKIMAGE.len());
        for (index, (row, (psnr, ssim))) in rows.iter().zip(SKIMAGE).enumerate() {
            assert_eq!((row.level, row.layer, row.channel), ((index / 8) as u32, (index / 4 % 2) as u32, TextureChannel::ALL[index % 4]));
            assert!((row.p_s_n_r - psnr).abs() < 1e-9, "row {index}: PSNR {} against {psnr}", row.p_s_n_r);
            assert!((10.0 * (1.0 / row.m_s_e).log10() - psnr).abs() < 1e-9, "row {index}: MSE {}", row.m_s_e);
            match (row.s_s_i_m, ssim) {
                (Some(got), Some(want)) => assert!((got - want).abs() < 1e-5, "row {index}: SSIM {got} against {want}"),
                (None, None) => {}
                (got, want) => panic!("row {index}: SSIM {got:?} against {want:?}"),
            }
        }
        // A texture against itself matches exactly.
        for row in compared(&reference, &reference, None, &mut Said::default()).expect("compared") {
            assert_eq!(row.m_s_e, 0.0);
            assert_eq!(row.p_s_n_r, f64::INFINITY);
            if let Some(ssim) = row.s_s_i_m {
                assert!((ssim - 1.0).abs() < 1e-6, "SSIM {ssim}");
            }
        }
    }

    /// A float texture's peak is the reference channel's largest value,
    /// here 8, for PSNR and for SSIM's data range, as scikit-image computes
    /// them with data_range=8.
    #[test]
    fn a_float_textures_peak_is_the_reference_channels_largest_value() {
        let Some(core) = core() else { return };
        let (mut reference, mut texture) = (Vec::new(), Vec::new());
        for y in 0..24u64 {
            for x in 0..24u64 {
                let k = ((x * 5 + y * 3) % 17) as f64;
                let m = (hashed(7, x, y, 0) % 101) as f64;
                reference.push((k / 2.0) as f32);
                texture.push((k / 2.0 + (m - 50.0) / 400.0) as f32);
            }
        }
        let a = upload(&core, &file(F::R32Float, 24, 24, 1, vec![floats(&reference)]));
        let b = upload(&core, &file(F::R32Float, 24, 24, 1, vec![floats(&texture)]));
        let rows = compared(&a, &b, None, &mut Said::default()).expect("compared");
        assert_eq!(rows.len(), 1);
        assert!((rows[0].p_s_n_r - 40.60948129021267).abs() < 1e-9, "PSNR {}", rows[0].p_s_n_r);
        assert!((rows[0].m_s_e - 0.0055620110195766116).abs() < 1e-15, "MSE {}", rows[0].m_s_e);
        let ssim = rows[0].s_s_i_m.expect("an SSIM");
        assert!((ssim - 0.999544773863833).abs() < 1e-5, "SSIM {ssim}");
    }

    /// Minimum, maximum, mean and standard deviation match numpy's on the
    /// oracle's reference, and 256 bins from 0 to 1 count each 8-bit value
    /// in a bin of its own.
    #[test]
    fn measures_as_numpy_does() {
        let Some(core) = core() else { return };
        let (reference, _) = oracle_pair(&core);
        let rows = measured(&reference, DEFAULT_BINS, &DEFAULT_RANGE, None, &mut Said::default()).expect("measured");
        assert_eq!(rows.len(), NUMPY.len());
        for (index, (row, (minimum, maximum, mean, std))) in rows.iter().zip(NUMPY).enumerate() {
            assert_eq!((row.level, row.layer, row.channel), ((index / 8) as u32, (index / 4 % 2) as u32, TextureChannel::ALL[index % 4]));
            assert_eq!((row.minimum, row.maximum), (minimum, maximum), "row {index}");
            assert!((row.average - mean).abs() < 1e-12, "row {index}: mean {} against {mean}", row.average);
            assert!((row.standard_deviation - std).abs() < 1e-12, "row {index}: std {} against {std}", row.standard_deviation);
        }
        let (texels, _) = oracle_level(0, 0);
        let mut counts = vec![0u64; 256];
        for texel in texels.as_chunks::<4>().0 {
            counts[usize::from(texel[0])] += 1;
        }
        assert_eq!(rows[0].histogram, counts);
        assert_eq!(rows[0].histogram.iter().sum::<u64>(), 40 * 33);
    }

    /// Values below the range count in the first bin and values above it in
    /// the last; NaN counts in none and makes the statistics NaN.
    #[test]
    fn counts_values_outside_the_range_in_the_end_bins() {
        let Some(core) = core() else { return };
        let values = [0.0, 0.25, 0.5, 0.75, 1.0, -5.0, 7.0, f32::NAN];
        let texture = upload(&core, &file(F::R32Float, 8, 1, 1, vec![floats(&values)]));
        let rows = measured(&texture, 4, &[0.0, 1.0], None, &mut Said::default()).expect("measured");
        assert_eq!(rows[0].histogram, [2, 1, 1, 3]);
        assert!(rows[0].minimum.is_nan() && rows[0].maximum.is_nan() && rows[0].average.is_nan() && rows[0].standard_deviation.is_nan());
        let texture = upload(&core, &file(F::R32Float, 2, 1, 1, vec![floats(&[0.25, 0.75])]));
        let rows = measured(&texture, 2, &[0.0, 1.0], None, &mut Said::default()).expect("measured");
        assert_eq!((rows[0].minimum, rows[0].maximum, rows[0].average, rows[0].standard_deviation), (0.25, 0.75, 0.5, 0.25));
        assert_eq!(rows[0].histogram, [1, 1]);
        for (bins, range) in [(0, vec![0.0, 1.0]), (4, vec![1.0, 0.0]), (4, vec![0.0]), (4, vec![0.0, f64::INFINITY])] {
            assert_eq!(refused(measured(&texture, bins, &range, None, &mut Said::default())).error_id, "WgpuMeasure");
        }
    }

    /// An integer texture reads through the norm format with its bits, so
    /// R8Uint compares with R8Unorm on one scale.
    #[test]
    fn reads_integer_formats_through_their_norm_twins() {
        let Some(core) = core() else { return };
        let bytes: Vec<u8> = (0..16 * 16).map(|i| (hashed(3, i, 0, 0) % 256) as u8).collect();
        let unorm = upload(&core, &file(F::R8Unorm, 16, 16, 1, vec![bytes.clone()]));
        let uint = upload(&core, &file(F::R8Uint, 16, 16, 1, vec![bytes]));
        let rows = compared(&unorm, &uint, None, &mut Said::default()).expect("compared");
        assert_eq!((rows.len(), rows[0].m_s_e, rows[0].p_s_n_r), (1, 0.0, f64::INFINITY));
        assert!((rows[0].s_s_i_m.expect("an SSIM") - 1.0).abs() < 1e-6);
        let signed: Vec<u8> = [i16::MIN, i16::MAX, 0, -16384].iter().flat_map(|v| v.to_le_bytes()).collect();
        let rows = measured(&upload(&core, &file(F::Rg16Sint, 2, 1, 1, vec![signed])), 4, &[-1.0, 1.0], None, &mut Said::default()).expect("measured");
        assert_eq!((rows[0].minimum, rows[0].maximum), (-1.0, 0.0));
        assert_eq!((rows[1].minimum, rows[1].maximum), (-16384.0 / 32767.0, 1.0));
    }

    /// Depth reads from 0 to 1, copied from Depth32Float and Depth16Unorm
    /// and read in a shader from Depth24Plus; stencil reads over 255; a
    /// depth-stencil texture has the depth as Red and the stencil as Green.
    #[test]
    fn reads_depth_and_stencil_from_0_to_1() {
        let Some(core) = core() else { return };
        let usage = wgpu::TextureUsages::COPY_SRC | wgpu::TextureUsages::TEXTURE_BINDING;
        let value = |operand: &Operand| {
            let rows = measured(operand, 1, &[0.0, 1.0], None, &mut Said::default()).expect("measured");
            rows.iter().map(|row| (row.channel, row.minimum, row.maximum)).collect::<Vec<_>>()
        };
        let float = cleared(&core, F::Depth32Float, 1, usage, [0.0; 4], 0.25, 0);
        assert_eq!(value(&float), [(TextureChannel::Red, 0.25, 0.25)]);
        let sixteen = value(&cleared(&core, F::Depth16Unorm, 1, usage, [0.0; 4], 0.5, 0));
        assert!((sixteen[0].1 - 0.5).abs() <= 1.0 / 65535.0 && sixteen[0].1 == sixteen[0].2, "{sixteen:?}");
        let plus = cleared(&core, F::Depth24Plus, 1, wgpu::TextureUsages::TEXTURE_BINDING, [0.0; 4], 0.25, 0);
        let read = value(&plus);
        assert!((read[0].1 - 0.25).abs() < 1e-6 && read[0].1 == read[0].2, "{read:?}");
        let rows = compared(&float, &plus, None, &mut Said::default()).expect("compared");
        assert!(rows[0].m_s_e < 1e-12, "MSE {}", rows[0].m_s_e);
        let both = value(&cleared(&core, F::Depth24PlusStencil8, 1, usage, [0.0; 4], 0.75, 51));
        assert_eq!(both.len(), 2);
        assert!((both[0].1 - 0.75).abs() < 1e-6, "{both:?}");
        assert_eq!(both[1], (TextureChannel::Green, 0.2, 0.2));
        assert_eq!(value(&cleared(&core, F::Stencil8, 1, wgpu::TextureUsages::COPY_SRC, [0.0; 4], 0.0, 102)), [(TextureChannel::Red, 0.4, 0.4)]);
        let blind = cleared(&core, F::Depth24Plus, 1, wgpu::TextureUsages::empty(), [0.0; 4], 0.25, 0);
        let e = refused(measured(&blind, 1, &[0.0, 1.0], None, &mut Said::default()));
        assert_eq!(e.error_id, "WgpuTextureUsage");
        assert_eq!(e.message, "the texture needs TextureBinding among its usages for a shader to read its depth, as no copy reads the depth of Depth24Plus");
    }

    /// A multisampled texture reads as a resolve would store it: cleared
    /// to one color, it matches a single-sample texture cleared to the
    /// same, an sRGB one and an integer one included; depth and stencil
    /// average too.
    #[test]
    fn reads_a_multisampled_texture_as_a_resolve_would_store_it() {
        let Some(core) = core() else { return };
        for (format, color) in [(F::Rgba8Unorm, [0.2, 0.4, 0.6, 1.0]), (F::Rgba8UnormSrgb, [0.2, 0.4, 0.6, 1.0]), (F::Rgba8Uint, [7.0, 200.0, 0.0, 255.0])] {
            let many = cleared(&core, format, 4, wgpu::TextureUsages::TEXTURE_BINDING, color, 0.0, 0);
            let one = cleared(&core, format, 1, wgpu::TextureUsages::COPY_SRC, color, 0.0, 0);
            let mut said = Said::default();
            let rows = compared(&one, &many, None, &mut said).expect("compared");
            assert_eq!(rows.len(), 4);
            for row in &rows {
                assert_eq!(row.m_s_e, 0.0, "{format:?} {:?}", row.channel);
            }
            assert_eq!(said.verbose, [format!("the 4 samples of each texel of the texture were averaged in a shader and rounded to {}", format_name(format))]);
        }
        let integer = cleared(&core, F::Rgba8Uint, 4, wgpu::TextureUsages::TEXTURE_BINDING, [7.0, 200.0, 0.0, 255.0], 0.0, 0);
        let rows = measured(&integer, 1, &[0.0, 1.0], None, &mut Said::default()).expect("measured");
        assert_eq!(rows.iter().map(|r| r.minimum).collect::<Vec<_>>(), [7.0 / 255.0, 200.0 / 255.0, 0.0, 1.0]);
        let depth = cleared(&core, F::Depth32Float, 4, wgpu::TextureUsages::TEXTURE_BINDING, [0.0; 4], 0.25, 0);
        let rows = measured(&depth, 1, &[0.0, 1.0], None, &mut Said::default()).expect("measured");
        assert_eq!((rows[0].minimum, rows[0].maximum), (0.25, 0.25));
        let both = cleared(&core, F::Depth24PlusStencil8, 4, wgpu::TextureUsages::TEXTURE_BINDING, [0.0; 4], 0.75, 51);
        let rows = measured(&both, 1, &[0.0, 1.0], None, &mut Said::default()).expect("measured");
        assert!((rows[0].minimum - 0.75).abs() < 1e-6 && rows[0].minimum == rows[0].maximum, "{:?}", rows[0]);
        assert_eq!((rows[1].channel, rows[1].minimum, rows[1].maximum), (TextureChannel::Green, 0.2, 0.2));
        let blind = cleared(&core, F::Rgba8Unorm, 4, wgpu::TextureUsages::empty(), [0.2, 0.4, 0.6, 1.0], 0.0, 0);
        assert_eq!(refused(measured(&blind, 1, &[0.0, 1.0], None, &mut Said::default())).error_id, "WgpuTextureUsage");
    }

    /// Each depth slice of a 3D level is a layer of its own.
    #[test]
    fn a_3d_textures_slices_are_its_layers() {
        let Some(core) = core() else { return };
        let volume = TextureFile {
            dimension: wgpu::TextureDimension::D3,
            width: 4,
            height: 4,
            depth: 4,
            layers: 1,
            cube: false,
            format: F::R8Unorm,
            levels: vec![(0..64).collect(), (0..8).map(|v| v * 30).collect()],
        };
        let texture = upload(&core, &volume);
        let rows = compared(&texture, &texture, None, &mut Said::default()).expect("compared");
        assert_eq!(rows.iter().map(|r| (r.level, r.layer)).collect::<Vec<_>>(), [(0, 0), (0, 1), (0, 2), (0, 3), (1, 0), (1, 1)]);
        let rows = measured(&texture, 4, &[0.0, 1.0], None, &mut Said::default()).expect("measured");
        assert_eq!((rows[1].minimum, rows[1].maximum), (16.0 / 255.0, 31.0 / 255.0));
        assert_eq!((rows[5].minimum, rows[5].maximum), (120.0 / 255.0, 210.0 / 255.0));
    }

    /// Textures of other dimensions, sizes or layer counts are refused,
    /// naming what differs; so is one that cannot be read back; their
    /// formats and mip levels may differ, and the rows cover the channels
    /// and levels both have.
    #[test]
    fn compares_what_both_textures_hold_and_refuses_other_shapes() {
        let Some(core) = core() else { return };
        let rgba = upload(&core, &file(F::Rgba8Unorm, 16, 16, 1, vec![vec![9; 1024], vec![9; 256]]));
        let rg = upload(&core, &file(F::Rg8Unorm, 16, 16, 1, vec![vec![9; 512]]));
        let rows = compared(&rgba, &rg, None, &mut Said::default()).expect("compared");
        assert_eq!(rows.iter().map(|r| (r.level, r.channel)).collect::<Vec<_>>(), [(0, TextureChannel::Red), (0, TextureChannel::Green)]);
        let e = refused(difference(&rgba, &rg, Some(TextureChannel::Blue), None, &mut Said::default()));
        assert_eq!((e.error_id.as_str(), e.message.as_str()), ("WgpuCompare", "the textures do not both store blue: they share red and green"));
        let short = upload(&core, &file(F::Rgba8Unorm, 16, 8, 1, vec![vec![9; 512]]));
        let e = refused(compared(&rgba, &short, None, &mut Said::default()));
        assert_eq!((e.error_id.as_str(), e.message.as_str()), ("WgpuCompare", "the reference is 16x16 and the texture 16x8: textures compare only at the same width and height"));
        let layered = upload(&core, &file(F::Rgba8Unorm, 16, 16, 2, vec![vec![9; 2048]]));
        let e = refused(compared(&rgba, &layered, None, &mut Said::default()));
        assert_eq!(e.message, "the reference has 1 layer and the texture 2 layers: textures compare only with the same number of layers");
        let volume = upload(&core, &TextureFile { dimension: wgpu::TextureDimension::D3, width: 16, height: 16, depth: 1, layers: 1, cube: false, format: F::Rgba8Unorm, levels: vec![vec![9; 1024]] });
        let e = refused(compared(&rgba, &volume, None, &mut Said::default()));
        assert_eq!(e.message, "the reference is a 2D texture and the texture a 3D one: textures compare only with the same dimension");
        let sealed = core
            .scoped("WgpuTexture", "the test texture cannot be made", || {
                core.device.create_texture(&wgpu::TextureDescriptor {
                    label: Some("sealed"),
                    size: wgpu::Extent3d { width: 16, height: 16, depth_or_array_layers: 1 },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format: F::Rgba8Unorm,
                    usage: wgpu::TextureUsages::TEXTURE_BINDING,
                    view_formats: &[],
                })
            })
            .expect("made");
        let sealed = (core.clone(), Source { texture: sealed, cube: false, label: "sealed".to_string() });
        let e = refused(compared(&sealed, &rgba, None, &mut Said::default()));
        assert_eq!((e.error_id.as_str(), e.message.as_str()), ("WgpuTextureUsage", "the reference needs CopySrc among its usages to be read back from the GPU"));
    }

    /// The picture shows each error on the fixed log scale: red, green and
    /// blue in their own colors with alpha opaque, or one channel as gray;
    /// it has the textures' layers and shared levels.
    #[test]
    fn draws_the_error_on_a_log_scale() {
        let Some(core) = core() else { return };
        let errors = [0.0, 2f32.powi(-13), 2f32.powi(-12), 2f32.powi(-6), 1.0, 4.0, f32::NAN, 0.5];
        let reference = upload(&core, &file(F::Rgba32Float, 8, 1, 1, vec![floats(&[0.0; 32])]));
        let texture: Vec<f32> = errors.iter().flat_map(|&e| [e, 2f32.powi(-6), 0.0, 1.0]).collect();
        let texture = upload(&core, &file(F::Rgba32Float, 8, 1, 1, vec![floats(&texture)]));
        let picture = difference(&reference, &texture, None, None, &mut Said::default()).expect("drawn");
        assert_eq!((picture.format.as_str(), picture.label.as_str(), picture.width, picture.height), ("Rgba8Unorm", "8x1 Rgba8Unorm", 8, 1));
        assert_eq!(picture.notes.len(), 1);
        let texels = texture::read_texels(&core, &picture.texture, 0, 0, wgpu::TextureAspect::All, None).expect("read");
        let red: Vec<u8> = texels.as_chunks::<4>().0.iter().map(|t| t[0]).collect();
        assert_eq!(red, [0, 0, 0, 128, 255, 255, 255, 234]);
        assert!(texels.as_chunks::<4>().0.iter().all(|t| t[1..] == [128, 0, 255]), "{texels:?}");
        let gray = difference(&reference, &texture, Some(TextureChannel::Alpha), None, &mut Said::default()).expect("drawn");
        let texels = texture::read_texels(&core, &gray.texture, 0, 0, wgpu::TextureAspect::All, None).expect("read");
        assert!(texels.as_chunks::<4>().0.iter().all(|t| *t == [255, 255, 255, 255]), "{texels:?}");
        let (a, b) = oracle_pair(&core);
        let picture = difference(&a, &b, None, None, &mut Said::default()).expect("drawn");
        assert_eq!((picture.mip_level_count, picture.depth_or_array_layers), (3, 2));
    }

    /// A 16 by 16 texture of planar `format`, its luma's bytes `luma` and
    /// its chroma's `chroma`.
    fn planar_texture(core: &CoreRef, format: F, luma: &[u8], chroma: &[u8]) -> Operand {
        let texture = core
            .scoped("WgpuTexture", "the test texture cannot be made", || {
                core.device.create_texture(&wgpu::TextureDescriptor {
                    label: Some("planar"),
                    size: wgpu::Extent3d { width: 16, height: 16, depth_or_array_layers: 1 },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format,
                    usage: wgpu::TextureUsages::COPY_SRC | wgpu::TextureUsages::COPY_DST,
                    view_formats: &[],
                })
            })
            .expect("made");
        texture::write_texels(core, &texture, 0, 0, wgpu::TextureAspect::Plane0, luma).expect("luma written");
        texture::write_texels(core, &texture, 0, 0, wgpu::TextureAspect::Plane1, chroma).expect("chroma written");
        (core.clone(), Source { texture, cube: false, label: "planar".to_string() })
    }

    /// NV12 and P010 compare and measure their planes as channels: luma as
    /// Red at the texture's size and chroma as Green and Blue at half its
    /// width and height; the picture spreads each chroma texel's error over
    /// the 2 by 2 texels sharing it; and a planar texture is refused against
    /// one that is not.
    #[test]
    fn planar_textures_read_their_planes_as_channels() {
        let Some(core) = core() else { return };
        if !core.features.contains(wgpu::Features::TEXTURE_FORMAT_NV12 | wgpu::Features::TEXTURE_FORMAT_P010) {
            eprintln!("skipped: {} has no NV12 or no P010 textures", core.info.name);
            return;
        }
        let pairs = |u: u8, v: u8, n: usize| [u, v].repeat(n);
        // The texture's luma is 1 above the reference's, and its chroma
        // differs only in the Blue of chroma texel (3, 2), by 2.
        let mut changed = pairs(50, 200, 64);
        changed[(2 * 8 + 3) * 2 + 1] = 202;
        let reference = planar_texture(&core, F::NV12, &[100; 256], &pairs(50, 200, 64));
        let texture = planar_texture(&core, F::NV12, &[101; 256], &changed);
        let mut said = Said::default();
        let rows = compared(&reference, &texture, None, &mut said).expect("compared");
        assert!(said.verbose.iter().any(|note| note == "the reference is NV12, whose luma was read as red at 16x16 and whose chroma as green and blue at 8x8"), "{:?}", said.verbose);
        let (red, green, blue) = (TextureChannel::Red, TextureChannel::Green, TextureChannel::Blue);
        // SSIM needs 11 texels on a side, which the 8 by 8 chroma has not.
        assert_eq!(rows.iter().map(|r| (r.channel, r.s_s_i_m.is_some())).collect::<Vec<_>>(), [(red, true), (green, false), (blue, false)]);
        let step = |n: f64| (n / 255.0).powi(2);
        for (row, mse) in rows.iter().zip([step(1.0), 0.0, step(2.0) / 64.0]) {
            assert!((row.m_s_e - mse).abs() <= mse * 1e-12, "{row:?}");
        }
        let measured_rows = measured(&reference, 4, &[0.0, 1.0], None, &mut Said::default()).expect("measured");
        assert_eq!(measured_rows.len(), 3);
        for (row, (channel, value, texels)) in measured_rows.iter().zip([(red, 100.0, 256), (green, 50.0, 64), (blue, 200.0, 64)]) {
            assert_eq!((row.channel, row.histogram.iter().sum::<u64>()), (channel, texels));
            assert!((row.average - value / 255.0).abs() < 1e-12, "{row:?}");
        }
        let mut said = Said::default();
        let picture = difference(&reference, &texture, None, None, &mut said).expect("drawn");
        assert!(said.verbose.iter().any(|note| note == "each chroma texel's error fills the 2 by 2 texels that share it"), "{:?}", said.verbose);
        // An error of 1/255 is shade 85 on the log scale, and 2/255 is 106.
        let texels = texture::read_texels(&core, &picture.texture, 0, 0, wgpu::TextureAspect::All, None).expect("read");
        for (at, texel) in texels.as_chunks::<4>().0.iter().enumerate() {
            let (x, y) = (at % 16, at / 16);
            let shown = if (x / 2, y / 2) == (3, 2) { 106 } else { 0 };
            assert_eq!(*texel, [85, 0, shown, 255], "texel ({x}, {y})");
        }
        let gray = difference(&reference, &texture, Some(blue), None, &mut Said::default()).expect("drawn");
        let texels = texture::read_texels(&core, &gray.texture, 0, 0, wgpu::TextureAspect::All, None).expect("read");
        let lit: Vec<usize> = texels.as_chunks::<4>().0.iter().enumerate().filter(|(_, t)| t[0] > 0).map(|(at, _)| at).collect();
        assert_eq!(lit, [4 * 16 + 6, 4 * 16 + 7, 5 * 16 + 6, 5 * 16 + 7]);
        let e = refused(difference(&reference, &texture, Some(TextureChannel::Alpha), None, &mut Said::default()));
        assert_eq!(e.message, "the textures do not both store alpha: they share red, green and blue");
        let rgba = upload(&core, &file(F::Rgba8Unorm, 16, 16, 1, vec![vec![9; 1024]]));
        let e = refused(compared(&reference, &rgba, None, &mut Said::default()));
        assert_eq!(
            (e.error_id.as_str(), e.message.as_str()),
            ("WgpuCompare", "the reference is NV12 and the texture Rgba8Unorm: textures compare only when both keep their texels in planes or neither does")
        );
        let wide = |values: [u16; 2], n: usize| values.iter().flat_map(|v| v.to_le_bytes()).collect::<Vec<u8>>().repeat(n);
        let p010 = planar_texture(&core, F::P010, &wide([0x8000, 0x8000], 128), &wide([0xFFFF, 0], 64));
        let rows = measured(&p010, 1, &[0.0, 1.0], None, &mut Said::default()).expect("measured");
        assert_eq!(rows.iter().map(|r| (r.channel, r.histogram[0])).collect::<Vec<_>>(), [(red, 256), (green, 64), (blue, 64)]);
        for (row, value) in rows.iter().zip([32768.0 / 65535.0, 1.0, 0.0]) {
            assert!((row.average - value).abs() < 1e-12, "{row:?}");
        }
        let e = refused(image_io::read_level(&core, &p010.1.texture, 0, None));
        assert_eq!(
            (e.error_id.as_str(), e.message.as_str()),
            ("WgpuAspect", "P010 keeps its texels in planes, which are read one at a time: Read-WgpuTexture reads its luma with -Aspect Plane0 and its chroma with -Aspect Plane1")
        );
    }
}
