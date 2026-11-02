//! egui's style built from the tokens both renderers draw from
//! ([`proto::tokens`]): each theme's visuals, the spacing, the text sizes,
//! and the frames around a window's widgets, its bars and its groups, and
//! a dialog's, a window with a button bar.

use super::egui;
use pwrs_gui_proto as proto;
use proto::tokens::{self, Color, Look, Palette, Shadow, Spacing, Stroke, TextSizes};
use std::collections::BTreeMap;

fn color(c: Color) -> egui::Color32 {
    let [r, g, b, a] = c.0;
    egui::Color32::from_rgba_premultiplied(r, g, b, a)
}

fn stroke(s: Stroke) -> egui::Stroke {
    egui::Stroke::new(s.width, color(s.color))
}

fn shadow(s: Shadow) -> egui::Shadow {
    egui::Shadow { offset: s.offset, blur: s.blur, spread: s.spread, color: color(s.color) }
}

fn look(l: &Look) -> egui::style::WidgetVisuals {
    egui::style::WidgetVisuals {
        weak_bg_fill: color(l.weak_bg_fill),
        bg_fill: color(l.bg_fill),
        bg_stroke: stroke(l.bg_stroke),
        fg_stroke: stroke(l.fg_stroke),
        corner_radius: egui::CornerRadius::same(l.corner_radius),
        expansion: l.expansion,
    }
}

/// The visuals of the theme in `p`.
pub(super) fn visuals(p: &Palette) -> egui::Visuals {
    let mut v = if p.dark { egui::Visuals::dark() } else { egui::Visuals::light() };
    v.widgets.noninteractive = look(&p.noninteractive);
    v.widgets.inactive = look(&p.inactive);
    v.widgets.hovered = look(&p.hovered);
    v.widgets.active = look(&p.active);
    v.widgets.open = look(&p.open);
    v.selection.bg_fill = color(p.selection_fill);
    v.selection.stroke = stroke(p.selection_stroke);
    v.hyperlink_color = color(p.hyperlink);
    v.faint_bg_color = color(p.faint_bg);
    v.extreme_bg_color = color(p.extreme_bg);
    v.code_bg_color = color(p.code_bg);
    v.warn_fg_color = color(p.warn);
    v.error_fg_color = color(p.error);
    v.window_corner_radius = egui::CornerRadius::same(p.window_corner_radius);
    v.window_shadow = shadow(p.window_shadow);
    v.window_fill = color(p.window_fill);
    v.window_stroke = stroke(p.window_stroke);
    v.menu_corner_radius = egui::CornerRadius::same(p.menu_corner_radius);
    v.panel_fill = color(p.panel_fill);
    v.popup_shadow = shadow(p.popup_shadow);
    v.text_cursor.stroke = stroke(p.text_cursor);
    v.weak_text_alpha = p.weak_text_alpha;
    v.disabled_alpha = p.disabled_alpha;
    v
}

/// The spacing in `s`.
pub(super) fn spacing(s: &Spacing) -> egui::Spacing {
    egui::Spacing {
        item_spacing: egui::vec2(s.item_spacing[0], s.item_spacing[1]),
        window_margin: egui::Margin::same(s.window_margin),
        menu_margin: egui::Margin::same(s.menu_margin),
        button_padding: egui::vec2(s.button_padding[0], s.button_padding[1]),
        indent: s.indent,
        interact_size: egui::vec2(s.interact_size[0], s.interact_size[1]),
        slider_width: s.slider_width,
        slider_rail_height: s.slider_rail_height,
        combo_width: s.combo_width,
        text_edit_width: s.text_edit_width,
        icon_width: s.icon_width,
        icon_width_inner: s.icon_width_inner,
        icon_spacing: s.icon_spacing,
        default_area_size: egui::vec2(s.default_area_size[0], s.default_area_size[1]),
        tooltip_width: s.tooltip_width,
        menu_width: s.menu_width,
        menu_spacing: s.menu_spacing,
        combo_height: s.combo_height,
        scroll: egui::style::ScrollStyle {
            bar_width: s.scroll_bar_width,
            floating_width: s.scroll_bar_floating_width,
            floating_allocated_width: s.scroll_bar_allocated_width,
            handle_min_length: s.scroll_handle_min_length,
            bar_inner_margin: s.scroll_bar_inner_margin,
            ..egui::style::ScrollStyle::thin()
        },
        ..Default::default()
    }
}

/// The text styles `t` sizes.
pub(super) fn text_styles(t: &TextSizes) -> BTreeMap<egui::TextStyle, egui::FontId> {
    use egui::FontFamily::{Monospace, Proportional};
    BTreeMap::from([
        (egui::TextStyle::Small, egui::FontId::new(t.small, Proportional)),
        (egui::TextStyle::Body, egui::FontId::new(t.body, Proportional)),
        (egui::TextStyle::Button, egui::FontId::new(t.button, Proportional)),
        (egui::TextStyle::Heading, egui::FontId::new(t.heading, Proportional)),
        (egui::TextStyle::Monospace, egui::FontId::new(t.monospace, Monospace)),
    ])
}

/// The text style of a Text of style `instruction`, a dialog's main line,
/// in [`tokens::DIALOG`]'s size.
pub(super) fn instruction() -> egui::TextStyle {
    egui::TextStyle::Name("instruction".into())
}

/// Gives `ctx` the tokens' style in both themes, with the instruction's
/// text style beside egui's own.
pub(super) fn apply(ctx: &egui::Context) {
    ctx.set_visuals_of(egui::Theme::Light, visuals(&tokens::LIGHT));
    ctx.set_visuals_of(egui::Theme::Dark, visuals(&tokens::DARK));
    ctx.all_styles_mut(|style| {
        style.spacing = spacing(&tokens::SPACING);
        style.interaction.resize_grab_radius_side = tokens::SPACING.resize_grab;
        style.text_styles = text_styles(&tokens::TEXT);
        style.text_styles.insert(instruction(), egui::FontId::new(tokens::DIALOG.instruction, egui::FontFamily::Proportional));
    });
}

/// egui's preference for a window's theme.
pub(super) fn theme(t: proto::Theme) -> egui::ThemePreference {
    match t {
        proto::Theme::System => egui::ThemePreference::System,
        proto::Theme::Light => egui::ThemePreference::Light,
        proto::Theme::Dark => egui::ThemePreference::Dark,
    }
}

/// The frame around a window's widgets.
pub(super) fn central_frame(style: &egui::Style) -> egui::Frame {
    egui::Frame::central_panel(style).inner_margin(tokens::SPACING.panel_margin)
}

/// The frame around the widgets of a dialog, a window with a button bar or
/// one its options open as a dialog: a dialog's margin.
pub(super) fn dialog_frame(style: &egui::Style) -> egui::Frame {
    egui::Frame::central_panel(style).inner_margin(dialog_margin())
}

/// The frame of a button bar: a dialog's margin around its buttons, on the
/// theme's strip with `strip`, as Windows draws the strip under a dialog's
/// buttons, and on the window's own color without.
pub(super) fn button_bar_frame(style: &egui::Style, strip: bool) -> egui::Frame {
    let fill = if strip { color(tokens::palette(style.visuals.dark_mode).strip) } else { style.visuals.panel_fill };
    egui::Frame::new().fill(fill).inner_margin(dialog_margin())
}

/// The room in points the line over a button bar takes outside its frame:
/// with `strip`, the line color's width rounded to a whole point, as egui's
/// panel reserves it in the frame's outer margin for its separator line;
/// without, none.
pub(super) fn bar_line(style: &egui::Style, strip: bool) -> i8 {
    if strip { style.visuals.widgets.noninteractive.bg_stroke.width.round() as i8 } else { 0 }
}

fn dialog_margin() -> egui::Margin {
    egui::Margin::same(tokens::DIALOG.margin.round() as i8)
}

/// The frame of a menu bar or a status bar.
pub(super) fn bar_frame(style: &egui::Style) -> egui::Frame {
    let [across, down] = tokens::SPACING.bar_margin;
    egui::Frame::side_top_panel(style).inner_margin(egui::Margin::symmetric(across, down))
}

/// The frame of a group.
pub(super) fn group_frame(style: &egui::Style) -> egui::Frame {
    egui::Frame::group(style).inner_margin(tokens::SPACING.group_margin)
}

/// The margin between a text field's text and its edges.
pub(super) fn text_edit_margin() -> egui::Margin {
    let [across, down] = tokens::SPACING.text_edit_margin;
    egui::Margin::symmetric(across, down)
}

/// Adds the text field `add` adds with its edge in the theme's line color
/// while it is neither hovered nor focused, so a field shows against the
/// window it sits on; hovered and focused, it takes egui's own strokes.
pub(super) fn field<R>(ui: &mut egui::Ui, add: impl FnOnce(&mut egui::Ui) -> R) -> R {
    ui.scope(|ui| {
        let widgets = &mut ui.visuals_mut().widgets;
        widgets.inactive.bg_stroke = widgets.noninteractive.bg_stroke;
        add(ui)
    })
    .inner
}

/// Adds the control `add` adds in a text field's look: the field's fill in
/// every state, its edge in the theme's line color while it is neither
/// hovered nor focused, the hovered stroke under the pointer, the
/// selection's stroke while pressed, focused or open, and its text in the
/// field's color. A number field, a drop-down and a date chooser take it,
/// as WinUI's NumberBox, ComboBox and CalendarDatePicker share TextBox's
/// look.
pub(super) fn well<R>(ui: &mut egui::Ui, add: impl FnOnce(&mut egui::Ui) -> R) -> R {
    ui.scope(|ui| {
        let fill = ui.visuals().text_edit_bg_color();
        let focus = ui.visuals().selection.stroke;
        let widgets = &mut ui.visuals_mut().widgets;
        widgets.inactive.bg_stroke = widgets.noninteractive.bg_stroke;
        let ink = widgets.inactive.fg_stroke;
        for state in [&mut widgets.inactive, &mut widgets.hovered, &mut widgets.active, &mut widgets.open] {
            state.weak_bg_fill = fill;
            state.bg_fill = fill;
            state.fg_stroke = ink;
        }
        widgets.active.bg_stroke = focus;
        widgets.open.bg_stroke = focus;
        add(ui)
    })
    .inner
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_tokens_build_eguis_own_style() {
        assert_eq!(visuals(&tokens::LIGHT), egui::Visuals::light());
        let mut dark = egui::Visuals::dark();
        dark.widgets.noninteractive.fg_stroke.color = egui::Color32::from_gray(200);
        assert_eq!(visuals(&tokens::DARK), dark, "egui's dark visuals but for the labels' gray");
        assert_eq!(spacing(&tokens::SPACING), egui::Spacing { scroll: egui::style::ScrollStyle::thin(), ..egui::Spacing::default() });
        assert_eq!(text_styles(&tokens::TEXT), egui::style::default_text_styles());
        let style = egui::Style::default();
        assert_eq!(central_frame(&style), egui::Frame::central_panel(&style));
        assert_eq!(bar_frame(&style), egui::Frame::side_top_panel(&style));
        assert_eq!(group_frame(&style), egui::Frame::group(&style));
    }

    #[test]
    fn a_field_draws_its_edge_in_the_line_color_while_idle_and_the_widgets_beside_it_keep_eguis() {
        for (palette, theme) in [(tokens::LIGHT, egui::Theme::Light), (tokens::DARK, egui::Theme::Dark)] {
            let ctx = egui::Context::default();
            apply(&ctx);
            ctx.set_theme(theme);
            let own = visuals(&palette).widgets;
            let frame = || {
                let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
                    let mut text = String::from("words");
                    field(ui, |ui| ui.text_edit_singleline(&mut text));
                    ui.add(egui::Button::new("OK"));
                    assert_eq!(ui.visuals().widgets.inactive.bg_stroke, own.inactive.bg_stroke, "the window's own widgets have egui's stroke");
                });
                output.textures_delta.clear();
                output.shapes.into_iter().map(|c| c.shape).collect::<Vec<_>>()
            };
            frame();
            let shapes = frame();
            let edged = shapes.iter().filter(|s| matches!(s, egui::Shape::Rect(r) if r.stroke == own.noninteractive.bg_stroke)).count();
            assert_eq!(edged, 1, "the field alone has the line color's edge in the {theme:?} theme: {shapes:?}");
        }
    }

    #[test]
    fn a_change_to_a_token_reaches_eguis_style() {
        let mut palette = tokens::LIGHT;
        palette.inactive.weak_bg_fill = Color::rgb(1, 2, 3);
        assert_eq!(visuals(&palette).widgets.inactive.weak_bg_fill, egui::Color32::from_rgb(1, 2, 3));
        let mut s = tokens::SPACING;
        s.button_padding = [9.0, 7.0];
        assert_eq!(spacing(&s).button_padding, egui::vec2(9.0, 7.0));
    }
}
