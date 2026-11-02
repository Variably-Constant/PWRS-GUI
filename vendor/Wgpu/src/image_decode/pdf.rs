//! PDF files: the first page drawn through hayro at the resolution the
//! import asks for, on white or on nothing, at the size and turn its
//! transform plans for the page's size at that resolution.

use std::fmt::Display;
use std::path::Path;
use std::sync::Arc;

use arc_swap::ArcSwap;
use hayro::hayro_interpret::util::TransformExt;
use hayro::hayro_interpret::{InterpreterSettings, InterpreterWarning};
use hayro::hayro_syntax::{DecryptionError, LoadPdfError, Pdf};
use hayro::kurbo::Affine;
use hayro::vello_cpu::color::palette::css::{TRANSPARENT, WHITE};
use hayro::vello_cpu::peniko::ImageAlphaType;
use hayro::vello_cpu::{Pixmap, RasterizerSettings, RenderContext, Resources, TargetInit};
use hayro::{render_into, RenderCache, RenderSettings};
use image::{DynamicImage, RgbaImage};
use pwrs::prelude::*;

use super::{noted, pixels_up, unreadable, Drawing, Reading};
use crate::transform::Transform;

/// Why hayro could not open a PDF.
fn unopened(e: LoadPdfError) -> &'static str {
    match e {
        LoadPdfError::Invalid => "hayro cannot read it as a PDF",
        LoadPdfError::Decryption(DecryptionError::PasswordProtected) => "it is protected by a password",
        LoadPdfError::Decryption(DecryptionError::MissingIDEntry) => "it is encrypted without the file ID its encryption needs",
        LoadPdfError::Decryption(DecryptionError::InvalidEncryption) => "its encryption is not valid",
        LoadPdfError::Decryption(DecryptionError::UnsupportedAlgorithm) => "it is encrypted with an algorithm hayro does not decrypt",
    }
}

/// What hayro left out of a page, as it reports it.
fn described(warning: InterpreterWarning) -> String {
    match warning {
        InterpreterWarning::UnsupportedFont => "text in a font hayro does not draw, a CID font whose encoding is not the identity".to_string(),
        InterpreterWarning::ImageDecodeFailure => "an image hayro could not decode".to_string(),
        InterpreterWarning::UnresolvedAnnotationAppearance => "an annotation whose appearance hayro could not load".to_string(),
        other => format!("what hayro reported as {other:?}"),
    }
}

/// The PDF file at `path`, holding `bytes`, opened with `password` where
/// it needs one: its first page drawn at `reading`'s resolution, on white
/// unless `reading` asks for no background, at the size and turn
/// `transform` plans for the page's size at that resolution.
pub(super) fn draw(path: &Path, bytes: &[u8], reading: Reading, transform: &Transform, password: Option<&str>) -> PsResult<Drawing> {
    let fail = |e: &dyn Display| unreadable(path, e);
    let data = Arc::new(bytes.to_vec());
    let mut warnings = Vec::new();
    let pdf = match (Pdf::new(Arc::clone(&data)), password) {
        (Ok(pdf), None) => pdf,
        (Ok(pdf), Some(_)) => {
            warnings.push("the PDF opens without a password, so -Password changed nothing".to_string());
            pdf
        }
        (Err(LoadPdfError::Decryption(DecryptionError::PasswordProtected)), Some(password)) => match Pdf::new_with_password(data, password) {
            Ok(pdf) => pdf,
            Err(LoadPdfError::Decryption(DecryptionError::PasswordProtected)) => return Err(fail(&"it is protected by a password, and -Password gives another")),
            Err(e) => return Err(fail(&unopened(e))),
        },
        (Err(LoadPdfError::Decryption(DecryptionError::PasswordProtected)), None) => return Err(fail(&"it is protected by a password: give it as a SecureString with -Password")),
        (Err(e), _) => return Err(fail(&unopened(e))),
    };
    let pages = pdf.pages();
    let page = match pages.first() {
        Some(page) => page,
        None => return Err(fail(&"it has no pages")),
    };
    // 72 dots per inch, a pixel a point, where -Dpi is not given.
    let dpi = reading.dpi.unwrap_or(72.0);
    let (points_across, points_down) = page.render_dimensions();
    let own = (pixels_up(f64::from(points_across) * dpi / 72.0), pixels_up(f64::from(points_down) * dpi / 72.0));
    let plan = transform.plan(own.0, own.1, wgpu::TextureDimension::D2, false)?;
    let ((width, height), m) = plan.drawn();
    let most = u16::MAX;
    let across = u16::try_from(width).map_err(|e| fail(&format!("it would be drawn {width} pixels wide, and hayro draws at most {most} a side ({e})")))?;
    let down = u16::try_from(height).map_err(|e| fail(&format!("it would be drawn {height} pixels high, and hayro draws at most {most} a side ({e})")))?;
    let at = Affine::new(m) * Affine::scale(dpi / 72.0) * page.initial_transform(true).to_kurbo();
    let reported = Arc::new(ArcSwap::from_pointee(Vec::new()));
    let reported_here = Arc::clone(&reported);
    let settings = InterpreterSettings { warning_sink: Arc::new(move |warning| noted(&reported_here, described(warning))), ..InterpreterSettings::default() };
    let mut context = RenderContext::new(across, down);
    render_into(page, &RenderCache::new(), &settings, &RenderSettings::default(), &mut context, at);
    context.flush();
    let mut pixmap = Pixmap::new(across, down);
    let ground = if reading.no_background { TRANSPARENT } else { WHITE };
    context.render_with(&mut pixmap, &mut Resources::default(), RasterizerSettings { target_init: TargetInit::Clear(ground), ..Default::default() });
    let image = RgbaImage::from_raw(width, height, pixmap.take_rgba8(ImageAlphaType::Alpha)).ok_or_else(|| fail(&"its pixels do not fill the page"))?;
    warnings.extend(reported.load().iter().map(|what| format!("the PDF's page was drawn without {what}")));
    let said = vec![format!("page 1 of the PDF's {} was drawn at {dpi} dots per inch, {}x{} at that resolution, at {width}x{height}", pages.len(), own.0, own.1)];
    Ok(Drawing { image: DynamicImage::ImageRgba8(image), warnings, said })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pdf_reference as r;
    use crate::transform::{Crop, FlipDirection};

    fn reading(dpi: Option<f64>, no_background: bool) -> Reading {
        Reading { dpi, no_background, ..Reading::default() }
    }

    fn drawn(name: &str, bytes: &[u8], reading: Reading, transform: Transform, password: Option<&str>) -> Drawing {
        match draw(Path::new(name), bytes, reading, &transform, password) {
            Ok(drawing) => drawing,
            Err(e) => panic!("{name} draws: {e:?}"),
        }
    }

    /// The page of the reference's rectangles at `scale` pixels a point, on
    /// white or on nothing, as PDFium draws it.
    fn painted(scale: u32, background: bool) -> RgbaImage {
        let ground = if background { [255; 4] } else { [0; 4] };
        let mut page = RgbaImage::from_pixel(r::WIDTH * scale, r::HEIGHT * scale, image::Rgba(ground));
        for (x, y, w, h, [red, green, blue]) in r::RECTS {
            for down in y * scale..(y + h) * scale {
                for across in x * scale..(x + w) * scale {
                    page.put_pixel(across, down, image::Rgba([red, green, blue, 255]));
                }
            }
        }
        page
    }

    fn assert_same(drawing: &Drawing, want: &RgbaImage, what: &str) {
        let got = drawing.image.to_rgba8();
        assert_eq!(got.dimensions(), want.dimensions(), "{what}: size");
        assert!(got.as_raw() == want.as_raw(), "{what}: the pixels differ from PDFium's");
    }

    /// Pages draw as PDFium draws them: at 72 and 144 dots per inch, on
    /// white and on nothing, turned by /Rotate, cut to the CropBox, the
    /// first of two pages, and opened with a password or none.
    #[test]
    fn draws_pages_as_pdfium_draws_them() {
        for (scale, dpi) in [(1, None), (2, Some(144.0))] {
            for background in [true, false] {
                let what = format!("{} dpi {}", 72 * scale, if background { "on white" } else { "on nothing" });
                assert_same(&drawn("rects.pdf", &r::RECTS_FILE, reading(dpi, !background), Transform::default(), None), &painted(scale, background), &what);
            }
        }
        let turned = image::imageops::rotate90(&painted(1, true));
        assert_same(&drawn("rotated.pdf", &r::ROTATED_FILE, Reading::default(), Transform::default(), None), &turned, "/Rotate 90");
        assert_same(&drawn("cropped.pdf", &r::CROPPED_FILE, Reading::default(), Transform::default(), None), &painted(1, true), "CropBox");
        let two = drawn("two.pdf", &r::TWO_PAGES_FILE, Reading::default(), Transform::default(), None);
        assert_same(&two, &painted(1, true), "the first of two pages");
        assert!(two.warnings.is_empty(), "{:?}", two.warnings);
        assert_same(&drawn("protected.pdf", &r::PROTECTED_FILE, Reading::default(), Transform::default(), Some(r::PASSWORD)), &painted(1, true), "user password");
        assert_same(&drawn("owner.pdf", &r::OWNER_ONLY_FILE, Reading::default(), Transform::default(), None), &painted(1, true), "owner password alone");
    }

    /// A page drawn at a new size, cropped, turned and flipped is the page
    /// drawn at that resolution with those changes made to its pixels.
    #[test]
    fn draws_the_resize_crop_turn_and_flip_into_the_page() {
        let wide = drawn("rects.pdf", &r::RECTS_FILE, Reading::default(), Transform { width: Some(r::WIDTH * 2), ..Default::default() }, None);
        assert_same(&wide, &painted(2, true), "twice as wide");
        let whole = DynamicImage::ImageRgba8(painted(1, true));
        for rotate in [None, Some(90), Some(180), Some(270)] {
            for flip in [None, Some(FlipDirection::Horizontal), Some(FlipDirection::Vertical)] {
                let asked = Transform { crop: Some(Crop::Region { x: 3, y: 2, width: 30, height: 20 }), rotate, flip, ..Default::default() };
                let mut want = whole.crop_imm(3, 2, 30, 20);
                want = match rotate {
                    Some(90) => want.rotate90(),
                    Some(180) => want.rotate180(),
                    Some(270) => want.rotate270(),
                    _ => want,
                };
                want = match flip {
                    Some(FlipDirection::Horizontal) => want.fliph(),
                    Some(FlipDirection::Vertical) => want.flipv(),
                    None => want,
                };
                assert_same(&drawn("rects.pdf", &r::RECTS_FILE, Reading::default(), asked, None), &want.to_rgba8(), &format!("{rotate:?} {flip:?}"));
            }
        }
    }

    /// A protected PDF opens only with its password, and a password a PDF
    /// does not need is warned about.
    #[test]
    fn takes_the_password_a_protected_pdf_needs() {
        for (password, wanted) in [(None, "it is protected by a password: give it as a SecureString with -Password"), (Some("not it"), "it is protected by a password, and -Password gives another")] {
            match draw(Path::new("protected.pdf"), &r::PROTECTED_FILE, Reading::default(), &Transform::default(), password) {
                Ok(_) => panic!("protected.pdf opens with {password:?}"),
                Err(e) => assert!(e.message.ends_with(wanted), "{}", e.message),
            }
        }
        let needless = drawn("rects.pdf", &r::RECTS_FILE, Reading::default(), Transform::default(), Some(r::PASSWORD));
        assert_eq!(needless.warnings, vec!["the PDF opens without a password, so -Password changed nothing".to_string()]);
    }

    /// Text in Helvetica, which the PDF does not carry, draws in the font
    /// hayro embeds for it.
    #[test]
    fn draws_text_in_the_standard_fonts() {
        let text = drawn("text.pdf", &r::TEXT_FILE, Reading::default(), Transform::default(), None).image.to_rgba8();
        assert_eq!(*text.get_pixel(0, 0), image::Rgba([255, 255, 255, 255]), "the page is white where nothing is drawn");
        assert!(text.pixels().any(|p| p[0] < 255), "the text draws");
    }
}
