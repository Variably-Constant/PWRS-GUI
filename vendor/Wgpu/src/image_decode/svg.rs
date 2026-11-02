//! SVG files drawn through resvg, at the size and turn the import's
//! transform plans for the size the file gives, with text in the machine's
//! fonts.

use std::fmt::Display;
use std::path::Path;
use std::sync::Arc;

use arc_swap::ArcSwap;
use image::{DynamicImage, RgbaImage};
use pwrs::prelude::*;
use resvg::tiny_skia::{Pixmap, Transform as Affine};
use resvg::usvg::fontdb::{self, Database};
use resvg::usvg::{FontFamily, FontResolver, Options, Tree};

use super::{noted, pixels_up, unreadable, Drawing};
use crate::transform::Transform;

/// The faces each CSS generic family is set to, the first of its list the
/// machine has: those Windows and macOS carry, then the usual ones of Linux
/// and FreeBSD.
const SERIF: [&str; 6] = ["Times New Roman", "Times", "Liberation Serif", "DejaVu Serif", "Noto Serif", "FreeSerif"];
const SANS_SERIF: [&str; 6] = ["Arial", "Helvetica", "Liberation Sans", "DejaVu Sans", "Noto Sans", "FreeSans"];
const MONOSPACE: [&str; 6] = ["Courier New", "Courier", "Liberation Mono", "DejaVu Sans Mono", "Noto Sans Mono", "FreeMono"];
const CURSIVE: [&str; 2] = ["Comic Sans MS", "Apple Chancery"];
const FANTASY: [&str; 2] = ["Impact", "Papyrus"];

/// The machine's fonts.
pub(super) fn system_fonts() -> Database {
    let mut fonts = Database::new();
    fonts.load_system_fonts();
    fonts
}

/// The first of `names` that a face `fonts` holds is of.
fn first_held(fonts: &Database, names: &[&str]) -> Option<String> {
    names.iter().find(|name| fonts.faces().any(|face| face.families.iter().any(|(family, _)| family == *name))).map(|name| name.to_string())
}

/// `fonts` with each CSS generic family set to the first face of its list
/// `fonts` holds, or else to the serif family's face, which is the first
/// face of all where `fonts` holds none of serif's list; with the serif
/// family's name, the default text is drawn in, `None` for no fonts.
fn with_generic_families(mut fonts: Database) -> (Database, Option<String>) {
    let any = fonts.faces().next().and_then(|face| face.families.first()).map(|(family, _)| family.clone());
    let serif = match first_held(&fonts, &SERIF).or(any) {
        Some(serif) => serif,
        None => return (fonts, None),
    };
    let or_serif = |held: Option<String>| held.unwrap_or_else(|| serif.clone());
    let sans_serif = or_serif(first_held(&fonts, &SANS_SERIF));
    let monospace = or_serif(first_held(&fonts, &MONOSPACE));
    let cursive = or_serif(first_held(&fonts, &CURSIVE));
    let fantasy = or_serif(first_held(&fonts, &FANTASY));
    fonts.set_serif_family(serif.clone());
    fonts.set_sans_serif_family(sans_serif);
    fonts.set_monospace_family(monospace);
    fonts.set_cursive_family(cursive);
    fonts.set_fantasy_family(fantasy);
    (fonts, Some(serif))
}

/// Whether `face` is of `family`, a generic family being the face `fonts`
/// sets it to.
fn of(face: &fontdb::FaceInfo, family: &FontFamily, fonts: &Database) -> bool {
    let name = match family {
        FontFamily::Serif => fonts.family_name(&fontdb::Family::Serif),
        FontFamily::SansSerif => fonts.family_name(&fontdb::Family::SansSerif),
        FontFamily::Cursive => fonts.family_name(&fontdb::Family::Cursive),
        FontFamily::Fantasy => fonts.family_name(&fontdb::Family::Fantasy),
        FontFamily::Monospace => fonts.family_name(&fontdb::Family::Monospace),
        FontFamily::Named(name) => name.as_str(),
    };
    face.families.iter().any(|(held, _)| held.eq_ignore_ascii_case(name))
}

/// The SVG file at `path`, holding `bytes`, drawn at the size and turn
/// `transform` plans for the size it gives, its text in `fonts`: a
/// font-family the fonts hold none of draws in the serif family's face,
/// which a warning says.
pub(super) fn draw(path: &Path, bytes: &[u8], transform: &Transform, fonts: Database) -> PsResult<Drawing> {
    let fail = |e: &dyn Display| unreadable(path, e);
    let (fonts, default_family) = with_generic_families(fonts);
    let missing = Arc::new(ArcSwap::from_pointee(Vec::new()));
    let missing_here = Arc::clone(&missing);
    let choose = FontResolver::default_font_selector();
    let options = Options {
        resources_dir: path.parent().map(Path::to_path_buf),
        // With no fonts, usvg's own default, as the text is left out.
        font_family: match &default_family {
            Some(family) => family.clone(),
            None => Options::default().font_family,
        },
        fontdb: Arc::new(fonts),
        font_resolver: FontResolver {
            // usvg's choice, which falls back to the serif family's face;
            // a choice of no face of the families asked for is noted.
            select_font: Box::new(move |font, fonts| {
                let chosen = choose(font, fonts);
                let asked = chosen.and_then(|id| fonts.face(id)).is_some_and(|face| font.families().iter().any(|family| of(face, family, fonts)));
                if !asked {
                    noted(&missing_here, font.families().iter().map(ToString::to_string).collect::<Vec<_>>().join(", "));
                }
                chosen
            }),
            select_fallback: FontResolver::default_fallback_selector(),
        },
        ..Options::default()
    };
    let tree = Tree::from_data(bytes, &options).map_err(|e| fail(&e))?;
    let size = tree.size();
    let own = (pixels_up(f64::from(size.width())), pixels_up(f64::from(size.height())));
    let plan = transform.plan(own.0, own.1, wgpu::TextureDimension::D2, false)?;
    let ((width, height), m) = plan.drawn();
    let mut pixmap = Pixmap::new(width, height).ok_or_else(|| fail(&format!("it would be drawn at {width}x{height} pixels, more than resvg draws")))?;
    resvg::render(&tree, Affine::from_row(m[0] as f32, m[1] as f32, m[2] as f32, m[3] as f32, m[4] as f32, m[5] as f32), &mut pixmap.as_mut());
    let rgba: Vec<u8> = pixmap
        .pixels()
        .iter()
        .flat_map(|pixel| {
            let color = pixel.demultiply();
            [color.red(), color.green(), color.blue(), color.alpha()]
        })
        .collect();
    let image = RgbaImage::from_raw(width, height, rgba).ok_or_else(|| fail(&"its pixels do not fill the drawing"))?;
    let warnings = missing
        .load()
        .iter()
        .map(|families| match &default_family {
            Some(face) => format!("the SVG's text in the font-family {families} was drawn in {face}, the default family, as the machine has none of the fonts it names"),
            None => format!("the SVG's text in the font-family {families} was left out, as the machine has no fonts"),
        })
        .collect();
    let said = vec![format!("the SVG file, {}x{} at its own size, was drawn at {width}x{height}", own.0, own.1)];
    Ok(Drawing { image: DynamicImage::ImageRgba8(image), warnings, said })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transform::{Crop, FlipDirection};

    /// The bundled Noto Sans subset, the only face the tests draw text in.
    const NOTO_SANS: &[u8] = include_bytes!("../../tests/fonts/NotoSans-Regular-subset.ttf");

    fn test_fonts() -> Database {
        let mut fonts = Database::new();
        fonts.load_font_data(NOTO_SANS.to_vec());
        fonts
    }

    fn drawn_as(name: &str, bytes: &[u8], transform: Transform) -> Drawing {
        match draw(Path::new(name), bytes, &transform, test_fonts()) {
            Ok(drawing) => drawing,
            Err(e) => panic!("{name} draws: {e:?}"),
        }
    }

    /// A reference PNG's pixels, straight RGBA.
    fn reference(png: &[u8]) -> RgbaImage {
        match image::load_from_memory(png) {
            Ok(image) => image.to_rgba8(),
            Err(e) => panic!("the reference decodes: {e}"),
        }
    }

    fn assert_same(drawing: &Drawing, want: &RgbaImage, what: &str) {
        let got = drawing.image.to_rgba8();
        assert_eq!(got.dimensions(), want.dimensions(), "{what}: size");
        assert!(got.as_raw() == want.as_raw(), "{what}: the pixels differ from resvg's own");
    }

    /// The drawing is resvg's own, as its command line draws and writes
    /// it: at the size the file gives, at a width, at half the size, from
    /// an SVGZ file, of an embedded PNG, and of text in the one face the
    /// fonts hold.
    #[test]
    fn draws_as_resvgs_command_line_draws() {
        let shapes: &[u8] = include_bytes!("../../tests/svg/shapes.svg");
        assert_same(&drawn_as("shapes.svg", shapes, Transform::default()), &reference(include_bytes!("../../tests/svg/shapes.png")), "own size");
        assert_same(&drawn_as("shapes.svg", shapes, Transform { width: Some(80), ..Default::default() }), &reference(include_bytes!("../../tests/svg/shapes-80.png")), "width 80");
        assert_same(&drawn_as("shapes.svg", shapes, Transform { scale: Some(0.5), ..Default::default() }), &reference(include_bytes!("../../tests/svg/shapes-half.png")), "half size");
        assert_same(&drawn_as("shapes.svgz", include_bytes!("../../tests/svg/shapes.svgz"), Transform::default()), &reference(include_bytes!("../../tests/svg/shapes.png")), "svgz");
        assert_same(&drawn_as("image.svg", include_bytes!("../../tests/svg/image.svg"), Transform::default()), &reference(include_bytes!("../../tests/svg/image.png")), "embedded PNG");
        let text = drawn_as("text.svg", include_bytes!("../../tests/svg/text.svg"), Transform::default());
        assert_same(&text, &reference(include_bytes!("../../tests/svg/text.png")), "text");
        assert_eq!(text.warnings, vec!["the SVG's text in the font-family \"No Such Family\" was drawn in Noto Sans, the default family, as the machine has none of the fonts it names".to_string()]);
    }

    /// Drawn cropped, turned and flipped, a file of shapes on whole pixels
    /// is its drawing at its own size with those changes made to the
    /// pixels.
    #[test]
    fn draws_the_crop_turn_and_flip_into_the_drawing() {
        let blocks: &[u8] = include_bytes!("../../tests/svg/blocks.svg");
        let whole = drawn_as("blocks.svg", blocks, Transform::default()).image;
        for rotate in [None, Some(90), Some(180), Some(270)] {
            for flip in [None, Some(FlipDirection::Horizontal), Some(FlipDirection::Vertical)] {
                let asked = Transform { crop: Some(Crop::Region { x: 1, y: 1, width: 10, height: 6 }), rotate, flip, ..Default::default() };
                let mut want = whole.crop_imm(1, 1, 10, 6);
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
                let got = drawn_as("blocks.svg", blocks, asked).image;
                assert!(got.to_rgba8() == want.to_rgba8(), "{rotate:?} {flip:?}");
            }
        }
    }

    /// Generic families are set to faces the fonts hold, and text with no
    /// fonts at all is left out with a warning.
    #[test]
    fn sets_generic_families_to_faces_the_machine_has() {
        let (fonts, default_family) = with_generic_families(test_fonts());
        assert_eq!(default_family.as_deref(), Some("Noto Sans"));
        for generic in [fontdb::Family::Serif, fontdb::Family::SansSerif, fontdb::Family::Monospace, fontdb::Family::Cursive, fontdb::Family::Fantasy] {
            assert_eq!(fonts.family_name(&generic), "Noto Sans");
        }
        let none = match draw(Path::new("text.svg"), include_bytes!("../../tests/svg/text.svg"), &Transform::default(), Database::new()) {
            Ok(drawing) => drawing,
            Err(e) => panic!("text.svg draws with no fonts: {e:?}"),
        };
        assert!(none.warnings.contains(&"the SVG's text in the font-family \"No Such Family\" was left out, as the machine has no fonts".to_string()), "{:?}", none.warnings);
        assert!(none.image.to_rgba8().pixels().all(|p| p[3] == 0), "no text is drawn without fonts");
    }

    /// An SVG's size in inches converts at CSS's 96 pixels an inch.
    #[test]
    fn takes_its_own_size_in_any_unit() {
        let inches = br#"<svg xmlns="http://www.w3.org/2000/svg" width="0.5in" height="0.25in"/>"#;
        let drawing = drawn_as("inches.svg", inches, Transform::default());
        assert_eq!((drawing.image.width(), drawing.image.height()), (48, 24));
    }
}
