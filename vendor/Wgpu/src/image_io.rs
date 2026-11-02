//! What Import-WgpuImage, Export-WgpuImage and the texture methods that
//! save and convert do: image files through the image crate, DDS and KTX2
//! files whole, and -Format, -MipLevels, -Quality, -Effort and -Zstd,
//! which put a texture in another format, give it mip levels, or encode
//! it in a BC format on the GPU or a Basis Universal format through
//! basisu.
//!
//! Mip levels and conversions render on the GPU in a float format with the
//! texture's channels, then pack into the asked format on the CPU or
//! encode into a BC format on the GPU, so every color format a shader
//! reads as floating point takes them, and 8-bit levels are rounded once.

use std::path::Path;

use half::f16;
use pwrs::prelude::*;
use wgpu::TextureFormat as F;

use crate::basis::{self, flags, Image, Pixels};
use crate::color;
use crate::convert::{self, MipOptions};
use crate::cube::{self, CubeLayout};
use crate::device::CoreRef;
use crate::error::invalid;
use crate::image_decode::{self, Reading};
use crate::names::feature_names;
use crate::normal::{self, HeightToNormal};
use crate::tasks::Interrupt;
use crate::texel;
use crate::texfile::{self, MipsMade, Said, TextureFile};
use crate::texture::{self, format_name, parse_format, Decoder, Texture, TextureDimension, TextureSpec};
use crate::transform::{self, Packed, Transform};

/// A Basis Universal encoding -Format names: basisu's code for it (its
/// BTF_ value), the name, whether it is HDR, and whether it has a quality
/// setting.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct BasisFormat {
    pub(crate) code: u32,
    pub(crate) name: &'static str,
    pub(crate) hdr: bool,
    pub(crate) quality: bool,
}

const fn basis(code: u32, name: &'static str, hdr: bool, quality: bool) -> BasisFormat {
    BasisFormat { code, name, hdr, quality }
}

/// Every encoding basisu writes, in its own order. UASTC HDR 4x4 and the
/// ASTC LDR formats have no quality setting.
pub(crate) const BASIS_FORMATS: [BasisFormat; 34] = [
    basis(0, "BasisEtc1s", false, true),
    basis(1, "BasisUastc4x4", false, true),
    basis(2, "BasisUastcHdr4x4", true, false),
    basis(3, "BasisAstcHdr6x6", true, true),
    basis(4, "BasisUastcHdr6x6", true, true),
    basis(5, "BasisXuastc4x4", false, true),
    basis(6, "BasisXuastc5x4", false, true),
    basis(7, "BasisXuastc5x5", false, true),
    basis(8, "BasisXuastc6x5", false, true),
    basis(9, "BasisXuastc6x6", false, true),
    basis(10, "BasisXuastc8x5", false, true),
    basis(11, "BasisXuastc8x6", false, true),
    basis(12, "BasisXuastc10x5", false, true),
    basis(13, "BasisXuastc10x6", false, true),
    basis(14, "BasisXuastc8x8", false, true),
    basis(15, "BasisXuastc10x8", false, true),
    basis(16, "BasisXuastc10x10", false, true),
    basis(17, "BasisXuastc12x10", false, true),
    basis(18, "BasisXuastc12x12", false, true),
    basis(19, "BasisAstc4x4", false, false),
    basis(20, "BasisAstc5x4", false, false),
    basis(21, "BasisAstc5x5", false, false),
    basis(22, "BasisAstc6x5", false, false),
    basis(23, "BasisAstc6x6", false, false),
    basis(24, "BasisAstc8x5", false, false),
    basis(25, "BasisAstc8x6", false, false),
    basis(26, "BasisAstc10x5", false, false),
    basis(27, "BasisAstc10x6", false, false),
    basis(28, "BasisAstc8x8", false, false),
    basis(29, "BasisAstc10x8", false, false),
    basis(30, "BasisAstc10x10", false, false),
    basis(31, "BasisAstc12x10", false, false),
    basis(32, "BasisAstc12x12", false, false),
    basis(33, "BasisXubc7", false, true),
];

/// What -Format names: a GPU format, or a Basis Universal encoding.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Target {
    Gpu(F),
    Basis(BasisFormat),
}

/// The -Format a name gives.
pub(crate) fn parse_target(name: &str) -> PsResult<Target> {
    if let Some(found) = BASIS_FORMATS.iter().find(|b| b.name.eq_ignore_ascii_case(name)) {
        return Ok(Target::Basis(*found));
    }
    if name.get(..5).is_some_and(|prefix| prefix.eq_ignore_ascii_case("basis")) {
        let names: Vec<&str> = BASIS_FORMATS.iter().map(|b| b.name).collect();
        return Err(invalid("WgpuUnknownFormat", format!("{name} is not a Basis Universal format; they are {}", names.join(", "))));
    }
    Ok(Target::Gpu(parse_format(name)?))
}

/// -Format, -MipLevels, -MipFilter, -MipEdge, -KeepCoverage, -NormalMips,
/// -TwoChannel, -Quality, -Effort, -Zstd, -Cube, -Layout, -Panorama, -Yaw,
/// -FaceSize and -Decoder, as a call gives them, what an import does with an
/// image file's orientation and color profile, and the size and orientation
/// it asks for.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct Settings {
    /// What an import does with an image file's orientation and color
    /// profile.
    pub(crate) reading: Reading,
    pub(crate) format: Option<Target>,
    pub(crate) mip_levels: Option<u32>,
    pub(crate) mips: MipOptions,
    /// Normals' X and Y alone, laid out for the format.
    pub(crate) two_channel: bool,
    pub(crate) quality: Option<u32>,
    pub(crate) effort: Option<u32>,
    pub(crate) zstd: bool,
    /// Import's -Cube, a cube made from the file, or Export's, the layers
    /// written as a cube's faces.
    pub(crate) cube: bool,
    /// How one image holds a cube's faces.
    pub(crate) layout: Option<CubeLayout>,
    /// A cube written as a panorama.
    pub(crate) panorama: bool,
    /// Degrees a panorama, an angular map or an octahedral map turns about
    /// the vertical axis, +X toward -Z.
    pub(crate) yaw: Option<f64>,
    /// The faces' size a projection is resampled for.
    pub(crate) face_size: Option<u32>,
    pub(crate) decoder: Option<Decoder>,
    pub(crate) transform: Transform,
}

impl Settings {
    /// Refuses a setting that does nothing for the format it goes with, a
    /// mip option that changes nothing whatever the file, and a rendering
    /// intent beside -IgnoreProfile, which converts nothing. -NormalMips
    /// alone beside a -Format marks the texels as normals, whose use the
    /// file decides: its format may store them as the -Format does.
    pub(crate) fn check(&self) -> PsResult<()> {
        if self.reading.ignore_profile && self.reading.intent.is_some() {
            return Err(invalid("WgpuRenderingIntent", "-RenderingIntent chooses how colors convert from a file's color profile, and -IgnoreProfile converts none: give one or the other"));
        }
        if let Some(dpi) = self.reading.dpi
            && !(dpi.is_finite() && dpi > 0.0)
        {
            return Err(invalid("WgpuDpi", format!("-Dpi is a number of dots per inch above 0, not {dpi}")));
        }
        let marks_normals = self.format.is_some() && self.mips.names_normals_alone();
        if self.mips.named() && !marks_normals && self.mip_levels.is_none() && self.transform.is_empty() {
            return Err(invalid(
                "WgpuMipFilter",
                "-MipFilter, -MipEdge, -KeepCoverage and -NormalMips choose how the mip levels -MipLevels adds and a transform renders again are made, and the call gives neither: give -MipLevels",
            ));
        }
        if self.mips.named() && !marks_normals && self.mip_levels == Some(1) {
            return Err(invalid("WgpuMipFilter", "-MipFilter, -MipEdge, -KeepCoverage and -NormalMips choose how the levels below level 0 are made, and -MipLevels 1 keeps level 0 alone"));
        }
        self.mips.check()?;
        if self.two_channel {
            two_channel_layout(self.format)?;
        }
        if self.transform.normal.is_some()
            && let Some(Target::Gpu(f)) = self.format
            && f.is_srgb()
        {
            return Err(invalid(
                "WgpuNormalMap",
                format!("-HeightToNormal makes directions, which {} would encode as sRGB colors: give its linear form, {}", format_name(f), format_name(f.remove_srgb_suffix())),
            ));
        }
        if let Some(quality) = self.quality {
            match self.format {
                Some(Target::Basis(b)) if b.quality => {
                    if !(1..=100).contains(&quality) {
                        return Err(invalid("WgpuQuality", format!("-Quality runs from 1 to 100, not {quality}")));
                    }
                }
                Some(Target::Basis(b)) => {
                    return Err(invalid("WgpuQuality", format!("{} has no quality setting, as BasisUastcHdr4x4 and the BasisAstc formats have none; -Quality applies to the other Basis formats", b.name)));
                }
                _ => return Err(invalid("WgpuQuality", "-Quality applies to a Basis Universal -Format with a quality setting")),
            }
        }
        if let Some(effort) = self.effort {
            if effort > 10 {
                return Err(invalid("WgpuEffort", format!("-Effort runs from 0 to 10, not {effort}")));
            }
            match self.format {
                Some(Target::Basis(_)) => {}
                Some(Target::Gpu(format)) => convert::check_effort(format, Some(effort))?,
                None => return Err(invalid("WgpuEffort", "-Effort applies with a -Format to encode in: a Basis Universal format, Bc7RgbaUnorm, Bc7RgbaUnormSrgb or Bc6hRgbUfloat")),
            }
        }
        Ok(())
    }
}

/// Where -TwoChannel puts a normal's X and Y.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum TwoChannel {
    /// In red and green, for a format with those two channels alone.
    RedGreen,
    /// X in red, green and blue and Y in alpha, for a Basis Universal LDR
    /// format, as ktx create --normal-mode stores them.
    ColorAlpha,
}

/// Where -TwoChannel puts X and Y for the -Format `format` names: in red
/// and green for a format with two channels, and X in red, green and blue
/// with Y in alpha for a Basis Universal LDR format. Any other format, or
/// none, is refused.
pub(crate) fn two_channel_layout(format: Option<Target>) -> PsResult<TwoChannel> {
    match format {
        Some(Target::Gpu(f)) if f.components() == 2 => Ok(TwoChannel::RedGreen),
        Some(Target::Basis(b)) if !b.hdr => Ok(TwoChannel::ColorAlpha),
        Some(Target::Basis(b)) => Err(invalid("WgpuNormalMap", format!("-TwoChannel lays X and Y out for a format with two channels or a Basis Universal LDR format, and {} is HDR", b.name))),
        Some(Target::Gpu(f)) => Err(invalid(
            "WgpuNormalMap",
            format!(
                "-TwoChannel keeps X and Y apart for a format with two channels, such as Rg8Unorm or Bc5RgUnorm, or a Basis Universal LDR format, which takes X in red, green and blue and Y in alpha; {} is neither",
                format_name(f)
            ),
        )),
        None => Err(invalid("WgpuNormalMap", "-TwoChannel lays X and Y out for the -Format they are written in: give one with two channels, such as Bc5RgUnorm, or a Basis Universal LDR format")),
    }
}

/// The format normals written for a -Format of `format` are stored as it
/// stores them: the format itself, and for a Basis Universal format one
/// that holds no negative values, as no Basis format does.
fn normal_target(format: Option<Target>) -> Option<F> {
    match format {
        Some(Target::Gpu(f)) => Some(f),
        Some(Target::Basis(b)) => Some(if b.hdr { F::Rgba16Unorm } else { F::Rgba8Unorm }),
        None => None,
    }
}

/// How a transform packs texels a -Format of `format` converts next: into
/// the texture's own format without one, into a GPU format's uncompressed
/// form, and into the format they decode into for Basis Universal.
fn packed_for(format: Option<Target>) -> Packed {
    match format {
        None => Packed::Own,
        Some(Target::Gpu(f)) => Packed::For(f),
        Some(Target::Basis(_)) => Packed::Decoded,
    }
}

/// The -Format `format` in the color space a transform settled: where
/// -ColorSpace switched the uncompressed form of a GPU format the texels
/// were packed in, `packed`, to its other form, the format switches with
/// it.
fn settled(format: Option<Target>, packed: F) -> Option<Target> {
    match format {
        Some(Target::Gpu(f)) => Some(Target::Gpu(match (color::forms(f), color::forms(packed)) {
            (Some((linear, encoded)), Some(_)) if packed.is_srgb() != f.is_srgb() => {
                if packed.is_srgb() {
                    encoded
                } else {
                    linear
                }
            }
            _ => f,
        })),
        other => other,
    }
}

/// What a path's extension says the file is.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    Dds,
    Ktx2,
    Image,
}

fn kind(path: &Path) -> Kind {
    match path.extension().map(|e| e.to_string_lossy().to_ascii_lowercase()).as_deref() {
        Some("dds") => Kind::Dds,
        Some("ktx2") => Kind::Ktx2,
        _ => Kind::Image,
    }
}

/// The formats an image file's pixels load straight into.
const IMAGE_FORMATS: [F; 12] = [
    F::Rgba8Unorm,
    F::Rgba8UnormSrgb,
    F::Bgra8Unorm,
    F::Bgra8UnormSrgb,
    F::R8Unorm,
    F::Rg8Unorm,
    F::R16Unorm,
    F::Rg16Unorm,
    F::Rgba16Unorm,
    F::Rgba16Float,
    F::R32Float,
    F::Rgba32Float,
];

/// An image file's pixels as one level of one layer: in `format` when
/// given, and otherwise Rgba16Float for an HDR file's light, Rgba32Float
/// for a float image, Rgba16Unorm for a 16-bit one (Rgba32Float when it is
/// `converting`, as a conversion reads it in a shader, which reads
/// Rgba16Unorm only with the 16-bit norm feature), and Rgba8Unorm, or
/// Rgba8UnormSrgb with `srgb`, for the rest. A two-channel format takes
/// red and green, and for a texture of `two_channel` texels a gray image
/// gives its gray in red and its alpha in green. The pixels are turned by
/// the file's orientation and converted from its color profile as
/// `reading` asks, or a vector file is drawn as `transform` plans for it
/// and opened with `password`; with whether it was drawn so.
#[allow(clippy::too_many_arguments)]
fn load_image(
    path: &Path,
    format: Option<F>,
    srgb: bool,
    converting: bool,
    two_channel: bool,
    reading: Reading,
    transform: &Transform,
    password: Option<&str>,
    said: &mut Said,
) -> PsResult<(TextureFile, bool)> {
    let opened = image_decode::open(path, reading, transform, password, said)?;
    let drawn = opened.drawn;
    let image = opened.image;
    let gray = matches!(image.color(), image::ColorType::L8 | image::ColorType::La8 | image::ColorType::L16 | image::ColorType::La16);
    let image = if two_channel && gray { gray_in_red_green(&image) } else { image };
    let format = match format {
        Some(f) => f,
        None if opened.hdr => F::Rgba16Float,
        None => match image.color() {
            image::ColorType::Rgb32F | image::ColorType::Rgba32F => F::Rgba32Float,
            image::ColorType::L16 | image::ColorType::La16 | image::ColorType::Rgb16 | image::ColorType::Rgba16 if converting => F::Rgba32Float,
            image::ColorType::L16 | image::ColorType::La16 | image::ColorType::Rgb16 | image::ColorType::Rgba16 => F::Rgba16Unorm,
            _ if srgb => F::Rgba8UnormSrgb,
            _ => F::Rgba8Unorm,
        },
    };
    let bytes: Vec<u8> = match format {
        F::Rgba8Unorm | F::Rgba8UnormSrgb => image.to_rgba8().into_raw(),
        F::Bgra8Unorm | F::Bgra8UnormSrgb => {
            let mut raw = image.to_rgba8().into_raw();
            for px in raw.as_chunks_mut::<4>().0 {
                px.swap(0, 2);
            }
            raw
        }
        F::R8Unorm => image.to_luma8().into_raw(),
        F::Rg8Unorm => image.to_rgba8().into_raw().as_chunks::<4>().0.iter().flat_map(|p| [p[0], p[1]]).collect(),
        F::R16Unorm => bytemuck::cast_slice(&image.to_luma16().into_raw()).to_vec(),
        F::Rg16Unorm => {
            let pairs: Vec<u16> = image.to_rgba16().into_raw().as_chunks::<4>().0.iter().flat_map(|p| [p[0], p[1]]).collect();
            bytemuck::cast_slice(&pairs).to_vec()
        }
        F::Rgba16Unorm => bytemuck::cast_slice(&image.to_rgba16().into_raw()).to_vec(),
        F::Rgba16Float => image.to_rgba32f().into_raw().iter().flat_map(|v| f16::from_f32(*v).to_le_bytes()).collect(),
        F::R32Float => bytemuck::cast_slice(&image.to_luma32f().into_raw()).to_vec(),
        F::Rgba32Float => bytemuck::cast_slice(&image.to_rgba32f().into_raw()).to_vec(),
        other => return Err(invalid("WgpuImageFormat", format!("an image file's pixels do not load straight into {}", format_name(other)))),
    };
    Ok((TextureFile { dimension: wgpu::TextureDimension::D2, width: image.width(), height: image.height(), depth: 1, layers: 1, cube: false, format, levels: vec![bytes] }, drawn))
}

/// A gray image as a color one with its gray in red and its alpha in
/// green, blue 0 and alpha opaque, at 16 bits where the image has them.
fn gray_in_red_green(image: &image::DynamicImage) -> image::DynamicImage {
    use image::{DynamicImage as D, ImageBuffer, Rgba};
    match image.color() {
        image::ColorType::L16 | image::ColorType::La16 => {
            let gray = image.to_luma_alpha16();
            D::ImageRgba16(ImageBuffer::from_fn(gray.width(), gray.height(), |x, y| {
                let p = gray.get_pixel(x, y);
                Rgba([p[0], p[1], 0, u16::MAX])
            }))
        }
        _ => {
            let gray = image.to_luma_alpha8();
            D::ImageRgba8(ImageBuffer::from_fn(gray.width(), gray.height(), |x, y| {
                let p = gray.get_pixel(x, y);
                Rgba([p[0], p[1], 0, u8::MAX])
            }))
        }
    }
}

/// The size `file`'s texture has: depth slices or layers as its third.
fn extent(file: &TextureFile) -> wgpu::Extent3d {
    let third = if file.dimension == wgpu::TextureDimension::D3 { file.depth } else { file.layers };
    wgpu::Extent3d { width: file.width, height: file.height, depth_or_array_layers: third }
}

/// The mip levels `requested` asks a texture of `size` for: 0 for the
/// full chain down to 1x1, and never more than that chain.
fn level_count(size: wgpu::Extent3d, dimension: wgpu::TextureDimension, requested: u32) -> PsResult<u32> {
    let full = size.max_mips(dimension);
    match requested {
        0 => Ok(full),
        n if n <= full => Ok(n),
        n => Err(invalid("WgpuMipLevels", format!("a {}x{} texture has {full} mip levels down to 1x1, not {n}", size.width, size.height))),
    }
}

/// A texture holding `file`, every level and layer written, with `usage`,
/// or every usage its format allows when `None`.
pub(crate) fn upload(core: &CoreRef, file: &TextureFile, usage: Option<wgpu::TextureUsages>, label: String, notes: Vec<String>) -> PsResult<Texture> {
    let size = extent(file);
    let spec = TextureSpec {
        width: size.width,
        height: size.height,
        depth_or_layers: size.depth_or_array_layers,
        format: file.format,
        mip_levels: file.levels.len() as u32,
        sample_count: 1,
        dimension: TextureDimension::from_wgpu(file.dimension),
        usage: None,
        view_formats: Vec::new(),
        label: Some(label),
    };
    let mut made = match usage {
        Some(flags) => texture::create_with_usage(core, &spec, flags)?,
        None => texture::create(core, &spec)?,
    };
    for (mip, level) in file.levels.iter().enumerate() {
        let len = file.layer_len(mip);
        for layer in 0..file.layers as usize {
            texture::write_texels(core, &made.texture, mip as u32, layer as u32, wgpu::TextureAspect::All, &level[layer * len..(layer + 1) * len])?;
        }
    }
    made.is_cube = file.cube;
    made.notes = notes;
    Ok(made)
}

/// Refuses `format` when it is planar, as a command that reads whole
/// texels does: its planes are read one at a time.
fn not_planar(format: F) -> PsResult<()> {
    if format.is_multi_planar_format() {
        return Err(invalid(
            "WgpuAspect",
            format!("{} keeps its texels in planes, which are read one at a time: Read-WgpuTexture reads its luma with -Aspect Plane0 and its chroma with -Aspect Plane1", format_name(format)),
        ));
    }
    Ok(())
}

/// Every layer of mip level `level` of `texture`, read back from the GPU:
/// for a 3D texture, every depth slice. Refused for a planar texture,
/// whose planes are read one at a time.
pub(crate) fn read_level(core: &CoreRef, texture: &wgpu::Texture, level: u32, stop: Option<&dyn Interrupt>) -> PsResult<Vec<u8>> {
    not_planar(texture.format())?;
    let layers = if texture.dimension() == wgpu::TextureDimension::D3 { 1 } else { texture.depth_or_array_layers() };
    let mut out = Vec::new();
    for layer in 0..layers {
        out.extend_from_slice(&texture::read_texels(core, texture, level, layer, wgpu::TextureAspect::All, stop)?);
    }
    Ok(out)
}

/// The usages in `flags`, by their names, joined with "and".
fn usage_names(flags: wgpu::TextureUsages) -> String {
    texture::usage_list(flags).iter().map(|u| format!("{u:?}")).collect::<Vec<_>>().join(" and ")
}

/// Refuses `texture` unless it has every usage in `needed`, which `what`
/// takes.
fn require_usage(texture: &wgpu::Texture, needed: wgpu::TextureUsages, what: &str) -> PsResult<()> {
    let missing = needed - texture.usage();
    if missing.is_empty() {
        return Ok(());
    }
    Err(invalid("WgpuTextureUsage", format!("{what}: the texture needs {} among its usages", usage_names(missing))))
}

/// The first `levels` mip levels of `texture`, every layer, read back from
/// the GPU as a file holds them.
pub(crate) fn read_file(core: &CoreRef, texture: &wgpu::Texture, cube: bool, levels: u32, stop: Option<&dyn Interrupt>) -> PsResult<TextureFile> {
    require_usage(texture, wgpu::TextureUsages::COPY_SRC, "the module reads a texture back from the GPU to save or convert it")?;
    let volume = texture.dimension() == wgpu::TextureDimension::D3;
    let mut read = Vec::with_capacity(levels as usize);
    for level in 0..levels {
        read.push(read_level(core, texture, level, stop)?);
    }
    Ok(TextureFile {
        dimension: texture.dimension(),
        width: texture.width(),
        height: texture.height(),
        depth: if volume { texture.depth_or_array_layers() } else { 1 },
        layers: if volume { 1 } else { texture.depth_or_array_layers() },
        cube,
        format: texture.format(),
        levels: read,
    })
}

/// Mip level `level` of `texture`, every layer, read back from the GPU as
/// a file of that level alone holds it.
pub(crate) fn read_level_file(core: &CoreRef, texture: &wgpu::Texture, cube: bool, level: u32, stop: Option<&dyn Interrupt>) -> PsResult<TextureFile> {
    require_usage(texture, wgpu::TextureUsages::COPY_SRC, "the module reads a texture back from the GPU to save or convert it")?;
    let size = texture.size().mip_level_size(level, texture.dimension());
    let volume = texture.dimension() == wgpu::TextureDimension::D3;
    Ok(TextureFile {
        dimension: texture.dimension(),
        width: size.width,
        height: size.height,
        depth: if volume { size.depth_or_array_layers } else { 1 },
        layers: if volume { 1 } else { size.depth_or_array_layers },
        cube,
        format: texture.format(),
        levels: vec![read_level(core, texture, level, stop)?],
    })
}

/// Mip levels `first` to `last` as a note names them, with the verb that
/// agrees: "mip level 0 was", "mip levels 1 to 3 were".
fn span(first: u32, last: u32) -> String {
    if first == last { format!("mip level {first} was") } else { format!("mip levels {first} to {last} were") }
}

/// `file` as `format` and `levels` ask: in `format`, its own when `None`,
/// with `levels` mip levels, its own count when `None` and the full chain
/// for 0. The levels it has in the asked format are kept as they are. The
/// rest are drawn from its levels in the float format its texels render
/// in, rendered as `mips` says, and packed into the format on the CPU, or
/// encoded on the GPU for a BC format; `effort` picks the encoder's preset.
/// When `mips` holds normals and the format stores them the other way from
/// `file`'s, as n itself rather than n / 2 + 1 / 2 or the reverse, every
/// level is written again as the format stores them.
#[allow(clippy::too_many_arguments)]
fn convert_file(
    core: &CoreRef,
    file: TextureFile,
    format: Option<F>,
    levels: Option<u32>,
    mips: MipOptions,
    effort: Option<u32>,
    stop: Option<&dyn Interrupt>,
    said: &mut Said,
) -> PsResult<TextureFile> {
    let count = match levels {
        Some(requested) => level_count(extent(&file), file.dimension, requested)?,
        None => file.levels.len() as u32,
    };
    let target = format.unwrap_or(file.format);
    let have = (file.levels.len() as u32).min(count);
    if target == file.format && have == count {
        let mut file = file;
        file.levels.truncate(count as usize);
        return Ok(file);
    }
    if target.is_compressed() {
        convert::check_encodable(target)?;
    }
    // With normals, a target that stores them the other way from the source
    // takes each one written again as it stores it.
    let rewrite = mips.normals.is_some() && normal::signed(file.format) != normal::signed(target);
    let mips = mips.stored_as(file.format);
    let mut out = if target == file.format { file.levels[..have as usize].to_vec() } else { Vec::new() };
    let first = out.len() as u32;
    let (dimension, height, cube, from) = (file.dimension, file.height, file.cube, file.format);
    let base = texfile::uncompressed(file, said)?;
    if from.is_compressed() {
        said.verbose.push(format!("the {} texels were decoded on the CPU into {}", format_name(from), format_name(base.format)));
    }
    let render = texel::render_format(base.format).ok_or_else(|| {
        invalid(
            "WgpuConvert",
            format!("mip levels and format conversions read texels as floating-point colors, and {} holds integers, depth or stencil, which a box filter cannot average", format_name(base.format)),
        )
    })?;
    if mips.coverage.is_some() && have < count && (target.components() < 4 || render.components() < 4) {
        let without = if target.components() < 4 { target } else { from };
        return Err(invalid("WgpuAlpha", format!("{} has no alpha channel, so -KeepCoverage would change nothing", format_name(without))));
    }
    let work = convert::float_levels(core, &base, render, count, have, mips, stop)?;
    if have < count {
        said.verbose.push(format!("{} rendered on the GPU in {}, {}", span(have, count - 1), format_name(render), mips.said()));
        said.mips = MipsMade::Rendered;
    }
    let (work, render) = if rewrite {
        let rewritten = normals_rewritten(core, &work, render, &base, from, target, stop)?;
        let stored = if normal::signed(target) { "n itself rather than n / 2 + 1 / 2" } else { "n / 2 + 1 / 2 rather than n itself" };
        let which = if count == 1 { "mip level 0".to_string() } else { format!("mip levels 0 to {}", count - 1) };
        said.verbose.push(format!("the normals of {which} were written again on the CPU as {} stores them, {stored}", format_name(target)));
        said.normals_rewritten = true;
        rewritten
    } else {
        (work, render)
    };
    if target.is_compressed() {
        let opaque = base.levels.iter().all(|level| texel::opaque(level, base.format));
        let (encoded, by) = convert::encode_bc(core, &work, target, effort, opaque, first, stop)?;
        said.verbose.push(format!("{} encoded into {} on the GPU by {by}", span(first, count - 1), format_name(target)));
        out.extend(encoded);
    } else {
        for level in first..count {
            let texels = read_level(core, &work, level, stop)?;
            out.push(if target == render { texels } else { texel::pack(&texel::colors(&texels, render)?, target)? });
        }
        if target != render {
            said.verbose.push(format!("{} packed into {} on the CPU", span(first, count - 1), format_name(target)));
        }
    }
    Ok(TextureFile { dimension, width: base.width, height, depth: base.depth, layers: base.layers, cube, format: target, levels: out })
}

/// Every mip level of `work`, a texture in `render` made from `file`, whose
/// texels are normals as `from` stores them, with each normal written again
/// as `target` stores it, in a new texture of a float format with four
/// channels, returned with that format. A format that holds no negative
/// values stores a normal n as n / 2 + 1 / 2 and any other n itself; a
/// normal `from` holds as X and Y alone has its Z rebuilt. Alpha is kept.
fn normals_rewritten(core: &CoreRef, work: &wgpu::Texture, render: F, file: &TextureFile, from: F, target: F, stop: Option<&dyn Interrupt>) -> PsResult<(wgpu::Texture, F)> {
    let wide = if convert::float_renderable(core, F::Rgba32Float) { F::Rgba32Float } else { F::Rgba16Float };
    let (read_signed, write_signed, xy_only) = (normal::signed(from), normal::signed(target), from.components() < 3);
    let count = work.mip_level_count();
    let mut levels = Vec::with_capacity(count as usize);
    for level in 0..count {
        let colors = texel::colors(&read_level(core, work, level, stop)?, render)?;
        let rewritten: Vec<[f32; 4]> = colors
            .iter()
            .map(|c| {
                let mut n = if read_signed { [c[0], c[1], c[2]] } else { [c[0] * 2.0 - 1.0, c[1] * 2.0 - 1.0, c[2] * 2.0 - 1.0] };
                if xy_only {
                    n[2] = (1.0 - n[0] * n[0] - n[1] * n[1]).max(0.0).sqrt();
                }
                let s = if write_signed { n } else { n.map(|v| v * 0.5 + 0.5) };
                [s[0], s[1], s[2], c[3]]
            })
            .collect();
        levels.push(texel::pack(&rewritten, wide)?);
    }
    let rewritten = TextureFile { dimension: file.dimension, width: file.width, height: file.height, depth: file.depth, layers: file.layers, cube: file.cube, format: wide, levels };
    Ok((convert::float_levels(core, &rewritten, wide, count, count, MipOptions::default(), stop)?, wide))
}

/// `file` encoded in `basis` into the bytes of a KTX2 file, with the mip
/// levels `levels` asks for: the file's own count when `None`, the full
/// chain for 0. A 2D LDR texture's levels are rendered on the GPU as `mips`
/// says and handed to basisu; an array, a cube map or an HDR format takes a
/// single level or a full chain, which basisu makes from level 0 with its
/// own filter. With `two_channel` each texel's red, a normal's X, goes into
/// red, green and blue and its green, the normal's Y, into alpha.
#[allow(clippy::too_many_arguments)]
pub(crate) fn encode_basis(
    core: &CoreRef,
    file: TextureFile,
    basis: BasisFormat,
    levels: Option<u32>,
    mips: MipOptions,
    quality: Option<u32>,
    effort: Option<u32>,
    two_channel: bool,
    stop: Option<&dyn Interrupt>,
    said: &mut Said,
) -> PsResult<Vec<u8>> {
    if file.dimension != wgpu::TextureDimension::D2 {
        return Err(invalid("WgpuBasisEncode", "Basis Universal encodes 2D textures, arrays and cube maps, not 1D or 3D ones"));
    }
    let size = extent(&file);
    let full = size.max_mips(wgpu::TextureDimension::D2);
    let count = match levels {
        Some(requested) => level_count(size, wgpu::TextureDimension::D2, requested)?,
        None => (file.levels.len() as u32).min(full),
    };
    let srgb = !basis.hdr && file.format.is_srgb();
    let pixel_format = if basis.hdr {
        F::Rgba32Float
    } else if srgb {
        F::Rgba8UnormSrgb
    } else {
        F::Rgba8Unorm
    };
    let own_levels = !basis.hdr && file.layers == 1;
    if !own_levels && count != 1 && count != full {
        return Err(invalid(
            "WgpuMipLevels",
            format!("{} takes a full mip chain (-MipLevels 0) or a single level for an array, a cube map or an HDR format, not {count} levels", basis.name),
        ));
    }
    let mut flags = flags::THREADED | flags::KTX2_UASTC_ZSTD;
    if srgb {
        flags |= flags::SRGB;
    }
    if !own_levels && count > 1 {
        said.mips = MipsMade::ByBasis;
        flags |= flags::GEN_MIPS_CLAMP;
        let made = if count == 2 { "mip level 1".to_string() } else { format!("mip levels 1 to {}", count - 1) };
        said.verbose.push(format!("Basis Universal made {made} from level 0"));
    }
    if file.layers > 1 {
        flags |= if file.cube { flags::TEXTURE_TYPE_CUBEMAP_ARRAY } else { flags::TEXTURE_TYPE_2D_ARRAY };
    }
    let layers = file.layers as usize;
    let asked = Some(if own_levels { count } else { 1 });
    // Basis Universal stores a normal n as n / 2 + 1 / 2, as Rgba16Unorm
    // does, and an HDR format's float pixels carry those values as they are.
    let mut pixels = if basis.hdr && mips.normals.is_some() {
        let stored = convert_file(core, file, Some(F::Rgba16Unorm), asked, mips, None, stop, said)?;
        let levels = stored.levels.iter().map(|level| level.as_chunks::<2>().0.iter().flat_map(|c| (f32::from(u16::from_le_bytes(*c)) / 65535.0).to_le_bytes()).collect()).collect();
        TextureFile { dimension: stored.dimension, width: stored.width, height: stored.height, depth: stored.depth, layers: stored.layers, cube: stored.cube, format: pixel_format, levels }
    } else {
        convert_file(core, file, Some(pixel_format), asked, mips, None, stop, said)?
    };
    if two_channel {
        for level in &mut pixels.levels {
            for texel in level.as_chunks_mut::<4>().0 {
                *texel = [texel[0], texel[0], texel[0], texel[1]];
            }
        }
        said.verbose.push("each normal's X went into red, green and blue and its Y into alpha, as ktx create --normal-mode stores a two-channel normal map".to_string());
    }
    // One image per mip level of a single layer, or per layer of level 0.
    let mut planes: Vec<(u32, u32, &[u8])> = Vec::new();
    if own_levels {
        for (mip, level) in pixels.levels.iter().enumerate() {
            planes.push(((size.width >> mip).max(1), (size.height >> mip).max(1), &level[..]));
        }
    } else {
        let len = pixels.layer_len(0);
        for layer in 0..layers {
            planes.push((size.width, size.height, &pixels.levels[0][layer * len..(layer + 1) * len]));
        }
    }
    let floats: Vec<Vec<f32>> = if basis.hdr { planes.iter().map(|(_, _, bytes)| bytemuck::pod_collect_to_vec(bytes)).collect() } else { Vec::new() };
    let images: Vec<Image> = planes
        .iter()
        .enumerate()
        .map(|(i, &(width, height, bytes))| Image { width, height, pixels: if basis.hdr { Pixels::RgbaFloat(&floats[i]) } else { Pixels::Rgba8(bytes) } })
        .collect();
    let quality = quality.map_or(-1, |q| q as i32);
    let effort = effort.map_or(-1, |e| e as i32);
    let bytes = basis::encode_ktx2(&images, basis.code, quality, effort, flags, &mut said.warnings)?;
    said.verbose.push(format!("the texture was encoded into {} by Basis Universal", basis.name));
    Ok(bytes)
}

/// The warning a call's mip options take when they changed nothing: no
/// level below level 0 was rendered, as `source`, the file or the texture,
/// had every level the call asked for, or Basis Universal made the levels
/// with its own filter. -NormalMips changed something wherever the
/// conversion wrote normals again.
fn mips_unused(mips: MipOptions, source: &str, said: &mut Said) {
    let mips = if said.normals_rewritten { mips.with_normals(mips.normals, false) } else { mips };
    if !mips.named() {
        return;
    }
    let why = match said.mips {
        MipsMade::Rendered => return,
        MipsMade::NoneRendered => format!("no mip level was rendered, as {source} has every level the call asks for"),
        MipsMade::ByBasis => "Basis Universal made the mip levels with its own filter".to_string(),
    };
    said.warnings.push(format!("{why}, so {} changed nothing", mips.what()));
}

/// `file` in `target`: a GPU format, converted on the GPU and packed on the
/// CPU or encoded there, or a Basis Universal format, which Basis Universal
/// encodes and the module transcodes to the best format the device holds.
pub(crate) fn into_target(core: &CoreRef, file: TextureFile, target: Target, stop: Option<&dyn Interrupt>, said: &mut Said) -> PsResult<TextureFile> {
    match target {
        Target::Basis(basis) => {
            let bytes = encode_basis(core, file, basis, None, MipOptions::default(), None, None, false, stop, said)?;
            texfile::read_ktx2(&bytes, false, core.features, said)
        }
        Target::Gpu(format) => {
            holdable(core, format)?;
            convert_file(core, file, Some(format), None, MipOptions::default(), None, stop, said)
        }
    }
}

/// Refuses a texture of `format` a device with `core`'s features cannot
/// hold.
pub(crate) fn holdable(core: &CoreRef, format: F) -> PsResult<()> {
    let missing = format.required_features().difference(core.features);
    if missing.is_empty() {
        return Ok(());
    }
    Err(invalid("WgpuTextureFormat", format!("this device cannot hold a {} texture: it has no {}", format_name(format), feature_names(missing).join(" or "))))
}

/// The texture an image, DDS or KTX2 file holds, as `settings` ask, with
/// `usage`, or every usage its format allows when `None`. `srgb` reads a
/// DDS or KTX2 file's color as sRGB, and an image file's 8-bit color when
/// no GPU -Format names its encoding. `password` opens a PDF that needs
/// one.
#[allow(clippy::too_many_arguments)]
pub(crate) fn import(
    core: &CoreRef,
    path: &Path,
    settings: &Settings,
    srgb: bool,
    password: Option<&str>,
    usage: Option<wgpu::TextureUsages>,
    label: Option<String>,
    stop: Option<&dyn Interrupt>,
    said: &mut Said,
) -> PsResult<Texture> {
    settings.check()?;
    if !settings.cube && (settings.layout.is_some() || settings.yaw.is_some() || settings.face_size.is_some()) {
        return Err(invalid("WgpuCubeMap", "-Layout, -Yaw and -FaceSize say how -Cube reads a cube from the file: give -Cube"));
    }
    let kind = kind(path);
    if srgb && kind == Kind::Image && matches!(settings.format, Some(Target::Gpu(_))) {
        return Err(invalid(
            "WgpuSrgb",
            "-Srgb applies to an image file loaded without -Format or with a Basis Universal format; a GPU format says whether its color is sRGB, as Bc7RgbaUnormSrgb does",
        ));
    }
    let label = match label {
        Some(l) => l,
        None => path.display().to_string(),
    };
    let read = |what: &str| std::fs::read(path).map_err(|e| invalid("WgpuImage", format!("{} cannot be read as {what}: {e}", path.display())));
    let (file, drawn) = match kind {
        Kind::Dds => {
            settings.reading.unused("a DDS file", password, said);
            (texfile::read_dds(&read("a DDS file")?, srgb, said)?, false)
        }
        Kind::Ktx2 => {
            settings.reading.unused("a KTX2 file", password, said);
            (texfile::read_ktx2(&read("a KTX2 file")?, srgb, core.features, said)?, false)
        }
        Kind::Image => {
            // A GPU format's own encoding says how 8-bit color reads. An
            // image of normals loads in a format that stores them as the
            // image does, n / 2 + 1 / 2 for integers and n for floats, for
            // the conversion to write them as -Format stores them.
            let normals = settings.transform.normal.is_none() && (settings.mips.normals.is_some() || settings.transform.color.writes_normals());
            // An image loads straight into a -Format only when no transform
            // runs first, as a transform reads the image's full range.
            let (straight, srgb, converting) = match settings.format {
                Some(Target::Gpu(format)) if normals => (None, format.is_srgb(), false),
                Some(Target::Gpu(format)) if IMAGE_FORMATS.contains(&format) && settings.transform.is_empty() => (Some(format), false, false),
                Some(Target::Gpu(format)) => (None, format.is_srgb(), true),
                Some(Target::Basis(_)) => (None, srgb, !normals),
                None => (None, srgb, false),
            };
            let two_channel = matches!(settings.format, Some(Target::Gpu(format)) if format.components() == 2);
            load_image(path, straight, srgb, converting, two_channel, settings.reading, &settings.transform, password, said)?
        }
    };
    // The size and orientation change before -Format and -MipLevels
    // convert; with -Format the texels stay uncompressed for it. A vector
    // file was drawn with its crop, rotation, flip and resize.
    let left = if drawn { settings.transform.after_drawing() } else { settings.transform };
    let (file, format) = if left.is_empty() {
        (file, settings.format)
    } else {
        let levels = file.levels.len() as u32;
        let transform = Transform { target: normal_target(settings.format), ..left };
        let file = transform::apply(core, file, levels, settings.mips, &transform, packed_for(settings.format), None, None, stop, said)?;
        let format = settled(settings.format, file.format);
        (file, format)
    };
    let file = if settings.cube { cube_file(core, file, settings, format, stop, said)? } else { file };
    let file = match format {
        Some(Target::Basis(basis)) => {
            let bytes = encode_basis(core, file, basis, settings.mip_levels, settings.mips, settings.quality, settings.effort, settings.two_channel, stop, said)?;
            texfile::read_ktx2(&bytes, false, core.features, said)?
        }
        Some(Target::Gpu(format)) => {
            holdable(core, format)?;
            convert_file(core, file, Some(format), settings.mip_levels, settings.mips, settings.effort, stop, said)?
        }
        None if settings.mip_levels.is_some() => convert_file(core, file, None, settings.mip_levels, settings.mips, None, stop, said)?,
        None => file,
    };
    mips_unused(settings.mips, "the file", said);
    let file = texfile::decode_for(file, core.features, said)?;
    let notes = said.verbose.iter().chain(&said.warnings).cloned().collect();
    upload(core, &file, usage, label, notes)
}

/// `file` as -Cube makes it a cube: square 2D layers, six to a cube,
/// recorded as cube faces, and one image made into a cube as
/// ConvertTo-WgpuCubeMap makes it, with the -Layout, -Yaw and -FaceSize in
/// `settings`, in `format` where it is a GPU format or the file's own; a
/// block-compressed format of the file's this device cannot hold gives the
/// format it decodes into, which a warning says.
fn cube_file(core: &CoreRef, file: TextureFile, settings: &Settings, format: Option<Target>, stop: Option<&dyn Interrupt>, said: &mut Said) -> PsResult<TextureFile> {
    if file.layers > 1 {
        if settings.layout.is_some() || settings.yaw.is_some() || settings.face_size.is_some() {
            return Err(invalid("WgpuCubeMap", format!("-Layout, -Yaw and -FaceSize read a cube from one image, and the file holds {} layers", file.layers)));
        }
        if file.dimension != wgpu::TextureDimension::D2 || file.width != file.height || !file.layers.is_multiple_of(6) {
            return Err(invalid("WgpuCube", format!("cube faces are square 2D layers, six to a cube; a {}x{} file with {} layers is not that", file.width, file.height, file.layers)));
        }
        if !file.cube {
            said.verbose.push(format!("the file's {} square layers were recorded as cube faces", file.layers));
        }
        return Ok(TextureFile { cube: true, ..file });
    }
    let own = match format {
        Some(Target::Gpu(format)) => format,
        _ => file.format,
    };
    let made = cube::cube_of_image(core, file, settings.layout, settings.yaw, settings.face_size, stop, said)?;
    let own = match texfile::decoded_format(own) {
        Some(decoded) if format.is_none() && !core.features.contains(own.required_features()) => {
            said.warnings.push(format!("this device cannot hold a {} texture, so the cube was stored in {}", format_name(own), format_name(decoded)));
            decoded
        }
        _ => own,
    };
    cube::stored(core, made, Target::Gpu(own), stop, said)
}

/// Level 0 of `layer` of a block-compressed `texture`, every depth slice
/// of a 3D one, decoded by `decoder` into `into`, the format
/// `texfile::decoded_format` names, with a line saying where. The GPU
/// reads the blocks in a shader into the float format `into` renders in,
/// and the CPU packs them into `into`.
fn decoded_plane(core: &CoreRef, texture: &wgpu::Texture, into: F, layer: u32, decoder: Decoder, stop: Option<&dyn Interrupt>, said: &mut Said) -> PsResult<Vec<u8>> {
    let from = texture.format();
    let volume = texture.dimension() == wgpu::TextureDimension::D3;
    let blocks = texture::read_texels(core, texture, 0, layer, wgpu::TextureAspect::All, stop)?;
    let file = TextureFile {
        dimension: texture.dimension(),
        width: texture.width(),
        height: texture.height(),
        depth: if volume { texture.depth_or_array_layers() } else { 1 },
        layers: 1,
        cube: false,
        format: from,
        levels: vec![blocks],
    };
    match decoder {
        Decoder::Cpu => {
            let file = texfile::uncompressed(file, said)?;
            said.verbose.push(format!("the {} texels were decoded on the CPU into {}", format_name(from), format_name(into)));
            Ok(file.levels.concat())
        }
        Decoder::Gpu => {
            let render = texel::render_format(into).ok_or_else(|| invalid("WgpuConvert", format!("{} texels render in no float format", format_name(into))))?;
            let work = convert::float_levels(core, &file, render, 1, 1, MipOptions::default(), stop)?;
            let texels = read_level(core, &work, 0, stop)?;
            if render == into {
                said.verbose.push(format!("the {} texels were decoded on the GPU into {}", format_name(from), format_name(into)));
                return Ok(texels);
            }
            said.verbose.push(format!("the {} texels were decoded on the GPU into {} and packed into {} on the CPU", format_name(from), format_name(render), format_name(into)));
            texel::pack(&texel::colors(&texels, render)?, into)
        }
    }
}

/// Refuses a `layer` past the last of `texture`'s layers; a 3D texture
/// has one.
fn check_layer(texture: &wgpu::Texture, layer: u32) -> PsResult<()> {
    let count = if texture.dimension() == wgpu::TextureDimension::D3 { 1 } else { texture.depth_or_array_layers() };
    if layer < count {
        return Ok(());
    }
    let layers = if count == 1 { "single layer".to_string() } else { format!("{count} layers") };
    Err(invalid("WgpuLayer", format!("layer {layer} is past the texture's {layers}")))
}

/// The image `layer` of mip level 0 of a texture holds: a block-compressed
/// texture decoded by `decoder`, on the CPU when `None`.
fn to_image(core: &CoreRef, texture: &wgpu::Texture, layer: u32, decoder: Option<Decoder>, stop: Option<&dyn Interrupt>, said: &mut Said) -> PsResult<image::DynamicImage> {
    check_layer(texture, layer)?;
    let (w, h) = (texture.width(), texture.height());
    let from = texture.format();
    not_planar(from)?;
    let (format, bytes) = match texfile::decoded_format(from) {
        // Signed texels have no image color type.
        Some(into @ (F::R8Snorm | F::Rg8Snorm)) => {
            return Err(invalid(
                "WgpuImageFormat",
                format!(
                    "a {} texture decodes into {}, which has no image form; images come from the unsigned norm and float formats, and a .dds or .ktx2 file holds the texture whole",
                    format_name(from),
                    format_name(into)
                ),
            ));
        }
        Some(into) => (into, decoded_plane(core, texture, into, layer, decoder.unwrap_or(Decoder::Cpu), stop, said)?),
        None => {
            if decoder.is_some() {
                return Err(invalid("WgpuDecoder", format!("-Decoder applies to a block-compressed texture, and {} texels need no decoding", format_name(from))));
            }
            (from, texture::read_texels(core, texture, 0, layer, wgpu::TextureAspect::All, stop)?)
        }
    };
    image_from(core, format, w, h, bytes, stop, said)
}

/// The image `bytes` of a `w` by `h` texture of `format` make, as
/// `image_of` makes it; a packed format, Rg11b10Ufloat, Rgb9e5Ufloat or
/// Rgb10a2Unorm, is rendered on the GPU into Rgba16Float first, as a
/// shader reads it.
fn image_from(core: &CoreRef, format: F, w: u32, h: u32, bytes: Vec<u8>, stop: Option<&dyn Interrupt>, said: &mut Said) -> PsResult<image::DynamicImage> {
    if !matches!(format, F::Rg11b10Ufloat | F::Rgb9e5Ufloat | F::Rgb10a2Unorm) {
        return image_of(format, w, h, bytes);
    }
    let file = TextureFile { dimension: wgpu::TextureDimension::D2, width: w, height: h, depth: 1, layers: 1, cube: false, format, levels: vec![bytes] };
    let file = convert_file(core, file, Some(F::Rgba16Float), None, MipOptions::default(), None, stop, said)?;
    let level = file.levels.into_iter().next().ok_or_else(|| invalid("WgpuImage", "the image has no texels to write"))?;
    image_of(F::Rgba16Float, w, h, level)
}

/// The image `bytes` of a `w` by `h` texture of `format` make.
fn image_of(format: F, w: u32, h: u32, bytes: Vec<u8>) -> PsResult<image::DynamicImage> {
    let shape = |what: &str| invalid("WgpuImageShape", format!("the texels do not make a {w}x{h} {what} image"));
    Ok(match format {
        F::Rgba8Unorm | F::Rgba8UnormSrgb => image::DynamicImage::ImageRgba8(image::RgbaImage::from_raw(w, h, bytes).ok_or_else(|| shape("RGBA"))?),
        F::Bgra8Unorm | F::Bgra8UnormSrgb => {
            let mut raw = bytes;
            for px in raw.as_chunks_mut::<4>().0 {
                px.swap(0, 2);
            }
            image::DynamicImage::ImageRgba8(image::RgbaImage::from_raw(w, h, raw).ok_or_else(|| shape("RGBA"))?)
        }
        F::R8Unorm => image::DynamicImage::ImageLuma8(image::GrayImage::from_raw(w, h, bytes).ok_or_else(|| shape("gray"))?),
        // Two channels are red and green, blue 0 and alpha opaque.
        F::Rg8Unorm => {
            let rgba: Vec<u8> = bytes.as_chunks::<2>().0.iter().flat_map(|p| [p[0], p[1], 0, u8::MAX]).collect();
            image::DynamicImage::ImageRgba8(image::RgbaImage::from_raw(w, h, rgba).ok_or_else(|| shape("RGBA"))?)
        }
        F::R16Unorm => image::DynamicImage::ImageLuma16(image::ImageBuffer::from_raw(w, h, bytemuck::pod_collect_to_vec(&bytes)).ok_or_else(|| shape("16-bit gray"))?),
        F::Rg16Unorm => {
            let pairs: Vec<u16> = bytemuck::pod_collect_to_vec(&bytes);
            let rgba: Vec<u16> = pairs.as_chunks::<2>().0.iter().flat_map(|p| [p[0], p[1], 0, u16::MAX]).collect();
            image::DynamicImage::ImageRgba16(image::ImageBuffer::from_raw(w, h, rgba).ok_or_else(|| shape("16-bit RGBA"))?)
        }
        F::Rgba16Unorm => image::DynamicImage::ImageRgba16(image::ImageBuffer::from_raw(w, h, bytemuck::pod_collect_to_vec(&bytes)).ok_or_else(|| shape("16-bit RGBA"))?),
        F::Rgba16Float => {
            let values: Vec<f32> = bytes.as_chunks::<2>().0.iter().map(|c| f16::from_le_bytes(*c).to_f32()).collect();
            image::DynamicImage::ImageRgba32F(image::ImageBuffer::from_raw(w, h, values).ok_or_else(|| shape("float RGBA"))?)
        }
        F::Rg16Float | F::Rg32Float => {
            let values: Vec<f32> = texel::colors(&bytes, format)?.into_iter().flatten().collect();
            image::DynamicImage::ImageRgba32F(image::ImageBuffer::from_raw(w, h, values).ok_or_else(|| shape("float RGBA"))?)
        }
        F::Rgba32Float => image::DynamicImage::ImageRgba32F(image::ImageBuffer::from_raw(w, h, bytemuck::pod_collect_to_vec(&bytes)).ok_or_else(|| shape("float RGBA"))?),
        F::R16Float | F::R32Float => {
            let values: Vec<f32> = texel::colors(&bytes, format)?.into_iter().flat_map(|c| [c[0], c[0], c[0], 1.0]).collect();
            image::DynamicImage::ImageRgba32F(image::ImageBuffer::from_raw(w, h, values).ok_or_else(|| shape("float gray"))?)
        }
        other => {
            return Err(invalid(
                "WgpuImageFormat",
                format!(
                    "a {} texture has no image form; images come from the unsigned norm and float formats, and a .dds or .ktx2 file holds the texture whole",
                    format_name(other)
                ),
            ))
        }
    })
}

/// The image in the color type `format` stores.
fn fit(format: image::ImageFormat, image: image::DynamicImage) -> image::DynamicImage {
    use image::DynamicImage as D;
    use image::ImageFormat as I;
    match format {
        I::Jpeg => D::ImageRgb8(image.to_rgb8()),
        I::OpenExr => D::ImageRgba32F(image.to_rgba32f()),
        I::Hdr => D::ImageRgb32F(image.to_rgb32f()),
        I::Png | I::Tiff => match image {
            D::ImageRgb32F(_) | D::ImageRgba32F(_) => D::ImageRgba16(image.to_rgba16()),
            other => other,
        },
        I::Farbfeld => D::ImageRgba16(image.to_rgba16()),
        _ => D::ImageRgba8(image.to_rgba8()),
    }
}

/// `format`, or the image format `path`'s extension names.
fn image_format_for(path: &Path, format: Option<image::ImageFormat>) -> PsResult<image::ImageFormat> {
    match format {
        Some(f) => Ok(f),
        None => image::ImageFormat::from_path(path).map_err(|e| invalid("WgpuImageExtension", format!("{} names no image format the module writes: {e}", path.display()))),
    }
}

/// Writes `image` to `path` in `format`, in the color type it stores; an
/// .exr file in half floats when `half`, as for a texture of 16-bit float
/// texels.
fn write_image(path: &Path, format: image::ImageFormat, image: image::DynamicImage, half: bool) -> PsResult<()> {
    if format == image::ImageFormat::OpenExr {
        return write_exr(path, image, half);
    }
    fit(format, image).save_with_format(path, format).map_err(|e| PsError::new(ErrorCategory::WriteError, "WgpuImageWrite", format!("{} cannot be written: {e}", path.display())))
}

/// Writes `image` to `path` as an .exr file: RGBA channels compressed with
/// ZIP in blocks of 16 scan lines, in half floats when `half` and in full
/// floats otherwise.
fn write_exr(path: &Path, image: image::DynamicImage, half: bool) -> PsResult<()> {
    use exr::prelude::{Encoding, SpecificChannels, Vec2, WritableImage};
    let rgba = image.to_rgba32f();
    let width = rgba.width() as usize;
    let size = (width, rgba.height() as usize);
    let texels = rgba.into_raw();
    let at = |p: Vec2<usize>| {
        let i = (p.y() * width + p.x()) * 4;
        (texels[i], texels[i + 1], texels[i + 2], texels[i + 3])
    };
    let written = if half {
        let channels = SpecificChannels::rgba(|p: Vec2<usize>| {
            let (r, g, b, a) = at(p);
            (f16::from_f32(r), f16::from_f32(g), f16::from_f32(b), f16::from_f32(a))
        });
        exr::prelude::Image::from_encoded_channels(size, Encoding::SMALL_LOSSLESS, channels).write().to_file(path)
    } else {
        exr::prelude::Image::from_encoded_channels(size, Encoding::SMALL_LOSSLESS, SpecificChannels::rgba(at)).write().to_file(path)
    };
    written.map_err(|e| PsError::new(ErrorCategory::WriteError, "WgpuImageWrite", format!("{} cannot be written: {e}", path.display())))
}

/// Whether an .exr file holds the texels of `format` exactly in half
/// floats: 16-bit floats, the packed floats with fewer bits, and the
/// block-compressed formats that decode into 16-bit floats.
fn half_floats(format: F) -> bool {
    matches!(texfile::decoded_format(format).unwrap_or(format), F::Rgba16Float | F::Rg16Float | F::R16Float | F::Rg11b10Ufloat | F::Rgb9e5Ufloat)
}

/// Saves mip level 0 of `layer` of a texture as an image file, in the
/// format the extension names, or `format`; a block-compressed texture is
/// decoded by `decoder`, on the CPU when `None`.
#[allow(clippy::too_many_arguments)]
fn export_image(
    core: &CoreRef,
    texture: &wgpu::Texture,
    path: &Path,
    format: Option<image::ImageFormat>,
    layer: Option<u32>,
    decoder: Option<Decoder>,
    stop: Option<&dyn Interrupt>,
    said: &mut Said,
) -> PsResult<()> {
    let format = image_format_for(path, format)?;
    write_image(path, format, to_image(core, texture, layer.unwrap_or(0), decoder, stop, said)?, half_floats(texture.format()))
}

/// Saves mip level 0 of `layer` of a texture, its size and orientation
/// changed as `transform` asks, as an image file, in the format the
/// extension names, or `format`. A block-compressed texture is decoded by
/// `decoder`, on the CPU when `None`, before the change; `cube` says the
/// texture is a cube map, which is not flipped or rotated.
#[allow(clippy::too_many_arguments)]
fn export_transformed_image(
    core: &CoreRef,
    texture: &wgpu::Texture,
    cube: bool,
    path: &Path,
    format: Option<image::ImageFormat>,
    layer: Option<u32>,
    decoder: Option<Decoder>,
    transform: &Transform,
    stop: Option<&dyn Interrupt>,
    said: &mut Said,
) -> PsResult<()> {
    let format = image_format_for(path, format)?;
    if decoder.is_some() && !texture.format().is_compressed() {
        return Err(invalid("WgpuDecoder", format!("-Decoder applies to a block-compressed texture, and {} texels need no decoding", format_name(texture.format()))));
    }
    let layer = layer.unwrap_or(0);
    check_layer(texture, layer)?;
    let top = read_file(core, texture, cube, 1, stop)?;
    // OpenEXR and Radiance HDR files store floats, which hold normals as
    // they are; every other image format stores them as n / 2 + 1 / 2.
    let float = matches!(format, image::ImageFormat::OpenExr | image::ImageFormat::Hdr);
    let transform = Transform { target: Some(if float { F::Rgba32Float } else { F::Rgba8Unorm }), ..*transform };
    let file = transform::apply(core, top, 1, MipOptions::default(), &transform, Packed::Decoded, decoder, None, stop, said)?;
    let (len, at) = (file.layer_len(0), layer as usize);
    let image = image_from(core, file.format, file.width, file.height, file.levels[0][at * len..(at + 1) * len].to_vec(), stop, said)?;
    write_image(path, format, image, half_floats(texture.format()))
}

/// Saves mip level 0 of a cube map as an image file in the layout or
/// projection `settings` names, as ConvertFrom-WgpuCubeMap writes it, in
/// the format the cube's texels decode into, its size and orientation then
/// changed as the transform asks; in the format the extension names, or
/// `format`.
#[allow(clippy::too_many_arguments)]
fn export_cube_image(
    core: &CoreRef,
    texture: &wgpu::Texture,
    path: &Path,
    format: Option<image::ImageFormat>,
    layer: Option<u32>,
    settings: &Settings,
    stop: Option<&dyn Interrupt>,
    said: &mut Said,
) -> PsResult<()> {
    let format = image_format_for(path, format)?;
    let shape = match (settings.layout, settings.panorama) {
        (Some(_), true) => return Err(invalid("WgpuCubeMap", "-Layout and -Panorama each name the image; give one")),
        (Some(layout), false) => cube::shape(layout),
        (None, _) => cube::PANORAMA,
    };
    if layer.is_some() {
        return Err(invalid("WgpuLayer", "-Layout and -Panorama write all six faces into the image; -Layer picks one layer of a texture saved as it is"));
    }
    if settings.decoder.is_some() {
        return Err(invalid("WgpuDecoder", "-Decoder applies to a texture saved as it is; -Layout and -Panorama decode a block-compressed cube's faces on the CPU"));
    }
    cube::check_faces(texture)?;
    let faces = read_level_file(core, texture, true, 0, stop)?;
    let image = cube::image_of_cube(core, faces, shape, settings.yaw, settings.face_size, stop, said)?;
    let own = texfile::decoded_format(texture.format()).unwrap_or(texture.format());
    let image = cube::stored(core, image, Target::Gpu(own), stop, said)?;
    let image = if settings.transform.is_empty() {
        image
    } else {
        // OpenEXR and Radiance HDR files store floats, which hold normals
        // as they are; every other image format stores them as n / 2 + 1 / 2.
        let float = matches!(format, image::ImageFormat::OpenExr | image::ImageFormat::Hdr);
        let transform = Transform { target: Some(if float { F::Rgba32Float } else { F::Rgba8Unorm }), ..settings.transform };
        transform::apply(core, image, 1, MipOptions::default(), &transform, Packed::Decoded, None, None, stop, said)?
    };
    let (texels, width, height) = (image.format, image.width, image.height);
    let bytes = image.levels.into_iter().next().ok_or_else(|| invalid("WgpuImage", "the image has no texels to write"))?;
    write_image(path, format, image_from(core, texels, width, height, bytes, stop, said)?, half_floats(texture.format()))
}

/// `file` as the bytes of a DDS file, or of a KTX2 file whose levels are
/// each compressed with Zstandard at level 6 when `zstd` asks.
fn file_bytes(kind: Kind, file: &TextureFile, zstd: bool) -> PsResult<Vec<u8>> {
    if kind == Kind::Dds { texfile::write_dds(file) } else { texfile::write_ktx2(file, zstd.then_some(6)) }
}

fn write_bytes(path: &Path, bytes: Vec<u8>) -> PsResult<()> {
    std::fs::write(path, bytes).map_err(|e| PsError::new(ErrorCategory::WriteError, "WgpuImageWrite", format!("{} cannot be written: {e}", path.display())))
}

/// Refuses a decoder for a DDS or KTX2 file, which holds a texture's
/// blocks as they are.
fn no_decoder(decoder: Option<Decoder>) -> PsResult<()> {
    if decoder.is_some() {
        return Err(invalid("WgpuDecoder", "-Decoder applies to an image file; a DDS or KTX2 file holds a block-compressed texture's blocks as they are"));
    }
    Ok(())
}

/// Saves a texture: a DDS or KTX2 file whole, its layers written as cube
/// faces when it is a `cube` map; mip level 0 of layer 0 as any other
/// image file, a block-compressed texture decoded by `decoder`, on the
/// CPU when `None`.
pub(crate) fn save(core: &CoreRef, texture: &wgpu::Texture, cube: bool, path: &Path, decoder: Option<Decoder>, stop: Option<&dyn Interrupt>, said: &mut Said) -> PsResult<()> {
    let kind = kind(path);
    if kind == Kind::Image {
        return export_image(core, texture, path, None, None, decoder, stop, said);
    }
    no_decoder(decoder)?;
    let file = read_file(core, texture, cube, texture.mip_level_count(), stop)?;
    write_bytes(path, file_bytes(kind, &file, false)?)
}

/// Saves a texture as `settings` ask: a DDS or KTX2 file whole, its layers
/// written as cube faces when it is a `cube` map or -Cube says so; mip
/// level 0 of `layer` as any other image file, or with `image_format`, a
/// block-compressed texture decoded where -Decoder says.
#[allow(clippy::too_many_arguments)]
pub(crate) fn export(
    core: &CoreRef,
    texture: &wgpu::Texture,
    cube: bool,
    path: &Path,
    settings: &Settings,
    image_format: Option<image::ImageFormat>,
    layer: Option<u32>,
    stop: Option<&dyn Interrupt>,
    said: &mut Said,
) -> PsResult<()> {
    let kind = if image_format.is_some() { Kind::Image } else { kind(path) };
    let cube_image = settings.layout.is_some() || settings.panorama;
    if !cube_image && (settings.yaw.is_some() || settings.face_size.is_some()) {
        return Err(invalid("WgpuCubeMap", "-Yaw and -FaceSize say how -Layout or -Panorama writes a cube into an image file: give one of them"));
    }
    if kind == Kind::Image {
        if settings.format.is_some()
            || settings.mip_levels.is_some()
            || settings.mips.named()
            || settings.two_channel
            || settings.quality.is_some()
            || settings.effort.is_some()
            || settings.zstd
            || settings.cube
        {
            return Err(invalid("WgpuImageFormat", "-Format, -MipLevels, -MipFilter, -MipEdge, -KeepCoverage, -NormalMips, -TwoChannel, -Quality, -Effort, -Zstd and -Cube apply to .dds and .ktx2 files"));
        }
        if cube_image {
            return export_cube_image(core, texture, path, image_format, layer, settings, stop, said);
        }
        if settings.transform.is_empty() {
            return export_image(core, texture, path, image_format, layer, settings.decoder, stop, said);
        }
        return export_transformed_image(core, texture, cube, path, image_format, layer, settings.decoder, &settings.transform, stop, said);
    }
    if cube_image {
        return Err(invalid("WgpuCubeMap", "-Layout and -Panorama write a cube into an image file; a .dds or .ktx2 file holds its faces as they are"));
    }
    settings.check()?;
    no_decoder(settings.decoder)?;
    if layer.is_some() {
        return Err(invalid("WgpuLayer", "a DDS or KTX2 file holds every layer; -Layer applies to other image files"));
    }
    if settings.zstd && (kind == Kind::Dds || matches!(settings.format, Some(Target::Basis(_)))) {
        return Err(invalid("WgpuZstd", "-Zstd applies to a .ktx2 file in a GPU format; a DDS file has no supercompression, and a Basis Universal format chooses its own"));
    }
    if settings.cube {
        texture::check_cube(texture)?;
    }
    let cube = cube || settings.cube;
    let (file, format) = if settings.transform.is_empty() {
        (read_file(core, texture, cube, texture.mip_level_count(), stop)?, settings.format)
    } else {
        // The size and orientation change before -Format and -MipLevels
        // convert; with -Format the texels stay uncompressed for it.
        let top = read_file(core, texture, cube, 1, stop)?;
        let transform = Transform { target: normal_target(settings.format), ..settings.transform };
        let file = transform::apply(core, top, texture.mip_level_count(), settings.mips, &transform, packed_for(settings.format), None, None, stop, said)?;
        let format = settled(settings.format, file.format);
        (file, format)
    };
    let bytes = match format {
        Some(Target::Basis(basis)) => {
            if kind == Kind::Dds {
                return Err(invalid("WgpuTextureFormat", format!("a DDS file cannot hold {}: give a .ktx2 path", basis.name)));
            }
            encode_basis(core, file, basis, settings.mip_levels, settings.mips, settings.quality, settings.effort, settings.two_channel, stop, said)?
        }
        Some(Target::Gpu(format)) => file_bytes(kind, &convert_file(core, file, Some(format), settings.mip_levels, settings.mips, settings.effort, stop, said)?, settings.zstd)?,
        None if settings.mip_levels.is_some() => file_bytes(kind, &convert_file(core, file, None, settings.mip_levels, settings.mips, None, stop, said)?, settings.zstd)?,
        None => file_bytes(kind, &file, settings.zstd)?,
    };
    mips_unused(settings.mips, "the texture", said);
    write_bytes(path, bytes)
}

/// A texture a method converts: its texels, whether it is a cube map, and
/// its label.
pub(crate) struct Source {
    pub(crate) texture: wgpu::Texture,
    pub(crate) cube: bool,
    pub(crate) label: String,
}

/// GenerateMips(): the texture's own mip levels 1 and on, rendered as
/// `mips` says and written in place.
pub(crate) fn fill_mips(core: &CoreRef, source: &Source, mips: MipOptions, stop: Option<&dyn Interrupt>) -> PsResult<()> {
    mips.check()?;
    let texture = &source.texture;
    let count = texture.mip_level_count();
    if count == 1 {
        return Err(invalid("WgpuMipLevels", "GenerateMips() fills a texture's own mip levels, and this one has a single level: GenerateMips(0) returns a copy with the full chain"));
    }
    require_usage(texture, wgpu::TextureUsages::COPY_SRC | wgpu::TextureUsages::COPY_DST, "GenerateMips() reads level 0 back and writes the levels below it")?;
    let top = read_file(core, texture, source.cube, 1, stop)?;
    // Notes record how a texture was made; the fill leaves them as they are.
    let mut said = Said::default();
    let file = convert_file(core, top, None, Some(count), mips, None, stop, &mut said)?;
    for (mip, level) in file.levels.iter().enumerate().skip(1) {
        let len = file.layer_len(mip);
        for layer in 0..file.layers as usize {
            texture::write_texels(core, texture, mip as u32, layer as u32, wgpu::TextureAspect::All, &level[layer * len..(layer + 1) * len])?;
        }
    }
    Ok(())
}

/// The usages a copy of `source` takes in `format`: the source's usages
/// that format allows, and CopyDst, which writing the copy takes. A note
/// says what differs from the source's.
fn copy_usage(core: &CoreRef, source: &wgpu::Texture, format: F, said: &mut Said) -> wgpu::TextureUsages {
    let usage = source.usage();
    let allowed = texture::allowed_usages(core, format);
    let dropped = usage - allowed;
    if !dropped.is_empty() {
        let noun = if texture::usage_list(dropped).len() == 1 { "usage" } else { "usages" };
        said.verbose.push(format!("the new texture leaves out the source's {} {noun}, which {} does not allow", usage_names(dropped), format_name(format)));
    }
    if !usage.contains(wgpu::TextureUsages::COPY_DST) {
        said.verbose.push("the new texture takes CopyDst among its usages, as writing its texels needs it".to_string());
    }
    (usage & allowed) | wgpu::TextureUsages::COPY_DST
}

fn notes(said: Said) -> Vec<String> {
    said.verbose.into_iter().chain(said.warnings).collect()
}

/// GenerateMips(levels): a copy of the texture with `levels` mip levels,
/// the full chain for 0, its own levels kept and the rest rendered as
/// `mips` says.
pub(crate) fn with_mips(core: &CoreRef, source: &Source, levels: u32, mips: MipOptions, stop: Option<&dyn Interrupt>) -> PsResult<Texture> {
    mips.check()?;
    if levels == 1 && mips.named() {
        return Err(invalid("WgpuMipFilter", "a filter, an edge, a coverage and how normals stay unit length choose how the levels below level 0 are made, and GenerateMips(1) keeps level 0 alone"));
    }
    let mut said = Said::default();
    let file = read_file(core, &source.texture, source.cube, source.texture.mip_level_count(), stop)?;
    let file = convert_file(core, file, None, Some(levels), mips, None, stop, &mut said)?;
    mips_unused(mips, "the texture", &mut said);
    let usage = copy_usage(core, &source.texture, file.format, &mut said);
    upload(core, &file, Some(usage), source.label.clone(), notes(said))
}

/// A copy of a texture with its size and orientation changed as
/// `transform` asks, in its format or the one `format` names, with as many
/// of its mip levels as the new size's chain has, rendered again as `mips`
/// says; a format the module has no encoder for, or one the device cannot
/// hold at the new size, gives the format the texels decode into, which a
/// warning says. Mip options on a texture with one level are refused. What
/// making it involved goes to `said` and the copy's Notes.
pub(crate) fn transformed(core: &CoreRef, source: &Source, transform: &Transform, mips: MipOptions, format: Option<Target>, stop: Option<&dyn Interrupt>, said: &mut Said) -> PsResult<Texture> {
    mips.check()?;
    if mips.named() && source.texture.mip_level_count() == 1 {
        return Err(invalid("WgpuMipFilter", "-MipFilter, -MipEdge, -KeepCoverage and -NormalMips choose how a texture's mip levels are rendered again, and this one has level 0 alone"));
    }
    let count = source.texture.mip_level_count();
    // A -Format alone converts every level as it is.
    let (file, format) = if transform.is_empty() {
        (read_file(core, &source.texture, source.cube, count, stop)?, format)
    } else {
        let top = read_file(core, &source.texture, source.cube, 1, stop)?;
        let transform = Transform { target: normal_target(format), ..*transform };
        let file = transform::apply(core, top, count, mips, &transform, packed_for(format), None, None, stop, said)?;
        let format = settled(format, file.format);
        (file, format)
    };
    let file = match format {
        None => file,
        Some(Target::Gpu(f)) => {
            holdable(core, f)?;
            convert_file(core, file, Some(f), None, mips, None, stop, said)?
        }
        Some(Target::Basis(basis)) => {
            let levels = file.levels.len() as u32;
            let bytes = encode_basis(core, file, basis, Some(levels), mips, None, None, false, stop, said)?;
            texfile::read_ktx2(&bytes, false, core.features, said)?
        }
    };
    if transform.is_empty() {
        mips_unused(mips, "the texture", said);
    }
    let file = texfile::decode_for(file, core.features, said)?;
    let usage = copy_usage(core, &source.texture, file.format, said);
    let notes = said.verbose.iter().chain(&said.warnings).cloned().collect();
    upload(core, &file, Some(usage), source.label.clone(), notes)
}

/// A texture of `colors`, `width` by `height` texels, in Rgba8Unorm, or in
/// the format `format` names: packed on the CPU, encoded on the GPU for a
/// BC format, or by Basis Universal for a Basis format and transcoded to
/// the best format the device holds. What making it involved goes to
/// `said` and its Notes.
pub(crate) fn joined(core: &CoreRef, colors: &[[f32; 4]], width: u32, height: u32, format: Option<&str>, stop: Option<&dyn Interrupt>, said: &mut Said) -> PsResult<Texture> {
    let floats: Vec<u8> = colors.iter().flatten().flat_map(|v| v.to_le_bytes()).collect();
    let file = TextureFile { dimension: wgpu::TextureDimension::D2, width, height, depth: 1, layers: 1, cube: false, format: F::Rgba32Float, levels: vec![floats] };
    let file = match format.map(parse_target).transpose()? {
        None => {
            said.verbose.push("the texels were packed into Rgba8Unorm on the CPU".to_string());
            TextureFile { format: F::Rgba8Unorm, levels: vec![texel::pack(colors, F::Rgba8Unorm)?], ..file }
        }
        Some(Target::Basis(basis)) => {
            let bytes = encode_basis(core, file, basis, None, MipOptions::default(), None, None, false, stop, said)?;
            texfile::read_ktx2(&bytes, false, core.features, said)?
        }
        Some(Target::Gpu(target)) => {
            holdable(core, target)?;
            convert_file(core, file, Some(target), None, MipOptions::default(), None, stop, said)?
        }
    };
    let file = texfile::decode_for(file, core.features, said)?;
    let label = format!("{width}x{height} {}", format_name(file.format));
    let notes = said.verbose.iter().chain(&said.warnings).cloned().collect();
    upload(core, &file, None, label, notes)
}

/// ConvertTo-WgpuNormalMap and ToNormalMap(): a normal map of the heights
/// of `source`, made as `step` says, in the format `format` names,
/// Rgba8Unorm when `None`, with X and Y alone where `two_channel` says,
/// and the source's mip levels rendered as `mips` says. What making it
/// involved goes to `said` and its Notes.
#[allow(clippy::too_many_arguments)]
pub(crate) fn normal_map(
    core: &CoreRef,
    source: &Source,
    step: &HeightToNormal,
    format: Option<&str>,
    two_channel: bool,
    mips: MipOptions,
    stop: Option<&dyn Interrupt>,
    said: &mut Said,
) -> PsResult<Texture> {
    let target = match format {
        Some(name) => parse_target(name)?,
        None => Target::Gpu(F::Rgba8Unorm),
    };
    if two_channel {
        two_channel_layout(Some(target))?;
    }
    if let Target::Gpu(f) = target
        && f.is_srgb()
    {
        return Err(invalid(
            "WgpuNormalMap",
            format!("a normal map stores directions, which {} would encode as sRGB colors: give its linear form, {}", format_name(f), format_name(f.remove_srgb_suffix())),
        ));
    }
    mips.check()?;
    let count = source.texture.mip_level_count();
    if mips.named() && count == 1 {
        return Err(invalid("WgpuMipFilter", "-MipFilter, -MipEdge and -NormalMips choose how the normal map's mip levels are rendered, and the height map has level 0 alone"));
    }
    let transform = Transform { normal: Some(*step), target: normal_target(Some(target)), ..Default::default() };
    let file = read_file(core, &source.texture, source.cube, 1, stop)?;
    let file = transform::apply(core, file, count, mips, &transform, Packed::Decoded, None, None, stop, said)?;
    let file = match target {
        Target::Basis(basis) => {
            let bytes = encode_basis(core, file, basis, Some(count), mips, None, None, two_channel, stop, said)?;
            texfile::read_ktx2(&bytes, false, core.features, said)?
        }
        Target::Gpu(f) => {
            holdable(core, f)?;
            convert_file(core, file, Some(f), Some(count), mips, None, stop, said)?
        }
    };
    mips_unused(mips, "the height map", said);
    let file = texfile::decode_for(file, core.features, said)?;
    let notes = said.verbose.iter().chain(&said.warnings).cloned().collect();
    upload(core, &file, None, source.label.clone(), notes)
}

/// Compress(format, quality, effort): a copy of the texture encoded in a
/// BC format on the GPU, or in a Basis Universal format by basisu and then
/// transcoded to the best format the device holds.
pub(crate) fn compressed(core: &CoreRef, source: &Source, format: &str, quality: Option<u32>, effort: Option<u32>, stop: Option<&dyn Interrupt>) -> PsResult<Texture> {
    let target = parse_target(format)?;
    Settings { format: Some(target), quality, effort, ..Default::default() }.check()?;
    let mut said = Said::default();
    let file = read_file(core, &source.texture, source.cube, source.texture.mip_level_count(), stop)?;
    let file = match target {
        Target::Basis(basis) => {
            let bytes = encode_basis(core, file, basis, None, MipOptions::default(), quality, effort, false, stop, &mut said)?;
            texfile::read_ktx2(&bytes, false, core.features, &mut said)?
        }
        Target::Gpu(format) if format.is_compressed() => {
            holdable(core, format)?;
            convert_file(core, file, Some(format), None, MipOptions::default(), effort, stop, &mut said)?
        }
        Target::Gpu(other) => return Err(invalid("WgpuCompressFormat", format!("Compress takes a BC format or a Basis Universal format, not {}", format_name(other)))),
    };
    let usage = copy_usage(core, &source.texture, file.format, &mut said);
    upload(core, &file, Some(usage), source.label.clone(), notes(said))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::convert::{MipEdge, MipFilter};
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

    /// A 2D texture of one layer and one level.
    fn image(format: F, width: u32, height: u32, texels: Vec<u8>) -> TextureFile {
        TextureFile { dimension: wgpu::TextureDimension::D2, width, height, depth: 1, layers: 1, cube: false, format, levels: vec![texels] }
    }

    /// The error `result` holds; the test fails when it holds a value.
    fn refused<T>(result: PsResult<T>) -> PsError {
        match result {
            Ok(_) => panic!("expected a refusal"),
            Err(e) => e,
        }
    }

    #[test]
    fn names_basis_formats_in_basisus_order_and_refuses_others() {
        for (i, format) in BASIS_FORMATS.iter().enumerate() {
            assert_eq!(format.code as usize, i, "{}", format.name);
        }
        assert_eq!(parse_target("basisuastc4x4").expect("named"), Target::Basis(BASIS_FORMATS[1]));
        assert_eq!(parse_target("Bc7RgbaUnormSrgb").expect("named"), Target::Gpu(F::Bc7RgbaUnormSrgb));
        let e = refused(parse_target("BasisAstc3x3"));
        assert!(e.message.contains("BasisEtc1s") && e.message.contains("BasisXubc7"), "{}", e.message);
    }

    #[test]
    fn refuses_settings_that_do_nothing_for_their_format() {
        let basis = |name: &str| Some(parse_target(name).expect("a Basis format"));
        let check = |format: Option<Target>, quality: Option<u32>, effort: Option<u32>| Settings { format, quality, effort, ..Default::default() }.check();
        assert!(check(basis("BasisUastc4x4"), Some(80), Some(5)).is_ok());
        assert!(check(basis("BasisAstc4x4"), None, Some(3)).is_ok());
        assert!(check(Some(Target::Gpu(F::Bc7RgbaUnorm)), None, Some(10)).is_ok());
        assert!(check(Some(Target::Gpu(F::Bc6hRgbUfloat)), None, Some(0)).is_ok());
        for (format, quality, effort, id) in [
            (basis("BasisAstc4x4"), Some(50), None, "WgpuQuality"),
            (basis("BasisUastcHdr4x4"), Some(50), None, "WgpuQuality"),
            (basis("BasisEtc1s"), Some(0), None, "WgpuQuality"),
            (basis("BasisEtc1s"), Some(101), None, "WgpuQuality"),
            (Some(Target::Gpu(F::Bc7RgbaUnorm)), Some(50), None, "WgpuQuality"),
            (Some(Target::Gpu(F::Bc1RgbaUnorm)), None, Some(5), "WgpuEffort"),
            (Some(Target::Gpu(F::Rgba8Unorm)), None, Some(5), "WgpuEffort"),
            (Some(Target::Gpu(F::Bc6hRgbUfloat)), None, Some(11), "WgpuEffort"),
            (None, None, Some(5), "WgpuEffort"),
        ] {
            assert_eq!(refused(check(format, quality, effort)).error_id, id, "{format:?} {quality:?} {effort:?}");
        }
    }

    /// An sRGB texture of black and white columns: level 0 kept byte for
    /// byte, and every texel below it the average of black and white in
    /// linear light, 0.5, which sRGB encodes as 188.
    #[test]
    fn renders_srgb_levels_in_linear_light_and_keeps_level_zero() {
        let Some(core) = core() else { return };
        let texels: Vec<u8> = (0..16).flat_map(|i| if i % 2 == 0 { [0, 0, 0, 255] } else { [255, 255, 255, 255] }).collect();
        let mut said = Said::default();
        let out = convert_file(&core, image(F::Rgba8UnormSrgb, 4, 4, texels.clone()), None, Some(0), MipOptions::default(), None, None, &mut said).expect("converted");
        assert_eq!(out.levels.len(), 3);
        assert_eq!(out.levels[0], texels);
        for level in &out.levels[1..] {
            assert!(level.as_chunks::<4>().0.iter().all(|t| *t == [188, 188, 188, 255]), "{level:?}");
        }
        assert_eq!(said.verbose, ["mip levels 1 to 2 were rendered on the GPU in Rgba16Float, each from the level above it", "mip levels 1 to 2 were packed into Rgba8UnormSrgb on the CPU"]);
    }

    /// Rgb9e5Ufloat, which the GPU does not render into, takes mip levels:
    /// level 1 is the average of level 0, packed on the CPU.
    #[test]
    fn gives_a_format_the_gpu_cannot_render_into_its_levels() {
        let Some(core) = core() else { return };
        let colors = [[1.0, 0.5, 0.0, 1.0], [2.0, 0.5, 0.0, 1.0], [3.0, 0.5, 0.0, 1.0], [6.0, 0.5, 0.0, 1.0]];
        let texels = texel::pack(&colors, F::Rgb9e5Ufloat).expect("packed");
        let out = convert_file(&core, image(F::Rgb9e5Ufloat, 2, 2, texels.clone()), None, Some(0), MipOptions::default(), None, None, &mut Said::default()).expect("converted");
        assert_eq!(out.levels[0], texels);
        assert_eq!(out.levels[1], texel::pack(&[[3.0, 0.5, 0.0, 1.0]], F::Rgb9e5Ufloat).expect("packed"));
    }

    /// A BC1 texture given the rest of its chain keeps its own blocks for
    /// level 0, and every level below decodes to its red.
    #[test]
    fn keeps_the_blocks_a_compressed_file_has() {
        let Some(core) = core() else { return };
        let red = [0x00, 0xF8, 0x1F, 0x00, 0, 0, 0, 0].repeat(4);
        let mut said = Said::default();
        let out = convert_file(&core, image(F::Bc1RgbaUnorm, 8, 8, red.clone()), None, Some(0), MipOptions::default(), None, None, &mut said).expect("converted");
        assert_eq!(out.levels.len(), 4);
        assert_eq!(out.levels[0], red);
        assert!(said.verbose.iter().any(|l| l == "mip levels 1 to 3 were encoded into Bc1RgbaUnorm on the GPU by block_compression"), "{said:?}");
        if core.features.contains(wgpu::Features::TEXTURE_COMPRESSION_BC) {
            // Levels smaller than a block upload and read back whole.
            let made = upload(&core, &out, None, "test".to_string(), Vec::new()).expect("uploaded");
            let back = read_file(&core, &made.texture, false, 4, None).expect("read back");
            assert_eq!(back.levels, out.levels);
        }
        let decoded = texfile::decode_for(out, wgpu::Features::empty(), &mut Said::default()).expect("decoded");
        for level in &decoded.levels {
            for texel in level.as_chunks::<4>().0 {
                assert!(texel.iter().zip([255u8, 0, 0, 255]).all(|(a, b)| a.abs_diff(b) <= 4), "{texel:?}");
            }
        }
    }

    /// BC7 and BC6H texels decode the same on the CPU and on the GPU, as
    /// their specifications define every block's texels exactly; a signed
    /// BC4 texture has no image form, and an uncompressed one takes no
    /// decoder.
    #[test]
    fn decodes_a_compressed_texture_for_an_image_on_the_cpu_and_the_gpu() {
        let Some(core) = core() else { return };
        if !core.features.contains(wgpu::Features::TEXTURE_COMPRESSION_BC) {
            eprintln!("skipped: the device has no TextureCompressionBc");
            return;
        }
        let colors: Vec<[f32; 4]> = (0..64).map(|i| [i as f32 / 63.0, 1.0 - i as f32 / 90.0, 0.35, 1.0]).collect();
        let unorm = texel::pack(&colors, F::Rgba8Unorm).expect("packed");
        let half = texel::pack(&colors.iter().map(|c| [c[0] * 4.0, c[1] * 2.0, c[2], 1.0]).collect::<Vec<_>>(), F::Rgba16Float).expect("packed");
        for (source, texels, target, gpu_line) in [
            (F::Rgba8Unorm, unorm.clone(), F::Bc7RgbaUnorm, "the Bc7RgbaUnorm texels were decoded on the GPU into Rgba16Float and packed into Rgba8Unorm on the CPU"),
            (F::Rgba8Unorm, unorm, F::Bc7RgbaUnormSrgb, "the Bc7RgbaUnormSrgb texels were decoded on the GPU into Rgba16Float and packed into Rgba8UnormSrgb on the CPU"),
            (F::Rgba16Float, half, F::Bc6hRgbUfloat, "the Bc6hRgbUfloat texels were decoded on the GPU into Rgba16Float"),
        ] {
            let encoded = convert_file(&core, image(source, 8, 8, texels), Some(target), None, MipOptions::default(), None, None, &mut Said::default()).expect("encoded");
            let made = upload(&core, &encoded, None, "test".to_string(), Vec::new()).expect("uploaded");
            let mut said = Said::default();
            let cpu = to_image(&core, &made.texture, 0, None, None, &mut said).expect("decoded on the CPU");
            let gpu = to_image(&core, &made.texture, 0, Some(Decoder::Gpu), None, &mut said).expect("decoded on the GPU");
            assert!(cpu.as_bytes() == gpu.as_bytes(), "{target:?} decodes differently on the CPU and the GPU");
            assert_eq!(said.verbose.len(), 2, "{said:?}");
            assert!(said.verbose[0].starts_with(&format!("the {} texels were decoded on the CPU into", format_name(target))), "{said:?}");
            assert_eq!(said.verbose[1], gpu_line);
        }
        let plain = upload(&core, &image(F::Rgba8Unorm, 4, 4, vec![0; 64]), None, "test".to_string(), Vec::new()).expect("uploaded");
        assert_eq!(refused(to_image(&core, &plain.texture, 0, Some(Decoder::Cpu), None, &mut Said::default())).error_id, "WgpuDecoder");
        let signed = upload(&core, &image(F::Bc4RSnorm, 4, 4, vec![0; 8]), None, "test".to_string(), Vec::new()).expect("uploaded");
        let e = refused(to_image(&core, &signed.texture, 0, None, None, &mut Said::default()));
        assert_eq!(e.error_id, "WgpuImageFormat");
        assert!(e.message.starts_with("a Bc4RSnorm texture decodes into R8Snorm, which has no image form"), "{}", e.message);
    }

    /// An image comes from a layer the texture has, and a 3D texture has
    /// one.
    #[test]
    fn saves_an_image_only_from_a_layer_the_texture_has() {
        let Some(core) = core() else { return };
        let texels: Vec<u8> = (0..192).map(|i| i as u8).collect();
        let three = TextureFile { layers: 3, levels: vec![texels.clone()], ..image(F::Rgba8Unorm, 4, 4, Vec::new()) };
        let three = upload(&core, &three, None, "test".to_string(), Vec::new()).expect("uploaded");
        let last = to_image(&core, &three.texture, 2, None, None, &mut Said::default()).expect("layer 2 made an image");
        assert_eq!(last.as_bytes(), &texels[128..]);
        let past = refused(to_image(&core, &three.texture, 3, None, None, &mut Said::default()));
        assert_eq!((past.error_id.as_str(), past.message.as_str()), ("WgpuLayer", "layer 3 is past the texture's 3 layers"));
        let volume = TextureFile { dimension: wgpu::TextureDimension::D3, depth: 3, levels: vec![texels], ..image(F::Rgba8Unorm, 4, 4, Vec::new()) };
        let volume = upload(&core, &volume, None, "test".to_string(), Vec::new()).expect("uploaded");
        let past = refused(to_image(&core, &volume.texture, 1, None, None, &mut Said::default()));
        assert_eq!((past.error_id.as_str(), past.message.as_str()), ("WgpuLayer", "layer 1 is past the texture's single layer"));
    }

    #[test]
    fn refuses_to_average_integer_texels() {
        let Some(core) = core() else { return };
        let e = refused(convert_file(&core, image(F::Rgba8Uint, 2, 2, vec![0; 16]), None, Some(0), MipOptions::default(), None, None, &mut Said::default()));
        assert!(e.message.contains("Rgba8Uint holds integers"), "{}", e.message);
    }

    /// A format the module does not encode is refused before any work.
    #[test]
    fn refuses_an_astc_target_and_names_the_basis_formats() {
        let Some(core) = core() else { return };
        let e = refused(convert_file(&core, image(F::Rgba8Unorm, 4, 4, vec![0; 64]), Some(F::Astc { block: wgpu::AstcBlock::B4x4, channel: wgpu::AstcChannel::Unorm }), None, MipOptions::default(), None, None, &mut Said::default()));
        assert!(e.message.contains("BasisAstc4x4"), "{}", e.message);
    }

    /// A 3D texture's next level averages each 2x2x2 cube of texels.
    #[test]
    fn renders_the_levels_of_a_volume() {
        let Some(core) = core() else { return };
        let texels: Vec<u8> = (0..8u8).map(|v| v * 30).collect();
        let volume = TextureFile { dimension: wgpu::TextureDimension::D3, width: 2, height: 2, depth: 2, layers: 1, cube: false, format: F::R8Unorm, levels: vec![texels.clone()] };
        let out = convert_file(&core, volume, None, Some(0), MipOptions::default(), None, None, &mut Said::default()).expect("converted");
        assert_eq!(out.levels, [texels, vec![105]]);
    }

    /// A 1D texture converts through a 2D texture one texel high, and
    /// stays 1D.
    #[test]
    fn converts_a_1d_texture() {
        let Some(core) = core() else { return };
        let line = TextureFile { dimension: wgpu::TextureDimension::D1, width: 4, height: 1, depth: 1, layers: 1, cube: false, format: F::R8Unorm, levels: vec![vec![0, 51, 102, 255]] };
        let out = convert_file(&core, line, Some(F::R32Float), None, MipOptions::default(), None, None, &mut Said::default()).expect("converted");
        assert_eq!((out.dimension, out.format), (wgpu::TextureDimension::D1, F::R32Float));
        let values: Vec<f32> = texel::colors(&out.levels[0], F::R32Float).expect("colors").iter().map(|c| c[0]).collect();
        assert!(values.iter().zip([0.0, 0.2, 0.4, 1.0]).all(|(a, b)| (a - b).abs() < 1e-3), "{values:?}");
    }

    /// A 2D texture hands basisu the levels the module renders; a cube map
    /// takes the full chain basisu makes, and refuses a part of one.
    #[test]
    fn encodes_basis_with_the_levels_asked_for() {
        let Some(core) = core() else { return };
        let uastc = BASIS_FORMATS[1];
        let texels: Vec<u8> = (0..16 * 16).flat_map(|i| [(i * 3) as u8, (i * 5) as u8, 90, 255]).collect();
        let mut said = Said::default();
        let bytes = encode_basis(&core, image(F::Rgba8Unorm, 16, 16, texels.clone()), uastc, Some(0), MipOptions::default(), None, None, false, None, &mut said).expect("encoded");
        let file = texfile::read_ktx2(&bytes, false, wgpu::Features::empty(), &mut said).expect("read");
        assert_eq!((file.width, file.height, file.levels.len(), file.format), (16, 16, 5, F::Rgba8Unorm));
        assert!(said.verbose.iter().any(|l| l == "the texture was encoded into BasisUastc4x4 by Basis Universal"), "{said:?}");
        assert_eq!(said.mips, MipsMade::Rendered);
        let cube = TextureFile { layers: 6, cube: true, levels: vec![texels.repeat(6)], ..image(F::Rgba8Unorm, 16, 16, Vec::new()) };
        let bytes = encode_basis(&core, cube, uastc, Some(0), MipOptions::given(Some(MipFilter::Kaiser), None), None, None, false, None, &mut said).expect("encoded");
        let file = texfile::read_ktx2(&bytes, false, wgpu::Features::empty(), &mut said).expect("read");
        assert_eq!((file.layers, file.cube, file.levels.len()), (6, true, 5));
        assert_eq!(said.mips, MipsMade::ByBasis);
        let cube = TextureFile { layers: 6, cube: true, levels: vec![texels.repeat(6)], ..image(F::Rgba8Unorm, 16, 16, Vec::new()) };
        assert_eq!(refused(encode_basis(&core, cube, uastc, Some(3), MipOptions::default(), None, None, false, None, &mut said)).error_id, "WgpuMipLevels");
    }

    /// Basis Universal stores a normal n as n / 2 + 1 / 2 in an HDR format
    /// too: normals stored as n go into its float pixels written again,
    /// with no value below 0 for its encoder to clamp.
    #[test]
    fn encodes_normals_into_a_basis_hdr_format_as_n_over_2_plus_one_half() {
        let Some(core) = core() else { return };
        let normals = MipOptions::default().with_normals(Some(normal::NormalMips::Renormalize), true);
        let texels = texel::pack(&[[0.48, -0.6, 0.64, 1.0]; 16], F::Rgba32Float).expect("packed");
        let mut said = Said::default();
        let bytes = encode_basis(&core, image(F::Rgba32Float, 4, 4, texels), BASIS_FORMATS[2], None, normals, None, None, false, None, &mut said).expect("encoded");
        let file = texfile::read_ktx2(&bytes, false, wgpu::Features::empty(), &mut said).expect("read");
        let colors = texel::colors(&file.levels[0], file.format).expect("colors");
        assert!(colors.iter().all(|c| (c[0] - 0.74).abs() < 0.02 && (c[1] - 0.2).abs() < 0.02 && (c[2] - 0.82).abs() < 0.02), "{:?}", colors[0]);
    }

    /// A mip filter or edge with no level to render is refused; one the
    /// file decides about runs, and a warning says it changed nothing when
    /// the file had the levels or Basis Universal made them.
    #[test]
    fn refuses_or_warns_of_a_mip_filter_that_changes_nothing() {
        let kaiser = MipOptions::given(Some(MipFilter::Kaiser), None);
        assert_eq!(refused(Settings { mips: kaiser, ..Default::default() }.check()).error_id, "WgpuMipFilter");
        let resize = Transform { width: Some(4), ..Default::default() };
        if let Err(e) = (Settings { mips: kaiser, transform: resize, ..Default::default() }.check()) {
            panic!("a transform renders levels again, and was refused: {e:?}");
        }
        assert_eq!(refused(Settings { mips: kaiser, mip_levels: Some(1), transform: resize, ..Default::default() }.check()).error_id, "WgpuMipFilter");
        let edge = MipOptions::given(Some(MipFilter::Min), Some(MipEdge::Mirror));
        assert_eq!(refused(Settings { mips: edge, mip_levels: Some(0), ..Default::default() }.check()).error_id, "WgpuMipEdge");
        let warned = |made: MipsMade, mips: MipOptions| {
            let mut said = Said { mips: made, ..Default::default() };
            mips_unused(mips, "the file", &mut said);
            said.warnings
        };
        assert_eq!(warned(MipsMade::NoneRendered, kaiser), ["no mip level was rendered, as the file has every level the call asks for, so the mip filter changed nothing"]);
        let both = MipOptions::given(Some(MipFilter::Lanczos), Some(MipEdge::Wrap));
        assert_eq!(warned(MipsMade::ByBasis, both), ["Basis Universal made the mip levels with its own filter, so the mip filter and edge changed nothing"]);
        assert_eq!(warned(MipsMade::Rendered, kaiser), Vec::<String>::new());
        assert_eq!(warned(MipsMade::NoneRendered, MipOptions::default()), Vec::<String>::new());
    }

    /// -NormalMips alone beside a -Format runs without -MipLevels, as the
    /// file decides whether its normals are written again; with nothing to
    /// convert into, or beside another mip option with no level to render,
    /// it is refused. A warning says it changed nothing only where no
    /// normal was written again either.
    #[test]
    fn lets_the_file_decide_about_normal_mips_beside_a_format() {
        let normals = MipOptions::default().with_normals(Some(normal::NormalMips::Renormalize), true);
        let float = Some(Target::Gpu(F::Rgba16Float));
        if let Err(e) = (Settings { mips: normals, format: float, ..Default::default() }.check()) {
            panic!("-NormalMips beside a -Format was refused: {e:?}");
        }
        if let Err(e) = (Settings { mips: normals, format: float, mip_levels: Some(1), ..Default::default() }.check()) {
            panic!("-NormalMips beside a -Format and -MipLevels 1 was refused: {e:?}");
        }
        assert_eq!(refused(Settings { mips: normals, ..Default::default() }.check()).error_id, "WgpuMipFilter");
        let kaiser = MipOptions::given(Some(MipFilter::Kaiser), None).with_normals(Some(normal::NormalMips::Renormalize), true);
        assert_eq!(refused(Settings { mips: kaiser, format: float, ..Default::default() }.check()).error_id, "WgpuMipFilter");
        let warned = |rewritten: bool, mips: MipOptions| {
            let mut said = Said { normals_rewritten: rewritten, ..Default::default() };
            mips_unused(mips, "the file", &mut said);
            said.warnings
        };
        assert_eq!(warned(false, normals), ["no mip level was rendered, as the file has every level the call asks for, so -NormalMips changed nothing"]);
        assert_eq!(warned(true, normals), Vec::<String>::new());
        assert_eq!(warned(true, kaiser), ["no mip level was rendered, as the file has every level the call asks for, so the mip filter changed nothing"]);
    }

    /// GenerateMips(levels) and Compress return copies whose Notes say what
    /// making them involved.
    #[test]
    fn copies_a_texture_with_levels_and_compressed_and_notes_how() {
        let Some(core) = core() else { return };
        let made = upload(&core, &image(F::Rgba8Unorm, 8, 8, vec![200; 256]), None, "test".to_string(), Vec::new()).expect("uploaded");
        let source = Source { texture: made.texture.clone(), cube: false, label: "test".to_string() };
        let mipped = with_mips(&core, &source, 0, MipOptions::default(), None).expect("levels");
        assert_eq!((mipped.mip_level_count, mipped.label.as_str()), (4, "test"));
        assert!(mipped.notes.iter().any(|n| n.starts_with("mip levels 1 to 3 were rendered")), "{:?}", mipped.notes);
        let has_them = Source { texture: mipped.texture.clone(), cube: false, label: "test".to_string() };
        let again = with_mips(&core, &has_them, 2, MipOptions::given(Some(MipFilter::Kaiser), None), None).expect("levels");
        assert_eq!(again.notes, ["no mip level was rendered, as the texture has every level the call asks for, so the mip filter changed nothing"]);
        assert_eq!(refused(with_mips(&core, &source, 0, MipOptions::given(None, Some(MipEdge::Wrap)), None)).error_id, "WgpuMipEdge");
        assert_eq!(refused(with_mips(&core, &source, 1, MipOptions::given(Some(MipFilter::Kaiser), None), None)).error_id, "WgpuMipFilter");
        if !core.features.contains(wgpu::Features::TEXTURE_COMPRESSION_BC) {
            eprintln!("skipped Compress: the device has no TextureCompressionBc");
            return;
        }
        let bc = compressed(&core, &source, "Bc7RgbaUnorm", None, Some(0), None).expect("compressed");
        assert_eq!(bc.format, "Bc7RgbaUnorm");
        assert!(bc.notes.iter().any(|n| n.ends_with("by block_compression's BC7 alpha ultra fast settings")), "{:?}", bc.notes);
        // The source takes every usage its format allows on the device, and
        // the copy leaves out StorageBinding only where the device allows it
        // for Rgba8Unorm and not for Bc7RgbaUnorm: Metal allows read-only
        // storage of every format.
        let storage = wgpu::TextureUsages::STORAGE_BINDING;
        let left_out = if made.texture.usage().contains(storage) && !texture::allowed_usages(&core, F::Bc7RgbaUnorm).contains(storage) {
            "the new texture leaves out the source's StorageBinding and RenderAttachment usages, which Bc7RgbaUnorm does not allow"
        } else {
            "the new texture leaves out the source's RenderAttachment usage, which Bc7RgbaUnorm does not allow"
        };
        assert!(bc.notes.iter().any(|n| n == left_out), "{:?}", bc.notes);
    }

    /// A conversion of normals into a format that stores them the other
    /// way writes each one again as that format stores it, in every mip
    /// level: n / 2 + 1 / 2 into n, n held as X and Y alone into
    /// n / 2 + 1 / 2 with its Z rebuilt, and n into BC5's n / 2 + 1 / 2.
    /// Without normals the values are copied as they are.
    #[test]
    fn writes_normals_again_for_a_format_that_stores_them_the_other_way() {
        let Some(core) = core() else { return };
        let normals = MipOptions::default().with_normals(Some(normal::NormalMips::Renormalize), true);
        let near = |got: [f32; 4], want: [f32; 4], tolerance: f32| (0..4).all(|c| (got[c] - want[c]).abs() <= tolerance);
        // (0.6, 0, 0.8) with alpha 1 and (0, -0.6, 0.8) with alpha 0.5,
        // stored as n / 2 + 1 / 2.
        let stored = texel::pack(&[[0.8, 0.5, 0.9, 1.0], [0.5, 0.2, 0.9, 0.5]], F::Rgba8Unorm).expect("packed");
        let mut said = Said::default();
        let out = convert_file(&core, image(F::Rgba8Unorm, 2, 1, stored.clone()), Some(F::Rgba32Float), Some(0), normals, None, None, &mut said).expect("converted");
        let top = texel::colors(&out.levels[0], F::Rgba32Float).expect("colors");
        assert!(near(top[0], [0.6, 0.0, 0.8, 1.0], 0.01) && near(top[1], [0.0, -0.6, 0.8, 0.5], 0.01), "{top:?}");
        let below = texel::colors(&out.levels[1], F::Rgba32Float).expect("colors");
        assert!(near(below[0], [0.331, -0.331, 0.883, 0.75], 0.01), "{below:?}");
        assert!(said.verbose.iter().any(|l| l == "the normals of mip levels 0 to 1 were written again on the CPU as Rgba32Float stores them, n itself rather than n / 2 + 1 / 2"), "{:?}", said.verbose);
        let copied = convert_file(&core, image(F::Rgba8Unorm, 2, 1, stored), Some(F::Rgba32Float), None, MipOptions::default(), None, None, &mut Said::default()).expect("converted");
        let copied = texel::colors(&copied.levels[0], F::Rgba32Float).expect("colors");
        assert!(near(copied[0], [0.8, 0.5, 0.9, 1.0], 0.003), "{copied:?}");
        // (0.48, 0.6, 0.64) stores as 0.74, 0.8 and 0.82: 189, 204 and 209.
        let xy = texel::pack(&[[0.48, 0.6, 0.0, 1.0]], F::Rg32Float).expect("packed");
        let out = convert_file(&core, image(F::Rg32Float, 1, 1, xy), Some(F::Rgba8Unorm), None, normals, None, None, &mut Said::default()).expect("converted");
        assert_eq!(out.levels[0], [189, 204, 209, 255]);
        let block = texel::pack(&[[0.48, 0.6, 0.64, 1.0]; 16], F::Rgba32Float).expect("packed");
        let out = convert_file(&core, image(F::Rgba32Float, 4, 4, block), Some(F::Bc5RgUnorm), None, normals, None, None, &mut Said::default()).expect("encoded");
        let decoded = texfile::uncompressed(out, &mut Said::default()).expect("decoded");
        assert_eq!(&decoded.levels[0][..2], [189, 204]);
    }
}
