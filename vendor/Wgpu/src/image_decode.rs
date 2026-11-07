//! An image file decoded into the pixels it means, as Import-WgpuImage
//! loads it: every frame of an animation and every page of a TIFF, or the
//! ones the import names, each turned as the file's orientation says, and
//! converted to sRGB, the encoding the module takes 8-bit color in, from
//! the color profile the file carries: an embedded ICC profile, RGB, gray
//! or CMYK, or an AVIF's or JPEG XL's CICP primaries and transfer. A
//! conversion is perceptual unless the import's Reading names another
//! intent, and colors sRGB lacks are clipped. A float image converts to
//! linear light in sRGB's primaries instead, its light below 0 clipped and
//! above 1 kept, as an HDR AVIF's light loads. A Reading can also load the
//! pixels or the colors as stored. An SVG file and a PDF's pages are drawn
//! in sRGB instead, at the size and turn the import's transform plans for
//! the size the file gives. The library's [`decode_image`] decodes a file's
//! bytes the same way, each frame or page at its own size and with its
//! pixels as the file stores them too, and decodes a DDS or KTX2 file's
//! texels.
//!
//! The image crate decodes every format but AVIF, JPEG XL, Photoshop's and
//! WebP, which avif-decode, jxl-oxide, the module's own reader and libwebp
//! decode, AVIF, JPEG XL and WebP in builds with the avif, jxl and webp
//! features, the image crate decoding WebP in a build without; it reports
//! the orientation and profile of JPEG, PNG, WebP and GIF files. Each TIFF
//! page's come from its own tags through the tiff crate, and an AVIF's
//! from its own irot, imir and colr boxes. A CMYK JPEG or TIFF with a
//! profile converts from its samples as stored, which zune-jpeg and the
//! tiff crate give; one with none keeps the image crate's formula. A file
//! whose orientation or profile cannot be read or used loads as stored,
//! with a warning saying why. resvg draws SVG files and hayro PDF pages,
//! in builds with the svg and pdf features.

use std::fmt::Display;
use std::io::Cursor;
use std::path::Path;

use image::codecs::gif::GifDecoder;
use image::codecs::png::PngDecoder;
use image::codecs::webp::WebPDecoder;
use image::metadata::Orientation;
use image::{AnimationDecoder, DynamicImage, GenericImageView, ImageBuffer, ImageDecoder, ImageFormat, ImageReader};
use moxcms::{adapt_to_illuminant_d, ColorPrimaries, ColorProfile, DataColorSpace, Layout, Matrix3d, ProfileText, ToneReprCurve, TransformOptions, XyY, Xyz, Xyzd};
use pwrs::prelude::*;

use crate::error::invalid;
use crate::profile_table::{self, TableConversion};
use crate::psd::{Header, Mode};
use crate::psd_fill::{Colors, Foreign, Light, Mapping};
use crate::texfile::Said;
use crate::transform::Transform;
use decoded::{as_stored, Kept};
pub use decoded::{decode_image, DecodeOptions, DecodedFrame, DecodedImage, PixelFormat, Pixels};

#[cfg(feature = "avif")]
mod avif;
#[cfg(any(feature = "avif", feature = "jxl"))]
mod cicp;
mod decoded;
#[cfg(feature = "jxl")]
mod jxl;
#[cfg(feature = "pdf")]
mod pdf;
#[cfg(feature = "svg")]
mod svg;

/// How a conversion from an image's color profile to sRGB maps colors,
/// above all those sRGB lacks.
#[psenum(name = "Wgpu.RenderingIntent")]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum RenderingIntent {
    /// The profile's mapping for photographs, which fits colors sRGB lacks
    /// in among the rest.
    #[default]
    Perceptual,
    /// Colors sRGB holds keep their values, with the profile's white made
    /// sRGB's white, and the others go to the nearest sRGB holds.
    RelativeColorimetric,
    /// The profile's mapping for graphics, which keeps colors vivid.
    Saturation,
    /// Colors sRGB holds keep their values, the profile's white included.
    AbsoluteColorimetric,
}

impl RenderingIntent {
    fn moxcms(self) -> moxcms::RenderingIntent {
        match self {
            RenderingIntent::Perceptual => moxcms::RenderingIntent::Perceptual,
            RenderingIntent::RelativeColorimetric => moxcms::RenderingIntent::RelativeColorimetric,
            RenderingIntent::Saturation => moxcms::RenderingIntent::Saturation,
            RenderingIntent::AbsoluteColorimetric => moxcms::RenderingIntent::AbsoluteColorimetric,
        }
    }

    fn words(self) -> &'static str {
        match self {
            RenderingIntent::Perceptual => "perceptually",
            RenderingIntent::RelativeColorimetric => "with the relative colorimetric intent",
            RenderingIntent::Saturation => "with the saturation intent",
            RenderingIntent::AbsoluteColorimetric => "with the absolute colorimetric intent",
        }
    }
}

/// The names a caller knows a decode's options by, which the messages
/// about them use: Import-WgpuImage's parameters, or the fields of the
/// library's [`DecodeOptions`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Names {
    ignore_orientation: &'static str,
    ignore_profile: &'static str,
    intent: &'static str,
    frames: &'static str,
    dpi: &'static str,
    no_background: &'static str,
    password: &'static str,
    /// What the refusal of a protected PDF given no password says to do.
    give_password: &'static str,
}

impl Names {
    const PARAMETERS: Names = Names {
        ignore_orientation: "-IgnoreOrientation",
        ignore_profile: "-IgnoreProfile",
        intent: "-RenderingIntent",
        frames: "-Frame",
        dpi: "-Dpi",
        no_background: "-NoBackground",
        password: "-Password",
        give_password: "give it as a SecureString with -Password",
    };

    const FIELDS: Names = Names {
        ignore_orientation: "ignore_orientation",
        ignore_profile: "ignore_profile",
        intent: "rendering_intent",
        frames: "frames",
        dpi: "dpi",
        no_background: "no_background",
        password: "password",
        give_password: "give the options a password",
    };
}

impl Default for Names {
    /// Import-WgpuImage's parameters.
    fn default() -> Names {
        Names::PARAMETERS
    }
}

/// What an import does with a file's orientation and color profile: turn
/// the pixels and convert the colors, as the default does, or load either
/// as stored, and the intent a conversion uses; how it draws a PDF's page;
/// the names its messages give these options; and whether it keeps each
/// frame's pixels as the file stores them beside those it loads.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct Reading {
    pub(crate) ignore_orientation: bool,
    pub(crate) ignore_profile: bool,
    pub(crate) intent: Option<RenderingIntent>,
    /// The dots per inch a PDF's page is drawn at: 72, a pixel a point,
    /// when `None`.
    pub(crate) dpi: Option<f64>,
    /// Leaves a PDF's page transparent where nothing is drawn, not white.
    pub(crate) no_background: bool,
    /// The names the messages give these options and the frames asked for.
    pub(crate) names: Names,
    /// Keeps each frame's pixels as the file stores them, turned as the
    /// pixels loaded are, a CMYK image's inks among them.
    pub(crate) stored: bool,
}

impl Reading {
    /// The intent a conversion uses: -RenderingIntent's, and Perceptual
    /// when it is omitted.
    fn intent(&self) -> RenderingIntent {
        match self.intent {
            Some(intent) => intent,
            None => RenderingIntent::Perceptual,
        }
    }

    /// The warnings these options and -Password, `password`, take for a
    /// file that records no orientation and no color profile and is not a
    /// PDF, such as a DDS or KTX2 file: `what` names it.
    pub(crate) fn unused(&self, what: &str, password: Option<&str>, said: &mut Said) {
        let names = self.names;
        if self.ignore_orientation {
            said.warnings.push(format!("{what} records no orientation, so {} changed nothing", names.ignore_orientation));
        }
        if self.ignore_profile {
            said.warnings.push(format!("{what} carries no color profile, so {} changed nothing", names.ignore_profile));
        }
        if self.intent.is_some() {
            said.warnings.push(format!("{what} carries no color profile, so {} changed nothing", names.intent));
        }
        self.pdf_unused(what, password, said);
    }

    /// The warnings the options that draw a PDF's page take for a file
    /// that is not a PDF: `what` names it, and `password` is -Password's.
    pub(crate) fn pdf_unused(&self, what: &str, password: Option<&str>, said: &mut Said) {
        let names = self.names;
        for (asked, name) in [(self.dpi.is_some(), names.dpi), (self.no_background, names.no_background), (password.is_some(), names.password)] {
            if asked {
                said.warnings.push(format!("{what} is not a PDF, so {name} changed nothing"));
            }
        }
    }
}

/// The color an image file says its pixels are in.
enum Profile {
    /// An ICC profile, as the file holds it.
    Icc(Vec<u8>),
    /// An AVIF's or a JPEG XL's CICP primaries, transfer and matrix.
    #[cfg(any(feature = "avif", feature = "jxl"))]
    Cicp(moxcms::CicpProfile),
    /// CIE Lab under D50, a Photoshop Lab document's color.
    Lab,
}

/// A CMYK image's samples as stored, four to a pixel, the ink of each from
/// 0, none, to the largest value, full.
#[derive(Clone)]
enum Cmyk {
    Eight(Vec<u8>),
    Sixteen(Vec<u16>),
}

/// An image file's pixels as decoded, and what the file says about them:
/// its orientation and profile, each as read or the reason it could not be.
struct Decoded {
    image: DynamicImage,
    orientation: Result<Orientation, String>,
    profile: Result<Option<Profile>, String>,
    /// A CMYK image's samples, which the profile converts in place of
    /// `image`, the image crate's RGB from them.
    cmyk: Option<Cmyk>,
    /// Whether `image`'s float RGB holds a gray image's samples, each
    /// pixel's three the same, which a gray profile converts from the first.
    gray_floats: bool,
    /// Whether `image`'s RGB holds a CIE Lab document's L, a and b as the
    /// document stores them.
    lab: bool,
    /// What reading the file left out of it, for the warnings, and what it
    /// did, for the verbose stream.
    warnings: Vec<String>,
    said: Vec<String>,
    /// The file's frames or pages after `image`, its first, each with its
    /// CMYK samples where it has them.
    rest: Vec<(DynamicImage, Option<Cmyk>)>,
    /// The timing of a file whose frames animate.
    timing: Option<Timing>,
}

impl Decoded {
    /// A file of `frames`, RGBA as an animation decoder composites them,
    /// with `timing` where there is more than one, and the orientation and
    /// profile the file records.
    fn animation(mut frames: Vec<DynamicImage>, timing: Timing, orientation: Result<Orientation, String>, profile: Result<Option<Profile>, String>) -> Result<Decoded, String> {
        if frames.is_empty() {
            return Err("it holds no frames".to_string());
        }
        let image = frames.remove(0);
        let timing = if frames.is_empty() { None } else { Some(timing) };
        let rest = frames.into_iter().map(|frame| (frame, None)).collect();
        Ok(Decoded { image, orientation, profile, cmyk: None, gray_floats: false, lab: false, warnings: Vec::new(), said: Vec::new(), rest, timing })
    }

    /// The frames `asked` names, in its order, every one when it is `None`,
    /// as one part that shares the file's orientation and profile, with the
    /// timing of those frames; or why `asked`, the option `name` names,
    /// names none or one past the last. A frame named more than once is
    /// copied for each use but its last, which takes it.
    fn chosen(self, asked: Option<&[u32]>, name: &str) -> Result<Chosen, String> {
        let Decoded { image, orientation, profile, cmyk, gray_floats, lab, warnings, said, rest, timing } = self;
        let mut all: Vec<Option<(DynamicImage, Option<Cmyk>)>> = std::iter::once((image, cmyk)).chain(rest).map(Some).collect();
        let picked = chosen(asked, all.len(), name)?;
        let timing = match timing {
            Some(timing) => {
                let delays = picked.iter().map(|&at| timing.delays.get(at).copied().ok_or_else(|| format!("its timing gives no delay for frame {at}"))).collect::<Result<_, _>>()?;
                Some(Timing { delays, loops: timing.loops })
            }
            None => None,
        };
        let mut uses = vec![0usize; all.len()];
        for &at in &picked {
            uses[at] += 1;
        }
        let mut frames = Vec::with_capacity(picked.len());
        for (place, &index) in picked.iter().enumerate() {
            uses[index] -= 1;
            let frame = if uses[index] == 0 { all[index].take() } else { all[index].clone() };
            let Some((image, cmyk)) = frame else {
                return Err(format!("frame {index} was taken before its last use"));
            };
            frames.push(Frame { image, cmyk, place, index });
        }
        let count = frames.len();
        Ok(Chosen { parts: vec![Part { frames, orientation, profile, gray_floats, lab }], count, warnings, said, timing })
    }
}

/// A file's frames or pages that an import asks for, as decoded: in parts
/// that each share one orientation and profile, with what reading the file
/// left out, for the warnings, and did, for the verbose stream, and the
/// timing of an animation's frames asked for.
struct Chosen {
    parts: Vec<Part>,
    /// How many frames the parts hold together.
    count: usize,
    warnings: Vec<String>,
    said: Vec<String>,
    timing: Option<Timing>,
}

/// Frames or pages of a file that share one orientation and profile, each
/// as read or the reason it could not be.
struct Part {
    frames: Vec<Frame>,
    orientation: Result<Orientation, String>,
    profile: Result<Option<Profile>, String>,
    /// Whether each frame's float RGB holds a gray image's samples, which
    /// a gray profile converts from the first of each pixel's three.
    gray_floats: bool,
    /// Whether each frame's RGB holds a CIE Lab document's L, a and b as
    /// the document stores them.
    lab: bool,
}

/// A frame or page as decoded, with its CMYK samples where it has them,
/// its place among the frames loaded, and its index in the file.
struct Frame {
    image: DynamicImage,
    cmyk: Option<Cmyk>,
    place: usize,
    index: usize,
}

/// An animation's timing: each frame's delay in milliseconds, and how many
/// times it plays, 0 for endlessly.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Timing {
    pub(crate) delays: Vec<u32>,
    pub(crate) loops: u32,
}

/// What an import takes for one file: its Reading, the transform a vector
/// file is drawn through, the password a PDF opens with, and the frames or
/// pages to load, counted from 0, every one when `None`.
#[derive(Clone, Copy)]
pub(crate) struct Asked<'a> {
    pub(crate) reading: Reading,
    #[cfg_attr(all(feature = "module", not(any(feature = "svg", feature = "pdf"))), expect(dead_code, reason = "only a vector file's drawing reads the transform"))]
    pub(crate) transform: &'a Transform,
    pub(crate) password: Option<&'a str>,
    pub(crate) frames: Option<&'a [u32]>,
}

/// The indices of the frames or pages of `count` that `asked` names, in
/// its order, or why it names none or one past the last, `name` naming the
/// option; every one when `asked` is `None`.
fn chosen(asked: Option<&[u32]>, count: usize, name: &str) -> Result<Vec<usize>, String> {
    match asked {
        None => Ok((0..count).collect()),
        Some([]) => Err(format!("{name} names no frame")),
        Some(indices) => indices
            .iter()
            .map(|&index| match usize::try_from(index) {
                Ok(at) if at < count => Ok(at),
                _ => Err(format!("{name} names {index}, past its last frame or page, {}", count.saturating_sub(1))),
            })
            .collect(),
    }
}

/// Every frame `frames` composites, as RGBA, with the animation's timing:
/// each frame's delay rounded to whole milliseconds, and the times it
/// plays, `times`.
fn animated(frames: image::Frames<'_>, times: u32) -> Result<(Vec<DynamicImage>, Timing), String> {
    let mut images = Vec::new();
    let mut delays = Vec::new();
    for frame in frames {
        let frame = frame.map_err(|e| e.to_string())?;
        let (numerator, denominator) = frame.delay().numer_denom_ms();
        let milliseconds = (u64::from(numerator) + u64::from(denominator) / 2) / u64::from(denominator.max(1));
        delays.push(u32::try_from(milliseconds).map_err(|e| format!("a frame's delay of {milliseconds} ms is past what the import holds: {e}"))?);
        images.push(DynamicImage::ImageRgba8(frame.into_buffer()));
    }
    Ok((images, Timing { delays, loops: times }))
}

/// How many times an animated WebP plays, 0 for endlessly, by the count
/// its file gives, which counts every play.
#[cfg(not(feature = "webp"))]
fn plays(count: image::metadata::LoopCount) -> u32 {
    match count {
        image::metadata::LoopCount::Infinite => 0,
        image::metadata::LoopCount::Finite(times) => times.get(),
    }
}

/// How many times a GIF plays, 0 for endlessly, by the repeat count of the
/// NETSCAPE2.0 extension before its first frame, which counts the plays
/// after the first; once where it has none, as a browser plays it.
fn gif_plays(bytes: &[u8]) -> Result<u32, String> {
    let mut decoder = gif::DecodeOptions::new().read_info(Cursor::new(bytes)).map_err(|e| e.to_string())?;
    decoder.next_frame_info().map_err(|e| e.to_string())?;
    Ok(match decoder.repeat() {
        gif::Repeat::Infinite => 0,
        gif::Repeat::Finite(repeats) => u32::from(repeats) + 1,
    })
}

/// `frames` brought to one color type and size, as an array texture's
/// layers share them: where they differ, each becomes RGBA of the widest
/// type among them, 32-bit float, 16 or 8 bits, and is centered on a canvas
/// of the largest width and height, the rest `ground`, from 0 to 1.
fn one_size(frames: Vec<DynamicImage>, ground: [f32; 4]) -> Vec<DynamicImage> {
    let same = frames.windows(2).all(|pair| pair[0].color() == pair[1].color() && pair[0].dimensions() == pair[1].dimensions());
    if same {
        return frames;
    }
    let width = frames.iter().map(|frame| frame.width()).max().unwrap_or(1);
    let height = frames.iter().map(|frame| frame.height()).max().unwrap_or(1);
    let float = frames.iter().any(|frame| matches!(frame.color(), image::ColorType::Rgb32F | image::ColorType::Rgba32F));
    let sixteen = frames.iter().any(|frame| frame.color().bytes_per_pixel() / frame.color().channel_count() == 2);
    frames
        .into_iter()
        .map(|frame| {
            let (x, y) = (i64::from((width - frame.width()) / 2), i64::from((height - frame.height()) / 2));
            if float {
                let mut canvas = image::Rgba32FImage::from_pixel(width, height, image::Rgba(ground));
                image::imageops::replace(&mut canvas, &frame.to_rgba32f(), x, y);
                DynamicImage::ImageRgba32F(canvas)
            } else if sixteen {
                let mut canvas = image::ImageBuffer::from_pixel(width, height, image::Rgba(ground.map(|v| (v * 65535.0).round() as u16)));
                image::imageops::replace(&mut canvas, &frame.to_rgba16(), x, y);
                DynamicImage::ImageRgba16(canvas)
            } else {
                let mut canvas = image::RgbaImage::from_pixel(width, height, image::Rgba(ground.map(|v| (v * 255.0).round() as u8)));
                image::imageops::replace(&mut canvas, &frame.to_rgba8(), x, y);
                DynamicImage::ImageRgba8(canvas)
            }
        })
        .collect()
}

/// An image file's pixels as the import loads them.
pub(crate) struct Opened {
    /// Its frames or pages, in the order loaded, all of one color type
    /// and size.
    pub(crate) frames: Vec<DynamicImage>,
    /// The timing of an animation's frames loaded.
    pub(crate) timing: Option<Timing>,
    /// Whether the pixels are an HDR file's light, linear, with BT.2408's
    /// HDR reference white at 1 and brighter light above it, which loads
    /// as Rgba16Float where -Format names no other format.
    pub(crate) hdr: bool,
    /// Whether the pixels are a vector file drawn through the import's
    /// transform, its crop, rotation, flip and resize done.
    pub(crate) drawn: bool,
}

/// The image file at `path` decoded as `asked` says: the frames or pages it
/// names, each turned by the file's orientation and converted from its
/// profile unless told otherwise, or a vector file drawn as its transform
/// plans and opened with its password, all brought to one size as a
/// texture's layers are, with what was done on the verbose stream and what
/// could not be done in the warnings.
pub(crate) fn open(path: &Path, asked: Asked<'_>, said: &mut Said) -> PsResult<Opened> {
    let bytes = std::fs::read(path).map_err(|e| unreadable(path, &e))?;
    let Images { frames, ground, timing, hdr, drawn, .. } = images(path, &bytes, asked, said)?;
    let frames = one_size(frames, ground);
    if frames.len() > 1 && !drawn {
        said.verbose.push(format!("{} frames or pages loaded as the texture's layers", frames.len()));
    }
    Ok(Opened { frames, timing, hdr, drawn })
}

/// An image file's frames or pages as decoded, each at its own size, with
/// what an `Opened` says of them.
struct Images {
    frames: Vec<DynamicImage>,
    /// Each frame's pixels as the file stores them, turned as `frames` are,
    /// where the Reading keeps them; none otherwise.
    stored: Vec<Kept>,
    /// What a canvas larger than a frame holds around it, from 0 to 1.
    ground: [f32; 4],
    timing: Option<Timing>,
    hdr: bool,
    drawn: bool,
}

/// A frame as loaded: its place among the frames loaded, its pixels, and
/// its pixels as the file stores them, turned the same way, where the
/// Reading keeps them.
struct Loaded {
    place: usize,
    image: DynamicImage,
    stored: Option<Kept>,
}

/// The image file at `path`, holding `bytes`, decoded as `asked` says, each
/// frame or page at its own size: the frames or pages it names, each turned
/// by the file's orientation and converted from its profile unless told
/// otherwise, or a vector file drawn as its transform plans and opened with
/// its password, with what was done on the verbose stream and what could
/// not be done in the warnings.
fn images(path: &Path, bytes: &[u8], asked: Asked<'_>, said: &mut Said) -> PsResult<Images> {
    if let Some(vector) = vector(path, bytes) {
        return drawn(path, bytes, vector, asked, said);
    }
    let reading = asked.reading;
    reading.pdf_unused("the file", asked.password, said);
    let chosen = decode(path, bytes, reading, asked.frames)?;
    said.warnings.extend(chosen.warnings);
    said.verbose.extend(chosen.said);
    let several = chosen.parts.len() > 1;
    let mut slots: Vec<Option<Loaded>> = (0..chosen.count).map(|_| None).collect();
    let mut hdr = false;
    let mut unchanged = Vec::new();
    for part in chosen.parts {
        let pages = pages_named(&part.frames.iter().map(|frame| frame.index).collect::<Vec<_>>());
        let mut notes = Said::default();
        let (frames, part_hdr, part_unchanged) = loaded(path, part, reading, &mut notes)?;
        hdr |= part_hdr;
        unchanged.push(part_unchanged);
        let scoped = |message: String| if several { format!("{pages}: {message}") } else { message };
        said.warnings.extend(notes.warnings.into_iter().map(&scoped));
        said.verbose.extend(notes.verbose.into_iter().map(&scoped));
        for frame in frames {
            let place = frame.place;
            slots[place] = Some(frame);
        }
    }
    said.warnings.extend(Unchanged::warnings(&unchanged));
    let mut frames = Vec::with_capacity(slots.len());
    let mut stored = Vec::new();
    for (place, slot) in slots.into_iter().enumerate() {
        let Some(frame) = slot else {
            return Err(unreadable(path, &format!("its frame {place} was not loaded")));
        };
        frames.push(frame.image);
        stored.extend(frame.stored);
    }
    Ok(Images { frames, stored, ground: [0.0; 4], timing: chosen.timing, hdr, drawn: false })
}

/// The options that read a file's profile and orientation which changed
/// nothing for a part of it, each as the warning that says so.
#[derive(Default)]
struct Unchanged {
    ignore_profile: Option<String>,
    intent: Option<String>,
    ignore_orientation: Option<String>,
}

impl Unchanged {
    /// The warnings of the options that changed nothing for every part of
    /// a file, each once.
    fn warnings(parts: &[Unchanged]) -> Vec<String> {
        let options: [fn(&Unchanged) -> Option<&String>; 3] = [|part| part.ignore_profile.as_ref(), |part| part.intent.as_ref(), |part| part.ignore_orientation.as_ref()];
        let mut out: Vec<String> = Vec::new();
        for option in options {
            if let Some(messages) = parts.iter().map(option).collect::<Option<Vec<_>>>() {
                for message in messages {
                    if !out.contains(message) {
                        out.push(message.clone());
                    }
                }
            }
        }
        out
    }
}

/// Pages by their indices in a file, counted from 1 as a person numbers
/// them, each once: "page 2", "pages 1 and 3", "pages 1, 3 and 4".
fn pages_named(indices: &[usize]) -> String {
    let mut numbers: Vec<String> = Vec::new();
    for index in indices {
        let number = (index + 1).to_string();
        if !numbers.contains(&number) {
            numbers.push(number);
        }
    }
    match numbers.as_slice() {
        [] => "no page".to_string(),
        [one] => format!("page {one}"),
        [rest @ .., last] => format!("pages {} and {last}", rest.join(", ")),
    }
}

/// A part's frames converted from its profile and turned by its
/// orientation as `reading` asks, each as loaded; with whether they are an
/// HDR file's light, what was done and could not be done going to `said`,
/// and the options that changed nothing for the part.
fn loaded(path: &Path, part: Part, reading: Reading, said: &mut Said) -> PsResult<(Vec<Loaded>, bool, Unchanged)> {
    let Part { frames, orientation, profile, gray_floats, lab } = part;
    let names = reading.names;
    let places: Vec<usize> = frames.iter().map(|frame| frame.place).collect();
    let mut kept = Vec::new();
    if reading.stored {
        for frame in &frames {
            kept.push(as_stored(&frame.image, frame.cmyk.as_ref(), lab, gray_floats).map_err(|e| unreadable(path, &e))?);
        }
    }
    let mut unchanged = Unchanged::default();
    let stored = |frames: Vec<Frame>| frames.into_iter().map(|frame| frame.image).collect::<Vec<_>>();
    let unused = |why: &str| format!("the color profile of {} was not used, as {why}, so its colors loaded as stored", path.display());
    let (mut images, hdr) = match profile {
        Err(why) => {
            said.warnings.push(unused(&why));
            (stored(frames), false)
        }
        Ok(None) => {
            if reading.ignore_profile {
                unchanged.ignore_profile = Some(format!("the file carries no color profile, so {} changed nothing", names.ignore_profile));
            }
            if reading.intent.is_some() {
                unchanged.intent = Some(format!("the file carries no color profile, so {} changed nothing", names.intent));
            }
            (stored(frames), false)
        }
        Ok(Some(_)) if reading.ignore_profile => {
            said.verbose.push(format!("the colors loaded as stored, as {} asks", names.ignore_profile));
            (stored(frames), false)
        }
        #[cfg(any(feature = "avif", feature = "jxl"))]
        Ok(Some(Profile::Cicp(cicp))) if cicp::HdrTransfer::of(&cicp).is_some() => {
            let linear: Result<Vec<_>, String> = frames.iter().map(|frame| cicp::hdr_linear(&frame.image, &cicp)).collect();
            match linear {
                Ok(linear) => {
                    let transfer = match linear.first() {
                        Some((_, transfer)) => transfer.to_string(),
                        None => String::new(),
                    };
                    said.verbose.push(format!(
                        "the colors were converted from the file's {transfer} light in {:?} primaries to linear light in sRGB's, BT.2408's HDR reference white of 203 nits at 1",
                        cicp.color_primaries
                    ));
                    if reading.intent.is_some() {
                        unchanged.intent = Some(format!("the file's HDR light converts by its transfer alone, so {} changed nothing", names.intent));
                    }
                    (linear.into_iter().map(|(image, _)| image).collect(), true)
                }
                Err(why) => {
                    said.warnings.push(unused(&why));
                    (stored(frames), false)
                }
            }
        }
        Ok(Some(profile)) => {
            let intent = reading.intent();
            let converted: Result<Vec<Converted>, String> = frames
                .iter()
                .map(|frame| if gray_floats { float_converted(&frame.image, true, &profile, intent) } else { convert(&frame.image, frame.cmyk.as_ref(), &profile, intent) })
                .collect();
            match converted {
                Ok(converted) => {
                    if let Some(first) = converted.first() {
                        let how = match &first.table {
                            Some(table) => format!(", {}", table.said),
                            None => String::new(),
                        };
                        let from = match profile {
                            Profile::Lab => "the document's CIE Lab".to_string(),
                            _ => format!("the file's color profile, {},", first.name),
                        };
                        let to = if first.linear { "linear light in sRGB's primaries" } else { "sRGB" };
                        said.verbose.push(format!("the colors were converted from {from} to {to}, {}{how}", intent.words()));
                    }
                    if matches!(profile, Profile::Lab) && reading.intent.is_some() {
                        unchanged.intent = Some(format!("the document's CIE Lab converts to sRGB the same with every intent, so {} changed nothing", names.intent));
                    }
                    (converted.into_iter().map(|converted| converted.image).collect(), false)
                }
                Err(why) => {
                    said.warnings.push(unused(&why));
                    (stored(frames), false)
                }
            }
        }
    };
    match orientation {
        Err(why) => said.warnings.push(format!("the orientation of {} was not used, as {why}, so its pixels loaded as stored", path.display())),
        Ok(Orientation::NoTransforms) if reading.ignore_orientation => {
            unchanged.ignore_orientation = Some(format!("the file records no orientation, so {} changed nothing", names.ignore_orientation));
        }
        Ok(Orientation::NoTransforms) => {}
        Ok(orientation) if reading.ignore_orientation => {
            said.verbose.push(format!("the pixels loaded as stored, as {} asks, where the file's orientation says they are {}", names.ignore_orientation, turned(orientation)));
        }
        Ok(orientation) => {
            for image in &mut images {
                image.apply_orientation(orientation);
            }
            kept = kept.into_iter().map(|frame| frame.oriented(orientation)).collect();
            said.verbose.push(format!("the pixels were {}, as the file's orientation says", turned(orientation)));
        }
    }
    let mut kept = kept.into_iter();
    let frames = places.into_iter().zip(images).map(|(place, image)| Loaded { place, image, stored: kept.next() }).collect();
    Ok((frames, hdr, unchanged))
}

/// The vector formats the import draws.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Vector {
    Svg,
    Pdf,
}

/// The vector format `bytes` hold, told by their content: a PDF's header,
/// or the markup or gzip stream an SVG or SVGZ file begins with; and by
/// `path`'s extension where neither that nor any other format the import
/// reads tells.
fn vector(path: &Path, bytes: &[u8]) -> Option<Vector> {
    if bytes.starts_with(b"%PDF-") {
        return Some(Vector::Pdf);
    }
    let text = match bytes.strip_prefix(b"\xEF\xBB\xBF") {
        Some(rest) => rest,
        None => bytes,
    };
    if text.iter().find(|b| !b.is_ascii_whitespace()) == Some(&b'<') || bytes.starts_with(&[0x1F, 0x8B]) {
        return Some(Vector::Svg);
    }
    if bytes.starts_with(b"8BPS") || is_jpeg_xl(bytes) || image::guess_format(bytes).is_ok() {
        return None;
    }
    match path.extension().map(|e| e.to_string_lossy().to_ascii_lowercase()).as_deref() {
        Some("pdf") => Some(Vector::Pdf),
        Some("svg" | "svgz") => Some(Vector::Svg),
        _ => None,
    }
}

/// The refusal of a `vector` file at `path` in a build without the
/// feature that draws it.
#[cfg(not(all(feature = "svg", feature = "pdf")))]
fn undrawable(path: &Path, vector: Vector) -> PsError {
    let (what, feature) = match vector {
        Vector::Svg => ("an SVG file", "svg"),
        Vector::Pdf => ("a PDF file", "pdf"),
    };
    unreadable(path, &format!("it is {what}, which wgpu-pwsh reads only with its {feature} feature"))
}

/// A vector file drawn: its pages' pixels, straight RGBA, each at its own
/// size, what a canvas larger than a page holds around it, from 0 to 1, and
/// what drawing it left out, for the warnings, and did, for the verbose
/// stream.
#[cfg(any(feature = "svg", feature = "pdf"))]
struct Drawing {
    frames: Vec<DynamicImage>,
    ground: [f32; 4],
    warnings: Vec<String>,
    said: Vec<String>,
}

/// `value`, a size in pixels, rounded up to a whole number of them, at
/// least 1: a vector file's own raster covers all of the file.
#[cfg(any(feature = "svg", feature = "pdf"))]
fn pixels_up(value: f64) -> u32 {
    value.ceil().clamp(1.0, f64::from(u32::MAX)) as u32
}

/// Adds `entry` to `list` unless it holds it already, from whichever
/// thread a drawing's callback runs on.
#[cfg(any(feature = "svg", feature = "pdf"))]
fn noted(list: &arc_swap::ArcSwap<Vec<String>>, entry: String) {
    list.rcu(|held| {
        let mut held = Vec::clone(held);
        if !held.contains(&entry) {
            held.push(entry.clone());
        }
        held
    });
}

/// The vector file at `path`, holding `bytes`, drawn as `asked` says: its
/// pages or its one drawing, each at the size and turn its transform plans
/// for the size it gives, a PDF opened with its password.
#[cfg(any(feature = "svg", feature = "pdf"))]
fn drawn(path: &Path, bytes: &[u8], vector: Vector, asked: Asked<'_>, said: &mut Said) -> PsResult<Images> {
    let (reading, transform, password) = (asked.reading, asked.transform, asked.password);
    let names = reading.names;
    if reading.ignore_orientation {
        said.warnings.push(format!("the file records no orientation, so {} changed nothing", names.ignore_orientation));
    }
    for (asked, name) in [(reading.ignore_profile, names.ignore_profile), (reading.intent.is_some(), names.intent)] {
        if asked {
            said.warnings.push(format!("the file is drawn in sRGB, so {name} changed nothing"));
        }
    }
    if transform.filter.is_some() {
        said.warnings.push("the file was drawn at its new size, so -Filter changed nothing".to_string());
    }
    let drawing = match vector {
        #[cfg(feature = "svg")]
        Vector::Svg => {
            reading.pdf_unused("the file", password, said);
            let picked = chosen(asked.frames, 1, names.frames).map_err(|e| unreadable(path, &e))?;
            let mut drawing = svg::draw(path, bytes, transform, svg::system_fonts())?;
            drawing.frames = picked.iter().flat_map(|_| drawing.frames.first().cloned()).collect();
            drawing
        }
        #[cfg(feature = "pdf")]
        Vector::Pdf => pdf::draw(path, bytes, reading, transform, password, asked.frames)?,
        #[cfg(not(all(feature = "svg", feature = "pdf")))]
        other => return Err(undrawable(path, other)),
    };
    said.warnings.extend(drawing.warnings);
    said.verbose.extend(drawing.said);
    let stored = if reading.stored {
        drawing.frames.iter().map(|frame| as_stored(frame, None, false, false)).collect::<Result<Vec<_>, String>>().map_err(|e| unreadable(path, &e))?
    } else {
        Vec::new()
    };
    Ok(Images { frames: drawing.frames, stored, ground: drawing.ground, timing: None, hdr: false, drawn: true })
}

/// The refusal of a vector file in a build that draws neither SVG nor PDF.
#[cfg(not(any(feature = "svg", feature = "pdf")))]
fn drawn(path: &Path, _bytes: &[u8], vector: Vector, _asked: Asked<'_>, _said: &mut Said) -> PsResult<Images> {
    Err(undrawable(path, vector))
}

fn unreadable(path: &Path, e: &dyn Display) -> PsError {
    invalid("WgpuImage", format!("{} cannot be read as an image: {e}", path.display()))
}

/// What applying `orientation` does to the pixels, in words.
fn turned(orientation: Orientation) -> &'static str {
    match orientation {
        Orientation::NoTransforms => "left as stored",
        Orientation::Rotate90 => "turned 90 degrees clockwise",
        Orientation::Rotate180 => "turned 180 degrees",
        Orientation::Rotate270 => "turned 90 degrees counterclockwise",
        Orientation::FlipHorizontal => "mirrored left to right",
        Orientation::FlipVertical => "mirrored top to bottom",
        Orientation::Rotate90FlipH => "turned 90 degrees clockwise and mirrored left to right",
        Orientation::Rotate270FlipH => "turned 90 degrees counterclockwise and mirrored left to right",
    }
}

/// The orientation an EXIF block gives, read past the "Exif\0\0" header
/// some writers put before its TIFF data, as Pillow does in a WebP file's
/// EXIF chunk; a block with no orientation tag records none.
fn orientation_in(exif: &[u8]) -> Orientation {
    let tiff = match exif.strip_prefix(b"Exif\0\0") {
        Some(rest) => rest,
        None => exif,
    };
    match Orientation::from_exif_chunk(tiff) {
        Some(orientation) => orientation,
        None => Orientation::NoTransforms,
    }
}

/// The orientation a decoder's EXIF block gives, or why it cannot be read.
fn exif_orientation(decoder: &mut impl ImageDecoder) -> Result<Orientation, String> {
    match decoder.exif_metadata() {
        Ok(Some(exif)) => Ok(orientation_in(&exif)),
        Ok(None) => Ok(Orientation::NoTransforms),
        Err(e) => Err(format!("it cannot be read: {e}")),
    }
}

/// Whether `bytes` begin as a JPEG XL file does: with a bare codestream's
/// signature, FF 0A, or with the container's signature box.
fn is_jpeg_xl(bytes: &[u8]) -> bool {
    bytes.starts_with(&[0xFF, 0x0A]) || bytes.starts_with(&[0, 0, 0, 0x0C, b'J', b'X', b'L', b' ', 0x0D, 0x0A, 0x87, 0x0A])
}

/// The frames or pages of the file that `asked` names, every one when it is
/// `None`, and what the file says about them, by its format: told by its
/// content, and by its extension where the content does not tell.
fn decode(path: &Path, bytes: &[u8], reading: Reading, asked: Option<&[u32]>) -> PsResult<Chosen> {
    let whole = |decoded: Decoded| decoded.chosen(asked, reading.names.frames).map_err(|e| unreadable(path, &e));
    if bytes.starts_with(b"8BPS") {
        return whole(photoshop(path, bytes)?);
    }
    if is_jpeg_xl(bytes) {
        #[cfg(feature = "jxl")]
        return whole(jxl::decode(path, bytes)?);
        #[cfg(not(feature = "jxl"))]
        return Err(unreadable(path, &"it is a JPEG XL file, which wgpu-pwsh reads only with its jxl feature"));
    }
    let mut reader = ImageReader::new(Cursor::new(bytes)).with_guessed_format().map_err(|e| unreadable(path, &e))?;
    let format = match reader.format() {
        Some(format) => format,
        None => ImageFormat::from_path(path).map_err(|e| unreadable(path, &e))?,
    };
    reader.set_format(format);
    match format {
        ImageFormat::Tiff => tiff(path, bytes, reading, asked),
        format => whole(raster(path, bytes, format, reader, reading)?),
    }
}

/// A raster file of `format` other than TIFF, which `reader` reads: its
/// frames, and what it says about them.
fn raster(path: &Path, bytes: &[u8], format: ImageFormat, reader: ImageReader<Cursor<&[u8]>>, reading: Reading) -> PsResult<Decoded> {
    match format {
        #[cfg(feature = "avif")]
        ImageFormat::Avif => avif::decode(path, bytes),
        #[cfg(not(feature = "avif"))]
        ImageFormat::Avif => Err(unreadable(path, &"it is an .avif file, which wgpu-pwsh reads only with its avif feature")),
        ImageFormat::Jpeg => jpeg(path, bytes, reader, reading),
        ImageFormat::Png => {
            let mut decoder = PngDecoder::new(Cursor::new(bytes)).map_err(|e| unreadable(path, &e))?;
            let orientation = match decoder.exif_metadata() {
                Ok(Some(exif)) => Ok(orientation_in(&exif)),
                Ok(None) => png_text_orientation(bytes).map(|found| found.unwrap_or(Orientation::NoTransforms)),
                Err(e) => Err(format!("it cannot be read: {e}")),
            };
            let profile = decoder.icc_profile().map(|icc| icc.map(Profile::Icc)).map_err(|e| format!("it cannot be read: {e}"));
            if decoder.is_apng().map_err(|e| unreadable(path, &e))? {
                let sixteen = matches!(decoder.color_type(), image::ColorType::L16 | image::ColorType::La16 | image::ColorType::Rgb16 | image::ColorType::Rgba16);
                let animation = if sixteen {
                    apng::<image::Rgba<u16>, 2>(bytes, u16::from_be_bytes).map(|(frames, timing)| (frames.into_iter().map(DynamicImage::ImageRgba16).collect(), timing))
                } else {
                    apng::<image::Rgba<u8>, 1>(bytes, |[sample]| sample).map(|(frames, timing)| (frames.into_iter().map(DynamicImage::ImageRgba8).collect(), timing))
                };
                let (frames, timing): (Vec<DynamicImage>, Timing) = animation.map_err(|e| unreadable(path, &e))?;
                return Decoded::animation(frames, timing, orientation, profile).map_err(|e| unreadable(path, &e));
            }
            let image = DynamicImage::from_decoder(decoder).map_err(|e| unreadable(path, &e))?;
            Ok(Decoded { image, orientation, profile, cmyk: None, gray_floats: false, lab: false, warnings: Vec::new(), said: Vec::new(), rest: Vec::new(), timing: None })
        }
        ImageFormat::Gif => {
            let mut decoder = GifDecoder::new(Cursor::new(bytes)).map_err(|e| unreadable(path, &e))?;
            let orientation = exif_orientation(&mut decoder);
            let profile = decoder.icc_profile().map(|icc| icc.map(Profile::Icc)).map_err(|e| format!("it cannot be read: {e}"));
            let times = gif_plays(bytes).map_err(|e| unreadable(path, &e))?;
            let (frames, timing) = animated(decoder.into_frames(), times).map_err(|e| unreadable(path, &e))?;
            Decoded::animation(frames, timing, orientation, profile).map_err(|e| unreadable(path, &e))
        }
        #[cfg(feature = "webp")]
        ImageFormat::WebP => webp(path, bytes),
        #[cfg(not(feature = "webp"))]
        ImageFormat::WebP => {
            let mut decoder = WebPDecoder::new(Cursor::new(bytes)).map_err(|e| unreadable(path, &e))?;
            let orientation = exif_orientation(&mut decoder);
            let profile = decoder.icc_profile().map(|icc| icc.map(Profile::Icc)).map_err(|e| format!("it cannot be read: {e}"));
            if decoder.has_animation() {
                let times = plays(decoder.loop_count());
                let (frames, timing) = animated(decoder.into_frames(), times).map_err(|e| unreadable(path, &e))?;
                return Decoded::animation(frames, timing, orientation, profile).map_err(|e| unreadable(path, &e));
            }
            let image = DynamicImage::from_decoder(decoder).map_err(|e| unreadable(path, &e))?;
            Ok(Decoded { image, orientation, profile, cmyk: None, gray_floats: false, lab: false, warnings: Vec::new(), said: Vec::new(), rest: Vec::new(), timing: None })
        }
        _ => {
            let mut decoder = reader.into_decoder().map_err(|e| unreadable(path, &e))?;
            let orientation = exif_orientation(&mut decoder);
            let profile = decoder.icc_profile().map(|icc| icc.map(Profile::Icc)).map_err(|e| format!("it cannot be read: {e}"));
            let image = DynamicImage::from_decoder(decoder).map_err(|e| unreadable(path, &e))?;
            Ok(Decoded { image, orientation, profile, cmyk: None, gray_floats: false, lab: false, warnings: Vec::new(), said: Vec::new(), rest: Vec::new(), timing: None })
        }
    }
}

/// The frames of an APNG, `bytes`, composited as the APNG specification
/// says, in RGBA of its samples, `N` bytes each, which `sample` reads from
/// the big-endian bytes PNG stores: each frame drawn over the canvas or in
/// place of it as its blend says, then its area cleared, or put back as it
/// was before the frame, as its dispose says, a first frame's put back
/// counting as cleared. The default image is no frame where no frame
/// control comes before it. With each frame's delay, its fraction of a
/// second, a denominator of 0 counting hundredths, rounded to whole
/// milliseconds, and the times the file plays.
#[allow(clippy::type_complexity)]
fn apng<P: image::Pixel + 'static, const N: usize>(bytes: &[u8], sample: fn([u8; N]) -> P::Subpixel) -> Result<(Vec<ImageBuffer<P, Vec<P::Subpixel>>>, Timing), String> {
    use image::{GenericImage, Primitive};
    let mut decoder = png::Decoder::new(Cursor::new(bytes));
    decoder.set_transformations(png::Transformations::EXPAND);
    let mut reader = decoder.read_info().map_err(|e| e.to_string())?;
    let (width, height) = (reader.info().width, reader.info().height);
    let (count, times) = match reader.info().animation_control() {
        Some(control) => (control.num_frames, control.num_plays),
        None => return Err("it has no animation control chunk".to_string()),
    };
    let size = reader.output_buffer_size().ok_or("its frames are larger than memory can address")?;
    let mut buffer = vec![0u8; size];
    if reader.info().frame_control().is_none() {
        reader.next_frame(&mut buffer).map_err(|e| e.to_string())?;
    }
    let max = <P::Subpixel as Primitive>::DEFAULT_MAX_VALUE;
    let mut canvas: ImageBuffer<P, Vec<P::Subpixel>> = ImageBuffer::new(width, height);
    let mut frames = Vec::new();
    let mut delays = Vec::new();
    for index in 0..count {
        let output = reader.next_frame(&mut buffer).map_err(|e| e.to_string())?;
        let (w, h, x, y, blend, dispose, (num, den)) = match reader.info().frame_control() {
            Some(control) => (control.width, control.height, control.x_offset, control.y_offset, control.blend_op, control.dispose_op, (control.delay_num, control.delay_den)),
            None => (width, height, 0, 0, png::BlendOp::Source, png::DisposeOp::None, (0, 1)),
        };
        if x.checked_add(w).is_none_or(|right| right > width) || y.checked_add(h).is_none_or(|bottom| bottom > height) {
            return Err(format!("frame {index}'s area is past the canvas"));
        }
        let dispose = match dispose {
            png::DisposeOp::Previous if index == 0 => png::DisposeOp::Background,
            other => other,
        };
        let before = match dispose {
            png::DisposeOp::Previous => Some(canvas.view(x, y, w, h).to_image()),
            _ => None,
        };
        let samples: Vec<P::Subpixel> = buffer[..output.buffer_size()].as_chunks::<N>().0.iter().map(|&bytes| sample(bytes)).collect();
        let rgba: Vec<P::Subpixel> = match output.color_type {
            png::ColorType::Rgba => samples,
            png::ColorType::Rgb => samples.as_chunks::<3>().0.iter().flat_map(|&[r, g, b]| [r, g, b, max]).collect(),
            png::ColorType::GrayscaleAlpha => samples.as_chunks::<2>().0.iter().flat_map(|&[v, a]| [v, v, v, a]).collect(),
            png::ColorType::Grayscale => samples.iter().flat_map(|&v| [v, v, v, max]).collect(),
            png::ColorType::Indexed => return Err(format!("frame {index} is still indexed after its palette was expanded")),
        };
        let source = ImageBuffer::<P, Vec<P::Subpixel>>::from_raw(w, h, rgba).ok_or_else(|| format!("frame {index}'s samples do not fill its area"))?;
        match blend {
            png::BlendOp::Source => canvas.copy_from(&source, x, y).map_err(|e| format!("frame {index} does not fit the canvas: {e}"))?,
            png::BlendOp::Over => {
                for (sx, sy, pixel) in source.enumerate_pixels() {
                    canvas.get_pixel_mut(x + sx, y + sy).blend(pixel);
                }
            }
        }
        frames.push(canvas.clone());
        let (milliseconds, per) = (u32::from(num) * 1000, if den == 0 { 100 } else { u32::from(den) });
        delays.push((milliseconds + per / 2) / per);
        match (dispose, before) {
            (png::DisposeOp::Background, _) => canvas.copy_from(&ImageBuffer::<P, Vec<P::Subpixel>>::new(w, h), x, y).map_err(|e| format!("frame {index}'s area does not clear: {e}"))?,
            (png::DisposeOp::Previous, Some(before)) => canvas.copy_from(&before, x, y).map_err(|e| format!("frame {index}'s area is not put back: {e}"))?,
            _ => {}
        }
    }
    Ok((frames, Timing { delays, loops: times }))
}

/// A WebP file's frames as libwebp composites them, a still file's one
/// among them, with the orientation and ICC profile of its EXIF and ICCP
/// chunks, which the image crate reads; where it cannot, the pixels still
/// load and a warning says why.
#[cfg(feature = "webp")]
fn webp(path: &Path, bytes: &[u8]) -> PsResult<Decoded> {
    let (orientation, profile) = match WebPDecoder::new(Cursor::new(bytes)) {
        Ok(mut decoder) => (exif_orientation(&mut decoder), decoder.icc_profile().map(|icc| icc.map(Profile::Icc)).map_err(|e| format!("it cannot be read: {e}"))),
        Err(e) => (Err(format!("its chunks cannot be read: {e}")), Err(format!("its chunks cannot be read: {e}"))),
    };
    let read = crate::webp::decode(bytes).map_err(|e| unreadable(path, &e))?;
    let frames = read
        .frames
        .into_iter()
        .map(|rgba| image::RgbaImage::from_raw(read.width, read.height, rgba).map(DynamicImage::ImageRgba8).ok_or_else(|| unreadable(path, &"a frame's samples do not fill its canvas")))
        .collect::<PsResult<Vec<_>>>()?;
    Decoded::animation(frames, Timing { delays: read.delays, loops: read.plays }, orientation, profile).map_err(|e| unreadable(path, &e))
}

/// The orientation in a PNG's "Raw profile type exif" text chunk, where
/// ImageMagick writes a file's EXIF in place of an eXIf chunk: hexadecimal
/// after a line naming the profile and one giving its length, read as
/// Pillow reads it. The text read is what comes before the image data,
/// where ImageMagick writes it, and a compressed chunk inflates within the
/// png crate's own limit. `None` for a file without the chunk there.
fn png_text_orientation(bytes: &[u8]) -> Result<Option<Orientation>, String> {
    const KEYWORD: &str = "Raw profile type exif";
    let unreadable = |e: &dyn Display| format!("its {KEYWORD} text cannot be read: {e}");
    let reader = png::Decoder::new(Cursor::new(bytes)).read_info().map_err(|e| unreadable(&e))?;
    let info = reader.info();
    let text = if let Some(chunk) = info.uncompressed_latin1_text.iter().find(|c| c.keyword == KEYWORD) {
        chunk.text.clone()
    } else if let Some(chunk) = info.compressed_latin1_text.iter().find(|c| c.keyword == KEYWORD) {
        let mut chunk = chunk.clone();
        chunk.decompress_text().map_err(|e| unreadable(&e))?;
        chunk.get_text().map_err(|e| unreadable(&e))?
    } else if let Some(chunk) = info.utf8_text.iter().find(|c| c.keyword == KEYWORD) {
        let mut chunk = chunk.clone();
        chunk.decompress_text().map_err(|e| unreadable(&e))?;
        chunk.get_text().map_err(|e| unreadable(&e))?
    } else {
        return Ok(None);
    };
    let digits = text
        .lines()
        .skip(3)
        .flat_map(str::chars)
        .filter(|c| !c.is_whitespace())
        .map(|c| c.to_digit(16).ok_or(c))
        .collect::<Result<Vec<u32>, char>>()
        .map_err(|c| format!("its {KEYWORD} text holds {c:?}, which is no hexadecimal digit"))?;
    if digits.len() % 2 == 1 {
        return Err(format!("its {KEYWORD} text holds an odd number of hexadecimal digits"));
    }
    let exif: Vec<u8> = digits.as_chunks::<2>().0.iter().map(|[high, low]| (high * 16 + low) as u8).collect();
    Ok(Some(orientation_in(&exif)))
}

/// A Photoshop document's picture through the module's own reader, with
/// the orientation its EXIF gives and its color: CIE Lab for a Lab
/// document, and its ICC profile for any other. A CMYK document's inks as
/// stored go to the profile to convert, beside the RGB the formula
/// (max - c)(max - k) / max gives, which loads where it has no profile;
/// its alpha goes with both. A Lab document's L, a and b load as red,
/// green and blue where they are not converted, and a 32-bit gray
/// document's gray as all three, which its gray profile converts from.
fn photoshop(path: &Path, bytes: &[u8]) -> PsResult<Decoded> {
    use crate::psd::{Color, Samples};
    use DynamicImage as D;
    let document = crate::psd::read(bytes, COLORS).map_err(|e| unreadable(path, &e))?;
    let picture = document.picture;
    let (w, h, alpha) = (picture.width, picture.height, picture.alpha);
    let orientation = Ok(match &document.exif {
        Some(exif) => orientation_in(exif),
        None => Orientation::NoTransforms,
    });
    let profile = Ok(match picture.color {
        Color::Lab => Some(Profile::Lab),
        _ => document.icc.map(Profile::Icc),
    });
    let unfit = |e: String| unreadable(path, &e);
    let stride = picture.color.channels() + usize::from(alpha);
    let gray_floats = matches!((picture.color, &picture.samples), (Color::Gray, Samples::Float(_)));
    let lab = matches!(picture.color, Color::Lab);
    let (image, cmyk) = match (picture.color, picture.samples) {
        (Color::Gray, Samples::Eight(s)) if alpha => (D::ImageLumaA8(filled(w, h, s).map_err(unfit)?), None),
        (Color::Gray, Samples::Eight(s)) => (D::ImageLuma8(filled(w, h, s).map_err(unfit)?), None),
        (Color::Gray, Samples::Sixteen(s)) if alpha => (D::ImageLumaA16(filled(w, h, s).map_err(unfit)?), None),
        (Color::Gray, Samples::Sixteen(s)) => (D::ImageLuma16(filled(w, h, s).map_err(unfit)?), None),
        (Color::Gray, Samples::Float(s)) => {
            let mut rgb = Vec::with_capacity(s.len() / stride * (3 + usize::from(alpha)));
            for pixel in s.chunks_exact(stride) {
                rgb.extend_from_slice(&[pixel[0]; 3]);
                if alpha {
                    rgb.push(pixel[1]);
                }
            }
            (if alpha { D::ImageRgba32F(filled(w, h, rgb).map_err(unfit)?) } else { D::ImageRgb32F(filled(w, h, rgb).map_err(unfit)?) }, None)
        }
        (Color::Rgb | Color::Lab, Samples::Eight(s)) if alpha => (D::ImageRgba8(filled(w, h, s).map_err(unfit)?), None),
        (Color::Rgb | Color::Lab, Samples::Eight(s)) => (D::ImageRgb8(filled(w, h, s).map_err(unfit)?), None),
        (Color::Rgb | Color::Lab, Samples::Sixteen(s)) if alpha => (D::ImageRgba16(filled(w, h, s).map_err(unfit)?), None),
        (Color::Rgb | Color::Lab, Samples::Sixteen(s)) => (D::ImageRgb16(filled(w, h, s).map_err(unfit)?), None),
        (Color::Rgb | Color::Lab, Samples::Float(s)) if alpha => (D::ImageRgba32F(filled(w, h, s).map_err(unfit)?), None),
        (Color::Rgb | Color::Lab, Samples::Float(s)) => (D::ImageRgb32F(filled(w, h, s).map_err(unfit)?), None),
        (Color::Cmyk, Samples::Eight(s)) => {
            let (inks, rgb) = cmyk_formula(&s, stride, 255, |v| v as u8);
            (if alpha { D::ImageRgba8(filled(w, h, rgb).map_err(unfit)?) } else { D::ImageRgb8(filled(w, h, rgb).map_err(unfit)?) }, Some(Cmyk::Eight(inks)))
        }
        (Color::Cmyk, Samples::Sixteen(s)) => {
            let (inks, rgb) = cmyk_formula(&s, stride, 65535, |v| v as u16);
            (if alpha { D::ImageRgba16(filled(w, h, rgb).map_err(unfit)?) } else { D::ImageRgb16(filled(w, h, rgb).map_err(unfit)?) }, Some(Cmyk::Sixteen(inks)))
        }
        (Color::Cmyk, Samples::Float(_)) => return Err(unreadable(path, &"its CMYK samples are floats, an encoding the module does not know")),
    };
    Ok(Decoded { image, orientation, profile, cmyk, gray_floats, lab, warnings: document.warnings, said: document.said, rest: Vec::new(), timing: None })
}

/// CMYK samples, `stride` to a pixel, as their four inks and as the RGB the
/// formula (max - c)(max - k) / max gives, `max` the samples' largest
/// value, each pixel's alpha after its RGB where it has one; `to` takes a
/// value within 0 to `max` back to a sample.
fn cmyk_formula<T: Copy + Into<u32>>(samples: &[T], stride: usize, max: u32, to: fn(u32) -> T) -> (Vec<T>, Vec<T>) {
    let pixels = samples.len() / stride;
    let mut inks = Vec::with_capacity(pixels * 4);
    let mut rgb = Vec::with_capacity(pixels * (stride - 1));
    for pixel in samples.chunks_exact(stride) {
        inks.extend_from_slice(&pixel[..4]);
        let k: u32 = pixel[3].into();
        for &ink in &pixel[..3] {
            let ink: u32 = ink.into();
            rgb.push(to((max - ink) * (max - k) / max));
        }
        rgb.extend_from_slice(&pixel[4..]);
    }
    (inks, rgb)
}

/// A JPEG through the image crate, and a CMYK one's samples as stored
/// through zune-jpeg when it carries a profile to convert them by or the
/// Reading keeps the pixels as stored.
fn jpeg(path: &Path, bytes: &[u8], reader: ImageReader<Cursor<&[u8]>>, reading: Reading) -> PsResult<Decoded> {
    let mut decoder = reader.into_decoder().map_err(|e| unreadable(path, &e))?;
    let orientation = exif_orientation(&mut decoder);
    let profile = decoder.icc_profile().map(|icc| icc.map(Profile::Icc)).map_err(|e| format!("it cannot be read: {e}"));
    let image = DynamicImage::from_decoder(decoder).map_err(|e| unreadable(path, &e))?;
    let cmyk = match &profile {
        Ok(Some(_)) if !reading.ignore_profile => jpeg_cmyk(bytes).map_err(|e| unreadable(path, &e))?,
        _ if reading.stored => jpeg_cmyk(bytes).map_err(|e| unreadable(path, &e))?,
        _ => None,
    };
    Ok(Decoded { image, orientation, profile, cmyk, gray_floats: false, lab: false, warnings: Vec::new(), said: Vec::new(), rest: Vec::new(), timing: None })
}

/// A CMYK or YCCK JPEG's inks, four bytes to a pixel, or `None` for a JPEG
/// of other color. An Adobe APP14 segment marks inks stored inverted, 255
/// for none, as Adobe's applications write them, and these are given the
/// right way round; YCCK, Adobe's other CMYK encoding, holds the inverted
/// cyan, magenta and yellow as YCbCr, which BT.601 at full range turns back.
fn jpeg_cmyk(bytes: &[u8]) -> Result<Option<Cmyk>, String> {
    use zune_core::bytestream::ZCursor;
    use zune_core::colorspace::ColorSpace;
    use zune_core::options::DecoderOptions;
    let options = DecoderOptions::default().set_strict_mode(false).set_max_width(usize::MAX).set_max_height(usize::MAX);
    let mut headers = zune_jpeg::JpegDecoder::new_with_options(ZCursor::new(bytes), options);
    headers.decode_headers().map_err(|e| e.to_string())?;
    let input = headers.input_colorspace().ok_or("the JPEG's headers name no color space")?;
    if !matches!(input, ColorSpace::CMYK | ColorSpace::YCCK) {
        return Ok(None);
    }
    let mut decoder = zune_jpeg::JpegDecoder::new_with_options(ZCursor::new(bytes), options.jpeg_set_out_colorspace(input));
    let mut samples = decoder.decode().map_err(|e| e.to_string())?;
    if input == ColorSpace::YCCK {
        for pixel in samples.as_chunks_mut::<4>().0 {
            let (y, cb, cr) = (f32::from(pixel[0]), f32::from(pixel[1]) - 128.0, f32::from(pixel[2]) - 128.0);
            let rgb = [y + 1.402 * cr, y - 0.344_136 * cb - 0.714_136 * cr, y + 1.772 * cb];
            for (sample, value) in pixel.iter_mut().zip(rgb) {
                *sample = 255 - value.round().clamp(0.0, 255.0) as u8;
            }
        }
    }
    if adobe_marked(bytes) {
        for sample in &mut samples {
            *sample = 255 - *sample;
        }
    }
    Ok(Some(Cmyk::Eight(samples)))
}

/// Whether a JPEG has an Adobe APP14 segment before its image data.
fn adobe_marked(bytes: &[u8]) -> bool {
    let mut at = 2;
    while let Some(&[0xFF, marker, high, low]) = bytes.get(at..at + 4) {
        if marker == 0xDA || marker == 0xD9 {
            return false;
        }
        if marker == 0xEE && bytes.get(at + 4..at + 9) == Some(&b"Adobe"[..]) {
            return true;
        }
        at += 2 + usize::from(u16::from_be_bytes([high, low]));
    }
    false
}

/// A TIFF's pages that `asked` names, counted among its directories that
/// hold a page: those its NewSubfileType tag marks as neither a smaller
/// copy of another image nor a transparency mask, nor its older
/// SubfileType as a smaller copy, or its first directory when it marks
/// every one so. Each page decodes as the image crate decodes a file's
/// first, with the orientation and ICC profile of its own tags, and pages
/// that share both make one part.
fn tiff(path: &Path, bytes: &[u8], reading: Reading, asked: Option<&[u32]>) -> PsResult<Chosen> {
    use tiff::decoder::Decoder;
    use tiff::tags::Tag;
    let fail = |e: &dyn Display| unreadable(path, e);
    let mut walk = Decoder::new(Cursor::new(bytes)).map_err(|e| fail(&e))?;
    let mut directories: Vec<(u64, bool)> = Vec::new();
    let mut seen = std::collections::HashSet::new();
    let mut next = walk.ifd_pointer();
    while let Some(at) = next {
        if !seen.insert(at.0) {
            return Err(fail(&"its directories form a loop"));
        }
        let number = directories.len() + 1;
        let kind = |e: tiff::TiffError| fail(&format!("the subfile type of its directory {number} cannot be read: {e}"));
        let directory = walk.read_directory(at).map_err(|e| fail(&e))?;
        let mut tags = walk.read_directory_tags(&directory);
        let new_kind = tags.find_tag_unsigned::<u32>(Tag::NewSubfileType).map_err(kind)?;
        let old_kind = tags.find_tag_unsigned::<u16>(Tag::SubfileType).map_err(kind)?;
        directories.push((at.0, is_page(new_kind, old_kind)));
        next = directory.next();
    }
    let total = directories.len();
    let mut pages: Vec<u64> = directories.iter().filter(|&&(_, page)| page).map(|&(at, _)| at).collect();
    let left_out = total - pages.len();
    let mut said = Vec::new();
    if pages.is_empty() {
        pages.extend(directories.first().map(|&(at, _)| at));
        if total > 1 {
            said.push(format!("the file marks each of its {total} directories as a smaller copy of another image or a transparency mask, so its first loaded"));
        }
    } else if left_out == 1 {
        said.push(format!("1 of the file's {total} directories holds a smaller copy of another image or a transparency mask, not a page, so it was left out"));
    } else if left_out > 1 {
        said.push(format!("{left_out} of the file's {total} directories hold a smaller copy of another image or a transparency mask, not a page, so they were left out"));
    }
    let picked = chosen(asked, pages.len(), reading.names.frames).map_err(|e| fail(&e))?;
    let mut parts: Vec<(PageTags, Vec<Frame>)> = Vec::new();
    for (place, &index) in picked.iter().enumerate() {
        let page = match tiff_page(bytes, pages[index], reading) {
            Ok(page) => page,
            Err(e) if pages.len() > 1 => return Err(fail(&format!("its page {} of {} does not decode: {e}", index + 1, pages.len()))),
            Err(e) => return Err(fail(&e)),
        };
        let frame = Frame { image: page.image, cmyk: page.cmyk, place, index };
        match parts.iter_mut().find(|(tags, _)| *tags == page.tags) {
            Some((_, frames)) => frames.push(frame),
            None => parts.push((page.tags, vec![frame])),
        }
    }
    let parts = parts
        .into_iter()
        .map(|(tags, frames)| Part { frames, orientation: tags.orientation, profile: tags.icc.map(|icc| icc.map(Profile::Icc)), gray_floats: false, lab: false })
        .collect();
    Ok(Chosen { parts, count: picked.len(), warnings: Vec::new(), said, timing: None })
}

/// Whether a TIFF directory holds a page by its subfile tags: bit 0 of its
/// NewSubfileType marks a smaller copy of another image and bit 2 a
/// transparency mask, and an older SubfileType of 2 a smaller copy.
fn is_page(new_kind: Option<u32>, old_kind: Option<u16>) -> bool {
    let marked = match new_kind {
        Some(kind) => kind & 0b101 != 0,
        None => false,
    };
    !marked && old_kind != Some(2)
}

/// A TIFF page as decoded, with its CMYK samples where it has them and
/// the tags its pixels are read by.
struct TiffPage {
    image: DynamicImage,
    cmyk: Option<Cmyk>,
    tags: PageTags,
}

/// The orientation and ICC profile a TIFF page's tags give, each as read
/// or the reason it could not be; pages whose tags are the same make one
/// part.
#[derive(PartialEq)]
struct PageTags {
    orientation: Result<Orientation, String>,
    icc: Result<Option<Vec<u8>>, String>,
}

/// The page of the TIFF `bytes` whose directory is at `at`, through the
/// image crate as it decodes a file's first page and within the limits its
/// reader decodes by, with the orientation and ICC profile from the page's
/// tags, and a CMYK page's samples as stored when it carries a profile to
/// convert them by or the Reading keeps the pixels as stored.
fn tiff_page(bytes: &[u8], at: u64, reading: Reading) -> Result<TiffPage, String> {
    use image::codecs::tiff::TiffDecoder;
    use tiff::decoder::{Decoder, DecodingResult};
    use tiff::tags::Tag;
    let mut decoder = TiffDecoder::new(Repointed::new(bytes, at).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
    let mut limits = image::Limits::default();
    limits.reserve(decoder.total_bytes()).map_err(|e| e.to_string())?;
    decoder.set_limits(limits).map_err(|e| e.to_string())?;
    let image = DynamicImage::from_decoder(decoder).map_err(|e| e.to_string())?;
    let mut tags = Decoder::new(Repointed::new(bytes, at).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
    let orientation = match tags.find_tag_unsigned::<u8>(Tag::Orientation) {
        Ok(None) => Ok(Orientation::NoTransforms),
        Ok(Some(value)) => Orientation::from_exif(value).ok_or_else(|| format!("its value, {value}, is not one of 1 to 8")),
        Err(e) => Err(format!("it cannot be read: {e}")),
    };
    let icc = match tags.find_tag(Tag::IccProfile) {
        Ok(None) => Ok(None),
        Ok(Some(_)) => tags.get_tag_u8_vec(Tag::IccProfile).map(Some).map_err(|e| format!("it cannot be read: {e}")),
        Err(e) => Err(format!("it cannot be read: {e}")),
    };
    let inks = match &icc {
        Ok(Some(_)) => !reading.ignore_profile || reading.stored,
        _ => reading.stored,
    };
    let cmyk = if inks {
        match tags.colortype().map_err(|e| e.to_string())? {
            tiff::ColorType::CMYK(8 | 16) => match tags.read_image().map_err(|e| e.to_string())? {
                DecodingResult::U8(samples) => Some(Cmyk::Eight(samples)),
                DecodingResult::U16(samples) => Some(Cmyk::Sixteen(samples)),
                _ => None,
            },
            _ => None,
        }
    } else {
        None
    };
    Ok(TiffPage { image, cmyk, tags: PageTags { orientation, icc } })
}

/// A TIFF's bytes with the header's offset of the first directory replaced,
/// so a decoder that reads a file's first page reads the page whose
/// directory is at that offset. Every other offset in a TIFF counts from
/// the file's start, so the rest of the file reads as it is.
struct Repointed<'a> {
    bytes: &'a [u8],
    header: [u8; 16],
    /// The header's length: 8 bytes, or 16 in a BigTIFF.
    header_len: usize,
    at: u64,
}

impl<'a> Repointed<'a> {
    /// `bytes` with their first directory at `first`, or why they do not
    /// begin as a TIFF does.
    fn new(bytes: &'a [u8], first: u64) -> std::io::Result<Repointed<'a>> {
        use std::io::{Error, ErrorKind};
        let not_tiff = || Error::new(ErrorKind::InvalidData, "it does not begin as a TIFF does");
        let little = match bytes.get(..2) {
            Some(b"II") => true,
            Some(b"MM") => false,
            _ => return Err(not_tiff()),
        };
        let version = match bytes.get(2..4) {
            Some(&[a, b]) if little => u16::from_le_bytes([a, b]),
            Some(&[a, b]) => u16::from_be_bytes([a, b]),
            _ => return Err(not_tiff()),
        };
        let header_len = if version == 43 { 16 } else { 8 };
        let mut header = [0; 16];
        header[..header_len].copy_from_slice(bytes.get(..header_len).ok_or_else(not_tiff)?);
        if version == 43 {
            header[8..16].copy_from_slice(&if little { first.to_le_bytes() } else { first.to_be_bytes() });
        } else {
            let first = u32::try_from(first).map_err(|e| Error::new(ErrorKind::InvalidData, format!("a directory's offset is past the 4 bytes a TIFF's offsets take: {e}")))?;
            header[4..8].copy_from_slice(&if little { first.to_le_bytes() } else { first.to_be_bytes() });
        }
        Ok(Repointed { bytes, header, header_len, at: 0 })
    }
}

impl std::io::BufRead for Repointed<'_> {
    fn fill_buf(&mut self) -> std::io::Result<&[u8]> {
        // A position at or past the end gives no bytes, as a Cursor's does.
        Ok(if self.at < self.header_len as u64 {
            &self.header[self.at as usize..self.header_len]
        } else if self.at < self.bytes.len() as u64 {
            &self.bytes[self.at as usize..]
        } else {
            &[]
        })
    }

    fn consume(&mut self, amount: usize) {
        self.at = self.at.saturating_add(amount as u64);
    }
}

impl std::io::Read for Repointed<'_> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let available = std::io::BufRead::fill_buf(self)?;
        let amount = available.len().min(buf.len());
        buf[..amount].copy_from_slice(&available[..amount]);
        std::io::BufRead::consume(self, amount);
        Ok(amount)
    }
}

impl std::io::Seek for Repointed<'_> {
    fn seek(&mut self, to: std::io::SeekFrom) -> std::io::Result<u64> {
        let to = match to {
            std::io::SeekFrom::Start(at) => Some(at),
            std::io::SeekFrom::End(by) => (self.bytes.len() as u64).checked_add_signed(by),
            std::io::SeekFrom::Current(by) => self.at.checked_add_signed(by),
        };
        match to {
            Some(at) => {
                self.at = at;
                Ok(at)
            }
            None => Err(std::io::Error::new(std::io::ErrorKind::InvalidInput, "a seek before the file's start or past the last position a u64 counts")),
        }
    }
}

/// The profile connection space's illuminant, D50 as ICC.1 gives it: the
/// white every ICC profile's colorants are adapted to.
const ICC_D50: Xyz = Xyz::new(0.9642, 1.0, 0.8249);

/// D65 as sRGB (IEC 61966-2-1), BT.709 and BT.2020 give it.
const D65: XyY = XyY { x: 0.3127, y: 0.3290, yb: 1.0 };

/// `profile` with the colorants of `primaries` under `white`, adapted by
/// Bradford to the ICC's D50, as littleCMS builds its sRGB. moxcms's own
/// sRGB takes its D65 from a 6504 K color temperature, its CICP profiles
/// take D65 at 0.31272, 0.32903, and both adapt to a D50 at 0.34567,
/// 0.35850; their colorants differ from these by up to 0.0002, enough to
/// move a dark channel beside a bright one by a level.
fn with_colorants(mut profile: ColorProfile, primaries: ColorPrimaries, white: XyY) -> ColorProfile {
    let m = adapt_to_illuminant_d(rgb_to_xyz(primaries, white), white, ICC_D50);
    profile.red_colorant = Xyzd::new(m.v[0][0], m.v[1][0], m.v[2][0]);
    profile.green_colorant = Xyzd::new(m.v[0][1], m.v[1][1], m.v[2][1]);
    profile.blue_colorant = Xyzd::new(m.v[0][2], m.v[1][2], m.v[2][2]);
    profile
}

/// The matrix from linear light in `primaries` to XYZ, both under `white`.
fn rgb_to_xyz(primaries: ColorPrimaries, white: XyY) -> Matrix3d {
    let [r, g, b] = [primaries.red, primaries.green, primaries.blue].map(|c| c.to_xyzd());
    ColorProfile::rgb_to_xyz_d(Matrix3d { v: [[r.x, g.x, b.x], [r.y, g.y, b.y], [r.z, g.z, b.z]] }, white.to_xyzd())
}

/// sRGB, and a gray profile with sRGB's curve for gray images, which keep
/// one channel.
fn srgb_for(gray: bool) -> ColorProfile {
    let srgb = with_colorants(ColorProfile::new_srgb(), ColorPrimaries::BT_709, D65);
    if !gray {
        return srgb;
    }
    let mut profile = ColorProfile::new_gray_with_gamma(2.2);
    profile.gray_trc = srgb.green_trc;
    profile
}

/// sRGB's encoding as linear light, from IEC 61966-2-1.
fn srgb_decode(v: f64) -> f64 {
    if v <= 0.04045 { v / 12.92 } else { ((v + 0.055) / 1.055).powf(2.4) }
}

/// The XYZ under D50 of an sRGB color, as sRGB's colorants built as
/// littleCMS builds them give it.
fn srgb_xyz(rgb: [f64; 3]) -> [f64; 3] {
    let linear = rgb.map(srgb_decode);
    srgb_for(false).colorant_matrix().v.map(|row| row[0] * linear[0] + row[1] * linear[1] + row[2] * linear[2])
}

/// A fill color as sRGB: a Lab one by its XYZ, its light in colors sRGB
/// lacks clipped, as the import converts a Lab document to sRGB.
fn srgb_of(color: Foreign) -> [f64; 3] {
    match color {
        Foreign::Srgb(rgb) => rgb,
        Foreign::Lab(lab) => {
            let xyz = profile_table::lab_to_xyz(lab);
            srgb_for(false).colorant_matrix().inverse().v.map(|row| profile_table::encode(row[0] * xyz[0] + row[1] * xyz[1] + row[2] * xyz[2]))
        }
    }
}

/// An sRGB color's Rec. 709 luminance in linear light, as -Grayscale
/// weighs it.
fn rec709_luminance(rgb: [f64; 3]) -> f64 {
    let [r, g, b] = rgb.map(srgb_decode);
    0.2126 * r + 0.7152 * g + 0.0722 * b
}

/// An RGB profile whose red, green and blue are X, Y and Z under D50, with
/// curves of gamma 1 and no CICP values, so a color's XYZ goes through it
/// into a profile's matrix and curves as it is.
fn xyz_profile() -> ColorProfile {
    let mut profile = ColorProfile::new_srgb();
    profile.red_colorant = Xyzd::new(1.0, 0.0, 0.0);
    profile.green_colorant = Xyzd::new(0.0, 1.0, 0.0);
    profile.blue_colorant = Xyzd::new(0.0, 0.0, 1.0);
    for trc in [&mut profile.red_trc, &mut profile.green_trc, &mut profile.blue_trc] {
        *trc = Some(ToneReprCurve::Parametric(vec![1.0]));
    }
    profile.cicp = None;
    profile
}

/// A Photoshop fill color of another mode than its document's as the
/// document's samples the merge holds, each from 0 to 1: a CMYK document's
/// as what each ink leaves, a Lab one's as L over 100 and a and b plus 128
/// over 256, as Photoshop's 15-bit Lab stores them. Into the document's ICC
/// profile it converts relative
/// colorimetrically, as `into_profile` converts. A Lab document takes the
/// color's own Lab. A document without a profile takes it as the import
/// reads such a document, inverted: an RGB one as sRGB, a gray one as
/// sRGB's gray of the color's Rec. 709 luminance in linear light, a CMYK
/// one with black 1 less the largest of red, green and blue and each other
/// ink what the import's formula takes back from that, and a 32-bit one in
/// linear light. Or why it cannot convert.
pub(crate) fn fill_color(color: Foreign, header: &Header, icc: Option<&[u8]>) -> Result<Vec<f32>, String> {
    let float = header.depth == 32;
    let values: Vec<f64> = match (header.mode, icc) {
        (Mode::Lab, _) => {
            let [l, a, b] = match color {
                Foreign::Lab(lab) => lab,
                Foreign::Srgb(rgb) => profile_table::xyz_to_lab(srgb_xyz(rgb)),
            };
            vec![l / 100.0, (a + 128.0) / 256.0, (b + 128.0) / 256.0]
        }
        (Mode::Rgb | Mode::Gray | Mode::Duotone | Mode::Cmyk, Some(icc)) => into_profile(color, header.mode, icc)?,
        (Mode::Rgb, None) => {
            let rgb = srgb_of(color);
            if float { rgb.map(srgb_decode).to_vec() } else { rgb.to_vec() }
        }
        (Mode::Gray | Mode::Duotone, None) => {
            let y = rec709_luminance(srgb_of(color));
            vec![if float { y } else { profile_table::encode(y) }]
        }
        (Mode::Cmyk, None) => {
            let [r, g, b] = srgb_of(color);
            let largest = r.max(g).max(b);
            // What each ink leaves: each of red, green and blue over the
            // largest, and the largest for black.
            if largest > 0.0 { vec![r / largest, g / largest, b / largest, largest] } else { vec![1.0, 1.0, 1.0, 0.0] }
        }
        (other, _) => return Err(format!("the merge converts fill colors into RGB, gray, CMYK and Lab documents, not {} ones", other.name())),
    };
    Ok(values.iter().map(|&v| v.clamp(0.0, 1.0) as f32).collect())
}

/// Each value of `from` through `f`, into `to`.
fn each(f: impl Fn(usize, f64) -> f64 + 'static) -> Mapping {
    Box::new(move |from, to| {
        for (c, (out, &v)) in to.iter_mut().zip(from).enumerate() {
            *out = f(c, v);
        }
    })
}

/// A document's samples as linear light and back, for a gradient of the
/// linear method: a 32-bit document's as they are, light already; an RGB
/// or gray one's through its ICC profile's curves, sRGB's curve where it
/// has no profile, or, for a profile without curves, as XYZ under D50
/// through the module's own reading of its tables, relative
/// colorimetrically, read and written as littleCMS picks them. Or why the
/// merge knows no light for its samples: another mode, or a profile of
/// another color or with no table for writing colors.
pub(crate) fn document_light(header: &Header, icc: Option<&[u8]>) -> Result<Light, String> {
    let (channels, space) = match header.mode {
        Mode::Rgb => (3, DataColorSpace::Rgb),
        Mode::Gray => (1, DataColorSpace::Gray),
        mode => return Err(format!("a {} document's samples are not light the merge mixes in", mode.name())),
    };
    if header.depth == 32 {
        return Ok(Light { width: channels, into: each(|_, v| v), back: each(|_, v| v) });
    }
    let Some(icc) = icc else {
        return Ok(Light { width: channels, into: each(|_, v| srgb_decode(v)), back: each(|_, v| profile_table::encode(v)) });
    };
    let profile = ColorProfile::new_from_slice(icc).map_err(|e| format!("its color profile cannot be read: {e}"))?;
    if profile.color_space != space {
        return Err(format!("its color profile is of {:?} color, where the document is {}", profile.color_space, header.mode.name()));
    }
    if profile_table::is_matrix_shaper(&profile) {
        let curves = std::rc::Rc::new(profile_table::ToneCurves::of(&profile)?);
        let back = std::rc::Rc::clone(&curves);
        return Ok(Light { width: channels, into: each(move |c, v| curves.light(c, v)), back: each(move |c, v| back.sample(c, v)) });
    }
    let to_srgb = srgb_for(false).colorant_matrix().inverse().v;
    let reading = TableConversion::evaluated(&profile, icc, RenderingIntent::RelativeColorimetric, to_srgb)?.ok_or("its color profile has neither curves nor a table to read colors by")?;
    if reading.inputs != channels {
        return Err(format!("its color profile's table reads {} channels, where the document has {channels}", reading.inputs));
    }
    let writing = profile_table::Writing::relative(&profile, icc)?.ok_or("its color profile has no table for writing colors, only one for reading them")?;
    let written = writing.device([0.0; 3]).len();
    if written != channels {
        return Err(format!("its color profile's table writes {written} channels, where the document has {channels}"));
    }
    Ok(Light {
        width: 3,
        into: Box::new(move |from, to| to.copy_from_slice(&reading.xyz_of(from))),
        back: Box::new(move |from, to| to.copy_from_slice(&writing.device([from[0], from[1], from[2]]))),
    })
}

/// The module's color handling as the PSD merge takes it.
pub(crate) const COLORS: Colors = Colors { convert: fill_color, light: document_light };

/// moxcms's options for a conversion with `intent`, through its
/// floating-point matrix-shaper paths on every platform: its fixed-point
/// ones on aarch64 hold the matrix in Q1.30, which cannot hold a
/// coefficient of 2 or more, as the matrices from XYZ and from ProPhoto
/// into sRGB have.
fn transform_options(intent: moxcms::RenderingIntent) -> TransformOptions {
    TransformOptions { rendering_intent: intent, prefer_fixed_point: false, ..TransformOptions::default() }
}

/// A fill color converted relative colorimetrically into the document's
/// ICC profile `icc`, by the table the profile writes colors by, read as
/// littleCMS reads it, or else by a matrix-shaper's matrix and curves
/// through moxcms, an sRGB color from sRGB as littleCMS builds it and a Lab
/// one by its XYZ; a CMYK document's values as what each ink leaves. A
/// profile with neither, which only reads colors, gives why.
fn into_profile(color: Foreign, mode: Mode, icc: &[u8]) -> Result<Vec<f64>, String> {
    let target = ColorProfile::new_from_slice(icc).map_err(|e| format!("its color profile cannot be read: {e}"))?;
    let (space, layout, channels) = match mode {
        Mode::Rgb => (DataColorSpace::Rgb, Layout::Rgb, 3),
        Mode::Gray | Mode::Duotone => (DataColorSpace::Gray, Layout::Gray, 1),
        _ => (DataColorSpace::Cmyk, Layout::Rgba, 4),
    };
    if target.color_space != space {
        return Err(format!("its color profile is of {:?} color, where the document is {}", target.color_space, mode.name()));
    }
    let xyz = match color {
        Foreign::Srgb(rgb) => srgb_xyz(rgb),
        Foreign::Lab(lab) => profile_table::lab_to_xyz(lab),
    };
    let device = if let Some(table) = profile_table::Writing::relative(&target, icc)? {
        table.device(xyz)
    } else if profile_table::is_matrix_shaper(&target) {
        let (source, input) = match color {
            Foreign::Srgb(rgb) => (srgb_for(false), rgb),
            Foreign::Lab(_) => (xyz_profile(), xyz),
        };
        let options = transform_options(moxcms::RenderingIntent::RelativeColorimetric);
        let failed = |e: moxcms::CmsError| format!("moxcms cannot convert into its color profile: {e}");
        let mut out = vec![0.0; channels];
        source.create_transform_f64(Layout::Rgb, &target, layout, options).map_err(failed)?.transform(&input, &mut out).map_err(failed)?;
        out
    } else {
        return Err("its color profile has no table for writing colors, only one for reading them".to_string());
    };
    Ok(if mode == Mode::Cmyk { device.iter().map(|ink| 1.0 - ink).collect() } else { device })
}

/// An image converted from its color profile: the pixels, the profile's
/// name, the table conversion that made them where a table did, and
/// whether they are linear light in sRGB's primaries, as a float image's
/// are, rather than in sRGB's encoding.
struct Converted {
    image: DynamicImage,
    name: String,
    table: Option<TableConversion>,
    linear: bool,
}

/// A file's color profile as moxcms reads it, with its name, the one it
/// gives itself or one the module gives it, and the ICC bytes it was read
/// from, none for CICP values.
struct Source<'a> {
    profile: ColorProfile,
    name: String,
    icc: &'a [u8],
}

/// `profile` as moxcms reads it; `None` for CIE Lab, which has no profile
/// to read.
fn source_of(profile: &Profile) -> Result<Option<Source<'_>>, String> {
    let (source, icc): (ColorProfile, &[u8]) = match profile {
        Profile::Icc(icc) => (ColorProfile::new_from_slice(icc).map_err(|e| format!("it cannot be read: {e}"))?, icc),
        #[cfg(any(feature = "avif", feature = "jxl"))]
        Profile::Cicp(cicp) => (cicp::cicp_profile(*cicp), &[]),
        Profile::Lab => return Ok(None),
    };
    let name = match (profile, described(&source)) {
        (_, Some(name)) => name,
        #[cfg(any(feature = "avif", feature = "jxl"))]
        (Profile::Cicp(c), None) => format!("CICP primaries {:?} and transfer {:?}", c.color_primaries, c.transfer_characteristics),
        (_, None) => "an unnamed profile".to_string(),
    };
    Ok(Some(Source { profile: source, name, icc }))
}

/// Why `source` cannot convert an image that is gray, or in color, as
/// `gray` says; `Ok` where it can.
fn fits(source: &ColorProfile, gray: bool) -> Result<(), String> {
    match source.color_space {
        DataColorSpace::Rgb if gray => Err("it is an RGB profile and the image is gray".to_string()),
        DataColorSpace::Gray if !gray => Err("it is a gray profile and the image is in color".to_string()),
        DataColorSpace::Rgb | DataColorSpace::Gray | DataColorSpace::Cmyk => Ok(()),
        other => Err(format!("it is a profile of {other:?} color, which the module converts from only for RGB, gray and CMYK")),
    }
}

/// `image`, or the CMYK samples `cmyk` holds of it, converted from
/// `profile` to sRGB with `intent` as littleCMS converts: through the
/// profile's table for the intent where it has one, and otherwise by its
/// matrix and curves, through moxcms, or through the module's own steps
/// where moxcms's conversion would differ from littleCMS's; a float image
/// to linear light in sRGB's primaries, as `float_converted` gives it; or
/// why it could not be.
fn convert(image: &DynamicImage, cmyk: Option<&Cmyk>, profile: &Profile, intent: RenderingIntent) -> Result<Converted, String> {
    if matches!(image, DynamicImage::ImageRgb32F(_) | DynamicImage::ImageRgba32F(_)) {
        return float_converted(image, false, profile, intent);
    }
    let to_srgb = srgb_for(false).colorant_matrix().inverse().v;
    let Some(Source { profile: source, name, icc }) = source_of(profile)? else {
        let table = TableConversion::lab(to_srgb);
        let image = table_converted(&table, image, cmyk)?;
        return Ok(Converted { image, name: "CIE Lab".to_string(), table: Some(table), linear: false });
    };
    let options = transform_options(intent.moxcms());
    let failed = |e: moxcms::CmsError| format!("moxcms cannot convert it: {e}");
    let (w, h) = (image.width(), image.height());
    let gray = matches!(image, DynamicImage::ImageLuma8(_) | DynamicImage::ImageLumaA8(_) | DynamicImage::ImageLuma16(_) | DynamicImage::ImageLumaA16(_));
    fits(&source, gray)?;
    if let Some(table) = TableConversion::new(&source, icc, intent, to_srgb)? {
        let image = table_converted(&table, image, cmyk)?;
        return Ok(Converted { image, name, table: Some(table), linear: false });
    }
    if source.color_space == DataColorSpace::Cmyk {
        return Err("it is a CMYK profile without a table to convert by".to_string());
    }
    let target = srgb_for(gray);
    let filled = |what: &str| format!("the converted {what} do not fill {w}x{h} pixels");
    macro_rules! eight {
        ($buffer:expr, $layout:expr, $variant:path) => {{
            let src = widened($buffer.as_raw());
            let mut out = vec![0u16; src.len()];
            source.create_transform_16bit($layout, &target, $layout, options).map_err(failed)?.transform(&src, &mut out).map_err(failed)?;
            $variant(ImageBuffer::from_raw(w, h, narrowed(&out)).ok_or_else(|| filled("pixels"))?)
        }};
    }
    macro_rules! sixteen {
        ($buffer:expr, $layout:expr, $variant:path) => {{
            let src = $buffer.as_raw();
            let mut out = vec![0u16; src.len()];
            source.create_transform_16bit($layout, &target, $layout, options).map_err(failed)?.transform(src, &mut out).map_err(failed)?;
            $variant(ImageBuffer::from_raw(w, h, out).ok_or_else(|| filled("pixels"))?)
        }};
    }
    let converted = match image {
        DynamicImage::ImageRgb8(b) => eight!(b, Layout::Rgb, DynamicImage::ImageRgb8),
        DynamicImage::ImageRgba8(b) => eight!(b, Layout::Rgba, DynamicImage::ImageRgba8),
        DynamicImage::ImageLuma8(b) => eight!(b, Layout::Gray, DynamicImage::ImageLuma8),
        DynamicImage::ImageLumaA8(b) => eight!(b, Layout::GrayAlpha, DynamicImage::ImageLumaA8),
        DynamicImage::ImageRgb16(b) => sixteen!(b, Layout::Rgb, DynamicImage::ImageRgb16),
        DynamicImage::ImageRgba16(b) => sixteen!(b, Layout::Rgba, DynamicImage::ImageRgba16),
        DynamicImage::ImageLuma16(b) => sixteen!(b, Layout::Gray, DynamicImage::ImageLuma16),
        DynamicImage::ImageLumaA16(b) => sixteen!(b, Layout::GrayAlpha, DynamicImage::ImageLumaA16),
        other => return Err(format!("the module converts no {:?} pixels", other.color())),
    };
    Ok(Converted { image: converted, name, table: None, linear: false })
}

/// A float image converted from `profile` to linear light in sRGB's
/// primaries with `intent`, as littleCMS's float transforms convert,
/// through the module's own steps for the profile's table or its matrix
/// and curves: light below 0, which colors sRGB lacks give, clipped to 0,
/// light above 1 kept, and alpha kept as it is. With `gray`, the image's
/// RGB holds a gray image's samples, each pixel's three the same, which a
/// gray profile converts from the first. The pixels convert on every core.
fn float_converted(image: &DynamicImage, gray: bool, profile: &Profile, intent: RenderingIntent) -> Result<Converted, String> {
    use DynamicImage as D;
    let to_srgb = srgb_for(false).colorant_matrix().inverse().v;
    let (table, name) = match source_of(profile)? {
        Some(Source { profile: source, name, icc }) => {
            fits(&source, gray)?;
            match TableConversion::evaluated(&source, icc, intent, to_srgb)? {
                Some(table) => (table, name),
                None => return Err("it holds neither a table nor a matrix and curves to convert by".to_string()),
            }
        }
        None => (TableConversion::lab(to_srgb), "CIE Lab".to_string()),
    };
    let colors = if gray { 1 } else { 3 };
    if table.inputs != colors {
        return Err(format!("the profile converts {} channels {}, where the image's color has {colors}", table.inputs, table.by()));
    }
    let light = |p: &[f32]| table.linear(&[f64::from(p[0]), f64::from(p[1]), f64::from(p[2])][..colors]).map(|v| v as f32);
    let (w, h) = (image.width(), image.height());
    let image = match image {
        D::ImageRgb32F(b) => D::ImageRgb32F(filled(w, h, in_parallel(b.as_raw(), 3, 3, |p, o| o.copy_from_slice(&light(p))))?),
        D::ImageRgba32F(b) => D::ImageRgba32F(filled(w, h, in_parallel(b.as_raw(), 4, 4, |p, o| o.copy_from_slice(&with_alpha(light(&p[..3]), p[3]))))?),
        other => return Err(format!("the module converts no {:?} pixels as floats", other.color())),
    };
    Ok(Converted { image, name, table: Some(table), linear: true })
}

/// `samples` as a `w` by `h` image of the pixel type the caller names.
fn filled<P: image::Pixel>(w: u32, h: u32, samples: Vec<P::Subpixel>) -> Result<ImageBuffer<P, Vec<P::Subpixel>>, String> {
    ImageBuffer::from_raw(w, h, samples).ok_or_else(|| format!("the converted samples do not fill {w}x{h} pixels"))
}

/// `image`, or the CMYK samples `cmyk` holds of it, through `table`: 8-bit
/// and 16-bit samples written as littleCMS writes them, and alpha kept as
/// it is. Samples convert by the conversion worked out for every sample
/// value where it can be, and the pixels convert on every core.
fn table_converted(table: &TableConversion, image: &DynamicImage, cmyk: Option<&Cmyk>) -> Result<DynamicImage, String> {
    use profile_table::{to_8, to_16};
    use DynamicImage as D;
    let colors = match (cmyk, image) {
        (Some(_), _) => 4,
        (None, D::ImageLuma8(_) | D::ImageLumaA8(_) | D::ImageLuma16(_) | D::ImageLumaA16(_)) => 1,
        (None, _) => 3,
    };
    if table.inputs != colors {
        return Err(format!("the profile converts {} channels {}, where the image's color has {colors}", table.inputs, table.by()));
    }
    let (w, h) = (image.width(), image.height());
    let eight = |v: f64| to_8(to_16(v));
    Ok(match (cmyk, image) {
        (Some(Cmyk::Eight(inks)), _) => {
            let t = table.tabled::<u8>();
            let rgb = in_parallel(inks, 4, 3, |p, o| o.copy_from_slice(&t.rgb(p).map(eight)));
            match image {
                D::ImageRgba8(b) => D::ImageRgba8(filled(w, h, alpha_joined(&rgb, b.as_raw()))?),
                _ => D::ImageRgb8(filled(w, h, rgb)?),
            }
        }
        (Some(Cmyk::Sixteen(inks)), _) => {
            let t = table.tabled::<u16>();
            let rgb = in_parallel(inks, 4, 3, |p, o| o.copy_from_slice(&t.rgb(p).map(to_16)));
            match image {
                D::ImageRgba16(b) => D::ImageRgba16(filled(w, h, alpha_joined(&rgb, b.as_raw()))?),
                _ => D::ImageRgb16(filled(w, h, rgb)?),
            }
        }
        (None, D::ImageRgb8(b)) => {
            let t = table.tabled::<u8>();
            D::ImageRgb8(filled(w, h, in_parallel(b.as_raw(), 3, 3, |p, o| o.copy_from_slice(&t.rgb(p).map(eight))))?)
        }
        (None, D::ImageRgba8(b)) => {
            let t = table.tabled::<u8>();
            D::ImageRgba8(filled(w, h, in_parallel(b.as_raw(), 4, 4, |p, o| o.copy_from_slice(&with_alpha(t.rgb(&p[..3]).map(eight), p[3]))))?)
        }
        (None, D::ImageLuma8(b)) => {
            let t = table.tabled::<u8>();
            D::ImageLuma8(filled(w, h, in_parallel(b.as_raw(), 1, 1, |p, o| o[0] = eight(t.gray(p[0]))))?)
        }
        (None, D::ImageLumaA8(b)) => {
            let t = table.tabled::<u8>();
            D::ImageLumaA8(filled(w, h, in_parallel(b.as_raw(), 2, 2, |p, o| o.copy_from_slice(&[eight(t.gray(p[0])), p[1]])))?)
        }
        (None, D::ImageRgb16(b)) => {
            let t = table.tabled::<u16>();
            D::ImageRgb16(filled(w, h, in_parallel(b.as_raw(), 3, 3, |p, o| o.copy_from_slice(&t.rgb(p).map(to_16))))?)
        }
        (None, D::ImageRgba16(b)) => {
            let t = table.tabled::<u16>();
            D::ImageRgba16(filled(w, h, in_parallel(b.as_raw(), 4, 4, |p, o| o.copy_from_slice(&with_alpha(t.rgb(&p[..3]).map(to_16), p[3]))))?)
        }
        (None, D::ImageLuma16(b)) => {
            let t = table.tabled::<u16>();
            D::ImageLuma16(filled(w, h, in_parallel(b.as_raw(), 1, 1, |p, o| o[0] = to_16(t.gray(p[0]))))?)
        }
        (None, D::ImageLumaA16(b)) => {
            let t = table.tabled::<u16>();
            D::ImageLumaA16(filled(w, h, in_parallel(b.as_raw(), 2, 2, |p, o| o.copy_from_slice(&[to_16(t.gray(p[0])), p[1]])))?)
        }
        (None, other) => return Err(format!("the module converts no {:?} pixels", other.color())),
    })
}

/// `samples`, `take` of them to a pixel, made `give` to a pixel by
/// `convert`, the pixels shared evenly among the machine's cores, or all
/// on one where the system cannot say how many it has.
fn in_parallel<T: Sync, U: Copy + Default + Send>(samples: &[T], take: usize, give: usize, convert: impl Fn(&[T], &mut [U]) + Sync) -> Vec<U> {
    let pixels = samples.len() / take;
    let mut out = vec![U::default(); pixels * give];
    let cores = std::thread::available_parallelism().map_or(1, |n| n.get());
    let share = pixels.div_ceil(cores).max(1);
    std::thread::scope(|scope| {
        for (from, to) in samples.chunks(share * take).zip(out.chunks_mut(share * give)) {
            let convert = &convert;
            scope.spawn(move || {
                for (pixel, converted) in from.chunks_exact(take).zip(to.chunks_exact_mut(give)) {
                    convert(pixel, converted);
                }
            });
        }
    });
    out
}

/// Three color samples a pixel from `rgb`, each pixel followed by its
/// alpha from `rgba`, four samples a pixel.
fn alpha_joined<T: Copy>(rgb: &[T], rgba: &[T]) -> Vec<T> {
    let mut out = Vec::with_capacity(rgb.len() / 3 * 4);
    for (color, pixel) in rgb.as_chunks::<3>().0.iter().zip(rgba.as_chunks::<4>().0) {
        out.extend_from_slice(color);
        out.push(pixel[3]);
    }
    out
}

/// A color's three samples and its alpha.
fn with_alpha<T>([r, g, b]: [T; 3], a: T) -> [T; 4] {
    [r, g, b, a]
}

/// 8-bit samples as 16-bit ones of the same value, 255 to 65535, for
/// moxcms's 16-bit transform: its tables hold 65536 entries where its 8-bit
/// transform's gamma table holds 4096, a step of 0.8 levels near black.
fn widened(samples: &[u8]) -> Vec<u16> {
    samples.iter().map(|&v| u16::from(v) * 257).collect()
}

/// 16-bit samples rounded to the nearest 8-bit value.
fn narrowed(samples: &[u16]) -> Vec<u8> {
    samples.iter().map(|&v| ((u32::from(v) + 128) / 257) as u8).collect()
}

/// A profile's description in English, or its first, where it has one.
fn described(profile: &ColorProfile) -> Option<String> {
    let text = match profile.description.as_ref()? {
        ProfileText::PlainString(text) => text.clone(),
        ProfileText::Localizable(strings) => strings.iter().find(|s| s.language == "en").or_else(|| strings.first())?.value.clone(),
        ProfileText::Description(description) => description.ascii_string.clone(),
    };
    let text = text.trim().trim_end_matches('\0').trim().to_string();
    if text.is_empty() { None } else { Some(text) }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A build without the avif feature refuses an .avif file, told by its
    /// content, with an error that names the feature.
    #[cfg(not(feature = "avif"))]
    #[test]
    fn refuses_an_avif_file_without_the_avif_feature() {
        let rgba = image::RgbaImage::from_fn(16, 8, |x, y| image::Rgba([(x * 15) as u8, (y * 30) as u8, 120, 255]));
        let mut written = Vec::new();
        DynamicImage::ImageRgba8(rgba).write_with_encoder(image::codecs::avif::AvifEncoder::new_with_speed_quality(&mut written, 10, 100)).expect("encoded");
        let Err(e) = decode(Path::new("photo.avif"), &written, Reading::default(), None) else {
            panic!("an .avif file decoded in a build without the avif feature");
        };
        assert_eq!(
            (e.error_id.as_str(), e.message.as_str()),
            ("WgpuImage", "photo.avif cannot be read as an image: it is an .avif file, which wgpu-pwsh reads only with its avif feature")
        );
    }

    /// A JPEG XL file is refused in a build without the jxl feature, told by
    /// its content, a bare codestream's signature, with an error that names
    /// the feature.
    #[cfg(not(feature = "jxl"))]
    #[test]
    fn refuses_a_jpeg_xl_file_without_the_jxl_feature() {
        match decode(Path::new("photo.jxl"), &[0xFF, 0x0A, 0, 0], Reading::default(), None) {
            Ok(_) => panic!("a JPEG XL file decoded in a build without the jxl feature"),
            Err(e) => assert_eq!(
                (e.error_id.as_str(), e.message.as_str()),
                ("WgpuImage", "photo.jxl cannot be read as an image: it is a JPEG XL file, which wgpu-pwsh reads only with its jxl feature")
            ),
        }
    }

    /// Without the svg feature an SVG file is refused, with an error that
    /// names the feature.
    #[cfg(not(feature = "svg"))]
    #[test]
    fn refuses_an_svg_file_without_the_svg_feature() {
        let asked = Asked { reading: Reading::default(), transform: &Transform::default(), password: None, frames: None };
        match drawn(Path::new("drawing.svg"), b"<svg/>", Vector::Svg, asked, &mut Said::default()) {
            Ok(_) => panic!("an SVG file drew in a build without the svg feature"),
            Err(e) => assert_eq!(
                (e.error_id.as_str(), e.message.as_str()),
                ("WgpuImage", "drawing.svg cannot be read as an image: it is an SVG file, which wgpu-pwsh reads only with its svg feature")
            ),
        }
    }

    /// Without the pdf feature a PDF file is refused, with an error that
    /// names the feature.
    #[cfg(not(feature = "pdf"))]
    #[test]
    fn refuses_a_pdf_file_without_the_pdf_feature() {
        let asked = Asked { reading: Reading::default(), transform: &Transform::default(), password: None, frames: None };
        match drawn(Path::new("page.pdf"), b"%PDF-1.7\n", Vector::Pdf, asked, &mut Said::default()) {
            Ok(_) => panic!("a PDF file drew in a build without the pdf feature"),
            Err(e) => assert_eq!((e.error_id.as_str(), e.message.as_str()), ("WgpuImage", "page.pdf cannot be read as an image: it is a PDF file, which wgpu-pwsh reads only with its pdf feature")),
        }
    }

    /// SVG and PDF files are told by their content, and by their extension
    /// where no format's content tells.
    #[test]
    fn tells_vector_files_by_content_then_extension() {
        let mut png = Vec::new();
        DynamicImage::ImageRgba8(image::RgbaImage::new(1, 1)).write_to(&mut Cursor::new(&mut png), ImageFormat::Png).expect("encoded");
        let cases: [(&str, &[u8], Option<Vector>); 8] = [
            ("a.pdf", b"%PDF-1.7\n", Some(Vector::Pdf)),
            ("a.bin", b"%PDF-1.4\n", Some(Vector::Pdf)),
            ("a.bin", b"\xEF\xBB\xBF  \n<svg/>", Some(Vector::Svg)),
            ("a.bin", b"<?xml version=\"1.0\"?><svg/>", Some(Vector::Svg)),
            ("a.bin", &[0x1F, 0x8B, 8, 0], Some(Vector::Svg)),
            ("a.svg", &png, None),
            ("a.svgz", b"\0\0", Some(Vector::Svg)),
            ("a.PDF", b"junk", Some(Vector::Pdf)),
        ];
        for (name, bytes, want) in cases {
            assert_eq!(vector(Path::new(name), bytes), want, "{name}");
        }
    }

    #[test]
    fn an_adobe_segment_is_found_before_the_image_data() {
        let mut jpeg = vec![0xFF, 0xD8, 0xFF, 0xE0, 0, 4, 0, 0, 0xFF, 0xEE, 0, 14];
        jpeg.extend_from_slice(b"Adobe\0\0\0\0\0\0\0");
        jpeg.extend_from_slice(&[0xFF, 0xDA, 0, 2]);
        assert!(adobe_marked(&jpeg));
        assert!(!adobe_marked(&[0xFF, 0xD8, 0xFF, 0xDA, 0, 2, 0xFF, 0xEE, 0, 14]));
    }

    use crate::profile_reference as reference;

    /// The most an 8-bit sample of the module's conversion may be from
    /// littleCMS's for the same profile and intent.
    const TOLERANCE: u8 = 1;

    /// The color test image tools/profile_reference.py converts, by the same
    /// integer formula.
    fn rgb_test_image() -> DynamicImage {
        DynamicImage::ImageRgb8(ImageBuffer::from_fn(reference::WIDTH, reference::HEIGHT, |x, y| {
            image::Rgb([((x * 37 + y * 91) % 256) as u8, ((x * x * 3 + y * 5) % 256) as u8, ((x * y * 7 + 13) % 256) as u8])
        }))
    }

    /// The gray test image tools/profile_reference.py converts.
    fn gray_test_image() -> DynamicImage {
        DynamicImage::ImageLuma8(ImageBuffer::from_fn(reference::WIDTH, reference::HEIGHT, |x, y| image::Luma([((x * 17 + y * 29) % 256) as u8])))
    }

    fn largest_difference(a: &[u8], b: &[u8]) -> u8 {
        assert_eq!(a.len(), b.len(), "the samples differ in count");
        a.iter().zip(b).map(|(x, y)| x.abs_diff(*y)).max().expect("samples")
    }

    fn converted(image: &DynamicImage, cmyk: Option<&Cmyk>, profile: &[u8], intent: RenderingIntent) -> DynamicImage {
        convert(image, cmyk, &Profile::Icc(profile.to_vec()), intent).expect("converted").image
    }

    /// The four intents, in the order tools/profile_reference.py lists them.
    const INTENTS: [RenderingIntent; 4] = [RenderingIntent::Perceptual, RenderingIntent::RelativeColorimetric, RenderingIntent::Saturation, RenderingIntent::AbsoluteColorimetric];

    /// The module's conversion of `image`, or of the inks `cmyk` holds, from
    /// `profile` with each intent, against littleCMS's in `references`: no
    /// sample more than TOLERANCE levels apart. The conversions, in the
    /// intents' order.
    fn matches_littlecms(name: &str, image: &DynamicImage, cmyk: Option<&Cmyk>, profile: &[u8], references: [&[u8]; 4]) -> Vec<DynamicImage> {
        let mut conversions = Vec::new();
        for (intent, expected) in INTENTS.into_iter().zip(references) {
            let got = converted(image, cmyk, profile, intent);
            let largest = largest_difference(got.as_bytes(), expected);
            assert!(largest <= TOLERANCE, "{name}, {intent:?}: a sample is {largest} levels from littleCMS's");
            conversions.push(got);
        }
        conversions
    }

    /// A profile the tests convert from: its name, its bytes, and
    /// littleCMS's conversions of the test image with each intent.
    type Case<'a> = (&'a str, &'a [u8], [&'a [u8]; 4]);

    #[test]
    fn converts_lab_as_littlecms_converts_from_its_own_lab_profile() {
        let image = rgb_test_image();
        for intent in [RenderingIntent::Perceptual, RenderingIntent::RelativeColorimetric, RenderingIntent::Saturation, RenderingIntent::AbsoluteColorimetric] {
            let converted = convert(&image, None, &Profile::Lab, intent).expect("converted").image;
            let largest = largest_difference(converted.as_bytes(), &reference::LAB);
            assert!(largest <= TOLERANCE, "{intent:?}: {largest} levels from littleCMS");
        }
    }

    #[test]
    fn converts_matrix_profiles_to_srgb_as_littlecms_does() {
        use reference as r;
        let cases: [Case; 5] = [
            ("Display P3", &r::DISPLAYP3_PROFILE, [&r::DISPLAYP3_PERCEPTUAL, &r::DISPLAYP3_RELATIVE, &r::DISPLAYP3_SATURATION, &r::DISPLAYP3_ABSOLUTE]),
            ("Display P3 version 2", &r::DISPLAYP3_V2_PROFILE, [&r::DISPLAYP3_V2_PERCEPTUAL, &r::DISPLAYP3_V2_RELATIVE, &r::DISPLAYP3_V2_SATURATION, &r::DISPLAYP3_V2_ABSOLUTE]),
            ("Adobe RGB", &r::ADOBE_PROFILE, [&r::ADOBE_PERCEPTUAL, &r::ADOBE_RELATIVE, &r::ADOBE_SATURATION, &r::ADOBE_ABSOLUTE]),
            ("ProPhoto", &r::PROPHOTO_PROFILE, [&r::PROPHOTO_PERCEPTUAL, &r::PROPHOTO_RELATIVE, &r::PROPHOTO_SATURATION, &r::PROPHOTO_ABSOLUTE]),
            ("Rec. 2020", &r::REC2020_PROFILE, [&r::REC2020_PERCEPTUAL, &r::REC2020_RELATIVE, &r::REC2020_SATURATION, &r::REC2020_ABSOLUTE]),
        ];
        let image = rgb_test_image();
        for (name, profile, references) in cases {
            matches_littlecms(name, &image, None, profile, references);
            for (intent, expected) in INTENTS.into_iter().zip(references) {
                let unconverted = largest_difference(image.as_bytes(), expected);
                assert!(unconverted > TOLERANCE, "{name}, {intent:?}: the colors as stored are within {unconverted} levels of littleCMS's, which tells the conversion nothing");
            }
        }
    }

    #[test]
    fn an_srgb_profile_leaves_the_bytes_as_they_are() {
        use reference as r;
        let image = rgb_test_image();
        for (intent, expected) in INTENTS.into_iter().zip([&r::SRGB_PERCEPTUAL, &r::SRGB_RELATIVE, &r::SRGB_SATURATION, &r::SRGB_ABSOLUTE]) {
            assert_eq!(&expected[..], image.as_bytes(), "{intent:?}: littleCMS keeps an sRGB image's bytes");
            assert_eq!(converted(&image, None, &r::SRGB_PROFILE, intent).as_bytes(), image.as_bytes(), "{intent:?}");
        }
    }

    #[test]
    fn converts_table_profiles_as_littlecms_does() {
        use reference as r;
        let rgb = rgb_test_image();
        matches_littlecms("lutAtoB RGB", &rgb, None, &r::MAB_RGB_PROFILE, [&r::MAB_RGB_PERCEPTUAL, &r::MAB_RGB_RELATIVE, &r::MAB_RGB_SATURATION, &r::MAB_RGB_ABSOLUTE]);
        matches_littlecms("lut8 RGB", &rgb, None, &r::LUT8_RGB_PROFILE, [&r::LUT8_RGB_PERCEPTUAL, &r::LUT8_RGB_RELATIVE, &r::LUT8_RGB_SATURATION, &r::LUT8_RGB_ABSOLUTE]);
        let gray = matches_littlecms("lutAtoB gray", &gray_test_image(), None, &r::MAB_GRAY_PROFILE, [&r::MAB_GRAY_PERCEPTUAL, &r::MAB_GRAY_RELATIVE, &r::MAB_GRAY_SATURATION, &r::MAB_GRAY_ABSOLUTE]);
        assert!(gray.iter().all(|g| matches!(g, DynamicImage::ImageLuma8(_))), "a gray image keeps one channel");
    }

    /// CICP values with `primaries` and sRGB's transfer, as an AVIF's nclx
    /// box gives them.
    #[cfg(feature = "avif")]
    fn cicp(primaries: moxcms::CicpColorPrimaries) -> Profile {
        use moxcms::{CicpProfile, MatrixCoefficients, TransferCharacteristics};
        Profile::Cicp(CicpProfile { color_primaries: primaries, transfer_characteristics: TransferCharacteristics::Srgb, matrix_coefficients: MatrixCoefficients::Bt709, full_range: true })
    }

    #[cfg(feature = "avif")]
    #[test]
    fn cicp_values_that_say_srgb_leave_the_bytes_as_they_are() {
        let image = rgb_test_image();
        for intent in INTENTS {
            let got = convert(&image, None, &cicp(moxcms::CicpColorPrimaries::Bt709), intent).expect("converted").image;
            assert_eq!(got.as_bytes(), image.as_bytes(), "{intent:?}");
        }
    }

    #[cfg(feature = "avif")]
    #[test]
    fn converts_display_p3_cicp_values_as_littlecms_converts_the_display_p3_profile() {
        let image = rgb_test_image();
        let got = convert(&image, None, &cicp(moxcms::CicpColorPrimaries::Smpte432), RenderingIntent::Perceptual).expect("converted").image;
        let largest = largest_difference(got.as_bytes(), &reference::DISPLAYP3_PERCEPTUAL);
        assert!(largest <= TOLERANCE, "a sample is {largest} levels from littleCMS's");
    }

    /// A JPEG XL file in Display P3 renders in the ICC profile jxl-oxide
    /// writes for it, which converts as littleCMS converts the Display P3
    /// profile.
    #[cfg(feature = "jxl")]
    #[test]
    fn converts_a_display_p3_jpeg_xl_file_as_littlecms_converts_the_display_p3_profile() {
        let decoded = jxl::decode(Path::new("test.jxl"), &crate::jxl_reference::DISPLAYP3_FILE).expect("decoded");
        assert_eq!(decoded.image.as_bytes(), rgb_test_image().as_bytes(), "the file holds the color test image");
        let profile = match decoded.profile {
            Ok(Some(profile @ Profile::Icc(_))) => profile,
            Ok(_) => panic!("the file gives a profile other than an ICC one"),
            Err(why) => panic!("the profile cannot be read: {why}"),
        };
        let got = convert(&decoded.image, None, &profile, RenderingIntent::Perceptual).expect("converted").image;
        let largest = largest_difference(got.as_bytes(), &reference::DISPLAYP3_PERCEPTUAL);
        assert!(largest <= TOLERANCE, "a sample is {largest} levels from littleCMS's");
    }

    #[test]
    fn converts_a_gray_profile_and_keeps_one_channel() {
        use reference as r;
        let gray = matches_littlecms("sGrey", &gray_test_image(), None, &r::SGREY_PROFILE, [&r::SGREY_PERCEPTUAL, &r::SGREY_RELATIVE, &r::SGREY_SATURATION, &r::SGREY_ABSOLUTE]);
        assert!(gray.iter().all(|g| matches!(g, DynamicImage::ImageLuma8(_))), "a gray image keeps one channel");
    }

    /// The header of a 1 by 1 document of `mode` at `depth` bits.
    fn header_of(mode: Mode, depth: usize) -> Header {
        Header { large: false, channels: mode.colors(), width: 1, height: 1, depth, mode }
    }

    /// An 8-bit level of a value from 0 to 1.
    fn level(v: f64) -> i32 {
        (v.clamp(0.0, 1.0) * 255.0).round() as i32
    }

    /// fill_reference's sRGB and Lab colors converted into `profile` for an
    /// 8-bit document of `mode` against littleCMS's conversions of them: no
    /// sample more than TOLERANCE levels apart, littleCMS's inks taken as
    /// what each leaves for CMYK. Unless `like_no_profile`, some color comes
    /// out more than TOLERANCE levels from what the document would take
    /// without the profile, so the profile is what converted; with it,
    /// every color comes out within TOLERANCE of that.
    fn converts_as_littlecms<const N: usize>(name: &str, profile: &[u8], mode: Mode, from_srgb: &[[f64; N]], from_lab: &[[f64; N]], like_no_profile: bool) {
        use crate::fill_reference as f;
        let header = header_of(mode, 8);
        let colors: Vec<Foreign> = f::FILL_SRGB.iter().map(|&c| Foreign::Srgb(c)).chain(f::FILL_LAB.iter().map(|&c| Foreign::Lab(c))).collect();
        let references: Vec<&[f64; N]> = from_srgb.iter().chain(from_lab).collect();
        assert_eq!(colors.len(), references.len(), "{name}: a reference for each color");
        let mut moved = 0;
        for (&color, reference) in colors.iter().zip(references) {
            let ours = fill_color(color, &header, Some(profile)).expect("converted");
            let theirs: Vec<f64> = if mode == Mode::Cmyk { reference.iter().map(|ink| 1.0 - ink).collect() } else { reference.to_vec() };
            assert_eq!(ours.len(), theirs.len(), "{name}: the document's channels");
            for (o, t) in ours.iter().zip(&theirs) {
                let apart = (level(f64::from(*o)) - level(*t)).abs();
                assert!(apart <= i32::from(TOLERANCE), "{name}, {color:?}: {ours:?} is {apart} levels from littleCMS's {theirs:?}");
            }
            let without = fill_color(color, &header, None).expect("converted");
            moved = moved.max(ours.iter().zip(&without).map(|(o, w)| (level(f64::from(*o)) - level(f64::from(*w))).abs()).max().expect("samples"));
        }
        if like_no_profile {
            assert!(moved <= i32::from(TOLERANCE), "{name}: a color comes out {moved} levels from the no-profile reading of an sRGB document");
        } else {
            assert!(moved > i32::from(TOLERANCE), "{name}: every color comes out within {moved} levels of the no-profile reading, which tells the test nothing");
        }
    }

    /// Fill colors of sRGB and of Lab convert into each kind of profile a fill
    /// writes into as littleCMS converts them relative colorimetrically: RGB
    /// and gray matrix-shapers through moxcms, a gray profile of a Lab PCS by
    /// its curve reversed, CMYK by lut16 and lutBtoA tables, and RGB by a
    /// BToD1 table. In an sRGB document they come out as without a profile.
    #[test]
    fn converts_fill_colors_into_profiles_as_littlecms_does() {
        use crate::fill_reference as f;
        use reference as r;
        converts_as_littlecms("sRGB", &r::SRGB_PROFILE, Mode::Rgb, &f::SRGB_FROM_SRGB, &f::SRGB_FROM_LAB, true);
        converts_as_littlecms("Display P3", &r::DISPLAYP3_PROFILE, Mode::Rgb, &f::DISPLAYP3_FROM_SRGB, &f::DISPLAYP3_FROM_LAB, false);
        converts_as_littlecms("Adobe RGB", &r::ADOBE_PROFILE, Mode::Rgb, &f::ADOBE_FROM_SRGB, &f::ADOBE_FROM_LAB, false);
        converts_as_littlecms("ProPhoto", &r::PROPHOTO_PROFILE, Mode::Rgb, &f::PROPHOTO_FROM_SRGB, &f::PROPHOTO_FROM_LAB, false);
        converts_as_littlecms("Rec. 2020", &r::REC2020_PROFILE, Mode::Rgb, &f::REC2020_FROM_SRGB, &f::REC2020_FROM_LAB, false);
        converts_as_littlecms("sGrey", &r::SGREY_PROFILE, Mode::Gray, &f::SGREY_FROM_SRGB, &f::SGREY_FROM_LAB, false);
        converts_as_littlecms("gray of a Lab PCS", &r::LAB_GRAY_PROFILE, Mode::Gray, &f::LAB_GRAY_FROM_SRGB, &f::LAB_GRAY_FROM_LAB, false);
        converts_as_littlecms("CMYK, lut16", &f::CMYK_V2_PROFILE, Mode::Cmyk, &f::CMYK_V2_FROM_SRGB, &f::CMYK_V2_FROM_LAB, false);
        converts_as_littlecms("CMYK, lutBtoA", &f::CMYK_V4_PROFILE, Mode::Cmyk, &f::CMYK_V4_FROM_SRGB, &f::CMYK_V4_FROM_LAB, false);
        converts_as_littlecms("RGB, BToD1", &f::BTOD_RGB_PROFILE, Mode::Rgb, &f::BTOD_RGB_FROM_SRGB, &f::BTOD_RGB_FROM_LAB, false);
    }

    /// A document without a profile takes a fill color as the import reads
    /// such a document, inverted: RGB as sRGB, at 32 bits as linear light;
    /// gray as sRGB's gray of the color's Rec. 709 luminance, at 32 bits as
    /// the luminance; CMYK with black 1 less the largest channel, whose inks
    /// the import's formula takes back to the color. A Lab document takes the
    /// color's Lab, a Lab one's as it is and an sRGB one's by its XYZ, L over
    /// 100 and a and b each plus 128 over 256, as Photoshop's 15-bit Lab
    /// holds them; a bitmap or indexed one none.
    #[test]
    fn converts_fill_colors_without_a_profile_as_the_import_reads_such_a_document() {
        let fill = |color: Foreign, mode: Mode, depth: usize| fill_color(color, &header_of(mode, depth), None).expect("converted");
        assert_eq!(fill(Foreign::Srgb([0.2, 0.4, 0.5]), Mode::Rgb, 8), [0.2, 0.4, 0.5]);
        assert_eq!(fill(Foreign::Srgb([1.0, 0.0, 0.04045]), Mode::Rgb, 32), [1.0, 0.0, (0.04045 / 12.92) as f32]);
        assert_eq!(fill(Foreign::Srgb([0.5, 0.5, 0.5]), Mode::Gray, 8), [0.5]);
        assert_eq!(fill(Foreign::Srgb([1.0, 0.0, 0.0]), Mode::Gray, 32), [0.2126]);
        assert_eq!(fill(Foreign::Srgb([1.0, 1.0, 1.0]), Mode::Duotone, 8), [1.0]);
        let leaves = fill(Foreign::Srgb([0.2, 0.4, 0.5]), Mode::Cmyk, 8);
        assert_eq!(leaves, [0.4, 0.8, 1.0, 0.5]);
        assert_eq!([0, 1, 2].map(|c| leaves[c] * leaves[3]), [0.2, 0.4, 0.5], "the import's formula takes the inks back to the color");
        assert_eq!(fill(Foreign::Srgb([0.0, 0.0, 0.0]), Mode::Cmyk, 8), [1.0, 1.0, 1.0, 0.0]);
        assert_eq!(fill(Foreign::Lab([50.0, 20.0, -30.0]), Mode::Lab, 8), [0.5, (148.0f64 / 256.0) as f32, (98.0f64 / 256.0) as f32]);
        let white = fill(Foreign::Srgb([1.0, 1.0, 1.0]), Mode::Lab, 16);
        assert_eq!(white, fill(Foreign::Lab(profile_table::xyz_to_lab(srgb_xyz([1.0, 1.0, 1.0]))), Mode::Lab, 16), "an sRGB color goes in as the Lab of its XYZ");
        assert_eq!(white[0], 1.0, "white's L is 100");
        for mode in [Mode::Bitmap, Mode::Indexed] {
            let why = fill_color(Foreign::Srgb([0.2, 0.4, 0.5]), &header_of(mode, 8), None).unwrap_err();
            assert!(why.contains("into RGB, gray, CMYK and Lab documents"), "{mode:?}: {why}");
        }
    }

    /// A fill color converts into no profile that only reads colors, which
    /// littleCMS refuses as well, nor into a profile of another color than
    /// its document's.
    #[test]
    fn refuses_to_convert_a_fill_color_into_a_profile_that_only_reads_colors() {
        use reference as r;
        let srgb = Foreign::Srgb([0.2, 0.4, 0.5]);
        for (name, profile, mode) in [("CMYK", &r::CMYK_PROFILE[..], Mode::Cmyk), ("lutAtoB RGB", &r::MAB_RGB_PROFILE, Mode::Rgb), ("lutAtoB gray", &r::MAB_GRAY_PROFILE, Mode::Gray)] {
            let why = fill_color(srgb, &header_of(mode, 8), Some(profile)).unwrap_err();
            assert!(why.contains("no table for writing colors"), "{name}: {why}");
        }
        let why = fill_color(srgb, &header_of(Mode::Cmyk, 8), Some(&r::SRGB_PROFILE)).unwrap_err();
        assert!(why.contains("is of Rgb color, where the document is CMYK"), "{why}");
    }

    /// Every ink of the CMYK profile's grid, cyan slowest, as
    /// tools/profile_reference.py converts them, and an image of as many
    /// pixels in place of the image crate's RGB from them, which a
    /// conversion from the inks does not read.
    fn ink_grid() -> (DynamicImage, Cmyk) {
        let steps: Vec<u8> = (0..reference::GRID).map(|i| u8::try_from(255 * i / (reference::GRID - 1)).expect("a step")).collect();
        let mut inks = Vec::new();
        for &c in &steps {
            for &m in &steps {
                for &y in &steps {
                    for &k in &steps {
                        inks.extend_from_slice(&[c, m, y, k]);
                    }
                }
            }
        }
        (DynamicImage::ImageRgb8(ImageBuffer::new(reference::GRID.pow(4), 1)), Cmyk::Eight(inks))
    }

    #[test]
    fn converts_cmyk_inks_as_littlecms_does() {
        use reference as r;
        let (formula, inks) = ink_grid();
        let rgb = matches_littlecms("CMYK", &formula, Some(&inks), &r::CMYK_PROFILE, [&r::CMYK_PERCEPTUAL, &r::CMYK_RELATIVE, &r::CMYK_SATURATION, &r::CMYK_ABSOLUTE]);
        assert!(rgb.iter().all(|i| (i.width(), i.height()) == (reference::GRID.pow(4), 1)), "every ink converts to a pixel");
    }

    /// `profile` with its D2B tags renamed, so that it converts by its A2B
    /// tables or its matrix and curves.
    fn without_d2b(profile: &[u8]) -> Vec<u8> {
        let mut out = profile.to_vec();
        let count = u32::from_be_bytes(out[128..132].try_into().expect("a tag count")) as usize;
        for entry in out[132..132 + 12 * count].as_chunks_mut::<12>().0 {
            if entry.starts_with(b"D2B") {
                entry[3] = b'_';
            }
        }
        out
    }

    /// What each intent's conversion from `profile` went through: the tag
    /// of the table the module converted by, "matrix-shaper" where the
    /// module converted by the profile's matrix and curves, or "moxcms"
    /// where moxcms converted.
    fn tables_used(image: &DynamicImage, cmyk: Option<&Cmyk>, profile: &[u8]) -> [&'static str; 4] {
        INTENTS.map(|intent| match convert(image, cmyk, &Profile::Icc(profile.to_vec()), intent).expect("converted").table {
            Some(table) => table.tag.unwrap_or("matrix-shaper"),
            None => "moxcms",
        })
    }

    /// Profiles of D2B tables, floating point ones, convert as littleCMS
    /// converts them, each intent by the table littleCMS picks: its own D2B
    /// table over any A2B table or matrix-shaper; and where it has no D2B
    /// table, the A2B table or matrix-shaper littleCMS falls back to, the
    /// absolute intent by A2B1, then A2B0, never D2B1.
    #[test]
    fn converts_d2b_tables_as_littlecms_does() {
        use reference as r;
        let rgb = rgb_test_image();
        let references: [&[u8]; 4] = [&r::FLOAT_RGB_PERCEPTUAL, &r::FLOAT_RGB_RELATIVE, &r::FLOAT_RGB_SATURATION, &r::FLOAT_RGB_ABSOLUTE];
        matches_littlecms("D2B RGB", &rgb, None, &r::FLOAT_RGB_PROFILE, references);
        assert_eq!(tables_used(&rgb, None, &r::FLOAT_RGB_PROFILE), ["D2B0", "D2B1", "D2B2", "D2B3"]);
        let without = without_d2b(&r::FLOAT_RGB_PROFILE);
        assert_eq!(tables_used(&rgb, None, &without), ["A2B0"; 4]);
        for (intent, expected) in INTENTS.into_iter().zip(references) {
            let apart = largest_difference(converted(&rgb, None, &without, intent).as_bytes(), expected);
            assert!(apart > TOLERANCE, "{intent:?}: the profile's A2B0 table converts within {apart} levels of its D2B table, which tells the test nothing");
        }
        let (formula, inks) = ink_grid();
        matches_littlecms("D2B CMYK", &formula, Some(&inks), &r::FLOAT_CMYK_PROFILE, [&r::FLOAT_CMYK_PERCEPTUAL, &r::FLOAT_CMYK_RELATIVE, &r::FLOAT_CMYK_SATURATION, &r::FLOAT_CMYK_ABSOLUTE]);
        assert_eq!(tables_used(&formula, Some(&inks), &r::FLOAT_CMYK_PROFILE), ["D2B0", "D2B1", "A2B0", "A2B0"]);
        let without = without_d2b(&r::FLOAT_CMYK_PROFILE);
        for (intent, expected) in [(RenderingIntent::Perceptual, &r::FLOAT_CMYK_PERCEPTUAL), (RenderingIntent::RelativeColorimetric, &r::FLOAT_CMYK_RELATIVE)] {
            let apart = largest_difference(converted(&formula, Some(&inks), &without, intent).as_bytes(), expected);
            assert!(apart > TOLERANCE, "CMYK, {intent:?}: the profile's A2B0 table converts within {apart} levels of its D2B table, which tells the test nothing");
        }
        let gray = gray_test_image();
        let grays = matches_littlecms("D2B gray", &gray, None, &r::FLOAT_GRAY_PROFILE, [&r::FLOAT_GRAY_PERCEPTUAL, &r::FLOAT_GRAY_RELATIVE, &r::FLOAT_GRAY_SATURATION, &r::FLOAT_GRAY_ABSOLUTE]);
        assert!(grays.iter().all(|g| matches!(g, DynamicImage::ImageLuma8(_))), "a gray image keeps one channel");
        assert_eq!(tables_used(&gray, None, &r::FLOAT_GRAY_PROFILE), ["D2B0", "moxcms", "moxcms", "moxcms"]);
        let apart = largest_difference(converted(&gray, None, &without_d2b(&r::FLOAT_GRAY_PROFILE), RenderingIntent::Perceptual).as_bytes(), &r::FLOAT_GRAY_PERCEPTUAL);
        assert!(apart > TOLERANCE, "gray: the profile's gray curve converts within {apart} levels of its D2B0 table, which tells the test nothing");
    }

    /// A conversion with its curves worked out for every 8-bit and 16-bit
    /// sample value, or a gray one worked out whole, gives exactly what it
    /// gives each pixel worked out alone.
    #[test]
    fn a_tabled_conversion_gives_what_each_pixel_does() {
        use reference as r;
        let to_srgb = srgb_for(false).colorant_matrix().inverse().v;
        let conversion = |icc: &[u8]| {
            let profile = ColorProfile::new_from_slice(icc).expect("a profile");
            TableConversion::new(&profile, icc, RenderingIntent::Perceptual, to_srgb).expect("the table reads").expect("a table")
        };
        let unit8 = |p: &[u8]| p.iter().map(|&s| f64::from(s) / 255.0).collect::<Vec<f64>>();
        let unit16 = |p: &[u16]| p.iter().map(|&s| f64::from(s) / 65535.0).collect::<Vec<f64>>();
        for icc in [&r::FLOAT_RGB_PROFILE[..], &r::MAB_RGB_PROFILE, &r::LIFTED_RGB_PROFILE] {
            let rgb = conversion(icc);
            let (eight, sixteen) = (rgb.tabled::<u8>(), rgb.tabled::<u16>());
            for v in 0..=255u8 {
                let p = [v, 255 - v, v / 2];
                assert_eq!(eight.rgb(&p), rgb.rgb(&unit8(&p)), "8-bit {p:?}");
            }
            for v in (0..=65535u16).step_by(97) {
                let p = [v, 65535 - v, v / 3];
                assert_eq!(sixteen.rgb(&p), rgb.rgb(&unit16(&p)), "16-bit {p:?}");
            }
        }
        let cmyk = conversion(&r::CMYK_PROFILE);
        let (eight, sixteen) = (cmyk.tabled::<u8>(), cmyk.tabled::<u16>());
        for v in 0..=255u8 {
            let p = [v, 255 - v, v / 2, v / 3];
            assert_eq!(eight.rgb(&p), cmyk.rgb(&unit8(&p)), "8-bit inks {p:?}");
            let wide = p.map(|s| u16::from(s) * 255 + u16::from(s % 7));
            assert_eq!(sixteen.rgb(&wide), cmyk.rgb(&unit16(&wide)), "16-bit inks {wide:?}");
        }
        let gray = conversion(&r::MAB_GRAY_PROFILE);
        let (eight, sixteen) = (gray.tabled::<u8>(), gray.tabled::<u16>());
        for v in 0..=255u8 {
            assert_eq!(eight.gray(v), gray.gray(&unit8(&[v])), "8-bit gray {v}");
        }
        for v in (0..=65535u16).step_by(31) {
            assert_eq!(sixteen.gray(v), gray.gray(&unit16(&[v])), "16-bit gray {v}");
        }
    }

    /// A profile the tests convert from, the image they convert and what
    /// each intent's conversion goes through, as tables_used gives it.
    type Expected<'a> = (Case<'a>, &'a DynamicImage, [&'a str; 4]);

    /// Matrix-shapers that moxcms converts otherwise than littleCMS convert
    /// as littleCMS does, through the module's own steps: a black above 0,
    /// which the perceptual and saturation intents compensate; an A2B1
    /// table alone; a media white that is not D50, which the absolute intent
    /// scales by; and a gray curve giving L* through a Lab PCS. moxcms
    /// converts the intents littleCMS adjusts nothing for.
    #[test]
    fn converts_matrix_shapers_moxcms_converts_otherwise_as_littlecms_does() {
        use reference as r;
        let (rgb, gray) = (rgb_test_image(), gray_test_image());
        let cases: [Expected; 4] = [
            (("lifted black", &r::LIFTED_RGB_PROFILE, [&r::LIFTED_RGB_PERCEPTUAL, &r::LIFTED_RGB_RELATIVE, &r::LIFTED_RGB_SATURATION, &r::LIFTED_RGB_ABSOLUTE]), &rgb, ["matrix-shaper", "moxcms", "matrix-shaper", "moxcms"]),
            (("A2B1 alone", &r::A2B1_ONLY_RGB_PROFILE, [&r::A2B1_ONLY_RGB_PERCEPTUAL, &r::A2B1_ONLY_RGB_RELATIVE, &r::A2B1_ONLY_RGB_SATURATION, &r::A2B1_ONLY_RGB_ABSOLUTE]), &rgb, ["matrix-shaper", "A2B1", "matrix-shaper", "A2B1"]),
            (("white not D50", &r::WHITE_RGB_PROFILE, [&r::WHITE_RGB_PERCEPTUAL, &r::WHITE_RGB_RELATIVE, &r::WHITE_RGB_SATURATION, &r::WHITE_RGB_ABSOLUTE]), &rgb, ["moxcms", "moxcms", "moxcms", "matrix-shaper"]),
            (("Lab gray", &r::LAB_GRAY_PROFILE, [&r::LAB_GRAY_PERCEPTUAL, &r::LAB_GRAY_RELATIVE, &r::LAB_GRAY_SATURATION, &r::LAB_GRAY_ABSOLUTE]), &gray, ["matrix-shaper"; 4]),
        ];
        for ((name, profile, references), image, used) in cases {
            matches_littlecms(name, image, None, profile, references);
            assert_eq!(tables_used(image, None, profile), used, "{name}");
        }
        // moxcms gives the relative intent's colors for the intents it does
        // not adjust, so littleCMS's adjusted ones must differ from them for
        // the test to tell the two ways apart.
        let compensated = largest_difference(&r::LIFTED_RGB_PERCEPTUAL, &r::LIFTED_RGB_RELATIVE);
        assert!(compensated > TOLERANCE, "littleCMS's compensated black is within {compensated} levels of its relative conversion");
        let scaled = largest_difference(&r::WHITE_RGB_ABSOLUTE, &r::WHITE_RGB_RELATIVE);
        assert!(scaled > TOLERANCE, "littleCMS's scaled white is within {scaled} levels of its relative conversion");
    }

    /// A D2B table littleCMS refuses, one of whose elements is of a type it
    /// does not read, leaves the colors unconverted with the reason, while
    /// the profile's other tables convert as littleCMS converts by them.
    #[test]
    fn a_d2b_table_littlecms_refuses_is_refused() {
        use reference as r;
        let image = rgb_test_image();
        match convert(&image, None, &Profile::Icc(r::FLOAT_RGB_BROKEN_PROFILE.to_vec()), RenderingIntent::Perceptual) {
            Ok(_) => panic!("the broken D2B0 table converted the colors"),
            Err(why) => assert!(why.contains("'clux'"), "the reason names no element type: {why}"),
        }
        let relative = converted(&image, None, &r::FLOAT_RGB_BROKEN_PROFILE, RenderingIntent::RelativeColorimetric);
        let largest = largest_difference(relative.as_bytes(), &r::FLOAT_RGB_RELATIVE);
        assert!(largest <= TOLERANCE, "a sample is {largest} levels from littleCMS's");
    }

    /// A float RGB image of `values`, a pixel each, in one row.
    fn float_image(values: &[[f32; 3]]) -> DynamicImage {
        let width = u32::try_from(values.len()).expect("a row of values");
        DynamicImage::ImageRgb32F(ImageBuffer::from_raw(width, 1, values.iter().flatten().copied().collect()).expect("one row of pixels"))
    }

    /// A profile the float tests convert from: its name, its bytes, and
    /// littleCMS's float conversions of the test values with each intent.
    type FloatCase<'a> = (&'a str, &'a [u8], [&'a [[f32; 3]]; 4]);

    /// How far a float conversion's value may be from littleCMS's: an
    /// absolute part, as littleCMS's float transforms round 8- and 16-bit
    /// tables to 16 bits where the module does not, and a part of the
    /// value's size.
    const FLOAT_ABSOLUTE: f32 = 2e-4;
    const FLOAT_RELATIVE: f32 = 2e-5;

    /// Whether `got` is within the float tolerance of littleCMS's `wanted`.
    fn near_littlecms(got: f32, wanted: f32) -> bool {
        (got - wanted).abs() <= FLOAT_ABSOLUTE + FLOAT_RELATIVE * wanted.abs()
    }

    /// Float values from below 0 to above 1 convert from every RGB and gray
    /// profile to linear light in sRGB's primaries as littleCMS's float
    /// transforms convert them, light below 0 clipped to 0.
    #[test]
    fn converts_float_images_to_linear_light_as_littlecms_does() {
        use crate::float_reference as f;
        use reference as r;
        let rgb: [FloatCase; 13] = [
            ("sRGB", &r::SRGB_PROFILE, [&f::SRGB_PERCEPTUAL, &f::SRGB_RELATIVE, &f::SRGB_SATURATION, &f::SRGB_ABSOLUTE]),
            ("Display P3", &r::DISPLAYP3_PROFILE, [&f::DISPLAYP3_PERCEPTUAL, &f::DISPLAYP3_RELATIVE, &f::DISPLAYP3_SATURATION, &f::DISPLAYP3_ABSOLUTE]),
            ("Display P3 version 2", &r::DISPLAYP3_V2_PROFILE, [&f::DISPLAYP3_V2_PERCEPTUAL, &f::DISPLAYP3_V2_RELATIVE, &f::DISPLAYP3_V2_SATURATION, &f::DISPLAYP3_V2_ABSOLUTE]),
            ("Adobe RGB", &r::ADOBE_PROFILE, [&f::ADOBE_PERCEPTUAL, &f::ADOBE_RELATIVE, &f::ADOBE_SATURATION, &f::ADOBE_ABSOLUTE]),
            ("ProPhoto", &r::PROPHOTO_PROFILE, [&f::PROPHOTO_PERCEPTUAL, &f::PROPHOTO_RELATIVE, &f::PROPHOTO_SATURATION, &f::PROPHOTO_ABSOLUTE]),
            ("Rec. 2020", &r::REC2020_PROFILE, [&f::REC2020_PERCEPTUAL, &f::REC2020_RELATIVE, &f::REC2020_SATURATION, &f::REC2020_ABSOLUTE]),
            ("lutAtoB", &r::MAB_RGB_PROFILE, [&f::MAB_RGB_PERCEPTUAL, &f::MAB_RGB_RELATIVE, &f::MAB_RGB_SATURATION, &f::MAB_RGB_ABSOLUTE]),
            ("lut8", &r::LUT8_RGB_PROFILE, [&f::LUT8_RGB_PERCEPTUAL, &f::LUT8_RGB_RELATIVE, &f::LUT8_RGB_SATURATION, &f::LUT8_RGB_ABSOLUTE]),
            ("D2B", &r::FLOAT_RGB_PROFILE, [&f::FLOAT_RGB_PERCEPTUAL, &f::FLOAT_RGB_RELATIVE, &f::FLOAT_RGB_SATURATION, &f::FLOAT_RGB_ABSOLUTE]),
            ("lifted black", &r::LIFTED_RGB_PROFILE, [&f::LIFTED_RGB_PERCEPTUAL, &f::LIFTED_RGB_RELATIVE, &f::LIFTED_RGB_SATURATION, &f::LIFTED_RGB_ABSOLUTE]),
            ("A2B1 alone", &r::A2B1_ONLY_RGB_PROFILE, [&f::A2B1_ONLY_RGB_PERCEPTUAL, &f::A2B1_ONLY_RGB_RELATIVE, &f::A2B1_ONLY_RGB_SATURATION, &f::A2B1_ONLY_RGB_ABSOLUTE]),
            ("white not D50", &r::WHITE_RGB_PROFILE, [&f::WHITE_RGB_PERCEPTUAL, &f::WHITE_RGB_RELATIVE, &f::WHITE_RGB_SATURATION, &f::WHITE_RGB_ABSOLUTE]),
            ("linear RGB", &f::LINEAR_RGB_PROFILE, [&f::LINEAR_RGB_PERCEPTUAL, &f::LINEAR_RGB_RELATIVE, &f::LINEAR_RGB_SATURATION, &f::LINEAR_RGB_ABSOLUTE]),
        ];
        let gray: [FloatCase; 5] = [
            ("sGrey", &r::SGREY_PROFILE, [&f::SGREY_PERCEPTUAL, &f::SGREY_RELATIVE, &f::SGREY_SATURATION, &f::SGREY_ABSOLUTE]),
            ("gray lutAtoB", &r::MAB_GRAY_PROFILE, [&f::MAB_GRAY_PERCEPTUAL, &f::MAB_GRAY_RELATIVE, &f::MAB_GRAY_SATURATION, &f::MAB_GRAY_ABSOLUTE]),
            ("gray D2B", &r::FLOAT_GRAY_PROFILE, [&f::FLOAT_GRAY_PERCEPTUAL, &f::FLOAT_GRAY_RELATIVE, &f::FLOAT_GRAY_SATURATION, &f::FLOAT_GRAY_ABSOLUTE]),
            ("Lab gray", &r::LAB_GRAY_PROFILE, [&f::LAB_GRAY_PERCEPTUAL, &f::LAB_GRAY_RELATIVE, &f::LAB_GRAY_SATURATION, &f::LAB_GRAY_ABSOLUTE]),
            ("linear gray", &f::LINEAR_GRAY_PROFILE, [&f::LINEAR_GRAY_PERCEPTUAL, &f::LINEAR_GRAY_RELATIVE, &f::LINEAR_GRAY_SATURATION, &f::LINEAR_GRAY_ABSOLUTE]),
        ];
        let rgb_image = float_image(&f::RGB_VALUES);
        let gray_image = float_image(&f::GRAY_VALUES.map(|v| [v; 3]));
        for (cases, image, gray) in [(&rgb[..], &rgb_image, false), (&gray[..], &gray_image, true)] {
            let stored = image.as_rgb32f().expect("float RGB").as_raw().clone();
            for &(name, profile, references) in cases {
                for (intent, expected) in INTENTS.into_iter().zip(references) {
                    let converted = float_converted(image, gray, &Profile::Icc(profile.to_vec()), intent).expect("converted");
                    assert!(converted.linear, "{name}, {intent:?}: the colors are not linear light");
                    let got = converted.image.as_rgb32f().expect("float RGB").as_raw().clone();
                    let wanted: Vec<f32> = expected.iter().flatten().map(|v| v.max(0.0)).collect();
                    assert_eq!(got.len(), wanted.len(), "{name}, {intent:?}: the conversion gives another count of values");
                    for (i, (g, w)) in got.iter().zip(&wanted).enumerate() {
                        assert!(near_littlecms(*g, *w), "{name}, {intent:?}: value {} channel {} is {g}, where littleCMS gives {w}", i / 3, i % 3);
                    }
                    if !name.starts_with("linear") {
                        let unconverted = stored.iter().zip(&wanted).all(|(s, w)| near_littlecms(*s, *w));
                        assert!(!unconverted, "{name}, {intent:?}: the values as stored are as near littleCMS's, which tells the conversion nothing");
                    }
                }
            }
        }
        // Rec. 2020's colors sRGB lacks give littleCMS light well below 0,
        // which the conversion clips to 0.
        let wide = float_converted(&rgb_image, false, &Profile::Icc(r::REC2020_PROFILE.to_vec()), RenderingIntent::RelativeColorimetric).expect("converted");
        let wide = wide.image.as_rgb32f().expect("float RGB").as_raw().clone();
        let lacking: Vec<usize> = f::REC2020_RELATIVE.iter().flatten().enumerate().filter(|&(_, &v)| v < -FLOAT_ABSOLUTE).map(|(i, _)| i).collect();
        assert!(!lacking.is_empty(), "littleCMS gives no light below 0 for Rec. 2020's colors");
        for i in lacking {
            assert_eq!(wide[i], 0.0, "value {} channel {}: light below 0 is not clipped", i / 3, i % 3);
        }
    }

    /// A document whose profile has tables both ways and no matrix-shaper's
    /// curves mixes a gradient of the linear method in the XYZ of those
    /// tables: two colors taken there, mixed in a straight line and taken
    /// back give littleCMS's mix through the same tables, within the float
    /// tolerance, and the middle mix differs from the mix of the stored
    /// values. A profile that only reads colors, one that only writes them,
    /// one of another color than the document, and a document whose samples
    /// are not light give why.
    #[test]
    fn mixes_in_the_xyz_of_a_profiles_tables() {
        use crate::fill_reference as f;
        use reference as r;
        let light = document_light(&header_of(Mode::Rgb, 8), Some(&f::TABLE_RGB_PROFILE)).expect("the profile's light");
        assert_eq!(light.width, 3, "a color's light is its XYZ");
        let ends = f::MIXED_COLORS.map(|color| {
            let mut xyz = [0.0; 3];
            (light.into)(&color, &mut xyz);
            xyz
        });
        let mut got = Vec::new();
        for (&t, wanted) in f::MIX_PLACES.iter().zip(&f::TABLE_RGB_MIXED) {
            let mixed = [0, 1, 2].map(|k| ends[0][k] + (ends[1][k] - ends[0][k]) * t);
            let mut samples = [0.0; 3];
            (light.back)(&mixed, &mut samples);
            for (g, w) in samples.iter().zip(wanted) {
                assert!(near_littlecms(*g as f32, *w as f32), "at {t}: {samples:?}, where littleCMS gives {wanted:?}");
            }
            got.push(samples);
        }
        let stored = [0, 1, 2].map(|k| (f::MIXED_COLORS[0][k] + f::MIXED_COLORS[1][k]) / 2.0);
        assert!(stored.iter().zip(&got[2]).any(|(s, g)| !near_littlecms(*s as f32, *g as f32)), "the mix in light is the mix of the stored values, which tells the test nothing");
        let why = |icc: &[u8], mode: Mode| match document_light(&header_of(mode, 8), Some(icc)) {
            Ok(_) => panic!("{mode:?}: the profile gives light"),
            Err(why) => why,
        };
        assert!(why(&r::MAB_RGB_PROFILE, Mode::Rgb).contains("no table for writing colors"));
        assert!(why(&f::BTOD_RGB_PROFILE, Mode::Rgb).contains("neither curves nor a table to read colors by"));
        assert!(why(&r::SGREY_PROFILE, Mode::Rgb).contains("of Gray color, where the document is RGB"));
        for mode in [Mode::Cmyk, Mode::Lab, Mode::Duotone] {
            let why = document_light(&header_of(mode, 8), None).err().expect("no light");
            assert!(why.contains("samples are not light the merge mixes in"), "{mode:?}: {why}");
        }
    }

    /// A float RGB TIFF of `values`, a pixel each in one row, carrying the
    /// ICC profile `icc`.
    fn float_tiff(values: &[[f32; 3]], icc: &[u8]) -> Vec<u8> {
        use tiff::encoder::{colortype::RGB32Float, TiffEncoder};
        let mut out = Cursor::new(Vec::new());
        let mut encoder = TiffEncoder::new(&mut out).expect("an encoder");
        let width = u32::try_from(values.len()).expect("a row of values");
        let mut image = encoder.new_image::<RGB32Float>(width, 1).expect("an image");
        image.encoder().write_tag(tiff::tags::Tag::IccProfile, icc).expect("the profile's tag");
        let samples: Vec<f32> = values.iter().flatten().copied().collect();
        image.write_data(&samples).expect("the samples");
        out.into_inner()
    }

    /// A float TIFF carrying a color profile loads as linear light in sRGB's
    /// primaries, as littleCMS's float transforms convert it.
    #[test]
    fn a_float_tiff_converts_to_linear_light() {
        use crate::float_reference as f;
        let part = only_part("float.tif", &float_tiff(&f::RGB_VALUES, &reference::SRGB_PROFILE));
        let profile = match part.profile {
            Ok(Some(profile)) => profile,
            Ok(None) => panic!("the TIFF's profile was not found"),
            Err(why) => panic!("the TIFF's profile was not read: {why}"),
        };
        let frame = &part.frames[0];
        let converted = convert(&frame.image, frame.cmyk.as_ref(), &profile, RenderingIntent::Perceptual).expect("converted");
        assert!(converted.linear, "the colors are not linear light");
        let got = converted.image.as_rgb32f().expect("float RGB").as_raw().clone();
        let wanted: Vec<f32> = f::SRGB_PERCEPTUAL.iter().flatten().map(|v| v.max(0.0)).collect();
        assert_eq!(got.len(), wanted.len(), "the TIFF loads another count of values");
        for (i, (g, w)) in got.iter().zip(&wanted).enumerate() {
            assert!(near_littlecms(*g, *w), "value {} channel {} is {g}, where littleCMS gives {w}", i / 3, i % 3);
        }
    }

    /// A 2 by 1 RGB PNG, red then blue, with a "Raw profile type exif" text
    /// chunk of `kind`, tEXt, zTXt or iTXt, holding `text`.
    fn png_with_text_exif(kind: &str, text: &str) -> Vec<u8> {
        let mut out = Vec::new();
        let mut encoder = png::Encoder::new(&mut out, 2, 1);
        encoder.set_color(png::ColorType::Rgb);
        encoder.set_depth(png::BitDepth::Eight);
        let keyword = "Raw profile type exif".to_string();
        match kind {
            "tEXt" => encoder.add_text_chunk(keyword, text.to_string()),
            "zTXt" => encoder.add_ztxt_chunk(keyword, text.to_string()),
            _ => encoder.add_itxt_chunk(keyword, text.to_string()),
        }
        .expect("a text chunk");
        let mut writer = encoder.write_header().expect("a header");
        writer.write_image_data(&[255, 0, 0, 0, 0, 255]).expect("the pixels");
        writer.finish().expect("the end");
        out
    }

    #[test]
    fn a_png_turns_by_the_exif_imagemagick_writes_in_a_text_chunk() {
        // A little-endian TIFF block whose one IFD entry is Orientation (274), a SHORT of 6.
        let tiff = [b'I', b'I', 42, 0, 8, 0, 0, 0, 1, 0, 0x12, 0x01, 3, 0, 1, 0, 0, 0, 6, 0, 0, 0, 0, 0, 0, 0];
        let exif = [b"Exif\0\0".as_slice(), &tiff].concat();
        let hex: String = exif.iter().map(|b| format!("{b:02x}")).collect();
        // ImageMagick's form: a blank line, the profile's name, its length, then the hexadecimal.
        let text = format!("\nexif\n{:8}\n{hex}\n", exif.len());
        for kind in ["tEXt", "zTXt", "iTXt"] {
            match only_part("text.png", &png_with_text_exif(kind, &text)).orientation {
                Ok(orientation) => assert_eq!(orientation, Orientation::Rotate90, "{kind}"),
                Err(why) => panic!("{kind}: the orientation was not read: {why}"),
            }
        }
        let odd = only_part("odd.png", &png_with_text_exif("tEXt", "\nexif\n       2\nabc\n"));
        assert!(odd.orientation.is_err(), "an odd number of digits is reported, not read as no orientation");
    }

    /// The one part of the file `name`, holding `bytes`, as decoded: all its
    /// frames, which share one orientation and profile.
    fn only_part(name: &str, bytes: &[u8]) -> Part {
        match decode(Path::new(name), bytes, Reading::default(), None) {
            Ok(chosen) => match <[Part; 1]>::try_from(chosen.parts) {
                Ok([part]) => part,
                Err(parts) => panic!("{name} decodes into {} parts", parts.len()),
            },
            Err(e) => panic!("{name} does not decode: {e:?}"),
        }
    }

    /// A directory of a test TIFF: its size and 8-bit RGBA samples, each
    /// from `seed` on, with the NewSubfileType and Orientation tags it has.
    struct Written {
        width: u32,
        height: u32,
        samples: Vec<u8>,
        kind: Option<u32>,
        orientation: Option<u16>,
    }

    fn written(width: u32, height: u32, kind: Option<u32>, orientation: Option<u16>, seed: u8) -> Written {
        let samples = (0..width * height * 4).map(|i| (i as u8).wrapping_mul(7).wrapping_add(seed)).collect();
        Written { width, height, samples, kind, orientation }
    }

    /// `directories` chained in their order into `encoder`'s file.
    fn write_directories<W: std::io::Write + std::io::Seek, K: tiff::encoder::TiffKind>(encoder: &mut tiff::encoder::TiffEncoder<W, K>, directories: &[Written]) {
        use tiff::tags::Tag;
        for directory in directories {
            let mut image = encoder.new_image::<tiff::encoder::colortype::RGBA8>(directory.width, directory.height).expect("an image");
            if let Some(kind) = directory.kind {
                image.encoder().write_tag(Tag::NewSubfileType, kind).expect("the subfile type");
            }
            if let Some(orientation) = directory.orientation {
                image.encoder().write_tag(Tag::Orientation, orientation).expect("the orientation");
            }
            image.write_data(&directory.samples).expect("the samples");
        }
    }

    /// A TIFF of `directories`, a BigTIFF with `big`.
    fn tiff_of(directories: &[Written], big: bool) -> Vec<u8> {
        let mut out = Cursor::new(Vec::new());
        if big {
            write_directories(&mut tiff::encoder::TiffEncoder::new_big(&mut out).expect("an encoder"), directories);
        } else {
            write_directories(&mut tiff::encoder::TiffEncoder::new(&mut out).expect("an encoder"), directories);
        }
        out.into_inner()
    }

    /// The frames of `chosen` in the order loaded, each with its index in
    /// the file and its samples as 8-bit RGBA.
    fn in_order(chosen: Chosen) -> Vec<(usize, Vec<u8>)> {
        let mut frames: Vec<Frame> = chosen.parts.into_iter().flat_map(|part| part.frames).collect();
        frames.sort_by_key(|frame| frame.place);
        frames.into_iter().map(|frame| (frame.index, frame.image.to_rgba8().into_raw())).collect()
    }

    /// A TIFF's pages load in its order, each as written, at its own size,
    /// in a TIFF and in a BigTIFF; the smaller copies and masks among its
    /// directories are left out, and -Frame counts the pages alone.
    #[test]
    fn loads_a_tiffs_pages_and_leaves_out_its_smaller_copies_and_masks() {
        let directories = [written(4, 3, None, None, 1), written(2, 1, Some(1), None, 2), written(3, 5, Some(2), None, 3), written(3, 5, Some(4), None, 4)];
        for big in [false, true] {
            let file = tiff_of(&directories, big);
            let chosen = decode(Path::new("pages.tif"), &file, Reading::default(), None).expect("decoded");
            assert_eq!(chosen.said, ["2 of the file's 4 directories hold a smaller copy of another image or a transparency mask, not a page, so they were left out"], "big {big}");
            assert_eq!(in_order(chosen), [(0, directories[0].samples.clone()), (1, directories[2].samples.clone())], "big {big}");
            let picked = decode(Path::new("pages.tif"), &file, Reading::default(), Some(&[1, 0, 1])).expect("decoded");
            let indices: Vec<usize> = in_order(picked).into_iter().map(|(index, _)| index).collect();
            assert_eq!(indices, [1, 0, 1], "big {big}");
            match decode(Path::new("pages.tif"), &file, Reading::default(), Some(&[2])) {
                Ok(_) => panic!("-Frame 2 picked a page of a file of two"),
                Err(e) => assert_eq!(e.message, "pages.tif cannot be read as an image: -Frame names 2, past its last frame or page, 1"),
            }
        }
    }

    /// A TIFF that marks every directory as a smaller copy loads its first.
    #[test]
    fn loads_the_first_directory_of_a_tiff_that_marks_every_one_a_smaller_copy() {
        let directories = [written(2, 2, Some(1), None, 5), written(1, 1, Some(1), None, 6)];
        let chosen = decode(Path::new("copies.tif"), &tiff_of(&directories, false), Reading::default(), None).expect("decoded");
        assert_eq!(chosen.said, ["the file marks each of its 2 directories as a smaller copy of another image or a transparency mask, so its first loaded"]);
        assert_eq!(in_order(chosen), [(0, directories[0].samples.clone())]);
    }

    /// Each TIFF page is read by its own tags: pages that record the same
    /// orientation make one part, and a page that records another makes
    /// its own.
    #[test]
    fn reads_each_tiff_page_by_its_own_tags() {
        let directories = [written(2, 1, None, None, 7), written(2, 1, None, Some(6), 8), written(2, 1, None, None, 9)];
        let chosen = decode(Path::new("turned.tif"), &tiff_of(&directories, false), Reading::default(), None).expect("decoded");
        let parts: Vec<(Vec<usize>, Orientation)> = chosen
            .parts
            .into_iter()
            .map(|part| match part.orientation {
                Ok(orientation) => (part.frames.iter().map(|frame| frame.index).collect(), orientation),
                Err(why) => panic!("an orientation was not read: {why}"),
            })
            .collect();
        assert_eq!(parts, [(vec![0, 2], Orientation::NoTransforms), (vec![1], Orientation::Rotate90)]);
    }

    #[test]
    fn names_pages_as_a_person_numbers_them() {
        assert_eq!(pages_named(&[1]), "page 2");
        assert_eq!(pages_named(&[0, 2]), "pages 1 and 3");
        assert_eq!(pages_named(&[0, 2, 3, 2]), "pages 1, 3 and 4");
    }

    /// An option warns that it changed nothing only where it changed
    /// nothing for every part of the file.
    #[test]
    fn warns_of_an_option_that_changed_nothing_only_for_every_part() {
        let none = "the file carries no color profile, so -IgnoreProfile changed nothing";
        let flat = "the file records no orientation, so -IgnoreOrientation changed nothing";
        let first = Unchanged { ignore_profile: Some(none.to_string()), intent: None, ignore_orientation: Some(flat.to_string()) };
        let second = Unchanged { ignore_profile: None, intent: None, ignore_orientation: Some(flat.to_string()) };
        assert_eq!(Unchanged::warnings(&[first, second]), [flat]);
    }

    /// A frame's width, height and 8-bit RGBA samples.
    type Rgba = (u32, u32, Vec<u8>);

    /// Each frame of `chosen` in the order loaded, with the timing of those
    /// frames.
    fn rgba_frames(chosen: Chosen) -> (Option<Timing>, Vec<Rgba>) {
        let mut frames: Vec<Frame> = chosen.parts.into_iter().flat_map(|part| part.frames).collect();
        frames.sort_by_key(|frame| frame.place);
        (chosen.timing, frames.into_iter().map(|frame| (frame.image.width(), frame.image.height(), frame.image.to_rgba8().into_raw())).collect())
    }

    fn reference_frames(frames: &[(u32, u32, &[u8])]) -> Vec<Rgba> {
        frames.iter().map(|&(width, height, samples)| (width, height, samples.to_vec())).collect()
    }

    /// The file `name`, holding `file`, decodes into `frames`, with
    /// `delays` and `plays`.
    fn assert_animation(name: &str, file: &[u8], frames: &[(u32, u32, &[u8])], delays: &[u32], plays: u32) {
        let (timing, got) = rgba_frames(decode(Path::new(name), file, Reading::default(), None).expect("decoded"));
        assert_eq!(timing, Some(Timing { delays: delays.to_vec(), loops: plays }), "{name}");
        assert_eq!(got, reference_frames(frames), "{name}");
    }

    /// An animated GIF's and an APNG's frames as Pillow composites them,
    /// through every disposal and both blends, with their delays and the
    /// times each plays.
    #[test]
    fn composites_gif_and_apng_frames_as_pillow_does() {
        use crate::frames_reference as r;
        assert_animation("animated.gif", &r::GIF_FILE, &r::GIF_FRAMES, &r::GIF_DELAYS, r::GIF_PLAYS);
        assert_animation("animated.png", &r::APNG_FILE, &r::APNG_FRAMES, &r::APNG_DELAYS, r::APNG_PLAYS);
    }

    /// An APNG whose second frame clears its area and whose third puts its
    /// area back composites as Pillow does, and a 16-bit APNG composites at
    /// 16 bits as ffmpeg does, with its delays and the times it plays.
    #[test]
    fn composites_apng_frames_put_back_and_at_sixteen_bits() {
        use crate::frames_reference as r;
        assert_animation("putback.png", &r::APNG_PUT_BACK_FILE, &r::APNG_PUT_BACK_FRAMES, &r::APNG_PUT_BACK_DELAYS, 0);
        let chosen = decode(Path::new("animated16.png"), &r::APNG16_FILE, Reading::default(), None).expect("decoded");
        assert_eq!(chosen.timing, Some(Timing { delays: r::APNG16_DELAYS.to_vec(), loops: 2 }));
        let mut frames: Vec<Frame> = chosen.parts.into_iter().flat_map(|part| part.frames).collect();
        frames.sort_by_key(|frame| frame.place);
        let got: Vec<(u32, u32, Vec<u16>)> = frames
            .into_iter()
            .map(|frame| match frame.image {
                DynamicImage::ImageRgba16(image) => (image.width(), image.height(), image.into_raw()),
                other => panic!("a frame decoded as {:?}, not 16-bit RGBA", other.color()),
            })
            .collect();
        let wanted: Vec<(u32, u32, Vec<u16>)> = r::APNG16_FRAMES.iter().map(|&(width, height, samples)| (width, height, samples.to_vec())).collect();
        assert_eq!(got, wanted);
    }

    /// A lossless animated WebP's frames and a still lossy WebP's pixels as
    /// Pillow decodes them through libwebp, with the animation's delays and
    /// the times it plays.
    #[cfg(feature = "webp")]
    #[test]
    fn decodes_webp_files_as_libwebp_does() {
        use crate::frames_reference as r;
        assert_animation("animated.webp", &r::WEBP_FILE, &r::WEBP_FRAMES, &r::WEBP_DELAYS, r::WEBP_PLAYS);
        let (timing, got) = rgba_frames(decode(Path::new("lossy.webp"), &r::WEBP_LOSSY_FILE, Reading::default(), None).expect("decoded"));
        assert_eq!(timing, None);
        assert_eq!(got, reference_frames(&r::WEBP_LOSSY_PIXELS));
    }

    /// A GIF plays one time more than its NETSCAPE2.0 repeat count, and
    /// once without one.
    #[test]
    fn counts_a_gifs_plays_from_its_repeat_count() {
        use crate::frames_reference as r;
        for (name, file, plays) in [("twice.gif", &r::GIF_TWICE_FILE[..], r::GIF_TWICE_PLAYS), ("once.gif", &r::GIF_ONCE_FILE[..], r::GIF_ONCE_PLAYS)] {
            let (timing, _) = rgba_frames(decode(Path::new(name), file, Reading::default(), None).expect("decoded"));
            assert_eq!(timing.map(|timing| timing.loops), Some(plays), "{name}");
        }
    }

    /// -Frame picks an animation's frames in its order, each with its own
    /// delay.
    #[test]
    fn picks_an_animations_frames_with_their_delays() {
        use crate::frames_reference as r;
        let (timing, got) = rgba_frames(decode(Path::new("animated.gif"), &r::GIF_FILE, Reading::default(), Some(&[2, 0])).expect("decoded"));
        assert_eq!(timing, Some(Timing { delays: vec![r::GIF_DELAYS[2], r::GIF_DELAYS[0]], loops: r::GIF_PLAYS }));
        assert_eq!(got, reference_frames(&[r::GIF_FRAMES[2], r::GIF_FRAMES[0]]));
    }

    /// The pages of a TIFF Pillow writes, of different sizes, as Pillow
    /// decodes them.
    #[test]
    fn reads_the_pages_pillow_writes_as_pillow_does() {
        use crate::frames_reference as r;
        let (timing, got) = rgba_frames(decode(Path::new("pages.tif"), &r::TIFF_FILE, Reading::default(), None).expect("decoded"));
        assert_eq!(timing, None);
        assert_eq!(got, reference_frames(&r::TIFF_PAGES));
    }

    /// A JPEG XL animation's frames as libjxl composites them, with the
    /// delays it was encoded at.
    #[cfg(feature = "jxl")]
    #[test]
    fn composites_a_jpeg_xl_animation_as_libjxl_does() {
        use crate::frames_reference as r;
        let (timing, got) = rgba_frames(decode(Path::new("animated.jxl"), &r::JXL_FILE, Reading::default(), None).expect("decoded"));
        assert_eq!(timing.map(|timing| timing.delays), Some(r::JXL_DELAYS.to_vec()));
        assert_eq!(got, reference_frames(&r::JXL_FRAMES));
    }

    #[test]
    fn exif_orientation_is_read_with_or_without_the_exif_header() {
        // A little-endian TIFF block whose one IFD entry is Orientation (274), a SHORT of 6.
        let tiff = [b'I', b'I', 42, 0, 8, 0, 0, 0, 1, 0, 0x12, 0x01, 3, 0, 1, 0, 0, 0, 6, 0, 0, 0, 0, 0, 0, 0];
        assert_eq!(orientation_in(&tiff), Orientation::Rotate90);
        assert_eq!(orientation_in(&[b"Exif\0\0".as_slice(), &tiff].concat()), Orientation::Rotate90);
        assert_eq!(orientation_in(b"II*\0\x08\0\0\0\0\0"), Orientation::NoTransforms);
    }
}
