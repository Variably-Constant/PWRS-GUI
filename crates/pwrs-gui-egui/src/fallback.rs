//! Fonts for the scripts the system's UI font lacks, found among the
//! system's own when a window's text first needs them. Each character of
//! a window's text that none of egui's faces has is looked for in the
//! system's faces, regular upright ones first, and the first face that has
//! it joins the end of egui's proportional and monospace families, so text
//! in any script the system has a font for draws, and only the faces some
//! text needs are read into memory.

use super::{Node, Value, egui};
use std::collections::HashSet;
use std::sync::Arc;

/// The faces egui draws in and the characters already looked for.
pub(super) struct Fallbacks {
    db: Arc<fontdb::Database>,
    held: Vec<Arc<egui::FontData>>,
    added: HashSet<fontdb::ID>,
    seen: HashSet<char>,
}

impl Fallbacks {
    /// Fallbacks over the system's faces `db`, egui having the faces of
    /// `installed` loaded.
    pub(super) fn new(db: Arc<fontdb::Database>, installed: &egui::FontDefinitions) -> Fallbacks {
        Fallbacks { db, held: installed.font_data.values().cloned().collect(), added: HashSet::new(), seen: HashSet::new() }
    }

    /// Makes every string of `node`'s subtree drawable.
    pub(super) fn cover_tree(&mut self, ctx: &egui::Context, node: &Node) {
        for v in node.props.values() {
            self.cover_value(ctx, v);
        }
        if let Some(name) = &node.name {
            self.cover(ctx, name);
        }
        for c in &node.children {
            self.cover_tree(ctx, c);
        }
    }

    /// Makes every string in `value` drawable.
    pub(super) fn cover_value(&mut self, ctx: &egui::Context, value: &Value) {
        match value {
            Value::Str(s) => self.cover(ctx, s),
            Value::List(items) => items.iter().for_each(|v| self.cover_value(ctx, v)),
            Value::Map(m) => m.values().for_each(|v| self.cover_value(ctx, v)),
            Value::Null | Value::Bool(_) | Value::Int(_) | Value::Float(_) | Value::Bytes(_) => {}
        }
    }

    /// Makes `text` drawable: each character none of egui's loaded faces
    /// has, the first time it is met, takes the first system face that has
    /// it, which egui then loads. A character no face has is traced, and
    /// egui draws its replacement for it.
    pub(super) fn cover(&mut self, ctx: &egui::Context, text: &str) {
        for c in text.chars() {
            if c.is_control() || c.is_whitespace() || !self.seen.insert(c) {
                continue;
            }
            if self.held.iter().any(|f| has(f.bytes(), f.index, c)) {
                continue;
            }
            match self.find(c) {
                Some((id, family)) => self.add(ctx, id, &family, c),
                None => pwrs_gui_proto::trace::line(|| format!("egui: no system font has {c:?} (U+{:04X})", u32::from(c))),
            }
        }
    }

    /// The first system face egui has not loaded that has `c`, regular
    /// upright faces first, and its family.
    fn find(&self, c: char) -> Option<(fontdb::ID, String)> {
        let regular = |f: &fontdb::FaceInfo| f.weight == fontdb::Weight::NORMAL && f.style == fontdb::Style::Normal && f.stretch == fontdb::Stretch::Normal;
        for wanted_regular in [true, false] {
            for face in self.db.faces() {
                if regular(face) != wanted_regular || self.added.contains(&face.id) {
                    continue;
                }
                if self.db.with_face_data(face.id, |data, index| has(data, index, c)) == Some(true) {
                    let family = face.families.first().map_or_else(|| face.post_script_name.clone(), |(name, _)| name.clone());
                    return Some((face.id, family));
                }
            }
        }
        None
    }

    /// Reads the face `id` into egui's fonts, after every face it has in
    /// both families, for `c` and the characters after it.
    fn add(&mut self, ctx: &egui::Context, id: fontdb::ID, family: &str, c: char) {
        let Some((bytes, index)) = self.db.with_face_data(id, |data, index| (data.to_vec(), index)) else {
            pwrs_gui_proto::trace::line(|| format!("egui: the font {family} has {c:?} and could not be read"));
            return;
        };
        let data = egui::FontData::from_blob(Arc::new(bytes), index);
        let name = format!("system-fallback-{family}-{index}");
        pwrs_gui_proto::trace::line(|| format!("egui: {c:?} (U+{:04X}) draws in {family}, which now follows the fonts egui had", u32::from(c)));
        let lowest = |family| egui::epaint::text::InsertFontFamily { family, priority: egui::epaint::text::FontPriority::Lowest };
        ctx.add_font(egui::epaint::text::FontInsert { name, data: data.clone(), families: vec![lowest(egui::FontFamily::Proportional), lowest(egui::FontFamily::Monospace)] });
        self.added.insert(id);
        self.held.push(Arc::new(data));
    }
}

/// Whether the face `index` of the font file `data` has a glyph for `c`; a
/// face that cannot be read is traced and has none.
fn has(data: &[u8], index: u32, c: char) -> bool {
    match ttf_parser::Face::parse(data, index) {
        Ok(face) => face.glyph_index(c).is_some(),
        Err(why) => {
            pwrs_gui_proto::trace::line(|| format!("egui: a font face could not be read to look for {c:?}: {why}"));
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn egui_s_own_faces_have_latin_and_no_han() {
        let own = egui::FontDefinitions::default();
        let held: Vec<Arc<egui::FontData>> = own.font_data.values().cloned().collect();
        let any = |c: char| held.iter().any(|f| has(f.bytes(), f.index, c));
        assert!(any('A') && any('ж'), "egui's own fonts draw Latin and Cyrillic");
        assert!(!any('漢'), "egui's own fonts have no Han, which a fallback must bring");
    }

    #[test]
    fn the_builds_faces_have_latin_greek_and_cyrillic_and_no_han() {
        for (name, face) in [(pwrs_gui_fonts::INTER_FAMILY, pwrs_gui_fonts::INTER), (pwrs_gui_fonts::JETBRAINS_MONO_FAMILY, pwrs_gui_fonts::JETBRAINS_MONO)] {
            assert!(has(face, 0, 'A') && has(face, 0, 'λ') && has(face, 0, 'ж'), "{name} draws Latin, Greek and Cyrillic");
            assert!(!has(face, 0, '漢'), "{name} has no Han, which the system's fonts draw");
        }
    }

    /// What a fallback costs on this machine: the system's faces read as the
    /// helper reads them, then for each script, the time `cover` takes to find
    /// and add a face for it, and for a character no face has, a walk of every
    /// face. Round 0 meets each face file for the first time since the faces
    /// were read; later rounds find them in the system's file cache. Each
    /// clock encloses one `cover` call and nothing else. Run it on a quiet
    /// machine:
    /// cargo test --profile test-fast -p pwrs-gui-egui --lib fallback_cost -- --ignored --nocapture
    #[test]
    #[ignore = "a measurement for a quiet machine, not a check"]
    fn fallback_cost() {
        let started = std::time::Instant::now();
        let found = crate::system_fonts(pwrs_gui_proto::Fonts::Bundled);
        let read = started.elapsed();
        println!("FALLBACK faces={} system_fonts_ms={:.1}", found.database.faces().count(), read.as_secs_f64() * 1000.0);
        let samples = [("Han", "漢字"), ("Kana", "かな"), ("Hangul", "한글"), ("Arabic", "عربي"), ("Devanagari", "हिन्दी"), ("Thai", "ไทย"), ("Hebrew", "עברית"), ("Emoji", "😀"), ("no-font", "\u{10FFFD}")];
        for round in 0..3 {
            let ctx = egui::Context::default();
            let mut fallbacks = Fallbacks::new(Arc::clone(&found.database), &found.definitions);
            for (script, text) in samples {
                let clock = std::time::Instant::now();
                fallbacks.cover(&ctx, text);
                let took = clock.elapsed();
                println!("FALLBACK round={round} script={script} ms={:.2} faces_added={}", took.as_secs_f64() * 1000.0, fallbacks.added.len());
            }
        }
    }
}
