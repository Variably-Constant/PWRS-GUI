//! A Photoshop gradient fill, from a GdFl block or a shape's vscg, drawn as
//! Photoshop draws it. The rules come from the pixels Photoshop stored for
//! the gradient fills of psd-tools', ag-psd's, Krita's and psd.rb's test
//! documents, and they reproduce every linear, reflected, angle and diamond
//! one there within Photoshop's dither.
//!
//! - The gradient runs from a start point to an end point. The end point is
//!   where the line through the center of the gradient's box, at the
//!   descriptor's angle, leaves the box, moved from the center by the
//!   scale; a linear gradient starts opposite it, and a reflected, angle or
//!   diamond gradient at the center. The offset moves the center by
//!   percents of the box. Both points are taken down to whole pixels, and
//!   each pixel is placed at its top left corner.
//! - A linear gradient places a pixel by how far along from start to end it
//!   is, a reflected one the same either side of the start, an angle one
//!   by how far round it turns clockwise from the end's direction, and a
//!   diamond by its distances along and across that direction, added, over
//!   the start to end length.
//! - The classic method at 100% smoothness, with every midpoint at 50,
//!   joins the stops by a Catmull-Rom spline over each span's own 0 to 1, a
//!   stop's tangent half the difference of its neighbors, an end stop
//!   standing in for its missing neighbor, each span held between its two
//!   stops' values, in the document's stored values; its transparency
//!   alike. The linear method moves each span by the midpoint its later
//!   stop holds, eases it by the smoothness, and mixes its colors in the
//!   document's linear light and its transparency as stored.
//!
//! What the merge does not draw comes back as why, for a warning: a radial
//! gradient, the perceptual and smooth methods, a noise gradient, and the
//! classic method at another smoothness or with a midpoint moved.

use std::f64::consts::TAU;

use crate::psd::Mode;
use crate::psd_descriptor::{Descriptor, Value};
use crate::psd_fill::{self, Light, Solid};

/// How a gradient places a pixel along it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Style {
    Linear,
    Reflected,
    Angle,
    Diamond,
}

/// How a gradient mixes between its stops.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Method {
    /// The classic method, at 100% smoothness with every midpoint at 50.
    Classic,
    /// The linear method, at a smoothness from 0 to 1.
    Linear(f64),
}

/// A gradient's stop: its place from 0 to 1, the midpoint of the span that
/// ends at it from 0 to 1, and its value.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Stop<T> {
    pub(crate) location: f64,
    pub(crate) midpoint: f64,
    pub(crate) value: T,
}

/// A gradient fill's settings, as the merge draws them.
#[derive(Debug, PartialEq)]
pub(crate) struct Gradient {
    pub(crate) style: Style,
    /// The angle in degrees, counterclockwise from pointing right.
    pub(crate) angle: f64,
    /// The scale, 1 for 100%.
    pub(crate) scale: f64,
    pub(crate) reverse: bool,
    /// Whether the gradient is laid out over its layer's box rather than
    /// the canvas.
    pub(crate) aligned: bool,
    /// How far the offset moves the center, as fractions of the box's width
    /// and height.
    pub(crate) offset: [f64; 2],
    pub(crate) method: Method,
    /// The color stops, each color as a solid fill reads it.
    pub(crate) colors: Vec<Stop<Solid>>,
    /// The transparency stops, each opacity from 0 to 1.
    pub(crate) opacities: Vec<Stop<f64>>,
}

/// A gradient fill's settings, or, for one the merge does not draw, why, as
/// the end of a warning naming the layer.
#[derive(Debug, PartialEq)]
pub(crate) enum Setting {
    Drawn(Gradient),
    Undrawn(String),
}

/// An enumerated item's value.
fn enumerated<'d>(d: &'d Descriptor, key: &str) -> Option<&'d str> {
    match d.get(key)? {
        Value::Enum(_, value) => Some(value.as_str()),
        _ => None,
    }
}

/// The stops of the gradient's list `key`, in the order of their locations,
/// each value read by `value`: Lctn from 0 to 4096, Mdpn in percent.
fn stops<T>(gradient: &Descriptor, key: &str, value: impl Fn(&Descriptor) -> Result<T, String>) -> Result<Vec<Stop<T>>, String> {
    let Some(Value::List(items)) = gradient.get(key) else { return Ok(Vec::new()) };
    let mut out = Vec::with_capacity(items.len());
    for item in items {
        let Value::Descriptor(stop) = item else { return Err(format!("its gradient's {} list holds an item that is not a stop", key.trim_end())) };
        let location = psd_fill::number(stop, "Lctn").ok_or("its gradient holds a stop with no location")? / 4096.0;
        let midpoint = psd_fill::number(stop, "Mdpn").unwrap_or(50.0) / 100.0;
        out.push(Stop { location, midpoint, value: value(stop)? });
    }
    out.sort_by(|a, b| a.location.total_cmp(&b.location));
    Ok(out)
}

/// The gradient a GdFl block's descriptor, or a shape's vscg, describes in a
/// document of `mode`: its Type, Angl, Scl , Rvrs, Algn, Ofst and
/// gradientsInterpolationMethod, linear, 0 degrees, 100%, not reversed,
/// aligned, not offset and the classic method where it gives none, and its
/// Grad's form, smoothness (Intr, 0 to 4096) and stops, each color read as a
/// solid fill's color is, the transparency opaque where it gives none.
pub(crate) fn gradient_of(d: &Descriptor, mode: Mode) -> Result<Setting, String> {
    let undrawn = |why: String| Ok(Setting::Undrawn(why));
    let style = match enumerated(d, "Type") {
        None | Some("Lnr ") => Style::Linear,
        Some("Rflc") => Style::Reflected,
        Some("Angl") => Style::Angle,
        Some("Dmnd") => Style::Diamond,
        Some("Rdl ") => return undrawn("is filled with a radial gradient, which the merge does not draw".to_string()),
        Some(other) => return undrawn(format!("is filled with a gradient of the style {}, which the merge does not draw", other.trim_end())),
    };
    let gradient = psd_fill::nested(d, "Grad").ok_or("its gradient fill holds no gradient")?;
    match enumerated(gradient, "GrdF") {
        None | Some("CstS") => {}
        Some("ClNs") => return undrawn("is filled with a noise gradient, which the merge does not draw".to_string()),
        Some(other) => return undrawn(format!("is filled with a gradient of the form {}, which the merge does not draw", other.trim_end())),
    }
    let smoothness = psd_fill::number(gradient, "Intr").unwrap_or(4096.0) / 4096.0;
    let colors = stops(gradient, "Clrs", |stop| psd_fill::solid(stop, mode))?;
    if colors.is_empty() {
        return Err("its gradient holds no color stops".to_string());
    }
    let mut opacities = stops(gradient, "Trns", |stop| Ok(psd_fill::number(stop, "Opct").map_or(1.0, |v| (v / 100.0).clamp(0.0, 1.0))))?;
    if opacities.is_empty() {
        opacities.push(Stop { location: 0.0, midpoint: 0.5, value: 1.0 });
    }
    let method = match enumerated(d, "gradientsInterpolationMethod") {
        None | Some("Gcls") => {
            if smoothness != 1.0 {
                return undrawn(format!("is filled with a classic gradient of {:.0}% smoothness, which the merge draws at 100% only", smoothness * 100.0));
            }
            let later = colors.iter().skip(1).map(|s| s.midpoint).chain(opacities.iter().skip(1).map(|s| s.midpoint));
            if later.into_iter().any(|m| m != 0.5) {
                return undrawn("is filled with a classic gradient with a midpoint moved from 50%, which the merge draws at 50% only".to_string());
            }
            Method::Classic
        }
        Some("Lnr ") => Method::Linear(smoothness),
        Some("Perc") => return undrawn("is filled with a gradient of the perceptual method, which the merge does not draw".to_string()),
        Some("Smoo") => return undrawn("is filled with a gradient of the smooth method, which the merge does not draw".to_string()),
        Some(other) => return undrawn(format!("is filled with a gradient of the method {}, which the merge does not draw", other.trim_end())),
    };
    let offset = psd_fill::nested(d, "Ofst").map_or([0.0, 0.0], |point| [psd_fill::number(point, "Hrzn").unwrap_or(0.0) / 100.0, psd_fill::number(point, "Vrtc").unwrap_or(0.0) / 100.0]);
    Ok(Setting::Drawn(Gradient {
        style,
        angle: psd_fill::number(d, "Angl").unwrap_or(0.0),
        scale: psd_fill::number(d, "Scl ").unwrap_or(100.0) / 100.0,
        reverse: d.flag("Rvrs").unwrap_or(false),
        aligned: d.flag("Algn").unwrap_or(true),
        offset,
        method,
        colors,
        opacities,
    }))
}

/// `v` taken down to a whole pixel, or to the nearest whole pixel where it
/// is within 16 epsilons of `magnitude` of one, `magnitude` the size of the
/// numbers it was made from, as their rounding leaves it.
fn whole(v: f64, magnitude: f64) -> f64 {
    let nearest = v.round();
    if (v - nearest).abs() <= 16.0 * f64::EPSILON * magnitude { nearest } else { v.floor() }
}

/// The size of the numbers a gradient's start and end over `bounds` are
/// made from: the largest of the box's edges, the points' coordinates and
/// 1.
pub(crate) fn operands(bounds: [f64; 4], start: [f64; 2], end: [f64; 2]) -> f64 {
    bounds.iter().chain(&start).chain(&end).fold(1.0_f64, |m, v| m.max(v.abs()))
}

/// The unit direction `angle` degrees counterclockwise from pointing right
/// points, y growing down the canvas, exact at each quarter turn.
fn direction(angle: f64) -> (f64, f64) {
    let turned = angle.rem_euclid(360.0);
    if turned == 0.0 {
        (1.0, 0.0)
    } else if turned == 90.0 {
        (0.0, -1.0)
    } else if turned == 180.0 {
        (-1.0, 0.0)
    } else if turned == 270.0 {
        (0.0, 1.0)
    } else {
        let radians = turned.to_radians();
        (radians.cos(), -radians.sin())
    }
}

/// The mean of a straight line and smoothstep at `u`, the classic method's
/// span between two stops alone.
fn half(u: f64) -> f64 {
    (u + u * u * (3.0 - 2.0 * u)) / 2.0
}

/// A channel's value at `t` by the classic method's spline through `stops`,
/// their locations and values in order, `tangents` each stop's: a value
/// past either end the end stop's.
fn spline(stops: &[(f64, f64)], tangents: &[f64], t: f64) -> f64 {
    let n = stops.len();
    if n == 1 || t <= stops[0].0 {
        return stops[0].1;
    }
    if t >= stops[n - 1].0 {
        return stops[n - 1].1;
    }
    let k = stops.partition_point(|s| s.0 <= t) - 1;
    let ((l0, v0), (l1, v1)) = (stops[k], stops[k + 1]);
    let u = (t - l0) / (l1 - l0);
    let (u2, u3) = (u * u, u * u * u);
    let curve = (2.0 * u3 - 3.0 * u2 + 1.0) * v0 + (u3 - 2.0 * u2 + u) * tangents[k] + (-2.0 * u3 + 3.0 * u2) * v1 + (u3 - u2) * tangents[k + 1];
    curve.clamp(v0.min(v1), v0.max(v1))
}

/// Each stop's tangent for the classic method's spline: half the
/// difference of its neighbors' values, an end stop standing in for its
/// missing neighbor.
fn tangents(stops: &[(f64, f64)]) -> Vec<f64> {
    let n = stops.len();
    (0..n).map(|k| (stops[(k + 1).min(n - 1)].1 - stops[k.saturating_sub(1)].1) / 2.0).collect()
}

/// A value at `t` by the linear method between `stops`, their locations,
/// midpoints and values in order, at `smoothness`: the span's place bent so
/// that its later stop's midpoint falls halfway, then eased by the
/// smoothness's share of the classic method's span, a value past either
/// end the end stop's.
fn linear_mix(stops: &[(f64, f64, f64)], smoothness: f64, t: f64) -> f64 {
    let n = stops.len();
    if n == 1 || t <= stops[0].0 {
        return stops[0].2;
    }
    if t >= stops[n - 1].0 {
        return stops[n - 1].2;
    }
    let k = stops.partition_point(|s| s.0 <= t) - 1;
    let ((l0, _, v0), (l1, m, v1)) = (stops[k], stops[k + 1]);
    let u = (t - l0) / (l1 - l0);
    let bent = if u < m {
        0.5 * u / m
    } else if m < 1.0 {
        0.5 + 0.5 * (u - m) / (1.0 - m)
    } else {
        1.0
    };
    v0 + (v1 - v0) * ((1.0 - smoothness) * bent + smoothness * half(bent))
}

impl Gradient {
    /// The gradient's start and end laid out over `bounds`, the box's left,
    /// top, right and bottom, before they are taken down to whole pixels.
    pub(crate) fn ends(&self, bounds: [f64; 4]) -> ([f64; 2], [f64; 2]) {
        let [left, top, right, bottom] = bounds;
        let (w, h) = (right - left, bottom - top);
        let center = [(left + right) / 2.0 + self.offset[0] * w, (top + bottom) / 2.0 + self.offset[1] * h];
        let (dx, dy) = direction(self.angle);
        let (ex, ey) = if dy == 0.0 || (dx != 0.0 && w / 2.0 * dy.abs() <= h / 2.0 * dx.abs()) {
            ((w / 2.0).copysign(dx), w / 2.0 * dy / dx.abs())
        } else {
            (h / 2.0 * dx / dy.abs(), (h / 2.0).copysign(dy))
        };
        let end = [center[0] + ex * self.scale, center[1] + ey * self.scale];
        let start = if self.style == Style::Linear { [center[0] - ex * self.scale, center[1] - ey * self.scale] } else { center };
        (start, end)
    }

    /// Where along the gradient each pixel of `area`, its left, top, width
    /// and height in the canvas, falls, from 0 to 1, row by row, the
    /// gradient laid out over `bounds`, the box's left, top, right and
    /// bottom; `None` where its start and end are the same pixel.
    pub(crate) fn places(&self, bounds: [f64; 4], area: [i64; 4]) -> Option<Vec<f64>> {
        let (start, end) = self.ends(bounds);
        let magnitude = operands(bounds, start, end);
        let (ax, ay) = (whole(start[0], magnitude), whole(start[1], magnitude));
        let (vx, vy) = (whole(end[0], magnitude) - ax, whole(end[1], magnitude) - ay);
        let span = vx * vx + vy * vy;
        if span == 0.0 {
            return None;
        }
        let reach = span.sqrt();
        let (ux, uy) = (vx / reach, vy / reach);
        let heading = (-vy).atan2(vx);
        let [x0, y0, width, height] = area;
        let mut out = Vec::with_capacity((width.max(0) * height.max(0)) as usize);
        for y in y0..y0 + height {
            for x in x0..x0 + width {
                let (px, py) = (x as f64 - ax, y as f64 - ay);
                let t = match self.style {
                    Style::Linear => (px * vx + py * vy) / span,
                    Style::Reflected => ((px * vx + py * vy) / span).abs(),
                    Style::Angle => (heading - (-py).atan2(px)).rem_euclid(TAU) / TAU,
                    Style::Diamond => ((px * ux + py * uy).abs() + (py * ux - px * uy).abs()) / reach,
                }
                .clamp(0.0, 1.0);
                out.push(if self.reverse { 1.0 - t } else { t });
            }
        }
        Some(out)
    }

    /// Each channel's values at `places`, given each color stop's values in
    /// the document's samples, in the order of the stops, channel by
    /// channel: by the classic method's spline, or the linear method's mix
    /// in the light `light` gives, which the linear method needs.
    pub(crate) fn colors_at(&self, places: &[f64], values: &[Vec<f64>], light: Option<&Light>) -> Vec<Vec<f32>> {
        let channels = values.first().map_or(0, Vec::len);
        if let (Method::Linear(smoothness), Some(light)) = (self.method, light) {
            let lights: Vec<Vec<f64>> = values
                .iter()
                .map(|v| {
                    let mut out = vec![0.0; light.width];
                    (light.into)(v, &mut out);
                    out
                })
                .collect();
            let stops: Vec<Vec<(f64, f64, f64)>> = (0..light.width).map(|k| self.colors.iter().zip(&lights).map(|(s, l)| (s.location, s.midpoint, l[k])).collect()).collect();
            let mut planes = vec![Vec::with_capacity(places.len()); channels];
            let (mut mixed, mut samples) = (vec![0.0; light.width], vec![0.0; channels]);
            for &t in places {
                for (m, stops) in mixed.iter_mut().zip(&stops) {
                    *m = linear_mix(stops, smoothness, t);
                }
                (light.back)(&mixed, &mut samples);
                for (plane, &v) in planes.iter_mut().zip(&samples) {
                    plane.push(v as f32);
                }
            }
            return planes;
        }
        (0..channels)
            .map(|c| {
                let stops: Vec<(f64, f64)> = self.colors.iter().zip(values).map(|(s, v)| (s.location, v[c])).collect();
                let tangents = tangents(&stops);
                places.iter().map(|&t| spline(&stops, &tangents, t) as f32).collect()
            })
            .collect()
    }

    /// The opacity at `places`, from the transparency stops, by the classic
    /// method's spline or the linear method's mix in their own values.
    pub(crate) fn opacity_at(&self, places: &[f64]) -> Vec<f32> {
        match self.method {
            Method::Linear(smoothness) => {
                let stops: Vec<(f64, f64, f64)> = self.opacities.iter().map(|s| (s.location, s.midpoint, s.value)).collect();
                places.iter().map(|&t| linear_mix(&stops, smoothness, t) as f32).collect()
            }
            Method::Classic => {
                let stops: Vec<(f64, f64)> = self.opacities.iter().map(|s| (s.location, s.value)).collect();
                let tangents = tangents(&stops);
                places.iter().map(|&t| spline(&stops, &tangents, t) as f32).collect()
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A descriptor of `class` holding `items`.
    fn descriptor(class: &str, items: Vec<(&str, Value)>) -> Descriptor {
        Descriptor { class: class.to_string(), items: items.into_iter().map(|(key, value)| (key.to_string(), value)).collect() }
    }

    /// An enumerated value.
    fn choice(kind: &str, value: &str) -> Value {
        Value::Enum(kind.to_string(), value.to_string())
    }

    /// A number in the unit of the 4-byte `code`.
    fn unit(code: &str, v: f64) -> Value {
        Value::Unit(code.to_string(), v)
    }

    /// A gray color stop at `location` of 4096, its `midpoint` in percent
    /// and its `gray` in percent of black.
    fn gray_stop(location: i64, midpoint: i64, gray: f64) -> Value {
        let color = descriptor("Grsc", vec![("Gry ", Value::Double(gray))]);
        Value::Descriptor(descriptor("Clrt", vec![("Clr ", Value::Descriptor(color)), ("Type", choice("Clry", "UsrS")), ("Lctn", Value::Integer(location)), ("Mdpn", Value::Integer(midpoint))]))
    }

    /// A transparency stop at `location` of 4096, its `midpoint` and
    /// `opacity` in percent.
    fn opacity_stop(location: i64, midpoint: i64, opacity: f64) -> Value {
        Value::Descriptor(descriptor("TrnS", vec![("Opct", unit("#Prc", opacity)), ("Lctn", Value::Integer(location)), ("Mdpn", Value::Integer(midpoint))]))
    }

    /// Black to white, the midpoint at 50.
    fn ramp() -> Vec<Value> {
        vec![gray_stop(0, 50, 100.0), gray_stop(4096, 50, 0.0)]
    }

    /// A gradient fill's descriptor: a custom gradient of `colors`, opaque,
    /// at `smoothness` of 4096, with `items` beside its Grad.
    fn fill(smoothness: f64, colors: Vec<Value>, items: Vec<(&str, Value)>) -> Descriptor {
        let opacities = vec![opacity_stop(0, 50, 100.0), opacity_stop(4096, 50, 100.0)];
        let gradient = descriptor("Grdn", vec![("GrdF", choice("GrdF", "CstS")), ("Intr", Value::Double(smoothness)), ("Clrs", Value::List(colors)), ("Trns", Value::List(opacities))]);
        let mut all = vec![("Grad", Value::Descriptor(gradient))];
        all.extend(items);
        descriptor("null", all)
    }

    /// The linear method's item.
    fn linear_method() -> (&'static str, Value) {
        ("gradientsInterpolationMethod", choice("gradientInterpolationMethodType", "Lnr "))
    }

    /// An offset of `x` and `y` percent of the box.
    fn offset(x: f64, y: f64) -> Value {
        Value::Descriptor(descriptor("Pnt ", vec![("Hrzn", unit("#Prc", x)), ("Vrtc", unit("#Prc", y))]))
    }

    /// The gradient a gray document's fill `d` describes, which the merge
    /// draws.
    fn drawn(d: &Descriptor) -> Gradient {
        match gradient_of(d, Mode::Gray).expect("the fill reads") {
            Setting::Drawn(gradient) => gradient,
            Setting::Undrawn(why) => panic!("the merge leaves it out: it {why}"),
        }
    }

    /// Why the merge leaves out a gray document's fill `d`.
    fn undrawn(d: &Descriptor) -> String {
        match gradient_of(d, Mode::Gray).expect("the fill reads") {
            Setting::Undrawn(why) => why,
            Setting::Drawn(gradient) => panic!("the merge draws it: {gradient:?}"),
        }
    }

    /// A fill with no Type is linear and each style the merge draws is read
    /// by its code; with no method it is the classic one, the linear method
    /// keeps its smoothness, and the angle, scale, reverse, alignment and
    /// offset take their defaults where the fill holds none.
    #[test]
    fn reads_each_style_method_and_placement() {
        let bare = drawn(&fill(4096.0, ramp(), Vec::new()));
        assert_eq!((bare.style, bare.method, bare.angle, bare.scale, bare.reverse, bare.aligned, bare.offset), (Style::Linear, Method::Classic, 0.0, 1.0, false, true, [0.0, 0.0]));
        let stops: Vec<(f64, f64, &Solid)> = bare.colors.iter().map(|s| (s.location, s.midpoint, &s.value)).collect();
        assert_eq!(stops, [(0.0, 0.5, &Solid::Color(vec![0.0])), (1.0, 0.5, &Solid::Color(vec![1.0]))]);
        assert_eq!(bare.opacities.iter().map(|s| (s.location, s.value)).collect::<Vec<_>>(), [(0.0, 1.0), (1.0, 1.0)]);
        for (code, style) in [("Lnr ", Style::Linear), ("Rflc", Style::Reflected), ("Angl", Style::Angle), ("Dmnd", Style::Diamond)] {
            assert_eq!(drawn(&fill(4096.0, ramp(), vec![("Type", choice("GrdT", code))])).style, style);
        }
        let placed = drawn(&fill(1638.0, ramp(), vec![linear_method(), ("Angl", unit("#Ang", -90.0)), ("Scl ", unit("#Prc", 48.0)), ("Rvrs", Value::Bool(true)), ("Algn", Value::Bool(false)), ("Ofst", offset(-2.5, 25.0))]));
        assert_eq!((placed.method, placed.angle, placed.scale, placed.reverse, placed.aligned, placed.offset), (Method::Linear(1638.0 / 4096.0), -90.0, 0.48, true, false, [-0.025, 0.25]));
    }

    /// What the merge does not draw comes back as why: a radial gradient or
    /// a style it does not know, a noise gradient, the perceptual and smooth
    /// methods, and the classic method at another smoothness than 100% or
    /// with a midpoint moved.
    #[test]
    fn says_why_it_leaves_a_gradient_out() {
        let style = |code: &str| undrawn(&fill(4096.0, ramp(), vec![("Type", choice("GrdT", code))]));
        assert_eq!(style("Rdl "), "is filled with a radial gradient, which the merge does not draw");
        assert_eq!(style("Spir"), "is filled with a gradient of the style Spir, which the merge does not draw");
        let mut noise = fill(4096.0, ramp(), Vec::new());
        if let Value::Descriptor(gradient) = &mut noise.items[0].1 {
            gradient.items[0].1 = choice("GrdF", "ClNs");
        }
        assert_eq!(undrawn(&noise), "is filled with a noise gradient, which the merge does not draw");
        let method = |code: &str| undrawn(&fill(4096.0, ramp(), vec![("gradientsInterpolationMethod", choice("gradientInterpolationMethodType", code))]));
        assert_eq!(method("Perc"), "is filled with a gradient of the perceptual method, which the merge does not draw");
        assert_eq!(method("Smoo"), "is filled with a gradient of the smooth method, which the merge does not draw");
        assert_eq!(undrawn(&fill(2048.0, ramp(), Vec::new())), "is filled with a classic gradient of 50% smoothness, which the merge draws at 100% only");
        let moved = vec![gray_stop(0, 50, 100.0), gray_stop(4096, 30, 0.0)];
        assert_eq!(undrawn(&fill(4096.0, moved, Vec::new())), "is filled with a classic gradient with a midpoint moved from 50%, which the merge draws at 50% only");
    }

    /// The end is where the line through the box's center at the angle
    /// leaves the box, through its side or its top or bottom by which comes
    /// first; a linear gradient starts opposite it and the other styles at
    /// the center, the scale moves both from the center, and the offset
    /// moves the center by fractions of the box.
    #[test]
    fn lays_its_ends_out_over_the_box() {
        let square = [0.0, 0.0, 4.0, 4.0];
        let ends = |items: Vec<(&str, Value)>| drawn(&fill(4096.0, ramp(), items)).ends(square);
        let angle = |degrees: f64| ("Angl", unit("#Ang", degrees));
        assert_eq!(ends(Vec::new()), ([0.0, 2.0], [4.0, 2.0]));
        assert_eq!(ends(vec![angle(90.0)]), ([2.0, 4.0], [2.0, 0.0]));
        assert_eq!(ends(vec![angle(180.0)]), ([4.0, 2.0], [0.0, 2.0]));
        assert_eq!(ends(vec![angle(-90.0)]), ([2.0, 0.0], [2.0, 4.0]));
        assert_eq!(ends(vec![("Scl ", unit("#Prc", 50.0))]), ([1.0, 2.0], [3.0, 2.0]));
        assert_eq!(ends(vec![("Ofst", offset(25.0, 0.0))]), ([1.0, 2.0], [5.0, 2.0]));
        assert_eq!(ends(vec![("Type", choice("GrdT", "Dmnd"))]), ([2.0, 2.0], [4.0, 2.0]));
        let (dx, dy) = (30f64.to_radians().cos(), -(30f64.to_radians().sin()));
        assert_eq!(ends(vec![("Type", choice("GrdT", "Rflc")), angle(30.0)]), ([2.0, 2.0], [4.0, 2.0 + 2.0 * dy / dx]));
        let (dx, dy) = (45f64.to_radians().cos(), -(45f64.to_radians().sin()));
        let wide = drawn(&fill(4096.0, ramp(), vec![angle(45.0)])).ends([0.0, 0.0, 8.0, 4.0]);
        assert_eq!(wide, ([4.0 - 2.0 * dx / dy.abs(), 4.0], [4.0 + 2.0 * dx / dy.abs(), 0.0]));
    }

    /// Each pixel is placed at its top left corner along the gradient
    /// between its ends taken down to whole pixels: a reflected gradient
    /// the same either side of its start, a reversed one from its end, an
    /// angle gradient by how far it turns clockwise from the end, and a
    /// diamond by its distances along and across, added.
    #[test]
    fn places_each_pixel_at_its_corner() {
        let square = [0.0, 0.0, 4.0, 4.0];
        let gradient = |items: Vec<(&str, Value)>| drawn(&fill(4096.0, ramp(), items));
        let reflected = gradient(vec![("Type", choice("GrdT", "Rflc")), ("Angl", unit("#Ang", 30.0))]);
        let places = reflected.places(square, [0, 0, 4, 4]).expect("placed");
        assert_eq!(places, [0.0, 0.25, 0.5, 0.75, 0.25, 0.0, 0.25, 0.5, 0.5, 0.25, 0.0, 0.25, 0.75, 0.5, 0.25, 0.0]);
        let reversed = gradient(vec![("Rvrs", Value::Bool(true))]);
        assert_eq!(reversed.places(square, [0, 1, 4, 1]).expect("placed"), [1.0, 0.75, 0.5, 0.25]);
        let angle = gradient(vec![("Type", choice("GrdT", "Angl"))]);
        let at = |g: &Gradient, x: i64, y: i64| g.places(square, [x, y, 1, 1]).expect("placed")[0];
        assert_eq!([at(&angle, 3, 2), at(&angle, 2, 3), at(&angle, 1, 2), at(&angle, 2, 1)], [0.0, 0.25, 0.5, 0.75]);
        let diamond = gradient(vec![("Type", choice("GrdT", "Dmnd"))]);
        assert_eq!([at(&diamond, 2, 2), at(&diamond, 3, 2), at(&diamond, 3, 3), at(&diamond, 2, 0)], [0.0, 0.5, 1.0, 1.0]);
        let short = gradient(vec![("Type", choice("GrdT", "Rflc")), ("Scl ", unit("#Prc", 10.0))]);
        assert_eq!(short.places(square, [0, 0, 4, 4]), None);
    }

    /// A start or end within the rounding of the arithmetic that made it of
    /// a whole pixel is taken as that pixel, as Photoshop takes the start of
    /// psd-tools' adjustment_clipping.psd's Gradient Fill 1, which its offset
    /// and scale put 0.00000000000011 px above the canvas; any other point
    /// is taken down.
    #[test]
    fn takes_a_point_within_rounding_of_a_whole_pixel_as_that_pixel() {
        assert_eq!(whole(-1.1368683772161603e-13, 200.27247619628895), 0.0);
        assert_eq!(whole(0.8452994616207488, 4.0), 0.0);
        assert_eq!(whole(-0.14, 100.0), -1.0);
        assert_eq!(whole(3.0 - 1e-9, 4.0), 2.0);
        let canvas = [0.0, 0.0, 200.0, 200.0];
        let gradient = drawn(&fill(1638.0, ramp(), vec![linear_method(), ("Angl", unit("#Ang", -90.0)), ("Scl ", unit("#Prc", 100.13623809814453)), ("Algn", Value::Bool(false)), ("Ofst", offset(-2.375000000000057, 0.06811904907220878))]));
        let (start, _) = gradient.ends(canvas);
        assert!(start[1] < 0.0, "the start falls short of the top by rounding, at {}", start[1]);
        assert_eq!(gradient.places(canvas, [95, 0, 1, 2]).expect("placed"), [0.0, 1.0 / 200.0]);
    }

    /// The classic method between two stops is the mean of a straight line
    /// and smoothstep; with more, each span follows a Catmull-Rom spline,
    /// each stop's tangent half the difference of its neighbors, held
    /// between its two stops' values.
    #[test]
    fn joins_stops_by_the_classic_spline() {
        let two = [(0.0, 0.0), (1.0, 1.0)];
        let slopes = tangents(&two);
        assert_eq!(spline(&two, &slopes, 0.25), 0.203125);
        assert_eq!(spline(&two, &slopes, 0.25), half(0.25));
        let three = [(0.0, 0.0), (0.5, 1.0), (1.0, 0.75)];
        let slopes = tangents(&three);
        assert_eq!(slopes, [0.5, 0.375, -0.125]);
        assert_eq!([0.25, 0.55, 0.75, 1.0].map(|t| spline(&three, &slopes, t)), [0.515625, 1.0, 0.9375, 0.75]);
    }

    /// The linear method bends each span so that its later stop's midpoint
    /// falls halfway, then eases it by the smoothness's share of the
    /// classic method's span.
    #[test]
    fn bends_each_span_by_its_midpoint_for_the_linear_method() {
        let stops = [(0.0, 0.5, 0.0), (1.0, 0.25, 1.0)];
        assert_eq!([0.125, 0.25, 0.625].map(|t| linear_mix(&stops, 0.0, t)), [0.25, 0.5, 0.75]);
        assert_eq!([0.125, 0.25].map(|t| linear_mix(&stops, 1.0, t)), [half(0.25), 0.5]);
        assert_eq!(linear_mix(&stops, 0.5, 0.125), 0.5 * 0.25 + 0.5 * half(0.25));
    }

    /// The linear method mixes colors in the light the document's samples
    /// stand for, here their square; the classic method mixes them as
    /// stored.
    #[test]
    fn mixes_the_linear_methods_colors_in_light() {
        let light = Light { width: 1, into: Box::new(|from, to| to[0] = from[0] * from[0]), back: Box::new(|from, to| to[0] = from[0].sqrt()) };
        let values = [vec![0.0], vec![1.0]];
        let linear = drawn(&fill(0.0, ramp(), vec![linear_method()]));
        assert_eq!(linear.colors_at(&[0.5], &values, Some(&light)), [vec![0.5f64.sqrt() as f32]]);
        let classic = drawn(&fill(4096.0, ramp(), Vec::new()));
        assert_eq!(classic.colors_at(&[0.5], &values, Some(&light)), [vec![0.5]]);
    }
}
