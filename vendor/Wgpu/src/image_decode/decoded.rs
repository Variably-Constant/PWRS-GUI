//! The library's decode of an image file's bytes, [`decode_image`]: each
//! frame or page at its own size, with its pixels as Import-WgpuImage loads
//! them and as the file stores them, and a DDS or KTX2 file's texels, every
//! layer, cube face and depth slice of its largest mip level.

use std::fmt;
use std::path::Path;

use image::metadata::Orientation;
use image::{DynamicImage, GenericImageView};
use pwrs::prelude::*;
use wgpu::{TextureDimension, TextureFormat};

use super::{images, unreadable, Asked, Cmyk, Images, Names, Reading, RenderingIntent, Timing};
use crate::error::invalid;
use crate::texfile::{self, Said};
use crate::texture::format_name;
use crate::transform::Transform;

/// What [`decode_image`] does with an image file: what Import-WgpuImage's
/// -IgnoreOrientation, -IgnoreProfile, -RenderingIntent and -Frame do, and,
/// for an SVG file or a PDF, its -Scale, -Dpi, -NoBackground and -Password.
/// The default decodes every frame or page, turned as the file says and
/// converted from its color profile; a caller sets the fields it needs and
/// takes the rest with `..Default::default()`. Its `Debug` says whether a
/// password is given, never the password.
#[derive(Clone, Default, PartialEq)]
pub struct DecodeOptions {
    /// Leaves the pixels as stored, not turned as the file's orientation
    /// says.
    pub ignore_orientation: bool,
    /// Leaves the display pixels' colors as stored, not converted from the
    /// file's color profile.
    pub ignore_profile: bool,
    /// How a conversion from the file's color profile maps colors:
    /// perceptually when `None`. It is refused beside `ignore_profile`,
    /// which converts nothing.
    pub rendering_intent: Option<RenderingIntent>,
    /// The frames of an animation or the pages of a TIFF file or a PDF to
    /// decode, counted from 0, in the order named and as often as named:
    /// every one when `None`. It is refused for a DDS or KTX2 file, which
    /// decodes every layer.
    pub frames: Option<Vec<u32>>,
    /// How many times its own size an SVG file, or a PDF's page at `dpi`,
    /// is drawn: a number above 0, and 1 when `None`.
    pub scale: Option<f64>,
    /// The dots per inch a PDF's pages are drawn at: a number above 0, and
    /// 72, a pixel a point, when `None`.
    pub dpi: Option<f64>,
    /// Leaves a PDF's pages transparent where nothing is drawn, not white.
    pub no_background: bool,
    /// The password a protected PDF opens with.
    pub password: Option<String>,
}

impl fmt::Debug for DecodeOptions {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DecodeOptions")
            .field("ignore_orientation", &self.ignore_orientation)
            .field("ignore_profile", &self.ignore_profile)
            .field("rendering_intent", &self.rendering_intent)
            .field("frames", &self.frames)
            .field("scale", &self.scale)
            .field("dpi", &self.dpi)
            .field("no_background", &self.no_background)
            .field("password", &self.password.as_ref().map(|_| "<hidden>"))
            .finish()
    }
}

/// An image file's frames or pages as [`decode_image`] decodes them, with
/// what decoding did and could not do.
#[derive(Clone, Debug, PartialEq)]
pub struct DecodedImage {
    /// The frames of an animation, the pages of a TIFF file or a PDF, or
    /// the layers, cube faces or depth slices of a DDS or KTX2 file, in the
    /// file's order or the order `frames` names them in; a still image's
    /// one.
    pub frames: Vec<DecodedFrame>,
    /// Each frame's delay in milliseconds, for an animation; empty for a
    /// file that is none.
    pub frame_delays: Vec<u32>,
    /// How many times an animation plays, 0 for endlessly, a GIF one time
    /// more than its repeat count, which counts the plays after the first;
    /// `None` for a file that is no animation.
    pub loop_count: Option<u32>,
    /// Whether the display pixels are an HDR file's light: linear, in
    /// sRGB's primaries, with BT.2408's HDR reference white of 203 nits at
    /// 1 and brighter light above it, as an AVIF or JPEG XL file of PQ or
    /// HLG transfer decodes.
    pub hdr: bool,
    /// Whether the frames are cube maps' faces, six to a cube in the order
    /// +X, -X, +Y, -Y, +Z and -Z.
    pub cube: bool,
    /// Whether the frames are a 3D texture's depth slices, slice 0 first.
    pub volume: bool,
    /// What decoding could not do or left out, as Import-WgpuImage warns
    /// of it.
    pub warnings: Vec<String>,
    /// What decoding did, as Import-WgpuImage's verbose stream says it.
    pub notes: Vec<String>,
}

/// A frame or page: its size, and its pixels as the file stores them and
/// as they display.
#[derive(Clone, Debug, PartialEq)]
pub struct DecodedFrame {
    /// The width in pixels, which both its pixels have.
    pub width: u32,
    /// The height in pixels, which both its pixels have.
    pub height: u32,
    /// The pixels as the file stores them, at its depth and in its own
    /// color before any conversion from its profile: gray, RGB, CMYK inks
    /// or CIE Lab, with alpha where the file has it. They are turned as
    /// `display` is, so a position names the same pixel in both. An SVG
    /// file's drawing, a PDF's page and a DDS or KTX2 file's texels are the
    /// same here as in `display`.
    pub stored: Pixels,
    /// The pixels as Import-WgpuImage loads them, turned as the file's
    /// orientation says: colors converted to sRGB at 8 or 16 bits, a float
    /// image's to linear light in sRGB's primaries with light above 1 kept,
    /// and an HDR file's light as [`DecodedImage::hdr`] says. Where the
    /// colors are not converted, as `ignore_profile` asks or as a profile
    /// cannot be used, they are as stored, a CMYK image's as the RGB
    /// (max - c)(max - k) / max gives and a CIE Lab document's L, a and b
    /// as red, green and blue. A DDS or KTX2 file's texels are in the
    /// texture's format, a block-compressed one decoded.
    pub display: Pixels,
}

/// Pixels in rows from the top, each row from the left, each pixel's
/// samples in its format's order, with nothing between pixels or rows:
/// 16-bit and float samples in the machine's byte order, and a GPU
/// format's texels as a texture holds them, little-endian.
#[derive(Clone, Debug, PartialEq)]
pub struct Pixels {
    /// What each pixel's samples are.
    pub format: PixelFormat,
    /// The samples.
    pub data: Vec<u8>,
}

/// What a pixel's samples are, and the order they come in.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PixelFormat {
    /// Gray, 8 bits.
    Gray8,
    /// Gray and alpha, 8 bits each.
    GrayAlpha8,
    /// Red, green and blue, 8 bits each.
    Rgb8,
    /// Red, green, blue and alpha, 8 bits each.
    Rgba8,
    /// Gray, 16 bits.
    Gray16,
    /// Gray and alpha, 16 bits each.
    GrayAlpha16,
    /// Red, green and blue, 16 bits each.
    Rgb16,
    /// Red, green, blue and alpha, 16 bits each.
    Rgba16,
    /// Gray, a 32-bit float.
    Gray32F,
    /// Gray and alpha, 32-bit floats.
    GrayAlpha32F,
    /// Red, green and blue, 32-bit floats.
    Rgb32F,
    /// Red, green, blue and alpha, 32-bit floats.
    Rgba32F,
    /// Cyan, magenta, yellow and black inks, 8 bits each, from 0, none, to
    /// 255, full.
    Cmyk8,
    /// Cyan, magenta, yellow and black inks and alpha, 8 bits each.
    CmykAlpha8,
    /// Cyan, magenta, yellow and black inks, 16 bits each, from 0, none, to
    /// 65535, full.
    Cmyk16,
    /// Cyan, magenta, yellow and black inks and alpha, 16 bits each.
    CmykAlpha16,
    /// CIE Lab under D50, 8 bits each: L from 0 to 255 for 0 to 100, and a
    /// and b from 0 to 255 for -128 to 127.
    Lab8,
    /// CIE Lab and alpha, 8 bits each.
    LabAlpha8,
    /// CIE Lab under D50, 16 bits each: L from 0 to 65535 for 0 to 100,
    /// and a and b from 0 to 65535 for -128 to 127.
    Lab16,
    /// CIE Lab and alpha, 16 bits each.
    LabAlpha16,
    /// Texels of an uncompressed GPU format, as a DDS or KTX2 file holds
    /// them or as its blocks decode: the format says their channels, their
    /// encoding, sRGB or linear among them, and their size.
    Texels(TextureFormat),
}

impl PixelFormat {
    /// The samples a pixel holds, and the bytes each takes, for every
    /// format but a GPU one.
    fn layout(self) -> Option<(u32, u32)> {
        use PixelFormat as P;
        Some(match self {
            P::Gray8 => (1, 1),
            P::GrayAlpha8 => (2, 1),
            P::Rgb8 | P::Lab8 => (3, 1),
            P::Rgba8 | P::Cmyk8 | P::LabAlpha8 => (4, 1),
            P::CmykAlpha8 => (5, 1),
            P::Gray16 => (1, 2),
            P::GrayAlpha16 => (2, 2),
            P::Rgb16 | P::Lab16 => (3, 2),
            P::Rgba16 | P::Cmyk16 | P::LabAlpha16 => (4, 2),
            P::CmykAlpha16 => (5, 2),
            P::Gray32F => (1, 4),
            P::GrayAlpha32F => (2, 4),
            P::Rgb32F => (3, 4),
            P::Rgba32F => (4, 4),
            P::Texels(_) => return None,
        })
    }

    /// The samples a pixel holds, as a GPU format counts its components.
    pub fn channels(self) -> u32 {
        match (self, self.layout()) {
            (_, Some((channels, _))) => channels,
            (PixelFormat::Texels(format), None) => u32::from(format.components()),
            (_, None) => 0,
        }
    }

    /// The bytes a pixel takes; `None` for a GPU format whose texels have
    /// no size of their own, a block-compressed or planar one, which no
    /// decode gives.
    pub fn pixel_bytes(self) -> Option<u32> {
        match (self, self.layout()) {
            (_, Some((channels, bytes))) => Some(channels * bytes),
            (PixelFormat::Texels(format), None) if !format.is_compressed() => format.block_copy_size(None),
            (_, None) => None,
        }
    }

    /// Whether a pixel carries alpha.
    pub fn has_alpha(self) -> bool {
        use PixelFormat as P;
        match self {
            P::GrayAlpha8 | P::Rgba8 | P::GrayAlpha16 | P::Rgba16 | P::GrayAlpha32F | P::Rgba32F | P::CmykAlpha8 | P::CmykAlpha16 | P::LabAlpha8 | P::LabAlpha16 => true,
            P::Texels(format) => format.has_color_aspect() && format.components() == 4,
            _ => false,
        }
    }
}

/// The image file `bytes` hold, decoded as Import-WgpuImage loads it, each
/// frame or page at its own size, with its pixels as the file stores them
/// beside those to display. `name`, the file's name or path, labels the
/// messages, which say "the file" without one, tells by its extension a
/// format the bytes do not, such as TGA's, and is where an SVG file's
/// relative references are read from.
///
/// It reads what Import-WgpuImage reads: the image crate's formats and
/// Photoshop documents; AVIF, JPEG XL, SVG and PDF files in builds with the
/// avif, jxl, svg and pdf features, and WebP through libwebp with the webp
/// feature; and DDS and KTX2 files, whose texels it decodes as
/// Import-WgpuImage decodes them for a device that cannot hold their
/// format. The error is the one Import-WgpuImage raises for the same
/// failure: WgpuImage for an image file that cannot be read, WgpuTextureFile
/// for a DDS or KTX2 file, and WgpuRenderingIntent, WgpuDpi, WgpuResize and
/// WgpuFrame for options refused.
pub fn decode_image(bytes: &[u8], name: Option<&str>, options: &DecodeOptions) -> PsResult<DecodedImage> {
    if options.ignore_profile && options.rendering_intent.is_some() {
        return Err(invalid("WgpuRenderingIntent", "rendering_intent chooses how colors convert from a file's color profile, and ignore_profile converts none: give one or the other"));
    }
    if let Some(dpi) = options.dpi
        && !(dpi.is_finite() && dpi > 0.0)
    {
        return Err(invalid("WgpuDpi", format!("dpi is a number of dots per inch above 0, not {dpi}")));
    }
    if let Some(scale) = options.scale
        && !(scale.is_finite() && scale > 0.0)
    {
        return Err(invalid("WgpuResize", format!("scale is a number above 0, not {scale}")));
    }
    let reading = Reading {
        ignore_orientation: options.ignore_orientation,
        ignore_profile: options.ignore_profile,
        intent: options.rendering_intent,
        dpi: options.dpi,
        no_background: options.no_background,
        names: Names::FIELDS,
        stored: true,
    };
    let transform = Transform { scale: options.scale, ..Transform::default() };
    let asked = Asked { reading, transform: &transform, password: options.password.as_deref(), frames: options.frames.as_deref() };
    let path = Path::new(match name {
        Some(name) => name,
        None => "the file",
    });
    let mut said = Said::default();
    let found = match texture_kind(bytes) {
        Some(kind) => textures(bytes, kind, asked, &mut said)?,
        None => images_of(path, bytes, asked, &mut said)?,
    };
    let Found { frames, timing, hdr, cube, volume, drawn } = found;
    if options.scale.is_some() && !drawn {
        said.warnings.push("the file is neither an SVG file nor a PDF, so scale changed nothing".to_string());
    }
    let (frame_delays, loop_count) = match timing {
        Some(Timing { delays, loops }) => (delays, Some(loops)),
        None => (Vec::new(), None),
    };
    Ok(DecodedImage { frames, frame_delays, loop_count, hdr, cube, volume, warnings: said.warnings, notes: said.verbose })
}

/// What a decode found: its frames, the timing of an animation's, whether
/// they are an HDR file's light, cube faces or depth slices, and whether a
/// vector file was drawn.
struct Found {
    frames: Vec<DecodedFrame>,
    timing: Option<Timing>,
    hdr: bool,
    cube: bool,
    volume: bool,
    drawn: bool,
}

/// An image file's frames, each displayed as Import-WgpuImage loads it and
/// kept as the file stores it.
fn images_of(path: &Path, bytes: &[u8], asked: Asked<'_>, said: &mut Said) -> PsResult<Found> {
    let Images { frames, stored, timing, hdr, drawn, .. } = images(path, bytes, asked, said)?;
    if stored.len() != frames.len() {
        return Err(unreadable(path, &format!("{} of its {} frames were kept as stored", stored.len(), frames.len())));
    }
    let frames = frames
        .into_iter()
        .zip(stored)
        .map(|(image, kept)| {
            let (width, height) = image.dimensions();
            if (kept.width, kept.height) != (width, height) {
                return Err(unreadable(path, &format!("a frame of {width}x{height} pixels was kept as stored at {}x{}", kept.width, kept.height)));
            }
            let display = Pixels { format: format_of(&image).map_err(|e| unreadable(path, &e))?, data: image.into_bytes() };
            Ok(DecodedFrame { width, height, stored: kept.pixels, display })
        })
        .collect::<PsResult<Vec<_>>>()?;
    Ok(Found { frames, timing, hdr, cube: false, volume: false, drawn })
}

/// The format of `image`'s pixels, or why the library names none.
fn format_of(image: &DynamicImage) -> Result<PixelFormat, String> {
    use DynamicImage as D;
    use PixelFormat as P;
    Ok(match image {
        D::ImageLuma8(_) => P::Gray8,
        D::ImageLumaA8(_) => P::GrayAlpha8,
        D::ImageRgb8(_) => P::Rgb8,
        D::ImageRgba8(_) => P::Rgba8,
        D::ImageLuma16(_) => P::Gray16,
        D::ImageLumaA16(_) => P::GrayAlpha16,
        D::ImageRgb16(_) => P::Rgb16,
        D::ImageRgba16(_) => P::Rgba16,
        D::ImageRgb32F(_) => P::Rgb32F,
        D::ImageRgba32F(_) => P::Rgba32F,
        other => return Err(format!("its pixels are {:?}, which the library names no format for", other.color())),
    })
}

/// A frame's pixels as the file stores them, and their size.
pub(super) struct Kept {
    pub(super) pixels: Pixels,
    pub(super) width: u32,
    pub(super) height: u32,
}

impl Kept {
    /// The pixels turned as `orientation` says, as image's
    /// `DynamicImage::apply_orientation` turns an image.
    pub(super) fn oriented(self, orientation: Orientation) -> Kept {
        let Kept { pixels: Pixels { format, data }, width, height } = self;
        let across = matches!(orientation, Orientation::Rotate90 | Orientation::Rotate270 | Orientation::Rotate90FlipH | Orientation::Rotate270FlipH);
        let (turned_width, turned_height) = if across { (height, width) } else { (width, height) };
        let (w, h) = (width as usize, height as usize);
        let count = w * h;
        if count == 0 {
            return Kept { pixels: Pixels { format, data }, width: turned_width, height: turned_height };
        }
        let size = data.len() / count;
        let mut turned = vec![0; data.len()];
        for (at, pixel) in data.chunks_exact(size).enumerate() {
            let (x, y) = (at % w, at / w);
            let (tx, ty) = match orientation {
                Orientation::NoTransforms => (x, y),
                Orientation::Rotate90 => (h - 1 - y, x),
                Orientation::Rotate180 => (w - 1 - x, h - 1 - y),
                Orientation::Rotate270 => (y, w - 1 - x),
                Orientation::FlipHorizontal => (w - 1 - x, y),
                Orientation::FlipVertical => (x, h - 1 - y),
                Orientation::Rotate90FlipH => (y, x),
                Orientation::Rotate270FlipH => (h - 1 - y, w - 1 - x),
            };
            let to = (ty * turned_width as usize + tx) * size;
            turned[to..to + size].copy_from_slice(pixel);
        }
        Kept { pixels: Pixels { format, data: turned }, width: turned_width, height: turned_height }
    }
}

/// A frame's pixels as the file stores them: `image` as decoded; a CMYK
/// image's inks, `cmyk`, each pixel's alpha from `image` after them where
/// it has alpha; a CIE Lab document's L, a and b, which `image` holds as
/// red, green and blue, when `lab`; and a gray float image's gray, the
/// first of the three samples `image` holds of each pixel, with its alpha,
/// when `gray_floats`.
pub(super) fn as_stored(image: &DynamicImage, cmyk: Option<&Cmyk>, lab: bool, gray_floats: bool) -> Result<Kept, String> {
    use DynamicImage as D;
    use PixelFormat as P;
    let (width, height) = image.dimensions();
    let kept = |format: PixelFormat, data: Vec<u8>| Kept { pixels: Pixels { format, data }, width, height };
    if let Some(inks) = cmyk {
        let samples = match inks {
            Cmyk::Eight(inks) => inks.len(),
            Cmyk::Sixteen(inks) => inks.len(),
        };
        if samples != width as usize * height as usize * 4 {
            return Err(format!("its {samples} ink samples do not fill its {width}x{height} pixels"));
        }
        return match (inks, image) {
            (Cmyk::Eight(inks), D::ImageRgb8(_)) => Ok(kept(P::Cmyk8, inks.clone())),
            (Cmyk::Eight(inks), D::ImageRgba8(rgba)) => Ok(kept(P::CmykAlpha8, with_alphas(inks, rgba.pixels().map(|pixel| pixel[3])))),
            (Cmyk::Sixteen(inks), D::ImageRgb16(_)) => Ok(kept(P::Cmyk16, bytemuck::cast_slice::<u16, u8>(inks).to_vec())),
            (Cmyk::Sixteen(inks), D::ImageRgba16(rgba)) => Ok(kept(P::CmykAlpha16, bytemuck::cast_slice::<u16, u8>(&with_alphas(inks, rgba.pixels().map(|pixel| pixel[3]))).to_vec())),
            (_, other) => Err(format!("its inks come with {:?} pixels, not the RGB they make", other.color())),
        };
    }
    if lab {
        let format = match image {
            D::ImageRgb8(_) => P::Lab8,
            D::ImageRgba8(_) => P::LabAlpha8,
            D::ImageRgb16(_) => P::Lab16,
            D::ImageRgba16(_) => P::LabAlpha16,
            other => return Err(format!("its CIE Lab is held as {:?} pixels", other.color())),
        };
        return Ok(kept(format, image.as_bytes().to_vec()));
    }
    if gray_floats {
        return match image {
            D::ImageRgb32F(rgb) => {
                let gray: Vec<f32> = rgb.pixels().map(|pixel| pixel[0]).collect();
                Ok(kept(P::Gray32F, bytemuck::cast_slice::<f32, u8>(&gray).to_vec()))
            }
            D::ImageRgba32F(rgba) => {
                let gray: Vec<f32> = rgba.pixels().flat_map(|pixel| [pixel[0], pixel[3]]).collect();
                Ok(kept(P::GrayAlpha32F, bytemuck::cast_slice::<f32, u8>(&gray).to_vec()))
            }
            other => Err(format!("its gray floats are held as {:?} pixels", other.color())),
        };
    }
    Ok(kept(format_of(image)?, image.as_bytes().to_vec()))
}

/// Each pixel's four inks with its alpha after them.
fn with_alphas<T: Copy>(inks: &[T], alphas: impl Iterator<Item = T>) -> Vec<T> {
    inks.as_chunks::<4>().0.iter().zip(alphas).flat_map(|(&[c, m, y, k], a)| [c, m, y, k, a]).collect()
}

/// The texture files a decode reads whole.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum TextureKind {
    Dds,
    Ktx2,
}

/// The identifier a KTX2 file begins with.
const KTX2_IDENTIFIER: [u8; 12] = [0xAB, b'K', b'T', b'X', b' ', b'2', b'0', 0xBB, b'\r', b'\n', 0x1A, b'\n'];

/// The texture file `bytes` hold, told by the magic number a DDS file and
/// the identifier a KTX2 file begin with; `None` for any other file.
fn texture_kind(bytes: &[u8]) -> Option<TextureKind> {
    if bytes.starts_with(b"DDS ") {
        Some(TextureKind::Dds)
    } else if bytes.starts_with(&KTX2_IDENTIFIER) {
        Some(TextureKind::Ktx2)
    } else {
        None
    }
}

/// A DDS or KTX2 file's texels as Import-WgpuImage loads them for a device
/// that cannot hold a block-compressed format: every layer, cube face and
/// depth slice of its largest mip level a frame, a block-compressed format
/// decoded on the CPU, and a Basis Universal file's texels transcoded into
/// an uncompressed format. Its stored pixels are its display pixels.
fn textures(bytes: &[u8], kind: TextureKind, asked: Asked<'_>, said: &mut Said) -> PsResult<Found> {
    let reading = asked.reading;
    let what = match kind {
        TextureKind::Dds => "a DDS file",
        TextureKind::Ktx2 => "a KTX2 file",
    };
    if asked.frames.is_some() {
        return Err(invalid("WgpuFrame", format!("{} picks the frames or pages of an image file, and {what} decodes every layer, face and slice of its largest mip level", reading.names.frames)));
    }
    reading.unused(what, asked.password, said);
    let mut file = match kind {
        TextureKind::Dds => texfile::read_dds(bytes, false, said)?,
        TextureKind::Ktx2 => texfile::read_ktx2(bytes, false, wgpu::Features::empty(), said)?,
    };
    let levels = file.levels.len();
    file.levels.truncate(1);
    if levels > 1 {
        said.verbose.push(format!("the largest of the file's {levels} mip levels was decoded"));
    }
    let from = file.format;
    let file = texfile::uncompressed(file, said)?;
    let format = file.format;
    if format != from {
        said.verbose.push(format!("the file's {} texels were decoded on the CPU into {}", format_name(from), format_name(format)));
    }
    let Some(texel) = format.block_copy_size(None) else {
        return Err(invalid("WgpuTextureFile", format!("a {} texture's texels have no size of their own", format_name(format))));
    };
    let volume = file.dimension == TextureDimension::D3;
    let slices = if volume { file.depth } else { 1 };
    let count = file.layers as usize * slices as usize;
    let slice_len = file.width as usize * file.height as usize * texel as usize;
    let wanted = slice_len * count;
    let level = match file.levels.first() {
        Some(level) if slice_len > 0 && level.len() == wanted => level,
        Some(level) => {
            return Err(invalid(
                "WgpuTextureFile",
                format!("its largest mip level holds {} bytes, where {count} layers and slices of {}x{} {} texels take {wanted}", level.len(), file.width, file.height, format_name(format)),
            ));
        }
        None => return Err(invalid("WgpuTextureFile", "the file holds no mip level")),
    };
    let frames = level
        .chunks_exact(slice_len)
        .map(|texels| {
            let pixels = Pixels { format: PixelFormat::Texels(format), data: texels.to_vec() };
            DecodedFrame { width: file.width, height: file.height, stored: pixels.clone(), display: pixels }
        })
        .collect();
    if count > 1 {
        let parts = if file.cube {
            "cube faces"
        } else if volume {
            "depth slices"
        } else {
            "layers"
        };
        said.verbose.push(format!("the texture's {count} {parts} decoded as frames"));
    }
    Ok(Found { frames, timing: None, hdr: false, cube: file.cube, volume, drawn: false })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::profile_reference as reference;
    use crate::texfile::TextureFile;
    use image::{ImageBuffer, ImageEncoder};

    /// A little-endian TIFF block whose one IFD entry is Orientation (274),
    /// a SHORT of 6: turned 90 degrees clockwise.
    const ROTATE90_EXIF: [u8; 26] = [b'I', b'I', 42, 0, 8, 0, 0, 0, 1, 0, 0x12, 0x01, 3, 0, 1, 0, 0, 0, 6, 0, 0, 0, 0, 0, 0, 0];

    /// A 16-bit RGB PNG of `samples`, with the ICC profile and the EXIF
    /// block given.
    fn png16(width: u32, height: u32, samples: &[u16], icc: Option<&[u8]>, exif: Option<&[u8]>) -> Vec<u8> {
        let mut out = Vec::new();
        let mut encoder = image::codecs::png::PngEncoder::new(&mut out);
        if let Some(icc) = icc {
            encoder.set_icc_profile(icc.to_vec()).expect("a PNG takes a profile");
        }
        if let Some(exif) = exif {
            encoder.set_exif_metadata(exif.to_vec()).expect("a PNG takes EXIF");
        }
        encoder.write_image(bytemuck::cast_slice(samples), width, height, image::ExtendedColorType::Rgb16).expect("the PNG is written");
        out
    }

    /// The one frame of `image`.
    fn only(image: &DecodedImage) -> &DecodedFrame {
        match image.frames.as_slice() {
            [frame] => frame,
            frames => panic!("{} frames decoded, not one", frames.len()),
        }
    }

    /// The error `options` meet decoding `file`, as its id and message.
    fn refused(file: &[u8], name: Option<&str>, options: &DecodeOptions) -> (String, String) {
        match decode_image(file, name, options) {
            Ok(_) => panic!("{options:?} decoded"),
            Err(e) => (e.error_id, e.message),
        }
    }

    /// Stored pixels turn as image turns an image, whatever a pixel's size.
    #[test]
    fn turns_stored_pixels_as_image_turns_an_image() {
        let (width, height) = (3, 2);
        let base = image::RgbaImage::from_fn(width, height, |x, y| {
            let i = (y * width + x) as u8;
            image::Rgba([i, i.wrapping_mul(7), i.wrapping_mul(13), 255])
        });
        let all = [
            Orientation::NoTransforms,
            Orientation::Rotate90,
            Orientation::Rotate180,
            Orientation::Rotate270,
            Orientation::FlipHorizontal,
            Orientation::FlipVertical,
            Orientation::Rotate90FlipH,
            Orientation::Rotate270FlipH,
        ];
        for orientation in all {
            let mut want = DynamicImage::ImageRgba8(base.clone());
            want.apply_orientation(orientation);
            let kept = Kept { pixels: Pixels { format: PixelFormat::Rgba8, data: base.as_raw().clone() }, width, height }.oriented(orientation);
            assert_eq!((kept.width, kept.height), want.dimensions(), "{orientation:?}");
            assert_eq!(kept.pixels.data, want.as_bytes(), "{orientation:?}");
            let inks: Vec<u8> = (0..width * height).flat_map(|i| [i as u8; 5]).collect();
            let turned = Kept { pixels: Pixels { format: PixelFormat::CmykAlpha8, data: inks }, width, height }.oriented(orientation);
            let firsts: Vec<u8> = turned.pixels.data.as_chunks::<5>().0.iter().map(|pixel| pixel[0]).collect();
            let wanted: Vec<u8> = want.to_rgba8().pixels().map(|pixel| pixel[0]).collect();
            assert_eq!(firsts, wanted, "{orientation:?}, five bytes a pixel");
        }
    }

    /// A 16-bit PNG with a Display P3 profile and an orientation keeps its
    /// samples, turned upright, as stored, and displays what the import
    /// loads: converted to sRGB and turned.
    #[test]
    fn keeps_a_pngs_samples_upright_beside_the_pixels_the_import_loads() {
        let samples: Vec<u16> = (0..18).map(|i| (i * 3000 + 17) as u16).collect();
        let file = png16(3, 2, &samples, Some(&reference::DISPLAYP3_PROFILE), Some(&ROTATE90_EXIF));
        let image = decode_image(&file, Some("photo.png"), &DecodeOptions::default()).expect("decoded");
        let frame = only(&image);
        assert_eq!((frame.width, frame.height), (2, 3), "turned upright");
        let mut upright = DynamicImage::ImageRgb16(ImageBuffer::from_raw(3, 2, samples).expect("an image"));
        upright.apply_orientation(Orientation::Rotate90);
        assert_eq!(frame.stored, Pixels { format: PixelFormat::Rgb16, data: upright.as_bytes().to_vec() });
        let transform = Transform::default();
        let asked = Asked { reading: Reading::default(), transform: &transform, password: None, frames: None };
        let loaded = images(Path::new("photo.png"), &file, asked, &mut Said::default()).expect("the import loads it");
        assert_eq!(frame.display, Pixels { format: PixelFormat::Rgb16, data: loaded.frames[0].as_bytes().to_vec() });
        assert_ne!(frame.display.data, frame.stored.data, "Display P3 converts to other values");
        assert!(image.notes.iter().any(|note| note.starts_with("the colors were converted from the file's color profile")), "{:?}", image.notes);
        assert!(image.notes.contains(&"the pixels were turned 90 degrees clockwise, as the file's orientation says".to_string()), "{:?}", image.notes);
        assert_eq!((image.frame_delays.len(), image.loop_count, image.hdr, image.cube, image.volume), (0, None, false, false, false));
        assert!(image.warnings.is_empty(), "{:?}", image.warnings);
    }

    /// An animation's frames decode with its delays and the times it plays.
    #[test]
    fn decodes_an_animations_frames_with_its_timing() {
        use crate::frames_reference as r;
        let image = decode_image(&r::GIF_FILE, None, &DecodeOptions::default()).expect("decoded");
        assert_eq!((image.frame_delays.as_slice(), image.loop_count), (&r::GIF_DELAYS[..], Some(r::GIF_PLAYS)));
        let got: Vec<(u32, u32, &[u8])> = image.frames.iter().map(|frame| (frame.width, frame.height, frame.display.data.as_slice())).collect();
        assert_eq!(got, r::GIF_FRAMES);
        assert!(image.frames.iter().all(|frame| frame.display.format == PixelFormat::Rgba8 && frame.stored == frame.display));
    }

    /// A TIFF's pages decode each at its own size.
    #[test]
    fn decodes_each_page_at_its_own_size() {
        use crate::frames_reference as r;
        let image = decode_image(&r::TIFF_FILE, Some("pages.tif"), &DecodeOptions::default()).expect("decoded");
        let got: Vec<(u32, u32, &[u8])> = image.frames.iter().map(|frame| (frame.width, frame.height, frame.display.data.as_slice())).collect();
        assert_eq!(got, r::TIFF_PAGES);
        assert!(image.frames.iter().all(|frame| frame.stored == frame.display));
    }

    /// A CMYK TIFF keeps its inks as stored and displays the RGB they make.
    #[test]
    fn keeps_a_cmyk_tiffs_inks_as_stored() {
        use tiff::encoder::{colortype::CMYK8, TiffEncoder};
        let inks = vec![0, 0, 0, 0, 255, 0, 0, 0, 0, 128, 255, 64];
        let mut out = std::io::Cursor::new(Vec::new());
        TiffEncoder::new(&mut out).expect("an encoder").write_image::<CMYK8>(3, 1, &inks).expect("the inks");
        let image = decode_image(out.get_ref(), Some("inks.tif"), &DecodeOptions::default()).expect("decoded");
        let frame = only(&image);
        assert_eq!(frame.stored, Pixels { format: PixelFormat::Cmyk8, data: inks });
        assert_eq!(frame.display.format, PixelFormat::Rgb8);
        assert_eq!(frame.display.data[..3], [255, 255, 255], "no ink is white");
    }

    /// A Photoshop document of `mode`, `depth` bits a sample, 2 by 1
    /// pixels, of the raw `planes` given.
    fn document(mode: u16, depth: u16, planes: &[&[u8]]) -> Vec<u8> {
        let mut out = b"8BPS".to_vec();
        out.extend_from_slice(&1u16.to_be_bytes());
        out.extend_from_slice(&[0; 6]);
        out.extend_from_slice(&(planes.len() as u16).to_be_bytes());
        out.extend_from_slice(&1u32.to_be_bytes());
        out.extend_from_slice(&2u32.to_be_bytes());
        out.extend_from_slice(&depth.to_be_bytes());
        out.extend_from_slice(&mode.to_be_bytes());
        // No color mode data, image resources or layers, and a raw composite.
        out.extend_from_slice(&[0; 12]);
        out.extend_from_slice(&0u16.to_be_bytes());
        for plane in planes {
            out.extend_from_slice(plane);
        }
        out
    }

    /// A CIE Lab document keeps its L, a and b as stored and displays them
    /// converted to sRGB.
    #[test]
    fn keeps_a_lab_documents_values_as_stored() {
        let image = decode_image(&document(9, 8, &[&[0, 255], &[128, 20], &[128, 230]]), Some("lab.psd"), &DecodeOptions::default()).expect("decoded");
        let frame = only(&image);
        assert_eq!(frame.stored, Pixels { format: PixelFormat::Lab8, data: vec![0, 128, 128, 255, 20, 230] });
        assert_eq!(frame.display.format, PixelFormat::Rgb8);
        assert_eq!(frame.display.data[..3], [0, 0, 0], "L of 0 is black");
    }

    /// A 32-bit gray document keeps one gray float a pixel as stored.
    #[test]
    fn keeps_a_gray_float_documents_gray_as_stored() {
        let plane: Vec<u8> = [0.25f32, 2.0].iter().flat_map(|v| v.to_be_bytes()).collect();
        let image = decode_image(&document(1, 32, &[&plane]), Some("gray.psd"), &DecodeOptions::default()).expect("decoded");
        let frame = only(&image);
        assert_eq!(frame.stored, Pixels { format: PixelFormat::Gray32F, data: bytemuck::cast_slice::<f32, u8>(&[0.25, 2.0]).to_vec() });
        assert_eq!(frame.display, Pixels { format: PixelFormat::Rgb32F, data: bytemuck::cast_slice::<f32, u8>(&[0.25, 0.25, 0.25, 2.0, 2.0, 2.0]).to_vec() });
    }

    /// Inks keep each pixel's alpha after them, at 8 and 16 bits, and inks
    /// that do not fill the image are refused.
    #[test]
    fn keeps_cmyk_inks_with_their_alpha() {
        let rgba = image::RgbaImage::from_raw(2, 1, vec![1, 2, 3, 40, 5, 6, 7, 80]).expect("an image");
        match as_stored(&DynamicImage::ImageRgba8(rgba), Some(&Cmyk::Eight(vec![10, 20, 30, 40, 50, 60, 70, 80])), false, false) {
            Ok(kept) => assert_eq!(kept.pixels, Pixels { format: PixelFormat::CmykAlpha8, data: vec![10, 20, 30, 40, 40, 50, 60, 70, 80, 80] }),
            Err(e) => panic!("the inks were not kept: {e}"),
        }
        let rgba16 = ImageBuffer::<image::Rgba<u16>, Vec<u16>>::from_raw(1, 1, vec![1, 2, 3, 4000]).expect("an image");
        match as_stored(&DynamicImage::ImageRgba16(rgba16), Some(&Cmyk::Sixteen(vec![100, 200, 300, 400])), false, false) {
            Ok(kept) => assert_eq!(kept.pixels, Pixels { format: PixelFormat::CmykAlpha16, data: bytemuck::cast_slice::<u16, u8>(&[100, 200, 300, 400, 4000]).to_vec() }),
            Err(e) => panic!("the 16-bit inks were not kept: {e}"),
        }
        let rgb = DynamicImage::ImageRgb8(image::RgbImage::new(2, 1));
        match as_stored(&rgb, Some(&Cmyk::Eight(vec![1, 2, 3])), false, false) {
            Ok(_) => panic!("three ink samples were kept for two pixels"),
            Err(e) => assert_eq!(e, "its 3 ink samples do not fill its 2x1 pixels"),
        }
    }

    /// The options a call refuses and those that changed nothing are named
    /// by their fields.
    #[test]
    fn names_the_options_by_their_fields() {
        let file = png16(1, 1, &[1, 2, 3], None, None);
        let pair = |id: &str, message: &str| (id.to_string(), message.to_string());
        let intent = DecodeOptions { ignore_profile: true, rendering_intent: Some(RenderingIntent::Saturation), ..Default::default() };
        assert_eq!(
            refused(&file, None, &intent),
            pair("WgpuRenderingIntent", "rendering_intent chooses how colors convert from a file's color profile, and ignore_profile converts none: give one or the other")
        );
        assert_eq!(refused(&file, None, &DecodeOptions { dpi: Some(0.0), ..Default::default() }), pair("WgpuDpi", "dpi is a number of dots per inch above 0, not 0"));
        assert_eq!(refused(&file, None, &DecodeOptions { scale: Some(-2.0), ..Default::default() }), pair("WgpuResize", "scale is a number above 0, not -2"));
        assert_eq!(refused(&file, None, &DecodeOptions { frames: Some(Vec::new()), ..Default::default() }), pair("WgpuImage", "the file cannot be read as an image: frames names no frame"));
        assert_eq!(
            refused(&file, Some("one.png"), &DecodeOptions { frames: Some(vec![1]), ..Default::default() }),
            pair("WgpuImage", "one.png cannot be read as an image: frames names 1, past its last frame or page, 0")
        );
        let options = DecodeOptions { ignore_orientation: true, ignore_profile: true, scale: Some(2.0), dpi: Some(144.0), no_background: true, password: Some("secret".to_string()), ..Default::default() };
        let image = decode_image(&file, Some("flat.png"), &options).expect("decoded");
        assert_eq!(
            image.warnings,
            [
                "the file is not a PDF, so dpi changed nothing",
                "the file is not a PDF, so no_background changed nothing",
                "the file is not a PDF, so password changed nothing",
                "the file carries no color profile, so ignore_profile changed nothing",
                "the file records no orientation, so ignore_orientation changed nothing",
                "the file is neither an SVG file nor a PDF, so scale changed nothing",
            ]
        );
        let shown = format!("{options:?}");
        assert!(shown.contains("password: Some(\"<hidden>\")") && !shown.contains("secret"), "{shown}");
    }

    /// A texture of `format` and shape, its mip levels' texels given.
    fn texture_file(format: TextureFormat, dimension: TextureDimension, (width, height, depth): (u32, u32, u32), layers: u32, cube: bool, levels: Vec<Vec<u8>>) -> TextureFile {
        TextureFile { dimension, width, height, depth, layers, cube, format, levels }
    }

    /// A DDS or KTX2 file decodes every layer of its largest mip level, its
    /// texels in its format as both its pixels, and refuses `frames`.
    #[test]
    fn decodes_every_layer_of_a_texture_files_largest_level() {
        let level0: Vec<u8> = (0..32).collect();
        let file = texture_file(TextureFormat::Rgba8UnormSrgb, TextureDimension::D2, (2, 2, 1), 2, false, vec![level0.clone(), vec![9; 8]]);
        let dds = texfile::write_dds(&file).expect("a DDS file");
        let image = decode_image(&dds, None, &DecodeOptions::default()).expect("decoded");
        assert_eq!(image.frames.len(), 2);
        for (layer, frame) in image.frames.iter().enumerate() {
            let texels = Pixels { format: PixelFormat::Texels(TextureFormat::Rgba8UnormSrgb), data: level0[layer * 16..(layer + 1) * 16].to_vec() };
            assert_eq!((frame.width, frame.height, &frame.display, &frame.stored), (2, 2, &texels, &texels), "layer {layer}");
        }
        assert_eq!(image.notes, ["the largest of the file's 2 mip levels was decoded", "the texture's 2 layers decoded as frames"]);
        let ktx2 = texfile::write_ktx2(&file, None).expect("a KTX2 file");
        assert_eq!(decode_image(&ktx2, None, &DecodeOptions::default()).expect("decoded").frames, image.frames);
        assert_eq!(
            refused(&dds, None, &DecodeOptions { frames: Some(vec![0]), ..Default::default() }),
            ("WgpuFrame".to_string(), "frames picks the frames or pages of an image file, and a DDS file decodes every layer, face and slice of its largest mip level".to_string())
        );
        let unused = decode_image(&dds, None, &DecodeOptions { ignore_profile: true, ..Default::default() }).expect("decoded");
        assert_eq!(unused.warnings, ["a DDS file carries no color profile, so ignore_profile changed nothing"]);
    }

    /// A block-compressed cube map decodes its faces on the CPU, and a 3D
    /// texture its depth slices.
    #[test]
    fn decodes_cube_faces_and_depth_slices() {
        // A BC1 block whose every texel takes color 0: pure red in RGB565.
        const RED_BC1: [u8; 8] = [0x00, 0xF8, 0x1F, 0x00, 0, 0, 0, 0];
        let cube = texture_file(TextureFormat::Bc1RgbaUnorm, TextureDimension::D2, (4, 4, 1), 6, true, vec![RED_BC1.repeat(6)]);
        let image = decode_image(&texfile::write_dds(&cube).expect("a DDS file"), Some("sky.dds"), &DecodeOptions::default()).expect("decoded");
        assert!(image.cube && !image.volume);
        let red = Pixels { format: PixelFormat::Texels(TextureFormat::Rgba8Unorm), data: [255, 0, 0, 255].repeat(16) };
        assert_eq!(image.frames.len(), 6);
        assert!(image.frames.iter().all(|frame| frame.display == red && frame.stored == red));
        assert_eq!(image.notes, ["the file's Bc1RgbaUnorm texels were decoded on the CPU into Rgba8Unorm", "the texture's 6 cube faces decoded as frames"]);
        let volume = texture_file(TextureFormat::R8Unorm, TextureDimension::D3, (1, 1, 3), 1, false, vec![vec![10, 20, 30]]);
        let slices = decode_image(&texfile::write_ktx2(&volume, None).expect("a KTX2 file"), None, &DecodeOptions::default()).expect("decoded");
        assert!(slices.volume && !slices.cube);
        let got: Vec<&[u8]> = slices.frames.iter().map(|frame| frame.display.data.as_slice()).collect();
        assert_eq!(got, [&[10u8][..], &[20], &[30]]);
        assert_eq!(slices.frames[0].display.format.pixel_bytes(), Some(1));
    }

    /// The sizes and alpha of the formats a decode gives.
    #[test]
    fn tells_each_formats_size_and_alpha() {
        assert_eq!((PixelFormat::CmykAlpha16.channels(), PixelFormat::CmykAlpha16.pixel_bytes(), PixelFormat::CmykAlpha16.has_alpha()), (5, Some(10), true));
        assert_eq!((PixelFormat::Rgb32F.channels(), PixelFormat::Rgb32F.pixel_bytes(), PixelFormat::Rgb32F.has_alpha()), (3, Some(12), false));
        let texels = PixelFormat::Texels(TextureFormat::Rgba16Float);
        assert_eq!((texels.channels(), texels.pixel_bytes(), texels.has_alpha()), (4, Some(8), true));
        assert_eq!(PixelFormat::Texels(TextureFormat::Bc7RgbaUnorm).pixel_bytes(), None);
    }

    /// An SVG file draws at a scale, its drawing both its pixels.
    #[cfg(feature = "svg")]
    #[test]
    fn draws_an_svg_file_at_a_scale() {
        let shapes: &[u8] = include_bytes!("../../tests/svg/shapes.svg");
        let image = decode_image(shapes, Some("shapes.svg"), &DecodeOptions { scale: Some(0.5), ..Default::default() }).expect("drawn");
        let want = match image::load_from_memory(include_bytes!("../../tests/svg/shapes-half.png")) {
            Ok(want) => want.to_rgba8(),
            Err(e) => panic!("the reference decodes: {e}"),
        };
        let frame = only(&image);
        assert_eq!((frame.width, frame.height), want.dimensions());
        assert_eq!(frame.display, Pixels { format: PixelFormat::Rgba8, data: want.into_raw() });
        assert_eq!(frame.stored, frame.display);
        assert!(image.warnings.is_empty(), "{:?}", image.warnings);
    }

    /// A protected PDF opens with the password option, and its refusal
    /// without one names it.
    #[cfg(feature = "pdf")]
    #[test]
    fn opens_a_protected_pdf_with_the_password_option() {
        use crate::pdf_reference as r;
        assert_eq!(
            refused(&r::PROTECTED_FILE, Some("protected.pdf"), &DecodeOptions::default()),
            ("WgpuImage".to_string(), "protected.pdf cannot be read as an image: it is protected by a password: give the options a password".to_string())
        );
        let opened = decode_image(&r::PROTECTED_FILE, Some("protected.pdf"), &DecodeOptions { password: Some(r::PASSWORD.to_string()), ..Default::default() }).expect("opened");
        assert_eq!(opened.frames.len(), 1);
    }
}
