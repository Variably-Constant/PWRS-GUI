//! A Photoshop document's picture as the import loads it, read as Adobe's
//! Photoshop File Formats Specification lays the file out: a header, color
//! mode data, image resources, layer and mask information, then the
//! composite image, each channel's plane in turn, uncompressed, PackBits
//! or zip compressed. A .psb file, the large document format, gives some
//! lengths 8 bytes and each PackBits row a 4-byte count.
//!
//! The picture is the composite image, the document's layers merged, where
//! the version info does not say the stored one is not the real merge;
//! where it does, the module merges the layers itself. A bitmap
//! document's pixels read as gray, an indexed one's as RGB through its
//! color table, with its transparent color clear, and a duotone one's as
//! its gray. Of the channels past the color ones, the composite's
//! transparency is the first where the layer count is negative, and the one
//! whose alpha identifier is 0; channels the file names, gives identifiers
//! or display kinds are saved selections and spot inks, never
//! transparency. A channel the file says nothing about is the transparency
//! where it matches the merged layers' alpha within 1 level at every pixel,
//! or, in a document without layers, where it is the only one. Photoshop
//! stores a transparent RGB or gray composite blended with white, which is
//! taken back out where the file marks the transparency, as psd-tools and
//! GIMP do, and where a channel the file says nothing of is the
//! transparency, only if every fully transparent pixel is white.

use std::io::Read as _;

/// A document's color mode, as its header names it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Mode {
    Bitmap,
    Gray,
    Indexed,
    Rgb,
    Cmyk,
    Duotone,
    Lab,
}

impl Mode {
    /// The channels a stored pixel's color takes.
    pub(crate) fn colors(self) -> usize {
        match self {
            Mode::Bitmap | Mode::Gray | Mode::Indexed | Mode::Duotone => 1,
            Mode::Rgb | Mode::Lab => 3,
            Mode::Cmyk => 4,
        }
    }

    /// The color the module reads a pixel of this mode as.
    fn color(self) -> Color {
        match self {
            Mode::Bitmap | Mode::Gray | Mode::Duotone => Color::Gray,
            Mode::Indexed | Mode::Rgb => Color::Rgb,
            Mode::Cmyk => Color::Cmyk,
            Mode::Lab => Color::Lab,
        }
    }

    pub(crate) fn name(self) -> &'static str {
        match self {
            Mode::Bitmap => "bitmap",
            Mode::Gray => "grayscale",
            Mode::Indexed => "indexed",
            Mode::Rgb => "RGB",
            Mode::Cmyk => "CMYK",
            Mode::Duotone => "duotone",
            Mode::Lab => "Lab",
        }
    }
}

/// The color of a picture's samples.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Color {
    Gray,
    Rgb,
    Cmyk,
    Lab,
}

impl Color {
    /// The channels a pixel's color takes.
    pub(crate) fn channels(self) -> usize {
        match self {
            Color::Gray => 1,
            Color::Rgb | Color::Lab => 3,
            Color::Cmyk => 4,
        }
    }
}

/// A picture's samples, interleaved: each pixel's color channels, then its
/// alpha where it has one. CMYK's inks run from 0, none, to the largest
/// value, full. Lab is as the document stores it: L from 0 to the largest
/// value for 0 to 100, and a and b at 8 bits the value less 128, at 16 bits
/// the value over 257 less 128.
#[derive(Debug, PartialEq)]
pub(crate) enum Samples {
    Eight(Vec<u8>),
    Sixteen(Vec<u16>),
    Float(Vec<f32>),
}

/// A document's picture: its size, color and samples.
#[derive(Debug)]
pub(crate) struct Picture {
    pub(crate) width: u32,
    pub(crate) height: u32,
    pub(crate) color: Color,
    pub(crate) alpha: bool,
    pub(crate) samples: Samples,
}

/// A document as the import reads it: its picture, ICC profile and EXIF
/// data, and what reading it did and left out.
#[derive(Debug)]
pub(crate) struct Document {
    pub(crate) picture: Picture,
    /// The ICC profile, image resource 1039.
    pub(crate) icc: Option<Vec<u8>>,
    /// The EXIF data, image resource 1058.
    pub(crate) exif: Option<Vec<u8>>,
    /// What the picture leaves out of the document, for the warnings.
    pub(crate) warnings: Vec<String>,
    /// What was done to read it, for the verbose stream.
    pub(crate) said: Vec<String>,
}

/// The bytes of a document read from a place in them; a read past their
/// end is refused.
pub(crate) struct Reader<'a> {
    pub(crate) bytes: &'a [u8],
    pub(crate) at: usize,
}

impl<'a> Reader<'a> {
    pub(crate) fn new(bytes: &'a [u8]) -> Reader<'a> {
        Reader { bytes, at: 0 }
    }

    pub(crate) fn take(&mut self, n: usize) -> Result<&'a [u8], String> {
        let end = self.at.checked_add(n).filter(|&end| end <= self.bytes.len()).ok_or("the document is cut short")?;
        let taken = &self.bytes[self.at..end];
        self.at = end;
        Ok(taken)
    }

    pub(crate) fn four(&mut self) -> Result<[u8; 4], String> {
        let taken = self.take(4)?;
        Ok([taken[0], taken[1], taken[2], taken[3]])
    }

    pub(crate) fn number(&mut self, n: usize) -> Result<u64, String> {
        Ok(self.take(n)?.iter().fold(0u64, |v, &b| (v << 8) | u64::from(b)))
    }

    pub(crate) fn length(&mut self, n: usize) -> Result<usize, String> {
        let value = self.number(n)?;
        usize::try_from(value).map_err(|e| format!("a length, {value}, does not fit in memory: {e}"))
    }

    pub(crate) fn signed16(&mut self) -> Result<i16, String> {
        let taken = self.take(2)?;
        Ok(i16::from_be_bytes([taken[0], taken[1]]))
    }

    pub(crate) fn signed32(&mut self) -> Result<i32, String> {
        Ok(i32::from_be_bytes(self.four()?))
    }

    pub(crate) fn left(&self) -> usize {
        self.bytes.len() - self.at
    }
}

/// A document's header.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Header {
    /// Whether the document is a .psb file.
    pub(crate) large: bool,
    pub(crate) channels: usize,
    pub(crate) width: usize,
    pub(crate) height: usize,
    pub(crate) depth: usize,
    pub(crate) mode: Mode,
}

impl Header {
    /// The bytes a row of one channel's samples `width` wide takes, a 1-bit
    /// row padded to whole bytes.
    pub(crate) fn row(&self, width: usize) -> Result<usize, String> {
        width.checked_mul(self.depth).map(|bits| bits.div_ceil(8)).ok_or_else(|| "a row does not fit in memory".to_string())
    }
}

fn header(r: &mut Reader) -> Result<Header, String> {
    if r.take(4)? != b"8BPS" {
        return Err("it does not start as a Photoshop document does".to_string());
    }
    let large = match r.number(2)? {
        1 => false,
        2 => true,
        other => return Err(format!("its version is {other}, where a .psd file is 1 and a .psb file 2")),
    };
    r.take(6)?;
    let channels = r.length(2)?;
    let height = r.length(4)?;
    let width = r.length(4)?;
    let depth = r.length(2)?;
    let mode = match r.number(2)? {
        0 => Mode::Bitmap,
        1 => Mode::Gray,
        2 => Mode::Indexed,
        3 => Mode::Rgb,
        4 => Mode::Cmyk,
        7 => return Err("its color mode is multichannel, whose channels are inks with no color of their own".to_string()),
        8 => Mode::Duotone,
        9 => Mode::Lab,
        other => return Err(format!("its color mode is {other}, which the specification does not define")),
    };
    let read = match mode {
        Mode::Bitmap => depth == 1,
        Mode::Indexed => depth == 8,
        Mode::Gray | Mode::Rgb => matches!(depth, 8 | 16 | 32),
        Mode::Cmyk | Mode::Lab | Mode::Duotone => matches!(depth, 8 | 16),
    };
    if !read {
        return Err(format!("its samples are {depth}-bit {}, an encoding the module does not know", mode.name()));
    }
    if channels < mode.colors() {
        return Err(format!("it has {channels} channels, fewer than {} color takes", mode.name()));
    }
    if width == 0 || height == 0 {
        return Err(format!("its size is {width}x{height}"));
    }
    Ok(Header { large, channels, width, height, depth, mode })
}

/// What a document's image resources say that the import reads.
#[derive(Debug, Default)]
pub(crate) struct Resources {
    pub(crate) icc: Option<Vec<u8>>,
    pub(crate) exif: Option<Vec<u8>>,
    /// The alpha identifiers, resource 1053, one for each of the last
    /// channels.
    pub(crate) alpha_ids: Option<Vec<u32>>,
    /// The alpha channels' names, from resource 1045, or 1006 without it,
    /// the first extra channel's first.
    pub(crate) names: Option<Vec<String>>,
    /// The display info's kind for each alpha channel, resource 1077: 0 an
    /// alpha channel, 1 an inverted one, 2 a spot ink.
    pub(crate) kinds: Option<Vec<u8>>,
    /// The version info's hasRealMergedData, resource 1057.
    pub(crate) real_merge: Option<bool>,
    /// The index of an indexed document's transparent color, resource 1047.
    pub(crate) transparent: Option<usize>,
}

/// A spot ink's kind in the display info.
const SPOT: u8 = 2;

/// The image resource blocks of `section`: each a signature, an id, a name
/// of a length byte and that many bytes padded to an even size, and a
/// length and that many bytes padded to an even size.
fn resources(section: &[u8]) -> Result<Resources, String> {
    let mut r = Reader::new(section);
    let mut found = Resources::default();
    let mut pascal_names = None;
    while r.left() > 0 {
        r.take(4)?;
        let id = r.number(2)?;
        let name = r.length(1)?;
        r.take(name + (name + 1) % 2)?;
        let size = r.length(4)?;
        let data = r.take(size)?;
        r.take(size % 2)?;
        let unreadable = |e: String| format!("its image resource {id} cannot be read: {e}");
        match id {
            1006 => pascal_names = Some(pascal_strings(data).map_err(unreadable)?),
            1039 => found.icc = Some(data.to_vec()),
            1045 => found.names = Some(unicode_strings(data).map_err(unreadable)?),
            1047 => found.transparent = Some(Reader::new(data).length(2).map_err(unreadable)?),
            1053 => found.alpha_ids = Some(data.as_chunks::<4>().0.iter().map(|c| u32::from_be_bytes(*c)).collect()),
            1057 => found.real_merge = Some(*data.get(4).ok_or_else(|| unreadable("it is cut short".to_string()))? != 0),
            1058 => found.exif = Some(data.to_vec()),
            1077 => found.kinds = Some(display_kinds(data).map_err(unreadable)?),
            _ => {}
        }
    }
    if found.names.is_none() {
        found.names = pascal_names;
    }
    Ok(found)
}

/// Resource 1006's names, a series of Pascal strings, with any byte past
/// ASCII shown as the replacement character.
fn pascal_strings(data: &[u8]) -> Result<Vec<String>, String> {
    let mut r = Reader::new(data);
    let mut names = Vec::new();
    while r.left() > 0 {
        let n = r.length(1)?;
        names.push(r.take(n)?.iter().map(|&b| if b.is_ascii() { char::from(b) } else { char::REPLACEMENT_CHARACTER }).collect());
    }
    Ok(names)
}

/// Resource 1045's names, a series of Unicode strings: a count of UTF-16
/// code units and that many, big-endian, a trailing NUL dropped.
fn unicode_strings(data: &[u8]) -> Result<Vec<String>, String> {
    let mut r = Reader::new(data);
    let mut names = Vec::new();
    while r.left() >= 4 {
        let n = r.length(4)?;
        let units: Vec<u16> = r.take(n.checked_mul(2).ok_or("a name does not fit in memory")?)?.as_chunks::<2>().0.iter().map(|c| u16::from_be_bytes(*c)).collect();
        names.push(String::from_utf16_lossy(&units).trim_end_matches('\0').to_string());
    }
    Ok(names)
}

/// Resource 1077's kind for each alpha channel: a 4-byte version, then for
/// each a 2-byte color space, four 2-byte color components, a 2-byte
/// opacity and the kind's byte.
fn display_kinds(data: &[u8]) -> Result<Vec<u8>, String> {
    let entries = data.get(4..).ok_or("it is cut short")?;
    Ok(entries.as_chunks::<13>().0.iter().map(|entry| entry[12]).collect())
}

/// One stored channel's plane: rows of samples as stored, 16- and 32-bit
/// ones big-endian, 1-bit rows padded to whole bytes.
pub(crate) type Plane = Vec<u8>;

/// `rows` rows of `row` bytes each, compressed as `compression` says, from
/// `data`; `counts` are the PackBits rows' byte counts. The rows of a zip
/// stream with prediction hold differences along each row, of samples at 8
/// and 16 bits and, at 32 bits, of the bytes of a row whose samples' first
/// bytes come first, then their second, and so on.
pub(crate) fn decompressed(compression: u64, data: &[u8], counts: &[usize], row: usize, rows: usize, depth: usize) -> Result<Vec<u8>, String> {
    let size = row.checked_mul(rows).ok_or("the samples do not fit in memory")?;
    match compression {
        0 => data.get(..size).map(<[u8]>::to_vec).ok_or_else(|| "its uncompressed samples are cut short".to_string()),
        1 => {
            let mut out = Vec::with_capacity(size);
            let mut at = 0usize;
            for &count in counts {
                let packed = at.checked_add(count).and_then(|end| data.get(at..end)).ok_or("its PackBits rows are cut short")?;
                unpack(packed, row, &mut out)?;
                at += count;
            }
            Ok(out)
        }
        2 | 3 => {
            let mut out = Vec::with_capacity(size);
            let limit = u64::try_from(size).map_err(|e| e.to_string())? + 1;
            flate2::read::ZlibDecoder::new(data).take(limit).read_to_end(&mut out).map_err(|e| format!("its zip data cannot be inflated: {e}"))?;
            if out.len() != size {
                return Err(format!("its zip data inflates to {} bytes, where its samples take {size}", out.len()));
            }
            if compression == 3 {
                predicted(&mut out, row, depth)?;
            }
            Ok(out)
        }
        other => Err(format!("its compression is {other}, where the specification defines 0 to 3")),
    }
}

/// Zip data with prediction made samples again, as psd-tools'
/// decode_prediction makes them: a running sum along each row, wrapping, of
/// bytes at 8 bits, of big-endian 16-bit samples at 16, and at 32 of the
/// row's bytes, whose samples' first bytes come first, then their second,
/// and so on, put back in order.
fn predicted(data: &mut [u8], row: usize, depth: usize) -> Result<(), String> {
    for line in data.chunks_exact_mut(row) {
        match depth {
            8 => {
                for i in 1..line.len() {
                    line[i] = line[i].wrapping_add(line[i - 1]);
                }
            }
            16 => {
                let mut sum = 0u16;
                for pair in line.as_chunks_mut::<2>().0 {
                    sum = sum.wrapping_add(u16::from_be_bytes(*pair));
                    *pair = sum.to_be_bytes();
                }
            }
            32 => {
                for i in 1..line.len() {
                    line[i] = line[i].wrapping_add(line[i - 1]);
                }
                let width = line.len() / 4;
                let planes = line.to_vec();
                for (x, sample) in line.as_chunks_mut::<4>().0.iter_mut().enumerate() {
                    for (b, byte) in sample.iter_mut().enumerate() {
                        *byte = planes[b * width + x];
                    }
                }
            }
            other => return Err(format!("its zip data with prediction holds {other}-bit samples, where prediction is defined for 8, 16 and 32")),
        }
    }
    Ok(())
}

/// A PackBits row unpacked onto `out`: a byte n below 128 copies the n + 1
/// bytes after it, one above repeats the next byte 257 - n times, and 128
/// does nothing. The row must come to `row` bytes.
fn unpack(packed: &[u8], row: usize, out: &mut Vec<u8>) -> Result<(), String> {
    let start = out.len();
    let mut at = 0;
    while at < packed.len() {
        let n = usize::from(packed[at]);
        at += 1;
        if n < 128 {
            let run = packed.get(at..at + n + 1).ok_or("a PackBits row is cut short")?;
            out.extend_from_slice(run);
            at += n + 1;
        } else if n > 128 {
            let value = *packed.get(at).ok_or("a PackBits row is cut short")?;
            out.extend(std::iter::repeat_n(value, 257 - n));
            at += 1;
        }
    }
    if out.len() - start != row {
        return Err(format!("a PackBits row unpacks to {} bytes, where a row is {row}", out.len() - start));
    }
    Ok(())
}

/// The composite image's planes for the channels `wanted`, in that order,
/// from `section`, the image data section.
pub(crate) fn composite_planes(section: &[u8], header: &Header, wanted: &[usize]) -> Result<Vec<Plane>, String> {
    let mut r = Reader::new(section);
    let compression = r.number(2)?;
    let row = header.row(header.width)?;
    let plane = row.checked_mul(header.height).ok_or("a plane does not fit in memory")?;
    let mut planes = Vec::with_capacity(wanted.len());
    match compression {
        0 => {
            let data = r.take(r.left())?;
            for &c in wanted {
                let at = c.checked_mul(plane).ok_or("a plane does not fit in memory")?;
                planes.push(data.get(at..at + plane).ok_or("its uncompressed planes are cut short")?.to_vec());
            }
        }
        1 => {
            let count = if header.large { 4 } else { 2 };
            let mut counts = Vec::with_capacity(header.channels * header.height);
            for _ in 0..header.channels * header.height {
                counts.push(r.length(count)?);
            }
            let data = r.take(r.left())?;
            let mut starts = Vec::with_capacity(header.channels);
            let mut at = 0usize;
            for channel in counts.chunks(header.height) {
                starts.push(at);
                at = at.checked_add(channel.iter().sum::<usize>()).ok_or("its PackBits rows do not fit in memory")?;
            }
            for &c in wanted {
                let rows = data.get(starts[c]..).ok_or("its PackBits rows are cut short")?;
                planes.push(decompressed(1, rows, &counts[c * header.height..(c + 1) * header.height], row, header.height, header.depth)?);
            }
        }
        2 | 3 => {
            let data = r.take(r.left())?;
            let all = decompressed(compression, data, &[], row, header.height * header.channels, header.depth)?;
            for &c in wanted {
                planes.push(all[c * plane..(c + 1) * plane].to_vec());
            }
        }
        other => return Err(format!("its composite image's compression is {other}, where the specification defines 0 to 3")),
    }
    Ok(planes)
}

/// Planes `width` by `height` interleaved into samples: a pixel's sample
/// from each plane in turn, at `depth` bits, 1-bit ones read as gray, a set
/// bit black.
pub(crate) fn interleaved(planes: &[Plane], depth: usize, width: usize, height: usize) -> Samples {
    let pixels = width * height;
    let n = planes.len();
    match depth {
        1 => {
            let row = width.div_ceil(8);
            let plane = &planes[0];
            Samples::Eight((0..pixels).map(|p| if plane[(p / width) * row + (p % width) / 8] & (0x80 >> (p % width % 8)) != 0 { 0 } else { 255 }).collect())
        }
        8 => Samples::Eight((0..pixels * n).map(|i| planes[i % n][i / n]).collect()),
        16 => Samples::Sixteen(
            (0..pixels * n)
                .map(|i| {
                    let (plane, p) = (&planes[i % n], 2 * (i / n));
                    u16::from_be_bytes([plane[p], plane[p + 1]])
                })
                .collect(),
        ),
        _ => Samples::Float(
            (0..pixels * n)
                .map(|i| {
                    let (plane, p) = (&planes[i % n], 4 * (i / n));
                    f32::from_be_bytes([plane[p], plane[p + 1], plane[p + 2], plane[p + 3]])
                })
                .collect(),
        ),
    }
}

/// Which channel past the color ones a document's composite takes as its
/// transparency, before any check against the layers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Transparency {
    /// The file marks this channel as the transparency, as Photoshop marks
    /// it.
    Marked(usize),
    /// The file says nothing of this channel, the only one past the color
    /// ones that it says nothing of, in a document without layers: it is
    /// taken as the transparency.
    Unmarked(usize),
    /// The same in a document with layers: it is the transparency if it
    /// matches the merged layers' alpha.
    ToCheck(usize),
    /// No channel is.
    None,
}

/// What the file says about the channels past the color ones, as
/// `Transparency`, and why a channel that might be transparency is not
/// taken as it, for the warnings. `layered` is whether the document has
/// layers.
fn transparency(header: &Header, resources: &Resources, layer_count: i16, merged_key: bool, layered: bool) -> (Transparency, Option<String>) {
    let colors = header.mode.colors();
    if header.channels == colors || header.mode == Mode::Bitmap || header.mode == Mode::Indexed {
        return (Transparency::None, None);
    }
    if layer_count < 0 {
        return (Transparency::Marked(colors), None);
    }
    if let Some(ids) = &resources.alpha_ids
        && let Some(at) = ids.iter().position(|&id| id == 0)
        && let Some(index) = (header.channels + at).checked_sub(ids.len()).filter(|&i| i >= colors && i < header.channels)
    {
        return (Transparency::Marked(index), None);
    }
    let described = |index: usize| {
        let n = index - colors;
        let id = resources.alpha_ids.as_ref().is_some_and(|ids| header.channels.checked_sub(ids.len()).is_some_and(|first| index >= first));
        let name = resources.names.as_ref().is_some_and(|names| n < names.len());
        let kind = resources.kinds.as_ref().is_some_and(|kinds| n < kinds.len());
        id || name || kind
    };
    let undescribed: Vec<usize> = (colors..header.channels).filter(|&i| !described(i)).collect();
    if merged_key {
        return match undescribed.first() {
            Some(&index) => (Transparency::Marked(index), None),
            None => (Transparency::None, Some("the document says it saved its transparency, but every channel past its color is a saved selection or a spot ink, so it loaded opaque".to_string())),
        };
    }
    match (undescribed.as_slice(), layered) {
        ([], _) => (Transparency::None, None),
        ([index], true) => (Transparency::ToCheck(*index), None),
        ([index], false) => (Transparency::Unmarked(*index), None),
        (many, _) => (Transparency::None, Some(format!("the document has {} channels past its color that it says nothing of, so none was taken as its transparency and it loaded opaque", many.len()))),
    }
}

/// The names of the spot inks among the channels past the color ones.
fn spot_inks(header: &Header, resources: &Resources) -> Vec<String> {
    let Some(kinds) = &resources.kinds else { return Vec::new() };
    let extra = header.channels - header.mode.colors();
    kinds
        .iter()
        .take(extra)
        .enumerate()
        .filter(|&(_, &kind)| kind == SPOT)
        .map(|(n, _)| match resources.names.as_ref().and_then(|names| names.get(n)) {
            Some(name) => name.clone(),
            None => format!("channel {}", header.mode.colors() + n + 1),
        })
        .collect()
}

/// A transparent composite's color with the white it is stored blended
/// with taken out, as psd-tools' _remove_background and GIMP's psd-load
/// do: each color sample c of a pixel whose alpha a is above 0 becomes
/// (c + a - 1) / a, on samples from 0 to 1, rounded at 8 and 16 bits.
fn unblended(samples: &mut Samples, colors: usize) {
    let stride = colors + 1;
    match samples {
        Samples::Eight(v) => unblend_integers(v, stride, 255.0, |x| x as u8),
        Samples::Sixteen(v) => unblend_integers(v, stride, 65535.0, |x| x as u16),
        Samples::Float(v) => {
            for pixel in v.chunks_exact_mut(stride) {
                let a = pixel[colors];
                if a > 0.0 {
                    for c in &mut pixel[..colors] {
                        *c = (*c + a - 1.0) / a;
                    }
                }
            }
        }
    }
}

/// Whether a transparent composite's fully transparent pixels show the
/// white a white blend leaves under them: there is at least one, and every
/// one is white.
fn white_under_clear(samples: &Samples, colors: usize) -> bool {
    fn white<T: Copy + PartialEq>(v: &[T], stride: usize, clear: T, full: T) -> bool {
        let mut any = false;
        for pixel in v.chunks_exact(stride) {
            if pixel[stride - 1] == clear {
                if pixel[..stride - 1].iter().any(|&c| c != full) {
                    return false;
                }
                any = true;
            }
        }
        any
    }
    let stride = colors + 1;
    match samples {
        Samples::Eight(v) => white(v, stride, 0, 255),
        Samples::Sixteen(v) => white(v, stride, 0, 65535),
        Samples::Float(v) => white(v, stride, 0.0, 1.0),
    }
}

/// `unblended` on integer samples whose largest value is `max`; `to` turns
/// a value already rounded and within 0 to `max` back into a sample.
fn unblend_integers<T: Copy + Into<f64>>(v: &mut [T], stride: usize, max: f64, to: fn(f64) -> T) {
    for pixel in v.chunks_exact_mut(stride) {
        let a: f64 = pixel[stride - 1].into();
        if a > 0.0 {
            for c in &mut pixel[..stride - 1] {
                *c = to(((Into::<f64>::into(*c) + a - max) * max / a).round().clamp(0.0, max));
            }
        }
    }
}

/// CMYK samples the way round the module converts them: Photoshop stores
/// an ink's largest value as none and 0 as full. Alpha, past the four
/// inks, keeps its value.
fn inverted(samples: &mut Samples, stride: usize) {
    match samples {
        Samples::Eight(v) => {
            for (i, s) in v.iter_mut().enumerate() {
                if i % stride < 4 {
                    *s = 255 - *s;
                }
            }
        }
        Samples::Sixteen(v) => {
            for (i, s) in v.iter_mut().enumerate() {
                if i % stride < 4 {
                    *s = 65535 - *s;
                }
            }
        }
        Samples::Float(_) => {}
    }
}

/// An indexed document's pixels through its color table, `table` its 768
/// bytes of color mode data, all reds, then all greens, then all blues,
/// with alpha clear at the transparent color where `transparent` names one.
fn indexed(samples: &Samples, table: &[u8], transparent: Option<usize>) -> Result<Samples, String> {
    let Samples::Eight(indices) = samples else { return Err("its indexed samples are not 8-bit".to_string()) };
    if table.len() != 768 {
        return Err(format!("its color table is {} bytes, where an indexed document's is 768", table.len()));
    }
    let color = |i: u8| [table[usize::from(i)], table[256 + usize::from(i)], table[512 + usize::from(i)]];
    Ok(Samples::Eight(match transparent {
        Some(clear) => indices
            .iter()
            .flat_map(|&i| {
                let [r, g, b] = color(i);
                [r, g, b, if usize::from(i) == clear { 0 } else { 255 }]
            })
            .collect(),
        None => indices.iter().flat_map(|&i| color(i)).collect(),
    }))
}

/// A document's layers and the global tagged blocks, as the merge reads
/// them, and the layer count's sign.
pub(crate) struct LayerSection<'a> {
    /// The layer count as stored; negative where the first alpha channel is
    /// the composite's transparency.
    pub(crate) count: i16,
    /// The layer info, its count first, then its records and channel data,
    /// from the section or from a 16- or 32-bit document's Lr16 or Lr32
    /// block.
    pub(crate) info: &'a [u8],
    /// Whether the global tagged blocks hold Mtrn, Mt16 or Mt32, which
    /// say the composite's transparency was saved.
    pub(crate) merged_key: bool,
    /// The global tagged blocks, among them the patterns, Patt, Pat2 and
    /// Pat3, a pattern fill tiles.
    pub(crate) blocks: Vec<Block<'a>>,
}

/// A tagged block: its key and its data.
pub(crate) type Block<'a> = ([u8; 4], &'a [u8]);

/// The tagged blocks of `data`: each a signature, 8BIM or 8B64, a key, a
/// length, 8 bytes in a .psb file for the keys the specification gives
/// one, and that many bytes, padded to a multiple of `padding`.
pub(crate) fn tagged_blocks(data: &[u8], large: bool, padding: usize) -> Result<Vec<Block<'_>>, String> {
    const LONG: [&[u8; 4]; 14] = [b"LMsk", b"Lr16", b"Lr32", b"Layr", b"Mt16", b"Mt32", b"Mtrn", b"Alph", b"FMsk", b"lnk2", b"FEid", b"FXid", b"PxSD", b"lnkE"];
    let mut r = Reader::new(data);
    let mut blocks = Vec::new();
    while r.left() >= 12 {
        let signature = r.four()?;
        if &signature != b"8BIM" && &signature != b"8B64" {
            break;
        }
        let key = r.four()?;
        let size = r.length(if large && LONG.contains(&&key) { 8 } else { 4 })?;
        let body = r.take(size)?;
        let pad = (padding - size % padding) % padding;
        r.take(pad.min(r.left()))?;
        blocks.push((key, body));
    }
    Ok(blocks)
}

/// The layer and mask information section's layers and global blocks.
fn layer_section(section: &[u8], large: bool) -> Result<LayerSection<'_>, String> {
    if section.is_empty() {
        return Ok(LayerSection { count: 0, info: &[], merged_key: false, blocks: Vec::new() });
    }
    let mut r = Reader::new(section);
    let info_length = r.length(if large { 8 } else { 4 })?;
    let info = r.take(info_length)?;
    if r.left() >= 4 && r.bytes[r.at..r.at + 4] != *b"8BIM" && r.bytes[r.at..r.at + 4] != *b"8B64" {
        let mask_length = r.length(4)?;
        r.take(mask_length)?;
    }
    let blocks = tagged_blocks(r.take(r.left())?, large, 4)?;
    let merged_key = blocks.iter().any(|(key, _)| matches!(key, b"Mtrn" | b"Mt16" | b"Mt32"));
    let info = if info.len() >= 2 {
        info
    } else {
        match blocks.iter().find(|(key, _)| key == b"Lr16" || key == b"Lr32") {
            Some(&(_, body)) => body,
            None => &[],
        }
    };
    let count = if info.len() >= 2 { Reader::new(info).signed16()? } else { 0 };
    Ok(LayerSection { count, info, merged_key, blocks })
}

/// A document's sections as the reader takes them apart: its header, its
/// color mode data, what its image resources say, its layer section, and
/// its image data, the composite image.
pub(crate) struct Sections<'a> {
    pub(crate) header: Header,
    pub(crate) color_data: &'a [u8],
    pub(crate) resources: Resources,
    pub(crate) layers: LayerSection<'a>,
    pub(crate) image_data: &'a [u8],
}

/// The sections of the document `bytes`, or why they cannot be read.
pub(crate) fn sections(bytes: &[u8]) -> Result<Sections<'_>, String> {
    let mut r = Reader::new(bytes);
    let header = header(&mut r)?;
    let color_data_length = r.length(4)?;
    let color_data = r.take(color_data_length)?;
    let resources_length = r.length(4)?;
    let resources = resources(r.take(resources_length)?)?;
    let section_length = r.length(if header.large { 8 } else { 4 })?;
    let layers = layer_section(r.take(section_length)?, header.large)?;
    let image_data = r.take(r.left())?;
    Ok(Sections { header, color_data, resources, layers, image_data })
}

/// The document `bytes` read as the import loads it, a fill color of
/// another mode than the document's converted into its samples and a
/// document's light given by `handling`, the module's color handling,
/// where the layers merge, or why it cannot be. The layers merge only with
/// the psd-merge feature: without it a document whose composite is not the
/// real merge is refused, and an extra channel only the merged layers could
/// show to be the transparency is left out with a warning.
pub(crate) fn read(bytes: &[u8], handling: crate::psd_fill::Colors) -> Result<Document, String> {
    let Sections { header, color_data, resources, layers: section, image_data } = sections(bytes)?;
    let mut warnings = Vec::new();
    let mut said = Vec::new();
    let layered = section.count != 0;
    if resources.real_merge == Some(false) {
        if !layered {
            return Err("its version info says its composite image is not the real merge, and it has no layers to merge".to_string());
        }
        if !cfg!(feature = "psd-merge") {
            return Err("its version info says its composite image is not the real merge, and the module does not merge a document's layers".to_string());
        }
        let merged = crate::psd_merge::merged(&header, &section, color_data, &resources, handling)?;
        warnings.extend(merged.warnings);
        said.push("the document's layers were merged, as its version info says its composite image is not the real merge".to_string());
        return Ok(Document { picture: merged.picture, icc: resources.icc, exif: resources.exif, warnings, said });
    }
    let (transparency, why) = transparency(&header, &resources, section.count, section.merged_key, layered);
    warnings.extend(why);
    let colors = header.mode.colors();
    let mut wanted: Vec<usize> = (0..colors).collect();
    let marked = matches!(transparency, Transparency::Marked(_));
    let alpha_channel = match transparency {
        Transparency::Marked(index) | Transparency::Unmarked(index) => Some(index),
        Transparency::ToCheck(index) if !cfg!(feature = "psd-merge") => {
            warnings.push(format!(
                "channel {} was not taken as the document's transparency, as only its merged layers could show that it is, and the module does not merge a document's layers",
                index + 1
            ));
            None
        }
        Transparency::ToCheck(index) => {
            let plane = composite_planes(image_data, &header, &[index])?.remove(0);
            let (alpha, left_out) = crate::psd_merge::merged_alpha(&header, &section, &resources, handling)?;
            if crate::psd_merge::matches(&plane, &alpha, &header) {
                said.push(format!("channel {} was taken as the document's transparency, as it matches its merged layers' alpha", index + 1));
                Some(index)
            } else if left_out.is_empty() {
                said.push(format!("channel {} was not taken as the document's transparency, as it does not match its merged layers' alpha", index + 1));
                None
            } else {
                warnings.push(format!(
                    "channel {} was not taken as the document's transparency, as it does not match its merged layers' alpha, which leaves out what the merge does not draw: {}",
                    index + 1,
                    left_out.join("; ")
                ));
                None
            }
        }
        Transparency::None => None,
    };
    if let Some(index) = alpha_channel {
        wanted.push(index);
    }
    let spots = spot_inks(&header, &resources);
    if !spots.is_empty() {
        warnings.push(format!("the document's spot inks {} were left out, as its composite image does not hold them", spots.join(", ")));
    }
    let planes = composite_planes(image_data, &header, &wanted)?;
    let mut samples = interleaved(&planes, header.depth, header.width, header.height);
    let mut alpha = alpha_channel.is_some();
    match header.mode {
        Mode::Cmyk => inverted(&mut samples, wanted.len()),
        Mode::Rgb | Mode::Gray if alpha => {
            if marked || white_under_clear(&samples, colors) {
                unblended(&mut samples, colors);
            } else {
                said.push("the colors loaded as stored, not as blended with white, as the document's fully transparent pixels are not all white".to_string());
            }
        }
        Mode::Indexed => {
            samples = indexed(&samples, color_data, resources.transparent)?;
            alpha = resources.transparent.is_some();
        }
        Mode::Duotone => warnings.push("the document is a duotone, which loaded as its gray, without its inks".to_string()),
        _ => {}
    }
    let picture = Picture { width: header.width as u32, height: header.height as u32, color: header.mode.color(), alpha, samples };
    Ok(Document { picture, icc: resources.icc, exif: resources.exif, warnings, said })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write as _;

    /// A document to write: a .psb file where `large`, its mode number,
    /// depth and size, each stored channel's plane as stored, its color mode
    /// data, image resources and layer and mask section, and the
    /// compression of its composite.
    struct Spec {
        large: bool,
        mode: u16,
        depth: u16,
        width: u32,
        height: u32,
        planes: Vec<Vec<u8>>,
        color_data: Vec<u8>,
        resources: Vec<u8>,
        layers: Vec<u8>,
        compression: u16,
    }

    impl Spec {
        fn new(mode: u16, depth: u16, width: u32, height: u32, planes: Vec<Vec<u8>>) -> Spec {
            Spec { large: false, mode, depth, width, height, planes, color_data: Vec::new(), resources: Vec::new(), layers: Vec::new(), compression: 0 }
        }

        /// The document's bytes as the specification lays them out, each
        /// PackBits row a single literal run.
        fn bytes(&self) -> Vec<u8> {
            let mut out = b"8BPS".to_vec();
            out.extend_from_slice(&(if self.large { 2u16 } else { 1 }).to_be_bytes());
            out.extend_from_slice(&[0; 6]);
            out.extend_from_slice(&(self.planes.len() as u16).to_be_bytes());
            out.extend_from_slice(&self.height.to_be_bytes());
            out.extend_from_slice(&self.width.to_be_bytes());
            out.extend_from_slice(&self.depth.to_be_bytes());
            out.extend_from_slice(&self.mode.to_be_bytes());
            for section in [&self.color_data, &self.resources] {
                out.extend_from_slice(&(section.len() as u32).to_be_bytes());
                out.extend_from_slice(section);
            }
            if self.large {
                out.extend_from_slice(&(self.layers.len() as u64).to_be_bytes());
            } else {
                out.extend_from_slice(&(self.layers.len() as u32).to_be_bytes());
            }
            out.extend_from_slice(&self.layers);
            out.extend_from_slice(&self.compression.to_be_bytes());
            let row = (self.width as usize * usize::from(self.depth)).div_ceil(8);
            match self.compression {
                0 => self.planes.iter().for_each(|plane| out.extend_from_slice(plane)),
                1 => {
                    for _ in self.planes.iter().flat_map(|plane| plane.chunks(row)) {
                        if self.large {
                            out.extend_from_slice(&(row as u32 + 1).to_be_bytes());
                        } else {
                            out.extend_from_slice(&(row as u16 + 1).to_be_bytes());
                        }
                    }
                    for line in self.planes.iter().flat_map(|plane| plane.chunks(row)) {
                        out.push((row - 1) as u8);
                        out.extend_from_slice(line);
                    }
                }
                _ => {
                    let mut all = self.planes.concat();
                    if self.compression == 3 {
                        with_prediction(&mut all, row, self.depth);
                    }
                    let mut encoder = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
                    encoder.write_all(&all).expect("deflate writes to memory");
                    out.extend_from_slice(&encoder.finish().expect("deflate writes to memory"));
                }
            }
            out
        }
    }

    /// Rows of samples as zip with prediction stores them, as psd-tools'
    /// encode_prediction makes them.
    fn with_prediction(data: &mut [u8], row: usize, depth: u16) {
        for line in data.chunks_exact_mut(row) {
            match depth {
                8 => {
                    for i in (1..line.len()).rev() {
                        line[i] = line[i].wrapping_sub(line[i - 1]);
                    }
                }
                16 => {
                    let values: Vec<u16> = line.as_chunks::<2>().0.iter().map(|c| u16::from_be_bytes(*c)).collect();
                    for (i, pair) in line.as_chunks_mut::<2>().0.iter_mut().enumerate() {
                        *pair = if i == 0 { values[0] } else { values[i].wrapping_sub(values[i - 1]) }.to_be_bytes();
                    }
                }
                _ => {
                    let width = line.len() / 4;
                    let planes: Vec<u8> = (0..4).flat_map(|b| (0..width).map(move |x| x * 4 + b)).map(|at| line[at]).collect();
                    for (i, byte) in line.iter_mut().enumerate() {
                        *byte = if i == 0 { planes[0] } else { planes[i].wrapping_sub(planes[i - 1]) };
                    }
                }
            }
        }
    }

    /// An image resource block of `id` holding `data`.
    fn resource(id: u16, data: &[u8]) -> Vec<u8> {
        let mut out = b"8BIM".to_vec();
        out.extend_from_slice(&id.to_be_bytes());
        out.extend_from_slice(&[0, 0]);
        out.extend_from_slice(&(data.len() as u32).to_be_bytes());
        out.extend_from_slice(data);
        if data.len() % 2 == 1 {
            out.push(0);
        }
        out
    }

    /// Resource 1045's data for `names`.
    fn unicode_names(names: &[&str]) -> Vec<u8> {
        let mut out = Vec::new();
        for name in names {
            let units: Vec<u16> = name.encode_utf16().chain([0]).collect();
            out.extend_from_slice(&(units.len() as u32).to_be_bytes());
            units.iter().for_each(|u| out.extend_from_slice(&u.to_be_bytes()));
        }
        out
    }

    /// Resource 1077's data for alpha channels of `kinds`.
    fn display_info(kinds: &[u8]) -> Vec<u8> {
        let mut out = 1u32.to_be_bytes().to_vec();
        for &kind in kinds {
            out.extend_from_slice(&[0; 12]);
            out.push(kind);
        }
        out
    }

    /// A layer and mask section of one layer over the whole canvas, its
    /// channels `planes`, the color ones then its transparency, uncompressed,
    /// the layer count stored as `count`.
    fn one_layer(count: i16, width: u32, height: u32, planes: &[Vec<u8>]) -> Vec<u8> {
        let mut info = count.to_be_bytes().to_vec();
        for edge in [0, 0, height as i32, width as i32] {
            info.extend_from_slice(&edge.to_be_bytes());
        }
        info.extend_from_slice(&(planes.len() as u16).to_be_bytes());
        let ids = (0..planes.len() as i16 - 1).chain([-1]);
        for (id, plane) in ids.zip(planes) {
            info.extend_from_slice(&id.to_be_bytes());
            info.extend_from_slice(&(2 + plane.len() as u32).to_be_bytes());
        }
        info.extend_from_slice(b"8BIMnorm");
        info.extend_from_slice(&[255, 0, 0, 0]);
        let extra = [0, 0, 0, 0, 0, 0, 0, 0, 1, b'L', 0, 0];
        info.extend_from_slice(&(extra.len() as u32).to_be_bytes());
        info.extend_from_slice(&extra);
        for plane in planes {
            info.extend_from_slice(&[0, 0]);
            info.extend_from_slice(plane);
        }
        let mut section = (info.len() as u32).to_be_bytes().to_vec();
        section.extend_from_slice(&info);
        section
    }

    fn read_spec(spec: &Spec) -> Document {
        match read(&spec.bytes(), crate::image_decode::COLORS) {
            Ok(document) => document,
            Err(why) => panic!("the document does not read: {why}"),
        }
    }

    fn eight(document: &Document) -> &[u8] {
        match &document.picture.samples {
            Samples::Eight(v) => v,
            other => panic!("8-bit samples, not {other:?}"),
        }
    }

    /// An RGB document 2 pixels wide with a fourth channel `extra`.
    fn rgb_with_extra(rgb: [[u8; 2]; 3], extra: [u8; 2]) -> Spec {
        Spec::new(3, 8, 2, 1, vec![rgb[0].to_vec(), rgb[1].to_vec(), rgb[2].to_vec(), extra.to_vec()])
    }

    #[test]
    fn reads_each_color_mode_as_the_import_takes_it() {
        let gray = read_spec(&Spec::new(1, 8, 2, 1, vec![vec![10, 200]]));
        assert_eq!((gray.picture.color, gray.picture.alpha, eight(&gray)), (Color::Gray, false, &[10u8, 200][..]));
        let rgb = read_spec(&Spec::new(3, 16, 2, 1, vec![vec![1, 2, 0xF0, 0xE0], vec![3, 4, 0xD0, 0xC0], vec![5, 6, 0xB0, 0xA0]]));
        assert_eq!(rgb.picture.samples, Samples::Sixteen(vec![0x0102, 0x0304, 0x0506, 0xF0E0, 0xD0C0, 0xB0A0]));
        let cmyk = read_spec(&Spec::new(4, 8, 2, 1, vec![vec![10, 200], vec![20, 150], vec![30, 100], vec![128, 64]]));
        assert_eq!((cmyk.picture.color, eight(&cmyk)), (Color::Cmyk, &[245u8, 235, 225, 127, 55, 105, 155, 191][..]));
        let lab = read_spec(&Spec::new(9, 8, 1, 1, vec![vec![128], vec![120], vec![140]]));
        assert_eq!((lab.picture.color, eight(&lab)), (Color::Lab, &[128u8, 120, 140][..]));
        let duotone = read_spec(&Spec { color_data: vec![1, 2, 3, 4], ..Spec::new(8, 8, 1, 1, vec![vec![77]]) });
        assert_eq!((duotone.picture.color, eight(&duotone)), (Color::Gray, &[77u8][..]));
        assert!(duotone.warnings.iter().any(|w| w.contains("duotone")), "{:?}", duotone.warnings);
        let float = read_spec(&Spec::new(1, 32, 2, 1, vec![[0.25f32.to_be_bytes(), 2.5f32.to_be_bytes()].concat()]));
        assert_eq!(float.picture.samples, Samples::Float(vec![0.25, 2.5]));
    }

    #[test]
    fn reads_a_bitmap_as_gray_and_an_indexed_document_through_its_color_table() {
        let bitmap = read_spec(&Spec::new(0, 1, 10, 1, vec![vec![0b1010_0000, 0b0100_0000]]));
        assert_eq!(eight(&bitmap), &[0, 255, 0, 255, 255, 255, 255, 255, 255, 0]);
        let mut table = vec![0u8; 768];
        table[1] = 10;
        table[256 + 1] = 20;
        table[512 + 1] = 30;
        table[2] = 200;
        table[256 + 2] = 100;
        table[512 + 2] = 50;
        let indexed = read_spec(&Spec { color_data: table.clone(), ..Spec::new(2, 8, 2, 1, vec![vec![1, 2]]) });
        assert_eq!((indexed.picture.color, indexed.picture.alpha, eight(&indexed)), (Color::Rgb, false, &[10u8, 20, 30, 200, 100, 50][..]));
        let clear = read_spec(&Spec { color_data: table, resources: resource(1047, &2u16.to_be_bytes()), ..Spec::new(2, 8, 2, 1, vec![vec![1, 2]]) });
        assert_eq!((clear.picture.alpha, eight(&clear)), (true, &[10u8, 20, 30, 255, 200, 100, 50, 0][..]));
    }

    #[test]
    fn unpacks_packbits_rows() {
        let mut out = Vec::new();
        unpack(&[2, 1, 2, 3, 254, 9, 128], 6, &mut out).expect("unpacked");
        assert_eq!(out, [1, 2, 3, 9, 9, 9]);
        assert!(unpack(&[2, 1], 3, &mut Vec::new()).is_err(), "a run cut short is refused");
        assert!(unpack(&[255, 4], 3, &mut Vec::new()).is_err(), "a row of another length is refused");
    }

    #[test]
    fn reads_every_compression_at_every_depth() {
        for depth in [8u16, 16, 32] {
            let sample = |c: usize, i: usize| -> Vec<u8> {
                match depth {
                    8 => vec![(i * 37 + c * 11) as u8],
                    16 => ((i * 4099 + c * 517) as u16).to_be_bytes().to_vec(),
                    _ => (i as f32 * 0.375 + c as f32 * 1.5 - 0.25).to_be_bytes().to_vec(),
                }
            };
            let planes: Vec<Vec<u8>> = (0..3).map(|c| (0..6).flat_map(|i| sample(c, i)).collect()).collect();
            let raw = read_spec(&Spec::new(3, depth, 3, 2, planes.clone()));
            for (compression, large) in [(1, false), (1, true), (2, false), (3, false)] {
                let other = read_spec(&Spec { compression, large, ..Spec::new(3, depth, 3, 2, planes.clone()) });
                assert_eq!(other.picture.samples, raw.picture.samples, "{depth}-bit, compression {compression}, large {large}");
            }
        }
    }

    #[test]
    fn reads_the_names_identifiers_kinds_and_flags_its_resources_give() {
        let mut data = resource(1006, &[3, b'a', b'b', b'c']);
        data.extend(resource(1045, &unicode_names(&["Ink", "Selection"])));
        data.extend(resource(1053, &[[0, 0, 0, 5], [0, 0, 0, 6]].concat()));
        data.extend(resource(1077, &display_info(&[2, 0])));
        data.extend(resource(1039, b"icc"));
        data.extend(resource(1058, b"exif"));
        data.extend(resource(1057, &[0, 0, 0, 1, 1]));
        let found = resources(&data).expect("the resources read");
        assert_eq!(found.names, Some(vec!["Ink".to_string(), "Selection".to_string()]));
        assert_eq!((found.alpha_ids, found.kinds), (Some(vec![5, 6]), Some(vec![2, 0])));
        assert_eq!((found.icc.as_deref(), found.exif.as_deref(), found.real_merge), (Some(&b"icc"[..]), Some(&b"exif"[..]), Some(true)));
    }

    #[test]
    fn takes_the_transparency_the_file_marks_and_takes_out_its_white() {
        let stored = [[255, 191], [255, 127], [255, 64]];
        let expected = [255, 255, 255, 0, 128, 0, 0, 128];
        let marks = [
            ("a negative layer count", Spec { layers: one_layer(-1, 2, 1, &[vec![0, 0], vec![0, 0], vec![0, 0], vec![0, 128]]), ..rgb_with_extra(stored, [0, 128]) }),
            ("alpha identifier 0", Spec { resources: resource(1053, &[0, 0, 0, 0]), ..rgb_with_extra(stored, [0, 128]) }),
            ("the Mtrn key", Spec { layers: [&[0u8, 0, 0, 0][..], b"8BIMMtrn", &[0, 0, 0, 0]].concat(), ..rgb_with_extra(stored, [0, 128]) }),
        ];
        for (mark, spec) in marks {
            let document = read_spec(&spec);
            assert_eq!((document.picture.alpha, eight(&document)), (true, &expected[..]), "{mark}");
        }
    }

    #[test]
    fn a_described_channel_is_never_transparency_and_a_spot_ink_is_named() {
        let selection = read_spec(&Spec { resources: resource(1053, &[0, 0, 0, 6]), ..rgb_with_extra([[1, 2]; 3], [0, 255]) });
        assert!(!selection.picture.alpha);
        let mut data = resource(1045, &unicode_names(&["Fluorescent pink"]));
        data.extend(resource(1077, &display_info(&[2])));
        let spot = read_spec(&Spec { resources: data, ..rgb_with_extra([[1, 2]; 3], [0, 255]) });
        assert!(!spot.picture.alpha);
        assert!(spot.warnings.iter().any(|w| w.contains("Fluorescent pink")), "{:?}", spot.warnings);
    }

    #[test]
    fn an_undescribed_channel_without_layers_is_transparency_its_white_taken_out_where_clear_pixels_are_white() {
        let white = read_spec(&rgb_with_extra([[255, 191], [255, 127], [255, 64]], [0, 128]));
        assert_eq!(eight(&white), &[255, 255, 255, 0, 128, 0, 0, 128]);
        let black = read_spec(&rgb_with_extra([[0, 191], [0, 127], [0, 64]], [0, 128]));
        assert_eq!(eight(&black), &[0, 0, 0, 0, 191, 127, 64, 128]);
        let two = read_spec(&Spec::new(3, 8, 1, 1, vec![vec![1], vec![2], vec![3], vec![4], vec![5]]));
        assert!(!two.picture.alpha);
        assert!(two.warnings.iter().any(|w| w.contains("2 channels")), "{:?}", two.warnings);
    }

    #[test]
    fn an_undescribed_channel_of_a_layered_document_is_transparency_only_where_the_merged_alpha_shows_it() {
        let layer = [vec![10, 20], vec![30, 40], vec![50, 60], vec![255, 0]];
        let matching = read_spec(&Spec { layers: one_layer(1, 2, 1, &layer), ..rgb_with_extra([[10, 255], [30, 255], [50, 255]], [255, 0]) });
        if cfg!(feature = "psd-merge") {
            assert_eq!((matching.picture.alpha, eight(&matching)), (true, &[10u8, 30, 50, 255, 255, 255, 255, 0][..]));
        } else {
            assert_eq!((matching.picture.alpha, eight(&matching)), (false, &[10u8, 30, 50, 255, 255, 255][..]));
            assert!(matching.warnings.iter().any(|w| w.contains("channel 4") && w.contains("does not merge")), "{:?}", matching.warnings);
        }
        let other = read_spec(&Spec { layers: one_layer(1, 2, 1, &layer), ..rgb_with_extra([[10, 255], [30, 255], [50, 255]], [0, 255]) });
        assert!(!other.picture.alpha);
    }

    #[test]
    fn refuses_what_it_does_not_read_and_says_why() {
        let why = |bytes: &[u8]| match read(bytes, crate::image_decode::COLORS) {
            Ok(_) => panic!("read"),
            Err(why) => why,
        };
        assert!(why(b"GIF89a").contains("Photoshop"));
        assert!(why(&Spec::new(7, 8, 1, 1, vec![vec![1], vec![2]]).bytes()).contains("multichannel"));
        assert!(why(&Spec::new(4, 32, 1, 1, vec![vec![0; 4]; 4]).bytes()).contains("32-bit CMYK"));
        let mut cut = Spec::new(3, 8, 2, 1, vec![vec![1, 2], vec![3, 4], vec![5, 6]]).bytes();
        cut.truncate(cut.len() - 1);
        assert!(why(&cut).contains("cut short"));
        let no_merge = Spec { resources: resource(1057, &[0, 0, 0, 1, 0]), ..Spec::new(3, 8, 1, 1, vec![vec![1], vec![2], vec![3]]) };
        assert!(why(&no_merge.bytes()).contains("no layers to merge"));
        if !cfg!(feature = "psd-merge") {
            let layered = Spec { layers: one_layer(1, 1, 1, &[vec![1], vec![2], vec![3], vec![255]]), ..no_merge };
            assert!(why(&layered.bytes()).contains("does not merge a document's layers"));
        }
    }
}
