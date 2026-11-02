//! Size and orientation: a texture cropped, rotated, flipped, resized with
//! a filter and padded on its device, in that order whatever order a call
//! names them in. Every layer, and every depth slice of a 3D texture, is
//! drawn as a 2D image by render passes in 32-bit floats: one that crops,
//! rotates and flips by moving texels, one for each axis whose size
//! changes, down before across, and one that pads. The last pass draws in
//! the float format the texture's texels render in; mip levels are
//! rendered again from level 0, and the texels are packed or encoded back
//! into the texture's format.

use pwrs::prelude::*;
use wgpu::util::DeviceExt;
use wgpu::{AstcBlock, AstcChannel, TextureFormat as F};

use crate::alpha::{self, Border};
use crate::basis::{self, target};
use crate::color::{self, Color, ColorSpace, Prepared, ToneMapCurve};
use crate::convert::{self, MipEdge, MipFilter};
use crate::device::CoreRef;
use crate::error::invalid;
use crate::filter::{self, EdgeOperator, FilterEdge};
use crate::image_io::{self, BasisFormat, BASIS_FORMATS};
use crate::normal::{self, HeightChannel, HeightEdge, HeightToNormal, NormalAlpha, NormalKernel, NormalMips};
use crate::tasks::Interrupt;
use crate::texel;
use crate::texfile::{self, MipsMade, Said, TextureFile};
use crate::texture::{format_name, tell, Decoder, Texture};

/// How a resize computes each new texel from the texels around where it
/// lands.
#[psenum(name = "Wgpu.ResizeFilter")]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum ResizeFilter {
    /// The texel the new texel's center lands in.
    Nearest,
    /// The average of the texels the new texel covers, each weighed by how
    /// much of it the new texel covers, as mip levels are made.
    #[default]
    Box,
    /// A triangle filter: two texels wide when enlarging, wider when
    /// shrinking.
    Bilinear,
    /// Catmull-Rom, the cubic that passes through the texels' values: four
    /// texels wide when enlarging, wider when shrinking.
    Bicubic,
    /// Lanczos with three lobes: six texels wide when enlarging, wider
    /// when shrinking.
    Lanczos,
}

impl ResizeFilter {
    /// The number the shader knows the filter by.
    fn code(self) -> u32 {
        match self {
            ResizeFilter::Nearest => 0,
            ResizeFilter::Box => 1,
            ResizeFilter::Bilinear => 2,
            ResizeFilter::Bicubic => 3,
            ResizeFilter::Lanczos => 4,
        }
    }

    /// How far the filter reaches each way, in texels of the larger image.
    fn support(self) -> f32 {
        match self {
            ResizeFilter::Nearest | ResizeFilter::Box => 0.0,
            ResizeFilter::Bilinear => 1.0,
            ResizeFilter::Bicubic => 2.0,
            ResizeFilter::Lanczos => 3.0,
        }
    }
}

/// How each side of a size rounds to a power of two.
#[psenum(name = "Wgpu.PowerOfTwo")]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum PowerOfTwo {
    /// The nearer power of two; the larger when both are as near.
    #[default]
    Nearest,
    /// The power of two at or above the side.
    Up,
    /// The power of two at or below the side.
    Down,
}

impl PowerOfTwo {
    /// `side` rounded to a power of two, or `None` past the largest a
    /// 32-bit size holds.
    fn round(self, side: u32) -> Option<u32> {
        let up = side.checked_next_power_of_two()?;
        let down = if up == side { side } else { up >> 1 };
        Some(match self {
            PowerOfTwo::Up => up,
            PowerOfTwo::Down => down,
            PowerOfTwo::Nearest => {
                if side - down < up - side {
                    down
                } else {
                    up
                }
            }
        })
    }
}

/// Where a crop to a size, or a texture on a larger canvas, is placed.
#[psenum(name = "Wgpu.Anchor")]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Anchor {
    /// The top left corner.
    TopLeft,
    /// The middle of the top edge.
    Top,
    /// The top right corner.
    TopRight,
    /// The middle of the left edge.
    Left,
    /// The middle.
    #[default]
    Center,
    /// The middle of the right edge.
    Right,
    /// The bottom left corner.
    BottomLeft,
    /// The middle of the bottom edge.
    Bottom,
    /// The bottom right corner.
    BottomRight,
}

impl Anchor {
    /// The anchor's place along each axis, in halves of the room left:
    /// 0 at the left or top, 1 in the middle, 2 at the right or bottom.
    fn halves(self) -> (u32, u32) {
        match self {
            Anchor::TopLeft => (0, 0),
            Anchor::Top => (1, 0),
            Anchor::TopRight => (2, 0),
            Anchor::Left => (0, 1),
            Anchor::Center => (1, 1),
            Anchor::Right => (2, 1),
            Anchor::BottomLeft => (0, 2),
            Anchor::Bottom => (1, 2),
            Anchor::BottomRight => (2, 2),
        }
    }

    /// Where something of `inner` size starts inside `outer`, which holds
    /// it: the room left on each axis split as the anchor says, the odd
    /// texel of a middle going after it.
    fn place(self, outer: (u32, u32), inner: (u32, u32)) -> (u32, u32) {
        let (hx, hy) = self.halves();
        ((outer.0 - inner.0) * hx / 2, (outer.1 - inner.1) * hy / 2)
    }
}

/// Which way a flip mirrors a texture.
#[psenum(name = "Wgpu.FlipDirection")]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum FlipDirection {
    /// Left and right swap.
    #[default]
    Horizontal,
    /// Top and bottom swap.
    Vertical,
}

/// The part of a texture a crop keeps.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Crop {
    /// A region: its left, top, width and height.
    Region { x: u32, y: u32, width: u32, height: u32 },
    /// A width and height, placed by the anchor.
    Size { width: u32, height: u32 },
    /// The texels cut from the left, top, right and bottom.
    Margins { left: u32, top: u32, right: u32, bottom: u32 },
}

impl Crop {
    /// The crop -Crop names: four numbers for a region, two for a size.
    pub(crate) fn of(values: &[u32]) -> PsResult<Crop> {
        match *values {
            [x, y, width, height] => Ok(Crop::Region { x, y, width, height }),
            [width, height] => Ok(Crop::Size { width, height }),
            _ => Err(invalid(
                "WgpuCrop",
                format!("-Crop takes four numbers, the left, top, width and height of a region, or two, a width and height the anchor places; {} were given", values.len()),
            )),
        }
    }

    /// The crop -CropMargin names: the texels cut from each side.
    pub(crate) fn margins(values: &[u32]) -> PsResult<Crop> {
        match *values {
            [left, top, right, bottom] => Ok(Crop::Margins { left, top, right, bottom }),
            _ => Err(invalid("WgpuCrop", format!("-CropMargin takes four numbers, the texels cut from the left, top, right and bottom; {} were given", values.len()))),
        }
    }
}

/// The parameters every cmdlet that transforms a texture takes, as they
/// were given.
#[derive(Default)]
pub(crate) struct Asked<'a> {
    pub(crate) width: Option<u32>,
    pub(crate) height: Option<u32>,
    pub(crate) scale: Option<f64>,
    pub(crate) max_size: Option<u32>,
    pub(crate) power_of_two: Option<PowerOfTwo>,
    pub(crate) filter: Option<ResizeFilter>,
    pub(crate) flip: Option<FlipDirection>,
    pub(crate) rotate: Option<u32>,
    pub(crate) crop: Option<&'a [u32]>,
    pub(crate) crop_margin: Option<&'a [u32]>,
    pub(crate) pad: Option<&'a [u32]>,
    pub(crate) anchor: Option<Anchor>,
    pub(crate) pad_color: Option<&'a [f64]>,
    pub(crate) swizzle: Option<&'a str>,
    pub(crate) reconstruct_z: bool,
    pub(crate) normalize: bool,
    pub(crate) flip_green: bool,
    /// -HeightToNormal's strength, and the settings that say how it makes
    /// normals of the heights.
    pub(crate) height_to_normal: Option<f64>,
    pub(crate) height_channel: Option<HeightChannel>,
    pub(crate) normal_kernel: Option<NormalKernel>,
    pub(crate) height_edge: Option<HeightEdge>,
    pub(crate) normal_alpha: Option<NormalAlpha>,
    pub(crate) premultiply: bool,
    pub(crate) unpremultiply: bool,
    pub(crate) color_space: Option<ColorSpace>,
    pub(crate) assign_color_space: Option<ColorSpace>,
    pub(crate) grayscale: bool,
    pub(crate) grayscale_weights: Option<&'a PsObject>,
    pub(crate) invert: bool,
    pub(crate) brightness: Option<f64>,
    pub(crate) contrast: Option<f64>,
    pub(crate) gamma: Option<f64>,
    pub(crate) exposure: Option<f64>,
    pub(crate) tone_map: Option<ToneMapCurve>,
    pub(crate) color_key: Option<&'a PsObject>,
    pub(crate) color_key_tolerance: Option<f64>,
    pub(crate) alpha_threshold: Option<f64>,
    pub(crate) fix_alpha_border: bool,
    pub(crate) border_reach: Option<u32>,
    /// The blur, the unsharp mask and the edges, and what they read past
    /// the edge.
    pub(crate) filters: filter::Asked<'a>,
}

impl Asked<'_> {
    /// The transform the parameters ask for.
    pub(crate) fn transform(&self) -> PsResult<Transform> {
        let crop = match (self.crop, self.crop_margin) {
            (Some(_), Some(_)) => return Err(invalid("WgpuCrop", "give -Crop or -CropMargin, not both")),
            (Some(values), None) => Some(Crop::of(values)?),
            (None, Some(values)) => Some(Crop::margins(values)?),
            (None, None) => None,
        };
        let normal = match self.height_to_normal {
            Some(_) if self.reconstruct_z || self.normalize => {
                return Err(invalid(
                    "WgpuNormalMap",
                    "-ReconstructZ and -Normalize change normals a texture holds, and -HeightToNormal makes normals from heights, each with its Z and of length 1: give one or the other",
                ));
            }
            Some(strength) => Some(HeightToNormal::new(strength, "-HeightToNormal", self.height_channel, self.normal_kernel, self.height_edge, self.normal_alpha, self.flip_green)?),
            None if self.height_channel.is_some() || self.normal_kernel.is_some() || self.height_edge.is_some() || self.normal_alpha.is_some() => {
                return Err(invalid("WgpuNormalMap", "-HeightChannel, -NormalKernel, -HeightEdge and -NormalAlpha say how -HeightToNormal makes normals from heights: give -HeightToNormal"));
            }
            None => None,
        };
        let color = Color {
            assign: self.assign_color_space,
            key: color::key_of(self.color_key, self.color_key_tolerance)?,
            swizzle: self.swizzle.map(color::swizzle_of).transpose()?,
            reconstruct_z: self.reconstruct_z,
            normalize: self.normalize,
            // With -HeightToNormal, green flips in the normals it makes.
            flip_green: self.flip_green && normal.is_none(),
            unpremultiply: self.unpremultiply,
            threshold: color::threshold_of(self.alpha_threshold)?,
            exposure: color::exposure_of(self.exposure)?,
            tone_map: self.tone_map,
            brightness: color::percent_of(self.brightness, "-Brightness")?,
            contrast: color::percent_of(self.contrast, "-Contrast")?,
            gamma: color::gamma_of(self.gamma)?,
            grayscale: color::gray_of(self.grayscale, self.grayscale_weights)?,
            invert: self.invert,
            premultiply: self.premultiply,
            space: self.color_space,
        };
        Ok(Transform {
            color,
            crop,
            rotate: self.rotate,
            flip: self.flip,
            width: self.width,
            height: self.height,
            scale: self.scale,
            max_size: self.max_size,
            power_of_two: self.power_of_two,
            filter: self.filter,
            pad: self.pad.map(pad_of).transpose()?,
            anchor: self.anchor,
            pad_color: self.pad_color.map(color_of).transpose()?,
            filters: self.filters.filters()?,
            border: alpha::border_of(self.fix_alpha_border, self.border_reach)?,
            normal,
            target: None,
        })
    }
}

/// The canvas -Pad names: a width and a height.
pub(crate) fn pad_of(values: &[u32]) -> PsResult<(u32, u32)> {
    match *values {
        [width, height] => Ok((width, height)),
        _ => Err(invalid("WgpuPad", format!("-Pad takes two numbers, the width and height of the canvas; {} were given", values.len()))),
    }
}

/// The color -PadColor names: red, green, blue and alpha, or red, green
/// and blue with full alpha.
pub(crate) fn color_of(values: &[f64]) -> PsResult<[f32; 4]> {
    match *values {
        [r, g, b] => Ok([r as f32, g as f32, b as f32, 1.0]),
        [r, g, b, a] => Ok([r as f32, g as f32, b as f32, a as f32]),
        _ => Err(invalid("WgpuPad", format!("a pad color is three or four numbers, red, green, blue and alpha; {} were given", values.len()))),
    }
}

/// The changes one call makes to a texture's size and orientation, applied
/// in the order the fields are listed: crop, rotate, flip, resize, pad;
/// then the changes to its texels' channels and colors, the filters, the
/// border color, and heights made into normals.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct Transform {
    pub(crate) crop: Option<Crop>,
    /// Degrees clockwise: 90, 180 or 270.
    pub(crate) rotate: Option<u32>,
    pub(crate) flip: Option<FlipDirection>,
    pub(crate) width: Option<u32>,
    pub(crate) height: Option<u32>,
    pub(crate) scale: Option<f64>,
    pub(crate) max_size: Option<u32>,
    pub(crate) power_of_two: Option<PowerOfTwo>,
    pub(crate) filter: Option<ResizeFilter>,
    /// The width and height of the canvas padding grows the texture to.
    pub(crate) pad: Option<(u32, u32)>,
    pub(crate) anchor: Option<Anchor>,
    pub(crate) pad_color: Option<[f32; 4]>,
    pub(crate) color: Color,
    /// The blur, the unsharp mask and the edges, over the texels the color
    /// pass leaves.
    pub(crate) filters: Option<filter::Filters>,
    /// Color carried into transparent texels after the color pass and the
    /// filters.
    pub(crate) border: Option<Border>,
    /// Heights made into normals, last.
    pub(crate) normal: Option<HeightToNormal>,
    /// The format a conversion after the transform stores the texels in,
    /// which says how normals the transform writes are stored; the
    /// texture's own, or the format its texels decode into, when `None`.
    pub(crate) target: Option<F>,
}

/// The integer map from a texel of the cropped, rotated and flipped image
/// to the source texel it shows: `base + x * step_x + y * step_y`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Map {
    base: [i32; 2],
    step_x: [i32; 2],
    step_y: [i32; 2],
}

impl Map {
    /// The source texel output texel (`x`, `y`) shows.
    #[cfg(test)]
    fn at(&self, x: u32, y: u32) -> (u32, u32) {
        let (x, y) = (x as i32, y as i32);
        ((self.base[0] + x * self.step_x[0] + y * self.step_y[0]) as u32, (self.base[1] + x * self.step_x[1] + y * self.step_y[1]) as u32)
    }
}

/// What a transform does to a texture of one size, worked out before any
/// GPU work.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Plan {
    map: Map,
    /// The size after cropping.
    cropped: (u32, u32),
    /// The size after cropping, rotating and flipping.
    oriented: (u32, u32),
    /// The size after resizing.
    resized: (u32, u32),
    /// The filter across and the filter down; `None` where the size stays.
    filters: (Option<ResizeFilter>, Option<ResizeFilter>),
    /// The size after padding, where the image's top left lands on it, and
    /// the color around it.
    canvas: (u32, u32),
    offset: (u32, u32),
    color: [f32; 4],
}

impl Plan {
    /// The size the transform gives.
    pub(crate) fn size(&self) -> (u32, u32) {
        self.canvas
    }

    /// Whether the padding leaves any texel of the canvas the image does
    /// not cover.
    fn pads(&self) -> bool {
        self.canvas != self.resized
    }

    /// The size the crop, rotation, flip and resize give, before the
    /// padding, and the map `[a, b, c, d, e, f]` that takes a point (x, y)
    /// of the source, in texels from its top left, to (a x + c y + e,
    /// b x + d y + f) of that image. A vector image drawn through the map
    /// at that size is its drawing at its own size with those steps done,
    /// and no resampling.
    #[cfg(any(feature = "svg", feature = "pdf", test))]
    pub(crate) fn drawn(&self) -> ((u32, u32), [f64; 6]) {
        let Map { base, step_x, step_y } = self.map;
        let (sx, sy) = (step_x.map(f64::from), step_y.map(f64::from));
        // A point p of the oriented image shows the source's point M p + c,
        // M's columns the steps and c = base + (1 - step_x - step_y) / 2.
        // M is a quarter turn or a mirror, so its inverse is its transpose.
        let c = [f64::from(base[0]) + (1.0 - sx[0] - sy[0]) / 2.0, f64::from(base[1]) + (1.0 - sx[1] - sy[1]) / 2.0];
        let across = f64::from(self.resized.0) / f64::from(self.oriented.0);
        let down = f64::from(self.resized.1) / f64::from(self.oriented.1);
        let map = [across * sx[0], down * sy[0], across * sx[1], down * sy[1], -across * (sx[0] * c[0] + sx[1] * c[1]), -down * (sy[0] * c[0] + sy[1] * c[1])];
        (self.resized, map)
    }
}

/// `value` rounded to a whole number of texels, at least 1.
fn side(value: f64) -> u32 {
    value.round().clamp(1.0, f64::from(u32::MAX)) as u32
}

/// The map that undoes a flip, then a rotation by `turns` quarter turns
/// clockwise, then a crop at (`x`, `y`) of `width` by `height`: what a
/// texel of the oriented image of size `oriented` shows of the source.
fn orient_map((x, y, width, height): (u32, u32, u32, u32), turns: u32, flip: Option<FlipDirection>, oriented: (u32, u32)) -> Map {
    let (cx, cy, cw, ch) = (x as i32, y as i32, width as i32, height as i32);
    let (ow, oh) = (oriented.0 as i32, oriented.1 as i32);
    // The flip undone: a texel of the rotated image is (fx * x + bx, fy * y + by).
    let (fx, fy, bx, by) = match flip {
        Some(FlipDirection::Horizontal) => (-1, 1, ow - 1, 0),
        Some(FlipDirection::Vertical) => (1, -1, 0, oh - 1),
        None => (1, 1, 0, 0),
    };
    // The rotation undone: the cropped texel of a rotated one is r * it + t.
    let (r, t) = match turns {
        0 => ([[1, 0], [0, 1]], [0, 0]),
        1 => ([[0, 1], [-1, 0]], [0, ch - 1]),
        2 => ([[-1, 0], [0, -1]], [cw - 1, ch - 1]),
        _ => ([[0, -1], [1, 0]], [cw - 1, 0]),
    };
    Map {
        base: [r[0][0] * bx + r[0][1] * by + t[0] + cx, r[1][0] * bx + r[1][1] * by + t[1] + cy],
        step_x: [r[0][0] * fx, r[1][0] * fx],
        step_y: [r[0][1] * fy, r[1][1] * fy],
    }
}

impl Transform {
    /// Whether the transform changes nothing; the format a conversion
    /// after it stores the texels in is no change of its own.
    pub(crate) fn is_empty(&self) -> bool {
        Transform { target: None, ..*self } == Transform::default()
    }

    /// Whether the transform writes normals: makes them from heights, or
    /// changes those the texture holds.
    pub(crate) fn writes_normals(&self) -> bool {
        self.normal.is_some() || self.color.writes_normals()
    }

    /// Whether the transform changes the size: a new size, a scale, a size
    /// limit or a power of two.
    fn resizes(&self) -> bool {
        self.width.is_some() || self.height.is_some() || self.scale.is_some() || self.max_size.is_some() || self.power_of_two.is_some()
    }

    /// What is left of the transform once a vector image is drawn through
    /// its plan's `drawn` map, which does the crop, rotation, flip and
    /// resize: the padding and every change after it, with the -Anchor
    /// kept where it places the padding.
    pub(crate) fn after_drawing(&self) -> Transform {
        Transform {
            crop: None,
            rotate: None,
            flip: None,
            width: None,
            height: None,
            scale: None,
            max_size: None,
            power_of_two: None,
            filter: None,
            anchor: self.pad.and(self.anchor),
            ..*self
        }
    }

    /// What the transform does to a texture of `width` by `height` texels
    /// of `dimension`, a cube map when `cube`, or why it cannot.
    pub(crate) fn plan(&self, width: u32, height: u32, dimension: wgpu::TextureDimension, cube: bool) -> PsResult<Plan> {
        let line = dimension == wgpu::TextureDimension::D1;
        if self.filter.is_some() && !self.resizes() {
            return Err(invalid("WgpuResize", "-Filter applies with a new size: -Width, -Height, -Scale, -MaxSize or -PowerOfTwo"));
        }
        if self.scale.is_some() && (self.width.is_some() || self.height.is_some()) {
            return Err(invalid("WgpuResize", "give -Scale, or -Width and -Height, not both"));
        }
        if let Some(scale) = self.scale
            && !(scale.is_finite() && scale > 0.0)
        {
            return Err(invalid("WgpuResize", format!("-Scale is a number above 0, not {scale}")));
        }
        if self.width == Some(0) || self.height == Some(0) || self.max_size == Some(0) {
            return Err(invalid("WgpuResize", "a width, height or size limit is at least 1 texel"));
        }
        if self.anchor.is_some() && !matches!(self.crop, Some(Crop::Size { .. })) && self.pad.is_none() {
            return Err(invalid("WgpuTransform", "-Anchor places a crop to a size or the padding: give -Crop with a width and height, or -Pad"));
        }
        if self.pad_color.is_some() && self.pad.is_none() {
            return Err(invalid("WgpuPad", "-PadColor fills the padding: give -Pad"));
        }
        if self.border.is_some() && self.color.premultiply {
            return Err(invalid("WgpuBorder", "-FixAlphaBorder colors texels with alpha 0, and a premultiplied texel with alpha 0 stays black: give -FixAlphaBorder or -Premultiply, not both"));
        }
        if cube && (self.rotate.is_some() || self.flip.is_some()) {
            return Err(invalid("WgpuTransform", "a cube map is not flipped or rotated, as its faces would no longer meet at their edges"));
        }
        if self.normal.is_some() {
            let refused = match dimension {
                _ if cube => Some("a cube map's faces meet at their edges, which each face read alone as a height map would not see"),
                wgpu::TextureDimension::D1 => Some("a 1D texture is one texel high, with no slope down it"),
                wgpu::TextureDimension::D3 => Some("a 3D texture's depth slices are no single height map"),
                wgpu::TextureDimension::D2 => None,
            };
            if let Some(why) = refused {
                return Err(invalid("WgpuNormalMap", format!("normals are made from the heights of a 2D texture, or of each layer of a 2D array, and {why}")));
            }
        }
        let anchor = self.anchor.unwrap_or(Anchor::Center);
        let square = |change: &str, w: u32, h: u32| {
            if cube && w != h {
                return Err(invalid("WgpuTransform", format!("a cube map's faces stay square, and {change} {w}x{h} is not")));
            }
            Ok(())
        };
        let one_high = |parameter: &str, h: u32| {
            if line && h != 1 {
                return Err(invalid("WgpuTransform", format!("a 1D texture stays one texel high, and {parameter} makes it {h} texels high")));
            }
            Ok(())
        };

        let crop = match self.crop {
            None => (0, 0, width, height),
            Some(Crop::Region { x, y, width: w, height: h }) => {
                let inside = w > 0 && h > 0 && x.checked_add(w).is_some_and(|end| end <= width) && y.checked_add(h).is_some_and(|end| end <= height);
                if !inside {
                    return Err(invalid("WgpuCrop", format!("the region at {x},{y} of {w}x{h} texels is not inside the {width}x{height} texture")));
                }
                (x, y, w, h)
            }
            Some(Crop::Size { width: w, height: h }) => {
                if w == 0 || h == 0 || w > width || h > height {
                    return Err(invalid("WgpuCrop", format!("a crop to {w}x{h} texels does not fit inside the {width}x{height} texture")));
                }
                let (x, y) = anchor.place((width, height), (w, h));
                (x, y, w, h)
            }
            Some(Crop::Margins { left, top, right, bottom }) => {
                let across = left.checked_add(right).filter(|&cut| cut < width);
                let down = top.checked_add(bottom).filter(|&cut| cut < height);
                match (across, down) {
                    (Some(across), Some(down)) => (left, top, width - across, height - down),
                    _ => return Err(invalid("WgpuCrop", format!("margins of {left}, {top}, {right} and {bottom} texels leave nothing of the {width}x{height} texture"))),
                }
            }
        };
        square("a crop to", crop.2, crop.3)?;

        let turns = match self.rotate {
            None => 0,
            Some(90) => 1,
            Some(180) => 2,
            Some(270) => 3,
            Some(other) => return Err(invalid("WgpuRotate", format!("a rotation is 90, 180 or 270 degrees clockwise, not {other}"))),
        };
        if line && turns % 2 == 1 {
            return Err(invalid("WgpuTransform", "a 1D texture stays one texel high, so it is not rotated by 90 or 270 degrees"));
        }
        let oriented = if turns % 2 == 1 { (crop.3, crop.2) } else { (crop.2, crop.3) };
        let map = orient_map(crop, turns, self.flip, oriented);

        let (ow, oh) = oriented;
        if let Some(h) = self.height {
            one_high("-Height", h)?;
        }
        // A 1D texture's height stays 1 however its width changes.
        let tall = |h: f64| if line { 1 } else { side(h) };
        let mut resized = match (self.width, self.height, self.scale) {
            (Some(w), Some(h), _) => (w, h),
            (Some(w), None, _) => (w, tall(f64::from(oh) * f64::from(w) / f64::from(ow))),
            (None, Some(h), _) => (side(f64::from(ow) * f64::from(h) / f64::from(oh)), h),
            (None, None, Some(scale)) => (side(f64::from(ow) * scale), tall(f64::from(oh) * scale)),
            (None, None, None) => oriented,
        };
        if let Some(limit) = self.max_size {
            let largest = resized.0.max(resized.1);
            if largest > limit {
                let shrink = f64::from(limit) / f64::from(largest);
                resized = (side(f64::from(resized.0) * shrink), tall(f64::from(resized.1) * shrink));
            }
        }
        if let Some(rule) = self.power_of_two {
            let round = |v: u32| rule.round(v).ok_or_else(|| invalid("WgpuResize", format!("{v} texels has no power of two a texture's size holds above it")));
            resized = (round(resized.0)?, if line { 1 } else { round(resized.1)? });
        }
        square("a resize to", resized.0, resized.1)?;
        let pick = |from: u32, to: u32| (from != to).then(|| self.filter.unwrap_or(if to < from { ResizeFilter::Box } else { ResizeFilter::Bicubic }));
        let filters = (pick(ow, resized.0), pick(oh, resized.1));

        let (canvas, offset) = match self.pad {
            None => (resized, (0, 0)),
            Some((pw, ph)) => {
                if pw < resized.0 || ph < resized.1 {
                    return Err(invalid("WgpuPad", format!("-Pad grows the canvas, and {pw}x{ph} is smaller than the {}x{} texture it pads", resized.0, resized.1)));
                }
                one_high("-Pad", ph)?;
                square("a canvas of", pw, ph)?;
                ((pw, ph), anchor.place((pw, ph), resized))
            }
        };
        Ok(Plan { map, cropped: (crop.2, crop.3), oriented, resized, filters, canvas, offset, color: self.pad_color.unwrap_or([0.0; 4]) })
    }

    /// What a note says the transform did, from `plan`, rendered in
    /// `render`.
    fn describe(&self, plan: &Plan, render: F) -> String {
        let mut done = Vec::new();
        if self.crop.is_some() {
            done.push(format!("cropped to {}x{}", plan.cropped.0, plan.cropped.1));
        }
        if let Some(degrees) = self.rotate {
            done.push(format!("rotated {degrees} degrees clockwise"));
        }
        if let Some(flip) = self.flip {
            done.push(format!("flipped {}", if flip == FlipDirection::Horizontal { "left to right" } else { "top to bottom" }));
        }
        if plan.resized != plan.oriented {
            let name = |f: Option<ResizeFilter>| f.map_or_else(String::new, |f| format!("{f:?}"));
            let filters = match plan.filters {
                (Some(a), Some(d)) if a == d => format!("the {a:?} filter"),
                (Some(_), Some(_)) => format!("the {} filter across and {} down", name(plan.filters.0), name(plan.filters.1)),
                (Some(a), None) | (None, Some(a)) => format!("the {a:?} filter"),
                (None, None) => String::new(),
            };
            done.push(format!("resized to {}x{} with {filters}", plan.resized.0, plan.resized.1));
        }
        if plan.pads() {
            done.push(format!("padded to {}x{}", plan.canvas.0, plan.canvas.1));
        }
        let list = if done.is_empty() { "copied".to_string() } else { listed(&done) };
        format!("the texture was {list} on the GPU in {}", format_name(render))
    }
}

/// `done` as one phrase: its items joined with commas, the last with and.
fn listed(done: &[String]) -> String {
    match done {
        [] => String::new(),
        [one] => one.clone(),
        [rest @ .., last] => format!("{} and {last}", rest.join(", ")),
    }
}

/// The uniforms every pass reads, laid out as the shader's `Params`: 140
/// bytes with no gaps, 4 that align `key` to 16, 20 more, and 12 that
/// round the struct to its alignment, 176 in all.
#[derive(Clone, Copy, Default)]
struct Params {
    base: [i32; 2],
    step_x: [i32; 2],
    step_y: [i32; 2],
    slice: u32,
    axis: u32,
    kernel: u32,
    ratio: f32,
    support: f32,
    scale: f32,
    offset: [i32; 2],
    size: [u32; 2],
    color: [f32; 4],
    swizzle: [u32; 4],
    weights: [f32; 4],
    flags: u32,
    curve: u32,
    exposure: f32,
    white: f32,
    slope: f32,
    intercept: f32,
    gamma: f32,
    key: [f32; 4],
    threshold: f32,
}

impl Params {
    fn bytes(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(176);
        for v in self.base.iter().chain(&self.step_x).chain(&self.step_y) {
            out.extend(v.to_le_bytes());
        }
        for v in [self.slice, self.axis, self.kernel] {
            out.extend(v.to_le_bytes());
        }
        for v in [self.ratio, self.support, self.scale] {
            out.extend(v.to_le_bytes());
        }
        for v in self.offset {
            out.extend(v.to_le_bytes());
        }
        for v in self.size {
            out.extend(v.to_le_bytes());
        }
        for v in self.color {
            out.extend(v.to_le_bytes());
        }
        for v in self.swizzle {
            out.extend(v.to_le_bytes());
        }
        for v in self.weights {
            out.extend(v.to_le_bytes());
        }
        for v in [self.flags, self.curve] {
            out.extend(v.to_le_bytes());
        }
        for v in [self.exposure, self.white, self.slope, self.intercept, self.gamma] {
            out.extend(v.to_le_bytes());
        }
        out.extend([0; 4]);
        for v in self.key.iter().chain([&self.threshold]) {
            out.extend(v.to_le_bytes());
        }
        out.extend([0; 12]);
        out
    }

    /// The uniforms of the color pass `prepared` describes.
    fn recolor(prepared: &Prepared) -> Params {
        Params {
            swizzle: prepared.swizzle,
            weights: prepared.weights,
            flags: prepared.flags,
            curve: prepared.curve,
            exposure: prepared.exposure,
            slope: prepared.slope,
            intercept: prepared.intercept,
            gamma: prepared.gamma,
            key: prepared.key,
            threshold: prepared.threshold,
            ..Default::default()
        }
    }
}

/// A pass's fragment entry point.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Entry {
    Orient,
    Resample,
    Place,
    Recolor,
}

impl Entry {
    fn name(self) -> &'static str {
        match self {
            Entry::Orient => "orient",
            Entry::Resample => "resample",
            Entry::Place => "place",
            Entry::Recolor => "recolor",
        }
    }
}

/// One render pass of a transform: what it draws, with what, and at what
/// size.
struct Step {
    entry: Entry,
    params: Params,
    size: (u32, u32),
}

/// The uniforms of a pass that resizes along `axis`, 0 across and 1 down,
/// from `from` texels to `to` with `filter`, in the image crate's terms: a
/// new texel's center lands at (i + 0.5) x ratio, and when shrinking the
/// filter widens by the ratio.
fn resample(axis: u32, from: u32, to: u32, filter: ResizeFilter) -> Params {
    let ratio = from as f32 / to as f32;
    let scale = ratio.max(1.0);
    Params { axis, kernel: filter.code(), ratio, support: filter.support() * scale, scale, ..Default::default() }
}

/// The passes `plan` takes: orient, which also turns a layer or a depth
/// slice into a 2D image; a resize down, then one across, where the size
/// changes; the padding; and the color pass `recolor` holds the uniforms
/// of, when there is one.
fn steps(plan: &Plan, recolor: Option<Params>) -> Vec<Step> {
    let map = plan.map;
    let mut steps = vec![Step { entry: Entry::Orient, params: Params { base: map.base, step_x: map.step_x, step_y: map.step_y, ..Default::default() }, size: plan.oriented }];
    let mut size = plan.oriented;
    if let Some(filter) = plan.filters.1 {
        size.1 = plan.resized.1;
        steps.push(Step { entry: Entry::Resample, params: resample(1, plan.oriented.1, plan.resized.1, filter), size });
    }
    if let Some(filter) = plan.filters.0 {
        size.0 = plan.resized.0;
        steps.push(Step { entry: Entry::Resample, params: resample(0, plan.oriented.0, plan.resized.0, filter), size });
    }
    if plan.pads() {
        let params = Params { offset: [plan.offset.0 as i32, plan.offset.1 as i32], size: [plan.resized.0, plan.resized.1], color: plan.color, ..Default::default() };
        steps.push(Step { entry: Entry::Place, params, size: plan.canvas });
    }
    if let Some(params) = recolor {
        steps.push(Step { entry: Entry::Recolor, params, size: plan.canvas });
    }
    steps
}

/// The shader's parts every source shares: the uniforms, the fullscreen
/// triangle, and the filters' kernels.
const COMMON: &str = "
struct Params {
    base: vec2<i32>,
    step_x: vec2<i32>,
    step_y: vec2<i32>,
    slice: u32,
    axis: u32,
    kernel: u32,
    ratio: f32,
    support: f32,
    scale: f32,
    offset: vec2<i32>,
    size: vec2<u32>,
    color: vec4<f32>,
    swizzle: vec4<u32>,
    weights: vec4<f32>,
    flags: u32,
    curve: u32,
    exposure: f32,
    white: f32,
    slope: f32,
    intercept: f32,
    gamma: f32,
    key: vec4<f32>,
    threshold: f32,
}

@group(0) @binding(1) var<uniform> p: Params;

@vertex fn vertex(@builtin(vertex_index) index: u32) -> @builtin(position) vec4<f32> {
    let corner = vec2<f32>(f32((index << 1u) & 2u), f32(index & 2u));
    return vec4<f32>(corner.x * 2.0 - 1.0, 1.0 - corner.y * 2.0, 0.0, 1.0);
}
";

/// The filters' kernels, which resizes and the mip levels filtered from
/// level 0 share: `kernel_weight` takes a kernel's number and an offset
/// in texels.
pub(crate) const KERNELS: &str = "
// Below 1, sin(a) / a would divide the error of a GPU's sin, which is
// bounded in absolute terms, by a small number, so its series serves
// there, exact to f32.
fn sinc(t: f32) -> f32 {
    let a = t * 3.14159265358979;
    if (abs(a) < 1.0) {
        let b = a * a;
        return 1.0 - b / 6.0 * (1.0 - b / 20.0 * (1.0 - b / 42.0 * (1.0 - b / 72.0)));
    }
    return sin(a) / a;
}

// The modified Bessel function of the first kind and order 0, by its
// series; at the 4 Kaiser's window takes, 30 terms reach f32's precision.
fn bessel_i0(x: f32) -> f32 {
    let q = x * x * 0.25;
    var term = 1.0;
    var sum = 1.0;
    for (var k = 1; k < 30; k++) {
        term = term * q / f32(k * k);
        sum += term;
    }
    return sum;
}

// The weight of a texel `x` texels from where the new texel lands, by
// `kernel`: 2 the triangle; 3 Catmull-Rom, Mitchell and Netravali's cubic
// with B = 0 and C = 0.5; 4 Lanczos 3; 5 Kaiser, a sinc under Kaiser's
// window 3 texels wide with alpha 4; 6 Mitchell and Netravali's cubic
// with B and C a third.
// It returns once, after the switch, as FXC refuses a function whose
// returns are all inside one.
fn kernel_weight(kernel: u32, x: f32) -> f32 {
    let a = abs(x);
    var w = 0.0;
    switch kernel {
        case 2u: {
            w = max(0.0, 1.0 - a);
        }
        case 3u: {
            if (a < 1.0) {
                w = (9.0 * a * a * a - 15.0 * a * a + 6.0) / 6.0;
            } else if (a < 2.0) {
                w = (-3.0 * a * a * a + 15.0 * a * a - 24.0 * a + 12.0) / 6.0;
            }
        }
        case 5u: {
            if (a < 3.0) {
                let t = a / 3.0;
                w = sinc(x) * bessel_i0(4.0 * sqrt(1.0 - t * t)) / bessel_i0(4.0);
            }
        }
        case 6u: {
            if (a < 1.0) {
                w = (7.0 * a * a * a - 12.0 * a * a + 16.0 / 3.0) / 6.0;
            } else if (a < 2.0) {
                w = (-7.0 / 3.0 * a * a * a + 12.0 * a * a - 20.0 * a + 32.0 / 3.0) / 6.0;
            }
        }
        default: {
            if (a < 3.0) {
                w = sinc(x) * sinc(x / 3.0);
            }
        }
    }
    return w;
}
";

/// The passes over a 2D source: orient, resample and place.
const SHADER_2D: &str = "
@group(0) @binding(0) var source: texture_2d<f32>;

// A texel of the cropped, rotated and flipped image: the source texel the
// map names.
@fragment fn orient(@builtin(position) position: vec4<f32>) -> @location(0) vec4<f32> {
    let at = vec2<i32>(position.xy);
    let src = p.base + at.x * p.step_x + at.y * p.step_y;
    return textureLoad(source, vec2<u32>(src), 0);
}

// The weight of a texel `x` texels from where the new texel lands, in
// texels widened by the scale when shrinking, by the pass's kernel.
fn weight(x: f32) -> f32 {
    return kernel_weight(p.kernel, x);
}

// Source texel `i` along the pass's axis, in the row or column of `at`.
fn along(at: vec2<u32>, i: u32) -> vec2<u32> {
    if (p.axis == 0u) {
        return vec2<u32>(i, at.y);
    }
    return vec2<u32>(at.x, i);
}

// A texel resized along one axis: the texel its center lands in for
// Nearest; for Box the texels it covers, each weighed by how much of it is
// covered; and for the rest the texels within the filter's reach, each
// weighed by the kernel, the weights scaled to sum to 1.
@fragment fn resample(@builtin(position) position: vec4<f32>) -> @location(0) vec4<f32> {
    let at = vec2<u32>(position.xy);
    let dims = textureDimensions(source);
    let n = select(dims.y, dims.x, p.axis == 0u);
    let i = select(at.y, at.x, p.axis == 0u);
    let center = (f32(i) + 0.5) * p.ratio;
    if (p.kernel == 0u) {
        return textureLoad(source, along(at, min(u32(center), n - 1u)), 0);
    }
    var sum = vec4<f32>(0.0);
    var total = 0.0;
    if (p.kernel == 1u) {
        let low = f32(i) * p.ratio;
        let high = low + p.ratio;
        for (var j = u32(low); j < n && f32(j) < high; j++) {
            let w = min(high, f32(j + 1u)) - max(low, f32(j));
            sum += textureLoad(source, along(at, j), 0) * w;
            total += w;
        }
        return sum / total;
    }
    let left = u32(clamp(floor(center - p.support), 0.0, f32(n - 1u)));
    let right = u32(clamp(ceil(center + p.support), f32(left + 1u), f32(n)));
    let origin = center - 0.5;
    for (var j = left; j < right; j++) {
        let w = weight((f32(j) - origin) / p.scale);
        sum += textureLoad(source, along(at, j), 0) * w;
        total += w;
    }
    return sum / total;
}

// A texel of the padded canvas: the image's texel where the image is
// placed, and the pad color around it.
@fragment fn place(@builtin(position) position: vec4<f32>) -> @location(0) vec4<f32> {
    let at = vec2<i32>(position.xy) - p.offset;
    if (at.x < 0 || at.y < 0 || u32(at.x) >= p.size.x || u32(at.y) >= p.size.y) {
        return p.color;
    }
    return textureLoad(source, vec2<u32>(at), 0);
}

// A texel with the channel and color changes made.
@fragment fn recolor(@builtin(position) position: vec4<f32>) -> @location(0) vec4<f32> {
    return changed(textureLoad(source, vec2<u32>(position.xy), 0));
}
";

/// The orient pass over one depth slice of a 3D source.
const SHADER_3D: &str = "
@group(0) @binding(0) var source: texture_3d<f32>;

@fragment fn orient(@builtin(position) position: vec4<f32>) -> @location(0) vec4<f32> {
    let at = vec2<i32>(position.xy);
    let src = p.base + at.x * p.step_x + at.y * p.step_y;
    return textureLoad(source, vec3<u32>(vec2<u32>(src), p.slice), 0);
}
";

/// The shader modules and bind group layouts of a transform's passes, and
/// the pipelines built from them for each entry, source and target format.
struct Passes {
    flat: (wgpu::ShaderModule, wgpu::BindGroupLayout),
    volume: Option<(wgpu::ShaderModule, wgpu::BindGroupLayout)>,
    built: Vec<((Entry, bool, F), wgpu::RenderPipeline)>,
}

impl Passes {
    fn new(core: &CoreRef, volume: bool) -> Passes {
        let make = |code: &str, dimension: wgpu::TextureViewDimension| {
            let module = core.device.create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("Wgpu transform"),
                source: wgpu::ShaderSource::Wgsl(format!("{COMMON}{KERNELS}{}{code}", color::wgsl()).into()),
            });
            let layout = core.device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("Wgpu transform"),
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
        Passes {
            flat: make(SHADER_2D, wgpu::TextureViewDimension::D2),
            volume: volume.then(|| make(SHADER_3D, wgpu::TextureViewDimension::D3)),
            built: Vec::new(),
        }
    }

    /// The module and layout of the passes over a 2D source, or over a 3D
    /// one when `volume`.
    fn parts(&self, volume: bool) -> &(wgpu::ShaderModule, wgpu::BindGroupLayout) {
        match (&self.volume, volume) {
            (Some(parts), true) => parts,
            _ => &self.flat,
        }
    }

    /// `entry`'s pipeline over a 3D source when `volume`, drawing into
    /// `format`: one built before, or one built now and kept.
    fn pipeline(&mut self, core: &CoreRef, entry: Entry, volume: bool, format: F) -> wgpu::RenderPipeline {
        if let Some((_, built)) = self.built.iter().find(|(key, _)| *key == (entry, volume, format)) {
            return built.clone();
        }
        let (module, layout) = self.parts(volume);
        let pipeline_layout = core.device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some("Wgpu transform"), bind_group_layouts: &[Some(layout)], immediate_size: 0 });
        let pipeline = core.device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("Wgpu transform"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState { module, entry_point: Some("vertex"), compilation_options: Default::default(), buffers: &[] },
            fragment: Some(wgpu::FragmentState {
                module,
                entry_point: Some(entry.name()),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState { format, blend: None, write_mask: wgpu::ColorWrites::ALL })],
            }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });
        self.built.push(((entry, volume, format), pipeline.clone()));
        pipeline
    }
}

/// A view of mip level 0 of `texture`: array layer `layer` of a 2D one, or
/// the whole of a 3D one.
fn top_view(texture: &wgpu::Texture, layer: u32) -> wgpu::TextureView {
    let volume = texture.dimension() == wgpu::TextureDimension::D3;
    texture.create_view(&wgpu::TextureViewDescriptor {
        label: Some("Wgpu transform"),
        format: None,
        dimension: Some(if volume { wgpu::TextureViewDimension::D3 } else { wgpu::TextureViewDimension::D2 }),
        usage: None,
        aspect: wgpu::TextureAspect::All,
        base_mip_level: 0,
        mip_level_count: Some(1),
        base_array_layer: if volume { 0 } else { layer },
        array_layer_count: Some(1),
    })
}

/// Where a transform's passes draw: its steps, the images between them,
/// and the texture the last draws into.
struct Drawing<'a> {
    source: &'a wgpu::Texture,
    target: &'a wgpu::Texture,
    steps: &'a [Step],
    /// Each step's image but the last's, which is drawn into the target.
    between: &'a [wgpu::Texture],
    /// The format of the images between the steps.
    work: F,
}

impl Drawing<'_> {
    /// Records the first `upto` steps for `plane`: the first reads the
    /// source's layer or depth slice, each later one the image of the step
    /// before, and each draws into its image, the last step into the
    /// target's layer or depth slice.
    fn record(&self, core: &CoreRef, passes: &mut Passes, encoder: &mut wgpu::CommandEncoder, upto: usize, plane: u32) {
        let volume = self.source.dimension() == wgpu::TextureDimension::D3;
        for (i, step) in self.steps[..upto].iter().enumerate() {
            let first = i == 0;
            let last = i + 1 == self.steps.len();
            let input = if first { top_view(self.source, plane) } else { top_view(&self.between[i - 1], 0) };
            let output = if last { top_view(self.target, plane) } else { top_view(&self.between[i], 0) };
            let mut params = step.params;
            params.slice = plane;
            let pipeline = passes.pipeline(core, step.entry, first && volume, if last { self.target.format() } else { self.work });
            let uniform = core.device.create_buffer_init(&wgpu::util::BufferInitDescriptor { label: Some("Wgpu transform"), contents: &params.bytes(), usage: wgpu::BufferUsages::UNIFORM });
            let bind_group = core.device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("Wgpu transform"),
                layout: &passes.parts(first && volume).1,
                entries: &[wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&input) }, wgpu::BindGroupEntry { binding: 1, resource: uniform.as_entire_binding() }],
            });
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("Wgpu transform"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &output,
                    depth_slice: (last && volume).then_some(plane),
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

    /// The largest luminance, by texconv's weights, of any texel of any
    /// plane of the image the color pass reads, after the changes before
    /// its tone map; the steps before the color pass are drawn for it.
    fn brightest(&self, core: &CoreRef, passes: &mut Passes, size: (u32, u32), stop: Option<&dyn Interrupt>) -> PsResult<f32> {
        let last = self.steps.len() - 1;
        let found = core.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Wgpu transform"),
            contents: &[0; 4],
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
        });
        let commands = core.scoped("WgpuTransform", "the texture cannot be transformed", || {
            let module = core.device.create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("Wgpu transform"),
                source: wgpu::ShaderSource::Wgsl(format!("{COMMON}{}{}", color::wgsl(), color::BRIGHTEST).into()),
            });
            let entry = |binding: u32, ty: wgpu::BindingType| wgpu::BindGroupLayoutEntry { binding, visibility: wgpu::ShaderStages::COMPUTE, ty, count: None };
            let layout = core.device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("Wgpu transform"),
                entries: &[
                    entry(
                        0,
                        wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Float { filterable: false }, view_dimension: wgpu::TextureViewDimension::D2, multisampled: false },
                    ),
                    entry(1, wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None }),
                    entry(2, wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Storage { read_only: false }, has_dynamic_offset: false, min_binding_size: None }),
                ],
            });
            let pipeline_layout = core.device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some("Wgpu transform"), bind_group_layouts: &[Some(&layout)], immediate_size: 0 });
            let pipeline = core.device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some("Wgpu transform"),
                layout: Some(&pipeline_layout),
                module: &module,
                entry_point: Some("brightest_texel"),
                compilation_options: Default::default(),
                cache: None,
            });
            let uniform = core.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("Wgpu transform"),
                contents: &self.steps[last].params.bytes(),
                usage: wgpu::BufferUsages::UNIFORM,
            });
            let image = top_view(&self.between[last - 1], 0);
            let bind_group = core.device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("Wgpu transform"),
                layout: &layout,
                entries: &[
                    wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&image) },
                    wgpu::BindGroupEntry { binding: 1, resource: uniform.as_entire_binding() },
                    wgpu::BindGroupEntry { binding: 2, resource: found.as_entire_binding() },
                ],
            });
            let mut encoder = core.device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("Wgpu transform") });
            for plane in 0..self.source_planes() {
                self.record(core, passes, &mut encoder, last, plane);
                let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor { label: Some("Wgpu transform"), timestamp_writes: None });
                pass.set_pipeline(&pipeline);
                pass.set_bind_group(0, &bind_group, &[]);
                pass.dispatch_workgroups(size.0.div_ceil(8), size.1.div_ceil(8), 1);
            }
            encoder.finish()
        })?;
        core.submit_and_wait(vec![commands], stop)?;
        let bytes = crate::buffer::read_bytes(core, &found, 0, 4, stop)?;
        let bits: [u8; 4] = bytes.as_slice().try_into().map_err(|e| invalid("WgpuTransform", format!("the brightest texel's luminance came back as {} bytes: {e}", bytes.len())))?;
        Ok(f32::from_bits(u32::from_le_bytes(bits)))
    }

    /// The source's layers, or its depth slices.
    fn source_planes(&self) -> u32 {
        self.source.depth_or_array_layers()
    }
}

/// Draws `plan` from level 0 of `source`, a texture of a float format the
/// passes render into, every layer or depth slice of it, into level 0 of a
/// new texture of the same format, dimension and layers at the plan's
/// size, with `levels` mip levels for the caller to render. The color pass
/// `recolor` describes runs last, where it changes any texel; for
/// Reinhard's curve the brightest texel's luminance is found first, and
/// returned.
pub(crate) fn draw(core: &CoreRef, source: &wgpu::Texture, plan: &Plan, recolor: Option<&Prepared>, levels: u32, stop: Option<&dyn Interrupt>) -> PsResult<(wgpu::Texture, Option<f32>)> {
    let format = source.format();
    let volume = source.dimension() == wgpu::TextureDimension::D3;
    // The passes between the first and the last keep 32-bit floats where
    // the device renders them.
    let work = if convert::float_renderable(core, F::Rgba32Float) { F::Rgba32Float } else { F::Rgba16Float };
    let make = |size: (u32, u32), layers: u32, levels: u32, format: F, usage: wgpu::TextureUsages, dimension: wgpu::TextureDimension| {
        core.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("Wgpu transform"),
            size: wgpu::Extent3d { width: size.0, height: size.1, depth_or_array_layers: layers },
            mip_level_count: levels,
            sample_count: 1,
            dimension,
            format,
            usage,
            view_formats: &[],
        })
    };
    let target = core.scoped("WgpuTransform", "the texture cannot be transformed", || {
        let usage = wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC | wgpu::TextureUsages::COPY_DST;
        make(plan.canvas, source.depth_or_array_layers(), levels, format, usage, source.dimension())
    })?;
    let recolor = recolor.filter(|r| r.changes());
    let mut steps = steps(plan, recolor.map(Params::recolor));
    let (mut passes, between) = core.scoped("WgpuTransform", "the texture cannot be transformed", || {
        let between: Vec<wgpu::Texture> = steps[..steps.len() - 1]
            .iter()
            .map(|step| make(step.size, 1, 1, work, wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::RENDER_ATTACHMENT, wgpu::TextureDimension::D2))
            .collect();
        (Passes::new(core, volume), between)
    })?;
    let white = match recolor {
        Some(r) if r.reinhard() => {
            let brightest = Drawing { source, target: &target, steps: &steps, between: &between, work }.brightest(core, &mut passes, plan.canvas, stop)?;
            if let Some(last) = steps.last_mut() {
                last.params.white = brightest * brightest;
            }
            Some(brightest)
        }
        _ => None,
    };
    let drawing = Drawing { source, target: &target, steps: &steps, between: &between, work };
    let commands = core.scoped("WgpuTransform", "the texture cannot be transformed", || {
        let mut encoder = core.device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("Wgpu transform") });
        for plane in 0..drawing.source_planes() {
            drawing.record(core, &mut passes, &mut encoder, steps.len(), plane);
        }
        encoder.finish()
    })?;
    core.submit(vec![commands], stop)?;
    Ok((target, white))
}

/// The Basis Universal format a texture of `format` is encoded again
/// through after a transform, and the transcoder target that gives
/// `format` back: ASTC LDR through the Basis ASTC format of its block
/// size, ASTC HDR 4x4 and 6x6 through Basis's HDR formats of those sizes,
/// and ETC2 and unsigned EAC through BasisUastc4x4.
fn basis_route(format: F) -> Option<(BasisFormat, u32)> {
    let uastc = BASIS_FORMATS[1];
    match format {
        F::Etc2Rgb8Unorm | F::Etc2Rgb8UnormSrgb => Some((uastc, target::ETC1_RGB)),
        F::Etc2Rgba8Unorm | F::Etc2Rgba8UnormSrgb => Some((uastc, target::ETC2_RGBA)),
        F::EacR11Unorm => Some((uastc, target::ETC2_EAC_R11)),
        F::EacRg11Unorm => Some((uastc, target::ETC2_EAC_RG11)),
        F::Astc { block, channel: AstcChannel::Unorm | AstcChannel::UnormSrgb } => {
            let size = F::Astc { block, channel: AstcChannel::Unorm }.block_dimensions();
            let index = basis::ASTC_LDR_SIZES.iter().position(|&s| s == size)?;
            let target = if index == 0 { target::ASTC_LDR_4X4_RGBA } else { target::ASTC_LDR_5X4_RGBA + index as u32 - 1 };
            Some((BASIS_FORMATS[19 + index], target))
        }
        F::Astc { block: AstcBlock::B4x4, channel: AstcChannel::Hdr } => Some((BASIS_FORMATS[2], target::ASTC_HDR_4X4_RGBA)),
        F::Astc { block: AstcBlock::B6x6, channel: AstcChannel::Hdr } => Some((BASIS_FORMATS[3], target::ASTC_HDR_6X6_RGBA)),
        _ => None,
    }
}

/// Every layer of each of the first `count` mip levels of `target`, read
/// back from the GPU as colors.
fn read_colors(core: &CoreRef, target: &wgpu::Texture, count: u32, stop: Option<&dyn Interrupt>) -> PsResult<Vec<Vec<[f32; 4]>>> {
    let mut levels = Vec::with_capacity(count as usize);
    for level in 0..count {
        levels.push(texel::colors(&image_io::read_level(core, target, level, stop)?, target.format())?);
    }
    Ok(levels)
}

/// The format `apply` packs the transformed texels into.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Packed {
    /// The file's own format: packed or encoded back into it.
    Own,
    /// The uncompressed format the file's texels decode into.
    Decoded,
    /// The uncompressed form of a -Format the caller converts the texels
    /// into next, in the color space the color changes settle against it.
    For(F),
}

/// `file` with `transform` applied. Mip level 0 is decoded where it is
/// block-compressed, by `decoder` (on the CPU when `None`), drawn on the
/// GPU, and the texture keeps as many of its `levels` as the new size's
/// full chain has, rendered again below level 0 as `mips` says. Packed
/// Own, the texels are packed or encoded back into `file`'s format, BC on
/// the GPU with `effort`, and ETC2, EAC and ASTC through Basis Universal;
/// a format the module has no encoder for stays in the format it decodes
/// into, which a warning says. Packed Decoded, they are packed into the
/// uncompressed format the texels decode into, and packed For a -Format,
/// into that format's uncompressed form, switched to its other color space
/// where -ColorSpace asks.
#[allow(clippy::too_many_arguments)]
pub(crate) fn apply(
    core: &CoreRef,
    file: TextureFile,
    levels: u32,
    mips: convert::MipOptions,
    transform: &Transform,
    packed: Packed,
    decoder: Option<Decoder>,
    effort: Option<u32>,
    stop: Option<&dyn Interrupt>,
    said: &mut Said,
) -> PsResult<TextureFile> {
    let plan = transform.plan(file.width, file.height, file.dimension, file.cube)?;
    let (dimension, depth, layers, cube, from) = (file.dimension, file.depth, file.layers, file.cube, file.format);
    let (width, height) = plan.size();
    let third = if dimension == wgpu::TextureDimension::D3 { depth } else { layers };
    let count = levels.min(wgpu::Extent3d { width, height, depth_or_array_layers: third }.max_mips(dimension)).max(1);
    let decoded = if from.is_compressed() { texfile::decoded_format(from).ok_or_else(|| invalid("WgpuTransform", format!("the module decodes no {} texels", format_name(from))))? } else { from };
    let render = texel::render_format(decoded).ok_or_else(|| {
        invalid("WgpuTransform", format!("a transform reads texels as floating-point colors, and {} holds integers, depth or stencil", format_name(decoded)))
    })?;
    if transform.border.is_some() && render.components() < 4 {
        return Err(invalid("WgpuAlpha", format!("{} has no alpha channel, so -FixAlphaBorder would change nothing", format_name(from))));
    }
    // Normals the transform writes take four channels, for Z and alpha, in
    // 32-bit floats where the device renders them, as a slope is the
    // difference of nearly equal heights.
    let render = if !transform.writes_normals() {
        render
    } else if convert::float_renderable(core, F::Rgba32Float) {
        F::Rgba32Float
    } else {
        F::Rgba16Float
    };
    let mut top = file;
    top.levels.truncate(1);
    let source_file = match (from.is_compressed(), decoder.unwrap_or(Decoder::Cpu)) {
        (true, Decoder::Cpu) => {
            let base = texfile::uncompressed(top, said)?;
            said.verbose.push(format!("the {} texels were decoded on the CPU into {}", format_name(from), format_name(base.format)));
            base
        }
        (true, Decoder::Gpu) => {
            said.verbose.push(format!("the {} texels were decoded on the GPU into {}", format_name(from), format_name(render)));
            top
        }
        (false, _) => top,
    };
    // Normals written for a conversion after the transform are kept in a
    // format that stores them as the conversion's target does.
    let into = match (packed, transform.writes_normals()) {
        (Packed::Own, _) => from,
        (_, true) => normal::kept_in(transform.target.unwrap_or(decoded), decoded),
        (Packed::Decoded, false) => decoded,
        (Packed::For(format), false) => {
            let form = texfile::decoded_format(format).unwrap_or(format);
            if texel::render_format(form).is_none() {
                return Err(invalid("WgpuTransform", format!("-Format names {}, which holds integers, depth or stencil, and a transform writes floating-point colors", format_name(format))));
            }
            form
        }
    };
    let retarget = matches!(packed, Packed::For(_)) && !transform.writes_normals();
    let prepared = if transform.color.is_empty() { None } else { Some(transform.color.prepare(from, into, normal::signed(from), retarget)?) };
    let into = prepared.as_ref().map_or(into, |p| p.format);
    if mips.coverage.is_some() && (into.components() < 4 || render.components() < 4) {
        let without = if into.components() < 4 { into } else { from };
        return Err(invalid("WgpuAlpha", format!("{} has no alpha channel, so -KeepCoverage would change nothing", format_name(without))));
    }
    // A swizzle can move a color channel into alpha, and a color key clears
    // alpha; normals made from heights have the alpha -NormalAlpha says.
    let opaque = match transform.normal {
        Some(step) => step.alpha == NormalAlpha::One,
        None => texel::opaque(&source_file.levels[0], source_file.format) && !(plan.pads() && plan.color[3] < 1.0) && transform.color.swizzle.is_none() && transform.color.key.is_none(),
    };
    let source = convert::float_levels(core, &source_file, render, 1, 1, convert::MipOptions::default(), stop)?;
    let (drawn, white) = draw(core, &source, &plan, prepared.as_ref(), count, stop)?;
    if let Some(filters) = &transform.filters {
        filter::apply(core, &drawn, filters, into.is_srgb(), stop)?;
    }
    if let Some(border) = transform.border {
        alpha::fix_border(core, &drawn, border, stop)?;
    }
    let mut tiling = None;
    let drawn = match transform.normal {
        Some(step) => {
            if step.edge != HeightEdge::Wrap {
                tiling = normal::tiling_note(core, &drawn, step.channel, stop)?;
            }
            normal::from_heights(core, &drawn, &step, render, count, normal::signed(into), stop)?
        }
        None => drawn,
    };
    if count > 1 {
        convert::render_mips(core, &drawn, 1, mips.stored_as(into), stop)?;
    }
    let geometry = Transform { color: Color::default(), filters: None, border: None, normal: None, ..*transform };
    if !geometry.is_empty() || (prepared.is_none() && transform.filters.is_none() && transform.border.is_none() && transform.normal.is_none()) {
        said.verbose.push(transform.describe(&plan, render));
    }
    if let Some(prepared) = &prepared {
        said.verbose.extend(prepared.said.iter().cloned());
        let done = transform.color.describe(prepared);
        if !done.is_empty() {
            said.verbose.push(format!("the texels were {} on the GPU in {}", listed(&done), format_name(render)));
        }
        if let Some(white) = white {
            said.verbose.push(format!("Reinhard's white point was the brightest texel's luminance, {white}"));
        }
    }
    if let Some(filters) = &transform.filters {
        said.verbose.extend(filters.said());
    }
    if let Some(border) = transform.border {
        said.verbose.push(border.said());
    }
    if let Some(step) = transform.normal {
        said.verbose.push(step.said());
        said.verbose.extend(tiling);
    }
    if count > 1 {
        let span = if count == 2 { "mip level 1 was".to_string() } else { format!("mip levels 1 to {} were", count - 1) };
        said.verbose.push(format!("{span} rendered again on the GPU, {}", mips.said()));
        said.mips = MipsMade::Rendered;
    }
    let shaped = |format: F, levels: Vec<Vec<u8>>| TextureFile { dimension, width, height, depth, layers, cube, format, levels };

    if !into.is_compressed() {
        let mut out = Vec::with_capacity(count as usize);
        for level in 0..count {
            let texels = image_io::read_level(core, &drawn, level, stop)?;
            out.push(if into == render { texels } else { texel::pack(&texel::colors(&texels, render)?, into)? });
        }
        if into != render {
            said.verbose.push(format!("the texels were packed into {} on the CPU", format_name(into)));
        }
        return Ok(shaped(into, out));
    }
    if convert::check_encodable(into).is_ok() {
        let (encoded, by) = convert::encode_bc(core, &drawn, into, effort, opaque, 0, stop)?;
        said.verbose.push(format!("the texels were encoded again into {} on the GPU by {by}", format_name(into)));
        return Ok(shaped(into, encoded));
    }
    // Basis Universal encodes 2D textures, arrays and cube maps only.
    if let Some((basis, target)) = basis_route(into).filter(|_| dimension == wgpu::TextureDimension::D2) {
        let hdr = basis.hdr;
        let pixels = if hdr { F::Rgba32Float } else if into.is_srgb() { F::Rgba8UnormSrgb } else { F::Rgba8Unorm };
        let mut colors = read_colors(core, &drawn, count, stop)?;
        // Basis Universal takes an RG11 texture's green from alpha.
        if into == F::EacRg11Unorm {
            for level in &mut colors {
                for c in level.iter_mut() {
                    c[3] = c[1];
                }
            }
        }
        let mut out = Vec::with_capacity(count as usize);
        for level in &colors {
            out.push(texel::pack(level, pixels)?);
        }
        // An array, a cube map or an HDR format takes a full chain or one
        // level from Basis Universal, which makes the chain from level 0;
        // the levels past `count` are left out after transcoding.
        let full = wgpu::Extent3d { width, height, depth_or_array_layers: layers }.max_mips(wgpu::TextureDimension::D2);
        let asked = if (hdr || layers > 1) && count > 1 { full } else { count };
        let bytes = image_io::encode_basis(core, shaped(pixels, out), basis, Some(asked), mips, None, effort, false, stop, said)?;
        let file = texfile::transcode_ktx2(&bytes, target, into, count, said)?;
        said.verbose.push(format!("the texels were encoded again into {} through Basis Universal's {}", format_name(into), basis.name));
        return Ok(TextureFile { dimension, depth, ..file });
    }
    // The format `into`'s texels decode into, sRGB where `into` is.
    let unencoded = texfile::decoded_format(into).unwrap_or(decoded);
    let mut out = Vec::with_capacity(count as usize);
    for level in 0..count {
        out.push(texel::pack(&texel::colors(&image_io::read_level(core, &drawn, level, stop)?, render)?, unencoded)?);
    }
    let what = match dimension {
        wgpu::TextureDimension::D1 => format!("a 1D {} texture", format_name(into)),
        wgpu::TextureDimension::D2 => format_name(into),
        wgpu::TextureDimension::D3 => format!("a 3D {} texture", format_name(into)),
    };
    said.warnings.push(format!(
        "the module has no encoder for {what}, so the transformed texture is {}, the format its texels decode into; Compress encodes it in another format",
        format_name(unencoded)
    ));
    Ok(shaped(unencoded, out))
}

/// Level 0 of `file`'s first layer as the colors a shader reads, linear for
/// an sRGB format, at `size`: resized there with the default filters when
/// its size differs. A block-compressed file is decoded on the CPU first.
pub(crate) fn colors_at(core: &CoreRef, file: TextureFile, size: (u32, u32), stop: Option<&dyn Interrupt>, said: &mut Said) -> PsResult<Vec<[f32; 4]>> {
    let resize = if (file.width, file.height) == size { Transform::default() } else { Transform { width: Some(size.0), height: Some(size.1), ..Default::default() } };
    let plan = resize.plan(file.width, file.height, file.dimension, false)?;
    let from = file.format;
    let mut top = file;
    top.levels.truncate(1);
    let len = top.layer_len(0);
    top.levels[0].truncate(len);
    top.layers = 1;
    top.cube = false;
    let top = if from.is_compressed() { texfile::uncompressed(top, said)? } else { top };
    let render = texel::render_format(top.format).ok_or_else(|| invalid("WgpuTransform", format!("a transform reads texels as floating-point colors, and {} holds integers, depth or stencil", format_name(top.format))))?;
    let source = convert::float_levels(core, &top, render, 1, 1, convert::MipOptions::default(), stop)?;
    let (drawn, _) = draw(core, &source, &plan, None, 1, stop)?;
    texel::colors(&image_io::read_level(core, &drawn, 0, stop)?, render)
}

/// Writes the texture `transform` makes of `texture`, its mip levels
/// rendered again as `mips` says, in the format `format` names or its own,
/// with what making it said to the verbose and warning streams.
fn write_transformed(texture: &PsProxy<Texture>, transform: &Transform, mips: convert::MipOptions, format: Option<&str>, ps: &Pipeline<'_>) -> PsResult<()> {
    let format = format.map(image_io::parse_target).transpose()?;
    let (core, source) = texture.with(|t| (t.core.clone(), t.source()))?;
    let mut said = Said::default();
    let made = image_io::transformed(&core, &source, transform, mips, format, Some(ps), &mut said);
    tell(ps, &said)?;
    ps.write(made?)
}

/// Changes a texture's size into a new texture: resizes it to a width and
/// height, a scale, a size limit or powers of two, with a filter; crops it
/// to a region, a size or margins; and pads it onto a larger canvas. It
/// crops, then resizes, then pads, whatever order the parameters come in.
///
/// The new texture has the texture's format, or the -Format named, and as
/// many of its mip levels as the new size's chain has, rendered again from
/// level 0 as -MipFilter, -MipEdge, -KeepCoverage and -NormalMips say; the
/// texture is left as it is. Shrinking
/// defaults to the Box filter and enlarging to Bicubic, each side on its
/// own. What was done goes to the verbose stream and the new texture's
/// Notes.
///
/// # Examples
/// Resize-WgpuTexture $tex -Width 256
/// Resize-WgpuTexture $tex -Scale 0.5 -Filter Lanczos
/// Resize-WgpuTexture $tex -MaxSize 1024 -PowerOfTwo Down
/// Resize-WgpuTexture $tex -Crop 16,16,256,256
/// Resize-WgpuTexture $tex -Pad 512,512 -Anchor TopLeft -PadColor 1,1,1,1
/// Resize-WgpuTexture $photo -MaxSize 512 -Format Bc7RgbaUnormSrgb
#[cmdlet(verb = "Resize", noun = "WgpuTexture", alias = ["Resize-GpuTexture"], output = ["Wgpu.Texture"])]
#[derive(Default)]
pub struct ResizeWgpuTexture {
    /// The texture to resize; it is left as it is.
    #[param(mandatory, position = 0, value_from_pipeline)]
    pub texture: Option<PsProxy<Texture>>,
    /// The new width; without -Height, the height keeps the aspect ratio.
    #[param]
    pub width: Option<u32>,
    /// The new height; without -Width, the width keeps the aspect ratio.
    #[param]
    pub height: Option<u32>,
    /// A factor for both sides, such as 0.5 for half the size.
    #[param]
    pub scale: Option<f64>,
    /// The most texels either side may have: a texture past it shrinks to
    /// it, keeping its aspect ratio, and a smaller one stays as it is.
    #[param]
    pub max_size: Option<u32>,
    /// Rounds each side to a power of two: Nearest, Up or Down.
    #[param]
    pub power_of_two: Option<PowerOfTwo>,
    /// The filter: Nearest, Box, Bilinear, Bicubic or Lanczos; Box where a
    /// side shrinks and Bicubic where it grows when omitted.
    #[param]
    pub filter: Option<ResizeFilter>,
    /// The part to keep: four numbers, the left, top, width and height of
    /// a region, or two, a width and height -Anchor places.
    #[param]
    pub crop: Option<Vec<u32>>,
    /// The texels to cut from the left, top, right and bottom.
    #[param]
    pub crop_margin: Option<Vec<u32>>,
    /// The width and height of a larger canvas to place the texture on.
    #[param]
    pub pad: Option<Vec<u32>>,
    /// Where a crop to a size, or the texture on the padded canvas, is
    /// placed: TopLeft, Top, TopRight, Left, Center, Right, BottomLeft,
    /// Bottom or BottomRight; Center when omitted.
    #[param]
    pub anchor: Option<Anchor>,
    /// The color around the padded texture: red, green, blue and alpha
    /// from 0 to 1; transparent black when omitted.
    #[param]
    pub pad_color: Option<Vec<f64>>,
    /// How the mip levels rendered again are made: Box, the default, each
    /// from the level above; Triangle, Kaiser, Mitchell or Lanczos from
    /// level 0; Min or Max, the smallest or largest value of each channel
    /// among the texels each new texel covers.
    #[param]
    pub mip_filter: Option<MipFilter>,
    /// What Triangle, Kaiser, Mitchell and Lanczos read past an edge of the
    /// image: Clamp, the default; Wrap, the other side; or Mirror.
    #[param]
    pub mip_edge: Option<MipEdge>,
    /// Scales the alpha of each mip level rendered again so the share of
    /// its texels with alpha at or above this, the alpha an alpha test
    /// passes at, stays level 0's: above 0 and at most 1.
    #[param]
    pub keep_coverage: Option<f64>,
    /// Keeps the normals of a normal map unit length in each mip level
    /// rendered again: Renormalize scales each texel's normal back to
    /// length 1, and SlopeSpace filters the normals' slopes.
    #[param]
    pub normal_mips: Option<NormalMips>,
    /// The new texture's format: a color format, a BC format, encoded on
    /// the GPU, or a Basis Universal format; the texture's own when omitted.
    #[param]
    pub format: Option<String>,
}

impl Cmdlet for ResizeWgpuTexture {
    fn process(&mut self, ps: &Pipeline<'_>) -> PsResult<()> {
        let Some(texture) = &self.texture else {
            return Err(invalid("WgpuNoTexture", "no texture was given"));
        };
        let transform = Asked {
            width: self.width,
            height: self.height,
            scale: self.scale,
            max_size: self.max_size,
            power_of_two: self.power_of_two,
            filter: self.filter,
            crop: self.crop.as_deref(),
            crop_margin: self.crop_margin.as_deref(),
            pad: self.pad.as_deref(),
            anchor: self.anchor,
            pad_color: self.pad_color.as_deref(),
            ..Default::default()
        }
        .transform()?;
        if transform.is_empty() {
            return Err(invalid("WgpuResize", "give a new size, a crop or padding: -Width, -Height, -Scale, -MaxSize, -PowerOfTwo, -Crop, -CropMargin or -Pad"));
        }
        let mips = convert::MipOptions::given(self.mip_filter, self.mip_edge).with_coverage(self.keep_coverage)?.with_normals(self.normal_mips, true);
        write_transformed(texture, &transform, mips, self.format.as_deref(), ps)
    }
}

/// Flips or rotates a texture, changes its channels and colors, or filters
/// it, into a new texture: -Flip mirrors it and -Rotate turns it 90, 180 or
/// 270 degrees clockwise, turning first; then -AssignColorSpace, -ColorKey,
/// -Swizzle, -ReconstructZ, -Normalize, -FlipGreen, -Unpremultiply,
/// -AlphaThreshold, -Exposure, -ToneMap, -Brightness and -Contrast, -Gamma,
/// -Grayscale, -Invert, -Premultiply and -ColorSpace change each texel, in
/// that order, whatever order they are given in; then -Blur blurs the
/// texels, -Sharpen sharpens them and -Edges finds their edges, in that
/// order; and last -FixAlphaBorder colors the texels with alpha 0 from the
/// nearest texels with alpha above 0. The filters work on level 0 of each
/// layer, and of each depth slice of a 3D texture, on its own.
///
/// The new texture keeps the texture's mip levels, rendered again from
/// level 0 as -MipFilter, -MipEdge, -KeepCoverage and -NormalMips say, and
/// its format, unless -ColorSpace or -AssignColorSpace switches it to the
/// format's sRGB or linear form, or -Format names another: a color format,
/// a BC format, encoded on the GPU, or a Basis Universal format, which
/// -ColorSpace switches to the form it names where the format has both and
/// -AssignColorSpace leaves as it is; -Format alone converts each level as
/// it is. The texture is left as it is. A
/// cube map is not flipped or rotated, as its faces would no longer meet
/// at their edges. A swizzle, the normal steps, brightness, contrast, gamma,
/// inversion and the gradients and Laplacian of -Edges work on the values
/// the texture stores, the others on linear values, which for an sRGB
/// texture are the values a shader reads.
/// The normal steps read a normal n stored as n / 2 + 1 / 2 in a format
/// that holds no negative values, and as n in any other.
///
/// # Examples
/// Convert-WgpuTexture $tex -Flip Vertical
/// Convert-WgpuTexture $tex -Rotate 90
/// $tex | Convert-WgpuTexture -Rotate 270 -Flip Horizontal
/// $tex | Convert-WgpuTexture -Swizzle bgra
/// $hdr | Convert-WgpuTexture -Exposure 1 -ToneMap AgX
/// $tex | Convert-WgpuTexture -Grayscale -GrayscaleWeights Rec601
/// $sprite | Convert-WgpuTexture -ColorKey '#FF00FF'
/// $leaves | Convert-WgpuTexture -AlphaThreshold 0.5
/// $sprite | Convert-WgpuTexture -ColorKey '#FF00FF' -FixAlphaBorder
/// $normals | Convert-WgpuTexture -FlipGreen
/// $normals | Convert-WgpuTexture -Normalize -NormalMips Renormalize
/// $hdr | Convert-WgpuTexture -ToneMap AgX -ColorSpace Srgb -Format Rgba8Unorm
/// $photo | Convert-WgpuTexture -Blur 2
/// $photo | Convert-WgpuTexture -Sharpen 0.8 -SharpenRadius 1.5
/// $photo | Convert-WgpuTexture -Edges Canny -EdgeSigma 1.4
#[cmdlet(verb = "Convert", noun = "WgpuTexture", alias = ["Convert-GpuTexture"], output = ["Wgpu.Texture"])]
#[derive(Default)]
pub struct ConvertWgpuTexture {
    /// The texture to change; it is left as it is.
    #[param(mandatory, position = 0, value_from_pipeline)]
    pub texture: Option<PsProxy<Texture>>,
    /// Mirrors the texture: Horizontal swaps left and right, and Vertical
    /// top and bottom.
    #[param]
    pub flip: Option<FlipDirection>,
    /// Turns the texture clockwise by 90, 180 or 270 degrees.
    #[param]
    pub rotate: Option<u32>,
    /// Rearranges the channels: four letters of r, g, b, a, 0 and 1, one
    /// for each channel of the new texture, as bgra to swap red and blue or
    /// rrr1 to spread red with an opaque alpha. An sRGB texture's channels
    /// move as the values it stores.
    #[param]
    pub swizzle: Option<String>,
    /// Rebuilds each normal's Z, in blue, from its X and Y, in red and
    /// green, as for a normal map stored in two channels.
    #[param]
    pub reconstruct_z: bool,
    /// Scales each normal to length 1.
    #[param]
    pub normalize: bool,
    /// Flips each normal's green, between the convention glTF reads, green
    /// up, and DirectX's, green down.
    #[param]
    pub flip_green: bool,
    /// Multiplies red, green and blue by alpha, in linear light for an sRGB
    /// texture.
    #[param]
    pub premultiply: bool,
    /// Divides red, green and blue by alpha where alpha is above 0, in
    /// linear light for an sRGB texture.
    #[param]
    pub unpremultiply: bool,
    /// Converts the texels to Linear or Srgb so their colors look the same:
    /// a format with an sRGB form, as Rgba8Unorm has Rgba8UnormSrgb,
    /// switches to that form, and any other keeps its format with its
    /// values encoded or decoded.
    #[param]
    pub color_space: Option<ColorSpace>,
    /// Relabels the texels as Linear or Srgb without changing them,
    /// switching the format to that form: for a texture loaded with the
    /// wrong one.
    #[param]
    pub assign_color_space: Option<ColorSpace>,
    /// Makes the texture grayscale, with Rec. 709's weights in linear light
    /// or those -GrayscaleWeights names.
    #[param]
    pub grayscale: bool,
    /// The weights -Grayscale uses: Rec709, in linear light; Rec601, on the
    /// stored values, as Pillow's convert('L'); or three numbers for red,
    /// green and blue, in linear light.
    #[param]
    pub grayscale_weights: Option<PsObject>,
    /// Turns red, green and blue into 1 minus their stored values, as
    /// ImageMagick's -negate; alpha is kept.
    #[param]
    pub invert: bool,
    /// Brightens or darkens the stored values, from -100 to 100, as
    /// ImageMagick's -brightness-contrast.
    #[param]
    pub brightness: Option<f64>,
    /// Raises or lowers the stored values' contrast around their middle,
    /// from -100 to 100, as ImageMagick's -brightness-contrast.
    #[param]
    pub contrast: Option<f64>,
    /// Gamma corrects the stored values as ImageMagick's -gamma: each
    /// becomes value^(1/gamma), so a gamma above 1 lightens.
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
    /// or of two, across and down, reaching 4 standard deviations. The blur
    /// works on linear values weighed by alpha, so no color of a
    /// transparent texel bleeds in.
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
    /// adds back in a channel, so smooth areas keep their texels; 0 when
    /// omitted.
    #[param]
    pub sharpen_threshold: Option<f64>,
    /// Replaces the texels with their edges: Sobel, Scharr or Prewitt, the
    /// size of each channel's gradient, where a step from 0 to 1 reads 1;
    /// Laplacian, the size of each channel's second derivative; or Canny,
    /// 1 on the thin, linked edges of the luminance and 0 elsewhere, as
    /// scikit-image's canny finds them. Alpha is kept.
    #[param]
    pub edges: Option<EdgeOperator>,
    /// Canny's low and high thresholds: a texel on an edge is kept where
    /// its gradient's size is at least the low one and it links to a texel
    /// at least the high one, in the scale where a step from 0 to 1 reads 1;
    /// 0.1 and 0.2 when omitted.
    #[param]
    pub edge_threshold: Option<Vec<f64>>,
    /// The standard deviation, in texels, of the Gaussian Canny smooths the
    /// luminance with before its gradient; 1 when omitted.
    #[param]
    pub edge_sigma: Option<f64>,
    /// What -Blur, -Sharpen and -Edges read past the edge of the texture:
    /// Clamp, the edge texel repeated, the default; Wrap, the other side,
    /// for a texture that tiles; or Mirror, the texture mirrored with the
    /// edge texel repeated. For Canny it is what the smoothing reads, and
    /// the gradient after repeats the edge texel, as scikit-image's does.
    #[param]
    pub filter_edge: Option<FilterEdge>,
    /// Gives each texel with alpha 0 the color of the nearest texel with
    /// alpha above 0, its alpha left at 0, so filtering and mip levels
    /// blend no dark or keyed color in at a cutout's edge.
    #[param]
    pub fix_alpha_border: bool,
    /// How far -FixAlphaBorder carries color, in texels: a texel with alpha
    /// 0 farther than this from every texel with alpha above 0 keeps its
    /// color. Every texel with alpha 0 is colored when omitted.
    #[param]
    pub border_reach: Option<u32>,
    /// How the mip levels rendered again are made: Box, the default, each
    /// from the level above; Triangle, Kaiser, Mitchell or Lanczos from
    /// level 0; Min or Max, the smallest or largest value of each channel
    /// among the texels each new texel covers.
    #[param]
    pub mip_filter: Option<MipFilter>,
    /// What Triangle, Kaiser, Mitchell and Lanczos read past an edge of the
    /// image: Clamp, the default; Wrap, the other side; or Mirror.
    #[param]
    pub mip_edge: Option<MipEdge>,
    /// Scales the alpha of each mip level rendered again so the share of
    /// its texels with alpha at or above this, the alpha an alpha test
    /// passes at, stays level 0's: above 0 and at most 1.
    #[param]
    pub keep_coverage: Option<f64>,
    /// Keeps the normals of a normal map unit length in each mip level
    /// rendered again: Renormalize scales each texel's normal back to
    /// length 1, and SlopeSpace filters the normals' slopes.
    #[param]
    pub normal_mips: Option<NormalMips>,
    /// The new texture's format: a color format, a BC format, encoded on
    /// the GPU, or a Basis Universal format, switched by -ColorSpace to the
    /// form it names where the format has both; the texture's own when
    /// omitted.
    #[param]
    pub format: Option<String>,
}

impl Cmdlet for ConvertWgpuTexture {
    fn process(&mut self, ps: &Pipeline<'_>) -> PsResult<()> {
        let Some(texture) = &self.texture else {
            return Err(invalid("WgpuNoTexture", "no texture was given"));
        };
        let transform = Asked {
            flip: self.flip,
            rotate: self.rotate,
            swizzle: self.swizzle.as_deref(),
            reconstruct_z: self.reconstruct_z,
            normalize: self.normalize,
            flip_green: self.flip_green,
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
                edges: self.edges,
                edge_threshold: self.edge_threshold.as_deref(),
                edge_sigma: self.edge_sigma,
                edge: self.filter_edge,
            },
            ..Default::default()
        }
        .transform()?;
        if transform.is_empty() && self.format.is_none() {
            return Err(invalid(
                "WgpuTransform",
                "give -Flip, -Rotate, -Format, a filter, -Blur, -Sharpen or -Edges, or a change to the channels or colors: -Swizzle, -ReconstructZ, -Normalize, -FlipGreen, -Premultiply, -Unpremultiply, -ColorSpace, -AssignColorSpace, -Grayscale, -Invert, -Brightness, -Contrast, -Gamma, -Exposure, -ToneMap, -ColorKey, -AlphaThreshold or -FixAlphaBorder",
            ));
        }
        let mips = convert::MipOptions::given(self.mip_filter, self.mip_edge).with_coverage(self.keep_coverage)?.with_normals(self.normal_mips, true);
        write_transformed(texture, &transform, mips, self.format.as_deref(), ps)
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

    const D2: wgpu::TextureDimension = wgpu::TextureDimension::D2;

    fn plan(t: Transform, width: u32, height: u32) -> Plan {
        t.plan(width, height, D2, false).expect("planned")
    }

    /// An RGBA image of `width` by `height` texels, each texel's channels
    /// a different mix of its position, so every texel differs.
    fn picture(width: u32, height: u32) -> image::RgbaImage {
        image::RgbaImage::from_fn(width, height, |x, y| image::Rgba([(x * 37 + y * 11) as u8, (x * 5 + y * 41) as u8, ((x ^ y) * 23) as u8, 255 - (x + y) as u8]))
    }

    /// A smooth RGBA gradient, which block compression keeps close.
    fn gradient(width: u32, height: u32) -> image::RgbaImage {
        image::RgbaImage::from_fn(width, height, |x, y| image::Rgba([(x * 255 / width) as u8, (y * 255 / height) as u8, 128, 255]))
    }

    /// An Rgba8Unorm file holding `image`.
    fn file_of(image: &image::RgbaImage) -> TextureFile {
        TextureFile { dimension: D2, width: image.width(), height: image.height(), depth: 1, layers: 1, cube: false, format: F::Rgba8Unorm, levels: vec![image.as_raw().clone()] }
    }

    /// The image `plan`'s map shows of `source`, worked out on the CPU.
    fn mapped(source: &image::RgbaImage, plan: &Plan) -> image::RgbaImage {
        image::RgbaImage::from_fn(plan.oriented.0, plan.oriented.1, |x, y| {
            let (sx, sy) = plan.map.at(x, y);
            *source.get_pixel(sx, sy)
        })
    }

    /// The drawing map takes the center of the source texel each texel of
    /// the cropped, turned and flipped image shows to that texel's center,
    /// for every orientation, and the resize scales each side.
    #[test]
    fn the_drawing_map_lands_each_source_texel_where_the_plan_puts_it() {
        let flips = [None, Some(FlipDirection::Horizontal), Some(FlipDirection::Vertical)];
        for rotate in [None, Some(90), Some(180), Some(270)] {
            for flip in flips {
                let asked = Transform { crop: Some(Crop::Region { x: 2, y: 1, width: 5, height: 3 }), rotate, flip, ..Default::default() };
                let plan = plan(asked, 9, 7);
                let (size, m) = plan.drawn();
                assert_eq!(size, plan.oriented, "{rotate:?} {flip:?}");
                for y in 0..plan.oriented.1 {
                    for x in 0..plan.oriented.0 {
                        let (sx, sy) = plan.map.at(x, y);
                        let (px, py) = (f64::from(sx) + 0.5, f64::from(sy) + 0.5);
                        let landed = (m[0] * px + m[2] * py + m[4], m[1] * px + m[3] * py + m[5]);
                        assert_eq!(landed, (f64::from(x) + 0.5, f64::from(y) + 0.5), "{rotate:?} {flip:?} texel {x},{y}");
                    }
                }
            }
        }
        let (size, m) = plan(Transform { width: Some(30), height: Some(14), ..Default::default() }, 10, 7).drawn();
        assert_eq!((size, m), ((30, 14), [3.0, 0.0, 0.0, 2.0, 0.0, 0.0]));
    }

    /// Drawing does the crop, rotation, flip and resize, and leaves the
    /// padding, with the anchor that places it, and the changes after it.
    #[test]
    fn after_drawing_keeps_the_padding_and_what_follows() {
        let asked = Transform {
            crop: Some(Crop::Size { width: 4, height: 4 }),
            rotate: Some(90),
            flip: Some(FlipDirection::Vertical),
            width: Some(8),
            filter: Some(ResizeFilter::Lanczos),
            anchor: Some(Anchor::TopLeft),
            pad: Some((10, 10)),
            pad_color: Some([1.0, 0.0, 0.0, 1.0]),
            ..Default::default()
        };
        let left = asked.after_drawing();
        assert_eq!(left, Transform { anchor: Some(Anchor::TopLeft), pad: Some((10, 10)), pad_color: Some([1.0, 0.0, 0.0, 1.0]), ..Default::default() });
        assert_eq!(Transform { crop: Some(Crop::Size { width: 4, height: 4 }), anchor: Some(Anchor::TopLeft), ..Default::default() }.after_drawing(), Transform::default(), "an anchor that placed the crop goes with it");
    }

    #[test]
    fn keeps_the_aspect_ratio_from_one_side_and_scales_both() {
        assert_eq!(plan(Transform { width: Some(100), ..Default::default() }, 400, 300).resized, (100, 75));
        assert_eq!(plan(Transform { height: Some(100), ..Default::default() }, 400, 300).resized, (133, 100));
        assert_eq!(plan(Transform { width: Some(10), height: Some(20), ..Default::default() }, 400, 300).resized, (10, 20));
        assert_eq!(plan(Transform { scale: Some(0.5), ..Default::default() }, 401, 300).resized, (201, 150));
        assert_eq!(plan(Transform { max_size: Some(256), ..Default::default() }, 1000, 500).resized, (256, 128));
        assert_eq!(plan(Transform { max_size: Some(2048), ..Default::default() }, 1000, 500).resized, (1000, 500), "a size limit never enlarges");
    }

    /// Nearest goes to the nearer power of two, and the larger on a tie, as
    /// Unity's ClosestPowerOfTwo does.
    #[test]
    fn rounds_each_side_to_a_power_of_two() {
        let pow2 = |rule, w, h| plan(Transform { power_of_two: Some(rule), ..Default::default() }, w, h).resized;
        assert_eq!(pow2(PowerOfTwo::Nearest, 600, 768), (512, 1024));
        assert_eq!(pow2(PowerOfTwo::Nearest, 700, 1), (512, 1));
        assert_eq!(pow2(PowerOfTwo::Up, 600, 512), (1024, 512));
        assert_eq!(pow2(PowerOfTwo::Down, 600, 1023), (512, 512));
        let both = plan(Transform { max_size: Some(1000), power_of_two: Some(PowerOfTwo::Up), ..Default::default() }, 3000, 1500).resized;
        assert_eq!(both, (1024, 512), "the size limit comes before the power of two");
    }

    /// Shrinking defaults to Box and enlarging to Bicubic, each axis on its
    /// own; an axis that keeps its size is not resampled.
    #[test]
    fn picks_a_filter_for_each_axis() {
        assert_eq!(plan(Transform { width: Some(50), height: Some(400), ..Default::default() }, 100, 100).filters, (Some(ResizeFilter::Box), Some(ResizeFilter::Bicubic)));
        assert_eq!(plan(Transform { width: Some(100), height: Some(50), filter: Some(ResizeFilter::Lanczos), ..Default::default() }, 100, 100).filters, (None, Some(ResizeFilter::Lanczos)));
    }

    #[test]
    fn crops_a_region_a_size_by_its_anchor_and_margins() {
        let crop = |c, a| plan(Transform { crop: Some(c), anchor: a, ..Default::default() }, 10, 8);
        assert_eq!(crop(Crop::Region { x: 2, y: 3, width: 4, height: 5 }, None).map.at(0, 0), (2, 3));
        assert_eq!(crop(Crop::Size { width: 4, height: 4 }, None).map.at(0, 0), (3, 2), "the middle, by default");
        assert_eq!(crop(Crop::Size { width: 4, height: 4 }, Some(Anchor::BottomRight)).map.at(0, 0), (6, 4));
        assert_eq!(crop(Crop::Size { width: 3, height: 3 }, Some(Anchor::Top)).map.at(0, 0), (3, 0), "the odd texel of a middle goes after it");
        let margins = crop(Crop::Margins { left: 1, top: 2, right: 3, bottom: 4 }, None);
        assert_eq!((margins.map.at(0, 0), margins.oriented), ((1, 2), (6, 2)));
    }

    /// Padding grows the canvas around the image, placed by the anchor.
    #[test]
    fn pads_around_the_image_where_the_anchor_places_it() {
        let pad = |a| plan(Transform { pad: Some((10, 7)), anchor: a, pad_color: Some([1.0, 0.0, 0.0, 1.0]), ..Default::default() }, 4, 4);
        assert_eq!((pad(None).canvas, pad(None).offset), ((10, 7), (3, 1)));
        assert_eq!(pad(Some(Anchor::TopLeft)).offset, (0, 0));
        assert_eq!(pad(Some(Anchor::Right)).offset, (6, 1));
        assert_eq!(pad(None).color, [1.0, 0.0, 0.0, 1.0]);
    }

    /// The map rotates and flips as the image crate's rotate90, rotate180,
    /// rotate270, fliph and flipv do, alone and after a crop, a rotation
    /// coming before a flip.
    #[test]
    fn maps_texels_as_the_image_crates_rotations_and_flips() {
        use image::imageops;
        let source = picture(5, 3);
        let shown = |t: Transform| mapped(&source, &plan(t, 5, 3));
        let turn = |degrees| Transform { rotate: Some(degrees), ..Default::default() };
        assert_eq!(shown(turn(90)), imageops::rotate90(&source));
        assert_eq!(shown(turn(180)), imageops::rotate180(&source));
        assert_eq!(shown(turn(270)), imageops::rotate270(&source));
        assert_eq!(shown(Transform { flip: Some(FlipDirection::Horizontal), ..Default::default() }), imageops::flip_horizontal(&source));
        assert_eq!(shown(Transform { flip: Some(FlipDirection::Vertical), ..Default::default() }), imageops::flip_vertical(&source));
        let cropped = imageops::crop_imm(&source, 1, 1, 3, 2).to_image();
        let all = Transform { crop: Some(Crop::Region { x: 1, y: 1, width: 3, height: 2 }), rotate: Some(90), flip: Some(FlipDirection::Horizontal), ..Default::default() };
        assert_eq!(shown(all), imageops::flip_horizontal(&imageops::rotate90(&cropped)));
    }

    /// What a transform cannot do is refused before any GPU work, with the
    /// id that names it.
    #[test]
    fn refuses_what_a_texture_cannot_take() {
        let check = |t: Transform, dimension, cube, w, h| refused(t.plan(w, h, dimension, cube)).error_id;
        let d1 = wgpu::TextureDimension::D1;
        assert_eq!(check(Transform { rotate: Some(90), ..Default::default() }, D2, true, 8, 8), "WgpuTransform", "a cube map is not rotated");
        assert_eq!(check(Transform { flip: Some(FlipDirection::Vertical), ..Default::default() }, D2, true, 8, 8), "WgpuTransform");
        assert_eq!(check(Transform { width: Some(8), height: Some(4), ..Default::default() }, D2, true, 8, 8), "WgpuTransform", "a cube map's faces stay square");
        assert_eq!(check(Transform { rotate: Some(90), ..Default::default() }, d1, false, 8, 1), "WgpuTransform", "a 1D texture stays one texel high");
        assert_eq!(check(Transform { height: Some(2), ..Default::default() }, d1, false, 8, 1), "WgpuTransform");
        assert_eq!(check(Transform { pad: Some((8, 2)), ..Default::default() }, d1, false, 8, 1), "WgpuTransform");
        assert_eq!(check(Transform { rotate: Some(45), ..Default::default() }, D2, false, 8, 8), "WgpuRotate");
        assert_eq!(check(Transform { crop: Some(Crop::Region { x: 6, y: 0, width: 4, height: 4 }), ..Default::default() }, D2, false, 8, 8), "WgpuCrop");
        assert_eq!(check(Transform { crop: Some(Crop::Margins { left: 4, top: 0, right: 4, bottom: 0 }), ..Default::default() }, D2, false, 8, 8), "WgpuCrop");
        assert_eq!(check(Transform { pad: Some((4, 9)), ..Default::default() }, D2, false, 8, 8), "WgpuPad");
        assert_eq!(check(Transform { pad_color: Some([0.0; 4]), ..Default::default() }, D2, false, 8, 8), "WgpuPad");
        assert_eq!(check(Transform { filter: Some(ResizeFilter::Box), ..Default::default() }, D2, false, 8, 8), "WgpuResize");
        assert_eq!(check(Transform { scale: Some(2.0), width: Some(4), ..Default::default() }, D2, false, 8, 8), "WgpuResize");
        assert_eq!(check(Transform { anchor: Some(Anchor::Top), ..Default::default() }, D2, false, 8, 8), "WgpuTransform");
        let premultiplied = Transform { color: Color { premultiply: true, ..Default::default() }, border: Some(Border::default()), ..Default::default() };
        assert_eq!(check(premultiplied, D2, false, 8, 8), "WgpuBorder", "a premultiplied texel with alpha 0 stays black");
        assert_eq!(refused(alpha::border_of(false, Some(2))).error_id, "WgpuBorder", "-BorderReach limits -FixAlphaBorder");
        assert_eq!(refused(alpha::border_of(true, Some(0))).error_id, "WgpuBorder");
        // A 1D texture keeps its height as its width changes.
        assert_eq!(Transform { scale: Some(2.0), ..Default::default() }.plan(8, 1, d1, false).expect("planned").resized, (16, 1));
    }

    /// `texels` of an Rgba8Unorm image drawn through `t` on the GPU.
    fn drawn(core: &CoreRef, source: &image::RgbaImage, t: Transform) -> image::RgbaImage {
        let out = apply(core, file_of(source), 1, convert::MipOptions::default(), &t, Packed::Own, None, None, None, &mut Said::default()).expect("transformed");
        assert_eq!(out.format, F::Rgba8Unorm);
        image::RgbaImage::from_raw(out.width, out.height, out.levels[0].clone()).expect("an image")
    }

    /// The largest difference of any channel between two images of one size.
    fn worst(a: &image::RgbaImage, b: &image::RgbaImage) -> u8 {
        assert_eq!(a.dimensions(), b.dimensions());
        a.as_raw().iter().zip(b.as_raw()).map(|(x, y)| x.abs_diff(*y)).max().expect("texels")
    }

    /// Each filter resizes as the image crate's resize does with the same
    /// filter, shrinking and enlarging, within one step of 8 bits: the GPU
    /// keeps the result in 16-bit floats before packing it.
    #[test]
    fn resizes_as_the_image_crate_does_with_each_filter() {
        let Some(core) = core() else { return };
        let source = picture(23, 17);
        for (ours, theirs) in [
            (ResizeFilter::Nearest, image::imageops::FilterType::Nearest),
            (ResizeFilter::Bilinear, image::imageops::FilterType::Triangle),
            (ResizeFilter::Bicubic, image::imageops::FilterType::CatmullRom),
            (ResizeFilter::Lanczos, image::imageops::FilterType::Lanczos3),
        ] {
            for (w, h) in [(9, 6), (40, 31), (7, 30)] {
                let got = drawn(&core, &source, Transform { width: Some(w), height: Some(h), filter: Some(ours), ..Default::default() });
                let expected = image::imageops::resize(&source, w, h, theirs);
                assert!(worst(&got, &expected) <= 1, "{ours:?} to {w}x{h} differs by {}", worst(&got, &expected));
            }
        }
    }

    /// Box shrinks to the average of the texels each new texel covers,
    /// each weighed by how much of it is covered, worked out exactly.
    #[test]
    fn shrinks_with_box_to_the_area_average() {
        let Some(core) = core() else { return };
        let source = picture(10, 7);
        let (w, h) = (4u32, 3u32);
        let got = drawn(&core, &source, Transform { width: Some(w), height: Some(h), ..Default::default() });
        let (rx, ry) = (10.0 / f64::from(w), 7.0 / f64::from(h));
        let cover = |i: u32, j: u32, r: f64| (f64::from(j + 1).min(f64::from(i + 1) * r) - f64::from(j).max(f64::from(i) * r)).max(0.0);
        for y in 0..h {
            for x in 0..w {
                for c in 0..4 {
                    let mut sum = 0.0;
                    for sy in 0..7 {
                        for sx in 0..10 {
                            sum += cover(x, sx, rx) * cover(y, sy, ry) * f64::from(source.get_pixel(sx, sy)[c]);
                        }
                    }
                    let expected = (sum / (rx * ry)).round() as u8;
                    let value = got.get_pixel(x, y)[c];
                    assert!(value.abs_diff(expected) <= 1, "texel ({x}, {y}) channel {c}: {value} for {expected}");
                }
            }
        }
    }

    /// Crops, rotations, flips and padding move texels without changing
    /// them, so the GPU's result is the CPU's exactly.
    #[test]
    fn moves_texels_exactly() {
        let Some(core) = core() else { return };
        let source = picture(6, 4);
        let t = Transform { crop: Some(Crop::Region { x: 1, y: 0, width: 4, height: 3 }), rotate: Some(270), flip: Some(FlipDirection::Vertical), ..Default::default() };
        assert_eq!(drawn(&core, &source, t), mapped(&source, &plan(t, 6, 4)));
        let padded = drawn(&core, &source, Transform { pad: Some((8, 7)), anchor: Some(Anchor::BottomRight), pad_color: Some([1.0, 0.0, 0.0, 1.0]), ..Default::default() });
        for (x, y, texel) in padded.enumerate_pixels() {
            let expected = if x >= 2 && y >= 3 { *source.get_pixel(x - 2, y - 3) } else { image::Rgba([255, 0, 0, 255]) };
            assert_eq!(*texel, expected, "texel ({x}, {y})");
        }
    }

    /// A texture keeps as many mip levels as the new size's chain has,
    /// each rendered again below the new level 0; every layer is
    /// transformed.
    #[test]
    fn keeps_mip_levels_up_to_the_new_chain_and_every_layer() {
        let Some(core) = core() else { return };
        let layer = |v: u8| vec![v; 16 * 16 * 4];
        let file = TextureFile { dimension: D2, width: 16, height: 16, depth: 1, layers: 2, cube: false, format: F::Rgba8Unorm, levels: vec![[layer(40), layer(200)].concat()] };
        let mut said = Said::default();
        let out = apply(&core, file, 5, convert::MipOptions::default(), &Transform { width: Some(4), height: Some(2), ..Default::default() }, Packed::Own, None, None, None, &mut said).expect("transformed");
        assert_eq!((out.width, out.height, out.layers, out.levels.len()), (4, 2, 2, 3));
        assert!(out.levels[2][..4].iter().all(|&v| v == 40) && out.levels[2][4..].iter().all(|&v| v == 200), "{:?}", out.levels[2]);
        assert_eq!(
            said.verbose,
            ["the texture was resized to 4x2 with the Box filter on the GPU in Rgba16Float", "mip levels 1 to 2 were rendered again on the GPU, each from the level above it", "the texels were packed into Rgba8Unorm on the CPU"]
        );
    }

    /// A BC7 texture is decoded, rotated and encoded again in BC7: nearer
    /// the rotated source than the source, and within twice the loss of
    /// one encoding, as the transform encodes a second time. How near BC7
    /// comes depends on the GPU running the encoder, so the test compares
    /// rather than bounds.
    #[test]
    fn encodes_a_bc_texture_again_in_its_format() {
        let Some(core) = core() else { return };
        let source = gradient(8, 4);
        let file = file_of(&source);
        let drawn = convert::float_levels(&core, &file, F::Rgba16Float, 1, 1, convert::MipOptions::default(), None).expect("drawn");
        let (blocks, _) = convert::encode_bc(&core, &drawn, F::Bc7RgbaUnorm, None, true, 0, None).expect("encoded");
        let bc7 = || TextureFile { format: F::Bc7RgbaUnorm, levels: blocks.clone(), ..file_of(&source) };
        let one_trip = mean(&decoded(bc7()), &source);
        let mut said = Said::default();
        let out = apply(&core, bc7(), 1, convert::MipOptions::default(), &Transform { rotate: Some(180), ..Default::default() }, Packed::Own, None, None, None, &mut said).expect("rotated");
        assert_eq!(out.format, F::Bc7RgbaUnorm);
        assert!(said.verbose.iter().any(|l| l.starts_with("the texels were encoded again into Bc7RgbaUnorm on the GPU by block_compression")), "{said:?}");
        let back = decoded(out);
        let turned = image::imageops::rotate180(&source);
        let (toward, away) = (mean(&back, &turned), mean(&back, &source));
        assert!(toward < away, "the rotated texture is {toward} from the rotated source and {away} from the source");
        assert!(toward <= 2.0 * one_trip + 1.0, "two BC7 encodings moved a channel {toward} on average, and one {one_trip}");
    }

    /// A color key on an opaque texture encoded again into BC7 keeps the
    /// alpha it cleared, as the encoder is not told the texels are opaque.
    #[test]
    fn keys_an_opaque_bc_texture_and_keeps_the_cleared_alpha() {
        let Some(core) = core() else { return };
        let mut source = gradient(8, 4);
        for (x, y) in (0..4).flat_map(|y| (0..4).map(move |x| (x, y))) {
            source.put_pixel(x, y, image::Rgba([255, 0, 255, 255]));
        }
        let file = file_of(&source);
        let drawn = convert::float_levels(&core, &file, F::Rgba16Float, 1, 1, convert::MipOptions::default(), None).expect("drawn");
        let (blocks, _) = convert::encode_bc(&core, &drawn, F::Bc7RgbaUnorm, None, true, 0, None).expect("encoded");
        let bc7 = TextureFile { format: F::Bc7RgbaUnorm, levels: blocks, ..file_of(&source) };
        // A BC7 block can move a channel a step from the color it encodes.
        let key = Color { key: Some(color::ColorKey { rgb: [1.0, 0.0, 1.0], tolerance: 0.02 }), ..Default::default() };
        let out = apply(&core, bc7, 1, convert::MipOptions::default(), &Transform { color: key, ..Default::default() }, Packed::Own, None, None, None, &mut Said::default()).expect("keyed");
        assert_eq!(out.format, F::Bc7RgbaUnorm);
        for (x, y, texel) in decoded(out).enumerate_pixels() {
            let want = if x < 4 && y < 4 { 0 } else { 255 };
            assert_eq!(texel[3], want, "texel ({x}, {y}) has alpha {}", texel[3]);
        }
    }

    /// The mean difference of the channels of two images of one size.
    fn mean(a: &image::RgbaImage, b: &image::RgbaImage) -> f64 {
        assert_eq!(a.dimensions(), b.dimensions());
        a.as_raw().iter().zip(b.as_raw()).map(|(x, y)| f64::from(x.abs_diff(*y))).sum::<f64>() / a.as_raw().len() as f64
    }

    /// `file`, block-compressed, decoded on the CPU into an image.
    fn decoded(file: TextureFile) -> image::RgbaImage {
        let (width, height) = (file.width, file.height);
        let file = texfile::uncompressed(file, &mut Said::default()).expect("decoded");
        image::RgbaImage::from_raw(width, height, file.levels[0].clone()).expect("an image")
    }

    /// An ETC2 texture goes through Basis Universal and comes back ETC2,
    /// flipped: nearer the flipped source than the source, and within twice
    /// the loss of one trip through Basis Universal and back, as the
    /// transform makes a second trip. A signed EAC texture, which no
    /// encoder here writes, comes back uncompressed with a warning.
    #[test]
    fn encodes_etc2_through_basis_and_leaves_signed_eac_uncompressed() {
        let Some(core) = core() else { return };
        let source = gradient(8, 8);
        let mut said = Said::default();
        let bytes = image_io::encode_basis(&core, file_of(&source), BASIS_FORMATS[1], Some(1), convert::MipOptions::default(), None, None, false, None, &mut said).expect("encoded");
        let etc2 = || texfile::transcode_ktx2(&bytes, target::ETC2_RGBA, F::Etc2Rgba8Unorm, 1, &mut Said::default()).expect("transcoded");
        let one_trip = mean(&decoded(etc2()), &source);
        let out = apply(&core, etc2(), 1, convert::MipOptions::default(), &Transform { flip: Some(FlipDirection::Horizontal), ..Default::default() }, Packed::Own, None, None, None, &mut said).expect("flipped");
        assert_eq!(out.format, F::Etc2Rgba8Unorm);
        assert!(said.verbose.iter().any(|l| l == "the texels were encoded again into Etc2Rgba8Unorm through Basis Universal's BasisUastc4x4"), "{said:?}");
        let back = decoded(out);
        let flipped = image::imageops::flip_horizontal(&source);
        let (toward, away) = (mean(&back, &flipped), mean(&back, &source));
        assert!(toward < away, "the flipped texture is {toward} from the flipped source and {away} from the source");
        assert!(toward <= 2.0 * one_trip + 1.0, "two trips through Basis Universal moved a channel {toward} on average, and one trip {one_trip}");

        let signed = TextureFile { dimension: D2, width: 4, height: 4, depth: 1, layers: 1, cube: false, format: F::EacR11Snorm, levels: vec![vec![0; 8]] };
        let mut said = Said::default();
        let out = apply(&core, signed, 1, convert::MipOptions::default(), &Transform { flip: Some(FlipDirection::Vertical), ..Default::default() }, Packed::Own, None, None, None, &mut said).expect("flipped");
        assert_eq!(out.format, F::R8Snorm);
        assert!(said.warnings.iter().any(|w| w.starts_with("the module has no encoder for EacR11Snorm")), "{said:?}");
    }

    /// A cutout of `width` by `height` texels, each at most 256: disks, a
    /// ring and scattered specks with alpha above 0, the specks' alpha from
    /// 1 to 254, each texel colored by its position (red x, green y, blue a
    /// mix of both); the rest alpha 0 and colored 1, 2, 3. `salt` places
    /// the shapes.
    fn cutout(width: u32, height: u32, salt: u64) -> image::RgbaImage {
        let mut state = salt;
        let mut next = move || {
            state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
            let mut z = state;
            z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
            z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
            z ^ (z >> 31)
        };
        let mut place = |r: u64| ((next() % u64::from(width)) as f64, (next() % u64::from(height)) as f64, (2 + next() % r) as f64);
        let disks: Vec<(f64, f64, f64)> = (0..5).map(|_| place(9)).collect();
        let ring = place(12);
        image::RgbaImage::from_fn(width, height, |x, y| {
            let d = |(cx, cy, _): (f64, f64, f64)| (f64::from(x) - cx).hypot(f64::from(y) - cy);
            let shape = disks.iter().any(|&disk| d(disk) <= disk.2) || (d(ring) <= ring.2 + 2.0 && d(ring) > ring.2);
            let speck = next() % 97 == 0;
            let alpha = if shape { 255 } else if speck { 1 + (next() % 254) as u8 } else { 0 };
            if alpha == 0 { image::Rgba([1, 2, 3, 0]) } else { image::Rgba([x as u8, y as u8, ((x * 7 + y * 13) % 256) as u8, alpha]) }
        })
    }

    /// Checks `out`, `source` with its border fixed within `reach`: each
    /// texel with alpha above 0 is unchanged; each texel with alpha 0
    /// within the reach of one holds the color of a texel with alpha above
    /// 0 at the least distance any has, by a search of every texel, and
    /// alpha 0; the rest keep their color.
    fn check_border(source: &image::RgbaImage, out: &image::RgbaImage, reach: Option<u32>, what: &str) {
        let seeds: Vec<(i64, i64)> = source.enumerate_pixels().filter(|(_, _, p)| p[3] > 0).map(|(x, y, _)| (i64::from(x), i64::from(y))).collect();
        let d2 = |(ax, ay): (i64, i64), (bx, by): (i64, i64)| (ax - bx).pow(2) + (ay - by).pow(2);
        for (x, y, texel) in source.enumerate_pixels() {
            let got = out.get_pixel(x, y);
            if texel[3] > 0 {
                assert_eq!(got, texel, "{what}: ({x}, {y}) has alpha above 0 and changed");
                continue;
            }
            let at = (i64::from(x), i64::from(y));
            let least = seeds.iter().map(|&s| d2(s, at)).min();
            let within = match (least, reach) {
                (None, _) => false,
                (Some(_), None) => true,
                (Some(least), Some(r)) => least <= i64::from(r).pow(2),
            };
            if !within {
                assert_eq!(got, texel, "{what}: ({x}, {y}) is out of reach and changed");
                continue;
            }
            let from = (u32::from(got[0]), u32::from(got[1]));
            assert!(got[3] == 0 && from.0 < source.width() && from.1 < source.height(), "{what}: ({x}, {y}) holds {got:?}, no texel's color");
            let seed = source.get_pixel(from.0, from.1);
            assert!(seed[3] > 0 && seed.0[..3] == got.0[..3], "{what}: ({x}, {y}) holds {got:?}, not the color of {from:?}, {seed:?}");
            assert_eq!(Some(d2((i64::from(from.0), i64::from(from.1)), at)), least, "{what}: ({x}, {y}) took the color of {from:?}, not of a nearest texel");
        }
    }

    /// Every transparent texel takes the color of a nearest texel with
    /// alpha above 0, checked against a search of every texel, and with
    /// -BorderReach only those within the reach do.
    #[test]
    fn colors_each_transparent_texel_from_a_nearest_texel() {
        let Some(core) = core() else { return };
        let source = cutout(160, 120, 7);
        for reach in [None, Some(3)] {
            let mut said = Said::default();
            let t = Transform { border: Some(Border { reach }), ..Default::default() };
            let out = apply(&core, file_of(&source), 1, convert::MipOptions::default(), &t, Packed::Own, None, None, None, &mut said).expect("fixed");
            let out = image::RgbaImage::from_raw(out.width, out.height, out.levels[0].clone()).expect("an image");
            check_border(&source, &out, reach, &format!("reach {reach:?}"));
            assert_eq!(said.verbose.first(), Some(&Border { reach }.said()), "{said:?}");
        }
    }

    /// Each layer of an array and each depth slice of a 3D texture is fixed
    /// on its own; a texture with no alpha channel is refused.
    #[test]
    fn fixes_the_border_of_every_layer_and_depth_slice() {
        let Some(core) = core() else { return };
        let (width, height) = (61, 47);
        let planes = [cutout(width, height, 11), cutout(width, height, 12)];
        let bytes = [planes[0].as_raw().as_slice(), planes[1].as_raw().as_slice()].concat();
        let t = Transform { border: Some(Border::default()), ..Default::default() };
        let file = |dimension, depth, layers| TextureFile { dimension, width, height, depth, layers, cube: false, format: F::Rgba8Unorm, levels: vec![bytes.clone()] };
        for (what, file) in [("layer", file(D2, 1, 2)), ("depth slice", file(wgpu::TextureDimension::D3, 2, 1))] {
            let out = apply(&core, file, 1, convert::MipOptions::default(), &t, Packed::Own, None, None, None, &mut Said::default()).expect("fixed");
            let plane = (width * height * 4) as usize;
            for (index, source) in planes.iter().enumerate() {
                let fixed = image::RgbaImage::from_raw(width, height, out.levels[0][index * plane..(index + 1) * plane].to_vec()).expect("an image");
                check_border(source, &fixed, None, &format!("{what} {index}"));
            }
        }
        let gray = TextureFile { dimension: D2, width: 4, height: 4, depth: 1, layers: 1, cube: false, format: F::R8Unorm, levels: vec![vec![0; 16]] };
        assert_eq!(refused(apply(&core, gray, 1, convert::MipOptions::default(), &t, Packed::Own, None, None, None, &mut Said::default())).error_id, "WgpuAlpha");
    }
}
