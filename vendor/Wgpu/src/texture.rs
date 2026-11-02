//! Textures, texture views and samplers, image files, and every texture
//! format wgpu has.

use std::path::Path;

use exhaust::Exhaust;
use pwrs::prelude::*;

use crate::alpha;
use crate::binding::Resource;
use crate::buffer;
use crate::color::{self, Color, ColorSpace, ToneMapCurve};
use crate::compare::{self, TextureChannel, TextureComparison, TextureMeasurement};
use crate::convert::{MipEdge, MipFilter, MipOptions};
use crate::cube::{self, CubeLayout, Harmonics, RoughnessMapping};
use crate::device::{core_for, CoreRef, Device};
use crate::element::{self, ElementType};
use crate::error::{for_each_file, input_files, invalid};
use crate::filter::{self, EdgeOperator, FilterEdge};
use crate::image_decode::{Reading, RenderingIntent};
use crate::image_io::{self, parse_target, Settings};
use crate::names;
use crate::normal::{HeightChannel, HeightEdge, HeightToNormal, NormalAlpha, NormalKernel, NormalMips};
use crate::shader::Reflected;
use crate::tasks::{self, Interrupt};
use crate::texfile::Said;
use crate::transform::{self, Anchor, Asked, Crop, FlipDirection, PowerOfTwo, ResizeFilter, Transform};

/// What a texture may be used for.
#[psenum(name = "Wgpu.TextureUsage")]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum TextureUsage {
    /// The source of a copy.
    CopySrc,
    /// The destination of a copy or a write.
    CopyDst,
    /// Sampled or loaded in a shader.
    #[default]
    TextureBinding,
    /// A storage texture in a shader.
    StorageBinding,
    /// A color or depth attachment of a render pass.
    RenderAttachment,
    /// Atomic operations on a storage texture.
    StorageAtomic,
    /// An attachment whose contents never leave the tile memory of the pass.
    TransientAttachment,
}

const TEXTURE_USAGES: [(TextureUsage, wgpu::TextureUsages); 7] = [
    (TextureUsage::CopySrc, wgpu::TextureUsages::COPY_SRC),
    (TextureUsage::CopyDst, wgpu::TextureUsages::COPY_DST),
    (TextureUsage::TextureBinding, wgpu::TextureUsages::TEXTURE_BINDING),
    (TextureUsage::StorageBinding, wgpu::TextureUsages::STORAGE_BINDING),
    (TextureUsage::RenderAttachment, wgpu::TextureUsages::RENDER_ATTACHMENT),
    (TextureUsage::StorageAtomic, wgpu::TextureUsages::STORAGE_ATOMIC),
    (TextureUsage::TransientAttachment, wgpu::TextureUsages::TRANSIENT_ATTACHMENT),
];

pub(crate) fn usage_flags(list: &[TextureUsage]) -> wgpu::TextureUsages {
    TEXTURE_USAGES.iter().filter(|(u, _)| list.contains(u)).fold(wgpu::TextureUsages::empty(), |all, (_, f)| all | *f)
}

pub(crate) fn usage_list(flags: wgpu::TextureUsages) -> Vec<TextureUsage> {
    TEXTURE_USAGES.iter().filter(|(_, f)| flags.contains(*f)).map(|(u, _)| *u).collect()
}

/// A texture's shape.
#[psenum(name = "Wgpu.TextureDimension")]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum TextureDimension {
    /// One dimension: a row of texels.
    D1,
    /// Two dimensions, with array layers.
    #[default]
    D2,
    /// Three dimensions.
    D3,
}

impl TextureDimension {
    fn wgpu(self) -> wgpu::TextureDimension {
        match self {
            TextureDimension::D1 => wgpu::TextureDimension::D1,
            TextureDimension::D2 => wgpu::TextureDimension::D2,
            TextureDimension::D3 => wgpu::TextureDimension::D3,
        }
    }

    pub(crate) fn from_wgpu(d: wgpu::TextureDimension) -> TextureDimension {
        match d {
            wgpu::TextureDimension::D1 => TextureDimension::D1,
            wgpu::TextureDimension::D2 => TextureDimension::D2,
            wgpu::TextureDimension::D3 => TextureDimension::D3,
        }
    }
}

/// Which part of a depth-stencil texture, or which plane of a planar one,
/// an operation touches.
#[psenum(name = "Wgpu.TextureAspect")]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum TextureAspect {
    /// Every part: the color of a color texture.
    #[default]
    All,
    /// The depth of a depth-stencil texture.
    DepthOnly,
    /// The stencil of a depth-stencil texture.
    StencilOnly,
    /// The first plane of an NV12 or P010 texture: luma, one channel at
    /// the texture's size.
    Plane0,
    /// The second plane of an NV12 or P010 texture: chroma, two channels
    /// at half the texture's width and height.
    Plane1,
}

impl TextureAspect {
    fn wgpu(self) -> wgpu::TextureAspect {
        match self {
            TextureAspect::All => wgpu::TextureAspect::All,
            TextureAspect::DepthOnly => wgpu::TextureAspect::DepthOnly,
            TextureAspect::StencilOnly => wgpu::TextureAspect::StencilOnly,
            TextureAspect::Plane0 => wgpu::TextureAspect::Plane0,
            TextureAspect::Plane1 => wgpu::TextureAspect::Plane1,
        }
    }
}

/// Where a block-compressed texture is decoded when it is saved as an
/// image file.
#[psenum(name = "Wgpu.Decoder")]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Decoder {
    /// On the CPU, by the module's decoders, which give the same pixels on
    /// every device.
    #[default]
    Cpu,
    /// On the GPU, by its own decoder, as a shader reads the texels.
    Gpu,
}

/// How a sampler filters between texels.
#[psenum(name = "Wgpu.FilterMode")]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum FilterMode {
    /// The nearest texel.
    #[default]
    Nearest,
    /// A blend of the nearest texels.
    Linear,
}

/// What a sampler reads past a texture's edge.
#[psenum(name = "Wgpu.AddressMode")]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum AddressMode {
    /// The edge texel.
    #[default]
    ClampToEdge,
    /// The texture again.
    Repeat,
    /// The texture mirrored.
    MirrorRepeat,
    /// The border color (feature AddressModeClampToBorder).
    ClampToBorder,
}

impl AddressMode {
    fn wgpu(self) -> wgpu::AddressMode {
        match self {
            AddressMode::ClampToEdge => wgpu::AddressMode::ClampToEdge,
            AddressMode::Repeat => wgpu::AddressMode::Repeat,
            AddressMode::MirrorRepeat => wgpu::AddressMode::MirrorRepeat,
            AddressMode::ClampToBorder => wgpu::AddressMode::ClampToBorder,
        }
    }
}

/// A comparison, for comparison samplers and depth and stencil tests.
#[psenum(name = "Wgpu.CompareFunction")]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum CompareFunction {
    /// Never passes.
    Never,
    /// Passes when the new value is less.
    #[default]
    Less,
    /// Passes when the values are equal.
    Equal,
    /// Passes when the new value is less or equal.
    LessEqual,
    /// Passes when the new value is greater.
    Greater,
    /// Passes when the values differ.
    NotEqual,
    /// Passes when the new value is greater or equal.
    GreaterEqual,
    /// Always passes.
    Always,
}

impl CompareFunction {
    pub(crate) fn wgpu(self) -> wgpu::CompareFunction {
        match self {
            CompareFunction::Never => wgpu::CompareFunction::Never,
            CompareFunction::Less => wgpu::CompareFunction::Less,
            CompareFunction::Equal => wgpu::CompareFunction::Equal,
            CompareFunction::LessEqual => wgpu::CompareFunction::LessEqual,
            CompareFunction::Greater => wgpu::CompareFunction::Greater,
            CompareFunction::NotEqual => wgpu::CompareFunction::NotEqual,
            CompareFunction::GreaterEqual => wgpu::CompareFunction::GreaterEqual,
            CompareFunction::Always => wgpu::CompareFunction::Always,
        }
    }
}

/// The color a ClampToBorder sampler reads past the edge.
#[psenum(name = "Wgpu.BorderColor")]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum BorderColor {
    /// Black with zero alpha.
    #[default]
    TransparentBlack,
    /// Black.
    OpaqueBlack,
    /// White.
    OpaqueWhite,
    /// Zero in every channel, alpha included, for any format.
    Zero,
}

/// A format's name: wgpu's own variant name, with ASTC formats spelled
/// `Astc4x4Unorm`, `Astc4x4UnormSrgb` and `Astc4x4Hdr`.
pub(crate) fn format_name(format: wgpu::TextureFormat) -> String {
    match format {
        wgpu::TextureFormat::Astc { block, channel } => {
            let block = format!("{block:?}");
            format!("Astc{}{channel:?}", block.trim_start_matches('B'))
        }
        other => format!("{other:?}"),
    }
}

/// Every texture format wgpu has.
pub(crate) fn all_formats() -> Vec<wgpu::TextureFormat> {
    wgpu::TextureFormat::exhaust().collect()
}

/// The format a name names, ignoring case, `_` and `-`.
pub(crate) fn parse_format(name: &str) -> PsResult<wgpu::TextureFormat> {
    let key = names::fold(name);
    all_formats()
        .into_iter()
        .find(|f| names::fold(&format_name(*f)) == key)
        .ok_or_else(|| invalid("WgpuUnknownFormat", format!("wgpu has no texture format named {name}; Get-WgpuTextureFormat lists them")))
}

/// The number type a format's texels read as; raw bytes for compressed,
/// packed and multi-planar formats.
pub(crate) fn texel_element(format: wgpu::TextureFormat, aspect: wgpu::TextureAspect) -> ElementType {
    use wgpu::TextureFormat as F;
    match format.aspect_specific_format(aspect).unwrap_or(format) {
        F::R8Snorm | F::Rg8Snorm | F::Rgba8Snorm | F::R8Sint | F::Rg8Sint | F::Rgba8Sint => ElementType::Int8,
        F::R16Uint | F::R16Unorm | F::Rg16Uint | F::Rg16Unorm | F::Rgba16Uint | F::Rgba16Unorm | F::Depth16Unorm => ElementType::UInt16,
        F::R16Sint | F::R16Snorm | F::Rg16Sint | F::Rg16Snorm | F::Rgba16Sint | F::Rgba16Snorm => ElementType::Int16,
        F::R16Float | F::Rg16Float | F::Rgba16Float => ElementType::Float16,
        F::R32Uint | F::Rg32Uint | F::Rgba32Uint => ElementType::UInt32,
        F::R32Sint | F::Rg32Sint | F::Rgba32Sint => ElementType::Int32,
        F::R32Float | F::Rg32Float | F::Rgba32Float | F::Depth32Float => ElementType::Float32,
        F::R64Uint => ElementType::UInt64,
        _ => ElementType::UInt8,
    }
}

/// A texture held in Rust, on one device.
#[psclass(name = "Wgpu.Texture", mode = proxy, native_bytes = Texture::native_bytes, columns = ["Label", "Width", "Height", "Format"])]
pub struct Texture {
    /// The texture's label.
    pub label: String,
    /// Width in texels.
    pub width: u32,
    /// Height in texels.
    pub height: u32,
    /// Depth of a 3D texture, or the number of array layers of a 2D one.
    pub depth_or_array_layers: u32,
    /// Mip levels.
    pub mip_level_count: u32,
    /// Samples per texel.
    pub sample_count: u32,
    /// D1, D2 or D3.
    pub dimension: TextureDimension,
    /// The format's name.
    pub format: String,
    /// What it may be used for.
    pub usage: Vec<TextureUsage>,
    /// The type texels read and write as.
    pub element_type: ElementType,
    /// The adapter of the device it lives on.
    pub adapter: String,
    /// Whether it is a cube map: square layers, six to a cube.
    pub is_cube: bool,
    /// What making it involved: how a file was read, decoded, converted or
    /// encoded, and what Basis Universal said.
    pub notes: Vec<String>,
    /// An irradiance map's irradiance as spherical harmonics, which
    /// ConvertTo-WgpuIrradianceMap -Harmonics gives it; empty on any other
    /// texture.
    pub harmonics: Option<Harmonics>,
    #[psfield(skip)]
    pub(crate) texture: wgpu::Texture,
    #[psfield(skip)]
    pub(crate) core: CoreRef,
}

impl Texture {
    fn native_bytes(&self) -> usize {
        let block = self.texture.format().block_copy_size(None).unwrap_or(4) as usize;
        self.width as usize * self.height as usize * self.depth_or_array_layers as usize * block
    }

    pub(crate) fn wrap(core: &CoreRef, texture: wgpu::Texture, label: String) -> Texture {
        let format = texture.format();
        Texture {
            label,
            width: texture.width(),
            height: texture.height(),
            depth_or_array_layers: texture.depth_or_array_layers(),
            mip_level_count: texture.mip_level_count(),
            sample_count: texture.sample_count(),
            dimension: TextureDimension::from_wgpu(texture.dimension()),
            format: format_name(format),
            usage: usage_list(texture.usage()),
            element_type: texel_element(format, wgpu::TextureAspect::All),
            adapter: core.info.name.clone(),
            is_cube: false,
            notes: Vec::new(),
            harmonics: None,
            texture,
            core: core.clone(),
        }
    }

    /// The texture as the methods that convert it read it.
    pub(crate) fn source(&self) -> image_io::Source {
        image_io::Source { texture: self.texture.clone(), cube: self.is_cube, label: self.label.clone() }
    }

    /// The dimension of a view of `count` layers from `base` when none is
    /// given: whole cubes of a cube map view as a cube, or as a cube array
    /// for more than one, and `None` leaves wgpu's default for the rest.
    fn cube_view(&self, base: u32, count: Option<u32>) -> Option<wgpu::TextureViewDimension> {
        let count = count.unwrap_or_else(|| self.depth_or_array_layers.saturating_sub(base));
        if !self.is_cube || !base.is_multiple_of(6) || count == 0 || !count.is_multiple_of(6) {
            return None;
        }
        Some(if count == 6 { wgpu::TextureViewDimension::Cube } else { wgpu::TextureViewDimension::CubeArray })
    }

    /// The copy `transform` makes of the texture. A method has no verbose
    /// or warning stream, so what making it involved goes to its Notes.
    fn transformed(&self, transform: &Transform) -> PsResult<Texture> {
        image_io::transformed(&self.core, &self.source(), transform, MipOptions::default(), None, None, &mut Said::default())
    }

    /// The copy `transform` makes of the texture, made on a thread of its
    /// own and handed to `task`.
    fn transformed_async(&self, transform: Transform, task: PsTask<Texture>) -> PsResult<()> {
        let (core, source) = (self.core.clone(), self.source());
        tasks::spawn(task, move |stop| image_io::transformed(&core, &source, &transform, MipOptions::default(), None, Some(stop), &mut Said::default()), Ok)
    }
}

/// The resize Resize(width, height, filter) asks for: a side given as 0,
/// or omitted, keeps the aspect ratio.
fn resize_to(width: Option<u32>, height: Option<u32>, filter: Option<ResizeFilter>) -> PsResult<Transform> {
    let side = |v: Option<u32>| v.filter(|&v| v > 0);
    let (width, height) = (side(width), side(height));
    if width.is_none() && height.is_none() {
        return Err(invalid("WgpuResize", "Resize takes a width, a height or both; a side given as 0 keeps the aspect ratio"));
    }
    Ok(Transform { width, height, filter, ..Default::default() })
}

/// The padding Pad(width, height, anchor, color) asks for.
fn pad_to(width: u32, height: u32, anchor: Option<Anchor>, color: Option<Vec<f64>>) -> PsResult<Transform> {
    Ok(Transform { pad: Some((width, height)), anchor, pad_color: color.as_deref().map(transform::color_of).transpose()?, ..Default::default() })
}

/// The crop Crop(x, y, width, height) asks for.
fn crop_to(x: u32, y: u32, width: u32, height: u32) -> Transform {
    Transform { crop: Some(Crop::Region { x, y, width, height }), ..Default::default() }
}

/// A transform that changes only the texels' channels and colors.
fn recolored(color: Color) -> Transform {
    Transform { color, ..Default::default() }
}

/// The transform that runs the filters `asked` names.
fn filtered(asked: filter::Asked<'_>) -> PsResult<Transform> {
    Ok(Transform { filters: asked.filters()?, ..Default::default() })
}

/// The color key a method was given, which it needs.
fn key_given(key: &PsObject, tolerance: Option<f64>) -> PsResult<color::ColorKey> {
    match color::key_of(Some(key), tolerance)? {
        Some(key) => Ok(key),
        None => Err(invalid("WgpuColorKey", "ColorKey takes a color: red, green and blue from 0 to 1, or '#RRGGBB'")),
    }
}

/// The weights Grayscale(weights) asks for: Rec. 709's when omitted.
fn gray(weights: Option<&PsObject>) -> PsResult<color::Gray> {
    match weights.filter(|w| !w.is_null()) {
        Some(w) => color::weights_of(w),
        None => Ok(color::Gray::Rec709),
    }
}

/// The changes Adjust(brightness, contrast, gamma, exposure) asks for.
fn adjusted(brightness: Option<f64>, contrast: Option<f64>, gamma: Option<f64>, exposure: Option<f64>) -> PsResult<Color> {
    let color = Color {
        brightness: color::percent_of(brightness, "brightness")?,
        contrast: color::percent_of(contrast, "contrast")?,
        gamma: color::gamma_of(gamma)?,
        exposure: color::exposure_of(exposure)?,
        ..Default::default()
    };
    if color.is_empty() {
        return Err(invalid("WgpuAdjust", "Adjust takes a brightness, a contrast, a gamma or an exposure"));
    }
    Ok(color)
}

/// Refuses to record `texture` as a cube map unless its layers can be cube
/// faces: square 2D layers, six to a cube.
pub(crate) fn check_cube(texture: &wgpu::Texture) -> PsResult<()> {
    let layers = texture.depth_or_array_layers();
    if texture.dimension() != wgpu::TextureDimension::D2 || texture.width() != texture.height() || !layers.is_multiple_of(6) {
        let shape = match (texture.dimension(), layers) {
            (wgpu::TextureDimension::D2, 1) => "1 layer".to_string(),
            (wgpu::TextureDimension::D2, n) => format!("{n} layers"),
            (wgpu::TextureDimension::D1, _) => "one dimension".to_string(),
            (wgpu::TextureDimension::D3, n) => format!("a depth of {n}"),
        };
        return Err(invalid("WgpuCube", format!("cube faces are square 2D layers, six to a cube; a {}x{} texture with {shape} is not that", texture.width(), texture.height())));
    }
    Ok(())
}

/// The extent of mip level `mip`, and the layer range one copy covers:
/// every depth slice of a 3D texture, one layer of an array.
fn copy_extent(texture: &wgpu::Texture, mip: u32) -> wgpu::Extent3d {
    let mut size = texture.size().mip_level_size(mip, texture.dimension());
    if texture.dimension() != wgpu::TextureDimension::D3 {
        size.depth_or_array_layers = 1;
    }
    size
}

/// The bytes one texel block of `aspect` of `format` takes in a copy to or
/// from a buffer. Such a copy is refused for a combined depth-stencil format
/// with no aspect named, for the depth of Depth24Plus and
/// Depth24PlusStencil8, whose layout only the GPU knows, for a planar
/// format with no plane named, and for a plane of a format that has none.
fn copy_block(format: wgpu::TextureFormat, aspect: wgpu::TextureAspect) -> PsResult<u32> {
    if aspect.to_plane().is_some() && !format.is_multi_planar_format() {
        return Err(invalid("WgpuAspect", format!("{} has no planes: -Aspect Plane0 and Plane1 name the planes of NV12 and P010", format_name(format))));
    }
    if let Some(block) = format.block_copy_size(Some(aspect)) {
        return Ok(block);
    }
    Err(if format.is_multi_planar_format() {
        invalid("WgpuAspect", format!("{} keeps its texels in planes: give -Aspect Plane0 for its luma or Plane1 for its chroma", format_name(format)))
    } else if aspect == wgpu::TextureAspect::All && format.block_copy_size(Some(wgpu::TextureAspect::DepthOnly)).is_some() {
        invalid("WgpuAspect", format!("{} needs -Aspect DepthOnly or StencilOnly for its texels to be read or written", format_name(format)))
    } else if format.has_stencil_aspect() {
        invalid("WgpuAspect", format!("only the stencil of {} can be read or written, with -Aspect StencilOnly: its depth has no layout outside the GPU", format_name(format)))
    } else {
        invalid("WgpuAspect", format!("the depth of {} has no layout outside the GPU, so it cannot be read or written", format_name(format)))
    })
}

/// The texels a copy of `aspect` of mip level `mip` covers: the level's
/// physical size, and for a plane, that size divided by the plane's
/// subsampling. `aspect` has passed copy_block for the texture's format.
fn copy_size(texture: &wgpu::Texture, mip: u32, aspect: wgpu::TextureAspect) -> wgpu::Extent3d {
    let format = texture.format();
    let physical = copy_extent(texture, mip).physical_size(format);
    match aspect.to_plane() {
        Some(plane) if format.is_multi_planar_format() => {
            let (across, down) = format.subsampling_factors(Some(plane));
            wgpu::Extent3d { width: physical.width / across, height: physical.height / down, ..physical }
        }
        _ => physical,
    }
}

/// The texels of mip level `mip` of layer `layer`, rows packed with no
/// padding.
pub(crate) fn read_texels(core: &CoreRef, texture: &wgpu::Texture, mip: u32, layer: u32, aspect: wgpu::TextureAspect, stop: Option<&dyn Interrupt>) -> PsResult<Vec<u8>> {
    let format = texture.format();
    let block = copy_block(format, aspect)?;
    let (bw, bh) = format.block_dimensions();
    let physical = copy_size(texture, mip, aspect);
    let row = physical.width / bw * block;
    let rows = physical.height / bh;
    let padded = row.next_multiple_of(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT);
    let total = padded as u64 * rows as u64 * physical.depth_or_array_layers as u64;
    let staging = buffer::create_raw(core, total, wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::COPY_SRC, "Wgpu texture read", None)?;
    let commands = core.scoped("WgpuTextureRead", "the texture cannot be read back", || {
        let mut encoder = core.device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("Wgpu texture read") });
        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo { texture, mip_level: mip, origin: wgpu::Origin3d { x: 0, y: 0, z: layer }, aspect },
            wgpu::TexelCopyBufferInfo { buffer: &staging, layout: wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(padded), rows_per_image: Some(rows) } },
            physical,
        );
        encoder.finish()
    })?;
    core.submit_and_wait(vec![commands], stop)?;
    let mut out = Vec::with_capacity(row as usize * rows as usize * physical.depth_or_array_layers as usize);
    buffer::read_mapped(core, &staging, 0, total, stop)?.with(|raw| {
        for chunk in raw.chunks_exact(padded as usize) {
            out.extend_from_slice(&chunk[..row as usize]);
        }
    })?;
    Ok(out)
}

/// Writes rows of texels, packed with no padding, into mip level `mip`
/// of layer `layer`.
pub(crate) fn write_texels(core: &CoreRef, texture: &wgpu::Texture, mip: u32, layer: u32, aspect: wgpu::TextureAspect, bytes: &[u8]) -> PsResult<()> {
    let format = texture.format();
    let block = copy_block(format, aspect)?;
    let (bw, bh) = format.block_dimensions();
    let physical = copy_size(texture, mip, aspect);
    let row = physical.width / bw * block;
    let rows = physical.height / bh;
    let expected = row as u64 * rows as u64 * physical.depth_or_array_layers as u64;
    if bytes.len() as u64 != expected {
        return Err(invalid("WgpuTexelCount", format!("mip {mip} of a {} texture holds {expected} bytes; {} were given", format_name(format), bytes.len())));
    }
    core.scoped("WgpuTextureWrite", "the texture cannot be written", || {
        core.queue.write_texture(
            wgpu::TexelCopyTextureInfo { texture, mip_level: mip, origin: wgpu::Origin3d { x: 0, y: 0, z: layer }, aspect },
            bytes,
            wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(row), rows_per_image: Some(rows) },
            physical,
        )
    })
}

/// `usages` less storage for a depth or stencil format, which WebGPU never
/// makes a storage texture. Direct3D 12 reports storage for Depth16Unorm
/// and Depth32Float from the color formats their views use, and cannot
/// create a depth texture a shader writes: WARP loses the device.
fn without_depth_storage(format: wgpu::TextureFormat, usages: wgpu::TextureUsages) -> wgpu::TextureUsages {
    if format.is_depth_stencil_format() { usages - wgpu::TextureUsages::STORAGE_BINDING } else { usages }
}

/// What `core`'s device can do with `format`, as wgpu-core decides it: the
/// adapter's own report when the device has
/// TextureAdapterSpecificFormatFeatures or the adapter lacks WebGPU's
/// format support, and otherwise what WebGPU guarantees with the device's
/// features.
pub(crate) fn format_features(core: &CoreRef, format: wgpu::TextureFormat) -> wgpu::TextureFormatFeatures {
    let specific = core.features.contains(wgpu::Features::TEXTURE_ADAPTER_SPECIFIC_FORMAT_FEATURES);
    let webgpu = core.adapter.get_downlevel_capabilities().flags.contains(wgpu::DownlevelFlags::WEBGPU_TEXTURE_FORMAT_SUPPORT);
    if specific || !webgpu { core.adapter.get_texture_format_features(format) } else { format.guaranteed_format_features(core.features) }
}

/// The usages a texture of `format` may take on `core`'s device: for a
/// planar format, those every plane's format allows, as wgpu-core checks
/// a planar texture's usages.
pub(crate) fn allowed_usages(core: &CoreRef, format: wgpu::TextureFormat) -> wgpu::TextureUsages {
    match format.planes() {
        Some(planes) => (0..planes)
            .filter_map(|plane| wgpu::TextureAspect::from_plane(plane).and_then(|aspect| format.aspect_specific_format(aspect)))
            .fold(wgpu::TextureUsages::all(), |usages, plane| usages & allowed_usages(core, plane)),
        None => without_depth_storage(format, format_features(core, format).allowed_usages),
    }
}

/// Every usage the format allows on the device, of the ones a texture made
/// from data or a file takes.
pub(crate) fn usable(core: &CoreRef, format: wgpu::TextureFormat) -> wgpu::TextureUsages {
    let wanted = wgpu::TextureUsages::COPY_SRC
        | wgpu::TextureUsages::COPY_DST
        | wgpu::TextureUsages::TEXTURE_BINDING
        | wgpu::TextureUsages::STORAGE_BINDING
        | wgpu::TextureUsages::RENDER_ATTACHMENT;
    allowed_usages(core, format) & wanted
}

/// What New-WgpuTexture and `Device.CreateTexture` take.
pub(crate) struct TextureSpec {
    pub(crate) width: u32,
    pub(crate) height: u32,
    pub(crate) depth_or_layers: u32,
    pub(crate) format: wgpu::TextureFormat,
    pub(crate) mip_levels: u32,
    pub(crate) sample_count: u32,
    pub(crate) dimension: TextureDimension,
    pub(crate) usage: Option<Vec<TextureUsage>>,
    pub(crate) view_formats: Vec<wgpu::TextureFormat>,
    pub(crate) label: Option<String>,
}

pub(crate) fn create(core: &CoreRef, spec: &TextureSpec) -> PsResult<Texture> {
    let usage = match &spec.usage {
        Some(list) => usage_flags(list),
        None => usable(core, spec.format),
    };
    create_with_usage(core, spec, usage)
}

/// A texture as `spec` describes it, with `usage` in place of the usages
/// it lists.
pub(crate) fn create_with_usage(core: &CoreRef, spec: &TextureSpec, usage: wgpu::TextureUsages) -> PsResult<Texture> {
    if spec.format.is_depth_stencil_format() && usage.contains(wgpu::TextureUsages::STORAGE_BINDING) {
        return Err(invalid("WgpuTextureUsage", format!("{} is a depth or stencil format, which cannot be a storage binding", format_name(spec.format))));
    }
    let label = match &spec.label {
        Some(l) => l.clone(),
        None => format!("{}x{} {}", spec.width, spec.height, format_name(spec.format)),
    };
    let texture = core.scoped("WgpuTexture", "the texture cannot be created", || {
        core.device.create_texture(&wgpu::TextureDescriptor {
            label: Some(&label),
            size: wgpu::Extent3d { width: spec.width, height: spec.height, depth_or_array_layers: spec.depth_or_layers },
            mip_level_count: spec.mip_levels,
            sample_count: spec.sample_count,
            dimension: spec.dimension.wgpu(),
            format: spec.format,
            usage,
            view_formats: &spec.view_formats,
        })
    })?;
    Ok(Texture::wrap(core, texture, label))
}

fn view_dimension(name: &str) -> PsResult<wgpu::TextureViewDimension> {
    Ok(match names::fold(name).as_str() {
        "d1" | "1d" => wgpu::TextureViewDimension::D1,
        "d2" | "2d" => wgpu::TextureViewDimension::D2,
        "d2array" | "2darray" => wgpu::TextureViewDimension::D2Array,
        "cube" => wgpu::TextureViewDimension::Cube,
        "cubearray" => wgpu::TextureViewDimension::CubeArray,
        "d3" | "3d" => wgpu::TextureViewDimension::D3,
        _ => return Err(invalid("WgpuViewDimension", format!("{name} is not a view dimension; give D1, D2, D2Array, Cube, CubeArray or D3"))),
    })
}

#[psmethods]
impl Texture {
    /// A view of the texture: another compatible format, a dimension
    /// (D1, D2, D2Array, Cube, CubeArray, D3), a range of mip levels and
    /// array layers, and an aspect. Everything omitted is the whole texture,
    /// and a cube map's whole cubes view as a cube, or as a cube array for
    /// more than one, when no dimension is given.
    // clippy::too_many_arguments: the arguments are the fields of wgpu's
    // TextureViewDescriptor a view sets, one PowerShell method argument each.
    #[allow(clippy::too_many_arguments)]
    pub fn create_view(
        &self,
        format: Option<String>,
        dimension: Option<String>,
        base_mip_level: Option<u32>,
        mip_level_count: Option<u32>,
        base_array_layer: Option<u32>,
        array_layer_count: Option<u32>,
        aspect: Option<TextureAspect>,
    ) -> PsResult<TextureView> {
        let format = match &format {
            Some(name) => Some(parse_format(name)?),
            None => None,
        };
        let dimension = match &dimension {
            Some(name) => Some(view_dimension(name)?),
            None => self.cube_view(base_array_layer.unwrap_or(0), array_layer_count),
        };
        // A view in another format takes only the texture's usages that
        // format allows, as an sRGB view of a texture that may be a storage
        // binding cannot be one.
        let usage = format.map(|f| self.texture.usage() & allowed_usages(&self.core, f));
        let view = self.core.scoped("WgpuTextureView", "the view cannot be created", || {
            self.texture.create_view(&wgpu::TextureViewDescriptor {
                label: Some(&self.label),
                format,
                dimension,
                usage,
                aspect: aspect.map_or(wgpu::TextureAspect::All, TextureAspect::wgpu),
                base_mip_level: base_mip_level.unwrap_or(0),
                mip_level_count,
                base_array_layer: base_array_layer.unwrap_or(0),
                array_layer_count,
            })
        })?;
        let own = self.texture.format();
        let read_as = format.unwrap_or_else(|| own.aspect_specific_format(aspect.map_or(wgpu::TextureAspect::All, TextureAspect::wgpu)).unwrap_or(own));
        Ok(TextureView { label: self.label.clone(), format: format_name(read_as), view, core: self.core.clone() })
    }

    /// The texels of mip level `mipLevel` of array layer `layer` (both 0
    /// when omitted), rows packed, as the format's number type.
    pub fn read(&self, mip_level: Option<u32>, layer: Option<u32>, aspect: Option<TextureAspect>) -> PsResult<PsObject> {
        let aspect = aspect.map_or(wgpu::TextureAspect::All, TextureAspect::wgpu);
        let bytes = read_texels(&self.core, &self.texture, mip_level.unwrap_or(0), layer.unwrap_or(0), aspect, None)?;
        element::to_clr_array(texel_element(self.texture.format(), aspect), &bytes)
    }

    /// Read on a thread of its own: returns at once with a task whose
    /// result is the texels.
    pub fn read_async(&self, mip_level: Option<u32>, layer: Option<u32>, aspect: Option<TextureAspect>, task: PsTask<PsObject>) -> PsResult<()> {
        let aspect = aspect.map_or(wgpu::TextureAspect::All, TextureAspect::wgpu);
        let ty = texel_element(self.texture.format(), aspect);
        let (core, texture, mip, layer) = (self.core.clone(), self.texture.clone(), mip_level.unwrap_or(0), layer.unwrap_or(0));
        tasks::spawn(task, move |stop| read_texels(&core, &texture, mip, layer, aspect, Some(stop)), move |bytes| element::to_clr_array(ty, &bytes))
    }

    /// Writes texels into mip level `mipLevel` of array layer `layer`
    /// (both 0 when omitted), converted to the format's number type.
    pub fn write(&self, data: PsObject, mip_level: Option<u32>, layer: Option<u32>, aspect: Option<TextureAspect>) -> PsResult<()> {
        let aspect = aspect.map_or(wgpu::TextureAspect::All, TextureAspect::wgpu);
        let ty = texel_element(self.texture.format(), aspect);
        element::with_host_bytes(&data, Some(ty), |_, bytes| write_texels(&self.core, &self.texture, mip_level.unwrap_or(0), layer.unwrap_or(0), aspect, bytes))
    }

    /// Saves the texture to a file: a .dds or .ktx2 file whole, with every
    /// mip level, layer and cube face, and any other file as an image of
    /// mip level 0 of layer 0, in the format its extension names. A
    /// block-compressed texture is decoded for an image file by `decoder`,
    /// Cpu or Gpu, on the CPU when omitted. A relative path is relative to
    /// the process's working directory, as for any .NET method;
    /// Export-WgpuImage resolves PowerShell paths.
    pub fn save(&self, path: String, decoder: Option<Decoder>) -> PsResult<()> {
        // A method has no verbose or warning stream for what saving says.
        image_io::save(&self.core, &self.texture, self.is_cube, Path::new(&path), decoder, None, &mut Said::default())
    }

    /// Save on a thread of its own: returns at once with a task that ends
    /// when the file is written.
    pub fn save_async(&self, path: String, decoder: Option<Decoder>, task: PsTask<()>) -> PsResult<()> {
        let (core, texture, cube) = (self.core.clone(), self.texture.clone(), self.is_cube);
        tasks::spawn(task, move |stop| image_io::save(&core, &texture, cube, Path::new(&path), decoder, Some(stop), &mut Said::default()), Ok)
    }

    /// Without `levels`, renders the texture's own mip levels 1 and on and
    /// returns nothing. With `levels`, returns a new texture with that many
    /// mip levels, 0 for the full chain down to 1x1: the texture's own
    /// levels kept and the rest rendered. `filter` makes each level: Box
    /// when omitted, from the level above; Triangle, Kaiser, Mitchell and
    /// Lanczos from level 0; Min and Max, the smallest or largest value of
    /// each channel among the texels under it. `edge` is what Triangle,
    /// Kaiser, Mitchell and Lanczos read past an edge of the image: Clamp
    /// when omitted, Wrap or Mirror, or a sampler's ClampToEdge, Repeat or
    /// MirrorRepeat. `coverage` scales each rendered level's alpha so the
    /// share of its texels with alpha at or above it, the alpha an alpha
    /// test passes at, stays level 0's: above 0 and at most 1. `normals`
    /// keeps a normal map's normals unit length: Renormalize scales each
    /// texel's normal back to length 1, and SlopeSpace filters their
    /// slopes. Levels are filtered as floating-point colors, in linear
    /// light for an sRGB format, and rounded once; a BC texture's levels
    /// are encoded on the GPU.
    pub fn generate_mips(&self, levels: Option<u32>, filter: Option<MipFilter>, edge: Option<PsObject>, coverage: Option<f64>, normals: Option<NormalMips>) -> PsResult<Option<Texture>> {
        let mips = MipOptions::given(filter, edge.as_ref().map(edge_of).transpose()?).with_coverage(coverage)?.with_normals(normals, true);
        let source = self.source();
        match levels {
            None => image_io::fill_mips(&self.core, &source, mips, None).map(|()| None),
            Some(n) => image_io::with_mips(&self.core, &source, n, mips, None).map(Some),
        }
    }

    /// GenerateMips on a thread of its own: returns at once with a task
    /// whose result is the new texture, or `$null` without `levels`.
    #[allow(clippy::too_many_arguments)]
    pub fn generate_mips_async(
        &self,
        levels: Option<u32>,
        filter: Option<MipFilter>,
        edge: Option<PsObject>,
        coverage: Option<f64>,
        normals: Option<NormalMips>,
        task: PsTask<Option<Texture>>,
    ) -> PsResult<()> {
        let mips = MipOptions::given(filter, edge.as_ref().map(edge_of).transpose()?).with_coverage(coverage)?.with_normals(normals, true);
        let (core, source) = (self.core.clone(), self.source());
        tasks::spawn(
            task,
            move |stop| match levels {
                None => image_io::fill_mips(&core, &source, mips, Some(stop)).map(|()| None),
                Some(n) => image_io::with_mips(&core, &source, n, mips, Some(stop)).map(Some),
            },
            Ok,
        )
    }

    /// A new texture with every mip level and layer encoded in `format`: a
    /// BC format, encoded on the GPU, or a Basis Universal format, such as
    /// BasisEtc1s or BasisUastc4x4, which Basis Universal encodes and the
    /// module transcodes to the best format the device holds. `quality`, 1
    /// to 100, and `effort`, 0 to 10, are Import-WgpuImage's -Quality and
    /// -Effort. The new texture's Notes say what the encoding involved,
    /// Basis Universal's warnings among it.
    pub fn compress(&self, format: String, quality: Option<u32>, effort: Option<u32>) -> PsResult<Texture> {
        image_io::compressed(&self.core, &self.source(), &format, quality, effort, None)
    }

    /// Compress on a thread of its own: returns at once with a task whose
    /// result is the new texture.
    pub fn compress_async(&self, format: String, quality: Option<u32>, effort: Option<u32>, task: PsTask<Texture>) -> PsResult<()> {
        let (core, source) = (self.core.clone(), self.source());
        tasks::spawn(task, move |stop| image_io::compressed(&core, &source, &format, quality, effort, Some(stop)), Ok)
    }

    /// A new texture of `width` by `height` texels, resized with `filter`:
    /// Box where a side shrinks and Bicubic where it grows when omitted. A
    /// side given as 0, or omitted, keeps the aspect ratio. The new texture
    /// keeps the format and as many mip levels as the new size's chain has,
    /// rendered again from level 0; its Notes say what was done.
    pub fn resize(&self, width: Option<u32>, height: Option<u32>, filter: Option<ResizeFilter>) -> PsResult<Texture> {
        self.transformed(&resize_to(width, height, filter)?)
    }

    /// Resize on a thread of its own: returns at once with a task whose
    /// result is the new texture.
    pub fn resize_async(&self, width: Option<u32>, height: Option<u32>, filter: Option<ResizeFilter>, task: PsTask<Texture>) -> PsResult<()> {
        self.transformed_async(resize_to(width, height, filter)?, task)
    }

    /// A new texture of the `width` by `height` texels whose top left is at
    /// (`x`, `y`).
    pub fn crop(&self, x: u32, y: u32, width: u32, height: u32) -> PsResult<Texture> {
        self.transformed(&crop_to(x, y, width, height))
    }

    /// Crop on a thread of its own: returns at once with a task whose
    /// result is the new texture.
    pub fn crop_async(&self, x: u32, y: u32, width: u32, height: u32, task: PsTask<Texture>) -> PsResult<()> {
        self.transformed_async(crop_to(x, y, width, height), task)
    }

    /// A new texture on a `width` by `height` canvas: the texture placed by
    /// `anchor`, Center when omitted, and the rest filled with `color`, red,
    /// green, blue and alpha from 0 to 1, transparent black when omitted.
    pub fn pad(&self, width: u32, height: u32, anchor: Option<Anchor>, color: Option<Vec<f64>>) -> PsResult<Texture> {
        self.transformed(&pad_to(width, height, anchor, color)?)
    }

    /// Pad on a thread of its own: returns at once with a task whose result
    /// is the new texture.
    pub fn pad_async(&self, width: u32, height: u32, anchor: Option<Anchor>, color: Option<Vec<f64>>, task: PsTask<Texture>) -> PsResult<()> {
        self.transformed_async(pad_to(width, height, anchor, color)?, task)
    }

    /// A new texture mirrored: Horizontal swaps left and right, and
    /// Vertical top and bottom.
    pub fn flip(&self, direction: FlipDirection) -> PsResult<Texture> {
        self.transformed(&Transform { flip: Some(direction), ..Default::default() })
    }

    /// Flip on a thread of its own: returns at once with a task whose
    /// result is the new texture.
    pub fn flip_async(&self, direction: FlipDirection, task: PsTask<Texture>) -> PsResult<()> {
        self.transformed_async(Transform { flip: Some(direction), ..Default::default() }, task)
    }

    /// A new texture turned `degrees` clockwise: 90, 180 or 270.
    pub fn rotate(&self, degrees: u32) -> PsResult<Texture> {
        self.transformed(&Transform { rotate: Some(degrees), ..Default::default() })
    }

    /// Rotate on a thread of its own: returns at once with a task whose
    /// result is the new texture.
    pub fn rotate_async(&self, degrees: u32, task: PsTask<Texture>) -> PsResult<()> {
        self.transformed_async(Transform { rotate: Some(degrees), ..Default::default() }, task)
    }

    /// A new texture with its channels rearranged by `mask`: four letters
    /// of r, g, b, a, 0 and 1, one for each channel, as 'bgra' or 'rrr1'.
    /// An sRGB texture's channels move as the values it stores.
    pub fn swizzle(&self, mask: String) -> PsResult<Texture> {
        self.transformed(&recolored(Color { swizzle: Some(color::swizzle_of(&mask)?), ..Default::default() }))
    }

    /// Swizzle on a thread of its own: returns at once with a task whose
    /// result is the new texture.
    pub fn swizzle_async(&self, mask: String, task: PsTask<Texture>) -> PsResult<()> {
        self.transformed_async(recolored(Color { swizzle: Some(color::swizzle_of(&mask)?), ..Default::default() }), task)
    }

    /// A new texture with red, green and blue multiplied by alpha, in
    /// linear light for an sRGB texture.
    pub fn premultiply(&self) -> PsResult<Texture> {
        self.transformed(&recolored(Color { premultiply: true, ..Default::default() }))
    }

    /// Premultiply on a thread of its own: returns at once with a task
    /// whose result is the new texture.
    pub fn premultiply_async(&self, task: PsTask<Texture>) -> PsResult<()> {
        self.transformed_async(recolored(Color { premultiply: true, ..Default::default() }), task)
    }

    /// A new texture with red, green and blue divided by alpha where alpha
    /// is above 0, in linear light for an sRGB texture.
    pub fn unpremultiply(&self) -> PsResult<Texture> {
        self.transformed(&recolored(Color { unpremultiply: true, ..Default::default() }))
    }

    /// Unpremultiply on a thread of its own: returns at once with a task
    /// whose result is the new texture.
    pub fn unpremultiply_async(&self, task: PsTask<Texture>) -> PsResult<()> {
        self.transformed_async(recolored(Color { unpremultiply: true, ..Default::default() }), task)
    }

    /// A new texture with its texels converted to `space`, Linear or Srgb,
    /// so their colors look the same: a format with an sRGB form switches
    /// to that form, and any other keeps its format with its values encoded
    /// or decoded.
    pub fn convert_color_space(&self, space: ColorSpace) -> PsResult<Texture> {
        self.transformed(&recolored(Color { space: Some(space), ..Default::default() }))
    }

    /// ConvertColorSpace on a thread of its own: returns at once with a
    /// task whose result is the new texture.
    pub fn convert_color_space_async(&self, space: ColorSpace, task: PsTask<Texture>) -> PsResult<()> {
        self.transformed_async(recolored(Color { space: Some(space), ..Default::default() }), task)
    }

    /// A new texture with its texels relabeled as `space`, Linear or Srgb,
    /// without changing them: the format switches to that form.
    pub fn assign_color_space(&self, space: ColorSpace) -> PsResult<Texture> {
        self.transformed(&recolored(Color { assign: Some(space), ..Default::default() }))
    }

    /// AssignColorSpace on a thread of its own: returns at once with a task
    /// whose result is the new texture.
    pub fn assign_color_space_async(&self, space: ColorSpace, task: PsTask<Texture>) -> PsResult<()> {
        self.transformed_async(recolored(Color { assign: Some(space), ..Default::default() }), task)
    }

    /// A new grayscale texture, weighed by `weights`: Rec709 in linear
    /// light when omitted, Rec601 on the stored values, or three numbers
    /// for red, green and blue in linear light.
    pub fn grayscale(&self, weights: Option<PsObject>) -> PsResult<Texture> {
        self.transformed(&recolored(Color { grayscale: Some(gray(weights.as_ref())?), ..Default::default() }))
    }

    /// Grayscale on a thread of its own: returns at once with a task whose
    /// result is the new texture.
    pub fn grayscale_async(&self, weights: Option<PsObject>, task: PsTask<Texture>) -> PsResult<()> {
        self.transformed_async(recolored(Color { grayscale: Some(gray(weights.as_ref())?), ..Default::default() }), task)
    }

    /// A new texture with red, green and blue turned into 1 minus their
    /// stored values, as ImageMagick's -negate; alpha is kept.
    pub fn invert(&self) -> PsResult<Texture> {
        self.transformed(&recolored(Color { invert: true, ..Default::default() }))
    }

    /// Invert on a thread of its own: returns at once with a task whose
    /// result is the new texture.
    pub fn invert_async(&self, task: PsTask<Texture>) -> PsResult<()> {
        self.transformed_async(recolored(Color { invert: true, ..Default::default() }), task)
    }

    /// A new texture with `brightness` and `contrast`, from -100 to 100 as
    /// ImageMagick's -brightness-contrast, and `gamma`, as its -gamma,
    /// applied to the stored values, and `exposure` in stops to linear
    /// values; each is left out when omitted.
    pub fn adjust(&self, brightness: Option<f64>, contrast: Option<f64>, gamma: Option<f64>, exposure: Option<f64>) -> PsResult<Texture> {
        self.transformed(&recolored(adjusted(brightness, contrast, gamma, exposure)?))
    }

    /// Adjust on a thread of its own: returns at once with a task whose
    /// result is the new texture.
    pub fn adjust_async(&self, brightness: Option<f64>, contrast: Option<f64>, gamma: Option<f64>, exposure: Option<f64>, task: PsTask<Texture>) -> PsResult<()> {
        self.transformed_async(recolored(adjusted(brightness, contrast, gamma, exposure)?), task)
    }

    /// A new texture with its HDR colors mapped into the range from 0 to 1
    /// by `curve`: Reinhard, AcesFilmic, AgX or Neutral.
    pub fn tone_map(&self, curve: ToneMapCurve) -> PsResult<Texture> {
        self.transformed(&recolored(Color { tone_map: Some(curve), ..Default::default() }))
    }

    /// ToneMap on a thread of its own: returns at once with a task whose
    /// result is the new texture.
    pub fn tone_map_async(&self, curve: ToneMapCurve, task: PsTask<Texture>) -> PsResult<()> {
        self.transformed_async(recolored(Color { tone_map: Some(curve), ..Default::default() }), task)
    }

    /// A new texture whose texels of the color `key` get alpha 0: red,
    /// green and blue from 0 to 1 as the texture stores them, rounded to
    /// its precision, or '#RRGGBB', each channel within `tolerance` (0, the
    /// same stored value, when omitted). Other texels keep their alpha.
    pub fn color_key(&self, key: PsObject, tolerance: Option<f64>) -> PsResult<Texture> {
        self.transformed(&recolored(Color { key: Some(key_given(&key, tolerance)?), ..Default::default() }))
    }

    /// ColorKey on a thread of its own: returns at once with a task whose
    /// result is the new texture.
    pub fn color_key_async(&self, key: PsObject, tolerance: Option<f64>, task: PsTask<Texture>) -> PsResult<()> {
        self.transformed_async(recolored(Color { key: Some(key_given(&key, tolerance)?), ..Default::default() }), task)
    }

    /// A new texture whose texels with alpha below `threshold` get alpha 0
    /// and the rest alpha 1, as an alpha test would: above 0 and at most 1.
    pub fn alpha_threshold(&self, threshold: f64) -> PsResult<Texture> {
        self.transformed(&recolored(Color { threshold: color::threshold_of(Some(threshold))?, ..Default::default() }))
    }

    /// AlphaThreshold on a thread of its own: returns at once with a task
    /// whose result is the new texture.
    pub fn alpha_threshold_async(&self, threshold: f64, task: PsTask<Texture>) -> PsResult<()> {
        self.transformed_async(recolored(Color { threshold: color::threshold_of(Some(threshold))?, ..Default::default() }), task)
    }

    /// A new texture whose texels with alpha 0 take the color of the
    /// nearest texel with alpha above 0, their alpha left at 0, so
    /// filtering and mip levels blend no dark or keyed color in at a
    /// cutout's edge: every such texel, or those within `reach` texels of
    /// a texel with alpha above 0.
    pub fn fix_alpha_border(&self, reach: Option<u32>) -> PsResult<Texture> {
        self.transformed(&Transform { border: alpha::border_of(true, reach)?, ..Default::default() })
    }

    /// FixAlphaBorder on a thread of its own: returns at once with a task
    /// whose result is the new texture.
    pub fn fix_alpha_border_async(&self, reach: Option<u32>, task: PsTask<Texture>) -> PsResult<()> {
        self.transformed_async(Transform { border: alpha::border_of(true, reach)?, ..Default::default() }, task)
    }

    /// A new texture blurred by a Gaussian of standard deviation `sigma`
    /// in texels, or of two, across and down, in linear values weighed by
    /// alpha, reading past the edge as `edge` says, Clamp when omitted.
    pub fn blur(&self, sigma: Vec<f64>, edge: Option<FilterEdge>) -> PsResult<Texture> {
        self.transformed(&filtered(filter::Asked { blur: Some(sigma.as_slice()), edge, ..Default::default() })?)
    }

    /// Blur on a thread of its own: returns at once with a task whose
    /// result is the new texture.
    pub fn blur_async(&self, sigma: Vec<f64>, edge: Option<FilterEdge>, task: PsTask<Texture>) -> PsResult<()> {
        self.transformed_async(filtered(filter::Asked { blur: Some(sigma.as_slice()), edge, ..Default::default() })?, task)
    }

    /// A new texture sharpened by an unsharp mask: `amount` times the
    /// difference between each texel and a Gaussian blur of it, of
    /// standard deviation `radius` texels, 1 when omitted, added back in
    /// each channel where it is at least `threshold`, 0 when omitted, in
    /// linear values weighed by alpha, reading past the edge as `edge`
    /// says, Clamp when omitted.
    pub fn sharpen(&self, amount: f64, radius: Option<f64>, threshold: Option<f64>, edge: Option<FilterEdge>) -> PsResult<Texture> {
        self.transformed(&filtered(filter::Asked { sharpen: Some(amount), sharpen_radius: radius, sharpen_threshold: threshold, edge, ..Default::default() })?)
    }

    /// Sharpen on a thread of its own: returns at once with a task whose
    /// result is the new texture.
    pub fn sharpen_async(&self, amount: f64, radius: Option<f64>, threshold: Option<f64>, edge: Option<FilterEdge>, task: PsTask<Texture>) -> PsResult<()> {
        self.transformed_async(filtered(filter::Asked { sharpen: Some(amount), sharpen_radius: radius, sharpen_threshold: threshold, edge, ..Default::default() })?, task)
    }

    /// A new texture of the texture's edges, as `method` finds them: the
    /// size of each channel's Sobel, Scharr or Prewitt gradient, where a
    /// step from 0 to 1 reads 1, or of its Laplacian, in the values the
    /// texture stores; or Canny's edges of the luminance, 1 on an edge and
    /// 0 elsewhere, kept above the low of `threshold`'s two numbers where
    /// they link to one above the high, 0.1 and 0.2 when omitted, after a
    /// Gaussian of standard deviation `sigma` texels, 1 when omitted. Alpha
    /// is kept, and the texels past the edge are read as `edge` says,
    /// Clamp when omitted; for Canny by the smoothing, its gradient after
    /// repeating the edge texel.
    pub fn find_edges(&self, method: EdgeOperator, threshold: Option<Vec<f64>>, sigma: Option<f64>, edge: Option<FilterEdge>) -> PsResult<Texture> {
        self.transformed(&filtered(filter::Asked { edges: Some(method), edge_threshold: threshold.as_deref(), edge_sigma: sigma, edge, ..Default::default() })?)
    }

    /// FindEdges on a thread of its own: returns at once with a task whose
    /// result is the new texture.
    pub fn find_edges_async(&self, method: EdgeOperator, threshold: Option<Vec<f64>>, sigma: Option<f64>, edge: Option<FilterEdge>, task: PsTask<Texture>) -> PsResult<()> {
        self.transformed_async(filtered(filter::Asked { edges: Some(method), edge_threshold: threshold.as_deref(), edge_sigma: sigma, edge, ..Default::default() })?, task)
    }

    /// A new texture with each normal's Z, in blue, rebuilt from its X and
    /// Y, in red and green, as for a normal map stored in two channels; the
    /// texture needs a blue channel. A format that holds no negative values
    /// stores a normal n as n / 2 + 1 / 2, and any other stores n.
    pub fn reconstruct_z(&self) -> PsResult<Texture> {
        self.transformed(&recolored(Color { reconstruct_z: true, ..Default::default() }))
    }

    /// ReconstructZ on a thread of its own: returns at once with a task
    /// whose result is the new texture.
    pub fn reconstruct_z_async(&self, task: PsTask<Texture>) -> PsResult<()> {
        self.transformed_async(recolored(Color { reconstruct_z: true, ..Default::default() }), task)
    }

    /// A new texture with each normal scaled to length 1.
    pub fn normalize(&self) -> PsResult<Texture> {
        self.transformed(&recolored(Color { normalize: true, ..Default::default() }))
    }

    /// Normalize on a thread of its own: returns at once with a task whose
    /// result is the new texture.
    pub fn normalize_async(&self, task: PsTask<Texture>) -> PsResult<()> {
        self.transformed_async(recolored(Color { normalize: true, ..Default::default() }), task)
    }

    /// A new texture with each normal's green flipped, between the
    /// convention glTF reads, green up, and DirectX's, green down.
    pub fn flip_green(&self) -> PsResult<Texture> {
        self.transformed(&recolored(Color { flip_green: true, ..Default::default() }))
    }

    /// FlipGreen on a thread of its own: returns at once with a task whose
    /// result is the new texture.
    pub fn flip_green_async(&self, task: PsTask<Texture>) -> PsResult<()> {
        self.transformed_async(recolored(Color { flip_green: true, ..Default::default() }), task)
    }

    /// A normal map made from the texture's heights, as
    /// ConvertTo-WgpuNormalMap makes it: `strength`, 1 when omitted, is the
    /// slope a height step of 1 between neighboring texels gives; `channel`
    /// is where the heights are read, Red when omitted; `kernel` takes the
    /// slopes, Prewitt when omitted; `edge` is what is read past the
    /// edges, Continue when omitted; and `alpha` is what alpha holds, One
    /// when omitted. The normal map is Rgba8Unorm with green up, and has the
    /// height map's mip levels, each texel's normal scaled back to length
    /// 1; its Notes say what was done.
    pub fn to_normal_map(&self, strength: Option<f64>, channel: Option<HeightChannel>, kernel: Option<NormalKernel>, edge: Option<HeightEdge>, alpha: Option<NormalAlpha>) -> PsResult<Texture> {
        let step = HeightToNormal::new(strength.unwrap_or(1.0), "the strength", channel, kernel, edge, alpha, false)?;
        let mips = MipOptions::default().with_normals(Some(NormalMips::Renormalize), false);
        image_io::normal_map(&self.core, &self.source(), &step, None, false, mips, None, &mut Said::default())
    }

    /// ToNormalMap on a thread of its own: returns at once with a task
    /// whose result is the normal map.
    pub fn to_normal_map_async(
        &self,
        strength: Option<f64>,
        channel: Option<HeightChannel>,
        kernel: Option<NormalKernel>,
        edge: Option<HeightEdge>,
        alpha: Option<NormalAlpha>,
        task: PsTask<Texture>,
    ) -> PsResult<()> {
        let step = HeightToNormal::new(strength.unwrap_or(1.0), "the strength", channel, kernel, edge, alpha, false)?;
        let mips = MipOptions::default().with_normals(Some(NormalMips::Renormalize), false);
        let (core, source) = (self.core.clone(), self.source());
        tasks::spawn(task, move |stop| image_io::normal_map(&core, &source, &step, None, false, mips, Some(stop), &mut Said::default()), Ok)
    }

    /// A cube map made from the texture, one image holding its faces, as
    /// ConvertTo-WgpuCubeMap makes it: `layout` names the layout or
    /// projection the image's shape does not tell apart; `yaw` turns a
    /// panorama, an angular map or an octahedral map about the vertical
    /// axis by degrees, +X toward -Z; and `faceSize` sets the faces' size
    /// for one of those three. The cube keeps the texture's format, or takes
    /// `format`, and has mip level 0 alone; its Notes say what was done.
    pub fn to_cube_map(&self, layout: Option<CubeLayout>, yaw: Option<f64>, face_size: Option<u32>, format: Option<String>) -> PsResult<Texture> {
        cube::cube_from_image(&self.core, &self.source(), layout, yaw, face_size, format.as_deref(), None, &mut Said::default())
    }

    /// ToCubeMap on a thread of its own: returns at once with a task whose
    /// result is the cube map.
    pub fn to_cube_map_async(&self, layout: Option<CubeLayout>, yaw: Option<f64>, face_size: Option<u32>, format: Option<String>, task: PsTask<Texture>) -> PsResult<()> {
        let (core, source) = (self.core.clone(), self.source());
        tasks::spawn(task, move |stop| cube::cube_from_image(&core, &source, layout, yaw, face_size, format.as_deref(), Some(stop), &mut Said::default()), Ok)
    }

    /// A cube map of six textures, its faces in the order +X, -X, +Y, -Y,
    /// +Z, -Z, as ConvertTo-WgpuCubeMap makes it: square 2D textures of one
    /// size and format, their texels and the mip levels all six have kept
    /// as they are, in their format or `format`.
    pub fn cube_from_faces(faces: Vec<PsProxy<Texture>>, format: Option<String>) -> PsResult<Texture> {
        let faces = cube::faces_held(&faces)?;
        cube::cube_from_faces(&faces, &cube::FACE_NAMES, format.as_deref(), None, &mut Said::default())
    }

    /// CubeFromFaces on a thread of its own: returns at once with a task
    /// whose result is the cube map.
    pub fn cube_from_faces_async(faces: Vec<PsProxy<Texture>>, format: Option<String>, task: PsTask<Texture>) -> PsResult<()> {
        let faces = cube::faces_held(&faces)?;
        tasks::spawn(task, move |stop| cube::cube_from_faces(&faces, &cube::FACE_NAMES, format.as_deref(), Some(stop), &mut Said::default()), Ok)
    }

    /// The cube map written into one image in `layout`, as
    /// ConvertFrom-WgpuCubeMap writes it: a layout's cells take the faces as
    /// they are, and an AngularMap or an Octahedral map is resampled, its
    /// faces `faceSize` texels across, the cube's when omitted, turned by
    /// `yaw` degrees about the vertical axis, +X toward -Z. The image comes
    /// from mip level `mipLevel`, 0 when omitted, in the cube's format or
    /// `format`.
    pub fn to_layout(&self, layout: CubeLayout, yaw: Option<f64>, face_size: Option<u32>, mip_level: Option<u32>, format: Option<String>) -> PsResult<Texture> {
        cube::image_from_cube(&self.core, &self.source(), cube::shape(layout), yaw, face_size, mip_level, format.as_deref(), None, &mut Said::default())
    }

    /// ToLayout on a thread of its own: returns at once with a task whose
    /// result is the image.
    pub fn to_layout_async(&self, layout: CubeLayout, yaw: Option<f64>, face_size: Option<u32>, mip_level: Option<u32>, format: Option<String>, task: PsTask<Texture>) -> PsResult<()> {
        let (core, source) = (self.core.clone(), self.source());
        tasks::spawn(task, move |stop| cube::image_from_cube(&core, &source, cube::shape(layout), yaw, face_size, mip_level, format.as_deref(), Some(stop), &mut Said::default()), Ok)
    }

    /// The cube map resampled into a panorama, as ConvertFrom-WgpuCubeMap
    /// -Panorama makes it: center +X and left quarter -Z, 4 faces wide and
    /// 2 high, the faces `faceSize` texels across, the cube's when omitted,
    /// turned by `yaw` degrees about the vertical axis, +X toward -Z. The
    /// panorama comes from mip level `mipLevel`, 0 when omitted, in the
    /// cube's format or `format`.
    pub fn to_panorama(&self, yaw: Option<f64>, face_size: Option<u32>, mip_level: Option<u32>, format: Option<String>) -> PsResult<Texture> {
        cube::image_from_cube(&self.core, &self.source(), cube::PANORAMA, yaw, face_size, mip_level, format.as_deref(), None, &mut Said::default())
    }

    /// ToPanorama on a thread of its own: returns at once with a task whose
    /// result is the panorama.
    pub fn to_panorama_async(&self, yaw: Option<f64>, face_size: Option<u32>, mip_level: Option<u32>, format: Option<String>, task: PsTask<Texture>) -> PsResult<()> {
        let (core, source) = (self.core.clone(), self.source());
        tasks::spawn(task, move |stop| cube::image_from_cube(&core, &source, cube::PANORAMA, yaw, face_size, mip_level, format.as_deref(), Some(stop), &mut Said::default()), Ok)
    }

    /// The irradiance cube of the texture, an environment cube map or a
    /// panorama or layout its shape tells apart, as
    /// ConvertTo-WgpuIrradianceMap makes it: faces `faceSize` texels
    /// across, 32 when omitted, in Rgba16Float or `format`, and with
    /// `harmonics` the irradiance's spherical harmonics as its Harmonics.
    pub fn to_irradiance_map(&self, face_size: Option<u32>, harmonics: Option<bool>, format: Option<String>) -> PsResult<Texture> {
        cube::irradiance_map(&self.core, &self.source(), None, None, face_size, harmonics.unwrap_or(false), format.as_deref(), None, &mut Said::default())
    }

    /// ToIrradianceMap on a thread of its own: returns at once with a task
    /// whose result is the irradiance map.
    pub fn to_irradiance_map_async(&self, face_size: Option<u32>, harmonics: Option<bool>, format: Option<String>, task: PsTask<Texture>) -> PsResult<()> {
        let (core, source, harmonics) = (self.core.clone(), self.source(), harmonics.unwrap_or(false));
        tasks::spawn(task, move |stop| cube::irradiance_map(&core, &source, None, None, face_size, harmonics, format.as_deref(), Some(stop), &mut Said::default()), Ok)
    }

    /// The prefiltered specular cube of the texture, an environment cube
    /// map or a panorama or layout its shape tells apart, as
    /// ConvertTo-WgpuSpecularMap makes it: `mapping` steps the mip levels
    /// through roughness, Linear when omitted, by `samples` samples a texel,
    /// 1024 when omitted, into `mipLevels` levels, down to 2x2 for Linear
    /// and 16x16 for Quadratic when omitted, in Rgba16Float or `format`.
    pub fn to_specular_map(&self, mapping: Option<RoughnessMapping>, samples: Option<u32>, mip_levels: Option<u32>, format: Option<String>) -> PsResult<Texture> {
        cube::specular_map(&self.core, &self.source(), None, None, mapping, samples, mip_levels, format.as_deref(), None, &mut Said::default())
    }

    /// ToSpecularMap on a thread of its own: returns at once with a task
    /// whose result is the specular map.
    pub fn to_specular_map_async(&self, mapping: Option<RoughnessMapping>, samples: Option<u32>, mip_levels: Option<u32>, format: Option<String>, task: PsTask<Texture>) -> PsResult<()> {
        let (core, source) = (self.core.clone(), self.source());
        tasks::spawn(task, move |stop| cube::specular_map(&core, &source, None, None, mapping, samples, mip_levels, format.as_deref(), Some(stop), &mut Said::default()), Ok)
    }

    /// The BRDF table split-sum image-based lighting reads, as
    /// New-WgpuBrdfTable makes it: `size` texels a side, 128 when omitted,
    /// by `samples` samples a texel, 1024 when omitted, in Rg16Float or
    /// `format`, on `device`, the default device when omitted.
    pub fn new_brdf_table(size: Option<u32>, samples: Option<u32>, format: Option<String>, device: Option<PsProxy<Device>>) -> PsResult<Texture> {
        let core = cube::device_core(device.as_ref())?;
        cube::new_brdf_table(&core, size, samples, format.as_deref(), None, &mut Said::default())
    }

    /// NewBrdfTable on a thread of its own: returns at once with a task
    /// whose result is the table.
    pub fn new_brdf_table_async(size: Option<u32>, samples: Option<u32>, format: Option<String>, device: Option<PsProxy<Device>>, task: PsTask<Texture>) -> PsResult<()> {
        let core = cube::device_core(device.as_ref())?;
        tasks::spawn(task, move |stop| cube::new_brdf_table(&core, size, samples, format.as_deref(), Some(stop), &mut Said::default()), Ok)
    }

    /// How the texture differs from `reference`, as Compare-WgpuTexture
    /// writes it: for each mip level both have, each layer and each channel
    /// both store, the mean squared error, PSNR and SSIM. The textures need
    /// the same dimension, width, height and layers, and an NV12 or P010
    /// texture compares only with another NV12 or P010 texture.
    pub fn compare(&self, reference: PsProxy<Texture>) -> PsResult<Vec<TextureComparison>> {
        let reference = reference.with(|t| (t.core.clone(), t.source()))?;
        compare::compared(&reference, &(self.core.clone(), self.source()), None, &mut Said::default())
    }

    /// Compare on a thread of its own: returns at once with a task whose
    /// result is the rows.
    pub fn compare_async(&self, reference: PsProxy<Texture>, task: PsTask<Vec<TextureComparison>>) -> PsResult<()> {
        let (reference, texture) = (reference.with(|t| (t.core.clone(), t.source()))?, (self.core.clone(), self.source()));
        tasks::spawn(task, move |stop| compare::compared(&reference, &texture, Some(stop), &mut Said::default()), Ok)
    }

    /// A picture of how the texture differs from `reference`, as
    /// Compare-WgpuTexture -Difference makes it: an Rgba8Unorm texture whose
    /// texels show the error on a log scale, black at 2^-12 or less and full
    /// at 1 or more, red, green and blue in their own colors, or `channel`
    /// as gray. Its Notes say what it shows.
    pub fn difference(&self, reference: PsProxy<Texture>, channel: Option<TextureChannel>) -> PsResult<Texture> {
        let reference = reference.with(|t| (t.core.clone(), t.source()))?;
        compare::difference(&reference, &(self.core.clone(), self.source()), channel, None, &mut Said::default())
    }

    /// Difference on a thread of its own: returns at once with a task whose
    /// result is the picture.
    pub fn difference_async(&self, reference: PsProxy<Texture>, channel: Option<TextureChannel>, task: PsTask<Texture>) -> PsResult<()> {
        let (reference, texture) = (reference.with(|t| (t.core.clone(), t.source()))?, (self.core.clone(), self.source()));
        tasks::spawn(task, move |stop| compare::difference(&reference, &texture, channel, Some(stop), &mut Said::default()), Ok)
    }

    /// The texture's values, as Measure-WgpuTexture measures them: for each
    /// mip level, layer and channel, the minimum, maximum, mean, population
    /// standard deviation and a histogram of `bins` bins splitting `range`,
    /// the low end and the high end; 256 bins from 0 to 1 when omitted.
    pub fn measure(&self, bins: Option<u32>, range: Option<Vec<f64>>) -> PsResult<Vec<TextureMeasurement>> {
        let range = range.unwrap_or(compare::DEFAULT_RANGE.to_vec());
        compare::measured(&(self.core.clone(), self.source()), bins.unwrap_or(compare::DEFAULT_BINS), &range, None, &mut Said::default())
    }

    /// Measure on a thread of its own: returns at once with a task whose
    /// result is the rows.
    pub fn measure_async(&self, bins: Option<u32>, range: Option<Vec<f64>>, task: PsTask<Vec<TextureMeasurement>>) -> PsResult<()> {
        let (texture, range) = ((self.core.clone(), self.source()), range.unwrap_or(compare::DEFAULT_RANGE.to_vec()));
        let bins = bins.unwrap_or(compare::DEFAULT_BINS);
        tasks::spawn(task, move |stop| compare::measured(&texture, bins, &range, Some(stop), &mut Said::default()), Ok)
    }

    /// A new texture with its channels from `red`, `green`, `blue` and
    /// `alpha`, as Join-WgpuTexture takes them: each a texture, whose red
    /// channel it reads, a texture and a channel letter as a pair, or a
    /// number; 0 for an omitted color channel and 1 for an omitted alpha.
    /// It is Rgba8Unorm, or `format`: a color format, a BC format or a
    /// Basis Universal format.
    pub fn join(red: Option<PsObject>, green: Option<PsObject>, blue: Option<PsObject>, alpha: Option<PsObject>, format: Option<String>) -> PsResult<Texture> {
        color::join(red.as_ref(), green.as_ref(), blue.as_ref(), alpha.as_ref(), format.as_deref(), None, &mut Said::default())
    }

    /// Frees the texture's GPU memory now; the object stops working.
    pub fn destroy(&self) -> PsResult<()> {
        self.texture.destroy();
        Ok(())
    }
}

/// A view of a texture, bound in place of the whole texture.
#[psclass(name = "Wgpu.TextureView", mode = proxy, columns = ["Label", "Format"])]
pub struct TextureView {
    /// The texture's label.
    pub label: String,
    /// The format the view reads the texels as.
    pub format: String,
    #[psfield(skip)]
    pub(crate) view: wgpu::TextureView,
    #[psfield(skip)]
    pub(crate) core: CoreRef,
}

/// How a shader samples textures.
#[psclass(name = "Wgpu.Sampler", mode = proxy, columns = ["Label", "MagFilter", "MinFilter", "AddressModeU", "Compare"])]
pub struct Sampler {
    /// The sampler's label.
    pub label: String,
    /// The filter when a texel covers more than a pixel.
    pub mag_filter: FilterMode,
    /// The filter when a texel covers less than a pixel.
    pub min_filter: FilterMode,
    /// The filter between mip levels.
    pub mipmap_filter: FilterMode,
    /// What it reads past the left and right edges.
    pub address_mode_u: AddressMode,
    /// What it reads past the top and bottom edges.
    pub address_mode_v: AddressMode,
    /// What it reads past the front and back of a 3D texture.
    pub address_mode_w: AddressMode,
    /// The comparison of a comparison sampler; `$null` for a filtering one.
    pub compare: Option<CompareFunction>,
    /// The anisotropic filtering clamp, 1 to 16.
    pub anisotropy: u16,
    #[psfield(skip)]
    pub(crate) sampler: wgpu::Sampler,
    #[psfield(skip)]
    pub(crate) core: CoreRef,
}

/// The resource a texture, a view or a sampler binds as for `binding`, and
/// its device. A view and a sampler bind as they were made; a texture binds
/// whole, through the view [`whole_view`] makes.
pub(crate) fn resolve(value: &PsObject, type_name: &str, binding: &Reflected) -> PsResult<(Resource, CoreRef)> {
    match type_name {
        "Wgpu.Texture" => {
            let (texture, label, core) = PsProxy::<Texture>::from_ps(value)?.with(|t| (t.texture.clone(), t.label.clone(), t.core.clone()))?;
            let view = whole_view(&core, &texture, &label, binding)?;
            Ok((Resource::View(view), core))
        }
        "Wgpu.TextureView" => PsProxy::<TextureView>::from_ps(value)?.with(|v| (Resource::View(v.view.clone()), v.core.clone())),
        "Wgpu.Sampler" => PsProxy::<Sampler>::from_ps(value)?.with(|s| (Resource::Sampler(s.sampler.clone()), s.core.clone())),
        other => Err(invalid("WgpuWrongResource", format!("{other} is not a texture, a view or a sampler"))),
    }
}

/// The view of all of `texture` that `binding` reads: the binding's
/// dimension over every layer and mip level, and of a format holding depth
/// and stencil, the one the binding's sample type reads. A binding that
/// cannot read the texture whole is refused, naming CreateView, which picks
/// the part to bind.
fn whole_view(core: &CoreRef, texture: &wgpu::Texture, label: &str, binding: &Reflected) -> PsResult<wgpu::TextureView> {
    use wgpu::TextureViewDimension as V;
    let (dimension, aspect, storage) = match binding.ty {
        wgpu::BindingType::Texture { sample_type, view_dimension, .. } => (view_dimension, read_aspect(texture.format(), sample_type), false),
        wgpu::BindingType::StorageTexture { view_dimension, .. } => (view_dimension, wgpu::TextureAspect::All, true),
        _ => {
            return core.scoped("WgpuTextureView", "the view cannot be created", || {
                texture.create_view(&wgpu::TextureViewDescriptor { label: Some(label), ..Default::default() })
            });
        }
    };
    let name = &binding.name;
    let binding_type = &binding.type_name;
    let (width, height, layers, levels) = (texture.width(), texture.height(), texture.depth_or_array_layers(), texture.mip_level_count());
    if dimension == V::D2 && texture.dimension() == wgpu::TextureDimension::D2 && layers != 1 {
        return Err(invalid("WgpuWrongResource", format!("{name} is a {binding_type} binding, which reads one layer, and the texture has {layers}; bind one through CreateView")));
    }
    if matches!(dimension, V::Cube | V::CubeArray) && width != height {
        return Err(invalid("WgpuWrongResource", format!("{name} is a {binding_type} binding, which reads square layers, and the texture is {width}x{height}")));
    }
    if dimension == V::Cube && layers != 6 {
        return Err(invalid("WgpuWrongResource", format!("{name} is a {binding_type} binding, which reads six layers, and the texture has {layers}; bind six through CreateView")));
    }
    if dimension == V::CubeArray && !layers.is_multiple_of(6) {
        return Err(invalid("WgpuWrongResource", format!("{name} is a {binding_type} binding, which reads layers six at a time, and the texture has {layers}; bind a multiple of six through CreateView")));
    }
    if storage && levels != 1 {
        return Err(invalid("WgpuWrongResource", format!("{name} is a {binding_type} binding, which takes one mip level, and the texture has {levels}; bind one through CreateView")));
    }
    core.scoped("WgpuTextureView", "the view cannot be created", || {
        texture.create_view(&wgpu::TextureViewDescriptor { label: Some(label), dimension: Some(dimension), aspect, ..Default::default() })
    })
}

/// The aspect of `format` a binding of `sample_type` reads: of a format
/// holding depth and stencil, the stencil for an unsigned integer binding
/// and the depth for any other; all of any other format.
fn read_aspect(format: wgpu::TextureFormat, sample_type: wgpu::TextureSampleType) -> wgpu::TextureAspect {
    if !format.is_combined_depth_stencil_format() {
        wgpu::TextureAspect::All
    } else if sample_type == wgpu::TextureSampleType::Uint {
        wgpu::TextureAspect::StencilOnly
    } else {
        wgpu::TextureAspect::DepthOnly
    }
}

/// A format and what it can do on one adapter.
#[psclass(name = "Wgpu.TextureFormatInfo", columns = ["Name", "BlockSize", "Components", "AllowedUsages"])]
#[derive(Default, Clone)]
pub struct TextureFormatInfo {
    /// The format's name, as -Format takes it.
    pub name: String,
    /// Bytes per block (per texel for an uncompressed format); `$null` for a combined depth-stencil format.
    pub block_size: Option<u32>,
    /// Texels across a block: 1 for uncompressed formats.
    pub block_width: u32,
    /// Texels down a block.
    pub block_height: u32,
    /// Channels per texel.
    pub components: u32,
    /// Whether it is block-compressed.
    pub is_compressed: bool,
    /// Whether it holds depth or stencil.
    pub is_depth_stencil: bool,
    /// Whether it stores sRGB-encoded color.
    pub is_srgb: bool,
    /// The type its texels read and write as.
    pub element_type: ElementType,
    /// The features a device needs to use it.
    pub required_features: Vec<String>,
    /// What a texture of this format may be used for on the adapter.
    pub allowed_usages: Vec<TextureUsage>,
    /// What the adapter can do with it beyond its usages: Filterable, Blendable, MultisampleX2..X16, MultisampleResolve, StorageReadOnly, StorageWriteOnly, StorageReadWrite, StorageAtomic.
    pub capabilities: Vec<String>,
}

fn describe_format(adapter: &wgpu::Adapter, format: wgpu::TextureFormat) -> TextureFormatInfo {
    let features = adapter.get_texture_format_features(format);
    let (bw, bh) = format.block_dimensions();
    TextureFormatInfo {
        name: format_name(format),
        block_size: format.block_copy_size(None),
        block_width: bw,
        block_height: bh,
        components: format.components() as u32,
        is_compressed: format.is_compressed(),
        is_depth_stencil: format.is_depth_stencil_format(),
        is_srgb: format.is_srgb(),
        element_type: texel_element(format, wgpu::TextureAspect::All),
        required_features: names::feature_names(format.required_features()),
        allowed_usages: usage_list(without_depth_storage(format, features.allowed_usages)),
        capabilities: features.flags.iter_names().map(|(n, _)| names::pascal(n)).collect(),
    }
}

/// The texture format names that start with `word`, ignoring case.
fn format_completions(word: &str) -> Vec<Completion> {
    let prefix = word.to_lowercase();
    all_formats().into_iter().map(format_name).filter(|n| n.to_lowercase().starts_with(&prefix)).map(Completion::value).collect()
}

/// Completes -Format with every texture format wgpu has.
#[completer(cmdlet = "New-WgpuTexture", parameter = "Format")]
pub(crate) fn complete_texture_format(ctx: &CompletionContext) -> PsResult<Vec<Completion>> {
    Ok(format_completions(&ctx.word))
}

/// The texture format names and Basis Universal format names that start
/// with `word`, ignoring case.
fn target_completions(word: &str) -> Vec<Completion> {
    let prefix = word.to_lowercase();
    let mut out = format_completions(word);
    out.extend(image_io::BASIS_FORMATS.iter().map(|b| b.name).filter(|n| n.to_lowercase().starts_with(&prefix)).map(Completion::value));
    out
}

/// Completes -Format with every texture format wgpu has and the Basis
/// Universal formats.
#[completer(cmdlet = "Import-WgpuImage", parameter = "Format")]
pub(crate) fn complete_image_format(ctx: &CompletionContext) -> PsResult<Vec<Completion>> {
    Ok(target_completions(&ctx.word))
}

/// Completes -Format with every texture format wgpu has and the Basis
/// Universal formats.
#[completer(cmdlet = "Export-WgpuImage", parameter = "Format")]
pub(crate) fn complete_export_format(ctx: &CompletionContext) -> PsResult<Vec<Completion>> {
    Ok(target_completions(&ctx.word))
}

/// Completes -Format with every texture format wgpu has and the Basis
/// Universal formats.
#[completer(cmdlet = "Join-WgpuTexture", parameter = "Format")]
pub(crate) fn complete_join_format(ctx: &CompletionContext) -> PsResult<Vec<Completion>> {
    Ok(target_completions(&ctx.word))
}

/// Completes -Format with every texture format wgpu has and the Basis
/// Universal formats.
#[completer(cmdlet = "ConvertTo-WgpuNormalMap", parameter = "Format")]
pub(crate) fn complete_normal_map_format(ctx: &CompletionContext) -> PsResult<Vec<Completion>> {
    Ok(target_completions(&ctx.word))
}

/// The weightings -GrayscaleWeights names that start with `word`, ignoring
/// case.
fn weighting_completions(word: &str) -> Vec<Completion> {
    let prefix = word.to_lowercase();
    ["Rec709", "Rec601"].into_iter().filter(|n| n.to_lowercase().starts_with(&prefix)).map(Completion::value).collect()
}

/// Completes -GrayscaleWeights with the weightings it names.
#[completer(cmdlet = "Convert-WgpuTexture", parameter = "GrayscaleWeights")]
pub(crate) fn complete_convert_weights(ctx: &CompletionContext) -> PsResult<Vec<Completion>> {
    Ok(weighting_completions(&ctx.word))
}

/// Completes -GrayscaleWeights with the weightings it names.
#[completer(cmdlet = "Import-WgpuImage", parameter = "GrayscaleWeights")]
pub(crate) fn complete_import_weights(ctx: &CompletionContext) -> PsResult<Vec<Completion>> {
    Ok(weighting_completions(&ctx.word))
}

/// Completes -GrayscaleWeights with the weightings it names.
#[completer(cmdlet = "Export-WgpuImage", parameter = "GrayscaleWeights")]
pub(crate) fn complete_export_weights(ctx: &CompletionContext) -> PsResult<Vec<Completion>> {
    Ok(weighting_completions(&ctx.word))
}

/// Completes -Name with every texture format wgpu has.
#[completer(cmdlet = "Get-WgpuTextureFormat", parameter = "Name")]
pub(crate) fn complete_format_name(ctx: &CompletionContext) -> PsResult<Vec<Completion>> {
    Ok(format_completions(&ctx.word))
}

/// A -MipEdge argument as the edge it names: ClampToEdge, Repeat and
/// MirrorRepeat, as names or Wgpu.AddressMode values, are Clamp, Wrap and
/// Mirror, and ClampToBorder is refused; any other value goes on for the
/// binder to read as a Wgpu.MipEdge.
fn mip_edge_argument(value: &PsObject) -> PsResult<PsObject> {
    match MipEdge::named(&String::from_ps(value)?)? {
        Some(edge) => edge.into_ps(),
        None => value.clone().into_ps(),
    }
}

/// Reads a sampler's address mode given to Import-WgpuImage -MipEdge.
#[transform(cmdlet = "Import-WgpuImage", parameter = "MipEdge")]
pub(crate) fn import_mip_edge(value: &PsObject) -> PsResult<PsObject> {
    mip_edge_argument(value)
}

/// Reads a sampler's address mode given to Export-WgpuImage -MipEdge.
#[transform(cmdlet = "Export-WgpuImage", parameter = "MipEdge")]
pub(crate) fn export_mip_edge(value: &PsObject) -> PsResult<PsObject> {
    mip_edge_argument(value)
}

/// The edge GenerateMips's `edge` names: Clamp, Wrap or Mirror, or a
/// sampler's ClampToEdge, Repeat or MirrorRepeat, as a name, a
/// Wgpu.MipEdge or a Wgpu.AddressMode.
fn edge_of(value: &PsObject) -> PsResult<MipEdge> {
    let name = String::from_ps(value)?;
    MipEdge::named(&name)?.ok_or_else(|| invalid("WgpuMipEdge", format!("{name} is not a mip edge: the edge is Clamp, Wrap or Mirror, or ClampToEdge, Repeat or MirrorRepeat")))
}

/// Creates a texture: 1D, 2D (with array layers) or 3D, of any format.
///
/// Without -Usage the texture takes every usage its format allows on the
/// adapter, of copy, sampling, storage and render attachment. A depth or
/// stencil format is never a storage binding. -Data fills mip level 0 of
/// layer 0, converted to the format's number type.
///
/// # Examples
/// $tex = New-WgpuTexture -Width 512 -Height 512
/// New-WgpuTexture 256 256 -Format R32Float -Data ([float[]]::new(65536))
/// New-WgpuTexture 64 64 -Depth 64 -Dimension D3 -Format Rgba16Float
#[cmdlet(verb = "New", noun = "WgpuTexture", alias = ["New-GpuTexture"], output = ["Wgpu.Texture"])]
#[derive(Default)]
pub struct NewWgpuTexture {
    /// Width in texels.
    #[param(mandatory, position = 0)]
    pub width: u32,
    /// Height in texels; 1 when omitted.
    #[param(position = 1)]
    pub height: Option<u32>,
    /// Depth of a 3D texture, or array layers of a 2D one; 1 when omitted.
    #[param(alias = ["Layers"])]
    pub depth: Option<u32>,
    /// The format's name; Rgba8Unorm when omitted.
    #[param]
    pub format: Option<String>,
    /// Mip levels; 1 when omitted.
    #[param]
    pub mip_levels: Option<u32>,
    /// Samples per texel; 1 when omitted.
    #[param]
    pub sample_count: Option<u32>,
    /// D1, D2 or D3; D2 when omitted.
    #[param]
    pub dimension: Option<TextureDimension>,
    /// What it may be used for; every usage the format allows when omitted.
    #[param]
    pub usage: Option<Vec<TextureUsage>>,
    /// Other formats views of it may take.
    #[param]
    pub view_format: Option<Vec<String>>,
    /// Texels for mip level 0 of layer 0, rows packed.
    #[param(raw)]
    pub data: Option<PsObject>,
    /// Record it as a cube map: square 2D layers, six to a cube, so
    /// -Height equal to -Width and -Depth 6, or a multiple of 6 for an
    /// array of cubes.
    #[param]
    pub cube: bool,
    /// The texture's label.
    #[param]
    pub label: Option<String>,
    /// The device to create it on; the default device when omitted.
    #[param]
    pub device: Option<PsProxy<Device>>,
}

impl Cmdlet for NewWgpuTexture {
    fn process(&mut self, ps: &Pipeline<'_>) -> PsResult<()> {
        let core = core_for(&self.device, ps)?;
        let format = match &self.format {
            Some(name) => parse_format(name)?,
            None => wgpu::TextureFormat::Rgba8Unorm,
        };
        let mut view_formats = Vec::new();
        if let Some(list) = &self.view_format {
            for name in list {
                view_formats.push(parse_format(name)?);
            }
        }
        let spec = TextureSpec {
            width: self.width,
            height: self.height.unwrap_or(1),
            depth_or_layers: self.depth.unwrap_or(1),
            format,
            mip_levels: self.mip_levels.unwrap_or(1),
            sample_count: self.sample_count.unwrap_or(1),
            dimension: self.dimension.unwrap_or(if self.height.is_some() { TextureDimension::D2 } else { TextureDimension::D1 }),
            usage: self.usage.clone(),
            view_formats,
            label: self.label.clone(),
        };
        let mut texture = create(&core, &spec)?;
        if self.cube {
            check_cube(&texture.texture)?;
            texture.is_cube = true;
        }
        if let Some(data) = &self.data {
            let ty = texel_element(format, wgpu::TextureAspect::All);
            element::with_host_bytes(data, Some(ty), |_, bytes| write_texels(&core, &texture.texture, 0, 0, wgpu::TextureAspect::All, bytes))?;
        }
        ps.write(texture)
    }
}

/// Writes what reading, converting or writing a texture file said: its
/// lines to the verbose stream, its warnings to the warning stream.
pub(crate) fn tell(ps: &Pipeline<'_>, said: &Said) -> PsResult<()> {
    for line in &said.verbose {
        ps.verbose(line)?;
    }
    for line in &said.warnings {
        ps.warning(line)?;
    }
    Ok(())
}

/// Copies a texture's texels back from the GPU and writes them as one
/// array, rows packed, as the format's number type.
///
/// # Examples
/// Read-WgpuTexture $tex
/// $tex | Read-WgpuTexture -MipLevel 1 -AsBytes
#[cmdlet(verb = "Read", noun = "WgpuTexture", alias = ["Read-GpuTexture"], output = ["System.Array"])]
#[derive(Default)]
pub struct ReadWgpuTexture {
    /// The texture to read.
    #[param(mandatory, position = 0, value_from_pipeline)]
    pub texture: Option<PsProxy<Texture>>,
    /// The mip level; 0 when omitted.
    #[param]
    pub mip_level: Option<u32>,
    /// The array layer; 0 when omitted.
    #[param]
    pub layer: Option<u32>,
    /// The aspect of a depth-stencil texture, or the plane of an NV12 or P010 texture.
    #[param]
    pub aspect: Option<TextureAspect>,
    /// Write the texels as byte[] instead of the format's number type.
    #[param]
    pub as_bytes: bool,
}

impl Cmdlet for ReadWgpuTexture {
    fn process(&mut self, ps: &Pipeline<'_>) -> PsResult<()> {
        let Some(texture) = &self.texture else {
            return Err(invalid("WgpuNoTexture", "no texture was given"));
        };
        let (core, raw) = texture.with(|t| (t.core.clone(), t.texture.clone()))?;
        let aspect = self.aspect.map_or(wgpu::TextureAspect::All, TextureAspect::wgpu);
        let bytes = read_texels(&core, &raw, self.mip_level.unwrap_or(0), self.layer.unwrap_or(0), aspect, Some(ps))?;
        let ty = if self.as_bytes { ElementType::UInt8 } else { texel_element(raw.format(), aspect) };
        ps.write_object(&element::to_clr_array(ty, &bytes)?)
    }
}

/// Writes texels into a texture, rows packed, converted to the format's
/// number type.
///
/// # Examples
/// Write-WgpuTexture $tex $pixels
/// Write-WgpuTexture $tex $level1 -MipLevel 1
#[cmdlet(verb = "Write", noun = "WgpuTexture", alias = ["Write-GpuTexture"])]
#[derive(Default)]
pub struct WriteWgpuTexture {
    /// The texture to write.
    #[param(mandatory, position = 0)]
    pub texture: Option<PsProxy<Texture>>,
    /// The texels, rows packed.
    #[param(mandatory, position = 1, raw)]
    pub data: Option<PsObject>,
    /// The mip level; 0 when omitted.
    #[param]
    pub mip_level: Option<u32>,
    /// The array layer; 0 when omitted.
    #[param]
    pub layer: Option<u32>,
    /// The aspect of a depth-stencil texture, or the plane of an NV12 or P010 texture.
    #[param]
    pub aspect: Option<TextureAspect>,
}

impl Cmdlet for WriteWgpuTexture {
    fn process(&mut self, _ps: &Pipeline<'_>) -> PsResult<()> {
        let (Some(texture), Some(data)) = (&self.texture, &self.data) else {
            return Err(invalid("WgpuNoTexture", "a texture and data are both needed"));
        };
        texture.with(|t| t.write(data.clone(), self.mip_level, self.layer, self.aspect))?
    }
}

/// Loads an image file into a texture: PNG, JPEG, GIF, WebP, AVIF, JPEG XL,
/// TIFF, Photoshop's PSD and PSB, BMP, TGA, EXR, HDR, QOI, ICO, PNM or
/// Farbfeld, and a DDS or KTX2 file whole, with every mip level, array
/// layer and cube face; and draws an SVG file or a PDF's first page.
///
/// An SVG file is drawn at the size it gives, in pixels of a 96th of an
/// inch, and a PDF's page at 72 dots per inch, a pixel a point, or at the
/// -Dpi given, on white unless -NoBackground leaves it transparent;
/// -Password opens a protected PDF. -Width, -Height, -Scale, -MaxSize and
/// -PowerOfTwo set the size either is drawn at, and -Crop, -CropMargin,
/// -Rotate and -Flip are drawn too, so the drawing is never resampled. An
/// SVG's text is drawn in the machine's fonts; text in a font-family the
/// machine has no font of is drawn in its default family, which a warning
/// says.
///
/// An image file's pixels are turned as its orientation says: an EXIF
/// orientation, a PNG's in the text chunk ImageMagick writes it in
/// included, an AVIF's rotation and mirror, or a JPEG XL's own. Its colors
/// are converted to sRGB from the color profile it carries, an ICC profile
/// or the CICP values of an AVIF or an HDR JPEG XL, as littleCMS converts
/// with its optimizations off: perceptually, or with the -RenderingIntent
/// named, with colors sRGB lacks clipped. A float image's colors are
/// converted to linear light in sRGB's primaries instead, light below 0
/// clipped and light above 1 kept, and a PQ or HLG AVIF or JPEG XL loads as
/// linear light in sRGB's primaries too, with BT.2408's HDR reference white
/// of 203 nits at 1. An orientation or profile the module cannot use leaves
/// the file as stored, which a warning says. -IgnoreOrientation loads the
/// pixels as stored, and -IgnoreProfile the colors.
///
/// Without -Format a float image loads as Rgba32Float, a PQ or HLG AVIF or
/// JPEG XL as Rgba16Float, a 16-bit image as Rgba16Unorm, and the rest as
/// Rgba8Unorm, or Rgba8UnormSrgb with -Srgb.
/// A DDS or KTX2 file keeps its format; a block-compressed one the device
/// cannot hold is decoded on the CPU, which a warning says. A KTX2 file in
/// a Basis Universal format is transcoded to the best format the device
/// holds.
///
/// -Format converts the texture: into a color format, a BC format encoded
/// on the GPU, or a Basis Universal format, which Basis Universal encodes
/// and the module transcodes. An image file's 8-bit color is taken as already in
/// the -Format's encoding, and a two-channel -Format takes red and green, or
/// a gray image's gray and alpha. -MipLevels gives it mip levels, rendered on the
/// GPU: each from the level above by the Box filter, or as -MipFilter and
/// -MipEdge say. -Blur and -Sharpen filter the texels after the size,
/// orientation and texel changes, as Convert-WgpuTexture does.
/// -HeightToNormal makes the heights into normals after those, as
/// ConvertTo-WgpuNormalMap does,
/// and -NormalMips keeps normals unit length in the mip levels. -Cube makes
/// a cube map after those changes and before -Format and -MipLevels: from a
/// panorama or a layout of the faces, read by its shape, -Layout, -Yaw and
/// -FaceSize as ConvertTo-WgpuCubeMap reads them, or from a DDS or KTX2
/// file's square layers, six to a cube, recorded as its faces. What the
/// import involved goes to the verbose stream and the texture's Notes.
///
/// # Examples
/// $photo = Import-WgpuImage .\photo.jpg
/// Import-WgpuImage .\height.png -Format R16Unorm
/// Import-WgpuImage .\albedo.png -Format Bc7RgbaUnormSrgb -MipLevels 0
/// Import-WgpuImage .\stone.png -MipLevels 0 -MipFilter Kaiser -MipEdge Wrap
/// Import-WgpuImage .\sky.ktx2
/// Get-ChildItem .\textures -Filter *.png | Import-WgpuImage -MipLevels 0
/// Import-WgpuImage .\height.png -HeightToNormal 4 -Format Bc5RgUnorm -TwoChannel -MipLevels 0
/// Import-WgpuImage .\sky.hdr -Cube -FaceSize 512 -Format Rgba16Float -MipLevels 0
/// Import-WgpuImage .\logo.svg -Width 1024
/// Import-WgpuImage .\page.pdf -Dpi 300 -NoBackground
#[cmdlet(verb = "Import", noun = "WgpuImage", alias = ["Import-GpuImage"], default_parameter_set = "Path", output = ["Wgpu.Texture"])]
#[derive(Default)]
pub struct ImportWgpuImage {
    /// The image, DDS or KTX2 file; a wildcard loads every file it matches.
    #[param(mandatory, position = 0, set = "Path", value_from_pipeline_by_property_name, supports_wildcards)]
    pub path: Option<String>,
    /// The image, DDS or KTX2 file, named exactly as written, wildcard
    /// characters included; the files Get-ChildItem pipes bind here.
    #[param(mandatory, set = "LiteralPath", value_from_pipeline_by_property_name, alias = ["PSPath", "LP", "FullName"])]
    pub literal_path: Option<String>,
    /// The format to load the texture in: a color format, a BC format,
    /// encoded on the GPU, or a Basis Universal format such as BasisEtc1s
    /// or BasisUastc4x4, which Basis Universal encodes.
    #[param]
    pub format: Option<String>,
    /// Mip levels: those the file has when omitted, and 0 for the full
    /// chain down to 1x1.
    #[param]
    pub mip_levels: Option<u32>,
    /// How each mip level -MipLevels adds, or a transform renders again,
    /// is made: Box, the default, from the level above; Triangle, Kaiser,
    /// Mitchell or Lanczos from level 0; Min or Max, the smallest or
    /// largest value of each channel among the texels it covers.
    #[param]
    pub mip_filter: Option<MipFilter>,
    /// What Triangle, Kaiser, Mitchell and Lanczos read past an edge of
    /// the image: Clamp, the default, nothing; Wrap, the other side, for a
    /// texture that tiles; or Mirror, the image mirrored. A sampler's
    /// ClampToEdge, Repeat and MirrorRepeat name the same.
    #[param]
    pub mip_edge: Option<MipEdge>,
    /// Scales the alpha of each mip level -MipLevels adds, or a transform
    /// renders again, so the share of its texels with alpha at or above
    /// this, the alpha an alpha test passes at, stays level 0's: above 0
    /// and at most 1.
    #[param]
    pub keep_coverage: Option<f64>,
    /// Keeps a normal map's normals unit length in each mip level
    /// -MipLevels adds, or a transform renders again: Renormalize scales
    /// each texel's normal back to length 1, and SlopeSpace filters the
    /// normals' slopes; Renormalize with -HeightToNormal when omitted. It
    /// marks the texels as normals, which a -Format that stores them the
    /// other way takes written again: n / 2 + 1 / 2 in a format that holds
    /// no negative values, and n in any other.
    #[param]
    pub normal_mips: Option<NormalMips>,
    /// Basis Universal's quality, 1 to 100, for a Basis format that has
    /// one; Basis Universal's default when omitted.
    #[param]
    pub quality: Option<u32>,
    /// Encoding effort, 0 to 10: Basis Universal's for a Basis format, and
    /// the preset of Bc7RgbaUnorm, Bc7RgbaUnormSrgb and Bc6hRgbUfloat; the
    /// encoder's default when omitted.
    #[param]
    pub effort: Option<u32>,
    /// Read 8-bit color as sRGB: an image file's without a GPU -Format, and
    /// a DDS or KTX2 file's where its format has an sRGB form.
    #[param]
    pub srgb: bool,
    /// Loads an image file's pixels as the file stores them, not turned as
    /// its orientation says.
    #[param]
    pub ignore_orientation: bool,
    /// Loads an image file's colors as the file stores them, not converted
    /// from its color profile.
    #[param]
    pub ignore_profile: bool,
    /// How a conversion from an image file's color profile maps colors,
    /// above all those sRGB lacks: Perceptual when omitted,
    /// RelativeColorimetric, Saturation or AbsoluteColorimetric.
    #[param]
    pub rendering_intent: Option<RenderingIntent>,
    /// The resolution a PDF's page is drawn at, in dots per inch: 72, a
    /// pixel a point, when omitted.
    #[param]
    pub dpi: Option<f64>,
    /// Draws a PDF's page with no background, transparent where nothing is
    /// drawn, not on white.
    #[param]
    pub no_background: bool,
    /// The password a protected PDF opens with.
    #[param]
    pub password: Option<PsSecureString>,
    /// What the texture may be used for; every usage the format allows when omitted.
    #[param]
    pub usage: Option<Vec<TextureUsage>>,
    /// The texture's label; the file's path when omitted.
    #[param]
    pub label: Option<String>,
    /// The device to load it onto; the default device when omitted.
    #[param]
    pub device: Option<PsProxy<Device>>,
    /// Resizes the texture to this width before -Format converts it;
    /// without -Height, the height keeps the aspect ratio.
    #[param]
    pub width: Option<u32>,
    /// Resizes the texture to this height; without -Width, the width keeps
    /// the aspect ratio.
    #[param]
    pub height: Option<u32>,
    /// Resizes both sides by this factor, such as 0.5 for half the size.
    #[param]
    pub scale: Option<f64>,
    /// Shrinks a texture with a side past this many texels to it, keeping
    /// the aspect ratio.
    #[param]
    pub max_size: Option<u32>,
    /// Rounds each side to a power of two: Nearest, Up or Down.
    #[param]
    pub power_of_two: Option<PowerOfTwo>,
    /// The resize filter: Box where a side shrinks and Bicubic where it
    /// grows when omitted.
    #[param]
    pub filter: Option<ResizeFilter>,
    /// Mirrors the texture: Horizontal or Vertical.
    #[param]
    pub flip: Option<FlipDirection>,
    /// Turns the texture clockwise by 90, 180 or 270 degrees.
    #[param]
    pub rotate: Option<u32>,
    /// Keeps a region, four numbers for its left, top, width and height,
    /// or a size -Anchor places, two numbers.
    #[param]
    pub crop: Option<Vec<u32>>,
    /// Cuts this many texels from the left, top, right and bottom.
    #[param]
    pub crop_margin: Option<Vec<u32>>,
    /// Places the texture on a larger canvas of this width and height.
    #[param]
    pub pad: Option<Vec<u32>>,
    /// Where a crop to a size or the padding places the texture; Center
    /// when omitted.
    #[param]
    pub anchor: Option<Anchor>,
    /// The color around the padded texture: red, green, blue and alpha
    /// from 0 to 1; transparent black when omitted.
    #[param]
    pub pad_color: Option<Vec<f64>>,
    /// Rearranges the channels: four letters of r, g, b, a, 0 and 1, one
    /// for each channel, as bgra or rrr1.
    #[param]
    pub swizzle: Option<String>,
    /// Multiplies red, green and blue by alpha, in linear light for an sRGB
    /// texture.
    #[param]
    pub premultiply: bool,
    /// Divides red, green and blue by alpha where alpha is above 0, in
    /// linear light for an sRGB texture.
    #[param]
    pub unpremultiply: bool,
    /// Converts the texels to Linear or Srgb so their colors look the same.
    #[param]
    pub color_space: Option<ColorSpace>,
    /// Relabels the texels as Linear or Srgb without changing them.
    #[param]
    pub assign_color_space: Option<ColorSpace>,
    /// Makes the texture grayscale, with Rec. 709's weights in linear light
    /// or those -GrayscaleWeights names.
    #[param]
    pub grayscale: bool,
    /// The weights -Grayscale uses: Rec709, Rec601 or three numbers for
    /// red, green and blue.
    #[param]
    pub grayscale_weights: Option<PsObject>,
    /// Turns red, green and blue into 1 minus their stored values; alpha is
    /// kept.
    #[param]
    pub invert: bool,
    /// Brightens or darkens the stored values, from -100 to 100, as
    /// ImageMagick's -brightness-contrast.
    #[param]
    pub brightness: Option<f64>,
    /// Raises or lowers the stored values' contrast, from -100 to 100, as
    /// ImageMagick's -brightness-contrast.
    #[param]
    pub contrast: Option<f64>,
    /// Gamma corrects the stored values to value^(1/gamma), as
    /// ImageMagick's -gamma.
    #[param]
    pub gamma: Option<f64>,
    /// Multiplies linear values by 2 to this power: stops of exposure,
    /// applied before -ToneMap.
    #[param]
    pub exposure: Option<f64>,
    /// Maps HDR colors into the range from 0 to 1: Reinhard, AcesFilmic,
    /// AgX or Neutral.
    #[param]
    pub tone_map: Option<ToneMapCurve>,
    /// Gives alpha 0 to the texels of this color, as the texture stores
    /// it: red, green and blue from 0 to 1, rounded to the texture's own
    /// precision, or '#RRGGBB'. Other texels keep their alpha.
    #[param]
    pub color_key: Option<PsObject>,
    /// How far each of red, green and blue may be from -ColorKey's, from 0
    /// to 1; 0, the same stored value, when omitted.
    #[param]
    pub color_key_tolerance: Option<f64>,
    /// Gives alpha 0 to texels whose alpha is below this, and 1 to the
    /// rest, as an alpha test would: above 0 and at most 1.
    #[param]
    pub alpha_threshold: Option<f64>,
    /// Blurs the texels by a Gaussian of this standard deviation in texels,
    /// or of two, across and down, in linear values weighed by alpha.
    #[param]
    pub blur: Option<Vec<f64>>,
    /// Sharpens the texels by an unsharp mask: this many times the
    /// difference between each texel and a Gaussian blur of it is added
    /// back, in linear values weighed by alpha.
    #[param]
    pub sharpen: Option<f64>,
    /// The standard deviation, in texels, of the blur -Sharpen compares
    /// each texel with; 1 when omitted.
    #[param]
    pub sharpen_radius: Option<f64>,
    /// The smallest difference from the blur, from 0 to 1, that -Sharpen
    /// adds back in a channel; 0 when omitted.
    #[param]
    pub sharpen_threshold: Option<f64>,
    /// What -Blur and -Sharpen read past the edge: Clamp, the edge texel
    /// repeated, the default; Wrap, the other side, for an image that
    /// tiles; or Mirror.
    #[param]
    pub filter_edge: Option<FilterEdge>,
    /// Gives each texel with alpha 0 the color of the nearest texel with
    /// alpha above 0, its alpha left at 0, so filtering and mip levels
    /// blend no dark or keyed color in at a cutout's edge.
    #[param]
    pub fix_alpha_border: bool,
    /// How far -FixAlphaBorder carries color, in texels; every texel with
    /// alpha 0 is colored when omitted.
    #[param]
    pub border_reach: Option<u32>,
    /// Rebuilds each normal's Z, in blue, from its X and Y, in red and
    /// green; the -Format must have blue.
    #[param]
    pub reconstruct_z: bool,
    /// Scales each normal to length 1.
    #[param]
    pub normalize: bool,
    /// Flips each normal's green between the convention glTF reads, up,
    /// and DirectX's, down; with -HeightToNormal, points green down in the
    /// normals it makes.
    #[param]
    pub flip_green: bool,
    /// Makes the heights into normals, after the size, orientation and
    /// texel changes: a height step of 1 between neighboring texels gives
    /// a slope of this.
    #[param]
    pub height_to_normal: Option<f64>,
    /// Where -HeightToNormal reads the heights: Red when omitted, Green,
    /// Blue, Alpha or Luminance.
    #[param]
    pub height_channel: Option<HeightChannel>,
    /// The kernel -HeightToNormal takes slopes by: Prewitt, texconv's, when
    /// omitted, Central, Sobel or Sobel5x5.
    #[param]
    pub normal_kernel: Option<NormalKernel>,
    /// What -HeightToNormal reads past the edges: Continue, the slope at
    /// the edge carried on, when omitted; Wrap, Clamp or Mirror.
    #[param]
    pub height_edge: Option<HeightEdge>,
    /// What alpha holds in the normals -HeightToNormal makes: One when
    /// omitted, Height or Occlusion.
    #[param]
    pub normal_alpha: Option<NormalAlpha>,
    /// Keeps normals' X and Y alone where -Format keeps them apart: in red
    /// and green for a format with two channels, such as Bc5RgUnorm, and X
    /// in red, green and blue with Y in alpha for a Basis Universal LDR
    /// format.
    #[param]
    pub two_channel: bool,
    /// Makes a cube map: from a panorama or a layout of the faces, read by
    /// its shape as ConvertTo-WgpuCubeMap reads it, or from a DDS or KTX2
    /// file's square layers, six to a cube.
    #[param]
    pub cube: bool,
    /// How the image -Cube reads holds the faces: needed for a
    /// HorizontalTee, a VerticalCrossTurned, a Grid2x3, a Grid3x2, an
    /// AngularMap or an Octahedral map, which the image's shape does not
    /// tell apart.
    #[param]
    pub layout: Option<CubeLayout>,
    /// Turns a panorama, an angular map or an octahedral map -Cube reads
    /// about the vertical axis by this many degrees, +X toward -Z.
    #[param]
    pub yaw: Option<f64>,
    /// The faces' size in texels for a cube -Cube resamples from a
    /// panorama, an angular map or an octahedral map; a quarter of a
    /// panorama's or an angular map's width, and half an octahedral map's,
    /// when omitted.
    #[param]
    pub face_size: Option<u32>,
}

impl Cmdlet for ImportWgpuImage {
    fn process(&mut self, ps: &Pipeline<'_>) -> PsResult<()> {
        let core = core_for(&self.device, ps)?;
        let files = match (self.path.as_deref(), self.literal_path.as_deref()) {
            (Some(path), _) => input_files(ps, path, false, "WgpuImage")?,
            (None, Some(path)) => input_files(ps, path, true, "WgpuImage")?,
            (None, None) => return Err(invalid("WgpuImage", "give the file as -Path or -LiteralPath")),
        };
        let settings = Settings {
            reading: Reading {
                ignore_orientation: self.ignore_orientation,
                ignore_profile: self.ignore_profile,
                intent: self.rendering_intent,
                dpi: self.dpi,
                no_background: self.no_background,
            },
            format: self.format.as_deref().map(parse_target).transpose()?,
            mip_levels: self.mip_levels,
            mips: MipOptions::given(self.mip_filter, self.mip_edge).with_coverage(self.keep_coverage)?.for_normals(self.normal_mips, self.height_to_normal.is_some()),
            two_channel: self.two_channel,
            quality: self.quality,
            effort: self.effort,
            cube: self.cube,
            layout: self.layout,
            yaw: self.yaw,
            face_size: self.face_size,
            transform: Asked {
                width: self.width,
                height: self.height,
                scale: self.scale,
                max_size: self.max_size,
                power_of_two: self.power_of_two,
                filter: self.filter,
                flip: self.flip,
                rotate: self.rotate,
                crop: self.crop.as_deref(),
                crop_margin: self.crop_margin.as_deref(),
                pad: self.pad.as_deref(),
                anchor: self.anchor,
                pad_color: self.pad_color.as_deref(),
                swizzle: self.swizzle.as_deref(),
                reconstruct_z: self.reconstruct_z,
                normalize: self.normalize,
                flip_green: self.flip_green,
                height_to_normal: self.height_to_normal,
                height_channel: self.height_channel,
                normal_kernel: self.normal_kernel,
                height_edge: self.height_edge,
                normal_alpha: self.normal_alpha,
                premultiply: self.premultiply,
                unpremultiply: self.unpremultiply,
                color_space: self.color_space,
                assign_color_space: self.assign_color_space,
                grayscale: self.grayscale,
                grayscale_weights: self.grayscale_weights.as_ref(),
                invert: self.invert,
                brightness: self.brightness,
                contrast: self.contrast,
                gamma: self.gamma,
                exposure: self.exposure,
                tone_map: self.tone_map,
                color_key: self.color_key.as_ref(),
                color_key_tolerance: self.color_key_tolerance,
                alpha_threshold: self.alpha_threshold,
                fix_alpha_border: self.fix_alpha_border,
                border_reach: self.border_reach,
                filters: filter::Asked {
                    blur: self.blur.as_deref(),
                    sharpen: self.sharpen,
                    sharpen_radius: self.sharpen_radius,
                    sharpen_threshold: self.sharpen_threshold,
                    edge: self.filter_edge,
                    ..Default::default()
                },
            }
            .transform()?,
            ..Default::default()
        };
        let password = match &self.password {
            Some(secret) => Some(Revealed(secret.reveal()?)),
            None => None,
        };
        for_each_file(ps, files, |file| {
            let mut said = Said::default();
            let opened_with = password.as_ref().map(|p| p.0.as_str());
            let texture = image_io::import(&core, Path::new(&file), &settings, self.srgb, opened_with, self.usage.as_deref().map(usage_flags), self.label.clone(), Some(ps), &mut said);
            tell(ps, &said)?;
            ps.write(texture?)
        })
    }
}

/// A password revealed from its SecureString, its text overwritten with
/// zeros when it is dropped.
struct Revealed(String);

impl Drop for Revealed {
    fn drop(&mut self) {
        let mut bytes = std::mem::take(&mut self.0).into_bytes();
        bytes.fill(0);
        std::hint::black_box(&bytes);
    }
}

/// Saves a texture: as a DDS or KTX2 file whole, with every mip level,
/// array layer and cube face, or as any other image file, from mip level 0
/// of one layer, in the format its extension names (or -ImageFormat),
/// converted to what that format stores, a two-channel texture as red and
/// green with blue 0 and alpha 1. An .exr file is compressed with ZIP, in
/// half floats for a texture of 16-bit float texels and in full floats for
/// any other.
///
/// For a DDS or KTX2 file, -Format converts the texture first: into a
/// color format, a BC format encoded on the GPU, or, for KTX2, a Basis
/// Universal format, which Basis Universal encodes. -MipLevels gives it mip
/// levels, rendered on the GPU: each from the level above by the Box
/// filter, or as -MipFilter and -MipEdge say. -Blur and -Sharpen filter the
/// texels after the size, orientation and texel changes, as
/// Convert-WgpuTexture does. -HeightToNormal makes the heights into normals
/// after those, as ConvertTo-WgpuNormalMap does, and -NormalMips keeps
/// normals unit length in the mip levels. -Zstd compresses a KTX2 file's
/// levels; -Cube writes
/// its layers as cube faces. What the export involved goes to the verbose
/// stream.
///
/// A block-compressed texture saved as an image file is decoded first:
/// on the CPU by the module's decoders, or with -Decoder Gpu by the GPU,
/// as a shader reads it.
///
/// -Layout writes a cube map into an image file as ConvertFrom-WgpuCubeMap
/// writes it, its faces in a layout of cells or an angular or octahedral
/// map, and -Panorama as a panorama, with -Yaw and -FaceSize, before the
/// size, orientation and texel changes; the image is in the format the
/// cube's texels decode into.
///
/// # Examples
/// $tex | Export-WgpuImage .\out.png
/// Export-WgpuImage $hdr .\light.exr
/// Export-WgpuImage $tex .\albedo.dds -Format Bc7RgbaUnormSrgb -MipLevels 0
/// Export-WgpuImage $tex .\albedo.ktx2 -Format BasisUastc4x4 -Quality 80
/// Export-WgpuImage $bc7 .\preview.png -Decoder Gpu
/// Export-WgpuImage $height .\normal.ktx2 -HeightToNormal 2 -Format BasisUastc4x4 -TwoChannel -MipLevels 0
/// Export-WgpuImage $cube .\sky.exr -Panorama
/// Export-WgpuImage $cube .\cross.png -Layout HorizontalCross -ToneMap AgX
#[cmdlet(verb = "Export", noun = "WgpuImage", alias = ["Export-GpuImage"])]
#[derive(Default)]
pub struct ExportWgpuImage {
    /// The texture to save.
    #[param(mandatory, position = 0, value_from_pipeline)]
    pub texture: Option<PsProxy<Texture>>,
    /// The file to write.
    #[param(mandatory, position = 1)]
    pub path: String,
    /// The image format, such as Png or Jpeg; taken from the extension when omitted.
    #[param]
    pub image_format: Option<String>,
    /// The array layer to save as an image; 0 when omitted.
    #[param]
    pub layer: Option<u32>,
    /// The format to write a DDS or KTX2 file in: a color format, a BC
    /// format, encoded on the GPU, or for KTX2 a Basis Universal format
    /// such as BasisEtc1s or BasisUastc4x4, which Basis Universal encodes.
    #[param]
    pub format: Option<String>,
    /// Mip levels to write: the texture's own when omitted, and 0 for the
    /// full chain down to 1x1.
    #[param]
    pub mip_levels: Option<u32>,
    /// How each mip level -MipLevels adds, or a transform renders again,
    /// is made: Box, the default, from the level above; Triangle, Kaiser,
    /// Mitchell or Lanczos from level 0; Min or Max, the smallest or
    /// largest value of each channel among the texels it covers.
    #[param]
    pub mip_filter: Option<MipFilter>,
    /// What Triangle, Kaiser, Mitchell and Lanczos read past an edge of
    /// the image: Clamp, the default, nothing; Wrap, the other side, for a
    /// texture that tiles; or Mirror, the image mirrored. A sampler's
    /// ClampToEdge, Repeat and MirrorRepeat name the same.
    #[param]
    pub mip_edge: Option<MipEdge>,
    /// Scales the alpha of each mip level -MipLevels adds, or a transform
    /// renders again, so the share of its texels with alpha at or above
    /// this, the alpha an alpha test passes at, stays level 0's: above 0
    /// and at most 1.
    #[param]
    pub keep_coverage: Option<f64>,
    /// Keeps a normal map's normals unit length in each mip level
    /// -MipLevels adds, or a transform renders again: Renormalize scales
    /// each texel's normal back to length 1, and SlopeSpace filters the
    /// normals' slopes; Renormalize with -HeightToNormal when omitted. It
    /// marks the texels as normals, which a -Format that stores them the
    /// other way takes written again: n / 2 + 1 / 2 in a format that holds
    /// no negative values, and n in any other.
    #[param]
    pub normal_mips: Option<NormalMips>,
    /// Basis Universal's quality, 1 to 100, for a Basis format that has
    /// one; Basis Universal's default when omitted.
    #[param]
    pub quality: Option<u32>,
    /// Encoding effort, 0 to 10: Basis Universal's for a Basis format, and
    /// the preset of Bc7RgbaUnorm, Bc7RgbaUnormSrgb and Bc6hRgbUfloat; the
    /// encoder's default when omitted.
    #[param]
    pub effort: Option<u32>,
    /// Compress each mip level of a KTX2 file in a GPU format with
    /// Zstandard; a Basis format chooses its own supercompression.
    #[param]
    pub zstd: bool,
    /// Write the texture's layers as cube faces: square 2D layers, six to
    /// a cube. A texture made as a cube map is written as one without it.
    #[param]
    pub cube: bool,
    /// Where a block-compressed texture is decoded for an image file: Cpu,
    /// by the module's decoders, or Gpu, as a shader reads it; Cpu when
    /// omitted.
    #[param]
    pub decoder: Option<Decoder>,
    /// Resizes the texture to this width before -Format converts it;
    /// without -Height, the height keeps the aspect ratio.
    #[param]
    pub width: Option<u32>,
    /// Resizes the texture to this height; without -Width, the width keeps
    /// the aspect ratio.
    #[param]
    pub height: Option<u32>,
    /// Resizes both sides by this factor, such as 0.5 for half the size.
    #[param]
    pub scale: Option<f64>,
    /// Shrinks a texture with a side past this many texels to it, keeping
    /// the aspect ratio.
    #[param]
    pub max_size: Option<u32>,
    /// Rounds each side to a power of two: Nearest, Up or Down.
    #[param]
    pub power_of_two: Option<PowerOfTwo>,
    /// The resize filter: Box where a side shrinks and Bicubic where it
    /// grows when omitted.
    #[param]
    pub filter: Option<ResizeFilter>,
    /// Mirrors the texture: Horizontal or Vertical.
    #[param]
    pub flip: Option<FlipDirection>,
    /// Turns the texture clockwise by 90, 180 or 270 degrees.
    #[param]
    pub rotate: Option<u32>,
    /// Keeps a region, four numbers for its left, top, width and height,
    /// or a size -Anchor places, two numbers.
    #[param]
    pub crop: Option<Vec<u32>>,
    /// Cuts this many texels from the left, top, right and bottom.
    #[param]
    pub crop_margin: Option<Vec<u32>>,
    /// Places the texture on a larger canvas of this width and height.
    #[param]
    pub pad: Option<Vec<u32>>,
    /// Where a crop to a size or the padding places the texture; Center
    /// when omitted.
    #[param]
    pub anchor: Option<Anchor>,
    /// The color around the padded texture: red, green, blue and alpha
    /// from 0 to 1; transparent black when omitted.
    #[param]
    pub pad_color: Option<Vec<f64>>,
    /// Rearranges the channels: four letters of r, g, b, a, 0 and 1, one
    /// for each channel, as bgra or rrr1.
    #[param]
    pub swizzle: Option<String>,
    /// Multiplies red, green and blue by alpha, in linear light for an sRGB
    /// texture.
    #[param]
    pub premultiply: bool,
    /// Divides red, green and blue by alpha where alpha is above 0, in
    /// linear light for an sRGB texture.
    #[param]
    pub unpremultiply: bool,
    /// Converts the texels to Linear or Srgb so their colors look the same.
    #[param]
    pub color_space: Option<ColorSpace>,
    /// Relabels the texels as Linear or Srgb without changing them.
    #[param]
    pub assign_color_space: Option<ColorSpace>,
    /// Makes the texture grayscale, with Rec. 709's weights in linear light
    /// or those -GrayscaleWeights names.
    #[param]
    pub grayscale: bool,
    /// The weights -Grayscale uses: Rec709, Rec601 or three numbers for
    /// red, green and blue.
    #[param]
    pub grayscale_weights: Option<PsObject>,
    /// Turns red, green and blue into 1 minus their stored values; alpha is
    /// kept.
    #[param]
    pub invert: bool,
    /// Brightens or darkens the stored values, from -100 to 100, as
    /// ImageMagick's -brightness-contrast.
    #[param]
    pub brightness: Option<f64>,
    /// Raises or lowers the stored values' contrast, from -100 to 100, as
    /// ImageMagick's -brightness-contrast.
    #[param]
    pub contrast: Option<f64>,
    /// Gamma corrects the stored values to value^(1/gamma), as
    /// ImageMagick's -gamma.
    #[param]
    pub gamma: Option<f64>,
    /// Multiplies linear values by 2 to this power: stops of exposure,
    /// applied before -ToneMap.
    #[param]
    pub exposure: Option<f64>,
    /// Maps HDR colors into the range from 0 to 1: Reinhard, AcesFilmic,
    /// AgX or Neutral.
    #[param]
    pub tone_map: Option<ToneMapCurve>,
    /// Gives alpha 0 to the texels of this color, as the texture stores
    /// it: red, green and blue from 0 to 1, rounded to the texture's own
    /// precision, or '#RRGGBB'. Other texels keep their alpha.
    #[param]
    pub color_key: Option<PsObject>,
    /// How far each of red, green and blue may be from -ColorKey's, from 0
    /// to 1; 0, the same stored value, when omitted.
    #[param]
    pub color_key_tolerance: Option<f64>,
    /// Gives alpha 0 to texels whose alpha is below this, and 1 to the
    /// rest, as an alpha test would: above 0 and at most 1.
    #[param]
    pub alpha_threshold: Option<f64>,
    /// Blurs the texels by a Gaussian of this standard deviation in texels,
    /// or of two, across and down, in linear values weighed by alpha.
    #[param]
    pub blur: Option<Vec<f64>>,
    /// Sharpens the texels by an unsharp mask: this many times the
    /// difference between each texel and a Gaussian blur of it is added
    /// back, in linear values weighed by alpha.
    #[param]
    pub sharpen: Option<f64>,
    /// The standard deviation, in texels, of the blur -Sharpen compares
    /// each texel with; 1 when omitted.
    #[param]
    pub sharpen_radius: Option<f64>,
    /// The smallest difference from the blur, from 0 to 1, that -Sharpen
    /// adds back in a channel; 0 when omitted.
    #[param]
    pub sharpen_threshold: Option<f64>,
    /// What -Blur and -Sharpen read past the edge: Clamp, the edge texel
    /// repeated, the default; Wrap, the other side, for an image that
    /// tiles; or Mirror.
    #[param]
    pub filter_edge: Option<FilterEdge>,
    /// Gives each texel with alpha 0 the color of the nearest texel with
    /// alpha above 0, its alpha left at 0, so filtering and mip levels
    /// blend no dark or keyed color in at a cutout's edge.
    #[param]
    pub fix_alpha_border: bool,
    /// How far -FixAlphaBorder carries color, in texels; every texel with
    /// alpha 0 is colored when omitted.
    #[param]
    pub border_reach: Option<u32>,
    /// Rebuilds each normal's Z, in blue, from its X and Y, in red and
    /// green; the -Format must have blue.
    #[param]
    pub reconstruct_z: bool,
    /// Scales each normal to length 1.
    #[param]
    pub normalize: bool,
    /// Flips each normal's green between the convention glTF reads, up,
    /// and DirectX's, down; with -HeightToNormal, points green down in the
    /// normals it makes.
    #[param]
    pub flip_green: bool,
    /// Makes the heights into normals, after the size, orientation and
    /// texel changes: a height step of 1 between neighboring texels gives
    /// a slope of this.
    #[param]
    pub height_to_normal: Option<f64>,
    /// Where -HeightToNormal reads the heights: Red when omitted, Green,
    /// Blue, Alpha or Luminance.
    #[param]
    pub height_channel: Option<HeightChannel>,
    /// The kernel -HeightToNormal takes slopes by: Prewitt, texconv's, when
    /// omitted, Central, Sobel or Sobel5x5.
    #[param]
    pub normal_kernel: Option<NormalKernel>,
    /// What -HeightToNormal reads past the edges: Continue, the slope at
    /// the edge carried on, when omitted; Wrap, Clamp or Mirror.
    #[param]
    pub height_edge: Option<HeightEdge>,
    /// What alpha holds in the normals -HeightToNormal makes: One when
    /// omitted, Height or Occlusion.
    #[param]
    pub normal_alpha: Option<NormalAlpha>,
    /// Keeps normals' X and Y alone where -Format keeps them apart: in red
    /// and green for a format with two channels, such as Bc5RgUnorm, and X
    /// in red, green and blue with Y in alpha for a Basis Universal LDR
    /// format.
    #[param]
    pub two_channel: bool,
    /// Writes a cube map into the image file in this layout of its faces,
    /// or as an AngularMap or an Octahedral map, as ConvertFrom-WgpuCubeMap
    /// writes it.
    #[param]
    pub layout: Option<CubeLayout>,
    /// Writes a cube map into the image file as a panorama, center +X and
    /// left quarter -Z.
    #[param]
    pub panorama: bool,
    /// Turns the panorama, angular map or octahedral map -Layout or
    /// -Panorama writes about the vertical axis by this many degrees, +X
    /// toward -Z, as ConvertTo-WgpuCubeMap reads it.
    #[param]
    pub yaw: Option<f64>,
    /// The size in texels of the faces the panorama, angular map or
    /// octahedral map -Layout or -Panorama writes is made for; the cube's
    /// when omitted.
    #[param]
    pub face_size: Option<u32>,
}

impl Cmdlet for ExportWgpuImage {
    fn process(&mut self, ps: &Pipeline<'_>) -> PsResult<()> {
        let Some(texture) = &self.texture else {
            return Err(invalid("WgpuNoTexture", "no texture was given"));
        };
        let resolved = ps.resolve_path(&self.path, true)?;
        let file = resolved.into_iter().next().ok_or_else(|| invalid("WgpuImage", format!("{} names no file", self.path)))?;
        let format = match &self.image_format {
            Some(name) => Some(
                image::ImageFormat::from_extension(name)
                    .or_else(|| image::ImageFormat::all().find(|f| format!("{f:?}").eq_ignore_ascii_case(name)))
                    .ok_or_else(|| invalid("WgpuImageFormat", format!("{name} is not an image format the image crate writes")))?,
            ),
            None => None,
        };
        let settings = Settings {
            format: self.format.as_deref().map(parse_target).transpose()?,
            mip_levels: self.mip_levels,
            mips: MipOptions::given(self.mip_filter, self.mip_edge).with_coverage(self.keep_coverage)?.for_normals(self.normal_mips, self.height_to_normal.is_some()),
            two_channel: self.two_channel,
            quality: self.quality,
            effort: self.effort,
            zstd: self.zstd,
            cube: self.cube,
            layout: self.layout,
            panorama: self.panorama,
            yaw: self.yaw,
            face_size: self.face_size,
            decoder: self.decoder,
            reading: Reading::default(),
            transform: Asked {
                width: self.width,
                height: self.height,
                scale: self.scale,
                max_size: self.max_size,
                power_of_two: self.power_of_two,
                filter: self.filter,
                flip: self.flip,
                rotate: self.rotate,
                crop: self.crop.as_deref(),
                crop_margin: self.crop_margin.as_deref(),
                pad: self.pad.as_deref(),
                anchor: self.anchor,
                pad_color: self.pad_color.as_deref(),
                swizzle: self.swizzle.as_deref(),
                reconstruct_z: self.reconstruct_z,
                normalize: self.normalize,
                flip_green: self.flip_green,
                height_to_normal: self.height_to_normal,
                height_channel: self.height_channel,
                normal_kernel: self.normal_kernel,
                height_edge: self.height_edge,
                normal_alpha: self.normal_alpha,
                premultiply: self.premultiply,
                unpremultiply: self.unpremultiply,
                color_space: self.color_space,
                assign_color_space: self.assign_color_space,
                grayscale: self.grayscale,
                grayscale_weights: self.grayscale_weights.as_ref(),
                invert: self.invert,
                brightness: self.brightness,
                contrast: self.contrast,
                gamma: self.gamma,
                exposure: self.exposure,
                tone_map: self.tone_map,
                color_key: self.color_key.as_ref(),
                color_key_tolerance: self.color_key_tolerance,
                alpha_threshold: self.alpha_threshold,
                fix_alpha_border: self.fix_alpha_border,
                border_reach: self.border_reach,
                filters: filter::Asked {
                    blur: self.blur.as_deref(),
                    sharpen: self.sharpen,
                    sharpen_radius: self.sharpen_radius,
                    sharpen_threshold: self.sharpen_threshold,
                    edge: self.filter_edge,
                    ..Default::default()
                },
            }
            .transform()?,
        };
        let (core, raw, cube) = texture.with(|t| (t.core.clone(), t.texture.clone(), t.is_cube))?;
        let mut said = Said::default();
        let written = image_io::export(&core, &raw, cube, Path::new(&file), &settings, format, self.layer, Some(ps), &mut said);
        tell(ps, &said)?;
        written
    }
}

/// Creates a sampler: how a shader filters and wraps when it samples a
/// texture.
///
/// # Examples
/// $linear = New-WgpuSampler -Filter Linear
/// New-WgpuSampler -AddressMode Repeat -Anisotropy 16 -Filter Linear
/// New-WgpuSampler -Compare LessEqual
#[cmdlet(verb = "New", noun = "WgpuSampler", alias = ["New-GpuSampler"], output = ["Wgpu.Sampler"])]
#[derive(Default)]
pub struct NewWgpuSampler {
    /// The mag, min and mipmap filters together; Nearest when omitted.
    #[param]
    pub filter: Option<FilterMode>,
    /// The filter when a texel covers more than a pixel.
    #[param]
    pub mag_filter: Option<FilterMode>,
    /// The filter when a texel covers less than a pixel.
    #[param]
    pub min_filter: Option<FilterMode>,
    /// The filter between mip levels.
    #[param]
    pub mipmap_filter: Option<FilterMode>,
    /// The address mode on every axis; ClampToEdge when omitted.
    #[param]
    pub address_mode: Option<AddressMode>,
    /// The address mode across.
    #[param]
    pub address_mode_u: Option<AddressMode>,
    /// The address mode down.
    #[param]
    pub address_mode_v: Option<AddressMode>,
    /// The address mode through a 3D texture.
    #[param]
    pub address_mode_w: Option<AddressMode>,
    /// The lowest mip level of detail; 0 when omitted.
    #[param]
    pub lod_min_clamp: Option<f32>,
    /// The highest mip level of detail; 32 when omitted.
    #[param]
    pub lod_max_clamp: Option<f32>,
    /// Make a comparison sampler with this comparison.
    #[param]
    pub compare: Option<CompareFunction>,
    /// The anisotropic filtering clamp, 1 to 16; 1 when omitted. Above 1 needs every filter Linear.
    #[param]
    pub anisotropy: Option<u16>,
    /// The border color for ClampToBorder.
    #[param]
    pub border_color: Option<BorderColor>,
    /// The sampler's label.
    #[param]
    pub label: Option<String>,
    /// The device to create it on; the default device when omitted.
    #[param]
    pub device: Option<PsProxy<Device>>,
}

impl Cmdlet for NewWgpuSampler {
    fn process(&mut self, ps: &Pipeline<'_>) -> PsResult<()> {
        let core = core_for(&self.device, ps)?;
        let filter = self.filter.unwrap_or(FilterMode::Nearest);
        let mag = self.mag_filter.unwrap_or(filter);
        let min = self.min_filter.unwrap_or(filter);
        let mip = self.mipmap_filter.unwrap_or(filter);
        let address = self.address_mode.unwrap_or(AddressMode::ClampToEdge);
        let (u, v, w) = (self.address_mode_u.unwrap_or(address), self.address_mode_v.unwrap_or(address), self.address_mode_w.unwrap_or(address));
        let anisotropy = self.anisotropy.unwrap_or(1);
        let label = match &self.label {
            Some(l) => l.clone(),
            None => "Wgpu sampler".to_string(),
        };
        let to_filter = |f: FilterMode| match f {
            FilterMode::Nearest => wgpu::FilterMode::Nearest,
            FilterMode::Linear => wgpu::FilterMode::Linear,
        };
        let to_mipmap = |f: FilterMode| match f {
            FilterMode::Nearest => wgpu::MipmapFilterMode::Nearest,
            FilterMode::Linear => wgpu::MipmapFilterMode::Linear,
        };
        let border = self.border_color.map(|b| match b {
            BorderColor::TransparentBlack => wgpu::SamplerBorderColor::TransparentBlack,
            BorderColor::OpaqueBlack => wgpu::SamplerBorderColor::OpaqueBlack,
            BorderColor::OpaqueWhite => wgpu::SamplerBorderColor::OpaqueWhite,
            BorderColor::Zero => wgpu::SamplerBorderColor::Zero,
        });
        let sampler = core.scoped("WgpuSampler", "the sampler cannot be created", || {
            core.device.create_sampler(&wgpu::SamplerDescriptor {
                label: Some(&label),
                address_mode_u: u.wgpu(),
                address_mode_v: v.wgpu(),
                address_mode_w: w.wgpu(),
                mag_filter: to_filter(mag),
                min_filter: to_filter(min),
                mipmap_filter: to_mipmap(mip),
                lod_min_clamp: self.lod_min_clamp.unwrap_or(0.0),
                lod_max_clamp: self.lod_max_clamp.unwrap_or(32.0),
                compare: self.compare.map(CompareFunction::wgpu),
                anisotropy_clamp: anisotropy,
                border_color: border,
            })
        })?;
        ps.write(Sampler {
            label,
            mag_filter: mag,
            min_filter: min,
            mipmap_filter: mip,
            address_mode_u: u,
            address_mode_v: v,
            address_mode_w: w,
            compare: self.compare,
            anisotropy,
            sampler,
            core,
        })
    }
}

/// Lists every texture format wgpu has, with its block size, channels and
/// number type, the features it needs, and what the adapter can do with
/// it.
///
/// # Examples
/// Get-WgpuTextureFormat
/// Get-WgpuTextureFormat Rgba16Float | Format-List
/// Get-WgpuTextureFormat | Where-Object AllowedUsages -contains StorageBinding
#[cmdlet(verb = "Get", noun = "WgpuTextureFormat", alias = ["Get-GpuTextureFormat"], output = ["Wgpu.TextureFormatInfo"])]
#[derive(Default)]
pub struct GetWgpuTextureFormat {
    /// Only the formats whose name contains this text.
    #[param(position = 0)]
    pub name: Option<String>,
    /// The device whose adapter's capabilities to report; the default device when omitted.
    #[param]
    pub device: Option<PsProxy<Device>>,
}

impl Cmdlet for GetWgpuTextureFormat {
    fn process(&mut self, ps: &Pipeline<'_>) -> PsResult<()> {
        let core = core_for(&self.device, ps)?;
        let needle = self.name.as_deref().map(names::fold);
        for format in all_formats() {
            if needle.as_deref().is_some_and(|n| !names::fold(&format_name(format)).contains(n)) {
                continue;
            }
            ps.write(describe_format(&core.adapter, format))?;
        }
        Ok(())
    }
}
