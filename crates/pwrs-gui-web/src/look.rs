//! The tokens both renderers draw from ([`proto::tokens`]) as the widget
//! runtime's page takes them: CSS variables for each theme's palette, the
//! spacing and the text sizes, the fonts the build includes
//! ([`pwrs_gui_fonts`]) when text is drawn in them, then the font families
//! egui chooses for each kind of text ([`proto::typeface`]), and egui's own
//! fonts after them, which the helper gives the page, so a page draws text
//! where the system has no fonts. Each size of text is given the height of
//! a row of it in the face egui draws it in, which the runtime rounds to a
//! whole physical pixel as egui rounds a row, so a line of text takes the
//! same room in both renderers.

use pwrs_gui_proto as proto;
use proto::Fonts;
use proto::tokens::{self, Color, Look, Palette, Shadow, Stroke};
use proto::typeface::{FaceMetrics, Families, Found, Wanted};

/// The family name egui's own proportional font is given to pages under.
pub(crate) const OWN_PROPORTIONAL: &str = "PWRS-GUI Ubuntu Light";
/// The family name egui's own monospace font is given to pages under.
pub(crate) const OWN_MONOSPACE: &str = "PWRS-GUI Hack";
/// egui's own fonts, by the path under `pwrs://localhost/` each loads from.
pub(crate) const OWN_FONTS: [(&str, &[u8]); 2] = [("/fonts/Ubuntu-Light.ttf", epaint_default_fonts::UBUNTU_LIGHT), ("/fonts/Hack-Regular.ttf", epaint_default_fonts::HACK_REGULAR)];
/// The family the build's proportional font is given to a page under.
pub(crate) const BUNDLED_UI: &str = "PWRS-GUI Inter";
/// The family the build's monospace font is given to a page under.
pub(crate) const BUNDLED_MONO: &str = "PWRS-GUI JetBrains Mono NL";
/// The fonts the build includes, by the path under `pwrs://localhost/` each
/// loads from, with the family a page names it by.
pub(crate) const BUNDLED_FONTS: [(&str, &str, &[u8]); 2] = [("/fonts/Inter.ttf", BUNDLED_UI, pwrs_gui_fonts::INTER), ("/fonts/JetBrainsMonoNL.ttf", BUNDLED_MONO, pwrs_gui_fonts::JETBRAINS_MONO)];
/// The family the system UI font egui draws is given to a page under.
pub(crate) const SYSTEM_UI: &str = "PWRS-GUI System UI";
/// The path under `pwrs://localhost/` that font loads from.
pub(crate) const SYSTEM_UI_PATH: &str = "/fonts/system-ui.ttf";

/// What the helper gives every widget page besides the runtime: the
/// tokens' stylesheet, and on macOS the file of the system UI font egui
/// draws, which the stylesheet names first, so WebKit lays text out in the
/// face egui does rather than the system font's sizes of its own.
pub(crate) struct Assets {
    pub css: String,
    pub ui_font: Option<&'static [u8]>,
}

/// The height in points of one row of each size of text in its face, before
/// egui rounds the row to a whole physical pixel, and the width of a space
/// in body text; none for a size whose face could not be read, which a page
/// then lays out at its engine's own height.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct Lines {
    pub small: Option<f32>,
    pub body: Option<f32>,
    pub button: Option<f32>,
    pub heading: Option<f32>,
    /// A dialog's main line ([`tokens::DIALOG`]'s instruction size).
    pub instruction: Option<f32>,
    pub mono: Option<f32>,
    /// A Markdown document's headings of levels 2 to 6, at the sizes
    /// [`proto::markdown::heading_size`] gives them.
    pub headings: [Option<f32>; 5],
    /// A space in body text, which a Markdown list's points are four of.
    pub space: Option<f32>,
}

/// The faces egui draws text in, as the system has them: the family of
/// each kind's, none where egui draws that kind in its own font; the row
/// height of each size of text in the face drawn first, the build's own
/// when text is drawn in it; and each problem met finding them.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Text {
    pub ui: Option<String>,
    pub mono: Option<String>,
    /// The system's UI face's file, stored on macOS while text is drawn in
    /// the system's fonts, where a page is given it.
    pub ui_font: Option<Vec<u8>>,
    /// The spacing, in em, a page gives text in that face so WebKit
    /// lays it out as egui does ([`proto::typeface::page_tracking`]).
    pub ui_tracking: Option<f32>,
    pub lines: Lines,
    /// The vertical metrics of the face proportional text is drawn in, the
    /// build's, the UI face or egui's own, which place a canvas's text at
    /// any size as egui places it; none where they cannot be read.
    pub ui_face: Option<FaceMetrics>,
    pub problems: Vec<String>,
}

/// Finds the faces egui draws each kind of text in with `fonts`, as the
/// egui renderer finds them, and the height of a row of each size of text in
/// the face drawn first: the build's own with [`Fonts::Bundled`], else the
/// system's, or egui's own where the system has none of a kind's families.
pub(crate) fn text(families: &Families, fonts: Fonts) -> Text {
    let db = proto::typeface::system_database();
    let mut problems = Vec::new();
    let mut find = |wanted: &[Wanted]| match proto::typeface::first_found(&db, wanted) {
        Ok(found) => found,
        Err(why) => {
            problems.push(why);
            None
        }
    };
    let (ui, mono) = (find(&families.ui), find(&families.mono));
    // The face each kind of text is drawn in first: its bytes, its index in
    // its file, and its family.
    let first = |found: &Option<Found>, bundled: (&'static [u8], &'static str), own: &'static [u8]| -> (Vec<u8>, u32, String) {
        match (fonts, found) {
            (Fonts::Bundled, _) => (bundled.0.to_vec(), 0, bundled.1.to_string()),
            (Fonts::System, Some(f)) => (f.data.clone(), f.index, f.family.clone()),
            (Fonts::System, None) => (own.to_vec(), 0, String::from("egui's own")),
        }
    };
    let ui_first = first(&ui, (pwrs_gui_fonts::INTER, pwrs_gui_fonts::INTER_FAMILY), epaint_default_fonts::UBUNTU_LIGHT);
    let mono_first = first(&mono, (pwrs_gui_fonts::JETBRAINS_MONO, pwrs_gui_fonts::JETBRAINS_MONO_FAMILY), epaint_default_fonts::HACK_REGULAR);
    let body = tokens::TEXT.body;
    let space = match proto::typeface::space_width(&ui_first.0, ui_first.1, body) {
        Ok(w) => Some(w),
        Err(why) => {
            problems.push(format!("the width of a space in {body}-point text in the font {} cannot be read, so a page sets a Markdown list's points in its engine's spaces: {why}", ui_first.2));
            None
        }
    };
    let mut height =|(data, index, name): &(Vec<u8>, u32, String), size: f32| match proto::typeface::row_height(data, *index, size) {
        Ok(h) => Some(h),
        Err(why) => {
            problems.push(format!("the height of a row of {size}-point text in the font {name} cannot be read, so a page lays it out at its engine's own: {why}"));
            None
        }
    };
    let t = &tokens::TEXT;
    let headings = std::array::from_fn(|i| height(&ui_first, proto::markdown::heading_size(i as u8 + 2, t.body, t.heading)));
    let lines = Lines {
        small: height(&ui_first, t.small),
        body: height(&ui_first, t.body),
        button: height(&ui_first, t.button),
        heading: height(&ui_first, t.heading),
        instruction: height(&ui_first, tokens::DIALOG.instruction),
        mono: height(&mono_first, t.monospace),
        headings,
        space,
    };
    let ui_face = match proto::typeface::face_metrics(&ui_first.0, ui_first.1) {
        Ok(m) => Some(m),
        Err(why) => {
            problems.push(format!("the metrics of the font {} cannot be read, so a page places a canvas's text by its engine's own: {why}", ui_first.2));
            None
        }
    };
    let ui_font = if cfg!(target_vendor = "apple") && fonts == Fonts::System { ui.as_ref().map(|f| f.data.clone()) } else { None };
    let mut ui_tracking = None;
    if let (Some(f), Some(_)) = (&ui, &ui_font) {
        match proto::typeface::page_tracking(&f.data, f.index) {
            Ok(tracking) => ui_tracking = tracking,
            Err(why) => problems.push(format!("the tracking of the font {} cannot be read, so a page spaces its text as WebKit does: {why}", f.family)),
        }
    }
    Text { ui: ui.map(|f| f.family), mono: mono.map(|f| f.family), ui_font, ui_tracking, lines, ui_face, problems }
}

/// The system UI face a page is given, as the stylesheet names it: the
/// spacing, in em, its text takes so WebKit lays it out as egui does, none
/// where the two space it alike.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct ServedUi {
    pub tracking: Option<f32>,
}

/// A color as CSS writes it: unmultiplied, and a color that adds its light
/// to what is under it as that light added to the panel it is drawn on.
fn css_color(c: Color, panel: Color) -> String {
    let [r, g, b, a] = c.0;
    if a == 0 && (r, g, b) != (0, 0, 0) {
        let [pr, pg, pb, _] = panel.0;
        return format!("rgb({} {} {})", pr.saturating_add(r), pg.saturating_add(g), pb.saturating_add(b));
    }
    if a == 255 {
        return format!("rgb({r} {g} {b})");
    }
    if a == 0 {
        return String::from("transparent");
    }
    let un = |v: u8| (f32::from(v) * 255.0 / f32::from(a)).round().min(255.0) as u8;
    format!("rgb({} {} {} / {:.3})", un(r), un(g), un(b), f32::from(a) / 255.0)
}

fn shadow(s: &Shadow, panel: Color) -> String {
    format!("{}px {}px {}px {}px {}", s.offset[0], s.offset[1], s.blur, s.spread, css_color(s.color, panel))
}

/// A widget state's variables, `--{state}-…`.
fn look(state: &str, l: &Look, panel: Color) -> String {
    format!(
        "--{state}-bg: {}; --{state}-weak-bg: {}; --{state}-stroke: {}; --{state}-stroke-width: {}px; --{state}-fg: {}; --{state}-fg-width: {}px; --{state}-radius: {}px; --{state}-expansion: {}px; ",
        css_color(l.bg_fill, panel),
        css_color(l.weak_bg_fill, panel),
        css_color(l.bg_stroke.color, panel),
        l.bg_stroke.width,
        css_color(l.fg_stroke.color, panel),
        l.fg_stroke.width,
        l.corner_radius,
        l.expansion
    )
}

fn stroke(name: &str, s: &Stroke, panel: Color) -> String {
    format!("--{name}: {}; --{name}-width: {}px; ", css_color(s.color, panel), s.width)
}

/// The variables of one theme's palette.
fn palette(p: &Palette) -> String {
    let panel = p.panel_fill;
    let mut out = String::new();
    for (state, l) in [("noninteractive", &p.noninteractive), ("inactive", &p.inactive), ("hovered", &p.hovered), ("active", &p.active), ("open", &p.open)] {
        out.push_str(&look(state, l, panel));
    }
    out.push_str(&stroke("window-stroke", &p.window_stroke, panel));
    out.push_str(&stroke("selection-stroke", &p.selection_stroke, panel));
    out.push_str(&stroke("cursor", &p.text_cursor, panel));
    let [r, g, b, _] = p.noninteractive.fg_stroke.color.0;
    out.push_str(&format!(
        "--panel: {}; --window: {}; --window-radius: {}px; --menu-radius: {}px; --text: {}; --strong: {}; --weak: rgb({r} {g} {b} / {}); --disabled-alpha: {}; \
         --selection: {}; --link: {}; --faint: {}; --extreme: {}; --code: {}; --warn: {}; --error: {}; --strip: {}; --popup-shadow: {}; --window-shadow: {}; color-scheme: {};",
        css_color(p.panel_fill, panel),
        css_color(p.window_fill, panel),
        p.window_corner_radius,
        p.menu_corner_radius,
        css_color(p.noninteractive.fg_stroke.color, panel),
        css_color(p.active.fg_stroke.color, panel),
        p.weak_text_alpha,
        p.disabled_alpha,
        css_color(p.selection_fill, panel),
        css_color(p.hyperlink, panel),
        css_color(p.faint_bg, panel),
        css_color(p.extreme_bg, panel),
        css_color(p.code_bg, panel),
        css_color(p.warn, panel),
        css_color(p.error, panel),
        css_color(p.strip, panel),
        shadow(&p.popup_shadow, panel),
        shadow(&p.window_shadow, panel),
        if p.dark { "dark" } else { "light" },
    ));
    out
}

/// The spacing's, a dialog's measures' and the text sizes' variables, a
/// Markdown heading's of level n as `--heading-n`, the same in both themes,
/// with the most a check box's corners are rounded, which egui limits to
/// three tenths of the box, the size a widget that takes the room it is
/// given takes where it is given none, a progress bar's thickness, the line
/// under a tab's title, a tree's chevron, and a password field's peek
/// button.
fn spacing() -> String {
    let s = &tokens::SPACING;
    let t = &tokens::TEXT;
    let d = &tokens::DIALOG;
    let dialog = format!(
        "--dialog-margin: {}px; --dialog-button-w: {}px; --dialog-button-h: {}px; --dialog-gap: {}px; --instruction: {}px; \
         --icon-round: {}px; --area-w: {}px; --area-h: {}px; --progress-h: {}px; --tab-mark: {}px; --chevron-span: {}px; --chevron-w: {}px; ",
        d.margin,
        d.button[0],
        d.button[1],
        d.button_gap,
        d.instruction,
        (0.3 * s.icon_width).round(),
        s.default_area_size[0],
        s.default_area_size[1],
        tokens::PROGRESS_BAR,
        tokens::TAB_MARK,
        tokens::CHEVRON.span,
        tokens::CHEVRON.stroke
    );
    let widgets = format!(
        "--item-x: {}px; --item-y: {}px; --button-px: {}px; --button-py: {}px; --indent: {}px; --interact-w: {}px; --interact-h: {}px; \
         --slider-w: {}px; --rail-h: {}px; --combo-w: {}px; --combo-h: {}px; --edit-w: {}px; --edit-px: {}px; --edit-py: {}px; --edit-rows: {}; \
         --icon: {}px; --icon-inner: {}px; --icon-gap: {}px; --menu-margin: {}px; --menu-w: {}px; --menu-gap: {}px; --tooltip-w: {}px; \
         --panel-margin: {}px; --bar-px: {}px; --bar-py: {}px; --group-margin: {}px; \
         --scroll-w: {}px; --scroll-thin: {}px; --scroll-room: {}px; --scroll-min: {}px; --scroll-gap: {}px; --grab: {}px; \
         --small: {}px; --body: {}px; --button: {}px; --heading: {}px; --mono: {}px;",
        s.item_spacing[0],
        s.item_spacing[1],
        s.button_padding[0],
        s.button_padding[1],
        s.indent,
        s.interact_size[0],
        s.interact_size[1],
        s.slider_width,
        s.slider_rail_height,
        s.combo_width,
        s.combo_height,
        s.text_edit_width,
        s.text_edit_margin[0],
        s.text_edit_margin[1],
        s.text_edit_rows,
        s.icon_width,
        s.icon_width_inner,
        s.icon_spacing,
        s.menu_margin,
        s.menu_width,
        s.menu_spacing,
        s.tooltip_width,
        s.panel_margin,
        s.bar_margin[0],
        s.bar_margin[1],
        s.group_margin,
        s.scroll_bar_width,
        s.scroll_bar_floating_width,
        s.scroll_bar_allocated_width,
        s.scroll_handle_min_length,
        s.scroll_bar_inner_margin,
        s.resize_grab,
        t.small,
        t.body,
        t.button,
        t.heading,
        t.monospace,
    );
    let headings: String = (2..=6).map(|level| format!(" --heading-{level}: {}px;", proto::markdown::heading_size(level, t.body, t.heading))).collect();
    dialog + &widgets + &headings + &peek() + &media_bar()
}

/// A media player's controls ([`tokens::MEDIA_BAR`]): the bar's sizes, its
/// shade and ink, how long it stays in full screen after the pointer rests,
/// and each icon as an SVG the page masks a button with, drawn as the egui
/// renderer paints it: its polygons filled, its lines stroked. Then its
/// subtitles' text size, padding, corners, shade, ink and lift
/// ([`tokens::MEDIA_CUE`]).
fn media_bar() -> String {
    let m = &tokens::MEDIA_BAR;
    let c = &tokens::MEDIA_CUE;
    let rgba = |c: tokens::Color| format!("rgba({}, {}, {}, {:.3})", c.0[0], c.0[1], c.0[2], f32::from(c.0[3]) / 255.0);
    let inset = m.button / 8.0;
    let stroke = m.stroke / (m.button - 2.0 * inset);
    let icon = |name: &str, icon: &tokens::Icon| {
        let points = |line: &[[f32; 2]]| line.iter().map(|p| format!("{} {}", p[0], p[1])).collect::<Vec<_>>().join(" ");
        let fills: String = icon.fills.iter().map(|f| format!("%3Cpolygon points='{}'/%3E", points(f))).collect();
        let strokes: String = icon.strokes.iter().map(|l| format!("%3Cpolyline points='{}' fill='none' stroke='black' stroke-width='{stroke}' stroke-linecap='round' stroke-linejoin='round'/%3E", points(l))).collect();
        format!(" --media-{name}: url(\"data:image/svg+xml,%3Csvg xmlns='http://www.w3.org/2000/svg' viewBox='0 0 1 1'%3E{fills}{strokes}%3C/svg%3E\");")
    };
    format!(
        " --media-bar-h: {}px; --media-button: {}px; --media-inset: {inset}px; --media-gap: {}px; --media-track: {}px; --media-knob: {}px; --media-volume-w: {}px; --media-shade: {}; --media-ink: {}; --media-idle: {};",
        m.height,
        m.button,
        m.gap,
        m.track,
        m.knob,
        m.volume,
        rgba(m.shade),
        rgba(m.ink),
        m.idle
    ) + &icon("play", &tokens::PLAY)
        + &icon("pause", &tokens::PAUSE)
        + &icon("sound", &tokens::SOUND)
        + &icon("muted", &tokens::MUTED)
        + &icon("full", &tokens::FULL)
        + &icon("windowed", &tokens::WINDOWED)
        + &format!(
            " --media-cue-size: {}px; --media-cue-pad-x: {}px; --media-cue-pad-y: {}px; --media-cue-round: {}px; --media-cue-shade: {}; --media-cue-ink: {}; --media-cue-lift: {}px;",
            c.size,
            c.pad[0],
            c.pad[1],
            c.round,
            rgba(c.shade),
            rgba(c.ink),
            c.lift
        )
}

/// A password field's peek button ([`tokens::PEEK`]): its width, and its
/// eye as an SVG the page masks the button with and the eye's box. The eye
/// is drawn as egui paints it: two quadratic curves between the eye's ends,
/// their control points the eye's height above and below its middle,
/// stroked, around a filled pupil, with a stroke's width of room around.
fn peek() -> String {
    let p = &tokens::PEEK;
    let [width, height] = p.eye;
    let room = p.stroke;
    let (across, down) = (width + 2.0 * room, height + 2.0 * room);
    let (cx, cy) = (across / 2.0, down / 2.0);
    let svg = format!(
        "%3Csvg xmlns='http://www.w3.org/2000/svg' viewBox='0 0 {across} {down}'%3E\
         %3Cpath d='M{room} {cy}Q{cx} {} {} {cy}Q{cx} {} {room} {cy}Z' fill='none' stroke='black' stroke-width='{}'/%3E\
         %3Ccircle cx='{cx}' cy='{cy}' r='{}'/%3E%3C/svg%3E",
        cy - height,
        room + width,
        cy + height,
        p.stroke,
        p.pupil / 2.0
    );
    format!(" --peek-button: {}px; --peek-eye: url(\"data:image/svg+xml,{svg}\"); --peek-eye-w: {across}px; --peek-eye-h: {down}px;", p.button)
}

/// A family as CSS names it: in quotes, a quote or backslash in it escaped.
fn quoted(name: &str) -> String {
    let mut out = String::from("\"");
    for c in name.chars() {
        if matches!(c, '"' | '\\') {
            out.push('\\');
        }
        out.push(c);
    }
    out.push('"');
    out
}

/// A kind of text's families as a CSS list: `served`, a family the helper
/// gives the page, first when there is one, then the system's in the order
/// it prefers them, then egui's own font, `own`. On macOS the system's UI
/// font is `system-ui` and SF Mono `ui-monospace`, the names WebKit gives
/// them.
fn family_list(served: Option<&str>, wanted: &[Wanted], own: &str) -> String {
    let mut names: Vec<String> = served.map(quoted).into_iter().collect();
    for w in wanted {
        match w {
            Wanted::Family(name) if cfg!(target_vendor = "apple") && name == "SF Mono" => names.push(String::from("ui-monospace")),
            Wanted::Family(name) => names.push(quoted(name)),
            #[cfg(target_vendor = "apple")]
            Wanted::File(_) => names.push(String::from("system-ui")),
        }
    }
    names.push(quoted(own));
    names.join(", ")
}

/// The row heights' variables, `--row-…`, for the sizes whose height was
/// read, a Markdown heading's of level n as `--row-heading-n`, and the width
/// of a space in body text as `--space`. The runtime rounds each row to a
/// whole physical pixel as egui rounds a row, and lays text out in
/// `--line-…`.
fn line_heights(lines: &Lines) -> String {
    let mut out = String::new();
    for (name, height) in [("small", lines.small), ("body", lines.body), ("button", lines.button), ("heading", lines.heading), ("instruction", lines.instruction), ("mono", lines.mono)] {
        if let Some(h) = height {
            out.push_str(&format!("--row-{name}: {h}px; "));
        }
    }
    for (level, height) in (2..).zip(lines.headings) {
        if let Some(h) = height {
            out.push_str(&format!("--row-heading-{level}: {h}px; "));
        }
    }
    if let Some(w) = lines.space {
        out.push_str(&format!("--space: {w}px; "));
    }
    out
}

/// The variables of the vertical metrics of the face proportional text is
/// drawn in, `--face-…` in its own units, which the runtime scales and
/// rounds as egui does to place a canvas's text at any size; none where
/// they were not read.
fn face_lines(face: Option<FaceMetrics>) -> String {
    match face {
        Some(f) => format!("--face-units: {}; --face-ascent: {}; --face-descent: {}; --face-gap: {}; ", f.units, f.ascent, f.descent, f.gap),
        None => String::new(),
    }
}

/// The stylesheet a widget page loads before the runtime's own: with
/// `fonts` [`Fonts::Bundled`] the build's fonts as faces, named first;
/// egui's own fonts as faces; and with `ui` the system UI font's file as
/// [`SYSTEM_UI`] at its default instance, its text spaced by
/// `--ui-tracking` where WebKit tracks it apart from egui. A face with an
/// optical size axis is drawn at its default instance, as egui draws it.
/// Then the families of each kind of text, the height of a row of each
/// size of text, the metrics of `face`, the proportional face, the spacing,
/// and each theme's palette: the dark one while the system prefers dark,
/// unless the page's root says `data-theme="light"`, and while the root
/// says `data-theme="dark"`.
pub(crate) fn stylesheet(families: &Families, lines: &Lines, face: Option<FaceMetrics>, ui: Option<ServedUi>, fonts: Fonts) -> String {
    let mut out = String::new();
    let bundled = fonts == Fonts::Bundled;
    let carried = BUNDLED_FONTS.iter().filter(|_| bundled).map(|(path, family, _)| (*path, *family));
    let served = ui.map(|_| (SYSTEM_UI_PATH, SYSTEM_UI));
    let own = OWN_FONTS.iter().map(|(path, _)| *path).zip([OWN_PROPORTIONAL, OWN_MONOSPACE]);
    for (path, family) in carried.chain(served).chain(own) {
        out.push_str(&format!("@font-face {{ font-family: {}; src: url(\"{}\") format(\"truetype\"); }}\n", quoted(family), path.trim_start_matches('/')));
    }
    let optical = if ui.is_some() || bundled { "font-optical-sizing: none; " } else { "" };
    let tracking = ui.and_then(|u| u.tracking).map_or_else(String::new, |t| format!("--ui-tracking: {t}em; "));
    let (first_ui, first_mono) = if bundled { (Some(BUNDLED_UI), Some(BUNDLED_MONO)) } else { (ui.map(|_| SYSTEM_UI), None) };
    out.push_str(&format!(
        ":root {{ {optical}{tracking}--font-ui: {}; --font-mono: {}; {}{}{} }}\n",
        family_list(first_ui, &families.ui, OWN_PROPORTIONAL),
        family_list(first_mono, &families.mono, OWN_MONOSPACE),
        line_heights(lines),
        face_lines(face),
        spacing()
    ));
    let dark = palette(&tokens::DARK);
    out.push_str(&format!(":root {{ {} }}\n", palette(&tokens::LIGHT)));
    out.push_str(&format!("@media (prefers-color-scheme: dark) {{ :root:not([data-theme=\"light\"]) {{ {dark} }} }}\n"));
    out.push_str(&format!(":root[data-theme=\"dark\"] {{ {dark} }}\n"));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn colors_are_written_unmultiplied_and_added_light_over_the_panel() {
        assert_eq!(css_color(Color::gray(248), Color::gray(248)), "rgb(248 248 248)");
        assert_eq!(css_color(Color::additive(5), Color::gray(27)), "rgb(32 32 32)");
        assert_eq!(css_color(Color::black_alpha(96), Color::gray(27)), "rgb(0 0 0 / 0.376)");
        assert_eq!(css_color(Color::TRANSPARENT, Color::gray(27)), "transparent");
        assert_eq!(css_color(Color([60, 60, 60, 128]), Color::gray(27)), "rgb(120 120 120 / 0.502)");
    }

    #[test]
    fn a_family_list_quotes_each_name_and_ends_in_eguis_own_font() {
        let wanted = vec![Wanted::Family(String::from("Segoe UI")), Wanted::Family(String::from("Odd \"Name\""))];
        assert_eq!(family_list(None, &wanted, OWN_PROPORTIONAL), "\"Segoe UI\", \"Odd \\\"Name\\\"\", \"PWRS-GUI Ubuntu Light\"");
        assert_eq!(family_list(None, &[], OWN_MONOSPACE), "\"PWRS-GUI Hack\"");
        assert_eq!(family_list(Some(SYSTEM_UI), &wanted[..1], OWN_PROPORTIONAL), "\"PWRS-GUI System UI\", \"Segoe UI\", \"PWRS-GUI Ubuntu Light\"", "a family the helper gives the page comes first");
    }

    #[test]
    fn with_no_family_of_the_systems_text_is_laid_out_in_eguis_own_fonts() {
        let found = text(&Families::default(), Fonts::System);
        assert_eq!((found.ui.as_deref(), found.mono.as_deref()), (None, None));
        assert!(found.problems.is_empty(), "{:?}", found.problems);
        let own = |data: &[u8], size: f32| proto::typeface::row_height(data, 0, size).expect("egui's own font reads");
        assert_eq!(found.lines.body, Some(own(epaint_default_fonts::UBUNTU_LIGHT, tokens::TEXT.body)));
        assert_eq!(found.lines.heading, Some(own(epaint_default_fonts::UBUNTU_LIGHT, tokens::TEXT.heading)));
        assert_eq!(found.lines.mono, Some(own(epaint_default_fonts::HACK_REGULAR, tokens::TEXT.monospace)));
        let own_face = proto::typeface::face_metrics(epaint_default_fonts::UBUNTU_LIGHT, 0).expect("egui's own face reads");
        assert_eq!(found.ui_face, Some(own_face), "a canvas's text is placed by egui's own proportional face");
    }

    #[test]
    fn with_the_builds_fonts_rows_are_laid_out_in_them_whatever_the_system_has() {
        let found = text(&Families::default(), Fonts::Bundled);
        assert!(found.problems.is_empty(), "{:?}", found.problems);
        let carried = |data: &[u8], size: f32| proto::typeface::row_height(data, 0, size).expect("the build's font reads");
        assert_eq!(found.lines.body, Some(carried(pwrs_gui_fonts::INTER, tokens::TEXT.body)));
        assert_eq!(found.lines.instruction, Some(carried(pwrs_gui_fonts::INTER, tokens::DIALOG.instruction)));
        assert_eq!(found.lines.mono, Some(carried(pwrs_gui_fonts::JETBRAINS_MONO, tokens::TEXT.monospace)));
        let t = &tokens::TEXT;
        for (level, row) in (2..).zip(found.lines.headings) {
            assert_eq!(row, Some(carried(pwrs_gui_fonts::INTER, proto::markdown::heading_size(level, t.body, t.heading))), "a level {level} heading's row");
        }
        assert_eq!(found.lines.space, Some(proto::typeface::space_width(pwrs_gui_fonts::INTER, 0, t.body).expect("Inter's space reads")));
        assert_eq!(found.ui_face, Some(proto::typeface::face_metrics(pwrs_gui_fonts::INTER, 0).expect("Inter reads")));
        assert_eq!(found.ui_font, None, "no system font is given to the page while the build's are drawn");
    }

    #[test]
    fn the_builds_fonts_are_faces_named_first_at_their_default_instance() {
        let families = Families { ui: vec![Wanted::Family(String::from("Segoe UI"))], mono: vec![Wanted::Family(String::from("Consolas"))], problems: Vec::new() };
        let css = stylesheet(&families, &Lines::default(), None, None, Fonts::Bundled);
        assert!(css.contains("@font-face { font-family: \"PWRS-GUI Inter\"; src: url(\"fonts/Inter.ttf\") format(\"truetype\"); }"), "{css}");
        assert!(css.contains("@font-face { font-family: \"PWRS-GUI JetBrains Mono NL\"; src: url(\"fonts/JetBrainsMonoNL.ttf\") format(\"truetype\"); }"), "{css}");
        assert!(css.contains(":root { font-optical-sizing: none; --font-ui: \"PWRS-GUI Inter\", \"Segoe UI\", \"PWRS-GUI Ubuntu Light\"; --font-mono: \"PWRS-GUI JetBrains Mono NL\", \"Consolas\", \"PWRS-GUI Hack\";"), "{css}");
        let system = stylesheet(&families, &Lines::default(), None, None, Fonts::System);
        assert!(!system.contains("PWRS-GUI Inter") && !system.contains("PWRS-GUI JetBrains Mono"), "{system}");
    }

    #[test]
    fn a_page_takes_the_systems_theme_unless_its_root_names_one() {
        let css = stylesheet(&Families::default(), &Lines::default(), None, None, Fonts::Bundled);
        assert!(css.contains("@media (prefers-color-scheme: dark) { :root:not([data-theme=\"light\"]) { --noninteractive-bg: rgb(27 27 27);"), "{css}");
        assert!(css.contains(":root[data-theme=\"dark\"] { --noninteractive-bg: rgb(27 27 27);"), "{css}");
    }

    #[test]
    fn a_served_system_ui_font_is_a_face_named_first_at_its_default_instance() {
        let families = Families { ui: vec![Wanted::Family(String::from("Segoe UI"))], mono: Vec::new(), problems: Vec::new() };
        let css = stylesheet(&families, &Lines::default(), None, Some(ServedUi::default()), Fonts::System);
        assert!(css.contains("@font-face { font-family: \"PWRS-GUI System UI\"; src: url(\"fonts/system-ui.ttf\") format(\"truetype\"); }"), "{css}");
        assert!(css.contains(":root { font-optical-sizing: none; --font-ui: \"PWRS-GUI System UI\", \"Segoe UI\", \"PWRS-GUI Ubuntu Light\";"), "{css}");
    }

    #[test]
    fn a_served_system_ui_fonts_text_is_spaced_by_its_tracking_and_egui_s_own_need_none() {
        let families = Families { ui: vec![Wanted::Family(String::from("SF"))], mono: Vec::new(), problems: Vec::new() };
        let spaced = stylesheet(&families, &Lines::default(), None, Some(ServedUi { tracking: Some(-28.0 / 2048.0) }), Fonts::System);
        assert!(spaced.contains(":root { font-optical-sizing: none; --ui-tracking: -0.013671875em; --font-ui: "), "{spaced}");
        let unspaced = stylesheet(&families, &Lines::default(), None, Some(ServedUi { tracking: None }), Fonts::System);
        assert!(!unspaced.contains("--ui-tracking"), "a face WebKit tracks as egui does is not spaced: {unspaced}");
        assert!(!stylesheet(&families, &Lines::default(), None, None, Fonts::System).contains("--ui-tracking"), "no face is spaced where none is given");
        for own in [epaint_default_fonts::UBUNTU_LIGHT, epaint_default_fonts::HACK_REGULAR, pwrs_gui_fonts::INTER, pwrs_gui_fonts::JETBRAINS_MONO] {
            assert_eq!(proto::typeface::page_tracking(own, 0), Ok(None), "egui's own fonts and the build's have no tracking table");
        }
    }

    #[test]
    fn the_stylesheet_holds_both_themes_the_spacing_and_eguis_fonts() {
        let families = Families { ui: vec![Wanted::Family(String::from("Segoe UI"))], mono: vec![Wanted::Family(String::from("Consolas"))], problems: Vec::new() };
        let headings = [Some(22.5), None, None, None, None];
        let lines = Lines { small: Some(12.0), body: Some(17.28125), button: Some(17.28125), heading: Some(23.9375), instruction: Some(19.9375), mono: None, headings, space: Some(3.25) };
        let face = FaceMetrics { units: 2048.0, ascent: 2210.0, descent: -514.0, gap: 0.0 };
        let css = stylesheet(&families, &lines, Some(face), None, Fonts::System);
        assert!(css.contains("--row-body: 17.28125px;") && css.contains("--row-heading: 23.9375px;") && css.contains("--row-instruction: 19.9375px;"), "{css}");
        assert!(css.contains("--row-heading-2: 22.5px;") && !css.contains("--row-heading-3") && css.contains("--space: 3.25px;"), "a Markdown heading's row and a space, each where it was read: {css}");
        assert!((2..=6).all(|level| css.contains(&format!("--heading-{level}: "))), "each Markdown heading's size: {css}");
        assert!(css.contains("--face-units: 2048; --face-ascent: 2210; --face-descent: -514; --face-gap: 0;"), "the proportional face's metrics, in its units: {css}");
        assert!(!stylesheet(&families, &lines, None, None, Fonts::System).contains("--face-"), "none where they were not read");
        assert!(!css.contains(SYSTEM_UI) && !css.contains("font-optical-sizing"), "a system font is given to the page only where the system UI font is a file: {css}");
        assert!(!css.contains("--row-mono"), "a size whose height was not read is left to the engine: {css}");
        assert!(css.contains("--font-ui: \"Segoe UI\", \"PWRS-GUI Ubuntu Light\";"), "{css}");
        assert!(css.contains("--font-mono: \"Consolas\", \"PWRS-GUI Hack\";"), "{css}");
        assert!(css.contains("--panel: rgb(248 248 248);") && css.contains("--panel: rgb(27 27 27);"), "{css}");
        assert!(css.contains("--item-x: 8px; --item-y: 3px;") && css.contains("--body: 13px;") && css.contains("--grab: 3px;"), "{css}");
        assert!(css.contains("--inactive-bg: rgb(230 230 230);") && css.contains("--inactive-stroke: transparent;"), "{css}");
        assert!(css.contains("@media (prefers-color-scheme: dark)"), "{css}");
        assert!(css.contains("src: url(\"fonts/Ubuntu-Light.ttf\")") && css.contains("src: url(\"fonts/Hack-Regular.ttf\")"), "{css}");
    }

    #[test]
    fn the_peek_button_and_its_eye_are_drawn_from_the_tokens() {
        let p = &tokens::PEEK;
        let [width, height] = p.eye;
        let (across, down) = (width + 2.0 * p.stroke, height + 2.0 * p.stroke);
        let (cx, cy) = (across / 2.0, down / 2.0);
        let css = peek();
        assert!(css.contains(&format!("--peek-button: {}px;", p.button)), "{css}");
        assert!(css.contains(&format!("--peek-eye-w: {across}px; --peek-eye-h: {down}px;")), "the eye's box, a stroke's width of room around it: {css}");
        let almond = format!("M{} {cy}Q{cx} {} {} {cy}Q{cx} {} {} {cy}Z", p.stroke, cy - height, p.stroke + width, cy + height, p.stroke);
        assert!(css.contains(&almond) && css.contains(&format!("stroke-width='{}'", p.stroke)), "the almond egui strokes, its control points the eye's height off its middle: {css}");
        assert!(css.contains(&format!("cx='{cx}' cy='{cy}' r='{}'", p.pupil / 2.0)), "the pupil, filled at the middle: {css}");
        assert!(!css.contains('<') && !css.contains('>') && css.matches('"').count() == 2, "the SVG is escaped inside the url's quotes: {css}");
    }
}
