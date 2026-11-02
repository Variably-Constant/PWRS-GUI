//! Channels and color: what one call changes in each texel after its size
//! and orientation, always in this order: the color space the texels are
//! read in, a color key, a swizzle, a normal's Z rebuilt, normals scaled
//! to length 1, green flipped, unpremultiplied alpha, an alpha threshold,
//! exposure, a tone-mapping curve, brightness and contrast, gamma,
//! grayscale, inversion, premultiplied alpha, and the color space they are
//! written in. The
//! changes run in the transform's last render pass, in 32-bit floats. The
//! passes work in the values a shader reads, linear for an sRGB texture; a
//! swizzle, the normal steps, brightness and contrast, gamma, Rec. 601
//! grayscale and inversion act on the values the texture stores, which for
//! an sRGB texture the pass encodes and decodes again around each of them.
//! Join-WgpuTexture packs channels of several textures, or numbers, into
//! one new texture.

use pwrs::prelude::*;
use wgpu::TextureFormat as F;

use crate::device::CoreRef;
use crate::error::invalid;
use crate::image_io;
use crate::normal;
use crate::tasks::Interrupt;
use crate::texfile::{self, Said};
use crate::texture::{format_name, tell, Texture};
use crate::transform;

/// How a texture's texels encode color.
#[psenum(name = "Wgpu.ColorSpace")]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum ColorSpace {
    /// Linear values, which a shader reads as they are stored.
    #[default]
    Linear,
    /// sRGB-encoded values, which a shader reads decoded into linear ones.
    Srgb,
}

impl ColorSpace {
    fn name(self) -> &'static str {
        match self {
            ColorSpace::Linear => "linear",
            ColorSpace::Srgb => "sRGB",
        }
    }
}

/// A curve that maps HDR colors into the range from 0 to 1.
#[psenum(name = "Wgpu.ToneMapCurve")]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum ToneMapCurve {
    /// Reinhard's, with the brightest texel's luminance as the white point,
    /// as texconv applies it.
    #[default]
    Reinhard,
    /// ACES Filmic, as three.js fits it.
    AcesFilmic,
    /// AgX, as three.js computes it.
    AgX,
    /// Khronos PBR Neutral, which keeps hues and changes colors below its
    /// compression point little.
    Neutral,
}

impl ToneMapCurve {
    /// The number the shader knows the curve by.
    fn code(self) -> u32 {
        match self {
            ToneMapCurve::Reinhard => 0,
            ToneMapCurve::AcesFilmic => 1,
            ToneMapCurve::AgX => 2,
            ToneMapCurve::Neutral => 3,
        }
    }
}

/// The weights -Grayscale gives red, green and blue, and the values it
/// weighs.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Gray {
    /// Rec. 709's, on linear values.
    Rec709,
    /// Rec. 601's, on stored values.
    Rec601,
    /// The caller's, on linear values.
    Weights([f32; 3]),
}

/// A color whose texels become transparent: red, green and blue as the
/// texture stores them, from 0 to 1, and how far each channel may be from
/// them, in the same units.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct ColorKey {
    pub(crate) rgb: [f32; 3],
    pub(crate) tolerance: f32,
}

/// The per-texel changes one call makes, applied after its size and
/// orientation in the order the fields are listed.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct Color {
    /// The color space the texels are read in, which relabels them.
    pub(crate) assign: Option<ColorSpace>,
    /// Texels whose stored colors match the key get alpha 0; others keep
    /// theirs.
    pub(crate) key: Option<ColorKey>,
    /// For each channel of the new texel, the channel it takes, 0 to 3 for
    /// red to alpha, or 4 for 0 and 5 for 1.
    pub(crate) swizzle: Option<[u8; 4]>,
    /// A normal's Z rebuilt from its X and Y, each normal scaled to length
    /// 1, and green flipped: on the values the texture stores.
    pub(crate) reconstruct_z: bool,
    pub(crate) normalize: bool,
    pub(crate) flip_green: bool,
    pub(crate) unpremultiply: bool,
    /// Alpha below this becomes 0, and alpha at or above it 1.
    pub(crate) threshold: Option<f32>,
    /// Stops: linear values are multiplied by 2 to this power.
    pub(crate) exposure: Option<f32>,
    pub(crate) tone_map: Option<ToneMapCurve>,
    /// From -100 to 100, as ImageMagick's -brightness-contrast.
    pub(crate) brightness: Option<f32>,
    pub(crate) contrast: Option<f32>,
    /// Stored values become value^(1/gamma), as ImageMagick's -gamma.
    pub(crate) gamma: Option<f32>,
    pub(crate) grayscale: Option<Gray>,
    pub(crate) invert: bool,
    pub(crate) premultiply: bool,
    /// The color space the texels are written in, which converts them.
    pub(crate) space: Option<ColorSpace>,
}

/// The letters of a swizzle, by the channel numbers `Color::swizzle` holds.
const SWIZZLE_LETTERS: [char; 6] = ['r', 'g', 'b', 'a', '0', '1'];

/// The swizzle a mask names: four letters of r, g, b, a, 0 and 1.
pub(crate) fn swizzle_of(mask: &str) -> PsResult<[u8; 4]> {
    let refuse = || invalid("WgpuSwizzle", format!("-Swizzle takes four letters of r, g, b, a, 0 and 1, one for each channel of the new texture, as 'bgra' or 'rrr1'; '{mask}' is not that"));
    let letters: Vec<char> = mask.chars().map(|c| c.to_ascii_lowercase()).collect();
    let [r, g, b, a] = letters.as_slice() else {
        return Err(refuse());
    };
    let mut out = [0u8; 4];
    for (slot, letter) in out.iter_mut().zip([r, g, b, a]) {
        match SWIZZLE_LETTERS.iter().position(|l| l == letter) {
            Some(i) => *slot = i as u8,
            None => return Err(refuse()),
        }
    }
    Ok(out)
}

/// The grayscale -Grayscale and -GrayscaleWeights ask for: Rec. 709 when
/// no weights are named.
pub(crate) fn gray_of(grayscale: bool, weights: Option<&PsObject>) -> PsResult<Option<Gray>> {
    let Some(weights) = weights.filter(|w| !w.is_null()) else {
        return Ok(grayscale.then_some(Gray::Rec709));
    };
    if !grayscale {
        return Err(invalid("WgpuGrayscale", "-GrayscaleWeights picks the weights of -Grayscale: give -Grayscale"));
    }
    weights_of(weights).map(Some)
}

/// The weights a value names: Rec709, Rec601, or three numbers for red,
/// green and blue.
pub(crate) fn weights_of(value: &PsObject) -> PsResult<Gray> {
    let type_name = value.type_name()?;
    if type_name == "System.String" {
        let name = String::from_ps(value)?;
        return match name.to_ascii_lowercase().as_str() {
            "rec709" => Ok(Gray::Rec709),
            "rec601" => Ok(Gray::Rec601),
            _ => Err(invalid("WgpuGrayscale", format!("grayscale weights are Rec709, Rec601 or three numbers for red, green and blue, not '{name}'"))),
        };
    }
    let items = if type_name.ends_with("[]") { Vec::<PsObject>::from_ps(value)? } else { vec![value.clone()] };
    let mut numbers = Vec::with_capacity(items.len());
    for item in &items {
        numbers.push(f64::from_ps(item)?);
    }
    match numbers.as_slice() {
        [r, g, b] if numbers.iter().all(|n| n.is_finite()) => Ok(Gray::Weights([*r as f32, *g as f32, *b as f32])),
        _ => Err(invalid("WgpuGrayscale", format!("grayscale weights are Rec709, Rec601 or three finite numbers for red, green and blue; {} numbers were given", numbers.len()))),
    }
}

/// -Brightness or -Contrast, named `name`: from -100 to 100.
pub(crate) fn percent_of(value: Option<f64>, name: &str) -> PsResult<Option<f32>> {
    match value {
        None => Ok(None),
        Some(v) if v.is_finite() && (-100.0..=100.0).contains(&v) => Ok(Some(v as f32)),
        Some(v) => Err(invalid("WgpuAdjust", format!("{name} runs from -100 to 100, as ImageMagick's -brightness-contrast, not {v}"))),
    }
}

/// -Gamma: a number above 0.
pub(crate) fn gamma_of(value: Option<f64>) -> PsResult<Option<f32>> {
    match value {
        None => Ok(None),
        Some(v) if v.is_finite() && v > 0.0 => Ok(Some(v as f32)),
        Some(v) => Err(invalid("WgpuAdjust", format!("-Gamma is a number above 0, not {v}"))),
    }
}

/// -Exposure: a finite number of stops.
pub(crate) fn exposure_of(value: Option<f64>) -> PsResult<Option<f32>> {
    match value {
        None => Ok(None),
        Some(v) if v.is_finite() => Ok(Some(v as f32)),
        Some(v) => Err(invalid("WgpuAdjust", format!("-Exposure is a finite number of stops, not {v}"))),
    }
}

/// -AlphaThreshold: above 0 and at most 1.
pub(crate) fn threshold_of(value: Option<f64>) -> PsResult<Option<f32>> {
    match value {
        None => Ok(None),
        Some(v) if v > 0.0 && v <= 1.0 => Ok(Some(v as f32)),
        Some(v) => Err(invalid("WgpuAlphaThreshold", format!("-AlphaThreshold is above 0 and at most 1, not {v}"))),
    }
}

/// -ColorKey and -ColorKeyTolerance: red, green and blue from 0 to 1, or
/// '#RRGGBB', and a tolerance from 0 to 1, 0 when omitted.
pub(crate) fn key_of(key: Option<&PsObject>, tolerance: Option<f64>) -> PsResult<Option<ColorKey>> {
    let Some(key) = key.filter(|k| !k.is_null()) else {
        if tolerance.is_some() {
            return Err(invalid("WgpuColorKey", "-ColorKeyTolerance widens -ColorKey: give -ColorKey"));
        }
        return Ok(None);
    };
    let tolerance = match tolerance {
        None => 0.0,
        Some(t) if (0.0..=1.0).contains(&t) => t as f32,
        Some(t) => return Err(invalid("WgpuColorKey", format!("-ColorKeyTolerance runs from 0 to 1, in the units of the key's channels, not {t}"))),
    };
    let rgb = if key.type_name()? == "System.String" { hex_of(&String::from_ps(key)?)? } else { numbers_of(key)? };
    Ok(Some(ColorKey { rgb, tolerance }))
}

/// A key written '#RRGGBB'.
fn hex_of(text: &str) -> PsResult<[f32; 3]> {
    let refuse = || invalid("WgpuColorKey", format!("-ColorKey is red, green and blue from 0 to 1, or '#RRGGBB', as '#FF00FF'; '{text}' is not that"));
    let digits = text.strip_prefix('#').ok_or_else(refuse)?;
    if digits.len() != 6 || !digits.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(refuse());
    }
    let mut rgb = [0.0f32; 3];
    for (slot, at) in rgb.iter_mut().zip([0, 2, 4]) {
        match u8::from_str_radix(&digits[at..at + 2], 16) {
            Ok(byte) => *slot = f32::from(byte) / 255.0,
            Err(e) => return Err(invalid("WgpuColorKey", format!("-ColorKey '{text}' is not hexadecimal: {e}"))),
        }
    }
    Ok(rgb)
}

/// A key written as three numbers from 0 to 1.
fn numbers_of(key: &PsObject) -> PsResult<[f32; 3]> {
    let items = if key.type_name()?.ends_with("[]") { Vec::<PsObject>::from_ps(key)? } else { vec![key.clone()] };
    let mut numbers = Vec::new();
    for item in &items {
        numbers.push(f64::from_ps(item)?);
    }
    match numbers.as_slice() {
        [r, g, b] if numbers.iter().all(|n| (0.0..=1.0).contains(n)) => Ok([*r as f32, *g as f32, *b as f32]),
        _ => Err(invalid(
            "WgpuColorKey",
            format!("-ColorKey is red, green and blue from 0 to 1, or '#RRGGBB'; {} numbers were given, {}", numbers.len(), numbers.iter().map(f64::to_string).collect::<Vec<_>>().join(", ")),
        )),
    }
}

/// The largest value a channel of `format` stores as an integer, for a
/// normalized integer format and the compressed formats that decode to 8
/// bits a channel; `None` for float formats, whose values are their own.
fn unorm_steps(format: F) -> Option<f32> {
    match format {
        F::R8Unorm | F::Rg8Unorm | F::Rgba8Unorm | F::Rgba8UnormSrgb | F::Bgra8Unorm | F::Bgra8UnormSrgb => Some(255.0),
        F::Bc1RgbaUnorm | F::Bc1RgbaUnormSrgb | F::Bc2RgbaUnorm | F::Bc2RgbaUnormSrgb | F::Bc3RgbaUnorm | F::Bc3RgbaUnormSrgb | F::Bc7RgbaUnorm | F::Bc7RgbaUnormSrgb => Some(255.0),
        F::Etc2Rgb8Unorm | F::Etc2Rgb8UnormSrgb | F::Etc2Rgb8A1Unorm | F::Etc2Rgb8A1UnormSrgb | F::Etc2Rgba8Unorm | F::Etc2Rgba8UnormSrgb | F::Astc { .. } => Some(255.0),
        F::R16Unorm | F::Rg16Unorm | F::Rgba16Unorm => Some(65535.0),
        F::Rgb10a2Unorm => Some(1023.0),
        _ => None,
    }
}

/// The bits of the color pass's `flags`, which the shader declares by the
/// same names.
pub(crate) const ASSIGN_DECODE: u32 = 1;
pub(crate) const ASSIGN_ENCODE: u32 = 1 << 1;
pub(crate) const SRGB: u32 = 1 << 2;
pub(crate) const SWIZZLE: u32 = 1 << 3;
pub(crate) const UNPREMULTIPLY: u32 = 1 << 4;
pub(crate) const TONE_MAP: u32 = 1 << 5;
pub(crate) const ADJUST: u32 = 1 << 6;
pub(crate) const GAMMA: u32 = 1 << 7;
pub(crate) const GRAY_LINEAR: u32 = 1 << 8;
pub(crate) const GRAY_STORED: u32 = 1 << 9;
pub(crate) const INVERT: u32 = 1 << 10;
pub(crate) const PREMULTIPLY: u32 = 1 << 11;
pub(crate) const OUT_ENCODE: u32 = 1 << 12;
pub(crate) const OUT_DECODE: u32 = 1 << 13;
pub(crate) const COLOR_KEY: u32 = 1 << 14;
pub(crate) const THRESHOLD: u32 = 1 << 15;
pub(crate) const RECONSTRUCT_Z: u32 = 1 << 16;
pub(crate) const NORMALIZE: u32 = 1 << 17;
pub(crate) const FLIP_GREEN: u32 = 1 << 18;
/// The source stores normals as they are, as a format that holds negative
/// values does, and not as n / 2 + 1 / 2.
pub(crate) const READ_SIGNED: u32 = 1 << 19;
/// The texels are written in such a format.
pub(crate) const WRITE_SIGNED: u32 = 1 << 20;

const FLAGS: [(&str, u32); 21] = [
    ("ASSIGN_DECODE", ASSIGN_DECODE),
    ("ASSIGN_ENCODE", ASSIGN_ENCODE),
    ("SRGB", SRGB),
    ("SWIZZLE", SWIZZLE),
    ("UNPREMULTIPLY", UNPREMULTIPLY),
    ("TONE_MAP", TONE_MAP),
    ("ADJUST", ADJUST),
    ("GAMMA", GAMMA),
    ("GRAY_LINEAR", GRAY_LINEAR),
    ("GRAY_STORED", GRAY_STORED),
    ("INVERT", INVERT),
    ("PREMULTIPLY", PREMULTIPLY),
    ("OUT_ENCODE", OUT_ENCODE),
    ("OUT_DECODE", OUT_DECODE),
    ("COLOR_KEY", COLOR_KEY),
    ("THRESHOLD", THRESHOLD),
    ("RECONSTRUCT_Z", RECONSTRUCT_Z),
    ("NORMALIZE", NORMALIZE),
    ("FLIP_GREEN", FLIP_GREEN),
    ("READ_SIGNED", READ_SIGNED),
    ("WRITE_SIGNED", WRITE_SIGNED),
];

/// ImageMagick's MagickEpsilon, below which its MagickSafeReciprocal
/// keeps a divisor.
const MAGICK_EPSILON: f64 = 1.0e-12;

/// ImageMagick's -brightness-contrast as the line its BrightnessContrast
/// applies to each stored value: the slope and the intercept.
pub(crate) fn brightness_contrast(brightness: f64, contrast: f64) -> (f64, f64) {
    let slope = if contrast < 0.0 {
        0.01 * contrast + 1.0
    } else {
        let d = 100.0 - contrast;
        100.0 * if d.abs() >= MAGICK_EPSILON { 1.0 / d } else { d.signum() / MAGICK_EPSILON }
    };
    (slope, (0.01 * brightness - 0.5) * slope + 0.5)
}

/// The linear and sRGB forms of a format that has both.
pub(crate) fn forms(format: F) -> Option<(F, F)> {
    let (linear, srgb) = (format.remove_srgb_suffix(), format.add_srgb_suffix());
    (linear != srgb).then_some((linear, srgb))
}

/// What the color pass is given: its uniforms, the format the texels are
/// written in, and lines saying what a color-space request did.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Prepared {
    pub(crate) flags: u32,
    pub(crate) swizzle: [u32; 4],
    pub(crate) weights: [f32; 4],
    pub(crate) curve: u32,
    pub(crate) exposure: f32,
    pub(crate) slope: f32,
    pub(crate) intercept: f32,
    pub(crate) gamma: f32,
    /// The color key's stored red, green and blue, rounded to the
    /// format's precision, and how far a channel may be from them, half a
    /// step of that precision included.
    pub(crate) key: [f32; 4],
    pub(crate) threshold: f32,
    pub(crate) format: F,
    pub(crate) said: Vec<String>,
}

impl Prepared {
    /// Whether the pass changes any texel.
    pub(crate) fn changes(&self) -> bool {
        self.flags & !SRGB != 0 || self.exposure != 1.0
    }

    /// Whether the pass needs the brightest texel's luminance.
    pub(crate) fn reinhard(&self) -> bool {
        self.flags & TONE_MAP != 0 && self.curve == ToneMapCurve::Reinhard.code()
    }
}

impl Color {
    pub(crate) fn is_empty(&self) -> bool {
        *self == Color::default()
    }

    /// Whether the changes write normals: -ReconstructZ, -Normalize or
    /// -FlipGreen.
    pub(crate) fn writes_normals(&self) -> bool {
        self.reconstruct_z || self.normalize || self.flip_green
    }

    /// The pass that makes these changes to texels of `source`, read as
    /// sRGB-encoded where it is an sRGB format, written in `format`, from a
    /// source that stores normals as they are when `read_signed`: the
    /// format they are written in after -AssignColorSpace and -ColorSpace,
    /// and the flags that say what the shader does. With `retarget`,
    /// `format` is the uncompressed form of a -Format the texels are
    /// converted into after the pass: -AssignColorSpace then says how the
    /// source reads and leaves `format` as it is, and -ColorSpace settles
    /// against `format` rather than the source.
    pub(crate) fn prepare(&self, source: F, format: F, read_signed: bool, retarget: bool) -> PsResult<Prepared> {
        let mut out = Prepared {
            flags: 0,
            swizzle: [0, 1, 2, 3],
            weights: [0.0; 4],
            curve: 0,
            exposure: 1.0,
            slope: 1.0,
            intercept: 0.0,
            gamma: 1.0,
            key: [0.0; 4],
            threshold: 0.0,
            format,
            said: Vec::new(),
        };
        if (self.key.is_some() || self.threshold.is_some()) && format.components() < 4 {
            let parameter = if self.key.is_some() { "-ColorKey" } else { "-AlphaThreshold" };
            return Err(invalid("WgpuAlpha", format!("{} has no alpha channel, so {parameter} would change nothing", format_name(format))));
        }
        let mut srgb = source.is_srgb();
        // The source's own uncompressed form, which says whether it has an
        // sRGB form to be relabeled as.
        let own = if retarget { texfile::decoded_format(source).unwrap_or(source) } else { format };
        if let Some(space) = self.assign {
            let Some((linear, encoded)) = forms(own) else {
                return Err(invalid(
                    "WgpuColorSpace",
                    format!("-AssignColorSpace relabels a format that has an sRGB form, as Rgba8Unorm has Rgba8UnormSrgb, and {} has none", format_name(own)),
                ));
            };
            let want = space == ColorSpace::Srgb;
            if want == srgb {
                out.said.push(format!("the texels were already read as {}, so -AssignColorSpace changed nothing", space.name()));
            } else {
                out.flags |= if want { ASSIGN_DECODE } else { ASSIGN_ENCODE };
                if retarget {
                    out.said.push(format!("the texels were read as {}, as -AssignColorSpace says", space.name()));
                } else {
                    out.format = if want { encoded } else { linear };
                    out.said.push(format!("the texels were relabeled as {}, which makes the texture {}", space.name(), format_name(out.format)));
                }
                srgb = want;
            }
        }
        if srgb {
            out.flags |= SRGB;
        }
        if let Some(key) = self.key {
            out.flags |= COLOR_KEY;
            out.key = match unorm_steps(out.format) {
                Some(steps) => {
                    let [r, g, b] = key.rgb.map(|c| (c * steps).round() / steps);
                    [r, g, b, key.tolerance + 0.5 / steps]
                }
                None => [key.rgb[0], key.rgb[1], key.rgb[2], key.tolerance],
            };
        }
        if let Some(swizzle) = self.swizzle {
            out.flags |= SWIZZLE;
            out.swizzle = swizzle.map(u32::from);
        }
        let needs = [(self.reconstruct_z, 3, "-ReconstructZ", "blue channel to hold the Z it rebuilds"), (self.normalize, 3, "-Normalize", "Z for it to scale with X and Y"), (self.flip_green, 2, "-FlipGreen", "green channel")];
        for (asked, channels, parameter, what) in needs {
            if asked && out.format.components() < channels {
                return Err(invalid(
                    "WgpuNormalMap",
                    format!("{} has no {what}, so {parameter} would change nothing; Import-WgpuImage and Export-WgpuImage write the texels into a -Format that has it", format_name(out.format)),
                ));
            }
        }
        if self.writes_normals() {
            out.flags |= if read_signed { READ_SIGNED } else { 0 };
            out.flags |= if normal::signed(out.format) { WRITE_SIGNED } else { 0 };
        }
        if self.reconstruct_z {
            out.flags |= RECONSTRUCT_Z;
        }
        if self.normalize {
            out.flags |= NORMALIZE;
        }
        if self.flip_green {
            out.flags |= FLIP_GREEN;
        }
        if self.unpremultiply {
            out.flags |= UNPREMULTIPLY;
        }
        if let Some(threshold) = self.threshold {
            out.flags |= THRESHOLD;
            out.threshold = threshold;
        }
        if let Some(stops) = self.exposure {
            out.exposure = stops.exp2();
        }
        if let Some(curve) = self.tone_map {
            out.flags |= TONE_MAP;
            out.curve = curve.code();
        }
        if self.brightness.is_some() || self.contrast.is_some() {
            let (slope, intercept) = brightness_contrast(f64::from(self.brightness.unwrap_or(0.0)), f64::from(self.contrast.unwrap_or(0.0)));
            out.flags |= ADJUST;
            out.slope = slope as f32;
            out.intercept = intercept as f32;
        }
        // ImageMagick's -gamma leaves the values as they are at 1.
        if let Some(gamma) = self.gamma.filter(|g| *g != 1.0) {
            out.flags |= GAMMA;
            out.gamma = 1.0 / gamma;
        }
        match self.grayscale {
            Some(Gray::Rec709) => {
                out.flags |= GRAY_LINEAR;
                out.weights = [0.2126, 0.7152, 0.0722, 0.0];
            }
            Some(Gray::Rec601) => {
                out.flags |= GRAY_STORED;
                out.weights = [0.299, 0.587, 0.114, 0.0];
            }
            Some(Gray::Weights([r, g, b])) => {
                out.flags |= GRAY_LINEAR;
                out.weights = [r, g, b, 0.0];
            }
            None => {}
        }
        if self.invert {
            out.flags |= INVERT;
        }
        if self.premultiply {
            out.flags |= PREMULTIPLY;
        }
        if let Some(space) = self.space {
            let want = space == ColorSpace::Srgb;
            match forms(out.format) {
                Some((linear, encoded)) => {
                    if want == out.format.is_srgb() {
                        out.said.push(format!("the texture was already {}, so -ColorSpace changed nothing", space.name()));
                    } else {
                        out.format = if want { encoded } else { linear };
                        out.said.push(format!("the texels were converted to {}, which makes the texture {}", space.name(), format_name(out.format)));
                    }
                }
                // A source with an sRGB form is read in linear light, as its
                // label says, so its values are linear already.
                None if !want && retarget && forms(own).is_some() => {
                    out.said.push(format!("the texels were read in linear light already, so -ColorSpace Linear changed nothing in {}", format_name(out.format)));
                }
                None => {
                    out.flags |= if want { OUT_ENCODE } else { OUT_DECODE };
                    out.said.push(format!(
                        "the texels were {} and the texture stays {}, a format that records no color space",
                        if want { "encoded as sRGB" } else { "decoded from sRGB" },
                        format_name(out.format)
                    ));
                }
            }
        }
        Ok(out)
    }

    /// What the pass did, in the order it did it, for a note.
    pub(crate) fn describe(&self, prepared: &Prepared) -> Vec<String> {
        let mut done = Vec::new();
        if let Some(key) = self.key {
            let [r, g, b] = key.rgb;
            let within = if key.tolerance > 0.0 { format!(", within {}", key.tolerance) } else { String::new() };
            done.push(format!("made transparent where red, green and blue were {r}, {g} and {b}{within}"));
        }
        if let Some(swizzle) = self.swizzle {
            done.push(format!("swizzled to {}", swizzle.iter().map(|&i| SWIZZLE_LETTERS[usize::from(i)]).collect::<String>()));
        }
        if self.reconstruct_z {
            done.push("given each normal's Z rebuilt from its X and Y".to_string());
        }
        if self.normalize {
            done.push("given normals of length 1".to_string());
        }
        if self.flip_green {
            done.push("given normals with green flipped".to_string());
        }
        let (reads_signed, writes_signed) = (prepared.flags & READ_SIGNED != 0, prepared.flags & WRITE_SIGNED != 0);
        if self.writes_normals() && reads_signed != writes_signed {
            let (from, to) = if reads_signed { ("as they are", "as n / 2 + 1 / 2") } else { ("as n / 2 + 1 / 2", "as they are") };
            done.push(format!("read with normals stored {from} and written with them stored {to}, as {} stores them", format_name(prepared.format)));
        }
        if self.unpremultiply {
            done.push("unpremultiplied".to_string());
        }
        if let Some(threshold) = self.threshold {
            done.push(format!("given alpha 0 below {threshold} and 1 from it"));
        }
        if let Some(stops) = self.exposure {
            done.push(format!("exposed by {stops:+} {}", if stops.abs() == 1.0 { "stop" } else { "stops" }));
        }
        match self.tone_map {
            Some(ToneMapCurve::Reinhard) => done.push("tone mapped with Reinhard's curve".to_string()),
            Some(ToneMapCurve::AcesFilmic) => done.push("tone mapped with ACES Filmic".to_string()),
            Some(ToneMapCurve::AgX) => done.push("tone mapped with AgX".to_string()),
            Some(ToneMapCurve::Neutral) => done.push("tone mapped with Khronos PBR Neutral".to_string()),
            None => {}
        }
        match (self.brightness, self.contrast) {
            (Some(b), Some(c)) => done.push(format!("given brightness {b} and contrast {c}, as ImageMagick's -brightness-contrast")),
            (Some(b), None) => done.push(format!("given brightness {b}, as ImageMagick's -brightness-contrast")),
            (None, Some(c)) => done.push(format!("given contrast {c}, as ImageMagick's -brightness-contrast")),
            (None, None) => {}
        }
        if prepared.flags & GAMMA != 0
            && let Some(gamma) = self.gamma
        {
            done.push(format!("gamma corrected by {gamma}, as ImageMagick's -gamma"));
        }
        match self.grayscale {
            Some(Gray::Rec709) => done.push("made grayscale with Rec. 709's weights in linear light".to_string()),
            Some(Gray::Rec601) => done.push("made grayscale with Rec. 601's weights on the stored values".to_string()),
            Some(Gray::Weights([r, g, b])) => done.push(format!("made grayscale with the weights {r}, {g} and {b} in linear light")),
            None => {}
        }
        if self.invert {
            done.push("inverted".to_string());
        }
        if self.premultiply {
            done.push("premultiplied".to_string());
        }
        done
    }
}

/// The shader's color functions, after the constants `wgsl` writes: the
/// sRGB transfer functions, the tone-mapping curves, and every change in
/// order. They read the transform's uniforms, `p`.
const COLOR: &str = "
// sRGB's transfer functions, each value's sign kept.
fn encode1(c: f32) -> f32 {
    let a = abs(c);
    let e = select(1.055 * pow(a, 1.0 / 2.4) - 0.055, a * 12.92, a <= 0.0031308);
    return select(e, -e, c < 0.0);
}

fn decode1(c: f32) -> f32 {
    let a = abs(c);
    let d = select(pow((a + 0.055) / 1.055, 2.4), a / 12.92, a <= 0.04045);
    return select(d, -d, c < 0.0);
}

fn encode(c: vec3<f32>) -> vec3<f32> {
    return vec3<f32>(encode1(c.r), encode1(c.g), encode1(c.b));
}

fn decode(c: vec3<f32>) -> vec3<f32> {
    return vec3<f32>(decode1(c.r), decode1(c.g), decode1(c.b));
}

// The values a texel stores, from the values the passes work in, and back.
fn stored(v: vec4<f32>) -> vec4<f32> {
    if ((p.flags & SRGB) != 0u) {
        return vec4<f32>(encode(v.rgb), v.a);
    }
    return v;
}

fn working(s: vec4<f32>) -> vec4<f32> {
    if ((p.flags & SRGB) != 0u) {
        return vec4<f32>(decode(s.rgb), s.a);
    }
    return s;
}

// The channel of `s` a swizzle letter names: 0 to 3 for red to alpha, 4
// for 0 and 5 for 1.
fn pick(s: vec4<f32>, which: u32) -> f32 {
    if (which < 4u) {
        return s[which];
    }
    return select(1.0, 0.0, which == 4u);
}

// texconv's Reinhard: each channel times (1 + c / white) over (1 + c),
// white the square of the brightest texel's luminance.
fn reinhard(c: vec3<f32>) -> vec3<f32> {
    let over = select(0.0, 1.0 / p.white, p.white > 0.0);
    return c * (1.0 + c * over) / (1.0 + c);
}

// three.js's ACES Filmic: Stephen Hill's fit of the RRT and ODT, scaled by
// 1 / 0.6.
fn rrt_and_odt(v: vec3<f32>) -> vec3<f32> {
    let a = v * (v + 0.0245786) - 0.000090537;
    let b = v * (0.983729 * v + 0.4329510) + 0.238081;
    return a / b;
}

fn aces(c: vec3<f32>) -> vec3<f32> {
    let to_ap1 = mat3x3<f32>(
        vec3<f32>(0.59719, 0.07600, 0.02840),
        vec3<f32>(0.35458, 0.90834, 0.13383),
        vec3<f32>(0.04823, 0.01566, 0.83777),
    );
    let from_ap1 = mat3x3<f32>(
        vec3<f32>(1.60475, -0.10208, -0.00327),
        vec3<f32>(-0.53108, 1.10813, -0.07276),
        vec3<f32>(-0.07367, -0.00605, 1.07602),
    );
    return clamp(from_ap1 * rrt_and_odt(to_ap1 * (c / 0.6)), vec3<f32>(0.0), vec3<f32>(1.0));
}

// three.js's AgX: Filament's, after Blender's, in Rec. 2020 primaries,
// with the sigmoid's polynomial fit.
fn agx_contrast(x: vec3<f32>) -> vec3<f32> {
    let x2 = x * x;
    let x4 = x2 * x2;
    return 15.5 * x4 * x2 - 40.14 * x4 * x + 31.96 * x4 - 6.868 * x2 * x + 0.4298 * x2 + 0.1191 * x - 0.00232;
}

fn agx(c: vec3<f32>) -> vec3<f32> {
    let srgb_to_2020 = mat3x3<f32>(
        vec3<f32>(0.6274, 0.0691, 0.0164),
        vec3<f32>(0.3293, 0.9195, 0.0880),
        vec3<f32>(0.0433, 0.0113, 0.8956),
    );
    let rec2020_to_srgb = mat3x3<f32>(
        vec3<f32>(1.6605, -0.1246, -0.0182),
        vec3<f32>(-0.5876, 1.1329, -0.1006),
        vec3<f32>(-0.0728, -0.0083, 1.1187),
    );
    let inset = mat3x3<f32>(
        vec3<f32>(0.856627153315983, 0.137318972929847, 0.11189821299995),
        vec3<f32>(0.0951212405381588, 0.761241990602591, 0.0767994186031903),
        vec3<f32>(0.0482516061458583, 0.101439036467562, 0.811302368396859),
    );
    let outset = mat3x3<f32>(
        vec3<f32>(1.1271005818144368, -0.1413297634984383, -0.14132976349843826),
        vec3<f32>(-0.11060664309660323, 1.157823702216272, -0.11060664309660294),
        vec3<f32>(-0.016493938717834573, -0.016493938717834257, 1.2519364065950405),
    );
    let ev_low = -12.47393;
    let ev_high = 4.026069;
    var v = inset * (srgb_to_2020 * c);
    v = (log2(max(v, vec3<f32>(1e-10))) - ev_low) / (ev_high - ev_low);
    v = agx_contrast(clamp(v, vec3<f32>(0.0), vec3<f32>(1.0)));
    v = pow(max(vec3<f32>(0.0), outset * v), vec3<f32>(2.2));
    return clamp(rec2020_to_srgb * v, vec3<f32>(0.0), vec3<f32>(1.0));
}

// three.js's Khronos PBR Neutral.
fn neutral(c: vec3<f32>) -> vec3<f32> {
    let compression = 0.8 - 0.04;
    let desaturation = 0.15;
    let x = min(c.r, min(c.g, c.b));
    let offset = select(0.04, x - 6.25 * x * x, x < 0.08);
    var v = c - offset;
    let peak = max(v.r, max(v.g, v.b));
    if (peak < compression) {
        return v;
    }
    let d = 1.0 - compression;
    let new_peak = 1.0 - d * d / (peak + d - compression);
    v = v * (new_peak / peak);
    let g = 1.0 - 1.0 / (desaturation * (peak - new_peak) + 1.0);
    return mix(v, vec3<f32>(new_peak), g);
}

// A normal from the values a texel stores, read as the source stores
// normals, and back as the new texels store them: a format that holds no
// negative values stores n / 2 + 1 / 2, and any other n itself.
fn normal_read(s: vec3<f32>) -> vec3<f32> {
    return select(s * 2.0 - 1.0, s, (p.flags & READ_SIGNED) != 0u);
}

fn normal_written(n: vec3<f32>) -> vec3<f32> {
    return select(n * 0.5 + 0.5, n, (p.flags & WRITE_SIGNED) != 0u);
}

// -ReconstructZ, -Normalize and -FlipGreen, in that order, on the stored
// values; a normal of no length stays as it is.
fn normal_steps(texel: vec4<f32>) -> vec4<f32> {
    let s = stored(texel);
    var n = normal_read(s.rgb);
    if ((p.flags & RECONSTRUCT_Z) != 0u) {
        n.z = sqrt(max(0.0, 1.0 - n.x * n.x - n.y * n.y));
    }
    if ((p.flags & NORMALIZE) != 0u) {
        let size = length(n);
        if (size > 0.0) {
            n = n / size;
        }
    }
    if ((p.flags & FLIP_GREEN) != 0u) {
        n.y = -n.y;
    }
    return working(vec4<f32>(normal_written(n), s.a));
}

// The changes before the tone map: the color space the texels are read
// in, the color key, the swizzle, the normal steps, unpremultiplied alpha,
// the alpha threshold and exposure.
fn before_tone_map(texel: vec4<f32>) -> vec4<f32> {
    var v = texel;
    if ((p.flags & ASSIGN_DECODE) != 0u) {
        v = vec4<f32>(decode(v.rgb), v.a);
    }
    if ((p.flags & ASSIGN_ENCODE) != 0u) {
        v = vec4<f32>(encode(v.rgb), v.a);
    }
    if ((p.flags & COLOR_KEY) != 0u && all(abs(stored(v).rgb - p.key.rgb) <= vec3<f32>(p.key.w))) {
        v = vec4<f32>(v.rgb, 0.0);
    }
    if ((p.flags & SWIZZLE) != 0u) {
        let s = stored(v);
        v = working(vec4<f32>(pick(s, p.swizzle.x), pick(s, p.swizzle.y), pick(s, p.swizzle.z), pick(s, p.swizzle.w)));
    }
    if ((p.flags & (RECONSTRUCT_Z | NORMALIZE | FLIP_GREEN)) != 0u) {
        v = normal_steps(v);
    }
    if ((p.flags & UNPREMULTIPLY) != 0u && v.a > 0.0) {
        v = vec4<f32>(v.rgb / v.a, v.a);
    }
    if ((p.flags & THRESHOLD) != 0u) {
        v = vec4<f32>(v.rgb, select(0.0, 1.0, v.a >= p.threshold));
    }
    return vec4<f32>(v.rgb * p.exposure, v.a);
}

// Every change, in order.
fn changed(texel: vec4<f32>) -> vec4<f32> {
    var v = before_tone_map(texel);
    if ((p.flags & TONE_MAP) != 0u) {
        switch p.curve {
            case 0u: {
                v = vec4<f32>(reinhard(v.rgb), v.a);
            }
            case 1u: {
                v = vec4<f32>(aces(v.rgb), v.a);
            }
            case 2u: {
                v = vec4<f32>(agx(v.rgb), v.a);
            }
            default: {
                v = vec4<f32>(neutral(v.rgb), v.a);
            }
        }
    }
    if ((p.flags & ADJUST) != 0u) {
        let s = stored(v);
        v = working(vec4<f32>(p.slope * s.rgb + p.intercept, s.a));
    }
    // ImageMagick's gamma leaves a negative value as it is.
    if ((p.flags & GAMMA) != 0u) {
        let s = stored(v);
        let powered = pow(max(s.rgb, vec3<f32>(0.0)), vec3<f32>(p.gamma));
        v = working(vec4<f32>(select(powered, s.rgb, s.rgb < vec3<f32>(0.0)), s.a));
    }
    if ((p.flags & GRAY_LINEAR) != 0u) {
        v = vec4<f32>(vec3<f32>(dot(v.rgb, p.weights.rgb)), v.a);
    }
    if ((p.flags & GRAY_STORED) != 0u) {
        let s = stored(v);
        v = working(vec4<f32>(vec3<f32>(dot(s.rgb, p.weights.rgb)), s.a));
    }
    if ((p.flags & INVERT) != 0u) {
        let s = stored(v);
        v = working(vec4<f32>(1.0 - s.rgb, s.a));
    }
    if ((p.flags & PREMULTIPLY) != 0u) {
        v = vec4<f32>(v.rgb * v.a, v.a);
    }
    if ((p.flags & OUT_ENCODE) != 0u) {
        v = vec4<f32>(encode(v.rgb), v.a);
    }
    if ((p.flags & OUT_DECODE) != 0u) {
        v = vec4<f32>(decode(v.rgb), v.a);
    }
    return v;
}
";

/// The compute pass that finds the brightest texel's luminance for
/// Reinhard's white point: texconv's weights on each texel after the
/// changes before the tone map, kept as the bits of a positive float,
/// which order as the floats do.
pub(crate) const BRIGHTEST: &str = "
@group(0) @binding(0) var source: texture_2d<f32>;
@group(0) @binding(2) var<storage, read_write> brightest: atomic<u32>;

@compute @workgroup_size(8, 8) fn brightest_texel(@builtin(global_invocation_id) id: vec3<u32>) {
    let size = textureDimensions(source);
    if (id.x >= size.x || id.y >= size.y) {
        return;
    }
    let v = before_tone_map(textureLoad(source, id.xy, 0));
    let luminance = dot(v.rgb, vec3<f32>(0.3, 0.59, 0.11));
    if (luminance > 0.0) {
        atomicMax(&brightest, bitcast<u32>(luminance));
    }
}
";

/// A linear value encoded as sRGB, its sign kept, as the shader encodes.
fn encode_srgb(c: f32) -> f32 {
    let a = c.abs();
    let e = if a <= 0.003_130_8 { a * 12.92 } else { 1.055 * a.powf(1.0 / 2.4) - 0.055 };
    e.copysign(c)
}

/// What one channel of a joined texture comes from: a channel of one of
/// the source textures, or a number.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Part {
    Channel { source: usize, channel: usize },
    Value(f32),
}

/// A texture a joined texture reads.
struct Source {
    texture: wgpu::Texture,
    core: CoreRef,
    label: String,
}

/// The source `value` names, by its place in `sources`, added when new.
fn source_of(value: &PsObject, sources: &mut Vec<Source>) -> PsResult<usize> {
    let (texture, core, label) = PsProxy::<Texture>::from_ps(value)?.with(|t| (t.texture.clone(), t.core.clone(), t.label.clone()))?;
    if let Some(at) = sources.iter().position(|s| s.texture == texture) {
        return Ok(at);
    }
    sources.push(Source { texture, core, label });
    Ok(sources.len() - 1)
}

/// The part `value` names for the channel `name` names: a texture, whose
/// red channel it reads; a texture and a channel letter as a pair; or a
/// number.
fn part_of(value: &PsObject, name: &str, sources: &mut Vec<Source>) -> PsResult<Part> {
    let shape = |what: &str| invalid("WgpuJoin", format!("{name} takes a texture, a texture and a channel letter, or a number, and {what} is none of those"));
    let type_name = value.type_name()?;
    if type_name == "Wgpu.Texture" {
        return Ok(Part::Channel { source: source_of(value, sources)?, channel: 0 });
    }
    if type_name.ends_with("[]") {
        let items = Vec::<PsObject>::from_ps(value)?;
        let [texture, letter] = items.as_slice() else {
            return Err(shape(&format!("a list of {} items", items.len())));
        };
        if texture.type_name()? != "Wgpu.Texture" {
            return Err(shape(&format!("a pair beginning with a {}", texture.type_name()?)));
        }
        let letter = String::from_ps(letter)?;
        let channel = match letter.to_ascii_lowercase().as_str() {
            "r" => 0,
            "g" => 1,
            "b" => 2,
            "a" => 3,
            _ => return Err(invalid("WgpuJoin", format!("{name}'s channel letter is r, g, b or a, not '{letter}'"))),
        };
        return Ok(Part::Channel { source: source_of(texture, sources)?, channel });
    }
    let number = f64::from_ps(value).map_err(|e| shape(&format!("a {type_name} ({})", e.message)))?;
    if !number.is_finite() {
        return Err(invalid("WgpuJoin", format!("{name} is a finite number, not {number}")));
    }
    Ok(Part::Value(number as f32))
}

/// The texture Join-WgpuTexture and [Wgpu.Texture]::Join make: each of its
/// channels from the part `red`, `green`, `blue` or `alpha` names, 0 for an
/// omitted color channel and 1 for an omitted alpha; every source resized
/// to the size of the largest first; in Rgba8Unorm, or the format `format`
/// names. An sRGB source gives the values it stores.
#[allow(clippy::too_many_arguments)]
pub(crate) fn join(
    red: Option<&PsObject>,
    green: Option<&PsObject>,
    blue: Option<&PsObject>,
    alpha: Option<&PsObject>,
    format: Option<&str>,
    stop: Option<&dyn Interrupt>,
    said: &mut Said,
) -> PsResult<Texture> {
    const NAMES: [&str; 4] = ["-Red", "-Green", "-Blue", "-Alpha"];
    const CHANNELS: [&str; 4] = ["red", "green", "blue", "alpha"];
    let mut sources = Vec::new();
    let mut parts = [Part::Value(0.0), Part::Value(0.0), Part::Value(0.0), Part::Value(1.0)];
    for (i, value) in [red, green, blue, alpha].into_iter().enumerate() {
        if let Some(value) = value.filter(|v| !v.is_null()) {
            parts[i] = part_of(value, NAMES[i], &mut sources)?;
        }
    }
    let Some(first) = sources.first() else {
        return Err(invalid("WgpuJoin", "Join-WgpuTexture takes at least one texture among -Red, -Green, -Blue and -Alpha"));
    };
    let core = first.core.clone();
    let mut size = (0, 0);
    for s in &sources {
        if s.core.id != core.id {
            return Err(invalid("WgpuOtherDevice", "Join-WgpuTexture joins textures of one device"));
        }
        if s.texture.dimension() != wgpu::TextureDimension::D2 || s.texture.depth_or_array_layers() != 1 {
            return Err(invalid("WgpuJoin", format!("Join-WgpuTexture joins 2D textures of one layer, and {} is not one", s.label)));
        }
        let own = (s.texture.width(), s.texture.height());
        if u64::from(own.0) * u64::from(own.1) > u64::from(size.0) * u64::from(size.1) {
            size = own;
        }
    }
    let mut colors = Vec::with_capacity(sources.len());
    for s in &sources {
        let file = image_io::read_file(&core, &s.texture, false, 1, stop)?;
        let srgb = file.format.is_srgb();
        let own = (file.width, file.height);
        let mut texels = transform::colors_at(&core, file, size, stop, said)?;
        if own != size {
            said.verbose.push(format!("{} was resized from {}x{} to {}x{} on the GPU", s.label, own.0, own.1, size.0, size.1));
        }
        if srgb {
            for t in &mut texels {
                for c in &mut t[..3] {
                    *c = encode_srgb(*c);
                }
            }
        }
        colors.push(texels);
    }
    let from: Vec<String> = parts
        .iter()
        .zip(CHANNELS)
        .map(|(part, channel)| match part {
            Part::Channel { source, channel: from } => format!("{channel} from the {} channel of {}", CHANNELS[*from], sources[*source].label),
            Part::Value(v) => format!("{channel} {v}"),
        })
        .collect();
    said.verbose.push(format!("the channels were joined: {}", from.join(", ")));
    let joined: Vec<[f32; 4]> = (0..size.0 as usize * size.1 as usize)
        .map(|i| {
            parts.map(|part| match part {
                Part::Channel { source, channel } => colors[source][i][channel],
                Part::Value(v) => v,
            })
        })
        .collect();
    image_io::joined(&core, &joined, size.0, size.1, format, stop, said)
}

/// Packs channels of several textures, or numbers, into one new texture:
/// -Red, -Green, -Blue and -Alpha each take a texture, whose red channel
/// they read; a texture and a channel letter as a pair, as ($mask, 'a');
/// or a number. An omitted color channel is 0, and an omitted alpha 1.
///
/// The textures are 2D, of one layer, on one device; one smaller than the
/// largest is resized to its size first, with Bicubic where it grows. An
/// sRGB texture gives the values it stores. The new texture is Rgba8Unorm,
/// or the -Format named: a color format, a BC format, encoded on the GPU,
/// or a Basis Universal format. What making it involved goes to the
/// verbose stream and its Notes.
///
/// # Examples
/// Join-WgpuTexture -Red $occlusion -Green $roughness -Blue $metal
/// Join-WgpuTexture $mask $mask $mask ($albedo, 'a')
/// Join-WgpuTexture -Red $height -Alpha 0.5 -Format Bc7RgbaUnorm
#[cmdlet(verb = "Join", noun = "WgpuTexture", alias = ["Join-GpuTexture"], output = ["Wgpu.Texture"])]
#[derive(Default)]
pub struct JoinWgpuTexture {
    /// The new texture's red: a texture, whose red channel it takes; a
    /// texture and a channel letter as a pair; or a number. 0 when omitted.
    #[param(position = 0)]
    pub red: Option<PsObject>,
    /// The new texture's green, as -Red takes it; 0 when omitted.
    #[param(position = 1)]
    pub green: Option<PsObject>,
    /// The new texture's blue, as -Red takes it; 0 when omitted.
    #[param(position = 2)]
    pub blue: Option<PsObject>,
    /// The new texture's alpha, as -Red takes it; 1 when omitted.
    #[param(position = 3)]
    pub alpha: Option<PsObject>,
    /// The new texture's format: a color format, a BC format, encoded on
    /// the GPU, or a Basis Universal format such as BasisUastc4x4;
    /// Rgba8Unorm when omitted.
    #[param]
    pub format: Option<String>,
}

impl Cmdlet for JoinWgpuTexture {
    fn process(&mut self, ps: &Pipeline<'_>) -> PsResult<()> {
        let mut said = Said::default();
        let made = join(self.red.as_ref(), self.green.as_ref(), self.blue.as_ref(), self.alpha.as_ref(), self.format.as_deref(), Some(ps), &mut said);
        tell(ps, &said)?;
        ps.write(made?)
    }
}

/// The color functions with the flag constants they read.
pub(crate) fn wgsl() -> String {
    let mut out = String::new();
    for (name, value) in FLAGS {
        out.push_str(&format!("const {name}: u32 = {value}u;\n"));
    }
    out.push_str(COLOR);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::convert::MipOptions;
    use crate::device::{self, AdapterQuery, CoreRef, InstanceOptions, OpenOptions};
    use crate::texfile::{Said, TextureFile};
    use crate::transform::{self, Packed, Transform};

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

    /// A 2D file of one level and layer.
    fn file(format: F, width: u32, height: u32, texels: Vec<u8>) -> TextureFile {
        TextureFile { dimension: wgpu::TextureDimension::D2, width, height, depth: 1, layers: 1, cube: false, format, levels: vec![texels] }
    }

    /// 64 RGBA texels, each channel a different mix of the texel's index,
    /// so every texel differs and every value from 0 to 255 is near one.
    fn bytes() -> Vec<u8> {
        (0..64u32).flat_map(|i| [(i * 4) as u8, (i * 37 % 256) as u8, (255 - i * 3) as u8, (64 + i * 2) as u8]).collect()
    }

    /// `color`'s changes made to `source` on the GPU, in its own format or
    /// the one a color-space change switches it to, with what was said.
    fn run(core: &CoreRef, source: TextureFile, color: Color) -> (TextureFile, Said) {
        let mut said = Said::default();
        let out = transform::apply(core, source, 1, MipOptions::default(), &Transform { color, ..Default::default() }, Packed::Own, None, None, None, &mut said).expect("recolored");
        (out, said)
    }

    /// An 8-bit value from a value from 0 to 1, as the packing rounds it.
    fn byte(v: f64) -> u8 {
        (v.clamp(0.0, 1.0) * 255.0).round() as u8
    }

    /// sRGB's transfer functions, from IEC 61966-2-1.
    fn encode(v: f64) -> f64 {
        if v <= 0.0031308 { v * 12.92 } else { 1.055 * v.powf(1.0 / 2.4) - 0.055 }
    }

    fn decode(v: f64) -> f64 {
        if v <= 0.04045 { v / 12.92 } else { ((v + 0.055) / 1.055).powf(2.4) }
    }

    /// The largest difference of any byte between two texel lists.
    fn worst(a: &[u8], b: &[u8]) -> u8 {
        assert_eq!(a.len(), b.len());
        a.iter().zip(b).map(|(x, y)| x.abs_diff(*y)).max().expect("texels")
    }

    /// The RGBA texels of 8-bit `bytes`.
    fn rgba(bytes: &[u8]) -> std::slice::Iter<'_, [u8; 4]> {
        bytes.as_chunks::<4>().0.iter()
    }

    /// What a change does to a stored value from 0 to 1.
    type Formula = fn(f64) -> f64;

    /// ImageMagick's -brightness-contrast, -gamma and -negate, as its
    /// enhance.c computes them, on each color channel's stored value, with
    /// alpha kept: what the module computes on Rgba8Unorm texels, and on
    /// Rgba8UnormSrgb texels around their decoding and encoding.
    #[test]
    fn adjusts_stored_values_as_imagemagick_does() {
        let Some(core) = core() else { return };
        let source = bytes();
        let oracle = |f: Formula| -> Vec<u8> { rgba(&source).flat_map(|t| [byte(f(f64::from(t[0]) / 255.0)), byte(f(f64::from(t[1]) / 255.0)), byte(f(f64::from(t[2]) / 255.0)), t[3]]).collect() };
        let cases: [(Color, Formula); 6] = [
            (Color { brightness: Some(20.0), ..Default::default() }, |v| v + 0.2),
            (Color { contrast: Some(40.0), ..Default::default() }, |v| (v - 0.5) * (100.0 / 60.0) + 0.5),
            (Color { brightness: Some(-25.0), contrast: Some(-40.0), ..Default::default() }, |v| (v - 0.25 - 0.5) * 0.6 + 0.5),
            (Color { gamma: Some(2.2), ..Default::default() }, |v| v.powf(1.0 / 2.2)),
            (Color { gamma: Some(0.5), ..Default::default() }, |v| v.powf(2.0)),
            (Color { invert: true, ..Default::default() }, |v| 1.0 - v),
        ];
        for (color, f) in cases {
            for format in [F::Rgba8Unorm, F::Rgba8UnormSrgb] {
                let (out, _) = run(&core, file(format, 8, 8, source.clone()), color);
                assert_eq!(out.format, format);
                let w = worst(&out.levels[0], &oracle(f));
                assert!(w <= 1, "{color:?} on {format:?} is {w} steps from ImageMagick's values");
            }
        }
        // The line ImageMagick's BrightnessContrastImage applies.
        assert_eq!(brightness_contrast(20.0, 40.0), (100.0 / 60.0, (0.2 - 0.5) * (100.0 / 60.0) + 0.5));
        assert_eq!(brightness_contrast(0.0, -40.0), (0.6, 0.5 - 0.5 * 0.6));
        assert_eq!(brightness_contrast(0.0, 100.0).0, 100.0 / MAGICK_EPSILON);
    }

    /// The largest of 0.3 r + 0.59 g + 0.11 b over `texels`, texconv's
    /// brightest luminance, from 0.
    fn brightest(texels: &[[f64; 4]]) -> f64 {
        texels.iter().map(|t| 0.3 * t[0] + 0.59 * t[1] + 0.11 * t[2]).fold(0.0, f64::max)
    }

    /// texconv's Reinhard with `brightest` as L: each color channel c
    /// becomes c (1 + c / L^2) / (1 + c), and alpha is kept.
    fn reinhard(texels: &[[f64; 4]], brightest: f64) -> Vec<[f64; 4]> {
        let white = brightest * brightest;
        texels.iter().map(|t| [t[0] * (1.0 + t[0] / white) / (1.0 + t[0]), t[1] * (1.0 + t[1] / white) / (1.0 + t[1]), t[2] * (1.0 + t[2] / white) / (1.0 + t[2]), t[3]]).collect()
    }

    /// `m` times `v`, `m` given as its columns, as GLSL's mat3 holds them.
    fn times(m: [[f64; 3]; 3], v: [f64; 3]) -> [f64; 3] {
        [0, 1, 2].map(|r| m[0][r] * v[0] + m[1][r] * v[1] + m[2][r] * v[2])
    }

    /// three.js's ACESFilmicToneMapping, at an exposure of 1.
    fn aces(c: [f64; 3]) -> [f64; 3] {
        let to_ap1 = [[0.59719, 0.07600, 0.02840], [0.35458, 0.90834, 0.13383], [0.04823, 0.01566, 0.83777]];
        let from_ap1 = [[1.60475, -0.10208, -0.00327], [-0.53108, 1.10813, -0.07276], [-0.07367, -0.00605, 1.07602]];
        let v = times(to_ap1, c.map(|x| x / 0.6));
        let fit = v.map(|x| (x * (x + 0.0245786) - 0.000090537) / (x * (0.983729 * x + 0.4329510) + 0.238081));
        times(from_ap1, fit).map(|x| x.clamp(0.0, 1.0))
    }

    /// three.js's AgXToneMapping, at an exposure of 1.
    fn agx(c: [f64; 3]) -> [f64; 3] {
        let srgb_to_2020 = [[0.6274, 0.0691, 0.0164], [0.3293, 0.9195, 0.0880], [0.0433, 0.0113, 0.8956]];
        let rec2020_to_srgb = [[1.6605, -0.1246, -0.0182], [-0.5876, 1.1329, -0.1006], [-0.0728, -0.0083, 1.1187]];
        let inset = [
            [0.856627153315983, 0.137318972929847, 0.11189821299995],
            [0.0951212405381588, 0.761241990602591, 0.0767994186031903],
            [0.0482516061458583, 0.101439036467562, 0.811302368396859],
        ];
        let outset = [
            [1.1271005818144368, -0.1413297634984383, -0.14132976349843826],
            [-0.11060664309660323, 1.157823702216272, -0.11060664309660294],
            [-0.016493938717834573, -0.016493938717834257, 1.2519364065950405],
        ];
        let (low, high) = (-12.47393, 4.026069);
        let v = times(inset, times(srgb_to_2020, c)).map(|x| ((x.max(1e-10).log2() - low) / (high - low)).clamp(0.0, 1.0));
        let v = v.map(|x| {
            let (x2, x4) = (x * x, x * x * x * x);
            15.5 * x4 * x2 - 40.14 * x4 * x + 31.96 * x4 - 6.868 * x2 * x + 0.4298 * x2 + 0.1191 * x - 0.00232
        });
        let v = times(outset, v).map(|x| x.max(0.0).powf(2.2));
        times(rec2020_to_srgb, v).map(|x| x.clamp(0.0, 1.0))
    }

    /// three.js's NeutralToneMapping, at an exposure of 1.
    fn neutral(c: [f64; 3]) -> [f64; 3] {
        let (start, desaturation) = (0.8 - 0.04, 0.15);
        let x = c[0].min(c[1]).min(c[2]);
        let offset = if x < 0.08 { x - 6.25 * x * x } else { 0.04 };
        let v = c.map(|y| y - offset);
        let peak = v[0].max(v[1]).max(v[2]);
        if peak < start {
            return v;
        }
        let d = 1.0 - start;
        let new_peak = 1.0 - d * d / (peak + d - start);
        let v = v.map(|y| y * new_peak / peak);
        let g = 1.0 - 1.0 / (desaturation * (peak - new_peak) + 1.0);
        v.map(|y| y * (1.0 - g) + new_peak * g)
    }

    /// Each curve maps 32-bit float texels as texconv's Reinhard and
    /// three.js's ACES Filmic, AgX and Neutral do, with exposure applied
    /// before it, and Reinhard's white point found after the exposure.
    #[test]
    fn tone_maps_as_texconv_and_three_js_do() {
        let Some(core) = core() else { return };
        let texels: Vec<[f64; 4]> = (0..64).map(|i| [f64::from(i) * 0.11, f64::from(63 - i) * 0.05 + 0.002, (f64::from(i) * 0.37).sin().abs() * 3.0, 0.5]).collect();
        let bytes: Vec<u8> = texels.iter().flat_map(|t| t.map(|v| v as f32)).flat_map(f32::to_le_bytes).collect();
        let values = |out: &TextureFile| -> Vec<f32> { out.levels[0].as_chunks::<4>().0.iter().map(|c| f32::from_le_bytes(*c)).collect() };
        let exposed: Vec<[f64; 4]> = texels.iter().map(|t| [t[0] * 2.0, t[1] * 2.0, t[2] * 2.0, t[3]]).collect();
        let curves: [(ToneMapCurve, Vec<[f64; 4]>); 4] = [
            (ToneMapCurve::Reinhard, reinhard(&exposed, brightest(&exposed))),
            (ToneMapCurve::AcesFilmic, exposed.iter().map(|t| { let c = aces([t[0], t[1], t[2]]); [c[0], c[1], c[2], t[3]] }).collect()),
            (ToneMapCurve::AgX, exposed.iter().map(|t| { let c = agx([t[0], t[1], t[2]]); [c[0], c[1], c[2], t[3]] }).collect()),
            (ToneMapCurve::Neutral, exposed.iter().map(|t| { let c = neutral([t[0], t[1], t[2]]); [c[0], c[1], c[2], t[3]] }).collect()),
        ];
        for (curve, expected) in &curves {
            let (out, said) = run(&core, file(F::Rgba32Float, 8, 8, bytes.clone()), Color { exposure: Some(1.0), tone_map: Some(*curve), ..Default::default() });
            let got = values(&out);
            let far = got.iter().zip(expected.iter().flatten()).map(|(g, e)| (f64::from(*g) - e).abs()).fold(0.0, f64::max);
            assert!(far <= 1e-3, "{curve:?} is {far} from the reference");
            if *curve == ToneMapCurve::Reinhard {
                assert!(said.verbose.iter().any(|l| l.starts_with("Reinhard's white point was the brightest texel's luminance, ")), "{said:?}");
                // A white point found before the exposure gives values this far off.
                let before = reinhard(&exposed, brightest(&texels));
                let off = got.iter().zip(before.iter().flatten()).map(|(g, e)| (f64::from(*g) - e).abs()).fold(0.0, f64::max);
                assert!(off > 1e-2, "a white point found before the exposure is only {off} from the result");
            }
        }
    }

    /// A swizzle moves stored values exactly, an sRGB texture's alpha into
    /// its color included; premultiplying works in linear light for sRGB.
    #[test]
    fn swizzles_stored_values_and_premultiplies_in_linear_light() {
        let Some(core) = core() else { return };
        let source = bytes();
        let (out, _) = run(&core, file(F::Rgba8Unorm, 8, 8, source.clone()), Color { swizzle: Some(swizzle_of("bgra").expect("a mask")), ..Default::default() });
        assert_eq!(out.levels[0], rgba(&source).flat_map(|t| [t[2], t[1], t[0], t[3]]).collect::<Vec<_>>());
        let (out, _) = run(&core, file(F::Rgba8UnormSrgb, 8, 8, source.clone()), Color { swizzle: Some(swizzle_of("AAA1").expect("a mask")), ..Default::default() });
        assert_eq!(out.levels[0], rgba(&source).flat_map(|t| [t[3], t[3], t[3], 255]).collect::<Vec<_>>());

        let (out, _) = run(&core, file(F::Rgba8Unorm, 8, 8, source.clone()), Color { premultiply: true, ..Default::default() });
        let expected: Vec<u8> = rgba(&source).flat_map(|t| { let a = f64::from(t[3]) / 255.0; [0, 1, 2].map(|i| byte(f64::from(t[i]) / 255.0 * a)).into_iter().chain([t[3]]) }).collect();
        assert!(worst(&out.levels[0], &expected) <= 1);
        let (out, _) = run(&core, file(F::Rgba8UnormSrgb, 8, 8, source.clone()), Color { premultiply: true, ..Default::default() });
        let expected: Vec<u8> = rgba(&source).flat_map(|t| { let a = f64::from(t[3]) / 255.0; [0, 1, 2].map(|i| byte(encode(decode(f64::from(t[i]) / 255.0) * a))).into_iter().chain([t[3]]) }).collect();
        assert!(worst(&out.levels[0], &expected) <= 1);
        let (out, _) = run(&core, file(F::Rgba8Unorm, 8, 8, expected_premultiplied(&source)), Color { unpremultiply: true, ..Default::default() });
        let back: Vec<u8> = rgba(&expected_premultiplied(&source)).flat_map(|t| { let a = f64::from(t[3]) / 255.0; [0, 1, 2].map(|i| byte(f64::from(t[i]) / 255.0 / a)).into_iter().chain([t[3]]) }).collect();
        assert!(worst(&out.levels[0], &back) <= 1);
    }

    /// A color key clears the alpha of the texels whose stored colors match
    /// it, within its tolerance and half a step of the format, and leaves
    /// the rest as they were; an alpha threshold makes alpha 0 or 1; both
    /// come before premultiplying, so a cut texel premultiplies to nothing
    /// and a kept one by full alpha.
    #[test]
    fn keys_a_color_and_thresholds_alpha() {
        let Some(core) = core() else { return };
        let texels: [[u8; 4]; 8] = [[255, 0, 255, 200], [250, 5, 250, 200], [240, 0, 255, 200], [10, 20, 30, 127], [10, 20, 30, 128], [200, 100, 50, 100], [0, 0, 0, 0], [255, 0, 255, 255]];
        let source: Vec<u8> = texels.iter().flatten().copied().collect();
        let magenta = ColorKey { rgb: hex_of("#ff00FF").expect("a key"), tolerance: 0.0 };
        assert_eq!(magenta.rgb, [1.0, 0.0, 1.0]);
        let with_alpha = |alphas: [u8; 8]| -> Vec<u8> { texels.iter().zip(alphas).flat_map(|(t, a)| [t[0], t[1], t[2], a]).collect() };

        let (out, said) = run(&core, file(F::Rgba8Unorm, 4, 2, source.clone()), Color { key: Some(magenta), ..Default::default() });
        assert_eq!(out.levels[0], with_alpha([0, 200, 200, 127, 128, 100, 0, 0]));
        assert!(said.verbose.iter().any(|l| l.contains("made transparent where red, green and blue were 1, 0 and 1")), "{said:?}");
        let (out, _) = run(&core, file(F::Rgba8Unorm, 4, 2, source.clone()), Color { key: Some(ColorKey { tolerance: 0.05, ..magenta }), ..Default::default() });
        assert_eq!(out.levels[0], with_alpha([0, 0, 200, 127, 128, 100, 0, 0]));
        let (out, _) = run(&core, file(F::Rgba8UnormSrgb, 4, 2, source.clone()), Color { key: Some(magenta), ..Default::default() });
        assert!(worst(&out.levels[0], &with_alpha([0, 200, 200, 127, 128, 100, 0, 0])) <= 1);

        let (out, _) = run(&core, file(F::Rgba8Unorm, 4, 2, source.clone()), Color { threshold: Some(0.5), ..Default::default() });
        assert_eq!(out.levels[0], with_alpha([255, 255, 255, 0, 255, 0, 0, 255]));
        let (out, _) = run(&core, file(F::Rgba8Unorm, 4, 2, source.clone()), Color { key: Some(magenta), premultiply: true, ..Default::default() });
        assert_eq!(out.levels[0][..4], [0, 0, 0, 0]);
        assert_eq!(out.levels[0][28..], [0, 0, 0, 0]);
        let (out, _) = run(&core, file(F::Rgba8Unorm, 4, 2, source.clone()), Color { threshold: Some(0.3), premultiply: true, ..Default::default() });
        assert_eq!(out.levels[0][12..16], [10, 20, 30, 255]);
        assert_eq!(out.levels[0][20..28], [200, 100, 50, 255, 0, 0, 0, 0]);

        assert_eq!(refused(threshold_of(Some(0.0))).error_id, "WgpuAlphaThreshold");
        assert_eq!(refused(threshold_of(Some(1.5))).error_id, "WgpuAlphaThreshold");
        assert_eq!(refused(key_of(None, Some(0.1))).error_id, "WgpuColorKey");
        assert_eq!(refused(hex_of("FF00FF")).error_id, "WgpuColorKey");
        assert_eq!(refused(hex_of("#FF00F")).error_id, "WgpuColorKey");
        assert_eq!(refused(Color { threshold: Some(0.5), ..Default::default() }.prepare(F::R8Unorm, F::R8Unorm, false, false)).error_id, "WgpuAlpha");
    }

    /// `texels` premultiplied on the CPU, for the unpremultiply test.
    fn expected_premultiplied(texels: &[u8]) -> Vec<u8> {
        rgba(texels).flat_map(|t| { let a = f64::from(t[3]) / 255.0; [0, 1, 2].map(|i| byte(f64::from(t[i]) / 255.0 * a)).into_iter().chain([t[3]]) }).collect()
    }

    /// -ColorSpace converts so colors look the same and switches a format
    /// to its other form, says when there was nothing to do, and encodes a
    /// format with no sRGB form in place; -AssignColorSpace relabels and is
    /// refused where there is no other form.
    #[test]
    fn converts_and_relabels_color_spaces() {
        let Some(core) = core() else { return };
        let source = bytes();
        let (out, said) = run(&core, file(F::Rgba8Unorm, 8, 8, source.clone()), Color { space: Some(ColorSpace::Srgb), ..Default::default() });
        assert_eq!(out.format, F::Rgba8UnormSrgb);
        let expected: Vec<u8> = rgba(&source).flat_map(|t| [0, 1, 2].map(|i| byte(encode(f64::from(t[i]) / 255.0))).into_iter().chain([t[3]])).collect();
        assert!(worst(&out.levels[0], &expected) <= 1);
        assert!(said.verbose.iter().any(|l| l == "the texels were converted to sRGB, which makes the texture Rgba8UnormSrgb"), "{said:?}");

        let (out, said) = run(&core, file(F::Rgba8UnormSrgb, 8, 8, source.clone()), Color { space: Some(ColorSpace::Srgb), ..Default::default() });
        assert_eq!(out.format, F::Rgba8UnormSrgb);
        assert!(worst(&out.levels[0], &source) <= 1);
        assert!(said.verbose.iter().any(|l| l == "the texture was already sRGB, so -ColorSpace changed nothing"), "{said:?}");

        let (out, _) = run(&core, file(F::Rgba8Unorm, 8, 8, source.clone()), Color { assign: Some(ColorSpace::Srgb), ..Default::default() });
        assert_eq!(out.format, F::Rgba8UnormSrgb);
        assert!(worst(&out.levels[0], &source) <= 1);

        let floats: Vec<u8> = source.iter().map(|&b| f32::from(b) / 255.0).flat_map(f32::to_le_bytes).collect();
        let (out, said) = run(&core, file(F::Rgba32Float, 8, 8, floats), Color { space: Some(ColorSpace::Srgb), ..Default::default() });
        assert_eq!(out.format, F::Rgba32Float);
        let got: Vec<f32> = out.levels[0].as_chunks::<4>().0.iter().map(|c| f32::from_le_bytes(*c)).collect();
        for (i, (g, b)) in got.iter().zip(&source).enumerate() {
            let want = if i % 4 == 3 { f64::from(*b) / 255.0 } else { encode(f64::from(*b) / 255.0) };
            assert!((f64::from(*g) - want).abs() < 1e-5, "texel value {i} is {g}, not {want}");
        }
        assert!(said.verbose.iter().any(|l| l.contains("a format that records no color space")), "{said:?}");

        let e = refused(Color { assign: Some(ColorSpace::Srgb), ..Default::default() }.prepare(F::Rgba16Float, F::Rgba16Float, false, false));
        assert_eq!(e.error_id, "WgpuColorSpace");
    }

    /// Rec. 709 weighs linear values, Rec. 601 the stored values as
    /// Pillow's convert('L') does, and custom weights linear values.
    #[test]
    fn makes_grayscale_with_each_weighting() {
        let Some(core) = core() else { return };
        let source = bytes();
        let (out, _) = run(&core, file(F::Rgba8UnormSrgb, 8, 8, source.clone()), Color { grayscale: Some(Gray::Rec709), ..Default::default() });
        let expected: Vec<u8> = rgba(&source)
            .flat_map(|t| {
                let l =[0.2126, 0.7152, 0.0722].iter().zip(t).map(|(w, v)| w * decode(f64::from(*v) / 255.0)).sum::<f64>();
                let g = byte(encode(l));
                [g, g, g, t[3]]
            })
            .collect();
        assert!(worst(&out.levels[0], &expected) <= 1);

        // Pillow's ImagingConvert to L: (R * 19595 + G * 38470 + B * 7471 + 0x8000) >> 16.
        let (out, _) = run(&core, file(F::Rgba8Unorm, 8, 8, source.clone()), Color { grayscale: Some(Gray::Rec601), ..Default::default() });
        let pillow: Vec<u8> = rgba(&source)
            .flat_map(|t| {
                let g = ((u32::from(t[0]) * 19595 + u32::from(t[1]) * 38470 + u32::from(t[2]) * 7471 + 0x8000) >> 16) as u8;
                [g, g, g, t[3]]
            })
            .collect();
        assert!(worst(&out.levels[0], &pillow) <= 1);

        let (out, _) = run(&core, file(F::Rgba8UnormSrgb, 8, 8, source.clone()), Color { grayscale: Some(Gray::Weights([1.0, 0.0, 0.0])), ..Default::default() });
        assert!(worst(&out.levels[0], &rgba(&source).flat_map(|t| [t[0], t[0], t[0], t[3]]).collect::<Vec<_>>()) <= 1);
    }

    /// The parameters' refusals, by their ids.
    #[test]
    fn refuses_what_the_parameters_cannot_mean() {
        for mask in ["rgb", "rgbx", "rgbaa", ""] {
            assert_eq!(refused(swizzle_of(mask)).error_id, "WgpuSwizzle", "{mask}");
        }
        assert_eq!(swizzle_of("Rrr1").expect("a mask"), [0, 0, 0, 5]);
        assert_eq!(refused(percent_of(Some(100.5), "-Brightness")).error_id, "WgpuAdjust");
        assert_eq!(refused(percent_of(Some(f64::NAN), "-Contrast")).error_id, "WgpuAdjust");
        assert_eq!(refused(gamma_of(Some(0.0))).error_id, "WgpuAdjust");
        assert_eq!(refused(exposure_of(Some(f64::INFINITY))).error_id, "WgpuAdjust");
        assert_eq!(gray_of(true, None).expect("weights"), Some(Gray::Rec709));
        assert_eq!(gray_of(false, None).expect("none"), None);
    }
}
