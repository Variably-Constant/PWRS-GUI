//! A MediaPlayer's controls, drawn from the shared tokens as the HTML
//! renderer draws them ([`tokens::MEDIA_BAR`]): play and pause, the
//! position and the length, a seek slider that seeks to key frames while it
//! is dragged and to the frame where it is let go, the mute and the volume,
//! and for a picture the full screen switch.

use crate::egui;
use proto::media::{MediaState, Phase};
use proto::tokens::{self, Icon, MEDIA_BAR};
use pwrs_gui_proto as proto;

/// How tall the controls' bar is, in points.
pub(super) const MEDIA_HEIGHT: f32 = MEDIA_BAR.height;

/// What a person asked of a player through its controls in one frame.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum Asked {
    /// Play, or pause a clip playing.
    Toggle,
    /// Move to `seconds`: to the frame where `exact`, else to a key frame,
    /// as while the slider is dragged.
    Seek { seconds: f64, exact: bool },
    /// Silence the sound, or let it be heard again.
    Mute,
    Volume(f64),
    /// Full screen, or back from it.
    FullScreen,
}

/// A color of the tokens as egui's.
fn color(c: tokens::Color) -> egui::Color32 {
    let [r, g, b, a] = c.0;
    egui::Color32::from_rgba_unmultiplied(r, g, b, a)
}

/// Paints `icon` inside `rect` in `ink`.
fn paint_icon(painter: &egui::Painter, rect: egui::Rect, icon: &Icon, ink: egui::Color32) {
    let at = |p: &[f32; 2]| egui::pos2(rect.min.x + p[0] * rect.width(), rect.min.y + p[1] * rect.height());
    for fill in icon.fills {
        painter.add(egui::Shape::convex_polygon(fill.iter().map(at).collect(), ink, egui::Stroke::NONE));
    }
    for line in icon.strokes {
        painter.add(egui::Shape::line(line.iter().map(at).collect(), egui::Stroke::new(MEDIA_BAR.stroke, ink)));
    }
}

/// A clock as players write one: minutes and seconds.
pub(super) fn clock(seconds: f64) -> String {
    let whole = seconds.max(0.0).floor() as u64;
    format!("{}:{:02}", whole / 60, whole % 60)
}

/// A button of the bar at `rect`, showing `icon`; whether it was clicked.
fn button(ui: &mut egui::Ui, id: egui::Id, rect: egui::Rect, icon: &Icon, ink: egui::Color32) -> bool {
    let response = ui.interact(rect, id, egui::Sense::click());
    if response.hovered() {
        ui.painter().rect_filled(rect, MEDIA_BAR.button / 4.0, ink.gamma_multiply(0.15));
    }
    paint_icon(ui.painter(), rect.shrink(MEDIA_BAR.button / 8.0), icon, ink);
    response.clicked()
}

/// A slider of the bar along `rect`, filled to `share` of it; the share
/// where the pointer drags or clicks it, and whether it was let go or
/// clicked there.
fn slider(ui: &mut egui::Ui, id: egui::Id, rect: egui::Rect, share: f32, ink: egui::Color32) -> Option<(f32, bool)> {
    let response = ui.interact(rect, id, egui::Sense::click_and_drag());
    let track = egui::Rect::from_center_size(rect.center(), egui::vec2(rect.width(), MEDIA_BAR.track));
    let painter = ui.painter();
    painter.rect_filled(track, MEDIA_BAR.track / 2.0, ink.gamma_multiply(0.35));
    let filled = egui::Rect::from_min_max(track.min, egui::pos2(track.min.x + track.width() * share.clamp(0.0, 1.0), track.max.y));
    painter.rect_filled(filled, MEDIA_BAR.track / 2.0, ink);
    painter.circle_filled(egui::pos2(filled.max.x, track.center().y), MEDIA_BAR.knob / 2.0, ink);
    let pointer = response.interact_pointer_pos()?;
    let at = ((pointer.x - track.min.x) / track.width().max(1.0)).clamp(0.0, 1.0);
    if response.drag_stopped() || response.clicked() {
        return Some((at, true));
    }
    response.dragged().then_some((at, false))
}

/// Draws the bar across `bar` for a player in `state`, with the full
/// screen switch where `picture`, `full` saying which way it points; what
/// the person asked of it this frame.
pub(super) fn draw(ui: &mut egui::Ui, id: egui::Id, bar: egui::Rect, state: &MediaState, picture: bool, full: bool) -> Vec<Asked> {
    let ink = color(MEDIA_BAR.ink);
    ui.painter().rect_filled(bar, 0.0, color(MEDIA_BAR.shade));
    let mut asked = Vec::new();
    let (side, gap) = (MEDIA_BAR.button, MEDIA_BAR.gap);
    let middle = bar.center().y;
    let square = |x: f32| egui::Rect::from_min_size(egui::pos2(x, middle - side / 2.0), egui::vec2(side, side));
    let mut x = bar.min.x + gap;
    let playing = state.phase == Phase::Playing;
    if button(ui, id.with("play"), square(x), if playing { &tokens::PAUSE } else { &tokens::PLAY }, ink) {
        asked.push(Asked::Toggle);
    }
    x += side + gap;
    let length = state.duration.unwrap_or(0.0);
    let text = format!("{} / {}", clock(state.position), clock(length));
    let galley = ui.painter().layout_no_wrap(text, egui::FontId::proportional(proto::tokens::TEXT.small + 2.0), ink);
    ui.painter().galley(egui::pos2(x, middle - galley.size().y / 2.0), galley.clone(), ink);
    x += galley.size().x + gap;
    let mut right = bar.max.x - gap;
    if picture {
        right -= side;
        if button(ui, id.with("full"), square(right), if full { &tokens::WINDOWED } else { &tokens::FULL }, ink) {
            asked.push(Asked::FullScreen);
        }
        right -= gap;
    }
    let volume = egui::Rect::from_min_max(egui::pos2(right - MEDIA_BAR.volume, middle - side / 2.0), egui::pos2(right, middle + side / 2.0));
    if let Some((at, _)) = slider(ui, id.with("volume"), volume, if state.muted { 0.0 } else { state.volume as f32 }, ink) {
        asked.push(Asked::Volume(f64::from(at)));
    }
    right -= MEDIA_BAR.volume + gap + side;
    if button(ui, id.with("mute"), square(right), if state.muted { &tokens::MUTED } else { &tokens::SOUND }, ink) {
        asked.push(Asked::Mute);
    }
    right -= gap;
    if right - x > MEDIA_BAR.knob {
        let seek = egui::Rect::from_min_max(egui::pos2(x, middle - side / 2.0), egui::pos2(right, middle + side / 2.0));
        let share = if length > 0.0 { (state.position / length) as f32 } else { 0.0 };
        if let Some((at, exact)) = slider(ui, id.with("seek"), seek, share, ink)
            && length > 0.0
        {
            asked.push(Asked::Seek { seconds: f64::from(at) * length, exact });
        }
    }
    asked
}
