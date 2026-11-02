//! What a clip's file says of itself, read from its own headers: its
//! container, the codecs of its picture and its sound, and its picture's
//! size. Every renderer names a format alike from it, and a window opens at
//! the picture's size before any backend has read the clip. The reader
//! looks only where a container keeps its headers, so a long file costs a
//! few small reads.

use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

/// A clip's container, the codecs of its picture and its sound, and the
/// picture's size in pixels as its headers give them.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Format {
    /// The container, as `MP4`, `Matroska` or `Ogg`.
    pub container: String,
    /// The picture's codec, as `H.264`; none for a clip of sound alone.
    pub video: Option<String>,
    /// The sound's codec, as `AAC`; none for a clip without sound.
    pub audio: Option<String>,
    /// The picture's size, where the headers give it.
    pub size: Option<[u32; 2]>,
}

impl Format {
    fn new(container: &str) -> Format {
        Format { container: container.to_string(), video: None, audio: None, size: None }
    }

    /// A file of sound alone, of codec `codec` in container `container`.
    fn sound(container: &str, codec: &str) -> Format {
        Format { audio: Some(codec.to_string()), ..Format::new(container) }
    }

    /// The format as an error names it: `H.265 (HEVC) video and AAC audio
    /// in MP4`, `H.264 video in MP4`, `Opus audio in Ogg`, or `FLAC audio`
    /// where the codec names its own container.
    pub fn name(&self) -> String {
        match (&self.video, &self.audio) {
            (Some(v), Some(a)) => format!("{v} video and {a} audio in {}", self.container),
            (Some(v), None) => format!("{v} video in {}", self.container),
            (None, Some(a)) if *a == self.container => format!("{a} audio"),
            (None, Some(a)) => format!("{a} audio in {}", self.container),
            (None, None) => format!("{} holding no picture or sound this reader names", self.container),
        }
    }
}

/// Where the reader reads a clip's headers from.
trait Source {
    fn length(&mut self) -> std::io::Result<u64>;
    /// Up to `count` bytes from `at`, fewer at the end.
    fn bytes(&mut self, at: u64, count: u64) -> std::io::Result<Vec<u8>>;
}

impl Source for std::fs::File {
    fn length(&mut self) -> std::io::Result<u64> {
        Ok(self.metadata()?.len())
    }

    fn bytes(&mut self, at: u64, count: u64) -> std::io::Result<Vec<u8>> {
        self.seek(SeekFrom::Start(at))?;
        let mut out = Vec::new();
        Read::by_ref(self).take(count).read_to_end(&mut out)?;
        Ok(out)
    }
}

impl Source for &[u8] {
    fn length(&mut self) -> std::io::Result<u64> {
        Ok(self.len() as u64)
    }

    fn bytes(&mut self, at: u64, count: u64) -> std::io::Result<Vec<u8>> {
        let len = self.len() as u64;
        let start = at.min(len) as usize;
        let end = at.saturating_add(count).min(len) as usize;
        Ok(self[start..end].to_vec())
    }
}

/// The format of the clip in the file at `path`, or why it cannot be told.
pub fn read(path: &Path) -> Result<Format, String> {
    let mut file = std::fs::File::open(path).map_err(|e| format!("the file cannot be opened: {e}"))?;
    read_from(&mut file)
}

/// The format of the clip whose file is `bytes`, or why it cannot be told.
pub fn read_bytes(bytes: &[u8]) -> Result<Format, String> {
    let mut source = bytes;
    read_from(&mut source)
}

/// The format of a clip read by its byte ranges, as a helper's loader reads
/// an https clip: `fetch` gives the bytes from a place, at most a count of
/// them, fewer at the end; `length` is the whole clip's where known.
pub fn read_ranges(length: Option<u64>, fetch: &mut dyn FnMut(u64, u64) -> Result<Vec<u8>, String>) -> Result<Format, String> {
    let mut source = Ranges { length, fetch };
    read_from(&mut source)
}

/// A clip read by its byte ranges.
struct Ranges<'a> {
    length: Option<u64>,
    fetch: &'a mut dyn FnMut(u64, u64) -> Result<Vec<u8>, String>,
}

impl Source for Ranges<'_> {
    fn length(&mut self) -> std::io::Result<u64> {
        Ok(self.length.unwrap_or(u64::MAX))
    }

    fn bytes(&mut self, at: u64, count: u64) -> std::io::Result<Vec<u8>> {
        (self.fetch)(at, count).map_err(std::io::Error::other)
    }
}

/// How many bytes the reader looks at first: where every container this
/// reader knows begins its headers.
const HEAD: u64 = 64 * 1024;

/// How many bytes the reader looks at where the first part held no header,
/// as in a stream recorded from its middle or one behind a large tag.
const WIDER: u64 = 1024 * 1024;

fn read_from(source: &mut dyn Source) -> Result<Format, String> {
    let io = |e: std::io::Error| format!("the file cannot be read: {e}");
    let head = source.bytes(0, HEAD).map_err(io)?;
    let first = named_in(source, &head);
    let found = first.as_ref().is_ok_and(|f| f.video.is_some() || f.audio.is_some());
    if found || (head.len() as u64) < HEAD {
        return first;
    }
    let wide = source.bytes(0, WIDER).map_err(io)?;
    named_in(source, &wide)
}

/// The format of the clip whose file begins with `head`, the rest read from
/// `source` where the container's own structure leads there.
fn named_in(source: &mut dyn Source, head: &[u8]) -> Result<Format, String> {
    let io = |e: std::io::Error| format!("the file cannot be read: {e}");
    let is = |at: usize, magic: &[u8]| has(head, at, magic);
    if [&b"ftyp"[..], b"moov", b"mdat", b"free", b"wide", b"skip"].iter().any(|code| is(4, code)) {
        return iso(source).map_err(io);
    }
    if is(0, &EBML.to_be_bytes()) {
        return matroska(source, head).map_err(io);
    }
    let found = if is(0, b"OggS") {
        Some(ogg(head))
    } else if is(0, b"RIFF") && is(8, b"WAVE") {
        Some(wave(head))
    } else if is(0, b"RIFF") && is(8, b"AVI ") {
        Some(avi(head))
    } else if is(0, &ASF_HEADER) {
        Some(asf(head))
    } else if is(0, b"FLV") {
        Some(flv(head))
    } else if is(0, b"FORM") && (is(8, b"AIFF") || is(8, b"AIFC")) {
        Some(aiff(head))
    } else if is(0, b"caff") {
        Some(caf(head))
    } else if is(0, b"DKIF") {
        Some(ivf(head))
    } else if is(0, &[0, 0, 1, 0xBA]) {
        Some(program(head))
    } else {
        let signatures: [(&[u8], &str, Option<&str>); 7] = [
            (b"fLaC", "FLAC", Some("FLAC")),
            (b"#!AMR-WB\n", "AMR", Some("AMR-WB")),
            (b"#!AMR\n", "AMR", Some("AMR-NB")),
            (b"wvpk", "WavPack", Some("WavPack")),
            (b"MAC ", "APE", Some("APE")),
            (b"MThd", "MIDI", None),
            (b".RMF", "RealMedia", None),
        ];
        signatures
            .iter()
            .find(|(magic, _, _)| is(0, magic))
        .map(|(_, container, codec)| Format { audio: codec.map(String::from), ..Format::new(container) })
        .or_else(|| transport(head))
        .or_else(|| elementary(head))
    };
    if let Some(format) = found {
        return Ok(format);
    }
    let start: Vec<String> = head.iter().take(8).map(|b| format!("{b:02x}")).collect();
    Err(if head.is_empty() { String::from("the file is empty") } else { format!("the file begins {}, which starts no clip this reader knows", start.join(" ")) })
}

fn u16be(b: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_be_bytes(*b.get(at..)?.first_chunk::<2>()?))
}

fn u32be(b: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_be_bytes(*b.get(at..)?.first_chunk::<4>()?))
}

fn u64be(b: &[u8], at: usize) -> Option<u64> {
    Some(u64::from_be_bytes(*b.get(at..)?.first_chunk::<8>()?))
}

fn u16le(b: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_le_bytes(*b.get(at..)?.first_chunk::<2>()?))
}

fn u32le(b: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes(*b.get(at..)?.first_chunk::<4>()?))
}

fn u64le(b: &[u8], at: usize) -> Option<u64> {
    Some(u64::from_le_bytes(*b.get(at..)?.first_chunk::<8>()?))
}

fn code_at(b: &[u8], at: usize) -> Option<[u8; 4]> {
    b.get(at..)?.first_chunk::<4>().copied()
}

/// Whether `b` holds `magic` at `at`.
fn has(b: &[u8], at: usize, magic: &[u8]) -> bool {
    b.get(at..).is_some_and(|rest| rest.starts_with(magic))
}

/// A four-character code as text, its unprintable bytes as `?`.
fn fourcc(code: &[u8]) -> String {
    code.iter().map(|&b| if b.is_ascii_graphic() || b == b' ' { b as char } else { '?' }).collect::<String>().trim_end().to_string()
}

/// A codec this reader does not know, named by its own code.
fn unknown(code: &str) -> String {
    format!("an unknown codec ({code})")
}

/// A size from a width and a height, none where either is 0 or absent.
fn size_of(width: Option<u32>, height: Option<u32>) -> Option<[u32; 2]> {
    let (w, h) = (width?, height?);
    (w > 0 && h > 0).then_some([w, h])
}

// ISO base media files: MP4, QuickTime, 3GP and M4A.

/// One ISO box: its type, and where its body starts and ends.
struct IsoBox {
    kind: [u8; 4],
    start: u64,
    end: u64,
}

/// The boxes from `start` to `end` of `source`. A box running past `end`
/// is cut to it, and the list ends where a box's header is cut short.
fn iso_boxes(source: &mut dyn Source, start: u64, end: u64) -> std::io::Result<Vec<IsoBox>> {
    let mut out = Vec::new();
    let mut at = start;
    while at + 8 <= end {
        let head = source.bytes(at, 16)?;
        let (Some(small), Some(kind)) = (u32be(&head, 0), code_at(&head, 4)) else {
            break;
        };
        let (header, size) = match (small, u64be(&head, 8)) {
            (0, _) => (8, end - at),
            (1, Some(big)) => (16, big),
            (1, None) => break,
            (n, _) => (8, u64::from(n)),
        };
        if size < header {
            break;
        }
        out.push(IsoBox { kind, start: at + header, end: at.saturating_add(size).min(end) });
        at = at.saturating_add(size);
    }
    Ok(out)
}

fn find<'a>(boxes: &'a [IsoBox], kind: &[u8; 4]) -> Option<&'a IsoBox> {
    boxes.iter().find(|b| &b.kind == kind)
}

/// The box at the end of `path` within `parent`, each step a child of the
/// one before.
fn descend(source: &mut dyn Source, parent: &IsoBox, path: &[&[u8; 4]]) -> std::io::Result<Option<IsoBox>> {
    let mut within = IsoBox { kind: parent.kind, start: parent.start, end: parent.end };
    for kind in path {
        let children = iso_boxes(source, within.start, within.end)?;
        match find(&children, kind) {
            Some(b) => within = IsoBox { kind: b.kind, start: b.start, end: b.end },
            None => return Ok(None),
        }
    }
    Ok(Some(within))
}

fn iso(source: &mut dyn Source) -> std::io::Result<Format> {
    let length = source.length()?;
    let top = iso_boxes(source, 0, length)?;
    let brand = match find(&top, b"ftyp") {
        Some(b) => code_at(&source.bytes(b.start, 4)?, 0),
        None => None,
    };
    let mut format = Format::new(match brand {
        Some(b) if &b == b"qt  " => "QuickTime",
        Some([b'3', b'g', _, _]) => "3GP",
        Some(_) => "MP4",
        None => "QuickTime",
    });
    let Some(moov) = find(&top, b"moov") else {
        return Ok(format);
    };
    for trak in iso_boxes(source, moov.start, moov.end)? {
        if &trak.kind != b"trak" {
            continue;
        }
        let display = match descend(source, &trak, &[b"tkhd"])? {
            Some(b) => {
                let tkhd = source.bytes(b.start, 96)?;
                let at = if tkhd.first() == Some(&1) { 88 } else { 76 };
                size_of(u32be(&tkhd, at).map(|w| w >> 16), u32be(&tkhd, at + 4).map(|h| h >> 16))
            }
            None => None,
        };
        let handler = match descend(source, &trak, &[b"mdia", b"hdlr"])? {
            Some(b) => code_at(&source.bytes(b.start + 8, 4)?, 0),
            None => None,
        };
        let Some(stsd) = descend(source, &trak, &[b"mdia", b"minf", b"stbl", b"stsd"])? else {
            continue;
        };
        let Some(entry) = iso_boxes(source, stsd.start + 8, stsd.end)?.into_iter().next() else {
            continue;
        };
        let body = source.bytes(entry.start, entry.end - entry.start)?;
        match handler {
            Some(h) if &h == b"vide" && format.video.is_none() => {
                format.video = Some(iso_video(&entry.kind));
                format.size = display.or(size_of(u16be(&body, 24).map(u32::from), u16be(&body, 26).map(u32::from)));
            }
            Some(h) if &h == b"soun" && format.audio.is_none() => {
                format.audio = Some(if &entry.kind == b"mp4a" { mp4a(&body) } else { iso_audio(&entry.kind) });
            }
            _ => {}
        }
    }
    Ok(format)
}

/// How far an ISO file's first picture and first sound stand from the
/// clip's start, in seconds: where each track's edit list starts its
/// media, less any empty edit before it. A player that reads no edit list,
/// as Media Foundation's, plays each that much late.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Edits {
    /// The first picture track's offset, where the clip has one.
    pub video: Option<f64>,
    /// The first sound track's offset, where the clip has one.
    pub audio: Option<f64>,
}

impl Edits {
    /// How far such a player's clock runs ahead of the clip's: by the
    /// picture's offset where the clip has a picture, else by the sound's.
    pub fn clock(&self) -> f64 {
        self.video.or(self.audio).unwrap_or(0.0)
    }

    /// How far later such a player must play the sound for it to stand
    /// where its clock puts the clip, in seconds; less than 0 is earlier.
    pub fn sound_shift(&self) -> f64 {
        self.audio.map_or(0.0, |a| self.clock() - a)
    }
}

/// The edit lists of the clip in the file at `path`: none for a clip that
/// is not an ISO file, and 0 for a track with none.
pub fn edits(path: &Path) -> Result<Edits, String> {
    let mut file = std::fs::File::open(path).map_err(|e| format!("the file cannot be opened: {e}"))?;
    edits_from(&mut file)
}

/// The edit lists of a clip read by its byte ranges, as [`read_ranges`]
/// reads its format.
pub fn edits_ranges(length: Option<u64>, fetch: &mut dyn FnMut(u64, u64) -> Result<Vec<u8>, String>) -> Result<Edits, String> {
    let mut source = Ranges { length, fetch };
    edits_from(&mut source)
}

fn edits_from(source: &mut dyn Source) -> Result<Edits, String> {
    let io = |e: std::io::Error| format!("the file cannot be read: {e}");
    let head = source.bytes(0, 8).map_err(io)?;
    let iso_file = [&b"ftyp"[..], b"moov", b"mdat", b"free", b"wide", b"skip"].iter().any(|code| has(&head, 4, code));
    if !iso_file {
        return Ok(Edits::default());
    }
    iso_edits(source).map_err(io)
}

/// The timescale a movie or media header gives, after its version and
/// flags and its two times, of 32 or 64 bits by its version.
fn timescale(header: &[u8]) -> Option<u32> {
    u32be(header, if header.first() == Some(&1) { 20 } else { 12 }).filter(|t| *t > 0)
}

fn iso_edits(source: &mut dyn Source) -> std::io::Result<Edits> {
    let length = source.length()?;
    let top = iso_boxes(source, 0, length)?;
    let mut edits = Edits::default();
    let Some(moov) = find(&top, b"moov") else {
        return Ok(edits);
    };
    let movie = match descend(source, moov, &[b"mvhd"])? {
        Some(b) => timescale(&source.bytes(b.start, 32)?),
        None => None,
    };
    for trak in iso_boxes(source, moov.start, moov.end)? {
        if &trak.kind != b"trak" {
            continue;
        }
        let handler = match descend(source, &trak, &[b"mdia", b"hdlr"])? {
            Some(b) => code_at(&source.bytes(b.start + 8, 4)?, 0),
            None => None,
        };
        let slot = match handler {
            Some(h) if &h == b"vide" && edits.video.is_none() => &mut edits.video,
            Some(h) if &h == b"soun" && edits.audio.is_none() => &mut edits.audio,
            _ => continue,
        };
        let media = match descend(source, &trak, &[b"mdia", b"mdhd"])? {
            Some(b) => timescale(&source.bytes(b.start, 32)?),
            None => None,
        };
        let list = match descend(source, &trak, &[b"edts", b"elst"])? {
            Some(b) => source.bytes(b.start, (b.end - b.start).min(8 + 20 * 16))?,
            None => Vec::new(),
        };
        *slot = Some(track_offset(&list, movie, media));
    }
    Ok(edits)
}

/// Where an edit list `list`, the body of an `elst` box, starts its
/// track's media, in seconds, less the empty edits before it: the movie's
/// timescale `movie` times those, the media's `media` the media's own. A
/// track with no list, or a list it cannot read, starts at 0.
fn track_offset(list: &[u8], movie: Option<u32>, media: Option<u32>) -> f64 {
    let wide = list.first() == Some(&1);
    let size = if wide { 20 } else { 12 };
    let count = u32be(list, 4).unwrap_or(0) as usize;
    let mut empty = 0.0;
    for i in 0..count {
        let at = 8 + i * size;
        let (duration, time) = if wide { (u64be(list, at), u64be(list, at + 8).map(|t| t as i64)) } else { (u32be(list, at).map(u64::from), u32be(list, at + 4).map(|t| i64::from(t as i32))) };
        let (Some(duration), Some(time)) = (duration, time) else {
            return 0.0;
        };
        if time < 0 {
            empty += movie.map_or(0.0, |m| duration as f64 / f64::from(m));
            continue;
        }
        return media.map_or(0.0, |m| time as f64 / f64::from(m)) - empty;
    }
    0.0
}

fn iso_video(code: &[u8; 4]) -> String {
    String::from(match code {
        b"avc1" | b"avc2" | b"avc3" | b"avc4" => "H.264",
        b"hvc1" | b"hev1" => "H.265 (HEVC)",
        b"vvc1" | b"vvi1" => "H.266 (VVC)",
        b"dvh1" | b"dvhe" | b"dvav" | b"dva1" => "Dolby Vision",
        b"av01" => "AV1",
        b"vp08" => "VP8",
        b"vp09" => "VP9",
        b"mp4v" => "MPEG-4 Part 2",
        b"s263" | b"h263" | b"H263" => "H.263",
        b"jpeg" | b"mjpa" | b"mjpb" | b"MJPG" => "Motion JPEG",
        b"mjp2" => "Motion JPEG 2000",
        b"apch" | b"apcn" | b"apcs" | b"apco" | b"ap4h" | b"ap4x" => "ProRes",
        b"raw " | b"2vuy" | b"yuv2" | b"v210" => "uncompressed",
        other => return unknown(&fourcc(other)),
    })
}

fn iso_audio(code: &[u8; 4]) -> String {
    String::from(match code {
        b".mp3" | b"mp3 " => "MP3",
        b"alac" => "ALAC",
        b"ac-3" => "AC-3",
        b"ec-3" => "E-AC-3",
        b"ac-4" => "AC-4",
        b"Opus" => "Opus",
        b"fLaC" => "FLAC",
        b"samr" => "AMR-NB",
        b"sawb" => "AMR-WB",
        b"lpcm" | b"sowt" | b"twos" | b"in24" | b"in32" | b"fl32" | b"fl64" | b"raw " | b"NONE" | b"ipcm" | b"fpcm" => "PCM",
        b"ulaw" => "mu-law",
        b"alaw" => "A-law",
        b"ima4" => "IMA ADPCM",
        b"dtsc" | b"dtsh" | b"dtsl" | b"dtse" => "DTS",
        b"mha1" | b"mhm1" => "MPEG-H 3D Audio",
        other => return unknown(&fourcc(other)),
    })
}

/// An `mp4a` sample entry's codec, which the object type its decoder
/// configuration gives tells.
fn mp4a(entry: &[u8]) -> String {
    String::from(match mp4a_object(entry) {
        Some(0x40 | 0x66 | 0x67 | 0x68) => "AAC",
        Some(0x69 | 0x6B) => "MP3",
        Some(0xA5) => "AC-3",
        Some(0xA6) => "E-AC-3",
        Some(0xA9) => "DTS",
        Some(0xAD) => "Opus",
        Some(0xDD) => "Vorbis",
        Some(other) => return unknown(&format!("object type 0x{other:02x}")),
        None => "MPEG-4 audio",
    })
}

/// The object type in the decoder configuration of the `esds` box within
/// an `mp4a` sample entry.
fn mp4a_object(entry: &[u8]) -> Option<u8> {
    let esds = entry.windows(4).position(|w| w == b"esds")? + 8;
    let (tag, mut at) = descriptor(entry, esds)?;
    if tag != 0x03 {
        return None;
    }
    let flags = *entry.get(at + 2)?;
    at += 3;
    if flags & 0x80 != 0 {
        at += 2;
    }
    if flags & 0x40 != 0 {
        at += 1 + usize::from(*entry.get(at)?);
    }
    if flags & 0x20 != 0 {
        at += 2;
    }
    let (tag, at) = descriptor(entry, at)?;
    if tag != 0x04 {
        return None;
    }
    entry.get(at).copied()
}

/// An MPEG-4 descriptor at `at`: its tag, and where its body starts after
/// a length of one to four bytes.
fn descriptor(b: &[u8], at: usize) -> Option<(u8, usize)> {
    let tag = *b.get(at)?;
    for i in 1..=4 {
        if b.get(at + i)? & 0x80 == 0 {
            return Some((tag, at + i + 1));
        }
    }
    None
}

// Matroska and WebM.

/// One EBML element: its id, and where its body starts and ends.
struct Element {
    id: u32,
    start: usize,
    end: usize,
}

/// An EBML element's id and how many bytes it takes, at `at`.
fn ebml_id(b: &[u8], at: usize) -> Option<(u32, usize)> {
    let first = *b.get(at)?;
    let length = first.leading_zeros() as usize + 1;
    if length > 4 {
        return None;
    }
    let bytes = b.get(at..at + length)?;
    Some((bytes.iter().fold(0, |id, &byte| (id << 8) | u32::from(byte)), length))
}

/// An EBML element's size and how many bytes it takes, at `at`; the size
/// is none where the file leaves it unknown.
fn ebml_size(b: &[u8], at: usize) -> Option<(Option<u64>, usize)> {
    let first = *b.get(at)?;
    let length = first.leading_zeros() as usize + 1;
    if length > 8 {
        return None;
    }
    let rest = b.get(at + 1..at + length)?;
    let marker = (0xFFu16 >> length) as u8;
    let size = rest.iter().fold(u64::from(first & marker), |v, &byte| (v << 8) | u64::from(byte));
    let unknown = first & marker == marker && rest.iter().all(|&byte| byte == 0xFF);
    Some((if unknown { None } else { Some(size) }, length))
}

/// The elements of `b` from `start` to `end`. An element running past
/// `end`, or of unknown size, is cut to it, and the list ends where an
/// element's header is cut short.
fn ebml_children(b: &[u8], start: usize, end: usize) -> Vec<Element> {
    let mut out = Vec::new();
    let mut at = start;
    while at < end {
        let Some((id, id_len)) = ebml_id(b, at) else {
            break;
        };
        let Some((size, size_len)) = ebml_size(b, at + id_len) else {
            break;
        };
        let body = at + id_len + size_len;
        if body > end {
            break;
        }
        let body_end = match size {
            Some(s) => (body as u64).saturating_add(s).min(end as u64) as usize,
            None => end,
        };
        out.push(Element { id, start: body, end: body_end });
        at = body_end;
    }
    out
}

fn ebml_uint(b: &[u8], e: &Element) -> u64 {
    b[e.start..e.end].iter().fold(0, |v, &byte| (v << 8) | u64::from(byte))
}

fn ebml_text(b: &[u8], e: &Element) -> String {
    String::from_utf8_lossy(&b[e.start..e.end]).trim_end_matches('\0').to_string()
}

fn first(elements: &[Element], id: u32) -> Option<&Element> {
    elements.iter().find(|e| e.id == id)
}

const EBML: u32 = 0x1A45_DFA3;
const DOC_TYPE: u32 = 0x4282;
const SEGMENT: u32 = 0x1853_8067;
const SEEK_HEAD: u32 = 0x114D_9B74;
const SEEK: u32 = 0x4DBB;
const SEEK_ID: u32 = 0x53AB;
const SEEK_POSITION: u32 = 0x53AC;
const TRACKS: u32 = 0x1654_AE6B;
const TRACK_ENTRY: u32 = 0xAE;
const TRACK_TYPE: u32 = 0x83;
const CODEC_ID: u32 = 0x86;
const VIDEO: u32 = 0xE0;
const PIXEL_WIDTH: u32 = 0xB0;
const PIXEL_HEIGHT: u32 = 0xBA;
const DISPLAY_WIDTH: u32 = 0x54B0;
const DISPLAY_HEIGHT: u32 = 0x54BA;
const DISPLAY_UNIT: u32 = 0x54B2;
const CLUSTER: u32 = 0x1F43_B675;

fn matroska(source: &mut dyn Source, head: &[u8]) -> std::io::Result<Format> {
    let top = ebml_children(head, 0, head.len());
    let doc_type = first(&top, EBML).and_then(|h| first(&ebml_children(head, h.start, h.end), DOC_TYPE).map(|d| ebml_text(head, d)));
    let mut format = Format::new(if doc_type.as_deref() == Some("webm") { "WebM" } else { "Matroska" });
    let Some(segment) = first(&top, SEGMENT) else {
        return Ok(format);
    };
    let children = ebml_children(head, segment.start, segment.end);
    let tracks = match first(&children, TRACKS) {
        Some(t) => head[t.start..t.end].to_vec(),
        None => match seek_to_tracks(head, &children) {
            Some(offset) => tracks_at(source, segment.start as u64 + offset)?,
            None => return Ok(format),
        },
    };
    for entry in ebml_children(&tracks, 0, tracks.len()).iter().filter(|e| e.id == TRACK_ENTRY) {
        let fields = ebml_children(&tracks, entry.start, entry.end);
        let kind = first(&fields, TRACK_TYPE).map(|e| ebml_uint(&tracks, e));
        let codec = match first(&fields, CODEC_ID) {
            Some(e) => matroska_codec(&ebml_text(&tracks, e)),
            None => String::from("an unnamed codec"),
        };
        match kind {
            Some(1) if format.video.is_none() => {
                format.video = Some(codec);
                if let Some(v) = first(&fields, VIDEO) {
                    let video = ebml_children(&tracks, v.start, v.end);
                    let side = |id: u32| first(&video, id).map(|e| ebml_uint(&tracks, e)).map(|n| n.min(u64::from(u32::MAX)) as u32);
                    let in_pixels = first(&video, DISPLAY_UNIT).is_none_or(|e| ebml_uint(&tracks, e) == 0);
                    let display = if in_pixels { size_of(side(DISPLAY_WIDTH), side(DISPLAY_HEIGHT)) } else { None };
                    format.size = display.or(size_of(side(PIXEL_WIDTH), side(PIXEL_HEIGHT)));
                }
            }
            Some(2) if format.audio.is_none() => format.audio = Some(codec),
            _ => {}
        }
    }
    Ok(format)
}

/// Where the Tracks element starts, counting from the segment's body, as
/// the segment's seek head gives it; none where the file's first part has
/// no seek head naming it.
fn seek_to_tracks(head: &[u8], segment: &[Element]) -> Option<u64> {
    for element in segment {
        if element.id == CLUSTER {
            return None;
        }
        if element.id != SEEK_HEAD {
            continue;
        }
        for seek in ebml_children(head, element.start, element.end).iter().filter(|e| e.id == SEEK) {
            let fields = ebml_children(head, seek.start, seek.end);
            let names = first(&fields, SEEK_ID).map(|e| &head[e.start..e.end]);
            if names == Some(&TRACKS.to_be_bytes()[..]) {
                return first(&fields, SEEK_POSITION).map(|e| ebml_uint(head, e));
            }
        }
    }
    None
}

/// The body of the Tracks element at `at` in `source`; empty where no
/// Tracks element stands there.
fn tracks_at(source: &mut dyn Source, at: u64) -> std::io::Result<Vec<u8>> {
    let header = source.bytes(at, 12)?;
    let Some((TRACKS, id_len)) = ebml_id(&header, 0) else {
        return Ok(Vec::new());
    };
    let Some((Some(size), size_len)) = ebml_size(&header, id_len) else {
        return Ok(Vec::new());
    };
    source.bytes(at + (id_len + size_len) as u64, size)
}

fn matroska_codec(id: &str) -> String {
    String::from(match id {
        "V_VP8" => "VP8",
        "V_VP9" => "VP9",
        "V_AV1" => "AV1",
        "V_MPEG4/ISO/AVC" => "H.264",
        "V_MPEGH/ISO/HEVC" => "H.265 (HEVC)",
        "V_MPEGI/ISO/VVC" => "H.266 (VVC)",
        "V_MPEG1" => "MPEG-1",
        "V_MPEG2" => "MPEG-2",
        "V_THEORA" => "Theora",
        "V_MJPEG" => "Motion JPEG",
        "V_PRORES" => "ProRes",
        "V_UNCOMPRESSED" => "uncompressed",
        "V_MS/VFW/FOURCC" => "a Video for Windows codec",
        "A_VORBIS" => "Vorbis",
        "A_OPUS" => "Opus",
        "A_FLAC" => "FLAC",
        "A_MPEG/L3" => "MP3",
        "A_MPEG/L2" => "MP2",
        "A_AC3" => "AC-3",
        "A_EAC3" => "E-AC-3",
        "A_DTS" => "DTS",
        "A_TRUEHD" => "Dolby TrueHD",
        "A_ALAC" => "ALAC",
        "A_MS/ACM" => "an ACM codec",
        other if other.starts_with("V_MPEG4/ISO/") => "MPEG-4 Part 2",
        other if other.starts_with("A_AAC") => "AAC",
        other if other.starts_with("A_PCM/") => "PCM",
        "" => "an unnamed codec",
        other => return unknown(other),
    })
}

// Ogg.

/// The codecs of the streams whose first pages open an Ogg file, and a
/// Theora picture's size.
fn ogg(head: &[u8]) -> Format {
    let mut format = Format::new("Ogg");
    let mut at = 0;
    while has(head, at, b"OggS") {
        let (Some(&flags), Some(&count)) = (head.get(at + 5), head.get(at + 26)) else {
            break;
        };
        if flags & 0x02 == 0 {
            break;
        }
        let table = at + 27;
        let Some(lengths) = head.get(table..table + usize::from(count)) else {
            break;
        };
        let data = table + usize::from(count);
        let length: usize = lengths.iter().map(|&b| usize::from(b)).sum();
        let packet = &head[data.min(head.len())..(data + length).min(head.len())];
        if packet.starts_with(b"\x80theora") {
            format.video = Some(String::from("Theora"));
            let side = |at: usize| packet.get(at..at + 3).map(|b| (u32::from(b[0]) << 16) | (u32::from(b[1]) << 8) | u32::from(b[2]));
            format.size = size_of(side(14), side(17));
        } else if packet.starts_with(b"OVP80") {
            format.video = Some(String::from("VP8"));
        } else if packet.starts_with(b"BBCD") {
            format.video = Some(String::from("Dirac"));
        } else if format.audio.is_none() {
            let sounds: [(&[u8], &str); 4] = [(b"\x01vorbis", "Vorbis"), (b"OpusHead", "Opus"), (b"\x7FFLAC", "FLAC"), (b"Speex   ", "Speex")];
            format.audio = sounds.iter().find(|(magic, _)| packet.starts_with(magic)).map(|(_, codec)| (*codec).to_string());
        }
        at = data + length;
    }
    format
}

// RIFF files, WAV and AVI, and IFF files, AIFF.

/// One RIFF or IFF chunk: its code, and where its body starts and ends.
struct Chunk {
    code: [u8; 4],
    start: usize,
    end: usize,
}

/// The chunks of `b` from `start` to `end`, their sizes little-endian for
/// RIFF and big-endian for IFF. A chunk running past `end` is cut to it.
fn chunks(b: &[u8], start: usize, end: usize, big_endian: bool) -> Vec<Chunk> {
    let mut out = Vec::new();
    let mut at = start;
    while at + 8 <= end {
        let size = if big_endian { u32be(b, at + 4) } else { u32le(b, at + 4) };
        let (Some(code), Some(size)) = (code_at(b, at), size) else {
            break;
        };
        let body = at + 8;
        let body_end = (body as u64 + u64::from(size)).min(end as u64) as usize;
        out.push(Chunk { code, start: body, end: body_end });
        at = body_end + (body_end - body) % 2;
    }
    out
}

fn chunk<'a>(list: &'a [Chunk], code: &[u8; 4]) -> Option<&'a Chunk> {
    list.iter().find(|c| &c.code == code)
}

/// A WAVEFORMATEX's codec, by its format tag, or for an extensible format
/// by the tag its subformat holds.
fn wave_codec(fmt: &[u8]) -> String {
    let tag = match (u16le(fmt, 0), u16le(fmt, 24)) {
        (Some(0xFFFE), Some(sub)) => sub,
        (Some(t), _) => t,
        (None, _) => return String::from("an unnamed codec"),
    };
    String::from(match tag {
        0x0001 | 0x0003 => "PCM",
        0x0002 => "MS ADPCM",
        0x0006 => "A-law",
        0x0007 => "mu-law",
        0x000A => "WMA Voice",
        0x0011 => "IMA ADPCM",
        0x0031 => "GSM 6.10",
        0x0050 => "MP2",
        0x0055 => "MP3",
        0x00FF | 0x1610 => "AAC",
        0x0160 | 0x0161 => "WMA",
        0x0162 => "WMA Pro",
        0x0163 => "WMA Lossless",
        0x2000 => "AC-3",
        0x2001 => "DTS",
        0xF1AC => "FLAC",
        0x704F => "Opus",
        0x674F..=0x6751 => "Vorbis",
        other => return unknown(&format!("format tag 0x{other:04x}")),
    })
}

fn wave(head: &[u8]) -> Format {
    let list = chunks(head, 12, head.len(), false);
    Format::sound("WAV", &chunk(&list, b"fmt ").map_or_else(|| String::from("an unnamed codec"), |c| wave_codec(&head[c.start..c.end])))
}

fn avi(head: &[u8]) -> Format {
    let mut format = Format::new("AVI");
    let top = chunks(head, 12, head.len(), false);
    let Some(hdrl) = top.iter().find(|c| &c.code == b"LIST" && has(head, c.start, b"hdrl")) else {
        return format;
    };
    for strl in chunks(head, hdrl.start + 4, hdrl.end, false) {
        if &strl.code != b"LIST" || !has(head, strl.start, b"strl") {
            continue;
        }
        let parts = chunks(head, strl.start + 4, strl.end, false);
        let kind = chunk(&parts, b"strh").and_then(|c| code_at(head, c.start));
        let Some(strf) = chunk(&parts, b"strf").map(|c| &head[c.start..c.end]) else {
            continue;
        };
        match kind {
            Some(k) if &k == b"vids" && format.video.is_none() => {
                format.video = Some(match code_at(strf, 16) {
                    Some(code) => avi_video(&code),
                    None => String::from("an unnamed codec"),
                });
                let side = |at: usize| u32le(strf, at).map(|n| n.cast_signed().unsigned_abs());
                format.size = size_of(side(4), side(8));
            }
            Some(k) if &k == b"auds" && format.audio.is_none() => format.audio = Some(wave_codec(strf)),
            _ => {}
        }
    }
    format
}

/// A Video for Windows codec, by its four-character code.
fn avi_video(code: &[u8; 4]) -> String {
    let mut upper = *code;
    upper.make_ascii_uppercase();
    String::from(match &upper {
        b"MJPG" | b"AVRN" | b"LJPG" => "Motion JPEG",
        b"H264" | b"X264" | b"AVC1" => "H.264",
        b"HEVC" | b"H265" | b"HVC1" => "H.265 (HEVC)",
        b"XVID" | b"DIVX" | b"DX50" | b"FMP4" | b"MP4V" => "MPEG-4 Part 2",
        b"MP42" | b"MP43" | b"DIV3" => "Microsoft MPEG-4",
        b"WMV1" => "WMV 7",
        b"WMV2" => "WMV 8",
        b"WMV3" => "WMV 9",
        b"WVC1" => "VC-1",
        b"CVID" => "Cinepak",
        b"IV50" | b"IV41" | b"IV32" => "Indeo",
        b"HFYU" => "HuffYUV",
        b"FFV1" => "FFV1",
        b"VP80" => "VP8",
        b"VP90" => "VP9",
        b"AV01" => "AV1",
        b"\0\0\0\0" | b"DIB " | b"RGB " => "uncompressed",
        _ => return unknown(&fourcc(code)),
    })
}

fn aiff(head: &[u8]) -> Format {
    let list = chunks(head, 12, head.len(), true);
    let compression = match (head.get(8..12), chunk(&list, b"COMM")) {
        (Some(b"AIFC"), Some(comm)) => code_at(head, comm.start + 18),
        _ => None,
    };
    let lower = compression.map(|mut c| {
        c.make_ascii_lowercase();
        c
    });
    let codec = match lower.as_ref() {
        None | Some(b"none" | b"sowt" | b"twos" | b"in24" | b"in32" | b"fl32" | b"fl64") => String::from("PCM"),
        Some(b"ulaw") => String::from("mu-law"),
        Some(b"alaw") => String::from("A-law"),
        Some(b"ima4") => String::from("IMA ADPCM"),
        Some(other) => unknown(&fourcc(other)),
    };
    Format::sound("AIFF", &codec)
}

/// A Core Audio file's codec, from its description chunk.
fn caf(head: &[u8]) -> Format {
    let mut at = 8;
    let mut codec = String::from("an unnamed codec");
    while let (Some(code), Some(size)) = (code_at(head, at), u64be(head, at + 4)) {
        if &code == b"desc" {
            if let Some(id) = code_at(head, at + 12 + 8) {
                codec = caf_codec(&id);
            }
            break;
        }
        at = (at as u64).saturating_add(12).saturating_add(size).min(head.len() as u64) as usize;
    }
    Format::sound("CAF", &codec)
}

fn caf_codec(id: &[u8; 4]) -> String {
    String::from(match id {
        b"lpcm" => "PCM",
        b"aac " => "AAC",
        b"alac" => "ALAC",
        b"opus" => "Opus",
        b"flac" => "FLAC",
        b"ima4" => "IMA ADPCM",
        b"ulaw" => "mu-law",
        b"alaw" => "A-law",
        b".mp3" => "MP3",
        b"ac-3" => "AC-3",
        other => return unknown(&fourcc(other)),
    })
}

// ASF: Windows Media.

/// The GUIDs of an ASF file's header object, its stream properties
/// objects, and their audio and video stream types, as the file stores
/// them.
const ASF_HEADER: [u8; 16] = [0x30, 0x26, 0xB2, 0x75, 0x8E, 0x66, 0xCF, 0x11, 0xA6, 0xD9, 0x00, 0xAA, 0x00, 0x62, 0xCE, 0x6C];
const ASF_STREAM: [u8; 16] = [0x91, 0x07, 0xDC, 0xB7, 0xB7, 0xA9, 0xCF, 0x11, 0x8E, 0xE6, 0x00, 0xC0, 0x0C, 0x20, 0x53, 0x65];
const ASF_AUDIO: [u8; 16] = [0x40, 0x9E, 0x69, 0xF8, 0x4D, 0x5B, 0xCF, 0x11, 0xA8, 0xFD, 0x00, 0x80, 0x5F, 0x5C, 0x44, 0x2B];
const ASF_VIDEO: [u8; 16] = [0xC0, 0xEF, 0x19, 0xBC, 0x4D, 0x5B, 0xCF, 0x11, 0xA8, 0xFD, 0x00, 0x80, 0x5F, 0x5C, 0x44, 0x2B];

fn asf(head: &[u8]) -> Format {
    let mut format = Format::new("ASF");
    let mut at = 30;
    while let (Some(guid), Some(size)) = (head.get(at..at + 16), u64le(head, at + 16)) {
        if size < 24 {
            break;
        }
        if guid == ASF_STREAM {
            let kind = head.get(at + 24..at + 40);
            let specific = at + 24 + 54;
            if kind == Some(&ASF_VIDEO[..]) && format.video.is_none() {
                format.size = size_of(u32le(head, specific), u32le(head, specific + 4));
                format.video = Some(match code_at(head, specific + 11 + 16) {
                    Some(code) => avi_video(&code),
                    None => String::from("an unnamed codec"),
                });
            } else if kind == Some(&ASF_AUDIO[..]) && format.audio.is_none() {
                format.audio = Some(match head.get(specific..) {
                    Some(fmt) => wave_codec(fmt),
                    None => String::from("an unnamed codec"),
                });
            }
        }
        at = (at as u64).saturating_add(size).min(head.len() as u64) as usize;
    }
    format
}

// Flash Video.

fn flv(head: &[u8]) -> Format {
    let mut format = Format::new("Flash Video (FLV)");
    let (Some(&flags), Some(offset)) = (head.get(4), u32be(head, 5)) else {
        return format;
    };
    let (has_audio, has_video) = (flags & 0x04 != 0, flags & 0x01 != 0);
    let mut at = offset as usize + 4;
    while let (Some(&kind), Some(size)) = (head.get(at), head.get(at + 1..at + 4)) {
        let size = (usize::from(size[0]) << 16) | (usize::from(size[1]) << 8) | usize::from(size[2]);
        let data = &head[(at + 11).min(head.len())..(at + 11 + size).min(head.len())];
        match kind & 0x1F {
            8 if format.audio.is_none() => format.audio = data.first().map(|b| flv_audio(b >> 4)),
            9 if format.video.is_none() => format.video = data.first().map(|&b| flv_video(b, data)),
            18 if format.size.is_none() => format.size = flv_size(data),
            _ => {}
        }
        if format.audio.is_some() == has_audio && format.video.is_some() == has_video && (format.size.is_some() || !has_video) {
            break;
        }
        at += 11 + size + 4;
    }
    format
}

fn flv_audio(code: u8) -> String {
    String::from(match code {
        0 | 3 => "PCM",
        1 => "ADPCM",
        2 | 14 => "MP3",
        4..=6 => "Nellymoser",
        7 => "A-law",
        8 => "mu-law",
        10 => "AAC",
        11 => "Speex",
        other => return unknown(&format!("sound format {other}")),
    })
}

/// An FLV video tag's codec, from its first byte, or from the four
/// characters after it where the tag is an enhanced one.
fn flv_video(first: u8, data: &[u8]) -> String {
    if first & 0x80 != 0 {
        return match code_at(data, 1) {
            Some(code) => iso_video(&code),
            None => String::from("an unnamed codec"),
        };
    }
    String::from(match first & 0x0F {
        2 => "Sorenson Spark (H.263)",
        3 => "Screen Video",
        4 => "On2 VP6",
        5 => "On2 VP6 with alpha",
        6 => "Screen Video 2",
        7 => "H.264",
        12 => "H.265 (HEVC)",
        other => return unknown(&format!("codec id {other}")),
    })
}

/// The width and height an FLV's onMetaData script gives.
fn flv_size(data: &[u8]) -> Option<[u32; 2]> {
    let number = |name: &[u8]| -> Option<u32> {
        let mut key = (name.len() as u16).to_be_bytes().to_vec();
        key.extend_from_slice(name);
        let at = data.windows(key.len()).position(|w| w == key.as_slice())? + key.len();
        if *data.get(at)? != 0 {
            return None;
        }
        let v = f64::from_be_bytes(*data.get(at + 1..)?.first_chunk::<8>()?);
        (v.is_finite() && v >= 1.0 && v <= f64::from(u32::MAX)).then_some(v as u32)
    };
    size_of(number(b"width"), number(b"height"))
}

// IVF, MPEG transport and program streams, and streams with no container.

fn ivf(head: &[u8]) -> Format {
    let video = match code_at(head, 8) {
        Some(code) if &code == b"VP80" => String::from("VP8"),
        Some(code) if &code == b"VP90" => String::from("VP9"),
        Some(code) if &code == b"AV01" => String::from("AV1"),
        Some(code) => unknown(&fourcc(&code)),
        None => String::from("an unnamed codec"),
    };
    let size = size_of(u16le(head, 12).map(u32::from), u16le(head, 14).map(u32::from));
    Format { video: Some(video), size, ..Format::new("IVF") }
}

/// An MPEG transport stream's codecs, which its first program's map
/// names; none for bytes that are no transport stream.
fn transport(head: &[u8]) -> Option<Format> {
    let synced = |offset: usize, step: usize| (0..3).all(|i| head.get(offset + i * step) == Some(&0x47));
    let (offset, step) = if synced(0, 188) {
        (0, 188)
    } else if synced(4, 192) {
        (4, 192)
    } else {
        return None;
    };
    let mut format = Format::new("MPEG-TS");
    let mut map = None;
    for packet in head.get(offset..)?.chunks(step).filter(|p| p.len() >= 188 && p[0] == 0x47 && p[1] & 0x40 != 0) {
        let pid = (u16::from(packet[1] & 0x1F) << 8) | u16::from(packet[2]);
        let payload = if packet[3] & 0x20 != 0 { 5 + usize::from(packet[4]) } else { 4 };
        let Some(&pointer) = packet.get(payload) else {
            continue;
        };
        let table = &packet[(payload + 1 + usize::from(pointer)).min(188)..188];
        let Some(length) = u16be(table, 1).map(|n| usize::from(n & 0x0FFF)) else {
            continue;
        };
        let section = &table[..(3 + length).saturating_sub(4).min(table.len())];
        if pid == 0 && table.first() == Some(&0x00) {
            map = section.get(8..).and_then(|programs| programs.chunks(4).find(|p| p.len() == 4 && (p[0], p[1]) != (0, 0))).map(|p| (u16::from(p[2] & 0x1F) << 8) | u16::from(p[3]));
        } else if Some(pid) == map && table.first() == Some(&0x02) {
            let info = u16be(section, 10).map_or(section.len(), |n| usize::from(n & 0x0FFF));
            let mut at = 12 + info;
            while let (Some(&kind), Some(es_info)) = (section.get(at), u16be(section, at + 3)) {
                match kind {
                    0x01 | 0x02 | 0x10 | 0x1B | 0x24 | 0x33 | 0x42 | 0xEA if format.video.is_none() => format.video = Some(transport_video(kind)),
                    0x03 | 0x04 | 0x0F | 0x11 | 0x81 | 0x87 if format.audio.is_none() => format.audio = Some(transport_audio(kind)),
                    _ => {}
                }
                at += 5 + usize::from(es_info & 0x0FFF);
            }
            return Some(format);
        }
    }
    Some(format)
}

fn transport_video(kind: u8) -> String {
    String::from(match kind {
        0x01 => "MPEG-1",
        0x02 => "MPEG-2",
        0x10 => "MPEG-4 Part 2",
        0x1B => "H.264",
        0x24 => "H.265 (HEVC)",
        0x33 => "H.266 (VVC)",
        0x42 => "AVS",
        _ => "VC-1",
    })
}

fn transport_audio(kind: u8) -> String {
    String::from(match kind {
        0x03 | 0x04 => "MPEG-1 or MPEG-2",
        0x0F | 0x11 => "AAC",
        0x81 => "AC-3",
        _ => "E-AC-3",
    })
}

/// An MPEG program stream's picture, from its first sequence header, and
/// whether it holds MPEG audio.
fn program(head: &[u8]) -> Format {
    let mut format = Format::new("MPEG-PS");
    if let Some(at) = head.windows(4).position(|w| w == [0, 0, 1, 0xB3]) {
        format.video = Some(String::from("MPEG-1 or MPEG-2"));
        if let Some(b) = head.get(at + 4..at + 7) {
            let width = (u32::from(b[0]) << 4) | (u32::from(b[1]) >> 4);
            let height = (u32::from(b[1] & 0x0F) << 8) | u32::from(b[2]);
            format.size = size_of(Some(width), Some(height));
        }
    }
    if head.windows(4).any(|w| w[..3] == [0, 0, 1] && (0xC0..=0xDF).contains(&w[3])) {
        format.audio = Some(String::from("MPEG-1 or MPEG-2"));
    }
    format
}

/// A stream of MPEG audio, ADTS AAC, AC-3 or DTS with no container, after
/// any ID3 tag; none for bytes that start no such stream.
fn elementary(head: &[u8]) -> Option<Format> {
    let mut at = 0;
    if head.starts_with(b"ID3") {
        let size = head.get(6..10)?.iter().fold(0usize, |v, &b| (v << 7) | usize::from(b & 0x7F));
        let footer = if head.get(5)? & 0x10 != 0 { 10 } else { 0 };
        at = 10 + size + footer;
    }
    let frame = head.get(at..at + 4)?;
    if frame[..2] == [0x0B, 0x77] {
        let bsid = head.get(at + 5).map(|b| b >> 3);
        return Some(Format::sound("AC-3", if bsid.is_some_and(|b| b > 10) { "E-AC-3" } else { "AC-3" }));
    }
    if frame == [0x7F, 0xFE, 0x80, 0x01] {
        return Some(Format::sound("DTS", "DTS"));
    }
    if frame[0] != 0xFF || frame[1] & 0xE0 != 0xE0 {
        return None;
    }
    let (version, layer) = ((frame[1] >> 3) & 0x03, (frame[1] >> 1) & 0x03);
    if frame[1] & 0xF6 == 0xF0 {
        return Some(Format::sound("ADTS", "AAC"));
    }
    let (bitrate, rate) = (frame[2] >> 4, (frame[2] >> 2) & 0x03);
    if version == 1 || layer == 0 || bitrate == 0x0F || rate == 0x03 {
        return None;
    }
    Some(match layer {
        1 => Format::sound("MP3", "MP3"),
        2 => Format::sound("MP2", "MP2"),
        _ => Format::sound("MP1", "MP1"),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn named(bytes: &[u8]) -> String {
        read_bytes(bytes).expect("a format").name()
    }

    /// An ISO box of type `kind` holding `body`.
    fn iso_box(kind: &[u8; 4], body: &[u8]) -> Vec<u8> {
        let mut out = u32::try_from(body.len() + 8).expect("a small box").to_be_bytes().to_vec();
        out.extend_from_slice(kind);
        out.extend_from_slice(body);
        out
    }

    /// A track of handler `handler` whose one sample entry is `entry`, its
    /// header giving `display` as its size.
    fn trak(handler: &[u8; 4], entry: &[u8], display: [u32; 2]) -> Vec<u8> {
        let mut tkhd = vec![0u8; 84];
        tkhd[76..80].copy_from_slice(&(display[0] << 16).to_be_bytes());
        tkhd[80..84].copy_from_slice(&(display[1] << 16).to_be_bytes());
        let mut hdlr = vec![0u8; 8];
        hdlr.extend_from_slice(handler);
        hdlr.extend_from_slice(&[0; 12]);
        let mut stsd = vec![0, 0, 0, 0, 0, 0, 0, 1];
        stsd.extend_from_slice(entry);
        let stbl = iso_box(b"stbl", &iso_box(b"stsd", &stsd));
        let mdia = [iso_box(b"hdlr", &hdlr), iso_box(b"minf", &stbl)].concat();
        iso_box(b"trak", &[iso_box(b"tkhd", &tkhd), iso_box(b"mdia", &mdia)].concat())
    }

    /// A visual sample entry of `code`, coded at `size`.
    fn visual(code: &[u8; 4], size: [u16; 2]) -> Vec<u8> {
        let mut body = vec![0u8; 70];
        body[24..26].copy_from_slice(&size[0].to_be_bytes());
        body[26..28].copy_from_slice(&size[1].to_be_bytes());
        iso_box(code, &body)
    }

    /// An `mp4a` sample entry whose decoder configuration gives `object`.
    fn mp4a_entry(object: u8) -> Vec<u8> {
        let esds = iso_box(b"esds", &[0, 0, 0, 0, 0x03, 0x80, 0x80, 0x80, 0x0D, 0x00, 0x01, 0x00, 0x04, 0x80, 0x80, 0x80, 0x05, object, 0x15, 0, 0, 0]);
        iso_box(b"mp4a", &[vec![0u8; 28], esds].concat())
    }

    #[test]
    fn an_mp4_names_its_codecs_and_its_picture_s_size_wherever_its_movie_box_stands() {
        let moov = [trak(b"vide", &visual(b"hvc1", [1920, 1088]), [1920, 1080]), trak(b"soun", &mp4a_entry(0x40), [0, 0])].concat();
        let file = [iso_box(b"ftyp", b"isom\0\0\x02\0isomiso2mp41"), iso_box(b"mdat", &[0u8; 5000]), iso_box(b"moov", &moov)].concat();
        let format = read_bytes(&file).expect("a format");
        assert_eq!(format, Format { container: String::from("MP4"), video: Some(String::from("H.265 (HEVC)")), audio: Some(String::from("AAC")), size: Some([1920, 1080]) }, "the track header's size over the coded one");
        assert_eq!(format.name(), "H.265 (HEVC) video and AAC audio in MP4");
        let quicktime = [iso_box(b"ftyp", b"qt  \0\0\0\0qt  "), iso_box(b"moov", &trak(b"vide", &visual(b"avc1", [320, 240]), [0, 0]))].concat();
        let format = read_bytes(&quicktime).expect("a format");
        assert_eq!((format.name(), format.size), (String::from("H.264 video in QuickTime"), Some([320, 240])), "the coded size where the track header gives none");
        let m4a = [iso_box(b"ftyp", b"M4A \0\0\0\0"), iso_box(b"moov", &trak(b"soun", &mp4a_entry(0x6B), [0, 0]))].concat();
        assert_eq!(named(&m4a), "MP3 audio in MP4", "by the decoder configuration's object type");
    }

    /// An EBML element of `id` holding `body`, its size in eight bytes.
    fn ebml(id: &[u8], body: &[u8]) -> Vec<u8> {
        let mut out = id.to_vec();
        out.push(0x01);
        out.extend_from_slice(&(body.len() as u64).to_be_bytes()[1..]);
        out.extend_from_slice(body);
        out
    }

    #[test]
    fn a_webm_names_its_tracks_codecs_and_its_picture_s_size_wherever_its_tracks_stand() {
        let header = ebml(&[0x1A, 0x45, 0xDF, 0xA3], &ebml(&[0x42, 0x82], b"webm"));
        let picture = [ebml(&[0xB0], &[0x01, 0x40]), ebml(&[0xBA], &[0xF0])].concat();
        let video = [ebml(&[0x83], &[1]), ebml(&[0x86], b"V_VP9"), ebml(&[0xE0], &picture)].concat();
        let audio = [ebml(&[0x83], &[2]), ebml(&[0x86], b"A_OPUS")].concat();
        let tracks = ebml(&[0x16, 0x54, 0xAE, 0x6B], &[ebml(&[0xAE], &video), ebml(&[0xAE], &audio)].concat());
        let unknown_size = [0x18, 0x53, 0x80, 0x67, 0x01, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF];
        let format = read_bytes(&[header.clone(), unknown_size.to_vec(), tracks.clone()].concat()).expect("a format");
        assert_eq!((format.name(), format.size), (String::from("VP9 video and Opus audio in WebM"), Some([320, 240])));
        let seek_head = |offset: u64| {
            let seek = [ebml(&[0x53, 0xAB], &[0x16, 0x54, 0xAE, 0x6B]), ebml(&[0x53, 0xAC], &offset.to_be_bytes())].concat();
            ebml(&[0x11, 0x4D, 0x9B, 0x74], &ebml(&[0x4D, 0xBB], &seek))
        };
        let void = ebml(&[0xEC], &vec![0u8; 70_000]);
        let offset = (seek_head(0).len() + void.len()) as u64;
        let body = [seek_head(offset), void, tracks].concat();
        let far = [header, ebml(&[0x18, 0x53, 0x80, 0x67], &body)].concat();
        assert_eq!(named(&far), "VP9 video and Opus audio in WebM", "the tracks past the first part, found through the seek head");
    }

    /// An Ogg page opening a stream with `packet`.
    fn ogg_page(packet: &[u8]) -> Vec<u8> {
        let mut out = b"OggS\0\x02".to_vec();
        out.extend_from_slice(&[0; 20]);
        out.push(1);
        out.push(u8::try_from(packet.len()).expect("a short packet"));
        out.extend_from_slice(packet);
        out
    }

    #[test]
    fn an_ogg_names_its_streams_and_a_theora_picture_s_size() {
        let mut theora = b"\x80theora\x03\x02\x01\0\x14\0\x0f".to_vec();
        theora.extend_from_slice(&[0x00, 0x01, 0x40, 0x00, 0x00, 0xF0]);
        theora.extend_from_slice(&[0; 20]);
        let format = read_bytes(&[ogg_page(&theora), ogg_page(b"\x01vorbis\0\0\0\0\x02")].concat()).expect("a format");
        assert_eq!((format.name(), format.size), (String::from("Theora video and Vorbis audio in Ogg"), Some([320, 240])));
        assert_eq!(named(&ogg_page(b"OpusHead\x01\x02")), "Opus audio in Ogg");
    }

    /// A RIFF chunk of `code` holding `body`.
    fn riff(code: &[u8; 4], body: &[u8]) -> Vec<u8> {
        let mut out = code.to_vec();
        out.extend_from_slice(&u32::try_from(body.len()).expect("a small chunk").to_le_bytes());
        out.extend_from_slice(body);
        if body.len() % 2 == 1 {
            out.push(0);
        }
        out
    }

    #[test]
    fn sound_files_name_their_codec() {
        let fmt = riff(b"fmt ", &[1, 0, 2, 0, 0x80, 0xBB, 0, 0, 0, 0, 0, 0, 4, 0, 16, 0]);
        assert_eq!(named(&[&b"RIFF\0\0\0\0WAVE"[..], &fmt].concat()), "PCM audio in WAV");
        assert_eq!(named(b"fLaC\0\0\0\x22"), "FLAC audio");
        assert_eq!(named(b"ID3\x04\0\0\0\0\0\0\xFF\xFB\x90\x64"), "MP3 audio", "after its ID3 tag");
        assert_eq!(named(&[0xFF, 0xF1, 0x50, 0x80]), "AAC audio in ADTS");
        let comm = [&b"COMM\0\0\0\x12"[..], &[0; 18]].concat();
        assert_eq!(named(&[&b"FORM\0\0\0\x26AIFF"[..], &comm].concat()), "PCM audio in AIFF");
        let comm = [&b"COMM\0\0\0\x16"[..], &[0; 18], b"ulaw"].concat();
        assert_eq!(named(&[&b"FORM\0\0\0\x2AAIFC"[..], &comm].concat()), "mu-law audio in AIFF");
        assert_eq!(named(b"#!AMR\n\x3c"), "AMR-NB audio in AMR");
    }

    #[test]
    fn an_avi_names_its_codecs_and_its_picture_s_size() {
        let mut bitmap = vec![0u8; 40];
        bitmap[4..8].copy_from_slice(&320i32.to_le_bytes());
        bitmap[8..12].copy_from_slice(&(-240i32).to_le_bytes());
        bitmap[16..20].copy_from_slice(b"MJPG");
        let strl = [b"strl".to_vec(), riff(b"strh", &[b"vids".to_vec(), vec![0; 52]].concat()), riff(b"strf", &bitmap)].concat();
        let hdrl = [b"hdrl".to_vec(), riff(b"avih", &[0; 56]), riff(b"LIST", &strl)].concat();
        let format = read_bytes(&[b"RIFF\0\0\0\0AVI ".to_vec(), riff(b"LIST", &hdrl)].concat()).expect("a format");
        assert_eq!((format.name(), format.size), (String::from("Motion JPEG video in AVI"), Some([320, 240])), "a picture stored top down");
    }

    #[test]
    fn an_flv_names_its_codecs_from_its_first_tags() {
        let mut file = b"FLV\x01\x05\0\0\0\x09\0\0\0\0".to_vec();
        file.extend_from_slice(&[9, 0, 0, 1, 0, 0, 0, 0, 0, 0, 0, 0x22, 0, 0, 0, 12]);
        file.extend_from_slice(&[8, 0, 0, 1, 0, 0, 0, 0, 0, 0, 0, 0x6A, 0, 0, 0, 12]);
        assert_eq!(named(&file), "Sorenson Spark (H.263) video and Nellymoser audio in Flash Video (FLV)");
    }

    /// A transport stream packet starting a section on `pid`.
    fn packet(pid: u16, section: &[u8]) -> Vec<u8> {
        let [hi, lo] = pid.to_be_bytes();
        let mut out = vec![0x47, 0x40 | hi, lo, 0x10, 0x00];
        out.extend_from_slice(section);
        out.resize(188, 0xFF);
        out
    }

    #[test]
    fn a_transport_stream_names_the_codecs_its_program_map_gives() {
        let pat = packet(0, &[0x00, 0xB0, 0x0D, 0x00, 0x01, 0xC1, 0x00, 0x00, 0x00, 0x01, 0xF0, 0x00, 0, 0, 0, 0]);
        let pmt = packet(0x1000, &[0x02, 0xB0, 0x17, 0x00, 0x01, 0xC1, 0x00, 0x00, 0xE1, 0x00, 0xF0, 0x00, 0x1B, 0xE1, 0x00, 0xF0, 0x00, 0x0F, 0xE1, 0x01, 0xF0, 0x00, 0, 0, 0, 0]);
        let mut null = vec![0x47, 0x1F, 0xFF, 0x10];
        null.resize(188, 0xFF);
        assert_eq!(named(&[pat, pmt, null].concat()), "H.264 video and AAC audio in MPEG-TS");
    }

    #[test]
    fn a_header_past_the_first_part_is_found_in_a_wider_read() {
        let size = 100_000usize;
        let syncsafe = [(size >> 21) as u8 & 0x7F, (size >> 14) as u8 & 0x7F, (size >> 7) as u8 & 0x7F, size as u8 & 0x7F];
        let mut file = b"ID3\x04\0\0".to_vec();
        file.extend_from_slice(&syncsafe);
        file.resize(10 + size, 0);
        file.extend_from_slice(b"\xFF\xFB\x90\x64");
        assert_eq!(named(&file), "MP3 audio", "an MP3 behind a tag of 100 KB");
        let mut null = vec![0x47, 0x1F, 0xFF, 0x10];
        null.resize(188, 0xFF);
        let pat = packet(0, &[0x00, 0xB0, 0x0D, 0x00, 0x01, 0xC1, 0x00, 0x00, 0x00, 0x01, 0xF0, 0x00, 0, 0, 0, 0]);
        let pmt = packet(0x1000, &[0x02, 0xB0, 0x17, 0x00, 0x01, 0xC1, 0x00, 0x00, 0xE1, 0x00, 0xF0, 0x00, 0x1B, 0xE1, 0x00, 0xF0, 0x00, 0x0F, 0xE1, 0x01, 0xF0, 0x00, 0, 0, 0, 0]);
        let cut = [null.repeat(400), pat, pmt].concat();
        assert_eq!(named(&cut), "H.264 video and AAC audio in MPEG-TS", "a stream whose first program map comes after 64 KiB");
    }

    #[test]
    fn bytes_that_start_no_clip_are_refused_naming_them() {
        let refused = read_bytes(b"\x89PNG\r\n\x1a\n").expect_err("a picture is no clip");
        assert_eq!(refused, "the file begins 89 50 4e 47 0d 0a 1a 0a, which starts no clip this reader knows");
        assert_eq!(read_bytes(b"").expect_err("nothing"), "the file is empty");
        let missing = read(Path::new("no-such-clip.mp4")).expect_err("no file");
        assert!(missing.starts_with("the file cannot be opened: "), "{missing}");
        assert_eq!(Format::new("MIDI").name(), "MIDI holding no picture or sound this reader names");
    }

    #[test]
    fn an_edit_list_gives_where_its_track_starts_its_media_less_its_empty_edits() {
        let list = |wide: bool, entries: &[(u64, i64)]| {
            let mut b = vec![u8::from(wide), 0, 0, 0];
            b.extend_from_slice(&(entries.len() as u32).to_be_bytes());
            for (duration, time) in entries {
                if wide {
                    b.extend_from_slice(&duration.to_be_bytes());
                    b.extend_from_slice(&time.to_be_bytes());
                } else {
                    b.extend_from_slice(&(*duration as u32).to_be_bytes());
                    b.extend_from_slice(&(*time as i32).to_be_bytes());
                }
                b.extend_from_slice(&[0, 1, 0, 0]);
            }
            b
        };
        let close = |a: f64, b: f64| (a - b).abs() < 1e-9;
        assert!(close(track_offset(&list(false, &[(2000, 2048)]), Some(1000), Some(15360)), 2048.0 / 15360.0), "two frames of B-frame delay at 15 a second");
        assert!(close(track_offset(&list(true, &[(2000, 1024)]), Some(1000), Some(48000)), 1024.0 / 48000.0), "the sound's priming, in a wide list");
        assert!(close(track_offset(&list(false, &[(500, -1), (2000, 0)]), Some(1000), Some(48000)), -0.5), "an empty edit of half a second first");
        assert!(close(track_offset(&[], Some(1000), Some(48000)), 0.0), "no list");
        let edits = Edits { video: Some(0.1333), audio: Some(0.0213) };
        assert!(close(edits.clock(), 0.1333) && close(edits.sound_shift(), 0.112));
        let sound = Edits { video: None, audio: Some(0.0213) };
        assert!(close(sound.clock(), 0.0213) && close(sound.sound_shift(), 0.0));
        assert_eq!(edits_from(&mut &b"\x1aE\xdf\xa3\0\0\0\0"[..]).expect("a Matroska head"), Edits::default(), "no ISO file, no edits");
    }
}
