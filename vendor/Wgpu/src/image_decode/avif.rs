//! .avif files: their pixels through avif-decode, their orientation and
//! color from their own irot, imir and colr boxes, an ICC profile or CICP
//! values, and the light of a file whose CICP transfer is PQ or HLG as
//! linear light in sRGB's primaries.

use std::borrow::Cow;
use std::fmt::Display;
use std::path::Path;

use image::metadata::Orientation;
use image::{DynamicImage, ImageBuffer};
use moxcms::{CicpColorPrimaries, CicpProfile, MatrixCoefficients, TransferCharacteristics};
use pwrs::prelude::*;

use super::{unreadable, Decoded, Profile};
use crate::error::invalid;

/// The .avif file at `path`, holding `bytes`, through avif-decode, with its
/// orientation and color from its irot, imir and colr boxes.
pub(super) fn decode(path: &Path, bytes: &[u8]) -> PsResult<Decoded> {
    let sized = last_box_sized(bytes).map_err(|e| unreadable(path, &e))?;
    let decoded = avif_decode::Decoder::from_avif(&sized).and_then(avif_decode::Decoder::to_image).map_err(|e| unreadable(path, &e))?;
    let image = avif_image(path, decoded)?;
    let meta = avif_boxes(bytes);
    let orientation = meta.as_ref().map(|m| m.orientation).map_err(Clone::clone);
    let profile = meta.map(|m| m.profile);
    Ok(Decoded { image, orientation, profile, cmyk: None, gray_floats: false, warnings: Vec::new(), said: Vec::new() })
}

/// `bytes` with a last top-level box that states size 0 given its size, in
/// a copy. ISO 14496-12 lets the last box run to the end of the file that
/// way, and .avif files Photoshop writes can end with one, which avif-parse
/// refuses. A file whose boxes all state their sizes comes back as it is,
/// as does one whose boxes do not read, for avif-decode to report.
fn last_box_sized(bytes: &[u8]) -> Result<Cow<'_, [u8]>, String> {
    let mut at = 0;
    while let Some(&[a, b, c, d]) = bytes.get(at..at + 4) {
        let span = match u32::from_be_bytes([a, b, c, d]) {
            0 => {
                let rest = bytes.len() - at;
                let stated = u32::try_from(rest).map_err(|e| format!("its last box runs to the end of the file, {rest} bytes, too long to be given a 32-bit size: {e}"))?;
                let mut copy = bytes.to_vec();
                copy[at..at + 4].copy_from_slice(&stated.to_be_bytes());
                return Ok(Cow::Owned(copy));
            }
            // A 64-bit size follows the box's type.
            1 => match bytes.get(at + 8..at + 16) {
                Some(&[a, b, c, d, e, f, g, h]) => u64::from_be_bytes([a, b, c, d, e, f, g, h]),
                _ => break,
            },
            n => u64::from(n),
        };
        match (at as u64).checked_add(span) {
            Some(next) if span >= 8 && next <= bytes.len() as u64 => at = next as usize,
            _ => break,
        }
    }
    Ok(Cow::Borrowed(bytes))
}

/// The pixels avif-decode gives for the file at `path` as an image of the
/// same channels and depth: 8-bit or 16-bit, gray or color, with or without
/// alpha.
fn avif_image(path: &Path, decoded: avif_decode::Image) -> PsResult<DynamicImage> {
    use DynamicImage as D;
    Ok(match decoded {
        avif_decode::Image::Rgb8(i) => {
            let (pixels, w, h) = i.into_contiguous_buf();
            D::ImageRgb8(sized(path, w, h, pixels.iter().flat_map(|p| [p.r, p.g, p.b]).collect())?)
        }
        avif_decode::Image::Rgba8(i) => {
            let (pixels, w, h) = i.into_contiguous_buf();
            D::ImageRgba8(sized(path, w, h, pixels.iter().flat_map(|p| [p.r, p.g, p.b, p.a]).collect())?)
        }
        avif_decode::Image::Rgb16(i) => {
            let (pixels, w, h) = i.into_contiguous_buf();
            D::ImageRgb16(sized(path, w, h, pixels.iter().flat_map(|p| [p.r, p.g, p.b]).collect())?)
        }
        avif_decode::Image::Rgba16(i) => {
            let (pixels, w, h) = i.into_contiguous_buf();
            D::ImageRgba16(sized(path, w, h, pixels.iter().flat_map(|p| [p.r, p.g, p.b, p.a]).collect())?)
        }
        avif_decode::Image::Gray8(i) => {
            let (pixels, w, h) = i.into_contiguous_buf();
            D::ImageLuma8(sized(path, w, h, pixels.iter().map(|p| p.value()).collect())?)
        }
        avif_decode::Image::Gray16(i) => {
            let (pixels, w, h) = i.into_contiguous_buf();
            D::ImageLuma16(sized(path, w, h, pixels.iter().map(|p| p.value()).collect())?)
        }
    })
}

/// `samples`, of the file at `path`, as a `w` by `h` image of the pixel
/// type the caller names.
fn sized<P: image::Pixel>(path: &Path, w: usize, h: usize, samples: Vec<P::Subpixel>) -> PsResult<ImageBuffer<P, Vec<P::Subpixel>>> {
    let unfit = |why: &dyn Display| invalid("WgpuImage", format!("{} cannot be read as an image: {w}x{h} pixels: {why}", path.display()));
    let width = u32::try_from(w).map_err(|e| unfit(&e))?;
    let height = u32::try_from(h).map_err(|e| unfit(&e))?;
    ImageBuffer::from_raw(width, height, samples).ok_or_else(|| unfit(&"the samples do not fill that size"))
}

/// The big-endian 16-bit number at `at` in `bytes`, part of `what`.
fn be16(bytes: &[u8], at: usize, what: &str) -> Result<u16, String> {
    match bytes.get(at..at + 2) {
        Some(&[a, b]) => Ok(u16::from_be_bytes([a, b])),
        _ => Err(format!("{what} is cut short")),
    }
}

/// The big-endian 32-bit number at `at` in `bytes`, part of `what`.
fn be32(bytes: &[u8], at: usize, what: &str) -> Result<u32, String> {
    match bytes.get(at..at + 4) {
        Some(&[a, b, c, d]) => Ok(u32::from_be_bytes([a, b, c, d])),
        _ => Err(format!("{what} is cut short")),
    }
}

/// The big-endian 64-bit number at `at` in `bytes`, part of `what`.
fn be64(bytes: &[u8], at: usize, what: &str) -> Result<u64, String> {
    match bytes.get(at..at + 8) {
        Some(&[a, b, c, d, e, f, g, h]) => Ok(u64::from_be_bytes([a, b, c, d, e, f, g, h])),
        _ => Err(format!("{what} is cut short")),
    }
}

/// What an .avif file's boxes say about its primary image.
struct AvifMeta {
    orientation: Orientation,
    profile: Option<Profile>,
}

/// An ISOBMFF box: its type, and its payload, after the header.
struct Bmff<'a> {
    kind: [u8; 4],
    payload: &'a [u8],
}

/// The boxes laid end to end in `bytes`.
fn boxes(mut bytes: &[u8]) -> Result<Vec<Bmff<'_>>, String> {
    let mut found = Vec::new();
    while !bytes.is_empty() {
        let size32 = be32(bytes, 0, "a box")?;
        let kind = match bytes.get(4..8) {
            Some(&[a, b, c, d]) => [a, b, c, d],
            _ => return Err("a box is cut short".to_string()),
        };
        let fits = |n: u64| usize::try_from(n).map_err(|e| format!("its {} box's size, {n}, does not fit in memory: {e}", String::from_utf8_lossy(&kind)));
        let (header, size) = match size32 {
            0 => (8, bytes.len()),
            1 => (16, fits(be64(bytes, 8, "a box")?)?),
            n => (8, fits(u64::from(n))?),
        };
        if size < header || size > bytes.len() {
            return Err(format!("its {} box claims {size} bytes where {} remain", String::from_utf8_lossy(&kind), bytes.len()));
        }
        found.push(Bmff { kind, payload: &bytes[header..size] });
        bytes = &bytes[size..];
    }
    Ok(found)
}

/// The orientation and profile an .avif file's meta box gives its primary
/// item: the irot and imir properties ipma associates with it, applied in
/// that order as MIAF has them, and its colr property, an ICC profile (prof
/// or rICC) or CICP values (nclx). A file with no meta box, or a primary item
/// with none of these, records neither.
fn avif_boxes(bytes: &[u8]) -> Result<AvifMeta, String> {
    let none = AvifMeta { orientation: Orientation::NoTransforms, profile: None };
    let Some(meta) = boxes(bytes)?.into_iter().find(|b| &b.kind == b"meta") else {
        return Ok(none);
    };
    // meta is a full box: its version and flags come first.
    let inner = boxes(meta.payload.get(4..).ok_or("its meta box is cut short")?)?;
    let primary = match inner.iter().find(|b| &b.kind == b"pitm") {
        Some(pitm) => match pitm.payload.first() {
            Some(0) => u32::from(be16(pitm.payload, 4, "its pitm box")?),
            Some(_) => be32(pitm.payload, 4, "its pitm box")?,
            None => return Err("its pitm box is empty".to_string()),
        },
        None => return Ok(none),
    };
    let Some(iprp) = inner.iter().find(|b| &b.kind == b"iprp") else {
        return Ok(none);
    };
    let properties = boxes(iprp.payload)?;
    let ipco = match properties.iter().find(|b| &b.kind == b"ipco") {
        Some(ipco) => boxes(ipco.payload)?,
        None => return Ok(none),
    };
    let mut associated = Vec::new();
    for ipma in properties.iter().filter(|b| &b.kind == b"ipma") {
        associated.extend(ipma_for(ipma.payload, primary)?);
    }
    let mut quarter_turns_clockwise = 0u8;
    let mut mirrored = false;
    let mut profile = None;
    for index in associated {
        let Some(property) = index.checked_sub(1).and_then(|i| ipco.get(usize::from(i))) else {
            return Err(format!("its ipma box names property {index}, of {} in ipco", ipco.len()));
        };
        match &property.kind {
            b"irot" => {
                let angle = property.payload.first().ok_or("its irot box is empty")? & 3;
                // irot turns counterclockwise; after a mirror a turn goes the other way.
                let clockwise = (4 - angle) % 4;
                quarter_turns_clockwise = (quarter_turns_clockwise + if mirrored { 4 - clockwise } else { clockwise }) % 4;
            }
            b"imir" => {
                let axis = property.payload.first().ok_or("its imir box is empty")? & 1;
                // A mirror top to bottom is one left to right and half a turn.
                if axis == 1 {
                    quarter_turns_clockwise = (quarter_turns_clockwise + 2) % 4;
                }
                mirrored = !mirrored;
            }
            b"colr" if profile.is_none() => profile = colr(property.payload)?,
            _ => {}
        }
    }
    let orientation = match (quarter_turns_clockwise, mirrored) {
        (0, false) => Orientation::NoTransforms,
        (1, false) => Orientation::Rotate90,
        (2, false) => Orientation::Rotate180,
        (3, false) => Orientation::Rotate270,
        (0, true) => Orientation::FlipHorizontal,
        (1, true) => Orientation::Rotate90FlipH,
        (2, true) => Orientation::FlipVertical,
        _ => Orientation::Rotate270FlipH,
    };
    Ok(AvifMeta { orientation, profile })
}

/// The property indexes an ipma box associates with `item`, in its order.
fn ipma_for(payload: &[u8], item: u32) -> Result<Vec<u16>, String> {
    const WHAT: &str = "its ipma box";
    let version = *payload.first().ok_or("its ipma box is empty")?;
    let wide_indexes = be32(payload, 0, WHAT)? & 1 == 1;
    let count = be32(payload, 4, WHAT)?;
    let mut at = 8;
    let mut found = Vec::new();
    for _ in 0..count {
        let id = if version < 1 {
            at += 2;
            u32::from(be16(payload, at - 2, WHAT)?)
        } else {
            at += 4;
            be32(payload, at - 4, WHAT)?
        };
        let associations = *payload.get(at).ok_or("its ipma box is cut short")?;
        at += 1;
        for _ in 0..associations {
            let index = if wide_indexes {
                at += 2;
                be16(payload, at - 2, WHAT)? & 0x7FFF
            } else {
                at += 1;
                u16::from(*payload.get(at - 1).ok_or("its ipma box is cut short")? & 0x7F)
            };
            if id == item {
                found.push(index);
            }
        }
    }
    Ok(found)
}

/// The color a colr box gives: an ICC profile for prof and rICC, CICP
/// values for nclx, and none for a type it does not name.
fn colr(payload: &[u8]) -> Result<Option<Profile>, String> {
    const WHAT: &str = "its colr box";
    match payload.get(0..4) {
        Some(b"prof" | b"rICC") => Ok(Some(Profile::Icc(payload[4..].to_vec()))),
        Some(b"nclx") => {
            let code = |at: usize, what: &str| -> Result<u8, String> {
                let value = be16(payload, at, WHAT)?;
                u8::try_from(value).map_err(|e| format!("its colr box's {what}, {value}, is not a CICP code: {e}"))
            };
            let primaries = CicpColorPrimaries::try_from(code(4, "primaries")?).map_err(|e| format!("its colr box's primaries are not ones moxcms knows: {e}"))?;
            let transfer = TransferCharacteristics::try_from(code(6, "transfer")?).map_err(|e| format!("its colr box's transfer is not one moxcms knows: {e}"))?;
            let matrix = MatrixCoefficients::try_from(code(8, "matrix")?).map_err(|e| format!("its colr box's matrix is not one moxcms knows: {e}"))?;
            let full_range = payload.get(10).ok_or("its colr box is cut short")? & 0x80 != 0;
            Ok(Some(Profile::Cicp(CicpProfile { color_primaries: primaries, transfer_characteristics: transfer, matrix_coefficients: matrix, full_range })))
        }
        Some(_) => Ok(None),
        None => Err(format!("{WHAT} is cut short")),
    }
}

#[cfg(test)]
mod tests {
    use super::super::cicp::hdr_linear;
    use super::*;

    /// An .avif file's ftyp and meta boxes, with `properties` in ipco, every
    /// one associated with item 1, the primary item, in the order given.
    fn avif_with(properties: &[(&[u8; 4], Vec<u8>)]) -> Vec<u8> {
        fn boxed(kind: &[u8; 4], payload: &[u8]) -> Vec<u8> {
            let mut out = u32::try_from(payload.len() + 8).expect("a small box").to_be_bytes().to_vec();
            out.extend_from_slice(kind);
            out.extend_from_slice(payload);
            out
        }
        let ipco: Vec<u8> = properties.iter().flat_map(|(kind, payload)| boxed(kind, payload)).collect();
        let mut ipma = vec![0, 0, 0, 0, 0, 0, 0, 1, 0, 1, u8::try_from(properties.len()).expect("a few properties")];
        ipma.extend((1..=properties.len()).map(|i| u8::try_from(i).expect("a few properties")));
        let mut iprp = boxed(b"ipco", &ipco);
        iprp.extend(boxed(b"ipma", &ipma));
        let mut meta = vec![0, 0, 0, 0];
        meta.extend(boxed(b"pitm", &[0, 0, 0, 0, 0, 1]));
        meta.extend(boxed(b"iprp", &iprp));
        let mut file = boxed(b"ftyp", b"avif\0\0\0\0avifmif1");
        file.extend(boxed(b"meta", &meta));
        file
    }

    /// Properties of an .avif file's primary item: each box's type and
    /// payload.
    type Properties<'a> = &'a [(&'a [u8; 4], Vec<u8>)];

    #[test]
    fn avif_rotation_and_mirror_give_the_orientation_they_compose_to() {
        let cases: [(Properties, Orientation); 7] = [
            (&[], Orientation::NoTransforms),
            (&[(b"irot", vec![1])], Orientation::Rotate270),
            (&[(b"irot", vec![2])], Orientation::Rotate180),
            (&[(b"irot", vec![3])], Orientation::Rotate90),
            (&[(b"imir", vec![0])], Orientation::FlipHorizontal),
            (&[(b"imir", vec![1])], Orientation::FlipVertical),
            (&[(b"irot", vec![1]), (b"imir", vec![0])], Orientation::Rotate270FlipH),
        ];
        for (properties, expected) in cases {
            let found = avif_boxes(&avif_with(properties)).expect("the boxes read");
            let named: Vec<String> = properties.iter().map(|(kind, payload)| format!("{}{payload:?}", String::from_utf8_lossy(&kind[..]))).collect();
            assert_eq!(found.orientation, expected, "{named:?}");
        }
    }

    #[test]
    fn avif_colr_gives_an_icc_profile_or_cicp_values() {
        let icc = avif_boxes(&avif_with(&[(b"colr", [b"prof".to_vec(), vec![1, 2, 3]].concat())])).expect("the boxes read");
        assert!(matches!(icc.profile, Some(Profile::Icc(ref bytes)) if bytes == &[1, 2, 3]));
        let nclx = avif_boxes(&avif_with(&[(b"colr", [b"nclx".to_vec(), vec![0, 12, 0, 13, 0, 6, 0x80]].concat())])).expect("the boxes read");
        match nclx.profile {
            Some(Profile::Cicp(c)) => {
                assert_eq!(c.color_primaries, CicpColorPrimaries::Smpte432);
                assert_eq!(c.transfer_characteristics, TransferCharacteristics::Srgb);
                assert!(c.full_range);
            }
            _ => panic!("nclx gave no CICP values"),
        }
    }

    #[test]
    fn a_box_that_claims_more_than_the_file_holds_is_refused() {
        let mut file = avif_with(&[(b"irot", vec![1])]);
        file.truncate(file.len() - 3);
        assert!(avif_boxes(&file).is_err());
    }

    /// Each kind of pixels avif-decode gives becomes an image of the same
    /// channels and depth with every sample where it was, rows given with a
    /// stride wider than the image included.
    #[test]
    fn makes_an_image_of_each_kind_of_avif_pixels() {
        use avif_decode::Image as A;
        use imgref::ImgVec;
        use rgb::{Gray, Rgb, Rgba};
        let path = Path::new("pixels.avif");
        let rgb8 = avif_image(path, A::Rgb8(ImgVec::new(vec![Rgb::new(1u8, 2, 3), Rgb::new(4, 5, 6)], 2, 1))).expect("an image");
        assert_eq!(rgb8.as_rgb8().expect("8-bit color").as_raw(), &vec![1, 2, 3, 4, 5, 6]);
        let rgba8 = avif_image(path, A::Rgba8(ImgVec::new(vec![Rgba::new(1u8, 2, 3, 4), Rgba::new(5, 6, 7, 8)], 2, 1))).expect("an image");
        assert_eq!(rgba8.as_rgba8().expect("8-bit color and alpha").as_raw(), &vec![1, 2, 3, 4, 5, 6, 7, 8]);
        let rgb16 = avif_image(path, A::Rgb16(ImgVec::new(vec![Rgb::new(1000u16, 2000, 3000), Rgb::new(4000, 5000, 65535)], 2, 1))).expect("an image");
        assert_eq!(rgb16.as_rgb16().expect("16-bit color").as_raw(), &vec![1000, 2000, 3000, 4000, 5000, 65535]);
        let rgba16 = avif_image(path, A::Rgba16(ImgVec::new(vec![Rgba::new(1u16, 2, 3, 65535), Rgba::new(5, 6, 7, 0)], 2, 1))).expect("an image");
        assert_eq!(rgba16.as_rgba16().expect("16-bit color and alpha").as_raw(), &vec![1, 2, 3, 65535, 5, 6, 7, 0]);
        let gray16 = avif_image(path, A::Gray16(ImgVec::new(vec![Gray::new(300u16), Gray::new(65000)], 2, 1))).expect("an image");
        assert_eq!(gray16.as_luma16().expect("16-bit gray").as_raw(), &vec![300, 65000]);
        let strided = ImgVec::new_stride(vec![Gray::new(1u8), Gray::new(2), Gray::new(99), Gray::new(3), Gray::new(4), Gray::new(99)], 2, 2, 3);
        let gray8 = avif_image(path, A::Gray8(strided)).expect("an image");
        assert_eq!((gray8.width(), gray8.height()), (2, 2));
        assert_eq!(gray8.as_luma8().expect("8-bit gray").as_raw(), &vec![1, 2, 3, 4]);
    }

    /// A picture the image crate's encoder writes as .avif, as
    /// Export-WgpuImage writes one, comes back through avif-decode at its
    /// size and color type, color with alpha and color without, every sample
    /// within 8 steps of what went in.
    #[test]
    fn decodes_the_avif_the_image_crate_writes() {
        let (w, h) = (16u32, 8u32);
        let rgba = image::RgbaImage::from_fn(w, h, |x, y| image::Rgba([(x * 15) as u8, (y * 30) as u8, 120, if x < 8 { 255 } else { 100 }]));
        let opaque = DynamicImage::ImageRgba8(rgba.clone()).to_rgb8();
        for source in [DynamicImage::ImageRgba8(rgba), DynamicImage::ImageRgb8(opaque)] {
            let mut bytes = Vec::new();
            source.write_with_encoder(image::codecs::avif::AvifEncoder::new_with_speed_quality(&mut bytes, 10, 100)).expect("encoded");
            let decoded = avif_decode::Decoder::from_avif(&bytes).and_then(avif_decode::Decoder::to_image).expect("decoded");
            let image = avif_image(Path::new("encoded.avif"), decoded).expect("an image");
            assert_eq!(image.color(), source.color());
            assert_eq!((image.width(), image.height()), (w, h));
            let largest = image.to_rgba8().as_raw().iter().zip(source.to_rgba8().as_raw()).map(|(a, b)| a.abs_diff(*b)).max().expect("samples");
            assert!(largest <= 8, "{:?}: a sample moved {largest} steps", source.color());
        }
    }

    /// The .avif the image crate's encoder writes, its last box, the mdat,
    /// made to state size 0 as Photoshop's files do: avif-decode refuses it
    /// as it is, and given the box's size it is the file as written, byte
    /// for byte, with the same pixels.
    #[test]
    fn reads_an_avif_whose_last_box_runs_to_the_end_of_the_file() {
        let rgba = image::RgbaImage::from_fn(16, 8, |x, y| image::Rgba([(x * 15) as u8, (y * 30) as u8, 120, 255]));
        let mut written = Vec::new();
        DynamicImage::ImageRgba8(rgba).write_with_encoder(image::codecs::avif::AvifEncoder::new_with_speed_quality(&mut written, 10, 100)).expect("encoded");
        let (mut at, mut last) = (0, 0);
        while at < written.len() {
            let size = u32::from_be_bytes(written[at..at + 4].try_into().expect("four bytes")) as usize;
            assert!(size >= 8, "a box at {at} states size {size}");
            (last, at) = (at, at + size);
        }
        assert_eq!(at, written.len(), "the boxes fill the file");
        let mut to_end = written.clone();
        to_end[last..last + 4].copy_from_slice(&[0, 0, 0, 0]);
        assert!(avif_decode::Decoder::from_avif(&to_end).is_err(), "avif-decode now reads a last box of size 0 itself");
        let unchanged = last_box_sized(&written).expect("the boxes read");
        assert!(matches!(unchanged, Cow::Borrowed(_)), "a file whose boxes state their sizes is left as it is");
        let sized = last_box_sized(&to_end).expect("sized");
        assert_eq!(&sized[..], &written[..]);
        let pixels = |bytes: &[u8]| {
            let decoded = avif_decode::Decoder::from_avif(bytes).and_then(avif_decode::Decoder::to_image).expect("decoded");
            avif_image(Path::new("encoded.avif"), decoded).expect("an image").to_rgba8().into_raw()
        };
        assert_eq!(pixels(&sized), pixels(&written));
    }

    /// The most a sample of the HDR test's light may be from the
    /// reference's, relative to it.
    const HDR_TOLERANCE: f32 = 1e-5;

    /// PQ and HLG signals in BT.2020 primaries load as the reference's light
    /// in sRGB's primaries: each sample within HDR_TOLERANCE of it, and
    /// exactly 0 where it is 0, light in colors sRGB lacks clipped there.
    #[test]
    fn converts_pq_and_hlg_light_as_the_reference_does() {
        use crate::hdr_reference as h;
        let count = u32::try_from(h::HDR_PIXELS.len() / 3).expect("a few pixels");
        let image = DynamicImage::ImageRgb16(ImageBuffer::from_raw(count, 1, h::HDR_PIXELS.to_vec()).expect("the pixels"));
        for (transfer, expected) in [(TransferCharacteristics::Smpte2084, &h::HDR_PQ), (TransferCharacteristics::Hlg, &h::HDR_HLG)] {
            let cicp = CicpProfile { color_primaries: CicpColorPrimaries::Bt2020, transfer_characteristics: transfer, matrix_coefficients: MatrixCoefficients::Bt709, full_range: true };
            let (linear, _) = hdr_linear(&image, &cicp).expect("converted");
            let got = linear.as_rgb32f().expect("float color").as_raw();
            assert_eq!(got.len(), expected.len(), "{transfer:?}: the samples differ in count");
            for (i, (&a, &b)) in got.iter().zip(expected).enumerate() {
                if b == 0.0 {
                    assert_eq!(a, 0.0, "{transfer:?}, sample {i}: {a} where the reference gives 0");
                } else {
                    let relative = ((a - b) / b).abs();
                    assert!(relative <= HDR_TOLERANCE, "{transfer:?}, sample {i}: {a} is {relative:e} from the reference's {b}");
                }
            }
        }
    }
}
