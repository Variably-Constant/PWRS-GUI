//! JPEG XL files: their first keyframe through jxl-oxide, in the color the
//! file renders in, an HDR file's by its CICP values, and the orientation
//! its codestream records.

use std::io::Cursor;
use std::path::Path;

use image::metadata::Orientation;
use image::{DynamicImage, ImageBuffer, Luma, LumaA, Rgb, Rgba};
use jxl_oxide::image::BitDepth;
use jxl_oxide::JxlImage;
use moxcms::{CicpColorPrimaries, CicpProfile, MatrixCoefficients, TransferCharacteristics};
use pwrs::prelude::*;

use super::{unreadable, Decoded, Profile};

/// The JPEG XL file at `path`, holding `bytes`: its first keyframe as
/// stored, before its orientation turns it, and the profile it renders in.
pub(super) fn decode(path: &Path, bytes: &[u8]) -> PsResult<Decoded> {
    let fail = |e: &dyn std::fmt::Display| unreadable(path, e);
    let image = JxlImage::builder().read(Cursor::new(bytes)).map_err(|e| fail(&e))?;
    if image.pixel_format().has_black() {
        return Err(fail(&"it is a CMYK JPEG XL file, which the module does not read"));
    }
    let render = image.render_frame(0).map_err(|e| fail(&e))?;
    let orientation = Orientation::from_exif(u8::try_from(render.orientation()).unwrap_or(0)).ok_or_else(|| fail(&format!("its orientation {} is not one EXIF gives", render.orientation())))?;
    let depth = &image.image_header().metadata.bit_depth;
    let float = matches!(depth, BitDepth::FloatSample { .. } | BitDepth::IntegerSample { bits_per_sample: 17.. });
    let mut stream = render.stream();
    let (width, height, channels) = (stream.width(), stream.height(), stream.channels());
    let samples = (width as usize) * (height as usize) * (channels as usize);
    let short = || fail(&"its samples do not fill the picture");
    let (mut turned, gray_floats) = if float {
        let mut buf = vec![0f32; samples];
        if stream.write_to_buffer(&mut buf) != samples {
            return Err(short());
        }
        floats(width, height, channels, buf).map_err(|e| fail(&e))?
    } else if depth.bits_per_sample() > 8 {
        let mut buf = vec![0u16; samples];
        if stream.write_to_buffer(&mut buf) != samples {
            return Err(short());
        }
        (sixteen(width, height, channels, buf).map_err(|e| fail(&e))?, false)
    } else {
        let mut buf = vec![0u8; samples];
        if stream.write_to_buffer(&mut buf) != samples {
            return Err(short());
        }
        (eight(width, height, channels, buf).map_err(|e| fail(&e))?, false)
    };
    // jxl-oxide hands the pixels over turned; the import turns them itself.
    turned.apply_orientation(unturning(orientation));
    let profile = match (image.hdr_type(), image.rendered_cicp()) {
        (Some(_), Some(code)) => cicp(code).map(Profile::Cicp),
        _ => Ok(Profile::Icc(image.rendered_icc())),
    };
    Ok(Decoded { image: turned, orientation: Ok(orientation), profile: profile.map(Some), cmyk: None, gray_floats, warnings: Vec::new(), said: Vec::new() })
}

/// The orientation that undoes `orientation`: a quarter turn undoes the
/// other way, and every other orientation undoes itself.
fn unturning(orientation: Orientation) -> Orientation {
    match orientation {
        Orientation::Rotate90 => Orientation::Rotate270,
        Orientation::Rotate270 => Orientation::Rotate90,
        other => other,
    }
}

/// 8-bit samples, interleaved by `channels`, as the image of that layout.
fn eight(width: u32, height: u32, channels: u32, buf: Vec<u8>) -> Result<DynamicImage, String> {
    let short = || "its samples do not fill the picture".to_string();
    Ok(match channels {
        1 => DynamicImage::ImageLuma8(ImageBuffer::<Luma<u8>, _>::from_raw(width, height, buf).ok_or_else(short)?),
        2 => DynamicImage::ImageLumaA8(ImageBuffer::<LumaA<u8>, _>::from_raw(width, height, buf).ok_or_else(short)?),
        3 => DynamicImage::ImageRgb8(ImageBuffer::<Rgb<u8>, _>::from_raw(width, height, buf).ok_or_else(short)?),
        4 => DynamicImage::ImageRgba8(ImageBuffer::<Rgba<u8>, _>::from_raw(width, height, buf).ok_or_else(short)?),
        other => return Err(format!("it renders {other} channels")),
    })
}

/// 16-bit samples, interleaved by `channels`, as the image of that layout.
fn sixteen(width: u32, height: u32, channels: u32, buf: Vec<u16>) -> Result<DynamicImage, String> {
    let short = || "its samples do not fill the picture".to_string();
    Ok(match channels {
        1 => DynamicImage::ImageLuma16(ImageBuffer::<Luma<u16>, _>::from_raw(width, height, buf).ok_or_else(short)?),
        2 => DynamicImage::ImageLumaA16(ImageBuffer::<LumaA<u16>, _>::from_raw(width, height, buf).ok_or_else(short)?),
        3 => DynamicImage::ImageRgb16(ImageBuffer::<Rgb<u16>, _>::from_raw(width, height, buf).ok_or_else(short)?),
        4 => DynamicImage::ImageRgba16(ImageBuffer::<Rgba<u16>, _>::from_raw(width, height, buf).ok_or_else(short)?),
        other => return Err(format!("it renders {other} channels")),
    })
}

/// Float samples, interleaved by `channels`, as RGB or RGBA: a gray image's
/// sample in each of the three, with whether it is one.
fn floats(width: u32, height: u32, channels: u32, buf: Vec<f32>) -> Result<(DynamicImage, bool), String> {
    let short = || "its samples do not fill the picture".to_string();
    Ok(match channels {
        1 => (DynamicImage::ImageRgb32F(ImageBuffer::from_raw(width, height, buf.iter().flat_map(|&v| [v; 3]).collect()).ok_or_else(short)?), true),
        2 => (DynamicImage::ImageRgba32F(ImageBuffer::from_raw(width, height, buf.as_chunks::<2>().0.iter().flat_map(|&[v, a]| [v, v, v, a]).collect()).ok_or_else(short)?), true),
        3 => (DynamicImage::ImageRgb32F(ImageBuffer::from_raw(width, height, buf).ok_or_else(short)?), false),
        4 => (DynamicImage::ImageRgba32F(ImageBuffer::from_raw(width, height, buf).ok_or_else(short)?), false),
        other => return Err(format!("it renders {other} channels")),
    })
}

/// A rendered encoding's CICP code, primaries, transfer, matrix and range,
/// as moxcms holds it, or why it cannot be.
fn cicp([primaries, transfer, matrix, range]: [u8; 4]) -> Result<CicpProfile, String> {
    Ok(CicpProfile {
        color_primaries: CicpColorPrimaries::try_from(primaries).map_err(|e| format!("its CICP primaries are not ones moxcms knows: {e}"))?,
        transfer_characteristics: TransferCharacteristics::try_from(transfer).map_err(|e| format!("its CICP transfer is not one moxcms knows: {e}"))?,
        matrix_coefficients: MatrixCoefficients::try_from(matrix).map_err(|e| format!("its CICP matrix is not one moxcms knows: {e}"))?,
        full_range: range != 0,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::jxl_reference as r;

    fn decoded(bytes: &[u8]) -> Decoded {
        match decode(Path::new("test.jxl"), bytes) {
            Ok(decoded) => decoded,
            Err(e) => panic!("the file decodes: {e:?}"),
        }
    }

    /// Files libjxl encoded losslessly come back as the samples they were
    /// made from, in their own layout and depth, as stored, with the ICC
    /// profile they render in.
    #[test]
    fn reads_lossless_files_sample_for_sample() {
        let rgb8 = decoded(&r::RGB8_FILE);
        assert_eq!(rgb8.image.as_rgb8().expect("8-bit RGB").as_raw(), &r::RGB8_SAMPLES[..]);
        match &rgb8.profile {
            Ok(Some(Profile::Icc(_))) => {}
            Ok(_) => panic!("the file gives a profile other than an ICC one"),
            Err(why) => panic!("the profile cannot be read: {why}"),
        }
        match &rgb8.orientation {
            Ok(Orientation::NoTransforms) => {}
            Ok(other) => panic!("the file records {other:?}"),
            Err(why) => panic!("the orientation cannot be read: {why}"),
        }
        assert_eq!(decoded(&r::RGBA8_FILE).image.as_rgba8().expect("8-bit RGBA").as_raw(), &r::RGBA8_SAMPLES[..]);
        assert_eq!(decoded(&r::GRAY8_FILE).image.as_luma8().expect("8-bit gray").as_raw(), &r::GRAY8_SAMPLES[..]);
        assert_eq!(decoded(&r::RGB16_FILE).image.as_rgb16().expect("16-bit RGB").as_raw(), &r::RGB16_SAMPLES[..]);
        assert_eq!((rgb8.image.width(), rgb8.image.height()), (r::WIDTH, r::HEIGHT));
    }

    /// A file whose samples are PQ signals keeps them, with the CICP values
    /// the import takes its light to linear light by.
    #[test]
    fn reads_a_pq_file_with_its_cicp_values() {
        let pq = decoded(&r::PQ16_FILE);
        assert_eq!(pq.image.as_rgb16().expect("16-bit RGB").as_raw(), &r::PQ16_SAMPLES[..]);
        let cicp = match pq.profile {
            Ok(Some(Profile::Cicp(cicp))) => cicp,
            Ok(_) => panic!("the file gives no CICP values"),
            Err(why) => panic!("the profile cannot be read: {why}"),
        };
        assert_eq!((cicp.color_primaries, cicp.transfer_characteristics), (CicpColorPrimaries::Bt2020, TransferCharacteristics::Smpte2084));
    }

    /// Turning by an orientation, then by what undoes it, leaves the pixels
    /// as they were, for each of the eight.
    #[test]
    fn undoes_every_orientation() {
        let pixels: Vec<u8> = (0..6).collect();
        let stored = DynamicImage::ImageLuma8(ImageBuffer::from_raw(3, 2, pixels).expect("3 by 2"));
        for code in 1..=8 {
            let orientation = Orientation::from_exif(code).expect("an EXIF orientation");
            let mut image = stored.clone();
            image.apply_orientation(orientation);
            image.apply_orientation(unturning(orientation));
            assert_eq!(image, stored, "orientation {code}");
        }
    }
}
