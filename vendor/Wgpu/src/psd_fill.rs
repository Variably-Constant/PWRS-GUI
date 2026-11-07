//! What a Photoshop layer that stores no pixels is filled with, as
//! psd-tools 1.26.0 draws it: a solid color, from a SoCo block or a shape's
//! vscg, the same over the layer's box, and a pattern, from a PtFl block,
//! the document's pattern of that id tiled over the box from its corner;
//! and the box itself, from the layer's origination items, its vector
//! mask, or its record and the canvas.

use crate::psd::{self, Block, Header, Mode, Reader};
use crate::psd_descriptor::{self, Descriptor, Value};
use crate::psd_vector;

/// A fill's color as the merge holds the document's samples, from 0 to 1:
/// what each ink leaves for CMYK, and L over 100 and a and b plus 128 over
/// 256 for Lab, as Photoshop's 15-bit Lab stores them; a color in another
/// mode than the document's, for the merge
/// to convert; or, for a color the merge does not draw, why, as the end of
/// a warning naming the layer.
#[derive(Debug, PartialEq)]
pub(crate) enum Solid {
    Color(Vec<f32>),
    Foreign(Foreign),
    Undrawn(String),
}

/// A fill color in another mode than its document's, as the merge reads it
/// to convert: sRGB's red, green and blue from 0 to 1 for an RGB or HSB
/// color, a gray's level in all three, and a CMYK color's each the
/// complement of its ink times black's, as the import reads inks without a
/// profile; or CIE Lab under D50, L from 0 to 100 and a and b from -128 to
/// 127.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Foreign {
    Srgb([f64; 3]),
    Lab([f64; 3]),
}

/// What converts a fill color of another mode into a document's samples as
/// the merge holds them, given the document's header and ICC profile; or
/// says why it cannot.
pub(crate) type Convert = fn(Foreign, &Header, Option<&[u8]>) -> Result<Vec<f32>, String>;

/// What takes a color's values to another's, written into the buffer given.
pub(crate) type Mapping = Box<dyn Fn(&[f64], &mut [f64])>;

/// A document's samples as light that mixes linearly and back, for a
/// gradient that mixes in light: `into` writes the `width` values of a
/// color's light from its samples, each from 0 to 1, and `back` writes the
/// samples of a light.
pub(crate) struct Light {
    pub(crate) width: usize,
    pub(crate) into: Mapping,
    pub(crate) back: Mapping,
}

/// What gives a document's light, given its header and ICC profile; or
/// says why its samples have none the merge knows.
pub(crate) type LightOf = fn(&Header, Option<&[u8]>) -> Result<Light, String>;

/// What the merge takes from the module's color handling, which the PSD
/// files keep out of themselves: the conversion of a fill color of another
/// mode, and a document's light.
#[derive(Clone, Copy)]
pub(crate) struct Colors {
    pub(crate) convert: Convert,
    pub(crate) light: LightOf,
}

/// A value within 0 to 1, as psd-tools' _clamp01 holds a descriptor's
/// component; not a number gives 0.
fn clamped(v: f64) -> f64 {
    if v.is_nan() { 0.0 } else { v.clamp(0.0, 1.0) }
}

/// A component within 0 to 1 as the float32 psd-tools' fill array holds.
fn unit(v: f64) -> f32 {
    clamped(v) as f32
}

/// A descriptor item's number, a double, a unit float or an integer.
pub(crate) fn number(d: &Descriptor, key: &str) -> Option<f64> {
    match d.get(key)? {
        Value::Double(v) | Value::Unit(_, v) => Some(*v),
        Value::Integer(v) => Some(*v as f64),
        _ => None,
    }
}

/// A descriptor item that is itself a descriptor.
pub(crate) fn nested<'d>(d: &'d Descriptor, key: &str) -> Option<&'d Descriptor> {
    match d.get(key)? {
        Value::Descriptor(inner) => Some(inner),
        _ => None,
    }
}

/// HSB as RGB, as psd-tools' hsb_to_rgb gives it: the hue a fraction of a
/// turn, folded onto the circle, saturation and brightness held within 0
/// to 1.
fn hsb_to_rgb(h: f64, s: f64, v: f64) -> [f64; 3] {
    let clamp = |x: f64| if x.is_nan() { 0.0 } else { x.clamp(0.0, 1.0) };
    let (s, v) = (clamp(s), clamp(v));
    if s == 0.0 || !h.is_finite() {
        return [v; 3];
    }
    let h = h.rem_euclid(1.0);
    let whole = (h * 6.0).trunc();
    let sector = whole as usize % 6;
    let f = h * 6.0 - whole;
    let (w, q, t) = (v * (1.0 - s), v * (1.0 - s * f), v * (1.0 - s * (1.0 - f)));
    [[v, t, w], [q, v, w], [w, v, t], [w, q, v], [t, w, v], [v, w, q]][sector]
}

/// The color of a fill descriptor's Clr  item in a document of `mode`, as
/// psd-tools' _get_color reads it where the color is in the document's own
/// mode: RGBC's red, green and blue over 255, or its floats; HSBC's hue in
/// degrees and saturation and brightness in percent, as RGB; Grsc's percent
/// of black; CMYC's percents of ink; and LbCl's L, a and b. A duotone's is
/// its gray. A color in another mode than the document's is the sRGB or
/// Lab that `Foreign` describes, for the merge to convert.
pub(crate) fn solid(d: &Descriptor, mode: Mode) -> Result<Solid, String> {
    let color = nested(d, "Clr ").ok_or("its fill holds no color")?;
    let get = |key: &str| number(color, key).ok_or_else(|| format!("its fill's color holds no {}", key.trim_end()));
    let rgb = || -> Result<[f64; 3], String> {
        let values = if color.get("Rd  ").is_some() { [get("Rd  ")? / 255.0, get("Grn ")? / 255.0, get("Bl  ")? / 255.0] } else { [get("redFloat")?, get("greenFloat")?, get("blueFloat")?] };
        Ok(values.map(clamped))
    };
    let hsb = || -> Result<[f64; 3], String> { Ok(hsb_to_rgb(get("H   ")? / 360.0, get("Strt")? / 100.0, get("Brgh")? / 100.0)) };
    let gray = || -> Result<f64, String> { Ok(clamped((100.0 - get("Gry ")?) / 100.0)) };
    let inks = || -> Result<[f64; 4], String> {
        let [c, m, y, k] = ["Cyn ", "Mgnt", "Ylw ", "Blck"].map(|key| get(key).map(|ink| clamped(ink / 100.0)));
        Ok([c?, m?, y?, k?])
    };
    let lab = || -> Result<[f64; 3], String> { Ok([get("Lmnc")?.clamp(0.0, 100.0), get("A   ")?.clamp(-128.0, 127.0), get("B   ")?.clamp(-128.0, 127.0)]) };
    let as_f32 = |values: &[f64]| values.iter().map(|&v| v as f32).collect();
    Ok(match (color.class.as_str(), mode) {
        ("RGBC", Mode::Rgb) => Solid::Color(as_f32(&rgb()?)),
        ("HSBC", Mode::Rgb) => Solid::Color(as_f32(&hsb()?)),
        ("Grsc", Mode::Gray | Mode::Duotone) => Solid::Color(vec![gray()? as f32]),
        ("CMYC", Mode::Cmyk) => Solid::Color(as_f32(&inks()?.map(|ink| 1.0 - ink))),
        ("LbCl", Mode::Lab) => {
            let [l, a, b] = lab()?;
            Solid::Color(vec![unit(l / 100.0), unit((a + 128.0) / 256.0), unit((b + 128.0) / 256.0)])
        }
        ("RGBC", _) => Solid::Foreign(Foreign::Srgb(rgb()?)),
        ("HSBC", _) => Solid::Foreign(Foreign::Srgb(hsb()?)),
        ("Grsc", _) => Solid::Foreign(Foreign::Srgb([gray()?; 3])),
        ("CMYC", _) => {
            let [c, m, y, k] = inks()?;
            Solid::Foreign(Foreign::Srgb([c, m, y].map(|ink| (1.0 - ink) * (1.0 - k))))
        }
        ("LbCl", _) => Solid::Foreign(Foreign::Lab(lab()?)),
        (other, _) => Solid::Undrawn(format!("is filled with a color of the class {other}, which the merge does not read")),
    })
}

/// A pattern fill: its pattern's id, and each of its scale, angle and
/// phase that moves or sizes the pattern from how it is stored, named as a
/// warning names it.
pub(crate) struct PatternFill {
    pub(crate) id: String,
    pub(crate) placed: Vec<String>,
}

impl PatternFill {
    /// What moves or sizes the pattern as one phrase, the settings joined
    /// with commas and the last with and; `None` where nothing does.
    pub(crate) fn placement(&self) -> Option<String> {
        match self.placed.as_slice() {
            [] => None,
            [one] => Some(one.clone()),
            [rest @ .., last] => Some(format!("{} and {last}", rest.join(", "))),
        }
    }
}

/// A pattern fill from its PtFl block: the Ptrn item's id, and a Scl  item
/// other than 100 percent, an Angl item other than 0 degrees, and a phase
/// item, a point of Hrzn and Vrtc, other than 0, 0.
pub(crate) fn pattern_of(d: &Descriptor) -> Result<PatternFill, String> {
    let pattern = nested(d, "Ptrn").ok_or("its pattern fill names no pattern")?;
    let id = match pattern.get("Idnt") {
        Some(Value::Text(id)) => id.trim_end_matches('\0').to_string(),
        _ => return Err("its pattern fill's pattern has no id".to_string()),
    };
    let mut placed = Vec::new();
    let scale = number(d, "Scl ").unwrap_or(100.0);
    if scale != 100.0 {
        placed.push(format!("a scale of {scale}%"));
    }
    let angle = number(d, "Angl").unwrap_or(0.0);
    if angle != 0.0 {
        placed.push(format!("an angle of {angle} degrees"));
    }
    if let Some(phase) = nested(d, "phase") {
        let (x, y) = (number(phase, "Hrzn").unwrap_or(0.0), number(phase, "Vrtc").unwrap_or(0.0));
        if x != 0.0 || y != 0.0 {
            placed.push(format!("a phase of {x}, {y}"));
        }
    }
    Ok(PatternFill { id, placed })
}

/// A pattern as the document stores it: its mode, as a document's header
/// numbers it, its size, its planes, and how many of the first are color,
/// the last the pattern's transparency where there are more.
pub(crate) struct Pattern {
    pub(crate) mode: u32,
    pub(crate) width: usize,
    pub(crate) height: usize,
    pub(crate) planes: Vec<Vec<f32>>,
    pub(crate) colors: usize,
}

/// The pattern of the id `id` among the document's global Patt, Pat2 and
/// Pat3 blocks, as psd-tools reads it; `None` where none has that id.
pub(crate) fn pattern(blocks: &[Block], id: &str) -> Result<Option<Pattern>, String> {
    for (_, data) in blocks.iter().filter(|(key, _)| matches!(key, b"Patt" | b"Pat2" | b"Pat3")) {
        let mut r = Reader::new(data);
        while r.left() >= 4 {
            let length = r.length(4)?;
            let record = r.take(length)?;
            r.take(((4 - length % 4) % 4).min(r.left()))?;
            if let Some(found) = pattern_record(record, id)? {
                return Ok(Some(found));
            }
        }
    }
    Ok(None)
}

/// One pattern record, read where its id is `id`: its version, 1, its mode,
/// its size, its name, its id, an indexed pattern's color table, and its
/// channels, as psd-tools' Pattern and VirtualMemoryArrayList read them.
fn pattern_record(record: &[u8], id: &str) -> Result<Option<Pattern>, String> {
    let mut r = Reader::new(record);
    let version = r.number(4)?;
    if version != 1 {
        return Err(format!("a pattern's version is {version}, where the specification gives 1"));
    }
    let mode = u32::try_from(r.number(4)?).map_err(|e| format!("a pattern's mode does not fit: {e}"))?;
    r.take(4)?;
    let name_length = r.length(4)?;
    r.take(name_length.checked_mul(2).ok_or("a pattern's name does not fit in memory")?)?;
    let id_length = r.length(1)?;
    let own_id = String::from_utf8_lossy(r.take(id_length)?).into_owned();
    if own_id != id {
        return Ok(None);
    }
    if mode == 2 {
        r.take(768 + 4)?;
    }
    let list_version = r.number(4)?;
    if list_version != 3 {
        return Err(format!("a pattern's channel list's version is {list_version}, where the specification gives 3"));
    }
    let list_length = r.length(4)?;
    let mut list = Reader::new(r.take(list_length)?);
    let [top, left, bottom, right] = [0; 4].map(|_| list.number(4));
    let (width, height) = (right?.saturating_sub(left?) as usize, bottom?.saturating_sub(top?) as usize);
    let slots = list.length(4)? + 2;
    let mut planes = Vec::new();
    let mut colors = 0;
    for slot in 0..slots {
        if let Some(plane) = pattern_channel(&mut list)? {
            if plane.len() != width * height {
                return Err(format!("a pattern's channel holds {} values where its {width} by {height} pixels need {}", plane.len(), width * height));
            }
            if slot < slots - 2 {
                colors += 1;
            }
            planes.push(plane);
        }
    }
    if colors == 0 {
        colors = planes.len();
    }
    Ok(Some(Pattern { mode, width, height, planes, colors }))
}

/// One of a pattern's channels, its values from 0 to 1, or `None` where it
/// is not written: whether it is written, its length, its depth, its
/// rectangle, its depth again, its compression and its data, which
/// decompress as a composite's do, a run-length row's count in 2 bytes.
fn pattern_channel(r: &mut Reader) -> Result<Option<Vec<f32>>, String> {
    if r.number(4)? == 0 {
        return Ok(None);
    }
    let length = r.length(4)?;
    if length == 0 {
        return Ok(None);
    }
    if length < 23 {
        return Err(format!("a pattern's channel is {length} bytes, too few to hold one"));
    }
    let mut c = Reader::new(r.take(length)?);
    let depth = c.length(4)?;
    let [top, left, bottom, right] = [0; 4].map(|_| c.number(4));
    let (width, height) = (right?.saturating_sub(left?) as usize, bottom?.saturating_sub(top?) as usize);
    let pixel_depth = c.length(2)?;
    if pixel_depth != depth {
        return Err(format!("a pattern's channel gives its depth as {depth} and {pixel_depth}"));
    }
    let compression = c.number(1)?;
    let row = width.checked_mul(depth).map(|bits| bits.div_ceil(8)).ok_or("a pattern's row does not fit in memory")?;
    let mut counts = Vec::new();
    if compression == 1 {
        for _ in 0..height {
            counts.push(c.length(2)?);
        }
    }
    let bytes = psd::decompressed(compression, &c.bytes[c.at..], &counts, row, height, depth)?;
    Ok(Some(match depth {
        8 => bytes.iter().map(|&b| f32::from(b) / 255.0).collect(),
        16 => bytes.as_chunks::<2>().0.iter().map(|v| f32::from(u16::from_be_bytes(*v)) / 65535.0).collect(),
        32 => bytes.as_chunks::<4>().0.iter().map(|v| f32::from_be_bytes(*v)).collect(),
        other => return Err(format!("a pattern's channel holds {other}-bit samples, which the merge does not read")),
    }))
}

impl Pattern {
    /// Whether the pattern's colors are in the document's mode `mode`, a
    /// gray pattern's in a duotone's gray, and as many as its channels.
    pub(crate) fn fits(&self, mode: Mode) -> bool {
        let same = matches!((self.mode, mode), (1, Mode::Gray | Mode::Duotone) | (8, Mode::Duotone) | (3, Mode::Rgb) | (4, Mode::Cmyk) | (9, Mode::Lab));
        same && self.colors == mode.colors()
    }

    /// The pattern's plane `plane` tiled from the document's corner, over
    /// the document's columns `cols` and rows `rows`, row by row.
    pub(crate) fn tiled(&self, plane: usize, cols: std::ops::Range<i64>, rows: std::ops::Range<i64>) -> Vec<f32> {
        let values = &self.planes[plane];
        let (width, height) = (self.width as i64, self.height as i64);
        let mut out = Vec::with_capacity(cols.clone().count() * rows.clone().count());
        for y in rows {
            let row = y.rem_euclid(height) * width;
            out.extend(cols.clone().map(|x| values[(row + x.rem_euclid(width)) as usize]));
        }
        out
    }
}

/// The box of a shape layer's origination items, its vogk block, as
/// psd-tools' ShapeLayer.bbox takes it: left, top, right and bottom, the
/// union of the items' boxes, each edge truncated toward 0; `None` where it
/// lists no item, the block says the shape is invalidated, an item is, or
/// an item has no box.
pub(crate) fn origination_box(data: &[u8]) -> Result<Option<[i32; 4]>, String> {
    let d = psd_descriptor::versioned(data.get(4..).ok_or("its origination data is cut short")?)?;
    if d.flag("keyShapeInvalidated") == Some(true) {
        return Ok(None);
    }
    let items = match d.get("keyDescriptorList") {
        Some(Value::List(items)) if !items.is_empty() => items,
        _ => return Ok(None),
    };
    let mut edges = [f64::INFINITY, f64::INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY];
    for item in items {
        let Value::Descriptor(item) = item else { return Ok(None) };
        if item.flag("keyShapeInvalidated") == Some(true) {
            return Ok(None);
        }
        let Some(rect) = nested(item, "keyOriginShapeBBox") else { return Ok(None) };
        let sides = ["Left", "Top ", "Rght", "Btom"].map(|key| number(rect, key));
        let [Some(l), Some(t), Some(r), Some(b)] = sides else { return Ok(None) };
        edges = [edges[0].min(l), edges[1].min(t), edges[2].max(r), edges[3].max(b)];
    }
    Ok(Some(edges.map(|e| e.trunc() as i32)))
}

/// The box a vector mask's paths span on a document `width` by `height`,
/// as psd-tools' ShapeLayer.bbox scales it: each edge rounded to the
/// nearest pixel, halves to even, as Python's round takes them.
pub(crate) fn vector_mask_box(mask: &psd_vector::VectorMask, width: usize, height: usize) -> [i32; 4] {
    let [l, t, r, b] = mask.bbox();
    let (w, h) = (width as f64, height as f64);
    [(l * w).round_ties_even() as i32, (t * h).round_ties_even() as i32, (r * w).round_ties_even() as i32, (b * h).round_ties_even() as i32]
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A PtFl descriptor of a pattern named by id, with the items given
    /// after its Ptrn.
    fn pattern_fill(items: Vec<(&str, Value)>) -> Descriptor {
        let pattern = Descriptor { class: "Ptrn".to_string(), items: vec![("Nm  ".to_string(), Value::Text("foo\0".to_string())), ("Idnt".to_string(), Value::Text("an-id\0".to_string()))] };
        let mut all = vec![("Ptrn".to_string(), Value::Descriptor(pattern))];
        all.extend(items.into_iter().map(|(key, value)| (key.to_string(), value)));
        Descriptor { class: "null".to_string(), items: all }
    }

    /// A fill descriptor of a color of `class` holding `items`.
    fn color_fill(class: &str, items: Vec<(&str, Value)>) -> Descriptor {
        let color = Descriptor { class: class.to_string(), items: items.into_iter().map(|(key, value)| (key.to_string(), value)).collect() };
        Descriptor { class: "null".to_string(), items: vec![("Clr ".to_string(), Value::Descriptor(color))] }
    }

    /// A color in another mode than its document's is the sRGB or Lab the
    /// merge converts: RGB and HSB as sRGB, a gray as its level in all three,
    /// CMYK by the import's formula for inks without a profile, and Lab as
    /// itself; in its own mode a color stays the document's samples.
    #[test]
    fn reads_a_color_of_another_mode_as_srgb_or_lab() {
        let d = Value::Double;
        let rgb = color_fill("RGBC", vec![("Rd  ", d(204.0)), ("Grn ", d(51.0)), ("Bl  ", d(0.0))]);
        assert_eq!(solid(&rgb, Mode::Gray).expect("read"), Solid::Foreign(Foreign::Srgb([0.8, 0.2, 0.0])));
        assert_eq!(solid(&rgb, Mode::Rgb).expect("read"), Solid::Color(vec![0.8, 0.2, 0.0]));
        let hsb = color_fill("HSBC", vec![("H   ", Value::Unit("#Ang".to_string(), 0.0)), ("Strt", d(100.0)), ("Brgh", d(100.0))]);
        assert_eq!(solid(&hsb, Mode::Lab).expect("read"), Solid::Foreign(Foreign::Srgb([1.0, 0.0, 0.0])));
        let gray = color_fill("Grsc", vec![("Gry ", d(25.0))]);
        assert_eq!(solid(&gray, Mode::Cmyk).expect("read"), Solid::Foreign(Foreign::Srgb([0.75; 3])));
        assert_eq!(solid(&gray, Mode::Duotone).expect("read"), Solid::Color(vec![0.75]));
        let cmyk = color_fill("CMYC", vec![("Cyn ", d(20.0)), ("Mgnt", d(50.0)), ("Ylw ", d(100.0)), ("Blck", d(50.0))]);
        assert_eq!(solid(&cmyk, Mode::Rgb).expect("read"), Solid::Foreign(Foreign::Srgb([0.4, 0.25, 0.0])));
        assert_eq!(solid(&cmyk, Mode::Cmyk).expect("read"), Solid::Color(vec![0.8, 0.5, 0.0, 0.5]));
        let lab = color_fill("LbCl", vec![("Lmnc", d(60.0)), ("A   ", d(-20.0)), ("B   ", d(30.0))]);
        assert_eq!(solid(&lab, Mode::Gray).expect("read"), Solid::Foreign(Foreign::Lab([60.0, -20.0, 30.0])));
        let other = color_fill("BkCl", vec![("Bk  ", d(1.0))]);
        assert_eq!(solid(&other, Mode::Rgb).expect("read"), Solid::Undrawn("is filled with a color of the class BkCl, which the merge does not read".to_string()));
    }

    /// A point of a pattern's phase.
    fn point(x: f64, y: f64) -> Value {
        Value::Descriptor(Descriptor { class: "Pnt ".to_string(), items: vec![("Hrzn".to_string(), Value::Double(x)), ("Vrtc".to_string(), Value::Double(y))] })
    }

    /// A pattern fill at 100 percent, 0 degrees and a phase of 0, 0, as
    /// psd-tools' example of one holds them, or with none of those items,
    /// moves nothing; any other scale, angle and phase is named, in that
    /// order, as the merge's warning lists them.
    #[test]
    fn names_each_scale_angle_and_phase_that_moves_the_pattern() {
        let unit = |code: &str, v: f64| Value::Unit(code.to_string(), v);
        let stored = pattern_of(&pattern_fill(vec![("Angl", unit("#Ang", 0.0)), ("Scl ", unit("#Prc", 100.0)), ("Algn", Value::Bool(true)), ("phase", point(0.0, 0.0))])).expect("a pattern fill");
        assert_eq!((stored.id.as_str(), stored.placement()), ("an-id", None));
        let bare = pattern_of(&pattern_fill(Vec::new())).expect("a pattern fill");
        assert_eq!(bare.placement(), None);
        let moved = pattern_of(&pattern_fill(vec![("Angl", unit("#Ang", 30.0)), ("Scl ", unit("#Prc", 87.0)), ("phase", point(3.0, -2.5))])).expect("a pattern fill");
        assert_eq!(moved.placed, ["a scale of 87%", "an angle of 30 degrees", "a phase of 3, -2.5"]);
        assert_eq!(moved.placement().as_deref(), Some("a scale of 87%, an angle of 30 degrees and a phase of 3, -2.5"));
    }
}
