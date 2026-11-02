//! A Photoshop document's layers merged into one picture, as psd-tools
//! 1.26.0's compositor merges them, which follows the PDF transparency
//! model: each layer's color, its shape and its alpha go onto the canvas
//! below by its blend mode, the shapes and alphas joined by union. A group
//! merges on a canvas of its own, which then goes on as one layer, or,
//! passing through, onto the canvas below; a clipped layer paints onto the
//! layer it clips to; a layer's mask, the mask's density, the layer's
//! opacity and its fill opacity scale what it covers; a knockout layer
//! punches through to its group's backdrop, or the Background layer's. A
//! CMYK document merges on its samples as stored, each ink's complement,
//! as psd-tools found matches Photoshop's numbers, and a 32-bit document's
//! light is kept above 1.
//!
//! A layer that stores no pixels is drawn from its fill over its box and
//! through its vector mask: a solid color or a pattern as psd-tools draws
//! them, and a gradient as Photoshop draws it, by psd_gradient's rules; a
//! color in another mode than the document's converts into the document's
//! samples by the conversion the merge is given. What the merge does not
//! draw gives a warning naming the layer: an adjustment layer, a gradient
//! psd_gradient does not draw, a vector stroke, a color of another mode
//! that does not convert, a pattern of another mode or at a scale, angle or
//! phase other than its own, layer effects, a mask's feather, Blend If
//! ranges, and dissolve's noise, which is drawn as normal. A vector mask
//! whose flags invert it covers 1 minus its paths, which psd-tools leaves
//! out.

use crate::psd::{self, Block, Color, Header, LayerSection, Mode, Picture, Reader, Resources, Samples};
use crate::psd_descriptor::{self, Descriptor, Value};
use crate::psd_fill::{self, Colors, Solid};
use crate::psd_composite8;
use crate::psd_gradient::{self, Setting};
use crate::psd_raster;
use crate::psd_stroke;
use crate::psd_vector;
use std::sync::atomic::{AtomicBool, Ordering};

/// A merged picture and what the merge left out, for the warnings.
pub(crate) struct Merged {
    pub(crate) picture: Picture,
    pub(crate) warnings: Vec<String>,
}

/// A rectangle in the document, by its edges.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Rect {
    top: i32,
    left: i32,
    bottom: i32,
    right: i32,
}

impl Rect {
    fn read(r: &mut Reader) -> Result<Rect, String> {
        Ok(Rect { top: r.signed32()?, left: r.signed32()?, bottom: r.signed32()?, right: r.signed32()? })
    }

    fn width(&self) -> usize {
        (i64::from(self.right) - i64::from(self.left)).max(0) as usize
    }

    fn height(&self) -> usize {
        (i64::from(self.bottom) - i64::from(self.top)).max(0) as usize
    }
}

/// One of a layer's channels: its id, 0 and up its colors, -1 its
/// transparency, -2 its user mask and -3 its real user mask, its data past
/// the 2-byte compression, and, once `prepare` has run, its samples
/// decompressed or why they cannot be.
struct Channel<'a> {
    id: i16,
    compression: u64,
    data: &'a [u8],
    decompressed: Option<Result<Vec<u8>, String>>,
}

/// A layer's user mask: its rectangle, its value outside it, whether it is
/// disabled, its densities and feather, and the real user mask Photoshop
/// stores beside a vector mask, its rectangle and value outside it, where
/// its flags say it applies.
struct Mask {
    rect: Rect,
    default: u8,
    disabled: bool,
    /// Whether a real user mask's flags were stored, applying or not.
    real_stored: bool,
    real: Option<(Rect, u8)>,
    density: Option<u8>,
    vector_density: Option<u8>,
    feathered: bool,
}

/// A layer record and its channels' data.
struct Record<'a> {
    rect: Rect,
    channels: Vec<Channel<'a>>,
    blend: [u8; 4],
    opacity: u8,
    clipping: bool,
    hidden: bool,
    /// Whether the flags' fifth bit says the pixels are irrelevant to how
    /// the document looks, as a shape layer's are.
    pixels_irrelevant: bool,
    mask: Option<Mask>,
    ranges: &'a [u8],
    name: String,
    blocks: Vec<([u8; 4], &'a [u8])>,
    /// Whether the mask repeats the transparency, once `prepare` has
    /// worked it out.
    repeats_shape: Option<Result<bool, String>>,
    /// The box of the pixels the vector mask covers enough to show, once
    /// `prepare` has worked it out for a shape a gradient is laid out over.
    visible_box: Option<Result<Option<[i64; 4]>, String>>,
}

/// The tagged block keys of adjustment layers, and what each adjusts.
const ADJUSTMENTS: [(&[u8; 4], &str); 17] = [
    (b"levl", "levels"),
    (b"curv", "curves"),
    (b"brit", "brightness and contrast"),
    (b"blnc", "color balance"),
    (b"hue2", "hue and saturation"),
    (b"hue ", "hue and saturation"),
    (b"selc", "selective color"),
    (b"mixr", "channel mixer"),
    (b"grdm", "gradient map"),
    (b"phfl", "photo filter"),
    (b"expA", "exposure"),
    (b"vibA", "vibrance"),
    (b"thrs", "threshold"),
    (b"post", "posterize"),
    (b"nvrt", "invert"),
    (b"blwh", "black and white"),
    (b"clrL", "color lookup"),
];

/// The tagged block keys of fills, and what each fills with.
const FILLS: [(&[u8; 4], &str); 3] = [(b"SoCo", "solid color"), (b"GdFl", "gradient"), (b"PtFl", "pattern")];

/// What a fill fills with.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum FillKind {
    Solid,
    Pattern,
    Gradient,
}

impl<'a> Record<'a> {
    fn block(&self, key: &[u8; 4]) -> Option<&'a [u8]> {
        self.blocks.iter().find(|(k, _)| k == key).map(|&(_, b)| b)
    }

    fn channel(&self, id: i16) -> Option<&Channel<'a>> {
        self.channels.iter().find(|c| c.id == id && !c.data.is_empty())
    }

    /// Whether any color channel holds data, as psd-tools' has_pixels asks.
    fn has_pixels(&self) -> bool {
        self.channels.iter().any(|c| c.id >= 0 && !c.data.is_empty())
    }

    /// The section divider's type: 1 or 2 a group's record, 3 the end of a
    /// group's layers below it, 0 any other layer.
    fn divider(&self) -> u32 {
        match self.block(b"lsct").or_else(|| self.block(b"lsdk")) {
            Some(data) if data.len() >= 4 => u32::from_be_bytes([data[0], data[1], data[2], data[3]]),
            _ => 0,
        }
    }

    /// The blend mode: a group's from its section divider where it gives
    /// one, as Photoshop writes pass through there.
    fn blend_mode(&self) -> [u8; 4] {
        match self.block(b"lsct").or_else(|| self.block(b"lsdk")) {
            Some(data) if data.len() >= 12 && &data[4..8] == b"8BIM" => [data[8], data[9], data[10], data[11]],
            _ => self.blend,
        }
    }

    /// The fill opacity, iOpa, from 0 to 1.
    fn fill_opacity(&self) -> f32 {
        match self.block(b"iOpa") {
            Some(data) if !data.is_empty() => f32::from(data[0]) / 255.0,
            _ => 1.0,
        }
    }

    /// The knockout, knko: 0 none, 1 shallow, 2 deep.
    fn knockout(&self) -> u8 {
        match self.block(b"knko") {
            Some(data) if !data.is_empty() => data[0],
            _ => 0,
        }
    }

    fn adjustment(&self) -> Option<&'static str> {
        ADJUSTMENTS.iter().find(|(key, _)| self.block(key).is_some()).map(|&(_, what)| what)
    }

    fn fill(&self) -> Option<&'static str> {
        FILLS.iter().find(|(key, _)| self.block(key).is_some()).map(|&(_, what)| what)
    }

    /// Whether the layer is filled from settings, as psd-tools' has_fill
    /// asks: a fill, or a shape's vector stroke content.
    fn has_fill(&self) -> bool {
        self.fill().is_some() || self.block(b"vscg").is_some()
    }

    /// The vector mask's data, vmsk or vsms, where it is not disabled: a
    /// version, then flags whose third bit disables it.
    fn vector_mask(&self) -> Option<&'a [u8]> {
        let data = self.block(b"vmsk").or_else(|| self.block(b"vsms"))?;
        let flags = data.get(4..8).map(|f| u32::from_be_bytes([f[0], f[1], f[2], f[3]]))?;
        (flags & 4 == 0).then_some(data)
    }

    /// Whether the layer holds a vector mask block, disabled or not.
    fn has_vector_mask(&self) -> bool {
        self.block(b"vmsk").is_some() || self.block(b"vsms").is_some()
    }

    /// Whether psd-tools takes the layer for a shape layer: its pixels
    /// irrelevant, and an origination, a vector mask or a vector stroke
    /// among its blocks.
    fn is_shape(&self) -> bool {
        self.pixels_irrelevant && [b"vogk", b"vmsk", b"vsms", b"vstk", b"vscg"].iter().any(|key| self.block(key).is_some())
    }

    /// The vector stroke's settings, its vstk block's descriptor.
    fn stroke(&self) -> Result<Option<Descriptor>, String> {
        self.block(b"vstk").map(psd_descriptor::versioned).transpose()
    }

    /// The fill's kind and settings: a SoCo, PtFl or GdFl block's
    /// descriptor, or, where the layer has none, a shape's vscg, whose
    /// descriptor holds a color, a pattern or a gradient, as psd-tools'
    /// create_fill looks for them.
    fn fill_settings(&self) -> Result<Option<(FillKind, Descriptor)>, String> {
        for (key, kind) in [(b"SoCo", FillKind::Solid), (b"PtFl", FillKind::Pattern), (b"GdFl", FillKind::Gradient)] {
            if let Some(data) = self.block(key) {
                return Ok(Some((kind, psd_descriptor::versioned(data)?)));
            }
        }
        let Some(data) = self.block(b"vscg") else { return Ok(None) };
        let d = psd_descriptor::descriptor(&mut Reader::new(data.get(8..).ok_or("its shape's fill settings are cut short")?))?;
        let kind = if d.get("Clr ").is_some() {
            FillKind::Solid
        } else if d.get("Ptrn").is_some() {
            FillKind::Pattern
        } else if d.get("Grad").is_some() {
            FillKind::Gradient
        } else {
            return Ok(None);
        };
        Ok(Some((kind, d)))
    }

    /// Whether the record has a real user mask its flags say applies.
    fn has_real(&self) -> bool {
        self.mask.as_ref().is_some_and(|m| m.real.is_some())
    }

    /// The names of the layer's enabled effects, or why they cannot be
    /// read.
    fn effects(&self) -> Result<Vec<&'static str>, String> {
        const NAMES: [(&str, &str); 15] = [
            ("DrSh", "drop shadow"),
            ("IrSh", "inner shadow"),
            ("OrGl", "outer glow"),
            ("IrGl", "inner glow"),
            ("ebbl", "bevel and emboss"),
            ("SoFi", "color overlay"),
            ("GrFl", "gradient overlay"),
            ("patternFill", "pattern overlay"),
            ("FrFX", "stroke"),
            ("ChFX", "satin"),
            ("dropShadowMulti", "drop shadow"),
            ("innerShadowMulti", "inner shadow"),
            ("solidFillMulti", "color overlay"),
            ("gradientFillMulti", "gradient overlay"),
            ("frameFXMulti", "stroke"),
        ];
        let Some(data) = self.block(b"lmfx").or_else(|| self.block(b"lfx2")) else {
            return Ok(if self.block(b"lrFX").is_some() { vec!["layer effects"] } else { Vec::new() });
        };
        let descriptor = psd_descriptor::versioned(data.get(4..).ok_or("its effects are cut short")?)?;
        if descriptor.flag("masterFXSwitch") == Some(false) {
            return Ok(Vec::new());
        }
        let enabled = |value: &Value| match value {
            Value::Descriptor(d) => d.flag("enab") == Some(true),
            Value::List(items) => items.iter().any(|item| matches!(item, Value::Descriptor(d) if d.flag("enab") == Some(true))),
            _ => false,
        };
        let mut found = Vec::new();
        for (key, value) in &descriptor.items {
            if let Some(&(_, name)) = NAMES.iter().find(|(k, _)| k == key)
                && enabled(value)
                && !found.contains(&name)
            {
                found.push(name);
            }
        }
        Ok(found)
    }

    /// Whether the Blend If ranges let anything less than every value
    /// through: each range 2 black values and 2 white values, every one
    /// 0, 0, 255, 255 where they do not.
    fn blends_if(&self) -> bool {
        self.ranges.as_chunks::<4>().0.iter().any(|range| range != &[0, 0, 255, 255])
    }
}

/// The layer records of the layer info `info`, bottom layer first, and
/// their channels' data.
fn records<'a>(header: &Header, info: &'a [u8]) -> Result<Vec<Record<'a>>, String> {
    if info.len() < 2 {
        return Ok(Vec::new());
    }
    let mut r = Reader::new(info);
    let count = usize::from(r.signed16()?.unsigned_abs());
    let wide = if header.large { 8 } else { 4 };
    let mut records = Vec::with_capacity(count);
    let mut lengths = Vec::with_capacity(count);
    for _ in 0..count {
        let rect = Rect::read(&mut r)?;
        let channel_count = r.length(2)?;
        let mut ids = Vec::with_capacity(channel_count);
        for _ in 0..channel_count {
            let id = r.signed16()?;
            ids.push((id, r.length(wide)?));
        }
        let signature = r.four()?;
        if &signature != b"8BIM" {
            return Err(format!("a layer's blend signature is {:?}, where the specification gives 8BIM", String::from_utf8_lossy(&signature)));
        }
        let blend = r.four()?;
        let opacity = r.take(1)?[0];
        let clipping = r.take(1)?[0] != 0;
        let flags = r.take(1)?[0];
        r.take(1)?;
        let extra_length = r.length(4)?;
        let extra = r.take(extra_length)?;
        let has_real = ids.iter().any(|&(id, _)| id == -3);
        let mut e = Reader::new(extra);
        let mask_length = e.length(4)?;
        let mask = mask(e.take(mask_length)?, has_real)?;
        let ranges_length = e.length(4)?;
        let ranges = e.take(ranges_length)?;
        let name_length = e.length(1)?;
        let pascal: String = e.take(name_length)?.iter().map(|&b| if b.is_ascii() { char::from(b) } else { char::REPLACEMENT_CHARACTER }).collect();
        let pad = (4 - (1 + name_length) % 4) % 4;
        e.take(pad.min(e.left()))?;
        let blocks = psd::tagged_blocks(&extra[e.at..], header.large, 1)?;
        let name = match blocks.iter().find(|(key, _)| key == b"luni") {
            Some((_, data)) => unicode(data)?,
            None => pascal,
        };
        records.push(Record { rect, channels: Vec::new(), blend, opacity, clipping, hidden: flags & 2 != 0, pixels_irrelevant: flags & 16 != 0, mask, ranges, name, blocks, repeats_shape: None, visible_box: None });
        lengths.push(ids);
    }
    for (record, ids) in records.iter_mut().zip(lengths) {
        for (id, length) in ids {
            let data = r.take(length)?;
            let (compression, data) = if length >= 2 { (u64::from(u16::from_be_bytes([data[0], data[1]])), &data[2..]) } else { (0, &data[..0]) };
            record.channels.push(Channel { id, compression, data, decompressed: None });
        }
    }
    Ok(records)
}

/// A Unicode string: a 4-byte count of UTF-16 code units and that many,
/// big-endian, a trailing NUL dropped.
fn unicode(data: &[u8]) -> Result<String, String> {
    let mut r = Reader::new(data);
    let n = r.length(4)?;
    let units: Vec<u16> = r.take(n.checked_mul(2).ok_or("a name does not fit in memory")?)?.as_chunks::<2>().0.iter().map(|c| u16::from_be_bytes(*c)).collect();
    Ok(String::from_utf16_lossy(&units).trim_end_matches('\0').to_string())
}

/// A layer's mask data, as psd-tools reads it: the rectangle, the value
/// outside it and the flags, then, where the layer has a real user mask
/// channel and the block room for it, the real mask's flags, value outside
/// and rectangle, then, where the flags' fifth bit is set, the parameters:
/// a byte of flags for a user density, a user feather, a vector density and
/// a vector feather, each present where its bit is.
fn mask(data: &[u8], has_real: bool) -> Result<Option<Mask>, String> {
    if data.is_empty() {
        return Ok(None);
    }
    let mut r = Reader::new(data);
    let rect = Rect::read(&mut r)?;
    let default = r.take(1)?[0];
    let flags = r.take(1)?[0];
    let mut real = None;
    let mut real_stored = false;
    if has_real && data.len() >= 36 {
        let real_flags = r.take(1)?[0];
        let real_default = r.take(1)?[0];
        let real_rect = Rect::read(&mut r)?;
        real_stored = true;
        if real_flags & 16 != 0 {
            real = Some((real_rect, real_default));
        }
    }
    let (mut density, mut vector_density, mut feathered) = (None, None, false);
    if flags & 16 != 0 {
        let parameters = r.take(1)?[0];
        if parameters & 1 != 0 {
            density = Some(r.take(1)?[0]);
        }
        if parameters & 2 != 0 {
            feathered |= f64::from_bits(r.number(8)?) > 0.0;
        }
        if parameters & 4 != 0 {
            vector_density = Some(r.take(1)?[0]);
        }
        if parameters & 8 != 0 {
            feathered |= f64::from_bits(r.number(8)?) > 0.0;
        }
    }
    Ok(Some(Mask { rect, default, disabled: flags & 2 != 0, real_stored, real, density, vector_density, feathered }))
}

/// A channel's samples over `rect` decompressed, as the document stores
/// them: `rect`'s rows of its width at the document's depth.
fn decompress(header: &Header, channel: &Channel, rect: Rect) -> Result<Vec<u8>, String> {
    let (w, h) = (rect.width(), rect.height());
    if w == 0 || h == 0 {
        return Ok(Vec::new());
    }
    let row = header.row(w)?;
    let mut r = Reader::new(channel.data);
    let mut counts = Vec::new();
    if channel.compression == 1 {
        let n = if header.large { 4 } else { 2 };
        for _ in 0..h {
            counts.push(r.length(n)?);
        }
    }
    psd::decompressed(channel.compression, &channel.data[r.at..], &counts, row, h, header.depth)
}

/// A channel's samples over `rect`, decompressed once by `prepare` or else
/// now.
fn decompressed_of<'c>(header: &Header, channel: &'c Channel, rect: Rect) -> Result<std::borrow::Cow<'c, [u8]>, String> {
    match &channel.decompressed {
        Some(Ok(bytes)) => Ok(std::borrow::Cow::Borrowed(bytes)),
        Some(Err(why)) => Err(why.clone()),
        None => decompress(header, channel, rect).map(std::borrow::Cow::Owned),
    }
}

/// Sample `i` of decompressed samples `bytes` at `depth` bits from 0 to 1:
/// an 8- or 16-bit one over its largest value, a 32-bit one as stored.
fn sample(bytes: &[u8], i: usize, depth: usize) -> f32 {
    match depth {
        8 => f32::from(bytes[i]) / 255.0,
        16 => f32::from(u16::from_be_bytes([bytes[2 * i], bytes[2 * i + 1]])) / 65535.0,
        _ => f32::from_be_bytes([bytes[4 * i], bytes[4 * i + 1], bytes[4 * i + 2], bytes[4 * i + 3]]),
    }
}

/// Why the merge reads no samples of a document's depth, where it reads
/// none: it reads 8, 16 and 32 bits.
fn readable(depth: usize) -> Result<(), String> {
    if matches!(depth, 8 | 16 | 32) { Ok(()) } else { Err(format!("its layers hold {depth}-bit samples, which the merge does not read")) }
}

/// A channel's values from 0 to 1 over `rect`, row by row.
fn rect_values(header: &Header, channel: &Channel, rect: Rect) -> Result<Vec<f32>, String> {
    let (w, h) = (rect.width(), rect.height());
    if w == 0 || h == 0 {
        return Ok(Vec::new());
    }
    let bytes = decompressed_of(header, channel, rect)?;
    readable(header.depth)?;
    Ok((0..w * h).map(|i| sample(&bytes, i, header.depth)).collect())
}

/// Whether a shape or fill layer's mask repeats its transparency pixel for
/// pixel, which psd-tools leaves out rather than square the edge.
fn repeats_shape(header: &Header, record: &Record) -> Result<bool, String> {
    let Some(mask) = &record.mask else { return Ok(false) };
    if !record.has_fill() || !record.has_pixels() || mask.real_stored || mask.rect != record.rect {
        return Ok(false);
    }
    if mask.density.is_some_and(|d| d != 255) || mask.vector_density.is_some_and(|d| d != 255) {
        return Ok(false);
    }
    match (record.channel(-2), record.channel(-1)) {
        (Some(m), Some(t)) => Ok(rect_values(header, m, mask.rect)? == rect_values(header, t, record.rect)?),
        _ => Ok(false),
    }
}

/// Whether the merge lays a gradient out over the pixels the layer's
/// vector mask shows, as `Merge::gradient_box` does for a shape layer that
/// stores no pixels, with its fill on and a gradient aligned with it; or
/// why the layer's settings do not read.
fn shows_gradient_through_mask(header: &Header, record: &Record) -> Result<bool, String> {
    if record.has_pixels() || !record.is_shape() || record.vector_mask().is_none() {
        return Ok(false);
    }
    if record.stroke()?.and_then(|s| s.flag("fillEnabled")) == Some(false) {
        return Ok(false);
    }
    let Some((FillKind::Gradient, settings)) = record.fill_settings()? else {
        return Ok(false);
    };
    Ok(matches!(psd_gradient::gradient_of(&settings, header.mode)?, Setting::Drawn(gradient) if gradient.aligned))
}

/// What every band of the merge reads of the layers `drawn` marks, worked
/// out once with the layers shared among the machine's cores: each channel
/// the merge reads decompressed, with why where it cannot be, kept on the
/// channel until the merge reads it, so a layer it never draws fails
/// nothing; whether each layer's mask repeats its transparency; and the box
/// a shape's vector mask shows where a gradient is laid out over it, with
/// why where the mask or the layer's settings do not read. A layer's colors
/// and transparency cover its box, its user mask the mask's and its real
/// user mask its own, where that applies.
fn prepare(header: &Header, records: &mut [Record], drawn: &[bool]) {
    let colors = header.mode.colors();
    let cores = std::thread::available_parallelism().map_or(1, |n| n.get());
    let share = records.len().div_ceil(cores).max(1);
    std::thread::scope(|scope| {
        for (chunk, drawn) in records.chunks_mut(share).zip(drawn.chunks(share)) {
            scope.spawn(move || {
                for (record, _) in chunk.iter_mut().zip(drawn).filter(|(_, drawn)| **drawn) {
                    for i in 0..record.channels.len() {
                        let id = record.channels[i].id;
                        let rect = match id {
                            0.. if usize::from(id.unsigned_abs()) < colors => Some(record.rect),
                            -1 => Some(record.rect),
                            -2 => record.mask.as_ref().map(|m| m.rect),
                            -3 => record.mask.as_ref().and_then(|m| m.real).map(|(rect, _)| rect),
                            _ => None,
                        };
                        if let Some(rect) = rect
                            && !record.channels[i].data.is_empty()
                        {
                            let decompressed = decompress(header, &record.channels[i], rect);
                            record.channels[i].decompressed = Some(decompressed);
                        }
                    }
                    record.repeats_shape = Some(repeats_shape(header, record));
                    match shows_gradient_through_mask(header, record) {
                        Ok(true) => {
                            if let Some(data) = record.vector_mask() {
                                record.visible_box = Some(psd_vector::read(data).map(|mask| mask.visible_box(header.width, header.height)));
                            }
                        }
                        Ok(false) => {}
                        Err(why) => record.visible_box = Some(Err(why)),
                    }
                }
            });
        }
    });
}

/// Which of the records the merge draws, by their place among them: each
/// one `Compositor::apply` reaches, which passes over a hidden layer and
/// all in it, a clipping layer with nothing to clip to and an adjustment;
/// and the bottom one, which deep knockout reaches.
fn drawn(nodes: &[Node], count: usize) -> Vec<bool> {
    fn mark(node: &Node, clip_compositing: bool, drawn: &mut [bool]) {
        let record = node.record;
        if record.hidden || (!clip_compositing && node.clips_below()) || record.adjustment().is_some() {
            return;
        }
        drawn[node.index] = true;
        for child in node.children.iter().flatten() {
            mark(child, false, drawn);
        }
        for clip in &node.clips {
            mark(clip, true, drawn);
        }
    }
    let mut drawn = vec![false; count];
    if let Some(bottom) = drawn.first_mut() {
        *bottom = true;
    }
    for node in nodes {
        mark(node, false, &mut drawn);
    }
    drawn
}

/// A layer, its place among the records, and what goes on it: the layers
/// of a group, and the layers clipped to it, bottom first.
struct Node<'a> {
    index: usize,
    record: &'a Record<'a>,
    children: Option<Vec<Node<'a>>>,
    clips: Vec<Node<'a>>,
}

/// The layer tree of `records`, bottom first: a group's layers are the
/// records between one whose divider is 3, below them, and the group's own
/// record, of divider 1 or 2, above them.
fn tree<'a>(records: &'a [Record<'a>]) -> Vec<Node<'a>> {
    let mut stack: Vec<Vec<Node<'a>>> = vec![Vec::new()];
    for (index, record) in records.iter().enumerate() {
        match record.divider() {
            3 => stack.push(Vec::new()),
            1 | 2 => {
                let children = if stack.len() > 1 {
                    match stack.pop() {
                        Some(children) => children,
                        None => unreachable!("more than one level is open"),
                    }
                } else {
                    Vec::new()
                };
                let node = Node { index, record, children: Some(clipped(children)), clips: Vec::new() };
                if let Some(level) = stack.last_mut() {
                    level.push(node);
                }
            }
            _ => {
                if let Some(level) = stack.last_mut() {
                    level.push(Node { index, record, children: None, clips: Vec::new() });
                }
            }
        }
    }
    while stack.len() > 1 {
        if let Some(open) = stack.pop()
            && let Some(level) = stack.last_mut()
        {
            level.extend(open);
        }
    }
    match stack.pop() {
        Some(top) => clipped(top),
        None => Vec::new(),
    }
}

/// `nodes` with each run of clipping layers directly above a layer moved
/// onto it, as psd-tools' clip_layers takes them. A group's clipping flag
/// counts for nothing, as Photoshop clips no group, so a group is a layer
/// others can clip to; a clipping layer with no layer to clip to stays
/// where it is, merged nowhere.
fn clipped(nodes: Vec<Node<'_>>) -> Vec<Node<'_>> {
    let mut out: Vec<Node> = Vec::new();
    let mut open = false;
    for node in nodes {
        if !node.clips_below() {
            open = true;
            out.push(node);
        } else if open {
            if let Some(base) = out.last_mut() {
                base.clips.push(node);
            }
        } else {
            out.push(node);
        }
    }
    out
}

impl Node<'_> {
    /// Whether the node clips to the layer below it: a layer with the
    /// clipping flag; never a group.
    fn clips_below(&self) -> bool {
        self.record.clipping && self.children.is_none()
    }
}

/// Color planes over the canvas, one for each of the document's channels.
type Planes = Vec<Vec<f32>>;

/// A fill's color planes and the shape it gives itself, where it gives one.
type Filled = (Planes, Option<Vec<f32>>);

/// What a layer or a group paints: its color, and its shape and alpha over
/// the canvas.
struct Painted {
    color: Planes,
    shape: Vec<f32>,
    alpha: Vec<f32>,
}

/// The document's layers merged: their color and alpha over the canvas,
/// and what the merge left out.
struct Canvas {
    color: Planes,
    alpha: Vec<f32>,
    warnings: Vec<String>,
}

/// A coverage over the canvas, or the same value at every pixel.
#[derive(Clone)]
enum Cover {
    Each(Vec<f32>),
    All(f32),
}

impl Cover {
    fn at(&self, i: usize) -> f32 {
        match self {
            Cover::Each(v) => v[i],
            Cover::All(v) => *v,
        }
    }
}

/// A layer resolved into what the blend equations take, as psd-tools'
/// _Source holds it: its color, its shape with its mask in, its alpha with
/// its mask and opacity in, its mask scaled by its opacity, its fill
/// opacity, its knockout, and whether a group isolated its adjustments.
struct Source {
    color: Vec<Vec<f32>>,
    shape: Vec<f32>,
    alpha: Vec<f32>,
    mask: Cover,
    fill: f32,
    knockout: u8,
    adjustment_isolated: Option<bool>,
}

/// What a merge works on: the document's size and color, the band of its
/// rows it merges, `rows` of them from row `top`, which every plane
/// covers, its global tagged blocks, its ICC profile and the module's color
/// handling, which converts a fill color of another mode into its samples
/// and gives its light, and what it has left out.
struct Merge<'a> {
    header: &'a Header,
    width: usize,
    height: usize,
    top: usize,
    rows: usize,
    channels: usize,
    color: Color,
    blocks: &'a [Block<'a>],
    icc: Option<&'a [u8]>,
    colors: Colors,
    warnings: Vec<String>,
}

impl<'a> Merge<'a> {
    /// A merge of the whole of `header`'s document, with nothing left out
    /// yet.
    fn whole(header: &'a Header, blocks: &'a [Block<'a>], icc: Option<&'a [u8]>, colors: Colors) -> Merge<'a> {
        Merge { header, width: header.width, height: header.height, top: 0, rows: header.height, channels: header.mode.colors(), color: color_of(header.mode), blocks, icc, colors, warnings: Vec::new() }
    }

    /// The same merge over the band of `rows` rows from row `top`, with
    /// nothing left out yet.
    fn band(&self, top: usize, rows: usize) -> Merge<'a> {
        Merge { top, rows, warnings: Vec::new(), ..*self }
    }

    /// The pixels of the band.
    fn pixels(&self) -> usize {
        self.width * self.rows
    }

    /// `rect` cut to the band's rows.
    fn in_band(&self, rect: Rect) -> Rect {
        Rect { top: rect.top.max(self.top as i32), bottom: rect.bottom.min((self.top + self.rows) as i32), ..rect }
    }

    /// The pixels of `rect` within the canvas and the band: left, right,
    /// top and bottom; `None` where there are none.
    fn span(&self, rect: Rect) -> Option<[i64; 4]> {
        let x0 = i64::from(rect.left).max(0);
        let x1 = (i64::from(rect.left) + rect.width() as i64).min(self.width as i64);
        let y0 = i64::from(rect.top).max(self.top as i64);
        let y1 = (i64::from(rect.top) + rect.height() as i64).min((self.top + self.rows) as i64);
        (x0 < x1 && y0 < y1).then_some([x0, x1, y0, y1])
    }

    /// The band's index of `x`, `y`, a pixel of the canvas in the band.
    fn at_band(&self, x: i64, y: i64) -> usize {
        (y as usize - self.top) * self.width + x as usize
    }

    /// `x` kept within what the document's samples hold: 0 to 1, and from 0
    /// up for a 32-bit document's light.
    fn clip(&self, x: f32) -> f32 {
        if self.header.depth == 32 { x.max(0.0) } else { x.clamp(0.0, 1.0) }
    }

    fn warn(&mut self, record: &Record, what: &str) {
        self.warnings.push(format!("the layer {} {what}", quoted(&record.name)));
    }

    /// A channel's values from 0 to 1 over `rect` placed in the band,
    /// `outside` everywhere else; or why the channel cannot be read.
    fn plane(&self, channel: &Channel, rect: Rect, outside: f32) -> Result<Vec<f32>, String> {
        let mut out = vec![outside; self.pixels()];
        let (w, h) = (rect.width(), rect.height());
        if w == 0 || h == 0 {
            return Ok(out);
        }
        let bytes = decompressed_of(self.header, channel, rect)?;
        readable(self.header.depth)?;
        if bytes.len() < w * h * (self.header.depth / 8) {
            return Ok(out);
        }
        if let Some([x0, x1, y0, y1]) = self.span(rect) {
            for y in y0..y1 {
                let from = (y - i64::from(rect.top)) as usize * w + (x0 - i64::from(rect.left)) as usize;
                let to = self.at_band(x0, y);
                for (k, value) in out[to..to + (x1 - x0) as usize].iter_mut().enumerate() {
                    *value = sample(&bytes, from + k, self.header.depth);
                }
            }
        }
        Ok(out)
    }

    /// `values` over `rect`, row by row, placed in the band, `outside`
    /// everywhere else.
    fn pasted(&self, values: &[f32], rect: Rect, outside: f32) -> Vec<f32> {
        let mut out = vec![outside; self.pixels()];
        let w = rect.width();
        if values.len() < w * rect.height() {
            return out;
        }
        if let Some([x0, x1, y0, y1]) = self.span(rect) {
            for y in y0..y1 {
                let from = (y - i64::from(rect.top)) as usize * w + (x0 - i64::from(rect.left)) as usize;
                let to = self.at_band(x0, y);
                let n = (x1 - x0) as usize;
                out[to..to + n].copy_from_slice(&values[from..from + n]);
            }
        }
        out
    }

    /// `value` over `rect` placed in the band, `outside` everywhere else.
    fn filled_plane(&self, rect: Rect, value: f32, outside: f32) -> Vec<f32> {
        let mut out = vec![outside; self.pixels()];
        if let Some([x0, x1, y0, y1]) = self.span(rect) {
            for y in y0..y1 {
                let to = self.at_band(x0, y);
                out[to..to + (x1 - x0) as usize].fill(value);
            }
        }
        out
    }

    /// Whether a shape or fill layer's mask repeats its transparency pixel
    /// for pixel, as `prepare` worked it out, or else now.
    fn mask_repeats_shape(&self, record: &Record) -> Result<bool, String> {
        match &record.repeats_shape {
            Some(repeats) => repeats.clone(),
            None => repeats_shape(self.header, record),
        }
    }

    /// The layer's mask coverage with its density in, as psd-tools'
    /// _get_mask makes it, and, where `vector`, its vector mask with the
    /// vector density in, where psd-tools draws it: for a layer that stores
    /// no pixels, or one with no fill whose only mask is a user mask.
    fn mask(&mut self, record: &Record, vector: bool) -> Result<Cover, String> {
        let mut shape = Cover::All(1.0);
        if let Some(mask) = &record.mask
            && !mask.disabled
            && !self.mask_repeats_shape(record)?
        {
            let (id, rect, outside) = match mask.real {
                Some((rect, outside)) => (-3, rect, outside),
                None => (-2, mask.rect, mask.default),
            };
            if let Some(channel) = record.channel(id) {
                shape = Cover::Each(self.plane(channel, rect, f32::from(outside) / 255.0)?);
            }
            if let Some(density) = mask.density.or(mask.vector_density) {
                shape = with_density(shape, density);
            }
            if mask.feathered {
                self.warn(record, "has a feathered mask, whose feather the merge does not apply");
            }
        }
        if vector
            && let Some(data) = record.vector_mask()
            && (!record.has_pixels() || (!record.has_fill() && record.mask.is_some() && !record.has_real()))
        {
            let mut coverage = psd_vector::read(data)?.coverage_rows(self.width, self.height, self.top, self.top + self.rows);
            for (i, v) in coverage.iter_mut().enumerate() {
                *v *= shape.at(i);
            }
            shape = Cover::Each(coverage);
            if let Some(density) = record.mask.as_ref().and_then(|m| m.vector_density) {
                shape = with_density(shape, density);
            }
        }
        Ok(shape)
    }

    /// A layer's own color, shape and alpha over the canvas, as psd-tools'
    /// _get_object reads a pixel layer: its color channels, white outside
    /// it; its transparency, or where it has none, opaque over its
    /// rectangle.
    fn object(&self, record: &Record) -> Result<Painted, String> {
        let mut color = Vec::with_capacity(self.channels);
        let mut any = false;
        for c in 0..self.channels {
            let plane = match record.channel(c as i16) {
                Some(channel) => {
                    any = true;
                    self.plane(channel, record.rect, 1.0)?
                }
                None => vec![1.0; self.pixels()],
            };
            color.push(plane);
        }
        let shape = match record.channel(-1) {
            Some(channel) => self.plane(channel, record.rect, 0.0)?,
            None if any => self.filled_plane(record.rect, 1.0, 0.0),
            None => vec![0.0; self.pixels()],
        };
        let alpha = shape.clone();
        Ok(Painted { color, shape, alpha })
    }

    /// The box a layer that stores no pixels is filled over, as psd-tools'
    /// bbox gives it: a shape layer's from its origination items where they
    /// give one, else from its vector mask's span, else nothing; any other
    /// fill layer's from its record, a right or bottom edge of 0 the
    /// canvas's. A shape layer whose vector mask is inverted is filled over
    /// the canvas, which the mask then covers outside its paths.
    fn layer_box(&self, record: &Record) -> Result<Rect, String> {
        if record.is_shape() {
            if let Some(data) = record.vector_mask()
                && psd_vector::read(data)?.inverted
            {
                return Ok(Rect { top: 0, left: 0, bottom: self.height as i32, right: self.width as i32 });
            }
            if let Some(data) = record.block(b"vogk")
                && let Some([left, top, right, bottom]) = psd_fill::origination_box(data)?
            {
                return Ok(Rect { top, left, bottom, right });
            }
            if let Some(data) = record.block(b"vmsk").or_else(|| record.block(b"vsms")) {
                let [left, top, right, bottom] = psd_fill::vector_mask_box(&psd_vector::read(data)?, self.width, self.height);
                return Ok(Rect { top, left, bottom, right });
            }
            return Ok(Rect { top: 0, left: 0, bottom: 0, right: 0 });
        }
        let edge = |e: i32, whole: usize| if e != 0 { e } else { whole as i32 };
        Ok(Rect { right: edge(record.rect.right, self.width), bottom: edge(record.rect.bottom, self.height), ..record.rect })
    }

    /// Where a layer that stores no pixels is painted: `rect`, its box, and
    /// with a vector mask every pixel the mask may cover past the box too,
    /// as Photoshop samples every edge pixel a mask covers.
    fn painted_box(&self, record: &Record, rect: Rect) -> Result<Rect, String> {
        let Some(data) = record.vector_mask() else { return Ok(rect) };
        let [left, top, right, bottom] = psd_vector::read(data)?.reach(self.width, self.height);
        Ok(Rect { top: rect.top.min(top as i32), left: rect.left.min(left as i32), bottom: rect.bottom.max(bottom as i32), right: rect.right.max(right as i32) })
    }

    /// A layer that stores no pixels drawn from its fill, as psd-tools'
    /// create_fill and _get_object draw it, laid out over the layer's box:
    /// the fill over every pixel `painted_box` gives and white outside it,
    /// those pixels covered, or as a pattern's transparency says, the
    /// pattern tiled from the document's corner, as Photoshop tiles it;
    /// nothing covered where the
    /// shape's fill is switched off. Or, for a fill the merge does not
    /// draw, why, as the end of a warning naming the layer.
    fn fill(&self, record: &Record) -> Result<Result<Painted, String>, String> {
        let pixels = self.pixels();
        if record.stroke()?.and_then(|s| s.flag("fillEnabled")) == Some(false) {
            return Ok(Ok(Painted { color: vec![vec![1.0; pixels]; self.channels], shape: vec![0.0; pixels], alpha: vec![0.0; pixels] }));
        }
        let Some((kind, settings)) = record.fill_settings()? else {
            return Ok(Err("has a shape fill the merge does not read".to_string()));
        };
        let rect = self.layer_box(record)?;
        let reach = self.painted_box(record, rect)?;
        let (color, own_shape) = match kind {
            FillKind::Gradient => match self.gradient(record, &settings, reach)? {
                Ok(painted) => painted,
                Err(why) => return Ok(Err(why)),
            },
            FillKind::Solid => {
                let values = match psd_fill::solid(&settings, self.header.mode)? {
                    Solid::Color(values) => values,
                    Solid::Foreign(color) => match (self.colors.convert)(color, self.header, self.icc) {
                        Ok(values) => values,
                        Err(why) => return Ok(Err(format!("is filled with a color of another mode than the {} document's, which does not convert into it: {why}", self.header.mode.name()))),
                    },
                    Solid::Undrawn(why) => return Ok(Err(why)),
                };
                (values.iter().map(|&v| self.filled_plane(reach, v, 1.0)).collect(), None)
            }
            FillKind::Pattern => {
                let fill = psd_fill::pattern_of(&settings)?;
                if let Some(placement) = fill.placement() {
                    return Ok(Err(format!("is filled with its pattern at {placement}, which the merge does not draw")));
                }
                let Some(pattern) = psd_fill::pattern(self.blocks, &fill.id)? else {
                    return Ok(Err("is filled with a pattern the document does not hold".to_string()));
                };
                if !pattern.fits(self.header.mode) {
                    return Ok(Err(format!("is filled with a pattern of another color mode than the {} document's, which the merge does not convert", self.header.mode.name())));
                }
                let band = self.in_band(reach);
                let rows = i64::from(band.top)..i64::from(band.bottom);
                let cols = i64::from(reach.left)..i64::from(reach.right);
                let color = (0..self.channels).map(|c| self.pasted(&pattern.tiled(c, cols.clone(), rows.clone()), band, 1.0)).collect();
                let transparency = (pattern.planes.len() > pattern.colors).then(|| self.pasted(&pattern.tiled(pattern.planes.len() - 1, cols.clone(), rows.clone()), band, 0.0));
                (color, transparency)
            }
        };
        let shape = match own_shape {
            Some(shape) => shape,
            None => self.filled_plane(reach, 1.0, 0.0),
        };
        let alpha = shape.clone();
        Ok(Ok(Painted { color, shape, alpha }))
    }

    /// A shape layer's fill with its vector stroke drawn over it sample by
    /// sample, as Photoshop samples both: a sample the stroke covers takes
    /// the stroke's color and any other the fill covers the fill's, so a
    /// pixel's color is their mean over its covered samples and its shape
    /// and alpha their count over 255, the fill's own transparency in. The
    /// stroke is drawn whole whatever its opacity item says, as Photoshop
    /// 2026 draws it. The layer's vector mask is drawn here, so its mask
    /// leaves it out. Or, for a stroke the merge does not draw, why, as the
    /// end of a warning naming the layer.
    fn stroked(&self, record: &Record, settings: &Descriptor, fill: Option<&Painted>) -> Result<Result<Painted, String>, String> {
        let stroke = match psd_stroke::Stroke::from_descriptor(settings) {
            Ok(stroke) => stroke,
            Err(why) => return Ok(Err(why)),
        };
        if !matches!(settings.get("strokeStyleBlendMode"), Some(Value::Enum(_, mode)) if mode == "normal" || mode == "Nrml") {
            return Ok(Err("it is blended other than normally".to_string()));
        }
        let Some(content) = psd_fill::nested(settings, "strokeStyleContent") else {
            return Ok(Err("it gives no content".to_string()));
        };
        if content.class != "solidColorLayer" {
            return Ok(Err(format!("its content is a {}, and the merge draws only solid colors", content.class)));
        }
        let color = match psd_fill::solid(content, self.header.mode)? {
            Solid::Color(values) => values,
            Solid::Foreign(color) => match (self.colors.convert)(color, self.header, self.icc) {
                Ok(values) => values,
                Err(why) => return Ok(Err(format!("its color is of another mode than the {} document's and does not convert into it: {why}", self.header.mode.name()))),
            },
            Solid::Undrawn(why) => return Ok(Err(why)),
        };
        if let Some(density) = record.mask.as_ref().and_then(|m| m.vector_density)
            && density != 255
        {
            return Ok(Err(format!("its vector mask's density is {density} of 255")));
        }
        let Some(data) = record.vector_mask() else { return Ok(Err("it has no path".to_string())) };
        let paths = psd_vector::read(data)?;
        if paths.inverted {
            return Ok(Err("its vector mask is inverted".to_string()));
        }
        let drawn = paths.drawn(self.width, self.height);
        let (first, count) = (self.top as i64 * psd_raster::DOWN, self.rows * psd_raster::DOWN as usize);
        let strokes = match psd_stroke::sample_stroke(&drawn, &stroke, first, count) {
            Ok(runs) => runs,
            Err(why) => return Ok(Err(why)),
        };
        let fills = paths.sampled(self.width, self.height, first, count, [0, self.width as i64 * psd_raster::ACROSS]);
        let fill_alone: Vec<Vec<psd_raster::Run>> = fills.iter().zip(&strokes).map(|(f, s)| psd_raster::combine_runs(f, s, psd_raster::Combine::Subtract)).collect();
        let on_stroke = psd_raster::pixel_counts(&strokes, self.rows, 0, self.width);
        let on_fill = psd_raster::pixel_counts(&fill_alone, self.rows, 0, self.width);
        let pixels = self.pixels();
        let mut painted = Painted { color: vec![vec![1.0; pixels]; self.channels], shape: vec![0.0; pixels], alpha: vec![0.0; pixels] };
        for i in 0..pixels {
            let by_stroke = f32::from(on_stroke[i]);
            let by_fill = fill.map_or(0.0, |f| f32::from(on_fill[i]) * f.shape[i]);
            let covered = by_stroke + by_fill;
            for (c, plane) in painted.color.iter_mut().enumerate() {
                let under = fill.map_or(1.0, |f| f.color[c][i]);
                plane[i] = if covered > 0.0 { (by_stroke * color[c] + by_fill * under) / covered } else { under };
            }
            painted.shape[i] = covered / 255.0;
            painted.alpha[i] = covered / 255.0;
        }
        Ok(Ok(painted))
    }

    /// A gradient fill drawn over `rect`, where the layer is painted: its
    /// colors, white outside it, and its opacity, the layer's own shape,
    /// laid out over the box `gradient_box` gives; a stop of another mode converts as
    /// a solid fill's color does, and the linear method mixes in the light
    /// the module's color handling gives. Or, for a gradient the merge does
    /// not draw, why.
    fn gradient(&self, record: &Record, settings: &Descriptor, rect: Rect) -> Result<Result<Filled, String>, String> {
        let gradient = match psd_gradient::gradient_of(settings, self.header.mode)? {
            Setting::Drawn(gradient) => gradient,
            Setting::Undrawn(why) => return Ok(Err(why)),
        };
        let mut values = Vec::with_capacity(gradient.colors.len());
        for stop in &gradient.colors {
            let color = match &stop.value {
                Solid::Color(color) => color.clone(),
                Solid::Foreign(color) => match (self.colors.convert)(*color, self.header, self.icc) {
                    Ok(color) => color,
                    Err(why) => return Ok(Err(format!("is filled with a gradient of colors of another mode than the {} document's, which do not convert into it: {why}", self.header.mode.name()))),
                },
                Solid::Undrawn(why) => return Ok(Err(why.clone())),
            };
            values.push(color.iter().map(|&v| f64::from(v)).collect::<Vec<f64>>());
        }
        let light = match gradient.method {
            psd_gradient::Method::Linear(_) => match (self.colors.light)(self.header, self.icc) {
                Ok(light) => Some(light),
                Err(why) => return Ok(Err(format!("is filled with a gradient of the linear method, which mixes in light: {why}"))),
            },
            psd_gradient::Method::Classic => None,
        };
        let bounds = self.gradient_box(record, &gradient, rect)?;
        let band = self.in_band(rect);
        let Some(places) = gradient.places(bounds, [i64::from(band.left), i64::from(band.top), band.width() as i64, band.height() as i64]) else {
            return Ok(Err("is filled with a gradient whose start and end are the same pixel, which the merge does not draw".to_string()));
        };
        let color = gradient.colors_at(&places, &values, light.as_ref()).iter().map(|plane| self.pasted(plane, band, 1.0)).collect();
        let shape = self.pasted(&gradient.opacity_at(&places), band, 0.0);
        Ok(Ok((color, Some(shape))))
    }

    /// The box a gradient fill is laid out over, its left, top, right and
    /// bottom: the canvas where it is not aligned with its layer, the box
    /// of the pixels a shape's mask covers enough to show at 8 bits, which
    /// is the canvas for a mask that covers past its paths, as an inverted
    /// one does, as `prepare` worked it out or else now, and `rect`, where
    /// the layer is painted, for any other layer.
    fn gradient_box(&self, record: &Record, gradient: &psd_gradient::Gradient, rect: Rect) -> Result<[f64; 4], String> {
        let own = [rect.left, rect.top, rect.right, rect.bottom].map(f64::from);
        if !gradient.aligned {
            return Ok([0.0, 0.0, self.width as f64, self.height as f64]);
        }
        if record.is_shape()
            && let Some(data) = record.vector_mask()
        {
            let visible = match &record.visible_box {
                Some(visible) => visible.clone()?,
                None => psd_vector::read(data)?.visible_box(self.width, self.height),
            };
            return Ok(visible.map_or(own, |b| b.map(|e| e as f64)));
        }
        Ok(own)
    }
}

/// `name` in quotation marks, or "with no name".
fn quoted(name: &str) -> String {
    if name.is_empty() { "with no name".to_string() } else { format!("'{name}'") }
}

/// The union of two coverages, as psd-tools' union gives it.
fn union(a: f32, b: f32) -> f32 {
    a + b - a * b
}

/// `a / b`, or `fill` where `b` is not above 0, as psd-tools' divide gives
/// it.
fn divide(a: f32, b: f32, fill: f32) -> f32 {
    if b > 0.0 { a / b } else { fill }
}

/// A coverage with a mask's density in, as psd-tools' _get_mask takes it:
/// d c + 1 - d, d the density over 255. Over a plane, d and 1 - d are each
/// the float32 of the double, as numpy takes a Python float to a float32
/// array.
fn with_density(shape: Cover, density: u8) -> Cover {
    let d = f64::from(density) / 255.0;
    match shape {
        Cover::Each(mut v) => {
            let (share, rest) = (d as f32, (1.0 - d) as f32);
            v.iter_mut().for_each(|c| *c = share * *c + rest);
            Cover::Each(v)
        }
        Cover::All(c) => Cover::All((d * f64::from(c) + (1.0 - d)) as f32),
    }
}

/// What a canvas holds as psd-tools' Compositor holds it: the backdrop it
/// started from, the color and alpha so far, and the union of the shapes
/// and alphas put on it.
struct Compositor {
    color_0: Vec<Vec<f32>>,
    alpha_0: Vec<f32>,
    color: Vec<Vec<f32>>,
    alpha: Vec<f32>,
    shape_g: Vec<f32>,
    alpha_g: Vec<f32>,
    adjustment_isolated: bool,
}

/// A backdrop for deep knockout: color planes and alpha.
struct Backdrop {
    color: Vec<Vec<f32>>,
    alpha: Vec<f32>,
}

impl Compositor {
    fn new(color: Vec<Vec<f32>>, alpha: Vec<f32>, adjustment_isolated: bool) -> Compositor {
        let pixels = alpha.len();
        Compositor { color_0: color.clone(), alpha_0: alpha.clone(), color, alpha, shape_g: vec![0.0; pixels], alpha_g: vec![0.0; pixels], adjustment_isolated }
    }

    /// The color with the backdrop it started from taken back out, for a
    /// result that goes on over that backdrop again, as psd-tools'
    /// result_isolated gives it.
    fn result_isolated(&self, m: &Merge) -> Vec<Vec<f32>> {
        self.color
            .iter()
            .zip(&self.color_0)
            .map(|(color, color_0)| (0..color.len()).map(|i| m.clip(color[i] + (color[i] - color_0[i]) * (divide(self.alpha_0[i], self.alpha_g[i], 1.0) - self.alpha_0[i]))).collect())
            .collect()
    }

    /// Puts `node` on the canvas, as psd-tools' Compositor.apply does; a
    /// clipping layer goes on only through the layer it clips to, when
    /// `clip_compositing`. `deep` is what deep knockout reaches.
    fn apply(&mut self, m: &mut Merge, node: &Node, clip_compositing: bool, deep: Option<&Backdrop>) -> Result<(), String> {
        let record = node.record;
        if record.hidden || (!clip_compositing && node.clips_below()) {
            return Ok(());
        }
        let Some((own, stroked)) = Compositor::own(m, node)? else { return Ok(()) };
        let blend = record.blend_mode();
        let Source { color, mut shape, mut alpha, mut mask, fill, knockout, adjustment_isolated } = self.resolve(m, node, deep, own, stroked)?;
        let scale = |plane: &mut [f32]| plane.iter_mut().for_each(|v| *v *= fill);
        if &blend == b"pass" && adjustment_isolated == Some(false) {
            scale(&mut shape);
            scale(&mut alpha);
            match &mut mask {
                Cover::Each(v) => scale(v),
                Cover::All(x) => *x *= fill,
            }
            self.passthrough(m, &color, &shape, &alpha, &mask);
        } else {
            if knockout == 0 {
                scale(&mut shape);
            }
            scale(&mut alpha);
            self.put(m, &color, &shape, &alpha, &blend, knockout, deep);
        }
        Ok(())
    }

    /// A layer's own content as the merge reads it, with the warnings of
    /// what it leaves out: `None` for an adjustment or a fill the merge does
    /// not draw; else what a layer that stores no pixels is painted with,
    /// and whether its vector mask is drawn into that with its stroke.
    fn own(m: &mut Merge, node: &Node) -> Result<Option<(Option<Painted>, bool)>, String> {
        let record = node.record;
        if let Some(what) = record.adjustment() {
            m.warn(record, &format!("is a {what} adjustment, which the merge does not apply"));
            return Ok(None);
        }
        let mut own = None;
        let mut stroked = false;
        if node.children.is_none() && !record.has_pixels() {
            if record.has_fill() {
                match m.fill(record)? {
                    Ok(painted) => own = Some(painted),
                    Err(why) => {
                        m.warn(record, &why);
                        return Ok(None);
                    }
                }
            }
            if record.has_vector_mask()
                && let Some(settings) = record.stroke()?
                && settings.flag("strokeEnabled") == Some(true)
            {
                match m.stroked(record, &settings, own.as_ref())? {
                    Ok(painted) => {
                        own = Some(painted);
                        stroked = true;
                    }
                    Err(why) => m.warn(record, &format!("has a vector stroke the merge does not draw: {why}")),
                }
            }
        }
        let effects = record.effects()?;
        if !effects.is_empty() {
            m.warn(record, &format!("has effects the merge does not draw: {}", effects.join(", ")));
        }
        if record.blends_if() {
            m.warn(record, "has Blend If ranges, which the merge does not apply");
        }
        if &record.blend_mode() == b"diss" {
            m.warn(record, "is in dissolve, which the merge draws as normal");
        }
        Ok(Some((own, stroked)))
    }

    /// `node` resolved into a source, as psd-tools' _resolve_source does;
    /// `own` is what a layer that stores no pixels is filled with, its
    /// vector mask already drawn in where `stroked`.
    fn resolve(&mut self, m: &mut Merge, node: &Node, deep: Option<&Backdrop>, own: Option<Painted>, stroked: bool) -> Result<Source, String> {
        let record = node.record;
        let knockout = record.knockout();
        let (painted, adjustment_isolated) = match (&node.children, own) {
            (Some(children), _) => {
                let (painted, isolated) = self.group(m, node, children, knockout, deep)?;
                (painted, Some(isolated))
            }
            (None, Some(painted)) => (painted, None),
            (None, None) => (m.object(record)?, None),
        };
        let Painted { mut color, mut shape, mut alpha } = painted;
        if !node.clips.is_empty() {
            let mut sub = Compositor::new(color, alpha.clone(), false);
            for clip in &node.clips {
                sub.apply(m, clip, true, None)?;
            }
            color = sub.color;
        }
        let mut mask = m.mask(record, !stroked)?;
        let opacity = f32::from(record.opacity) / 255.0;
        match &mut mask {
            Cover::Each(v) => {
                for ((s, a), k) in shape.iter_mut().zip(alpha.iter_mut()).zip(v.iter_mut()) {
                    *s *= *k;
                    *k *= opacity;
                    *a *= *k;
                }
            }
            Cover::All(k) => {
                shape.iter_mut().for_each(|s| *s *= *k);
                *k *= opacity;
                alpha.iter_mut().for_each(|a| *a *= *k);
            }
        }
        Ok(Source { color, shape, alpha, mask, fill: record.fill_opacity(), knockout, adjustment_isolated })
    }

    /// A group merged on a canvas of its own, as psd-tools' _get_group
    /// merges it: seeded with the canvas below, transparent unless it
    /// passes through; its color, shape and alpha, and whether it isolates
    /// its adjustments, as one with a fill opacity below 1 or layers
    /// clipped to it does.
    fn group(&self, m: &mut Merge, node: &Node, children: &[Node], knockout: u8, deep: Option<&Backdrop>) -> Result<(Painted, bool), String> {
        let pass = node.record.blend_mode() == *b"pass";
        let (color_b, alpha_b) = if knockout != 0 { Compositor::knockout_backdrop(knockout, deep, &self.color_0, &self.alpha_0) } else { (&self.color[..], &self.alpha[..]) };
        let isolate = node.record.fill_opacity() < 1.0 || !node.clips.is_empty();
        let alpha_start = if pass { alpha_b.to_vec() } else { vec![0.0; m.pixels()] };
        let mut sub = Compositor::new(color_b.to_vec(), alpha_start, self.adjustment_isolated || isolate);
        let inner = if pass { deep } else { None };
        for child in children {
            sub.apply(m, child, false, inner)?;
        }
        let color = if isolate { sub.result_isolated(m) } else { sub.color.clone() };
        Ok((Painted { color, shape: sub.shape_g, alpha: sub.alpha_g }, isolate))
    }

    /// What knockout reaches: shallow, the backdrop `color_0` and `alpha_0`
    /// a canvas started from; deep, the Background layer where the document
    /// has one.
    fn knockout_backdrop<'s>(knockout: u8, deep: Option<&'s Backdrop>, color_0: &'s [Vec<f32>], alpha_0: &'s [f32]) -> (&'s [Vec<f32>], &'s [f32]) {
        match (knockout, deep) {
            (2, Some(backdrop)) => (&backdrop.color, &backdrop.alpha),
            _ => (color_0, alpha_0),
        }
    }

    /// A source put on by its blend mode, as psd-tools' _apply_source puts
    /// it.
    #[allow(clippy::too_many_arguments)]
    fn put(&mut self, m: &Merge, color: &[Vec<f32>], shape: &[f32], alpha: &[f32], blend: &[u8; 4], knockout: u8, deep: Option<&Backdrop>) {
        let channels = m.channels;
        let (k_color, k_alpha) = Compositor::knockout_backdrop(knockout, deep, &self.color_0, &self.alpha_0);
        let mut cb = vec![0.0f32; channels];
        let mut cs = vec![0.0f32; channels];
        let mut blended = vec![0.0f32; channels];
        for i in 0..shape.len() {
            let (s, a) = (shape[i], alpha[i]);
            self.shape_g[i] = union(self.shape_g[i], s);
            self.alpha_g[i] = if knockout != 0 { (1.0 - s) * self.alpha_g[i] + (s - a) * k_alpha[i] + a } else { union(self.alpha_g[i], a) };
            let alpha_previous = self.alpha[i];
            self.alpha[i] = union(self.alpha_0[i], self.alpha_g[i]);
            let alpha_b = if knockout != 0 { k_alpha[i] } else { alpha_previous };
            for c in 0..channels {
                cb[c] = if knockout != 0 { k_color[c][i] } else { self.color[c][i] };
                cs[c] = color[c][i];
            }
            blend_pixel(blend, m.color, &cb, &cs, &mut blended);
            for c in 0..channels {
                let t = (s - a) * alpha_b * cb[c] + a * ((1.0 - alpha_b) * cs[c] + alpha_b * blended[c]);
                self.color[c][i] = m.clip(divide((1.0 - s) * alpha_previous * self.color[c][i] + t, self.alpha[i], 1.0));
            }
        }
    }

    /// A pass-through group's result put back, as psd-tools'
    /// _apply_passthrough_source puts it: the canvas below and the group's
    /// result over it, mixed by the group's mask and opacity, in
    /// premultiplied terms.
    fn passthrough(&mut self, m: &Merge, color: &[Vec<f32>], shape: &[f32], alpha: &[f32], mask: &Cover) {
        for i in 0..shape.len() {
            let (alpha_b, k) = (self.alpha[i], mask.at(i));
            self.shape_g[i] = union(self.shape_g[i], shape[i]);
            self.alpha_g[i] = union(self.alpha_g[i], alpha[i]);
            self.alpha[i] = union(self.alpha_0[i], self.alpha_g[i]);
            for (c, source) in color.iter().enumerate().take(m.channels) {
                let t = (1.0 - k) * alpha_b * self.color[c][i] + (k * alpha_b + alpha[i] * (1.0 - alpha_b)) * source[i];
                self.color[c][i] = m.clip(divide(t, self.alpha[i], source[i]));
            }
        }
    }
}

/// psd-tools' guard against division by 0 in the blend modes.
const EPSILON: f32 = 1e-9;

/// The blend mode `mode` of the backdrop pixel `cb` and the source pixel
/// `cs`, into `out`, as psd-tools' blend.py gives each mode; a mode it does
/// not know is normal.
fn blend_pixel(mode: &[u8; 4], color: Color, cb: &[f32], cs: &[f32], out: &mut [f32]) {
    let separable = |f: fn(f32, f32) -> f32, out: &mut [f32]| {
        for ((o, &b), &s) in out.iter_mut().zip(cb).zip(cs) {
            *o = f(b, s);
        }
    };
    match mode {
        b"dark" => separable(f32::min, out),
        b"mul " => separable(|b, s| b * s, out),
        b"idiv" => separable(color_burn, out),
        b"lbrn" => separable(|b, s| (b + s - 1.0).max(0.0), out),
        b"lite" => separable(f32::max, out),
        b"scrn" => separable(screen, out),
        b"div " => separable(color_dodge, out),
        b"lddg" => separable(|b, s| (b + s).min(1.0), out),
        b"over" => separable(|b, s| hard_light(s, b), out),
        b"sLit" => separable(soft_light, out),
        b"hLit" => separable(hard_light, out),
        b"vLit" => separable(vivid_light, out),
        b"lLit" => separable(|b, s| if s > 0.5 { (b + 2.0 * s - 1.0).min(1.0) } else { (b + 2.0 * s - 1.0).max(0.0) }, out),
        b"pLit" => separable(|b, s| if s > 0.5 { b.max(2.0 * s - 1.0) } else { b.min(2.0 * s) }, out),
        b"hMix" => separable(|b, s| if b + s > 1.0 || (b + s == 1.0 && b > 0.5) { 1.0 } else { 0.0 }, out),
        b"diff" => separable(|b, s| (b - s).abs(), out),
        b"smud" => separable(|b, s| b + s - 2.0 * b * s, out),
        b"fsub" => separable(|b, s| (b - s).max(0.0), out),
        b"fdiv" => separable(|b, s| (b / (s + EPSILON)).min(1.0), out),
        b"hue " | b"sat " | b"colr" | b"lum " | b"dkCl" | b"lgCl" => non_separable(mode, color, cb, cs, out),
        _ => out.copy_from_slice(cs),
    }
}

fn screen(b: f32, s: f32) -> f32 {
    b + s - b * s
}

fn hard_light(b: f32, s: f32) -> f32 {
    if s > 0.5 { screen(b, 2.0 * s - 1.0) } else { b * (2.0 * s) }
}

fn color_dodge(b: f32, s: f32) -> f32 {
    if b == 0.0 {
        0.0
    } else if s == 1.0 {
        1.0
    } else {
        (b / (1.0 - s + EPSILON)).min(1.0)
    }
}

fn color_burn(b: f32, s: f32) -> f32 {
    if b == 1.0 {
        1.0
    } else if s == 0.0 {
        0.0
    } else {
        1.0 - ((1.0 - b) / (s + EPSILON)).min(1.0)
    }
}

fn soft_light(b: f32, s: f32) -> f32 {
    let d = if b <= 0.25 { ((16.0 * b - 12.0) * b + 4.0) * b } else { b.sqrt() };
    if s <= 0.5 { b - (1.0 - 2.0 * s) * b * (1.0 - b) } else { b + (2.0 * s - 1.0) * (d - b) }
}

fn vivid_light(b: f32, s: f32) -> f32 {
    if s == 0.0 {
        0.0
    } else if s == 1.0 {
        1.0
    } else if s <= 0.5 {
        1.0 - ((1.0 - b) / (2.0 * s + EPSILON)).min(1.0)
    } else {
        (b / (2.0 * (1.0 - s) + EPSILON)).min(1.0)
    }
}

/// The six non-separable modes, as psd-tools' blend.py gives them: on RGB
/// and Lab's three channels; on CMYK's first three, K taken from the
/// backdrop for hue, saturation and color and from the source for
/// luminosity, the selections between whole pixels comparing lightness with
/// K in; normal on one channel.
fn non_separable(mode: &[u8; 4], color: Color, cb: &[f32], cs: &[f32], out: &mut [f32]) {
    if cb.len() < 3 || color == Color::Gray {
        out.copy_from_slice(cs);
        return;
    }
    let b = [cb[0], cb[1], cb[2]];
    let s = [cs[0], cs[1], cs[2]];
    let lightness = |c: &[f32]| if c.len() == 4 { lum([c[0] * c[3], c[1] * c[3], c[2] * c[3]]) } else { lum([c[0], c[1], c[2]]) };
    match mode {
        b"dkCl" => out.copy_from_slice(if lightness(cs) < lightness(cb) { cs } else { cb }),
        b"lgCl" => out.copy_from_slice(if lightness(cs) > lightness(cb) { cs } else { cb }),
        _ => {
            let rgb = match mode {
                b"hue " => set_lum(with_saturation(s, saturation(b)), lum(b)),
                b"sat " => set_lum(with_saturation(b, saturation(s)), lum(b)),
                b"colr" => set_lum(s, lum(b)),
                _ => set_lum(b, lum(s)),
            };
            out[..3].copy_from_slice(&rgb);
            if out.len() == 4 {
                out[3] = if mode == b"lum " { cs[3] } else { cb[3] };
            }
        }
    }
}

fn lum(c: [f32; 3]) -> f32 {
    0.3 * c[0] + 0.59 * c[1] + 0.11 * c[2]
}

fn set_lum(c: [f32; 3], l: f32) -> [f32; 3] {
    let d = l - lum(c);
    clip_color([c[0] + d, c[1] + d, c[2] + d])
}

/// psd-tools' _clip_color: each step's minimum and maximum taken from the
/// color as it came in.
fn clip_color(mut c: [f32; 3]) -> [f32; 3] {
    let l = lum(c);
    let min = c[0].min(c[1]).min(c[2]);
    let max = c[0].max(c[1]).max(c[2]);
    if min < 0.0 {
        for v in &mut c {
            *v = l + (*v - l) * l / (l - min + EPSILON);
        }
    }
    if max > 1.0 {
        for v in &mut c {
            *v = l + (*v - l) * (1.0 - l) / (max - l + EPSILON);
        }
    }
    c.map(|v| v.clamp(0.0, 1.0))
}

fn saturation(c: [f32; 3]) -> f32 {
    c[0].max(c[1]).max(c[2]) - c[0].min(c[1]).min(c[2])
}

/// `c` given the saturation `s`, as psd-tools' blend.py gives it, ties
/// between channels included.
fn with_saturation(c: [f32; 3], s: f32) -> [f32; 3] {
    let max = c[0].max(c[1]).max(c[2]);
    let min = c[0].min(c[1]).min(c[2]);
    let mut sorted = c;
    sorted.sort_by(f32::total_cmp);
    let mid = sorted[1];
    let mut out = [0.0f32; 3];
    let diff = max > min;
    for i in 0..3 {
        let is_mid = c[i] == mid;
        let is_max = c[i] == max && !is_mid;
        if is_mid && diff {
            out[i] = (mid - min) * s / (max - min + EPSILON);
        } else if is_max && diff {
            out[i] = s;
        }
        if c[i] == min {
            out[i] = 0.0;
        }
    }
    out
}

/// The color the merge blends a document's channels as.
fn color_of(mode: Mode) -> Color {
    match mode {
        Mode::Cmyk => Color::Cmyk,
        Mode::Lab => Color::Lab,
        Mode::Rgb => Color::Rgb,
        Mode::Bitmap | Mode::Gray | Mode::Indexed | Mode::Duotone => Color::Gray,
    }
}

/// A band's rows of the merged canvas: its first row, how many, and its
/// share of each color plane and of the alpha, which its merge fills.
struct Slot<'p> {
    top: usize,
    rows: usize,
    color: Vec<&'p mut [f32]>,
    alpha: &'p mut [f32],
}

/// One band of the merge: `slot`'s rows through every layer of `nodes` as
/// the whole canvas goes, its color and alpha written into `slot`; what it
/// left out, or why it could not merge.
fn merge_band(whole: &Merge, records: &[Record], nodes: &[Node], slot: Slot) -> Result<Vec<String>, String> {
    let mut m = whole.band(slot.top, slot.rows);
    let pixels = m.pixels();
    let deep = match records.first() {
        Some(bottom) if bottom.divider() == 0 && bottom.channels.iter().all(|c| c.id != -1) && bottom.has_pixels() => {
            let Painted { color, .. } = m.object(bottom)?;
            Some(Backdrop { color, alpha: vec![1.0; pixels] })
        }
        _ => None,
    };
    let mut canvas = Compositor::new(vec![vec![1.0; pixels]; m.channels], vec![0.0; pixels], false);
    for node in nodes {
        canvas.apply(&mut m, node, false, deep.as_ref())?;
    }
    for (out, plane) in slot.color.into_iter().zip(canvas.result_isolated(&m)) {
        out.copy_from_slice(&plane);
    }
    slot.alpha.copy_from_slice(&canvas.alpha_g);
    Ok(m.warnings)
}

/// Whether the merge takes a document in whole levels, as Photoshop 2026
/// composites it (psd_composite8): 8 bits, RGB or gray, and only what that
/// arithmetic was measured on: layers, not groups, none clipped or with
/// layers clipped to it, none knocking out, none with a user mask, every
/// shown layer in a blend mode psd_composite8 takes. Without a user mask a
/// vector mask is only the coverage of a shape that stores no pixels; a
/// layer that stores them holds its shape already.
fn in_levels(header: &Header, nodes: &[Node]) -> bool {
    header.depth == 8
        && matches!(header.mode, Mode::Rgb | Mode::Gray)
        && nodes.iter().all(|node| {
            let r = node.record;
            node.children.is_none()
                && node.clips.is_empty()
                && !node.clips_below()
                && r.knockout() == 0
                && r.mask.is_none()
                && (r.hidden || psd_composite8::takes(&r.blend_mode()))
        })
}

/// A band of a document `in_levels` takes, merged in whole levels: each
/// shown layer's own color and coverage read as the float merge reads them,
/// then composited onto the band by psd_composite8 at the layer's opacity
/// and fill opacity, in its blend mode.
fn merge_band_levels(whole: &Merge, nodes: &[Node], slot: Slot) -> Result<Vec<String>, String> {
    let mut m = whole.band(slot.top, slot.rows);
    let (pixels, channels) = (m.pixels(), m.channels);
    let level = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as i32;
    let mut color = vec![vec![255i32; pixels]; channels];
    let mut alpha = vec![0i32; pixels];
    let mut canvas = Compositor::new(vec![vec![1.0; pixels]; channels], vec![0.0; pixels], false);
    let (mut backdrop, mut source) = (vec![0i32; channels], vec![0i32; channels]);
    for node in nodes {
        let record = node.record;
        if record.hidden {
            continue;
        }
        let Some((own, stroked)) = Compositor::own(&mut m, node)? else { continue };
        let resolved = canvas.resolve(&mut m, node, None, own, stroked)?;
        let paint = psd_composite8::Paint::new(record.blend_mode(), record.opacity, level(resolved.fill) as u8);
        for i in 0..pixels {
            for c in 0..channels {
                backdrop[c] = color[c][i];
                source[c] = level(resolved.color[c][i]);
            }
            psd_composite8::over(&paint, &source, level(resolved.shape[i]), &mut backdrop, &mut alpha[i]);
            for c in 0..channels {
                color[c][i] = backdrop[c];
            }
        }
    }
    for (out, plane) in slot.color.into_iter().zip(&color) {
        for (o, &v) in out.iter_mut().zip(plane) {
            *o = v as f32 / 255.0;
        }
    }
    for (o, &a) in slot.alpha.iter_mut().zip(&alpha) {
        *o = a as f32 / 255.0;
    }
    Ok(m.warnings)
}

/// The document's layers merged in bands of `rows` rows, dealt out in turn
/// to the machine's cores. Each band goes through every layer as the whole
/// canvas would, so each pixel takes the arithmetic it takes in a merge of
/// the whole at once, and each band leaves out what every other does, which
/// the first band reports; a band that cannot merge stops the bands not
/// yet begun.
fn merge_in_bands(header: &Header, section: &LayerSection, icc: Option<&[u8]>, colors: Colors, rows: usize) -> Result<Canvas, String> {
    if matches!(header.mode, Mode::Bitmap | Mode::Indexed) {
        return Err(format!("its layers would need merging, and the module merges no {} document's layers", if header.mode == Mode::Bitmap { "bitmap" } else { "indexed" }));
    }
    let mut records = records(header, section.info)?;
    let drawn = drawn(&tree(&records), records.len());
    prepare(header, &mut records, &drawn);
    let nodes = tree(&records);
    let levels = in_levels(header, &nodes);
    let whole = Merge::whole(header, &section.blocks, icc, colors);
    let (width, channels, rows) = (header.width, whole.channels, rows.max(1));
    let band = (rows * width).max(1);
    let mut color = vec![vec![0.0f32; whole.pixels()]; channels];
    let mut alpha = vec![0.0f32; whole.pixels()];
    let mut slots: Vec<Slot> = alpha.chunks_mut(band).enumerate().map(|(b, alpha)| Slot { top: b * rows, rows: alpha.len() / width, color: Vec::with_capacity(channels), alpha }).collect();
    for plane in &mut color {
        for (slot, part) in slots.iter_mut().zip(plane.chunks_mut(band)) {
            slot.color.push(part);
        }
    }
    if slots.is_empty() {
        slots.push(Slot { top: 0, rows: 0, color: Vec::new(), alpha: Default::default() });
    }
    let cores = std::thread::available_parallelism().map_or(1, |n| n.get()).min(slots.len());
    let mut shares: Vec<Vec<Slot>> = (0..cores).map(|_| Vec::new()).collect();
    for (i, slot) in slots.into_iter().enumerate() {
        shares[i % cores].push(slot);
    }
    let failed = AtomicBool::new(false);
    let (whole, records, nodes, failed) = (&whole, &records, &nodes, &failed);
    let mut outcomes: Vec<(usize, Result<Vec<String>, String>)> = std::thread::scope(|scope| {
        let workers: Vec<_> = shares
            .into_iter()
            .map(|share| {
                scope.spawn(move || {
                    let mut outcomes = Vec::with_capacity(share.len());
                    for slot in share {
                        if failed.load(Ordering::Relaxed) {
                            break;
                        }
                        let top = slot.top;
                        let outcome = if levels { merge_band_levels(whole, nodes, slot) } else { merge_band(whole, records, nodes, slot) };
                        if outcome.is_err() {
                            failed.store(true, Ordering::Relaxed);
                        }
                        outcomes.push((top, outcome));
                    }
                    outcomes
                })
            })
            .collect();
        workers.into_iter().flat_map(|worker| worker.join().unwrap_or_else(|panic| std::panic::resume_unwind(panic))).collect()
    });
    outcomes.sort_by_key(|&(top, _)| top);
    let mut bands = outcomes.into_iter().map(|(_, outcome)| outcome);
    let warnings = match bands.next() {
        Some(first) => first?,
        None => unreachable!("every merge has a band, and a band is passed over only once another has failed"),
    };
    for band in bands {
        band?;
    }
    Ok(Canvas { color, alpha, warnings })
}

/// The rows of a band of the merge. Each layer makes buffers a band's size,
/// so small bands keep the merge's work in the processor's cache; two rows
/// are fastest or nearly so on documents from 530 to 7106 pixels wide,
/// where a single row of a very wide document makes many requests of a
/// size that queue in the system's allocator.
const BAND_ROWS: usize = 2;

/// The document's layers merged, in bands of `BAND_ROWS` rows.
fn merge_layers(header: &Header, section: &LayerSection, icc: Option<&[u8]>, colors: Colors) -> Result<Canvas, String> {
    merge_in_bands(header, section, icc, colors, BAND_ROWS)
}

/// The document's layers merged into its picture, at its own depth, CMYK's
/// inks the way round the module converts them, a fill color of another
/// mode converted into the document's samples and a document's light given
/// by `colors`.
pub(crate) fn merged(header: &Header, section: &LayerSection, _color_data: &[u8], resources: &Resources, colors: Colors) -> Result<Merged, String> {
    let Canvas { color, alpha, mut warnings } = merge_layers(header, section, resources.icc.as_deref(), colors)?;
    let has_alpha = alpha.iter().any(|&a| a < 1.0);
    let channels = color.len();
    let cmyk = header.mode == Mode::Cmyk;
    let value = |c: usize, i: usize| if cmyk { 1.0 - color[c][i] } else { color[c][i] };
    let mut floats = Vec::with_capacity(alpha.len() * (channels + usize::from(has_alpha)));
    for (i, &a) in alpha.iter().enumerate() {
        for c in 0..channels {
            floats.push(value(c, i));
        }
        if has_alpha {
            floats.push(a);
        }
    }
    let samples = match header.depth {
        8 => Samples::Eight(floats.iter().map(|&x| (x.clamp(0.0, 1.0) * 255.0).round() as u8).collect()),
        16 => Samples::Sixteen(floats.iter().map(|&x| (x.clamp(0.0, 1.0) * 65535.0).round() as u16).collect()),
        _ => Samples::Float(floats),
    };
    if header.mode == Mode::Duotone {
        warnings.push("the document is a duotone, which loaded as its gray, without its inks".to_string());
    }
    Ok(Merged { picture: Picture { width: header.width as u32, height: header.height as u32, color: color_of(header.mode), alpha: has_alpha, samples }, warnings })
}

/// The alpha of the document's layers merged, and what the merge left out.
pub(crate) fn merged_alpha(header: &Header, section: &LayerSection, resources: &Resources, colors: Colors) -> Result<(Vec<f32>, Vec<String>), String> {
    let canvas = merge_layers(header, section, resources.icc.as_deref(), colors)?;
    Ok((canvas.alpha, canvas.warnings))
}

/// Whether the stored channel `plane` is within 1/255 of full scale of
/// `alpha` at every pixel: 1 level at 8 bits, 257 at 16, and 1/255 of a
/// 32-bit float.
pub(crate) fn matches(plane: &[u8], alpha: &[f32], header: &Header) -> bool {
    match header.depth {
        8 => plane.len() == alpha.len() && plane.iter().zip(alpha).all(|(&v, &a)| (i32::from(v) - (a.clamp(0.0, 1.0) * 255.0).round() as i32).abs() <= 1),
        16 => plane.len() == 2 * alpha.len() && plane.as_chunks::<2>().0.iter().zip(alpha).all(|(c, &a)| (i32::from(u16::from_be_bytes(*c)) - (a.clamp(0.0, 1.0) * 65535.0).round() as i32).abs() <= 257),
        _ => plane.len() == 4 * alpha.len() && plane.as_chunks::<4>().0.iter().zip(alpha).all(|(c, &a)| (f32::from_be_bytes(*c) - a).abs() <= 1.0 / 255.0),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::psd_reference as reference;

    /// The largest differences, in 8-bit levels, between the merge of
    /// `document`'s layers and psd-tools' `color` and `alpha`: the color
    /// premultiplied by alpha on both sides, so a color under an alpha of
    /// nearly 0 counts as little as it shows.
    fn largest_difference(document: &[u8], color: &[u8], alpha: &[u8]) -> (i32, i32) {
        largest_difference_over(document, color, alpha, &|_| true)
    }

    /// The largest differences as `largest_difference` measures them, over
    /// the pixels `compared` keeps.
    fn largest_difference_over(document: &[u8], color: &[u8], alpha: &[u8], compared: &dyn Fn(usize) -> bool) -> (i32, i32) {
        let s = psd::sections(document).expect("the document reads");
        let merged = merged(&s.header, &s.layers, s.color_data, &s.resources, crate::image_decode::COLORS).expect("the layers merge");
        let Samples::Eight(samples) = &merged.picture.samples else { panic!("an 8-bit document merges to 8-bit samples") };
        let channels = s.header.mode.colors();
        let stride = channels + usize::from(merged.picture.alpha);
        let (mut worst_color, mut worst_alpha) = (0, 0);
        for (p, (ours, &theirs_alpha)) in samples.chunks_exact(stride).zip(alpha).enumerate() {
            if !compared(p) {
                continue;
            }
            let ours_alpha = if merged.picture.alpha { i32::from(ours[channels]) } else { 255 };
            worst_alpha = worst_alpha.max((ours_alpha - i32::from(theirs_alpha)).abs());
            for c in 0..channels {
                let premultiplied = |value: u8, alpha: i32| (f64::from(value) * f64::from(alpha) / 255.0).round() as i32;
                let difference = premultiplied(ours[c], ours_alpha) - premultiplied(color[p * channels + c], i32::from(theirs_alpha));
                worst_color = worst_color.max(difference.abs());
            }
        }
        (worst_color, worst_alpha)
    }

    /// A reference document: its name, its bytes, and psd-tools' merge of
    /// its layers, color then alpha.
    type Reference<'a> = (&'a str, &'a [u8], &'a [u8], &'a [u8]);

    #[test]
    fn merges_each_blend_mode_as_psd_tools_does() {
        let documents: [Reference; 3] = [
            ("RGB", &reference::BLEND_RGB_DOCUMENT, &reference::BLEND_RGB_COLOR, &reference::BLEND_RGB_ALPHA),
            ("CMYK", &reference::BLEND_CMYK_DOCUMENT, &reference::BLEND_CMYK_COLOR, &reference::BLEND_CMYK_ALPHA),
            ("gray", &reference::BLEND_GRAY_DOCUMENT, &reference::BLEND_GRAY_COLOR, &reference::BLEND_GRAY_ALPHA),
        ];
        for (name, document, color, alpha) in documents {
            assert_eq!(largest_difference(document, color, alpha), (0, 0), "{name}: the color and alpha levels from psd-tools' merge");
        }
    }

    #[test]
    fn merges_groups_clipped_layers_and_hidden_layers_as_psd_tools_does() {
        let difference = largest_difference(&reference::STRUCTURE_RGB_DOCUMENT, &reference::STRUCTURE_RGB_COLOR, &reference::STRUCTURE_RGB_ALPHA);
        assert_eq!(difference, (0, 0), "the color and alpha levels from psd-tools' merge");
    }

    /// The pixels of `document` some layer's vector mask's path passes
    /// through (psd_vector's crossed): the pixels a shape's edge may cover
    /// in part. Every other pixel a mask covers wholly or not at all.
    fn crossed(document: &[u8]) -> Vec<bool> {
        let s = psd::sections(document).expect("the document reads");
        let records = records(&s.header, s.layers.info).expect("the layer records read");
        let (width, height) = (s.header.width, s.header.height);
        let mut marked = vec![false; width * height];
        for record in &records {
            let Some(data) = record.vector_mask() else { continue };
            let crossed = psd_vector::read(data).expect("the vector mask reads").crossed(width, height);
            for (m, c) in marked.iter_mut().zip(crossed) {
                *m |= c;
            }
        }
        marked
    }

    /// The pixels psd-tools' box for a shape cuts from it: inside what its
    /// vector mask may cover, `painted_box`, but outside the layer's box,
    /// over which psd-tools paints the fill. Photoshop covers them, so the
    /// psd-tools comparisons leave them out.
    fn cut_by_box(document: &[u8]) -> Vec<bool> {
        let s = psd::sections(document).expect("the document reads");
        let records = records(&s.header, s.layers.info).expect("the layer records read");
        let m = Merge::whole(&s.header, &s.layers.blocks, s.resources.icc.as_deref(), crate::image_decode::COLORS);
        let mut marked = vec![false; m.width * m.height];
        for record in records.iter().filter(|r| !r.has_pixels() && r.vector_mask().is_some()) {
            let rect = m.layer_box(record).expect("the layer's box reads");
            let reach = m.painted_box(record, rect).expect("the vector mask reads");
            for y in reach.top.max(0)..reach.bottom.min(m.height as i32) {
                for x in reach.left.max(0)..reach.right.min(m.width as i32) {
                    if x < rect.left || x >= rect.right || y < rect.top || y >= rect.bottom {
                        marked[y as usize * m.width + x as usize] = true;
                    }
                }
            }
        }
        marked
    }

    /// The CMYK fills document, solid colors through vector masks, which the
    /// merge composites in floats as psd-tools does until Photoshop's CMYK
    /// arithmetic is measured, merges to psd-tools' merge, level for level,
    /// at every pixel no mask's path passes through and psd-tools' box does
    /// not cut from a shape: there psd-tools and Photoshop both cover a
    /// pixel wholly or not at all. The pixels a path crosses are drawn as
    /// Photoshop draws them, which draws_fills_as_photoshop_drew_them
    /// checks, and the pixels the box cuts as Photoshop covers them, which
    /// paints_live_shapes_past_their_origination_box_as_photoshop_did
    /// checks; the RGB and gray documents are checked against Photoshop's
    /// composites by merges_fills_as_photoshop_composites_them.
    #[test]
    fn merges_fills_and_vector_masks_as_psd_tools_does_inside_and_outside() {
        let documents: [Reference; 1] = [("CMYK", &reference::FILLS_CMYK_DOCUMENT, &reference::FILLS_CMYK_COLOR, &reference::FILLS_CMYK_ALPHA)];
        for (name, document, color, alpha) in documents {
            let s = psd::sections(document).expect("the document reads");
            let warnings = merged(&s.header, &s.layers, s.color_data, &s.resources, crate::image_decode::COLORS).expect("the layers merge").warnings;
            assert!(warnings.is_empty(), "{name}: the merge draws every layer, but warns {warnings:?}");
            let crossed = crossed(document);
            let cut = cut_by_box(document);
            assert!(crossed.iter().zip(&cut).any(|(&c, &k)| !c && !k), "{name}: every pixel is on a path or cut by a box");
            assert_eq!(largest_difference_over(document, color, alpha, &|p| !crossed[p] && !cut[p]), (0, 0), "{name}: the color and alpha levels from psd-tools' merge");
        }
    }

    /// Each live shape of the liveshape render document, a rectangle or an
    /// ellipse made with Photoshop's shape tool at fractional bounds, whose
    /// origination box truncated toward 0 falls short of its right and
    /// bottom edge pixels, painted as the merge paints a shape that stores
    /// no pixels, its fill through its vector mask, has the shape and color
    /// Photoshop stored at every pixel, to the nearest level; and some of
    /// the pixels it covers are past its box.
    #[test]
    fn paints_live_shapes_past_their_origination_box_as_photoshop_did() {
        let document: &[u8] = include_bytes!("../tests/ps-render/liveshape.psd");
        let s = psd::sections(document).expect("the document reads");
        let records = records(&s.header, s.layers.info).expect("the layer records read");
        let m = Merge::whole(&s.header, &s.layers.blocks, s.resources.icc.as_deref(), crate::image_decode::COLORS);
        let level = |v: f32| (v * 255.0).round() as i32;
        let mut past_box = 0;
        for record in &records {
            let fill = m.fill(record).expect("the fill reads").expect("the fill is drawn");
            let coverage = psd_vector::read(record.vector_mask().expect("a vector mask")).expect("the vector mask reads").coverage(m.width, m.height);
            let stored = m.object(record).expect("the stored pixels read");
            let rect = m.layer_box(record).expect("the layer's box reads");
            for (i, &covered) in coverage.iter().enumerate() {
                let (x, y) = ((i % m.width) as i32, (i / m.width) as i32);
                assert_eq!(level(fill.shape[i] * covered), level(stored.shape[i]), "'{}' pixel {x}, {y}'s shape", record.name);
                if stored.shape[i] > 0.0 {
                    for c in 0..m.channels {
                        assert_eq!(level(fill.color[c][i]), level(stored.color[c][i]), "'{}' pixel {x}, {y}, channel {c}", record.name);
                    }
                    if x < rect.left || x >= rect.right || y < rect.top || y >= rect.bottom {
                        past_box += 1;
                    }
                }
            }
        }
        assert!(past_box > 0, "no pixel a shape covers is past its box");
    }

    /// A CROSS document to merge: its name, its bytes, the color of each
    /// column's fill, and the profile it is given.
    type Cross<'a> = (&'a str, &'a [u8], Vec<crate::psd_fill::Foreign>, Option<&'a [u8]>);

    /// Each column of the CROSS documents merges to what the module's
    /// conversion makes of its fill's color: without a profile by the
    /// import's no-profile reading inverted, and with a profile given the
    /// document into it. A profile that only reads colors leaves each layer
    /// out with a warning that says so.
    #[test]
    fn merges_fills_of_other_modes_as_their_conversion_gives_them() {
        use crate::fill_reference as f;
        use crate::profile_reference as r;
        use crate::psd_fill::Foreign;
        // The colors tools/psd_reference.py's CROSS_COLORS hold, as solid()
        // reads them.
        let rgbc = Foreign::Srgb([200.0 / 255.0, 60.0 / 255.0, 30.0 / 255.0]);
        let hsbc = Foreign::Srgb([0.32, 0.64, 0.8]);
        let grsc = Foreign::Srgb([0.65; 3]);
        let cmyc = Foreign::Srgb([0.9 * 0.95, 0.3 * 0.95, 0.15 * 0.95]);
        let lbcl = Foreign::Lab([60.0, 40.0, 30.0]);
        let cases: [Cross; 7] = [
            ("RGB", &reference::CROSS_RGB_DOCUMENT, vec![grsc, cmyc, lbcl], None),
            ("RGB in Display P3", &reference::CROSS_RGB_DOCUMENT, vec![grsc, cmyc, lbcl], Some(&r::DISPLAYP3_PROFILE)),
            ("gray", &reference::CROSS_GRAY_DOCUMENT, vec![rgbc, hsbc, cmyc, lbcl], None),
            ("gray in sGrey", &reference::CROSS_GRAY_DOCUMENT, vec![rgbc, hsbc, cmyc, lbcl], Some(&r::SGREY_PROFILE)),
            ("CMYK", &reference::CROSS_CMYK_DOCUMENT, vec![rgbc, hsbc, grsc, lbcl], None),
            ("CMYK in a lut16 profile", &reference::CROSS_CMYK_DOCUMENT, vec![rgbc, hsbc, grsc, lbcl], Some(&f::CMYK_V2_PROFILE)),
            ("Lab", &reference::CROSS_LAB_DOCUMENT, vec![rgbc, grsc, cmyc], None),
        ];
        for (name, document, colors, icc) in cases {
            let mut s = psd::sections(document).expect("the document reads");
            s.resources.icc = icc.map(<[u8]>::to_vec);
            let merged = merged(&s.header, &s.layers, s.color_data, &s.resources, crate::image_decode::COLORS).expect("the layers merge");
            assert!(merged.warnings.is_empty(), "{name}: the merge draws every layer, but warns {:?}", merged.warnings);
            let Samples::Eight(samples) = &merged.picture.samples else { panic!("an 8-bit document merges to 8-bit samples") };
            let channels = s.header.mode.colors();
            let stride = channels + usize::from(merged.picture.alpha);
            let width = s.header.width / colors.len();
            for (column, &color) in colors.iter().enumerate() {
                let at = (s.header.height / 2 * s.header.width + column * width + width / 2) * stride;
                let converted = crate::image_decode::fill_color(color, &s.header, s.resources.icc.as_deref()).expect("converted");
                let cmyk = s.header.mode == Mode::Cmyk;
                let expected: Vec<u8> = converted.iter().map(|&v| ((if cmyk { 1.0 - v } else { v }).clamp(0.0, 1.0) * 255.0).round() as u8).collect();
                assert_eq!(&samples[at..at + channels], &expected[..], "{name}: column {column}, {color:?}");
            }
        }
        let mut s = psd::sections(&reference::CROSS_CMYK_DOCUMENT).expect("the document reads");
        s.resources.icc = Some(r::CMYK_PROFILE.to_vec());
        let warnings = merged(&s.header, &s.layers, s.color_data, &s.resources, crate::image_decode::COLORS).expect("the layers merge").warnings;
        assert_eq!(warnings.len(), 4, "a warning for each layer: {warnings:?}");
        assert!(warnings.iter().all(|w| w.contains("does not convert into it: its color profile has no table for writing colors")), "{warnings:?}");
    }

    /// One solid ellipse whose vector mask is inverted, over nothing, merges
    /// to its color over the canvas outside the ellipse, alpha 1 minus
    /// psd-tools' coverage of it, level for level at every pixel its path
    /// does not pass through; with the mask's invert flag cleared, the same
    /// document does not.
    #[test]
    fn merges_an_inverted_vector_mask_as_1_minus_its_paths() {
        let document = &reference::INVERTED_RGB_DOCUMENT;
        let s = psd::sections(document).expect("the document reads");
        let warnings = merged(&s.header, &s.layers, s.color_data, &s.resources, crate::image_decode::COLORS).expect("the layers merge").warnings;
        assert!(warnings.is_empty(), "the merge draws the layer, but warns {warnings:?}");
        let crossed = crossed(document);
        assert_eq!(largest_difference_over(document, &reference::INVERTED_RGB_COLOR, &reference::INVERTED_RGB_ALPHA, &|p| !crossed[p]), (0, 0), "the color and alpha levels");
        // The vmsk key, its length, the block's version, then its flags, the
        // invert bit in their last byte.
        let flags = document.windows(4).position(|w| w == b"vmsk").expect("a vmsk block") + 15;
        let mut plain = document.to_vec();
        plain[flags] &= !1;
        assert_ne!(largest_difference(&plain, &reference::INVERTED_RGB_COLOR, &reference::INVERTED_RGB_ALPHA), (0, 0), "with the invert flag cleared");
    }

    /// A merge in bands of any number of rows gives each pixel the bits a
    /// merge of the whole canvas at once gives it, and the same warnings:
    /// over each blend mode, groups, clipped and hidden layers, fills,
    /// vector masks, an inverted mask, and Photoshop's gradients, shapes
    /// with strokes, adjustments and effects.
    #[test]
    fn merges_in_bands_of_any_rows_as_at_once() {
        let mut documents: Vec<(&str, &[u8])> = vec![
            ("blends in RGB", &reference::BLEND_RGB_DOCUMENT[..]),
            ("blends in CMYK", &reference::BLEND_CMYK_DOCUMENT[..]),
            ("blends in gray", &reference::BLEND_GRAY_DOCUMENT[..]),
            ("groups, clipping and hidden layers", &reference::STRUCTURE_RGB_DOCUMENT[..]),
            ("fills in RGB", &reference::FILLS_RGB_DOCUMENT[..]),
            ("fills in CMYK", &reference::FILLS_CMYK_DOCUMENT[..]),
            ("an inverted vector mask", &reference::INVERTED_RGB_DOCUMENT[..]),
        ];
        documents.extend(PHOTOSHOP.iter().map(|&(path, document, _)| (path, document)));
        let bits = |canvas: &Canvas| -> Vec<u32> { canvas.color.iter().flatten().chain(&canvas.alpha).map(|v| v.to_bits()).collect() };
        for (name, document) in documents {
            let s = psd::sections(document).expect("the document reads");
            let merge = |rows| merge_in_bands(&s.header, &s.layers, s.resources.icc.as_deref(), crate::image_decode::COLORS, rows).expect("the layers merge");
            let at_once = merge(s.header.height);
            for rows in [1, 2, 7] {
                let banded = merge(rows);
                assert!(bits(&banded) == bits(&at_once), "{name}: bands of {rows} rows give other bits than the whole canvas at once");
                assert_eq!(banded.warnings, at_once.warnings, "{name}: bands of {rows} rows");
            }
        }
    }

    /// Photoshop documents from psd-tools 1.26.0's tests/psd_files, as it
    /// ships them, under psd-tools' license in tests/psd-tools/LICENSE: each
    /// one's path there, its bytes, and how many of its gradient-filled
    /// layers store pixels Photoshop drew clear of a stroke.
    const PHOTOSHOP: [(&str, &[u8], usize); 10] = [
        ("colormodes/4x4_8bit_rgb.psd", include_bytes!("../tests/psd-tools/colormodes/4x4_8bit_rgb.psd"), 1),
        ("colormodes/4x4_8bit_grayscale.psd", include_bytes!("../tests/psd-tools/colormodes/4x4_8bit_grayscale.psd"), 1),
        ("colormodes/4x4_8bit_lab.psd", include_bytes!("../tests/psd-tools/colormodes/4x4_8bit_lab.psd"), 1),
        ("colormodes/4x4_8bit_cmyk.psd", include_bytes!("../tests/psd-tools/colormodes/4x4_8bit_cmyk.psd"), 1),
        ("colormodes/4x4_8bit_duotone.psd", include_bytes!("../tests/psd-tools/colormodes/4x4_8bit_duotone.psd"), 1),
        ("colorprofiles/north_america_newspaper.psd", include_bytes!("../tests/psd-tools/colorprofiles/north_america_newspaper.psd"), 1),
        ("adjustments/adjustment_clipping.psd", include_bytes!("../tests/psd-tools/adjustments/adjustment_clipping.psd"), 2),
        ("layers/gradient-fill.psd", include_bytes!("../tests/psd-tools/layers/gradient-fill.psd"), 1),
        ("effects/inner-stroke-transparent-gradient.psd", include_bytes!("../tests/psd-tools/effects/inner-stroke-transparent-gradient.psd"), 1),
        ("effects/stroke-effects.psd", include_bytes!("../tests/psd-tools/effects/stroke-effects.psd"), 10),
    ];

    /// `region` worn away `times` times by a cross: a pixel kept where it
    /// and its four neighbors are.
    fn worn(region: &[bool], width: usize, height: usize, times: usize) -> Vec<bool> {
        let mut now = region.to_vec();
        for _ in 0..times {
            let before = now.clone();
            for y in 0..height {
                for x in 0..width {
                    let kept = |dx: i64, dy: i64| {
                        let (nx, ny) = (x as i64 + dx, y as i64 + dy);
                        nx >= 0 && ny >= 0 && (nx as usize) < width && (ny as usize) < height && before[ny as usize * width + nx as usize]
                    };
                    now[y * width + x] = kept(0, 0) && kept(-1, 0) && kept(1, 0) && kept(0, -1) && kept(0, 1);
                }
            }
        }
        now
    }

    /// The pixels of `record` that hold its fill alone: inside its box, and
    /// for a shape, those its mask covers fully, clear of its stroke, the
    /// stroke's reach inside the outline and two pixels more worn away,
    /// since a pixel all of whose samples are covered may still have the
    /// path cross the strip outside its outermost samples.
    fn fill_alone(m: &Merge, record: &Record) -> Vec<bool> {
        let (width, height) = (m.width, m.height);
        let rect = record.rect;
        let mut region: Vec<bool> = (0..width * height)
            .map(|i| {
                let (x, y) = ((i % width) as i32, (i / width) as i32);
                x >= rect.left && x < rect.right && y >= rect.top && y < rect.bottom
            })
            .collect();
        let Some(data) = record.vector_mask() else { return region };
        let coverage = psd_vector::read(data).expect("the vector mask reads").coverage(width, height);
        for (r, c) in region.iter_mut().zip(&coverage) {
            *r = *r && *c == 1.0;
        }
        let Some(stroke) = record.stroke().expect("the stroke reads") else { return region };
        if stroke.flag("strokeEnabled") != Some(true) {
            return region;
        }
        let wide = crate::psd_fill::number(&stroke, "strokeStyleLineWidth").unwrap_or(0.0);
        let reach = match stroke.get("strokeStyleLineAlignment") {
            Some(Value::Enum(_, side)) if side == "strokeStyleAlignInside" => wide.ceil() as usize,
            Some(Value::Enum(_, side)) if side == "strokeStyleAlignCenter" => (wide / 2.0).ceil() as usize,
            _ => 0,
        };
        worn(&region, width, height, reach + 2)
    }

    /// Photoshop 2026 render documents made for the rasterizer, under
    /// tests/ps-render: each one's name, its bytes, and its paths file,
    /// the coordinates Photoshop drew its layers from
    /// (tools/ps_render_paths.py).
    const RENDERS: [(&str, &[u8], &str); 12] = [
        ("edges", include_bytes!("../tests/ps-render/edges.psd"), include_str!("../tests/ps-render/edges.paths")),
        ("curves", include_bytes!("../tests/ps-render/curves.psd"), include_str!("../tests/ps-render/curves.paths")),
        ("split", include_bytes!("../tests/ps-render/split.psd"), include_str!("../tests/ps-render/split.paths")),
        ("random", include_bytes!("../tests/ps-render/random.psd"), include_str!("../tests/ps-render/random.paths")),
        ("flatness", include_bytes!("../tests/ps-render/flatness.psd"), include_str!("../tests/ps-render/flatness.paths")),
        ("combine", include_bytes!("../tests/ps-render/combine.psd"), include_str!("../tests/ps-render/combine.paths")),
        ("strokecaps", include_bytes!("../tests/ps-render/strokecaps.psd"), include_str!("../tests/ps-render/strokecaps.paths")),
        ("strokejoins", include_bytes!("../tests/ps-render/strokejoins.psd"), include_str!("../tests/ps-render/strokejoins.paths")),
        ("strokelines", include_bytes!("../tests/ps-render/strokelines.psd"), include_str!("../tests/ps-render/strokelines.paths")),
        ("geometry", include_bytes!("../tests/ps-render/geometry.psd"), include_str!("../tests/ps-render/geometry.paths")),
        ("placement", include_bytes!("../tests/ps-render/placement.psd"), include_str!("../tests/ps-render/placement.paths")),
        ("mix", include_bytes!("../tests/ps-render/mix.psd"), include_str!("../tests/ps-render/mix.paths")),
    ];

    /// A render layer's stroke settings, where its stroke is on.
    fn stroke_on(record: &Record) -> Option<Descriptor> {
        record.stroke().expect("the stroke reads").filter(|s| s.flag("strokeEnabled") == Some(true))
    }

    /// A subpath of a render document's paths file: whether it is closed,
    /// its operation, and its knots, each its anchor, handle before and
    /// handle after in pixels.
    struct RenderPath {
        closed: bool,
        operation: i16,
        knots: Vec<[f64; 6]>,
    }

    /// A render document's paths file read: each fill layer's name and its
    /// subpaths as drawn, from its `layer`, `path` and `knot` lines.
    fn render_paths(text: &str) -> Vec<(String, Vec<psd_vector::Drawn>)> {
        let mut layers: Vec<(String, Vec<RenderPath>)> = Vec::new();
        for line in text.lines() {
            let (word, rest) = line.split_once(' ').unwrap_or((line, ""));
            match word {
                "canvas" => {}
                "layer" => layers.push((rest.to_string(), Vec::new())),
                "path" => {
                    let (closed, operation) = rest.split_once(' ').expect("a path line names whether it is closed and its operation");
                    let subpaths = &mut layers.last_mut().expect("a path line follows a layer line").1;
                    subpaths.push(RenderPath { closed: closed == "closed", operation: operation.parse().expect("an operation number"), knots: Vec::new() });
                }
                "knot" => {
                    let v: Vec<f64> = rest.split(' ').map(|n| n.parse().expect("a knot's number")).collect();
                    let knot: [f64; 6] = v.try_into().expect("a knot of six numbers");
                    let subpaths = &mut layers.last_mut().expect("a knot line follows a layer line").1;
                    subpaths.last_mut().expect("a knot line follows a path line").knots.push(knot);
                }
                other => panic!("a paths file line starts with {other:?}"),
            }
        }
        let cubic = |a: &[f64; 6], b: &[f64; 6]| [[a[0], a[1]], [a[4], a[5]], [b[2], b[3]], [b[0], b[1]]];
        layers
            .into_iter()
            .map(|(name, subpaths)| {
                let drawn = subpaths
                    .into_iter()
                    .map(|path| {
                        let knots = &path.knots;
                        let mut cubics: Vec<psd_raster::Cubic> = knots.windows(2).map(|w| cubic(&w[0], &w[1])).collect();
                        if path.closed && knots.len() > 1 {
                            cubics.push(cubic(&knots[knots.len() - 1], &knots[0]));
                        }
                        psd_vector::Drawn { closed: path.closed, operation: path.operation, cubics }
                    })
                    .collect();
                (name, drawn)
            })
            .collect()
    }

    /// Each fill layer of the render documents, its stroke off, drawn from
    /// the coordinates Photoshop drew it from, covers every pixel of the
    /// canvas with the samples Photoshop stored for it, level for level:
    /// rectangles at fractional edges, circles and curves, split and random
    /// shapes of curves, curves near the flattening's limits, overlapping
    /// shapes added, subtracted, intersected and excluded, and the shapes
    /// the stroke documents also stroke.
    #[test]
    fn draws_fills_as_photoshop_drew_them() {
        for (batch, document, paths) in RENDERS {
            let s = psd::sections(document).expect("the document reads");
            let records = records(&s.header, s.layers.info).expect("the layer records read");
            let m = Merge::whole(&s.header, &s.layers.blocks, s.resources.icc.as_deref(), crate::image_decode::COLORS);
            let (width, height) = (s.header.width, s.header.height);
            let layers = render_paths(paths);
            assert!(!layers.is_empty(), "{batch}: the paths file holds no layer");
            for (name, drawn) in &layers {
                let record = records.iter().find(|r| &r.name == name).unwrap_or_else(|| panic!("{batch}: no layer '{name}'"));
                if stroke_on(record).is_some() {
                    continue;
                }
                let stored = m.object(record).expect("the stored pixels read");
                let runs = psd_vector::sample_paths(drawn, false, false, 0, height * psd_raster::DOWN as usize, [0, width as i64 * psd_raster::ACROSS]);
                let counts = psd_raster::pixel_counts(&runs, height, 0, width);
                for (i, (&count, &shape)) in counts.iter().zip(&stored.shape).enumerate() {
                    let level = (shape * 255.0).round() as u8;
                    assert_eq!(count, level, "{batch}: layer '{name}' pixel {}, {} covers {count} samples where Photoshop stored {level}", i % width, i / width);
                }
            }
        }
    }

    /// Stroked render layers the stroker draws but not yet as Photoshop
    /// drew them, by document and layer name: round caps and round joins at
    /// some widths and turns, square caps on vees, the inner corners of
    /// center strokes on closed polygons, and two strokes of a hexagon at
    /// fractional places.
    const STROKE_RESIDUES: &[(&str, &str)] = &[
        ("strokecaps", "cap round 8 30 at 240 128"),
        ("strokecaps", "cap round 32 30 at 300 364"),
        ("strokejoins", "join round 8 turn 30 at 86 86"),
        ("strokejoins", "join round 8 turn 90 at 430 86"),
        ("strokejoins", "join round 8 turn 150 at 774 86"),
        ("strokejoins", "join round 16 turn 90 at 430 258"),
        ("strokejoins", "join round 16 turn 150 at 774 258"),
        ("strokejoins", "join round 32 turn 90 at 430 430"),
        ("strokejoins", "join round 32 turn 150 at 774 430"),
        ("strokelines", "triangle line center"),
        ("geometry", "vee 150 miter round"),
        ("geometry", "vee 150 miter square"),
        ("geometry", "vee 150 round butt"),
        ("geometry", "vee 150 round round"),
        ("geometry", "vee 150 round square"),
        ("geometry", "vee 150 bevel round"),
        ("geometry", "vee 150 bevel square"),
        ("geometry", "vee 45 miter square"),
        ("geometry", "vee 45 round square"),
        ("geometry", "vee 45 bevel square"),
        ("geometry", "vee 20 miter round"),
        ("geometry", "vee 20 round butt"),
        ("geometry", "vee 20 round round"),
        ("geometry", "vee 20 round square"),
        ("geometry", "vee 20 bevel round"),
        ("placement", "center 0.5 triangle"),
        ("placement", "center 0.5 hexagon fractional"),
        ("placement", "center 1 diamond"),
        ("placement", "center 1 hexagon fractional"),
        ("placement", "center 1.5 diamond"),
        ("placement", "center 1.5 triangle"),
        ("placement", "center 1.5 hexagon fractional"),
        ("placement", "center 2 triangle"),
        ("placement", "center 2 hexagon fractional"),
        ("placement", "center 2.5 triangle"),
        ("placement", "center 2.5 hexagon fractional"),
        ("placement", "center 3 diamond"),
        ("placement", "center 3 triangle"),
        ("placement", "center 4 diamond"),
        ("placement", "center 4 triangle"),
        ("placement", "center 4 hexagon fractional"),
        ("placement", "center 5 triangle"),
        ("placement", "center 5 hexagon fractional"),
        ("placement", "inside 5 hexagon fractional"),
        ("placement", "outside 1.5 hexagon fractional"),
        ("mix", "center 3 triangle"),
    ];

    /// Why the stroker leaves stroked render layers undrawn, and how many
    /// it leaves for each reason.
    const STROKES_UNDRAWN: &[(&str, usize)] = &[("it is dashed", 20), ("its path has curves", 56), ("its path has more than one subpath", 4)];

    /// Each stroked layer of the render documents that the stroker draws,
    /// from the coordinates Photoshop drew it from and its stroke as the
    /// document stores it, its fill united with it sample by sample where
    /// the fill is on, covers every pixel of the canvas with the samples
    /// Photoshop stored for it, level for level; each layer of
    /// STROKE_RESIDUES still differs somewhere; and the layers it leaves
    /// undrawn are those STROKES_UNDRAWN counts.
    #[test]
    fn draws_strokes_as_photoshop_drew_them() {
        let mut checked = 0;
        let mut wrong = Vec::new();
        let mut undrawn: std::collections::BTreeMap<String, usize> = std::collections::BTreeMap::new();
        for (batch, document, paths) in RENDERS {
            let s = psd::sections(document).expect("the document reads");
            let records = records(&s.header, s.layers.info).expect("the layer records read");
            let m = Merge::whole(&s.header, &s.layers.blocks, s.resources.icc.as_deref(), crate::image_decode::COLORS);
            let (width, height) = (s.header.width, s.header.height);
            let rows = height * psd_raster::DOWN as usize;
            for (name, drawn) in &render_paths(paths) {
                let record = records.iter().find(|r| &r.name == name).unwrap_or_else(|| panic!("{batch}: no layer '{name}'"));
                let Some(settings) = stroke_on(record) else { continue };
                let stroked = psd_stroke::Stroke::from_descriptor(&settings).and_then(|stroke| psd_stroke::sample_stroke(drawn, &stroke, 0, rows));
                let mut runs = match stroked {
                    Ok(runs) => runs,
                    Err(why) => {
                        *undrawn.entry(why).or_insert(0) += 1;
                        continue;
                    }
                };
                if settings.flag("fillEnabled") == Some(true) {
                    let fill = psd_vector::sample_paths(drawn, false, false, 0, rows, [0, width as i64 * psd_raster::ACROSS]);
                    runs = runs.iter().zip(&fill).map(|(a, b)| psd_raster::combine_runs(a, b, psd_raster::Combine::Union)).collect();
                }
                let stored = m.object(record).expect("the stored pixels read");
                let counts = psd_raster::pixel_counts(&runs, height, 0, width);
                let off = counts.iter().zip(&stored.shape).filter(|&(&count, &shape)| count != (shape * 255.0).round() as u8).count();
                let residue = STROKE_RESIDUES.contains(&(batch, name.as_str()));
                checked += 1;
                if (off > 0) != residue {
                    wrong.push(format!("{batch} '{name}': {off} pixels off{}", if residue { ", listed as a residue" } else { "" }));
                }
            }
        }
        let undrawn: Vec<(&str, usize)> = undrawn.iter().map(|(why, &n)| (why.as_str(), n)).collect();
        assert!(undrawn == STROKES_UNDRAWN && wrong.is_empty(), "{checked} stroked layers drawn; left undrawn, by why: {undrawn:?}; not as listed:\n{}", wrong.join("\n"));
    }

    /// Where `merged`, the merge of an 8-bit document, is more than
    /// `allowed(x, y)` levels from the composite Photoshop stored in `saved`,
    /// a line for each such pixel: the merge's color matted with white by
    /// its alpha, as a stored composite is, against the stored color, and
    /// its alpha against the stored transparency, or 255 where the layer
    /// count is not negative and the file stores none.
    fn composite_misses(merged: &Merged, saved: &[u8], allowed: impl Fn(usize, usize) -> i32) -> Vec<String> {
        let r = |v: i32| (v + 127) / 255;
        let Samples::Eight(samples) = &merged.picture.samples else { panic!("an 8-bit document merges to 8-bit samples") };
        let p = psd::sections(saved).expect("Photoshop's document reads");
        let colors = p.header.mode.colors();
        let transparent = p.layers.count < 0;
        let wanted: Vec<usize> = (0..colors + usize::from(transparent)).collect();
        let planes = psd::composite_planes(p.image_data, &p.header, &wanted).expect("its composite reads");
        let step = colors + usize::from(merged.picture.alpha);
        assert_eq!(samples.len(), p.header.width * p.header.height * step, "the merge covers the document");
        let mut misses = Vec::new();
        for (i, pixel) in samples.chunks(step).enumerate() {
            let (x, y) = (i % p.header.width, i / p.header.width);
            let t = if merged.picture.alpha { i32::from(pixel[colors]) } else { 255 };
            let stored_alpha = if transparent { i32::from(planes[colors][i]) } else { 255 };
            let matted: Vec<i32> = pixel[..colors].iter().map(|&v| r(i32::from(v) * t + 255 * (255 - t))).collect();
            let stored: Vec<i32> = planes[..colors].iter().map(|plane| i32::from(plane[i])).collect();
            let worst = matted.iter().zip(&stored).fold((t - stored_alpha).abs(), |worst, (m, s)| worst.max((m - s).abs()));
            if worst > allowed(x, y) {
                misses.push(format!("pixel {x}, {y}: the merge gives {matted:?} at alpha {t}, Photoshop {stored:?} at alpha {stored_alpha}"));
            }
        }
        misses
    }

    /// The fills reference documents merged as Photoshop composited them on
    /// opening and saving each (tests/ps-render/fills-rgb.psd and
    /// fills-gray.psd), level for level at every pixel.
    #[test]
    fn merges_fills_as_photoshop_composites_them() {
        let documents: [(&str, &[u8], &[u8]); 2] = [
            ("RGB", &reference::FILLS_RGB_DOCUMENT, include_bytes!("../tests/ps-render/fills-rgb.psd")),
            ("gray", &reference::FILLS_GRAY_DOCUMENT, include_bytes!("../tests/ps-render/fills-gray.psd")),
        ];
        for (name, document, saved) in documents {
            let s = psd::sections(document).expect("the document reads");
            let merged = merged(&s.header, &s.layers, s.color_data, &s.resources, crate::image_decode::COLORS).expect("the layers merge");
            let misses = composite_misses(&merged, saved, |_, _| 0);
            assert!(misses.is_empty(), "{name}: {} pixels off:\n{}", misses.len(), misses.join("\n"));
        }
    }

    /// Photoshop 2026's composites of two pixel layers, opened and saved by
    /// it, under tests/ps-render/composite: each document's name and bytes.
    /// Every document is 32 rows of panels 32 columns wide, a level stepping
    /// from 0 to 255 in 32 even steps across a panel or down. A blend mode's
    /// document, named for it, has four panels, a backdrop under a source in
    /// that mode: a gray backdrop at its column's level under a gray source
    /// at its row's level, both opaque, then the source at alpha 128 over the
    /// opaque backdrop, then the opaque source over the backdrop at alpha
    /// 128; and random colors and alphas on both layers. A fill document,
    /// multiply's and the eight modes' that take fill opacity inside, has
    /// the source at opacity 200 and fill opacity 77 in four panels: an
    /// opaque gray source at its row's level over the opaque gray backdrop
    /// at its column's, the same at alpha 128, random colors with random
    /// source alphas over an opaque backdrop of random colors, and random
    /// colors with random alphas over nothing. normal-op77 and
    /// normal-fill128-op128 have three panels, the opaque gray backdrop
    /// under a source of one color for each panel whose alpha is its row's
    /// level, at opacity 77, and at fill opacity 128 and opacity 128; clear
    /// has that source over one color whose alpha is its column's level.
    const COMPOSITES: [(&str, &[u8]); 32] = [
        ("multiply-fill", include_bytes!("../tests/ps-render/composite/multiply-fill.psd")),
        ("color-burn-fill", include_bytes!("../tests/ps-render/composite/color-burn-fill.psd")),
        ("linear-burn-fill", include_bytes!("../tests/ps-render/composite/linear-burn-fill.psd")),
        ("color-dodge-fill", include_bytes!("../tests/ps-render/composite/color-dodge-fill.psd")),
        ("linear-dodge-fill", include_bytes!("../tests/ps-render/composite/linear-dodge-fill.psd")),
        ("vivid-light-fill", include_bytes!("../tests/ps-render/composite/vivid-light-fill.psd")),
        ("linear-light-fill", include_bytes!("../tests/ps-render/composite/linear-light-fill.psd")),
        ("hard-mix-fill", include_bytes!("../tests/ps-render/composite/hard-mix-fill.psd")),
        ("difference-fill", include_bytes!("../tests/ps-render/composite/difference-fill.psd")),
        ("normal", include_bytes!("../tests/ps-render/composite/normal.psd")),
        ("darken", include_bytes!("../tests/ps-render/composite/darken.psd")),
        ("multiply", include_bytes!("../tests/ps-render/composite/multiply.psd")),
        ("color-burn", include_bytes!("../tests/ps-render/composite/color-burn.psd")),
        ("linear-burn", include_bytes!("../tests/ps-render/composite/linear-burn.psd")),
        ("lighten", include_bytes!("../tests/ps-render/composite/lighten.psd")),
        ("screen", include_bytes!("../tests/ps-render/composite/screen.psd")),
        ("color-dodge", include_bytes!("../tests/ps-render/composite/color-dodge.psd")),
        ("linear-dodge", include_bytes!("../tests/ps-render/composite/linear-dodge.psd")),
        ("overlay", include_bytes!("../tests/ps-render/composite/overlay.psd")),
        ("soft-light", include_bytes!("../tests/ps-render/composite/soft-light.psd")),
        ("hard-light", include_bytes!("../tests/ps-render/composite/hard-light.psd")),
        ("vivid-light", include_bytes!("../tests/ps-render/composite/vivid-light.psd")),
        ("linear-light", include_bytes!("../tests/ps-render/composite/linear-light.psd")),
        ("pin-light", include_bytes!("../tests/ps-render/composite/pin-light.psd")),
        ("hard-mix", include_bytes!("../tests/ps-render/composite/hard-mix.psd")),
        ("difference", include_bytes!("../tests/ps-render/composite/difference.psd")),
        ("exclusion", include_bytes!("../tests/ps-render/composite/exclusion.psd")),
        ("subtract", include_bytes!("../tests/ps-render/composite/subtract.psd")),
        ("divide", include_bytes!("../tests/ps-render/composite/divide.psd")),
        ("normal-op77", include_bytes!("../tests/ps-render/composite/normal-op77.psd")),
        ("normal-fill128-op128", include_bytes!("../tests/ps-render/composite/normal-fill128-op128.psd")),
        ("clear", include_bytes!("../tests/ps-render/composite/clear.psd")),
    ];

    /// Each document in COMPOSITES merged as Photoshop composited it, level
    /// for level at every pixel.
    #[test]
    fn merges_blend_modes_and_opacities_as_photoshop_composites_them() {
        let mut off = Vec::new();
        for (name, saved) in COMPOSITES {
            let s = psd::sections(saved).expect("the document reads");
            let merged = merged(&s.header, &s.layers, s.color_data, &s.resources, crate::image_decode::COLORS).expect("the layers merge");
            let misses = composite_misses(&merged, saved, |_, _| 0);
            if let Some(first) = misses.first() {
                off.push(format!("{name}: {} pixels off, the first {first}", misses.len()));
            }
        }
        assert!(off.is_empty(), "{}", off.join("\n"));
    }

    /// Each layer of the mix render document, a red fill with a blue
    /// stroke, painted as the merge paints it from the paths the document
    /// stores, the stroke over the fill sample by sample, has the shape and
    /// color Photoshop stored at every pixel, to the nearest level, but for
    /// its layers in STROKE_RESIDUES and those whose stroke has curves.
    #[test]
    fn paints_strokes_over_fills_as_photoshop_did() {
        let (_, document, paths) = RENDERS.iter().find(|(batch, _, _)| *batch == "mix").expect("the mix document");
        let s = psd::sections(document).expect("the document reads");
        let records = records(&s.header, s.layers.info).expect("the layer records read");
        let m = Merge::whole(&s.header, &s.layers.blocks, s.resources.icc.as_deref(), crate::image_decode::COLORS);
        let level = |v: f32| (v * 255.0).round() as i32;
        let mut painted_layers = 0;
        for (name, _) in &render_paths(paths) {
            if STROKE_RESIDUES.contains(&("mix", name.as_str())) {
                continue;
            }
            let record = records.iter().find(|r| &r.name == name).unwrap_or_else(|| panic!("no layer '{name}'"));
            let settings = stroke_on(record).unwrap_or_else(|| panic!("'{name}' has no stroke"));
            let fill = m.fill(record).expect("the fill reads").expect("the fill is drawn");
            let painted = match m.stroked(record, &settings, Some(&fill)).expect("the stroke reads") {
                Ok(painted) => painted,
                Err(why) => {
                    assert_eq!(why, "its path has curves", "'{name}' is left undrawn");
                    continue;
                }
            };
            let stored = m.object(record).expect("the stored pixels read");
            for i in 0..m.pixels() {
                assert_eq!(level(painted.shape[i]), level(stored.shape[i]), "'{name}' pixel {i}'s shape");
                if stored.shape[i] > 0.0 {
                    for c in 0..m.channels {
                        assert_eq!(level(painted.color[c][i]), level(stored.color[c][i]), "'{name}' pixel {}, {}, channel {c}", i % m.width, i / m.width);
                    }
                }
            }
            painted_layers += 1;
        }
        assert_eq!(painted_layers, 6, "the mix layers painted");
    }

    /// Each gradient-filled layer of psd-tools' Photoshop documents that
    /// stores the pixels Photoshop drew, drawn from its settings, is within
    /// 1.5 levels of those pixels, color and transparency, at every
    /// pixel that holds its fill alone; the reflected gradient in each
    /// color mode, the linear method, a transparency gradient, an angle
    /// gradient and gradients through shapes with strokes among them.
    #[test]
    fn draws_gradients_as_photoshop_drew_them() {
        for (path, document, layers) in PHOTOSHOP {
            let s = psd::sections(document).expect("the document reads");
            let records = records(&s.header, s.layers.info).expect("the layer records read");
            let m = Merge::whole(&s.header, &s.layers.blocks, s.resources.icc.as_deref(), crate::image_decode::COLORS);
            let mut compared = 0;
            for record in &records {
                let Some((FillKind::Gradient, _)) = record.fill_settings().expect("the fill settings read") else { continue };
                if !record.has_pixels() {
                    continue;
                }
                let drawn = m.fill(record).expect("the fill reads").unwrap_or_else(|why| panic!("{path}: layer '{}' {why}", record.name));
                let stored = m.object(record).expect("the stored pixels read");
                let (mut color, mut shape, mut pixels) = (0.0f64, 0.0f64, 0);
                for (i, alone) in fill_alone(&m, record).into_iter().enumerate() {
                    if !alone {
                        continue;
                    }
                    pixels += 1;
                    for c in 0..m.channels {
                        color = color.max(f64::from((drawn.color[c][i] - stored.color[c][i]).abs()) * 255.0);
                    }
                    shape = shape.max(f64::from((drawn.shape[i] - stored.shape[i]).abs()) * 255.0);
                }
                if pixels == 0 {
                    continue;
                }
                assert!(color <= 1.5 && shape <= 1.5, "{path}: layer '{}' is {color:.3} levels from Photoshop's color and {shape:.3} from its transparency over {pixels} pixels", record.name);
                compared += 1;
            }
            assert_eq!(compared, layers, "{path}: the gradient layers compared with Photoshop's pixels");
        }
    }
}
