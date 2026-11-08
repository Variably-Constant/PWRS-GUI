//! The Canvas widget: its items as lyon makes their triangles, in physical
//! pixels from the canvas's top left, drawn offscreen as a GpuSurface is.
//! A paint callback records them into a texture of the canvas's size,
//! [`SAMPLES`] samples to the pixel, a Group that is translucent or clips
//! first drawing its items into a layer of its own that is drawn back
//! through its clip shape, or the whole canvas, at its opacity; the resolved
//! texture is blended into egui's pass. Text is laid out by egui and drawn
//! from the outlines of the glyphs egui laid out, which the patched epaint
//! gives, filled and stroked as a path is. A picture is the texture egui's
//! image loaders make of it, which the callback reaches through the
//! textures the patched egui-wgpu gives it. Every item named, or inside a
//! named Group, is hit-tested where it was drawn, the topmost taking the
//! pointer's events. A view places the items, moved by the gestures and
//! keys its controls take ([`crate::view`]) and by the session's calls.

use crate::view::{self, Taken, ViewState};
use crate::{Inputs, Node, egui, event, flag, opt_points, pointer, proto};
use arc_swap::ArcSwapOption;
use crossbeam_queue::SegQueue;
use eframe::egui_wgpu;
use lyon_algorithms::hit_test::hit_test_path;
use lyon_algorithms::measure::{PathMeasurements, SampleType};
use lyon_tessellation::math::{Point, Transform, point};
use lyon_tessellation::path::iterator::PathIterator;
use lyon_tessellation::path::{Path, PathEvent};
use lyon_tessellation::{BuffersBuilder, FillOptions, FillTessellator, FillVertex, LineCap, LineJoin, StrokeOptions, StrokeTessellator, StrokeVertex, VertexBuffers};
use crate::egui::epaint::text::OutlineSegment;
use proto::canvas::{Board, Bounds, Cap, Entry, Item, ItemId, Join, Motions, Paint, Picture, Rgba, Shape, Style, TEXT_SIZE, Target as Animated};
use proto::view::{ViewChange, ViewFit, ViewSpec};
use proto::svg_path::Segment;
use proto::{EventKind, FromHelper, NodeId, Value, WindowId};
use pwrs_gui_gpu::wgpu;
use std::borrow::Cow;
use std::collections::hash_map::DefaultHasher;
use std::collections::{BTreeMap, HashMap};
use std::hash::{Hash, Hasher};
use std::sync::Arc;
use std::time::Instant;
use wgpu::util::DeviceExt;

/// The samples to a pixel each canvas and each layer is drawn with.
const SAMPLES: u32 = 4;

/// How far, in pixels, lyon's straight edges may stray from a curve.
const TOLERANCE: f64 = 0.1;

/// The longest edge, in pixels, a gradient's triangles are cut down to, its
/// colors being worked out at every corner and blended across.
const GRADIENT_STEP: f32 = 4.0;

/// How near, in points, the pointer is to a line or a thin stroke to be on
/// it: egui's default interact radius.
const NEAR: f32 = 5.0;

/// The format each canvas and layer texture is in, its bytes egui's own
/// gamma-encoded, premultiplied colors.
const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

/// A plane transform, `x' = a x + c y + e` and `y' = b x + d y + f`.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Affine([f64; 6]);

impl Affine {
    const IDENTITY: Affine = Affine([1.0, 0.0, 0.0, 1.0, 0.0, 0.0]);

    fn translate(x: f64, y: f64) -> Affine {
        Affine([1.0, 0.0, 0.0, 1.0, x, y])
    }

    fn scale(x: f64, y: f64) -> Affine {
        Affine([x, 0.0, 0.0, y, 0.0, 0.0])
    }

    /// A turn of `degrees` clockwise, y growing downwards.
    fn rotate(degrees: f64) -> Affine {
        let (s, c) = degrees.to_radians().sin_cos();
        Affine([c, s, -s, c, 0.0, 0.0])
    }

    /// A skew of `x` degrees along x and `y` along y.
    fn skew(x: f64, y: f64) -> Affine {
        Affine([1.0, y.to_radians().tan(), x.to_radians().tan(), 1.0, 0.0, 0.0])
    }

    /// `self` after `inner`: a point goes through `inner` first.
    fn then_inner(self, inner: Affine) -> Affine {
        let [a, b, c, d, e, f] = self.0;
        let [a2, b2, c2, d2, e2, f2] = inner.0;
        Affine([a * a2 + c * b2, b * a2 + d * b2, a * c2 + c * d2, b * c2 + d * d2, a * e2 + c * f2 + e, b * e2 + d * f2 + f])
    }

    fn apply(self, [x, y]: [f64; 2]) -> [f64; 2] {
        let [a, b, c, d, e, f] = self.0;
        [a * x + c * y + e, b * x + d * y + f]
    }

    fn inverse(self) -> Option<Affine> {
        let [a, b, c, d, e, f] = self.0;
        let det = a * d - b * c;
        if det.abs() < 1e-12 {
            return None;
        }
        let (ia, ib, ic, id) = (d / det, -b / det, -c / det, a / det);
        Some(Affine([ia, ib, ic, id, -(ia * e + ic * f), -(ib * e + id * f)]))
    }

    /// How much the transform grows a length, on average over directions.
    fn growth(self) -> f64 {
        let [a, b, c, d, ..] = self.0;
        (a * d - b * c).abs().sqrt()
    }

    fn lyon(self) -> Transform {
        let [a, b, c, d, e, f] = self.0;
        Transform::new(a as f32, b as f32, c as f32, d as f32, e as f32, f as f32)
    }
}

/// What turns, skews and scales an item about `pivot`: its turn, skew and
/// scale, applied to it in that order from the inside out.
fn placed(style: &Style, pivot: [f64; 2]) -> Affine {
    Affine::translate(pivot[0], pivot[1])
        .then_inner(Affine::rotate(style.rotate))
        .then_inner(Affine::skew(style.skew[0], style.skew[1]))
        .then_inner(Affine::scale(style.scale[0], style.scale[1]))
        .then_inner(Affine::translate(-pivot[0], -pivot[1]))
}

/// The point an item turns about: `origin`'s fractions of its box, or the
/// box's middle.
fn pivot_of(style: &Style, own: Bounds) -> [f64; 2] {
    within(own, style.origin.unwrap_or([0.5, 0.5]))
}

/// The box `points` span; none for no points.
fn spanned(points: impl IntoIterator<Item = [f64; 2]>) -> Option<Bounds> {
    let mut low = [f64::INFINITY; 2];
    let mut high = [f64::NEG_INFINITY; 2];
    let mut any = false;
    for [x, y] in points {
        any = true;
        low = [low[0].min(x), low[1].min(y)];
        high = [high[0].max(x), high[1].max(y)];
    }
    any.then(|| Bounds { x: low[0], y: low[1], width: high[0] - low[0], height: high[1] - low[1] })
}

/// The corners of `b`.
fn corners(b: Bounds) -> [[f64; 2]; 4] {
    [[b.x, b.y], [b.x + b.width, b.y], [b.x + b.width, b.y + b.height], [b.x, b.y + b.height]]
}

/// The point `fraction` of the way across and down `b`.
fn within(b: Bounds, fraction: [f64; 2]) -> [f64; 2] {
    [b.x + b.width * fraction[0], b.y + b.height * fraction[1]]
}

/// How an error names `item`: its kind and name.
fn label(item: &Item) -> String {
    match &item.name {
        Some(n) => format!("{} {n}", item.kind().name()),
        None => item.kind().article(),
    }
}

/// The lyon path `segments` draw, absolute ones in the item's own
/// coordinates, as a shape's outline ([`Shape::outline`]) and a Path's data
/// give them: each arc as the protocol's cubic pieces, a straight line
/// where a radius is zero, nothing where its ends are one point.
fn svg_path(segments: &[Segment]) -> Path {
    let p = |[x, y]: [f64; 2]| point(x as f32, y as f32);
    let mut builder = Path::svg_builder();
    let (mut current, mut start) = ([0.0, 0.0], [0.0, 0.0]);
    for s in segments {
        match s {
            Segment::Move(to) => {
                builder.move_to(p(*to));
                start = *to;
            }
            Segment::Line(to) => {
                builder.line_to(p(*to));
            }
            Segment::Quad(c, to) => {
                builder.quadratic_bezier_to(p(*c), p(*to));
            }
            Segment::Cubic(c1, c2, to) => {
                builder.cubic_bezier_to(p(*c1), p(*c2), p(*to));
            }
            Segment::Arc { radii, rotation, large, sweep, to } if current != *to => match proto::svg_path::center_arc(current, *radii, *rotation, *large, *sweep, *to) {
                Some(arc) => {
                    let mut pieces = proto::svg_path::arc_cubics(&arc);
                    if let Some(last) = pieces.last_mut() {
                        last[2] = *to;
                    }
                    for [c1, c2, end] in pieces {
                        builder.cubic_bezier_to(p(c1), p(c2), p(end));
                    }
                }
                None => {
                    builder.line_to(p(*to));
                }
            },
            Segment::Arc { .. } => {}
            Segment::Close => builder.close(),
        }
        current = match s {
            Segment::Move(to) | Segment::Line(to) | Segment::Quad(_, to) | Segment::Cubic(_, _, to) | Segment::Arc { to, .. } => *to,
            Segment::Close => start,
        };
    }
    builder.build()
}

/// The exact box of what `segments` draw once `transform` places them.
fn box_of(segments: &[Segment], transform: Affine) -> Option<Bounds> {
    proto::svg_path::bounds(segments, transform.0).map(|[x, y, width, height]| Bounds { x, y, width, height })
}

/// The part of `b` within `area`; none where no room is left of it.
fn cut(b: Bounds, area: Area) -> Option<Bounds> {
    let a = match area {
        Area::Everywhere => return Some(b),
        Area::Inside(a) => a,
        Area::Nowhere => return None,
    };
    let (left, top) = (b.x.max(a.x), b.y.max(a.y));
    let (right, bottom) = ((b.x + b.width).min(a.x + a.width), (b.y + b.height).min(a.y + a.height));
    (right >= left && bottom >= top).then_some(Bounds { x: left, y: top, width: right - left, height: bottom - top })
}

/// The box around every one of `boxes`; none for none.
fn union(boxes: impl IntoIterator<Item = Bounds>) -> Option<Bounds> {
    spanned(boxes.into_iter().flat_map(|b| [[b.x, b.y], [b.x + b.width, b.y + b.height]]))
}

/// The box a fit counts for an outline drawn by `full`, `outline` being
/// in the item's own coordinates: its exact box, grown on each side by
/// half of `stroke`, the stroke's width when it is stroked, as `full`
/// stretches that width across and down, as Inkscape's visual box grows,
/// and cut to `area`.
fn visual_box(outline: &[Segment], full: Affine, stroke: Option<f64>, area: Area) -> Option<Bounds> {
    let b = box_of(outline, full)?;
    let [a, b2, c, d, ..] = full.0;
    let (across, down) = match stroke {
        Some(width) if width > 0.0 => (width / 2.0 * a.hypot(c), width / 2.0 * b2.hypot(d)),
        _ => (0.0, 0.0),
    };
    cut(Bounds { x: b.x - across, y: b.y - down, width: b.width + 2.0 * across, height: b.height + 2.0 * down }, area)
}

/// One corner of a canvas's triangles: where, in physical pixels from its
/// top left, the point of its texture it takes, and its color, gamma-encoded
/// and premultiplied, as egui's own.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Vertex {
    at: [f32; 2],
    uv: [f32; 2],
    color: [f32; 4],
}

/// What a canvas's triangles take their texels from.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum Tex {
    /// None: every texel white.
    White,
    /// A texture of egui's, its font atlas or a picture it loaded.
    Egui(egui::TextureId),
}

/// Triangles, three indices to each, and what they take their texels from.
#[derive(Clone, Debug, PartialEq)]
struct Mesh {
    vertices: Vec<Vertex>,
    indices: Vec<u32>,
    texture: Tex,
}

/// One step of drawing a canvas.
#[derive(Clone, Debug, PartialEq)]
enum Op {
    Mesh(Arc<Mesh>),
    /// A group's own steps, drawn into a layer of their own, which is drawn
    /// back through the `mask` triangles, whose corners are white at
    /// `opacity`.
    Layer { ops: Vec<Op>, mask: Vec<Vertex>, mask_indices: Vec<u32>, opacity: f32 },
}

impl Op {
    fn mesh(vertices: Vec<Vertex>, indices: Vec<u32>, texture: Tex) -> Op {
        Op::Mesh(Arc::new(Mesh { vertices, indices, texture }))
    }
}

/// How many layers `ops` draw into, at every depth.
fn layer_count(ops: &[Op]) -> usize {
    ops.iter()
        .map(|op| match op {
            Op::Mesh(_) => 0,
            Op::Layer { ops, .. } => 1 + layer_count(ops),
        })
        .sum()
}

/// A color as a canvas's vertex stores it, `opacity` folded in.
fn vertex_color(c: Rgba, opacity: f64) -> [f32; 4] {
    let [r, g, b, a] = c.0;
    let a = f32::from(a) / 255.0 * opacity as f32;
    [f32::from(r) / 255.0 * a, f32::from(g) / 255.0 * a, f32::from(b) / 255.0 * a, a]
}

/// The color a gradient's colors give at `t`, from 0 to 1, between the
/// stops around it, blended premultiplied: the first color before the first
/// stop and the last after the last.
fn along(colors: &[Rgba], stops: &[f64], t: f64, opacity: f64) -> [f32; 4] {
    let t = t.clamp(0.0, 1.0);
    let premultiplied: Vec<[f32; 4]> = colors.iter().map(|c| vertex_color(*c, opacity)).collect();
    let (Some(first), Some(last)) = (premultiplied.first(), premultiplied.last()) else {
        return [0.0; 4];
    };
    let after = match stops.iter().position(|s| *s >= t) {
        None => return *last,
        Some(0) => return *first,
        Some(i) => i,
    };
    let (from, to) = (stops[after - 1], stops[after]);
    let share = if to > from { ((t - from) / (to - from)) as f32 } else { 1.0 };
    let (a, b) = (premultiplied[after - 1], premultiplied[after]);
    [a[0] + (b[0] - a[0]) * share, a[1] + (b[1] - a[1]) * share, a[2] + (b[2] - a[2]) * share, a[3] + (b[3] - a[3]) * share]
}

/// The color `paint` gives at `fraction` of the way across and down the
/// item's box.
fn paint_at(paint: &Paint, fraction: [f64; 2], opacity: f64) -> [f32; 4] {
    match paint {
        Paint::Color(c) => vertex_color(*c, opacity),
        Paint::Linear { colors, stops, from, to } => {
            let (dx, dy) = (to[0] - from[0], to[1] - from[1]);
            let length = dx * dx + dy * dy;
            let t = if length > 0.0 { ((fraction[0] - from[0]) * dx + (fraction[1] - from[1]) * dy) / length } else { 0.0 };
            along(colors, stops, t, opacity)
        }
        Paint::Radial { colors, stops, center, radius } => {
            let (dx, dy) = (fraction[0] - center[0], fraction[1] - center[1]);
            along(colors, stops, (dx * dx + dy * dy).sqrt() / radius, opacity)
        }
    }
}

/// Triangles in pixels, `positions` and three `indices` to each, cut until
/// no edge is longer than [`GRADIENT_STEP`], so colors worked out at their
/// corners follow a gradient closely.
fn subdivided(mut positions: Vec<[f32; 2]>, indices: Vec<u32>) -> (Vec<[f32; 2]>, Vec<u32>) {
    let length = |a: [f32; 2], b: [f32; 2]| ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2)).sqrt();
    let mut work: Vec<[u32; 3]> = indices.as_chunks::<3>().0.to_vec();
    let mut done = Vec::with_capacity(indices.len());
    let mut middles: HashMap<(u32, u32), u32> = HashMap::new();
    while let Some([a, b, c]) = work.pop() {
        let (pa, pb, pc) = (positions[a as usize], positions[b as usize], positions[c as usize]);
        let edges = [(length(pa, pb), a, b, c), (length(pb, pc), b, c, a), (length(pc, pa), c, a, b)];
        let (longest, x, y, z) = edges.into_iter().fold(edges[0], |kept, e| if e.0 > kept.0 { e } else { kept });
        if longest <= GRADIENT_STEP {
            done.extend([a, b, c]);
            continue;
        }
        let key = (x.min(y), x.max(y));
        let middle = *middles.entry(key).or_insert_with(|| {
            let (px, py) = (positions[x as usize], positions[y as usize]);
            positions.push([(px[0] + py[0]) / 2.0, (px[1] + py[1]) / 2.0]);
            (positions.len() - 1) as u32
        });
        work.push([x, middle, z]);
        work.push([middle, y, z]);
    }
    (positions, done)
}

/// What an item leaves to its group when it does not set it: a Group's
/// fill, stroke, stroke width, dash, caps and joins, as SVG's `<g>`.
#[derive(Clone, Debug, Default)]
struct Inherited {
    fill: Option<Paint>,
    stroke: Option<Rgba>,
    stroke_width: Option<f64>,
    dash: Option<Vec<f64>>,
    cap: Option<Cap>,
    join: Option<Join>,
}

impl Inherited {
    /// What `style` passes on, its own keys over those passed to it.
    fn under(&self, style: &Style) -> Inherited {
        Inherited {
            fill: style.fill.clone().or_else(|| self.fill.clone()),
            stroke: style.stroke.or(self.stroke),
            stroke_width: style.stroke_width.or(self.stroke_width),
            dash: style.dash.clone().or_else(|| self.dash.clone()),
            cap: style.cap.or(self.cap),
            join: style.join.or(self.join),
        }
    }
}

/// Where a named item, or one inside a named Group, was drawn, in pixels
/// from the canvas's top left: the name its events give, what it covers,
/// the clip shapes of the groups around it, each of which must contain a
/// point for the point to be on it, and the box a fit counts for it.
#[derive(Clone, Debug)]
struct Hit {
    name: Arc<str>,
    covers: Covers,
    clips: Arc<[Path]>,
    span: Option<Bounds>,
}

/// The clips of the groups around an item, in pixels from the canvas's top
/// left: their outlines, each of which must contain a point for the point to
/// be on the item, and the room all of them leave, which the box a fit
/// counts for the item is cut to.
#[derive(Clone, Debug, Default)]
struct Clips {
    paths: Arc<[Path]>,
    area: Area,
}

impl Clips {
    /// These clips and one more, `outline` placed by `world`.
    fn and(&self, outline: &[Segment], world: Affine) -> Clips {
        Clips {
            paths: self.paths.iter().cloned().chain(std::iter::once(svg_path(outline).transformed(&world.lyon()))).collect(),
            area: match box_of(outline, world) {
                Some(b) => match cut(b, self.area) {
                    Some(left) => Area::Inside(left),
                    None => Area::Nowhere,
                },
                None => Area::Nowhere,
            },
        }
    }
}

/// Where clips leave room: everywhere, inside a box, or nowhere.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
enum Area {
    #[default]
    Everywhere,
    Inside(Bounds),
    Nowhere,
}

#[derive(Clone, Debug)]
enum Covers {
    /// An outline: inside `inside`, the whole outline when the item is
    /// filled and else its closed subpaths, none when it has none, or within
    /// `reach` pixels of the outline.
    Outline { path: Arc<Path>, inside: Option<Arc<Path>>, reach: f32 },
    /// A quadrilateral, a text's or a picture's box where it was drawn.
    Quad([[f32; 2]; 4]),
}

impl Hit {
    fn holds(&self, at: Point) -> bool {
        let inside = |path: &Path| hit_test_path(&at, path.iter(), lyon_tessellation::FillRule::NonZero, TOLERANCE as f32);
        if !self.clips.iter().all(inside) {
            return false;
        }
        match &self.covers {
            Covers::Outline { path, inside: region, reach } => region.as_deref().is_some_and(inside) || distance(path, at) <= *reach,
            Covers::Quad(q) => in_quad(q, at),
        }
    }
}

/// How far `at` is from the nearest point of `path`'s outline.
fn distance(path: &Path, at: Point) -> f32 {
    let mut nearest = f32::INFINITY;
    for e in path.iter().flattened(TOLERANCE as f32) {
        let (from, to) = match e {
            PathEvent::Line { from, to } => (from, to),
            PathEvent::End { last, first, close: true } => (last, first),
            _ => continue,
        };
        let along = to - from;
        let length = along.square_length();
        let t = if length > 0.0 { ((at - from).dot(along) / length).clamp(0.0, 1.0) } else { 0.0 };
        nearest = nearest.min((from + along * t - at).length());
    }
    nearest
}

/// Whether `at` is inside the convex quadrilateral `q`, its corners in
/// order.
fn in_quad(q: &[[f32; 2]; 4], at: Point) -> bool {
    let side = |a: [f32; 2], b: [f32; 2]| (b[0] - a[0]) * (at.y - a[1]) - (b[1] - a[1]) * (at.x - a[0]);
    let sides = [side(q[0], q[1]), side(q[1], q[2]), side(q[2], q[3]), side(q[3], q[0])];
    sides.iter().all(|s| *s >= 0.0) || sides.iter().all(|s| *s <= 0.0)
}

/// What a frame of a canvas draws, where each item that takes events was
/// drawn, topmost last, the box a fit counts for each item drawn, and what
/// could not be drawn.
#[derive(Clone, Debug, Default)]
struct Frame {
    ops: Vec<Op>,
    hits: Vec<Hit>,
    spans: Vec<Bounds>,
    problems: Vec<String>,
    /// Whether a picture was still loading, so the next frame makes it
    /// again.
    pending: bool,
}

impl Frame {
    /// The name of the topmost item at `at`, in pixels.
    fn item_at(&self, at: [f32; 2]) -> Option<&str> {
        let at = point(at[0], at[1]);
        self.hits.iter().rev().find(|h| h.holds(at)).map(|h| h.name.as_ref())
    }

    /// Whether a step takes one of egui's textures.
    fn takes_egui_textures(&self) -> bool {
        fn any(ops: &[Op]) -> bool {
            ops.iter().any(|op| match op {
                Op::Mesh(mesh) => matches!(mesh.texture, Tex::Egui(_)),
                Op::Layer { ops, .. } => any(ops),
            })
        }
        any(&self.ops)
    }
}

/// What an item drew, which a later frame takes again while the item, its
/// animated values and what places it stay as they were: its steps, where
/// it takes events, the box a fit counts for it, and what could not be
/// drawn.
#[derive(Debug, Default)]
struct Drawn {
    ops: Vec<Op>,
    hits: Vec<Hit>,
    spans: Vec<Bounds>,
    problems: Vec<String>,
}

/// What each item drew in a frame, by its id, with the key of what it drew
/// from.
type DrawnItems = HashMap<ItemId, (u64, Arc<Drawn>)>;

/// What making a frame reads: the canvas's size in pixels, the display's
/// pixels per point, the theme's text color, the egui context its text is
/// laid out and its pictures loaded through (none in a test), the board and
/// its animations at `now`, and what each item drew in the last frame;
/// `kept` gathers what each drew in this one.
struct Making<'a> {
    size: [f32; 2],
    ppp: f32,
    text: Rgba,
    ctx: Option<&'a egui::Context>,
    board: &'a Board,
    motions: &'a mut Motions,
    now: Instant,
    last: &'a DrawnItems,
    kept: DrawnItems,
    frame: Frame,
}

/// What `values` hash to, each a property and its value.
fn values_key<'v>(values: impl Iterator<Item = (&'v str, Animated)>) -> u64 {
    let mut h = DefaultHasher::new();
    for (property, value) in values {
        property.hash(&mut h);
        match value {
            Animated::Number(n) => n.to_bits().hash(&mut h),
            Animated::Pair([x, y]) => (x.to_bits(), y.to_bits()).hash(&mut h),
            Animated::Color(c) => c.0.hash(&mut h),
        }
    }
    h.finish()
}

/// Where an animation of `property` begins on `item`, under what its groups
/// pass on, when no item gives the property: a Text giving neither Fill
/// nor Stroke is filled in the theme's `text` color and any other item
/// outlined in it, as [`Making::paints`] draws them; else a missing color
/// is transparent and a missing width 1.
fn first_drawn(item: &Item, inherited: &Inherited, property: &str, text: Rgba) -> Option<Animated> {
    let passed = inherited.under(&item.style);
    let neither = passed.fill.is_none() && passed.stroke.is_none();
    let clear = Animated::Color(Rgba([0, 0, 0, 0]));
    let is_text = matches!(item.shape, Shape::Text { .. });
    Some(match property {
        "fill" if neither && is_text => Animated::Color(text),
        "fill" => match passed.fill {
            Some(Paint::Color(c)) => Animated::Color(c),
            _ => clear,
        },
        "stroke" if passed.stroke.is_none() && ((neither && !is_text) || matches!(item.shape, Shape::Line(..))) => Animated::Color(text),
        "stroke" => passed.stroke.map_or(clear, Animated::Color),
        "strokewidth" => Animated::Number(passed.stroke_width.unwrap_or(1.0)),
        _ => return item.value_of(property),
    })
}

impl<'a> Making<'a> {
    /// The steps that draw the items `ids` give, placed by `world`, under
    /// what their group passes on and inside `clips`, `context` keying all
    /// of those, `named` the name of the nearest named group around them.
    #[allow(clippy::too_many_arguments)]
    fn items(&mut self, ids: &[ItemId], world: Affine, inherited: &Inherited, clips: &Clips, context: u64, named: Option<&'a str>) -> Vec<Op> {
        let board = self.board;
        let mut order: Vec<(ItemId, &'a Entry)> = ids.iter().filter_map(|id| board.entry(*id).map(|e| (*id, e))).filter(|(_, e)| e.item.style.visible).collect();
        order.sort_by_key(|(_, e)| e.item.style.order);
        let mut out = Vec::new();
        for (id, entry) in order {
            let name = entry.item.name.as_deref().or(named);
            self.item(id, entry, world, inherited, clips, context, name, &mut out);
        }
        out
    }

    /// The item in `entry` with its animations' values at `now` set on
    /// it, and the key of those values; an animation's start a frame reads
    /// is read here, under what the item's groups pass on.
    fn effective(&mut self, id: ItemId, entry: &'a Entry, inherited: &Inherited) -> (Cow<'a, Item>, u64) {
        let (now, text) = (self.now, self.text);
        let Some(motions) = self.motions.of_mut(id).filter(|m| !m.is_empty()) else {
            return (Cow::Borrowed(&entry.item), 0);
        };
        let mut item = entry.item.clone();
        let mut values = Vec::with_capacity(motions.len());
        for (property, motion) in motions.iter_mut() {
            if motion.from.is_none() {
                motion.from = first_drawn(&entry.item, inherited, property, text);
            }
            if let Some(value) = motion.value(now) {
                item.set_value(property, value);
                values.push((property.as_str(), value));
            }
        }
        let key = values_key(values.into_iter());
        (Cow::Owned(item), key)
    }

    /// The item in `entry` with the values its animations have at `now`
    /// set on it, for measuring: animations whose start is not yet read are
    /// passed over.
    fn measured(&self, id: ItemId, entry: &'a Entry) -> Cow<'a, Item> {
        let Some(motions) = self.motions.of(id).filter(|m| !m.is_empty()) else {
            return Cow::Borrowed(&entry.item);
        };
        let mut item = entry.item.clone();
        for (property, motion) in motions {
            if let Some(value) = motion.value(self.now) {
                item.set_value(property, value);
            }
        }
        Cow::Owned(item)
    }

    #[allow(clippy::too_many_arguments)]
    fn item(&mut self, id: ItemId, entry: &'a Entry, world: Affine, inherited: &Inherited, clips: &Clips, context: u64, name: Option<&'a str>, out: &mut Vec<Op>) {
        let (item, drawn) = self.effective(id, entry, inherited);
        if let Shape::Group { at, clip } = &item.shape {
            let passed = inherited.under(&item.style);
            self.group(id, entry, &item, *at, clip.as_deref(), world, &passed, clips, (context, drawn), name, out);
            return;
        }
        let mut hasher = DefaultHasher::new();
        (entry.revision, drawn, context, name).hash(&mut hasher);
        let key = hasher.finish();
        if let Some((was, done)) = self.last.get(&id)
            && *was == key
        {
            out.extend(done.ops.iter().cloned());
            self.frame.hits.extend(done.hits.iter().cloned());
            self.frame.spans.extend(done.spans.iter().copied());
            self.frame.problems.extend(done.problems.iter().cloned());
            self.kept.insert(id, (key, Arc::clone(done)));
            return;
        }
        let (hits, spans, problems, pending) = (self.frame.hits.len(), self.frame.spans.len(), self.frame.problems.len(), self.frame.pending);
        self.frame.pending = false;
        let mut ops = Vec::new();
        let passed = inherited.under(&item.style);
        match &item.shape {
            Shape::Text { at, text, size } => self.text(&item, *at, text, *size, world, &passed, clips, name, &mut ops),
            Shape::Image(b, picture) => self.image(&item, *b, picture, world, clips, name, &mut ops),
            shape => {
                if let Some(segments) = shape.outline() {
                    let own = box_of(&segments, Affine::IDENTITY);
                    let local = own.map_or(Affine::IDENTITY, |b| placed(&item.style, pivot_of(&item.style, b)));
                    self.shape(&item, &segments, own, world.then_inner(local), &passed, clips, name, &mut ops);
                }
            }
        }
        if !self.frame.pending {
            let done = Drawn { ops: ops.clone(), hits: self.frame.hits[hits..].to_vec(), spans: self.frame.spans[spans..].to_vec(), problems: self.frame.problems[problems..].to_vec() };
            self.kept.insert(id, (key, Arc::new(done)));
        }
        self.frame.pending |= pending;
        out.extend(ops);
    }

    /// A Group's items, placed by its offset, turn, skew and scale about its
    /// pivot inside `world`, drawn into a layer of their own when it is
    /// translucent or clips; `keys` are the context its own items are keyed
    /// under, and the key of its animated values.
    #[allow(clippy::too_many_arguments)]
    fn group(&mut self, id: ItemId, entry: &'a Entry, item: &Item, at: [f64; 2], clip: Option<&Shape>, world: Affine, passed: &Inherited, clips: &Clips, keys: (u64, u64), name: Option<&'a str>, out: &mut Vec<Op>) {
        let inner_world = world.then_inner(self.group_placing(entry, item, at));
        let clip_outline = clip.and_then(Shape::outline);
        let inner_clips = match &clip_outline {
            Some(outline) => clips.and(outline, inner_world),
            None => clips.clone(),
        };
        let clip_path = clip_outline.as_ref().and(inner_clips.paths.last());
        let mut hasher = DefaultHasher::new();
        (keys.0, id, entry.revision, keys.1, inner_world.0.map(f64::to_bits)).hash(&mut hasher);
        let ops = self.items(&entry.children, inner_world, passed, &inner_clips, hasher.finish(), name);
        if item.style.opacity >= 1.0 && clip_path.is_none() {
            out.extend(ops);
            return;
        }
        let opacity = item.style.opacity as f32;
        let corner = |at: [f32; 2]| Vertex { at, uv: [0.0; 2], color: [opacity; 4] };
        let (mask, mask_indices) = match &clip_path {
            Some(path) => match filled(path) {
                Ok((positions, indices)) => (positions.into_iter().map(corner).collect(), indices),
                Err(e) => {
                    self.frame.problems.push(format!("{}: its Clip could not be filled: {e}", label(item)));
                    return;
                }
            },
            None => {
                let [w, h] = self.size;
                ([[0.0, 0.0], [w, 0.0], [w, h], [0.0, h]].into_iter().map(corner).collect(), vec![0, 1, 2, 0, 2, 3])
            }
        };
        out.push(Op::Layer { ops, mask, mask_indices, opacity });
    }

    /// What places a Group's items in its group's coordinates: its offset
    /// `at`, then its turn, skew and scale about its pivot, the point its
    /// Origin gives in fractions of the box around its items, shown or not,
    /// or else its own 0,0.
    fn group_placing(&self, entry: &Entry, item: &Item, at: [f64; 2]) -> Affine {
        let pivot = match item.style.origin {
            Some(o) => union(entry.children.iter().filter_map(|c| self.placed_box(*c, Affine::IDENTITY))).map_or([0.0, 0.0], |b| within(b, o)),
            None => [0.0, 0.0],
        };
        Affine::translate(at[0], at[1]).then_inner(placed(&item.style, pivot))
    }

    /// The exact box of what the item `id` draws, its animations' values
    /// set on it, once `under` places its group's coordinates: its outline's,
    /// a Text's box as egui lays it out, an Image's box, or a Group's items'
    /// together, each as placed; none for an item that draws nothing.
    fn placed_box(&self, id: ItemId, under: Affine) -> Option<Bounds> {
        let entry = self.board.entry(id)?;
        let item = self.measured(id, entry);
        let quad = |own: Bounds| spanned(corners(own).map(|c| under.then_inner(placed(&item.style, pivot_of(&item.style, own))).apply(c)));
        match &item.shape {
            Shape::Group { at, .. } => {
                let inner = under.then_inner(self.group_placing(entry, &item, *at));
                union(entry.children.iter().filter_map(|c| self.placed_box(*c, inner)))
            }
            Shape::Text { at, text, size } => quad(self.text_box(*at, text, *size)?),
            Shape::Image(b, _) => quad(*b),
            shape => {
                let segments = shape.outline()?;
                let own = box_of(&segments, Affine::IDENTITY)?;
                box_of(&segments, under.then_inner(placed(&item.style, pivot_of(&item.style, own))))
            }
        }
    }

    /// The box a Text at `at` takes, as egui lays it out; none without egui.
    fn text_box(&self, at: [f64; 2], text: &str, size: Option<f64>) -> Option<Bounds> {
        let ctx = self.ctx?;
        let galley = ctx.fonts_mut(|f| f.layout_no_wrap(text.to_string(), egui::FontId::proportional(size.unwrap_or(TEXT_SIZE) as f32), egui::Color32::WHITE));
        let measured = galley.size();
        Some(Bounds { x: at[0], y: at[1], width: f64::from(measured.x), height: f64::from(measured.y) })
    }

    /// What fills and strokes `item` under `style`: a Line is never filled;
    /// a Text giving neither is filled in the theme's text color, and any
    /// other item giving neither is outlined in it.
    fn paints(&self, item: &Item, style: &Inherited) -> (Option<Paint>, Option<Rgba>) {
        match (&item.shape, style.fill.clone(), style.stroke) {
            (Shape::Line(..), _, given) => (None, given.or(Some(self.text))),
            (Shape::Text { .. }, None, None) => (Some(Paint::Color(self.text)), None),
            (_, None, None) => (None, Some(self.text)),
            (_, fill, stroke) => (fill, stroke),
        }
    }

    /// A shape's fill and stroke, its outline `segments` in its own
    /// coordinates, `own` their box, over which a gradient spreads, and
    /// `full` placing them in pixels. The box a fit counts for it joins the
    /// frame's, and a named one takes events where it is drawn: inside its
    /// closed subpaths, filled or not, inside the rest where its fill
    /// paints, and within half its stroke or 5 points of its outline.
    #[allow(clippy::too_many_arguments)]
    fn shape(&mut self, item: &Item, segments: &[Segment], own: Option<Bounds>, full: Affine, style: &Inherited, clips: &Clips, name: Option<&str>, out: &mut Vec<Op>) {
        let (fill, stroke) = self.paints(item, style);
        let path = svg_path(segments);
        let in_pixels = path.clone().transformed(&full.lyon());
        self.paint(item, &path, &in_pixels, own, full, fill.as_ref(), stroke, style, out);
        let width = style.stroke_width.unwrap_or(1.0);
        let span = visual_box(segments, full, stroke.map(|_| width), clips.area);
        self.frame.spans.extend(span);
        if let Some(name) = name {
            let reach = ((width * full.growth()) as f32 / 2.0).max(NEAR * self.ppp);
            let outline = Arc::new(in_pixels);
            let inside = match fill {
                Some(_) => Some(Arc::clone(&outline)),
                None => {
                    let closed = proto::svg_path::closed(segments);
                    (!closed.is_empty()).then(|| Arc::new(svg_path(&closed).transformed(&full.lyon())))
                }
            };
            self.frame.hits.push(Hit { name: Arc::from(name), covers: Covers::Outline { path: outline, inside, reach }, clips: Arc::clone(&clips.paths), span });
        }
    }

    /// The triangles that fill `path`, in the item's own coordinates, with
    /// `fill` and stroke it with `stroke` under `style`, `in_pixels` the
    /// path `full` places, a gradient spread over `own`, the item's box.
    #[allow(clippy::too_many_arguments)]
    fn paint(&mut self, item: &Item, path: &Path, in_pixels: &Path, own: Option<Bounds>, full: Affine, fill: Option<&Paint>, stroke: Option<Rgba>, style: &Inherited, out: &mut Vec<Op>) {
        let opacity = item.style.opacity;
        let width = style.stroke_width.unwrap_or(1.0);
        if let Some(paint) = fill {
            match filled(in_pixels) {
                Ok((positions, indices)) => out.push(colored(positions, indices, paint, own, full.inverse(), opacity)),
                Err(e) => self.frame.problems.push(format!("{}: its Fill could not be drawn: {e}", label(item))),
            }
        }
        if let Some(color) = stroke
            && width > 0.0
        {
            let growth = full.growth().max(1e-9);
            let dashed = match &style.dash {
                Some(d) if !d.is_empty() => dashes(path, d, TOLERANCE / growth),
                _ => path.clone(),
            };
            let options = StrokeOptions::default()
                .with_line_width(width as f32)
                .with_line_cap(match style.cap.unwrap_or(Cap::Butt) {
                    Cap::Butt => LineCap::Butt,
                    Cap::Round => LineCap::Round,
                    Cap::Square => LineCap::Square,
                })
                .with_line_join(match style.join.unwrap_or(Join::Miter) {
                    Join::Miter => LineJoin::Miter,
                    Join::Round => LineJoin::Round,
                    Join::Bevel => LineJoin::Bevel,
                })
                .with_tolerance((TOLERANCE / growth) as f32);
            let mut buffers: VertexBuffers<Point, u32> = VertexBuffers::new();
            match StrokeTessellator::new().tessellate_path(&dashed, &options, &mut BuffersBuilder::new(&mut buffers, |v: StrokeVertex| v.position())) {
                Ok(_) => {
                    let c = vertex_color(color, opacity);
                    let vertices = buffers.vertices.iter().map(|p| Vertex { at: to_f32(full.apply([f64::from(p.x), f64::from(p.y)])), uv: [0.0; 2], color: c }).collect();
                    out.push(Op::mesh(vertices, buffers.indices, Tex::White));
                }
                Err(e) => self.frame.problems.push(format!("{}: its Stroke could not be drawn: {e:?}", label(item))),
            }
        }
    }

    /// A Text, filled and stroked from the outlines of the glyphs egui lays
    /// it out with, its top left at `at`; it takes events across the box it
    /// is laid out in. A glyph whose font has no outline for it is not
    /// drawn and is reported.
    #[allow(clippy::too_many_arguments)]
    fn text(&mut self, item: &Item, at: [f64; 2], text: &str, size: Option<f64>, world: Affine, style: &Inherited, clips: &Clips, name: Option<&str>, out: &mut Vec<Op>) {
        let Some((path, own, missed)) = self.text_outline(at, text, size) else {
            return;
        };
        if missed > 0 {
            self.frame.problems.push(format!("{}: {missed} of its glyphs have no outline in their font, so they are not drawn", label(item)));
        }
        let full = world.then_inner(placed(&item.style, pivot_of(&item.style, own)));
        let (fill, stroke) = self.paints(item, style);
        let in_pixels = path.clone().transformed(&full.lyon());
        self.paint(item, &path, &in_pixels, Some(own), full, fill.as_ref(), stroke, style, out);
        self.boxed(own, full, clips, name);
    }

    /// Where a Text's or an Image's box `own`, placed in pixels by `full`,
    /// was drawn: the box a fit counts for it joins the frame's, and a named
    /// one takes events across it.
    fn boxed(&mut self, own: Bounds, full: Affine, clips: &Clips, name: Option<&str>) {
        let span = spanned(corners(own).map(|c| full.apply(c))).and_then(|b| cut(b, clips.area));
        self.frame.spans.extend(span);
        if let Some(name) = name {
            self.frame.hits.push(Hit { name: Arc::from(name), covers: Covers::Quad(corners(own).map(|c| to_f32(full.apply(c)))), clips: Arc::clone(&clips.paths), span });
        }
    }

    /// The outlines of a Text's glyphs as egui lays it out at `size`
    /// points, its top left at `at`, as one path in the item's coordinates,
    /// the box it is laid out in, and how many glyphs have no outline; none
    /// without egui.
    fn text_outline(&self, at: [f64; 2], text: &str, size: Option<f64>) -> Option<(Path, Bounds, usize)> {
        let ctx = self.ctx?;
        let font = egui::FontId::proportional(size.unwrap_or(TEXT_SIZE) as f32);
        let (measured, (segments, missed)) = ctx.fonts_mut(|f| {
            let galley = f.layout_no_wrap(text.to_string(), font, egui::Color32::WHITE);
            (galley.size(), f.galley_outlines(&galley, &egui::epaint::text::VariationCoords::default()))
        });
        let p = |at_point: egui::Pos2| point((at[0] + f64::from(at_point.x)) as f32, (at[1] + f64::from(at_point.y)) as f32);
        let mut builder = Path::svg_builder();
        for segment in segments {
            match segment {
                OutlineSegment::MoveTo(to) => {
                    builder.move_to(p(to));
                }
                OutlineSegment::LineTo(to) => {
                    builder.line_to(p(to));
                }
                OutlineSegment::QuadTo(control, to) => {
                    builder.quadratic_bezier_to(p(control), p(to));
                }
                OutlineSegment::CubicTo(first, second, to) => {
                    builder.cubic_bezier_to(p(first), p(second), p(to));
                }
                OutlineSegment::Close => builder.close(),
            }
        }
        Some((builder.build(), Bounds { x: at[0], y: at[1], width: f64::from(measured.x), height: f64::from(measured.y) }, missed))
    }

    #[allow(clippy::too_many_arguments)]
    fn image(&mut self, item: &Item, b: Bounds, picture: &Picture, world: Affine, clips: &Clips, name: Option<&str>, out: &mut Vec<Op>) {
        let full = world.then_inner(placed(&item.style, pivot_of(&item.style, b)));
        let quad = corners(b).map(|c| to_f32(full.apply(c)));
        self.boxed(b, full, clips, name);
        let Some(ctx) = self.ctx else {
            return;
        };
        let wanted = egui::vec2((b.width * full.growth()) as f32, (b.height * full.growth()) as f32);
        let image = egui::Image::new(egui::ImageSource::Bytes { uri: picture_uri(picture).into(), bytes: egui::load::Bytes::Shared(Arc::clone(&picture.bytes)) });
        match image.load_for_size(ctx, wanted) {
            Ok(egui::load::TexturePoll::Ready { texture }) => {
                let c = vertex_color(Rgba([255, 255, 255, 255]), item.style.opacity);
                let uv = [[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]];
                let vertices = quad.into_iter().zip(uv).map(|(at, uv)| Vertex { at, uv, color: c }).collect();
                out.push(Op::mesh(vertices, vec![0, 1, 2, 0, 2, 3], Tex::Egui(texture.id)));
            }
            Ok(egui::load::TexturePoll::Pending { .. }) => self.frame.pending = true,
            Err(e) => self.frame.problems.push(format!("{}: its picture could not be drawn: {e}", label(item))),
        }
    }
}

fn to_f32([x, y]: [f64; 2]) -> [f32; 2] {
    [x as f32, y as f32]
}

/// The address egui's loaders store a canvas picture's bytes under, the
/// same for as long as the scene has them.
fn picture_uri(picture: &Picture) -> String {
    format!("bytes://pwrs-gui-canvas-{:x}-{}.{}", Arc::as_ptr(&picture.bytes).cast::<u8>() as usize, picture.bytes.len(), picture.format)
}

/// `path` filled under the non-zero rule, as SVG fills: its triangles'
/// corners and three indices to each.
fn filled(path: &Path) -> Result<(Vec<[f32; 2]>, Vec<u32>), String> {
    let mut buffers: VertexBuffers<Point, u32> = VertexBuffers::new();
    let options = FillOptions::non_zero().with_tolerance(TOLERANCE as f32);
    FillTessellator::new().tessellate_path(path, &options, &mut BuffersBuilder::new(&mut buffers, |v: FillVertex| v.position())).map_err(|e| format!("{e:?}"))?;
    Ok((buffers.vertices.iter().map(|p| [p.x, p.y]).collect(), buffers.indices))
}

/// A fill's triangles in pixels with its paint's colors at their corners,
/// worked out in fractions of `item_box` through `back`, which takes a
/// pixel to the item's own coordinates; a gradient's triangles are cut
/// smaller first.
fn colored(positions: Vec<[f32; 2]>, indices: Vec<u32>, paint: &Paint, item_box: Option<Bounds>, back: Option<Affine>, opacity: f64) -> Op {
    let (positions, indices) = match paint {
        Paint::Color(_) => (positions, indices),
        _ => subdivided(positions, indices),
    };
    let fraction = |at: [f32; 2]| match (item_box, back) {
        (Some(b), Some(back)) => {
            let [x, y] = back.apply([f64::from(at[0]), f64::from(at[1])]);
            [(x - b.x) / b.width.max(1e-9), (y - b.y) / b.height.max(1e-9)]
        }
        _ => [0.0, 0.0],
    };
    let vertices = positions.into_iter().map(|at| Vertex { at, uv: [0.0; 2], color: paint_at(paint, fraction(at), opacity) }).collect();
    Op::mesh(vertices, indices, Tex::White)
}

/// The pieces of `path` a dash pattern draws, lengths of dash then gap
/// repeated along it, an odd count of lengths taken twice, as SVG takes it.
fn dashes(path: &Path, pattern: &[f64], tolerance: f64) -> Path {
    let pattern: Vec<f64> = if pattern.len() % 2 == 1 { pattern.iter().chain(pattern).copied().collect() } else { pattern.to_vec() };
    let measurements = PathMeasurements::from_path(path, tolerance as f32);
    let mut sampler = measurements.create_sampler(path, SampleType::Distance);
    let total = f64::from(sampler.length());
    let mut builder = Path::builder();
    if total <= 0.0 || pattern.iter().sum::<f64>() <= 0.0 {
        return builder.build();
    }
    let mut at = 0.0;
    let mut index = 0;
    while at < total {
        let length = pattern[index % pattern.len()];
        if index % 2 == 0 && length > 0.0 {
            sampler.split_range(at as f32..(at + length).min(total) as f32, &mut builder);
        }
        at += length;
        index += 1;
    }
    builder.build()
}

/// The steps that draw `canvas`'s board, its animations at `now`, on a
/// canvas `size` pixels across and down at `ppp` pixels per point, the
/// theme's text color `text`, through `ctx`, under its view as drawn at
/// `now`: an item drawn from what it was drawn from in the last frame takes
/// what it drew then.
fn frame_of(canvas: &mut Canvas, size: [f32; 2], ppp: f32, text: Rgba, ctx: Option<&egui::Context>, now: Instant) -> Frame {
    let points = [f64::from(size[0] / ppp), f64::from(size[1] / ppp)];
    let shown = canvas.steering.shown(now, points);
    let placing = placing_of(shown.as_ref(), points);
    let under = (size, ppp, text, placing.0.map(f64::to_bits));
    if canvas.drawn_under != Some(under) {
        canvas.drawn.clear();
        canvas.drawn_under = Some(under);
    }
    canvas.drawn_view = shown;
    let last = std::mem::take(&mut canvas.drawn);
    let mut making = Making { size, ppp, text, ctx, board: &canvas.board, motions: &mut canvas.motions, now, last: &last, kept: HashMap::new(), frame: Frame::default() };
    let world = Affine::scale(f64::from(ppp), f64::from(ppp)).then_inner(placing);
    making.frame.ops = making.items(canvas.board.top(), world, &Inherited::default(), &Clips::default(), 0, None);
    let Making { frame, kept, .. } = making;
    canvas.drawn = kept;
    frame
}

/// The transform `view` places the items with on a canvas `size` points
/// across and down, the identity without one.
fn placing_of(view: Option<&ViewSpec>, size: [f64; 2]) -> Affine {
    match view {
        Some(v) => Affine(v.transform(size)),
        None => Affine::IDENTITY,
    }
}

/// What a Canvas stores between frames.
#[derive(Default)]
pub(crate) struct Canvas {
    /// Its items, read from its node's `items` and changed in place.
    board: Board,
    /// Its items' animations.
    motions: Motions,
    /// What each item drew in the last frame made, and the canvas's size in
    /// pixels, pixels per point and text color it was drawn at.
    drawn: DrawnItems,
    drawn_under: Option<([f32; 2], f32, Rgba, [u64; 6])>,
    /// Moves on with each change to the board or its animations, so a
    /// frame made before is made again.
    version: u64,
    /// The last frame made, with what it was made from.
    made: Option<(Made, Arc<Frame>)>,
    /// Whether an animation was moving when the last frame was made, so the
    /// frame after the last one moves draws where it ends.
    animating: bool,
    /// Its view and what is moving it, and a fit requested before it was
    /// first drawn, made at its first frame.
    steering: ViewState,
    fit_pending: Option<ViewFit>,
    /// The view the last frame made was drawn under.
    drawn_view: Option<ViewSpec>,
    /// Whether a press, not the keyboard, gave it the focus it has, so
    /// no ring shows.
    focus_from_press: bool,
    /// The textures its frames draw into, and the frame they contain.
    targets: Arc<ArcSwapOption<Targets>>,
    recorded: Arc<ArcSwapOption<Frame>>,
    /// What went wrong drawing a frame in the paint callback, which the
    /// window's next frame reports.
    problems: Arc<SegQueue<String>>,
    /// The last problem reported, so one that recurs each frame is reported
    /// once.
    reported: Option<String>,
    /// The named item the pointer was over in the last frame.
    hovered: Option<String>,
    /// The item being dragged, and where the pointer was at its last move.
    dragging: Option<(String, Option<egui::Pos2>)>,
    pointer: pointer::Tracked,
    /// Where the last frame drew the canvas, which a test event's fractions
    /// are measured on.
    pub place: Option<pointer::Place>,
}

/// What a frame is made from: the board's version, the canvas's size in
/// pixels, the display's pixels per point, the theme's text color and the
/// transform its view places the items with.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Made {
    version: u64,
    size: [u32; 2],
    ppp: f32,
    text: [u8; 4],
    placing: [u64; 6],
}

/// Reads `node`'s items onto its canvas's board in place of the ones it
/// had, their animations starting now.
pub(crate) fn seed(node: &Node, inputs: &mut Inputs) -> Result<(), String> {
    let board = Board::of(node)?;
    let canvas = inputs.canvases.entry(node.id).or_default();
    let mut motions = Motions::default();
    motions.started(&board, &board.all(), Instant::now());
    canvas.board = board;
    canvas.motions = motions;
    canvas.drawn.clear();
    canvas.version += 1;
    Ok(())
}

/// Changes the item named `item` of the canvas `node` at `now`, as a
/// SetItem does: the board takes the change, each property it sets
/// directly ends its animation, and its Animate starts the animations it
/// gives.
pub(crate) fn set_item(canvases: &mut HashMap<NodeId, Canvas>, node: NodeId, item: &str, props: &BTreeMap<String, Value>, now: Instant) -> Result<(), String> {
    let canvas = canvases.entry(node).or_default();
    let id = canvas.board.set(item, props)?;
    canvas.motions.changed(&canvas.board, id, props, now);
    canvas.version += 1;
    Ok(())
}

/// Adds `items` to the canvas `node` at `now`, above its own or with
/// `group` above that Group's, as an AddItems does, their animations
/// starting then.
pub(crate) fn add_items(canvases: &mut HashMap<NodeId, Canvas>, node: NodeId, group: Option<&str>, items: &[Value], now: Instant) -> Result<(), String> {
    let canvas = canvases.entry(node).or_default();
    let ids = canvas.board.add(group, items)?;
    canvas.motions.started(&canvas.board, &ids, now);
    canvas.version += 1;
    Ok(())
}

/// Takes the items `names` name out of the canvas `node`, as a RemoveItems
/// does, with their animations.
pub(crate) fn remove_items(canvases: &mut HashMap<NodeId, Canvas>, node: NodeId, names: &[String]) -> Result<(), String> {
    let canvas = canvases.entry(node).or_default();
    let gone = canvas.board.remove(names)?;
    canvas.motions.forget(&gone);
    for id in &gone {
        canvas.drawn.remove(id);
    }
    canvas.version += 1;
    Ok(())
}

/// The items of the canvas `node` as its board has them; none for a
/// canvas whose items were never read.
pub(crate) fn board(canvases: &HashMap<NodeId, Canvas>, node: NodeId) -> Option<&Board> {
    canvases.get(&node).map(|c| &c.board)
}

/// What a view call requests of a canvas.
pub(crate) enum ViewCall {
    Set(ViewChange),
    Get,
    Remove,
}

impl Canvas {
    /// Its size in points when it was last drawn.
    fn size_points(&self) -> Option<[f64; 2]> {
        self.place.map(|p| [f64::from(p.across / p.ppp), f64::from(p.down / p.ppp)])
    }

    /// The box, `[x, y, width, height]` in the items' coordinates, of what
    /// `fit` names as its last frame drew it, through the view that frame
    /// was drawn under; none before it is drawn, or when it drew nothing
    /// `fit` names.
    fn drawn_box(&self, fit: &ViewFit) -> Option<[f64; 4]> {
        let (size, (_, frame), place) = (self.size_points()?, self.made.as_ref()?, self.place?);
        let px = drawn_bounds(frame, fit)?;
        let ppp = f64::from(place.ppp);
        let under = match &self.drawn_view {
            Some(v) => v.clone(),
            None => ViewSpec::default(),
        };
        let low = under.to_items([px.x / ppp, px.y / ppp], size);
        let high = under.to_items([(px.x + px.width) / ppp, (px.y + px.height) / ppp], size);
        Some([low[0], low[1], high[0] - low[0], high[1] - low[1]])
    }

    /// `view` fitted to what `fit` names as the last frame drew it, or as
    /// it is when that frame drew nothing `fit` names.
    fn fitted(&self, mut view: ViewSpec, fit: &ViewFit) -> ViewSpec {
        if let (Some(size), Some(b)) = (self.size_points(), self.drawn_box(fit)) {
            view.fit(b, size);
        }
        view
    }
}

/// The box, in pixels, of what `fit` names as `frame` drew it: the boxes a
/// fit counts for every item drawn, or for the items the names give.
fn drawn_bounds(frame: &Frame, fit: &ViewFit) -> Option<Bounds> {
    match fit {
        ViewFit::All => union(frame.spans.iter().copied()),
        ViewFit::Items(names) => union(frame.hits.iter().filter(|h| names.iter().any(|n| h.name.eq_ignore_ascii_case(n))).filter_map(|h| h.span)),
    }
}

/// Sets, reads or takes away the view of the canvas `node` at `now`, as
/// `call` requests. A set starts from the view drawn then, ending a move under
/// way where it is, or from the default view when the canvas has none;
/// once the canvas has been drawn its fit is made at once and the view
/// moves to the change over its ease, and before that the change is taken
/// at once and its fit made at the first frame. A change that cannot make a
/// view, or that fits to an item the canvas does not have, is refused.
pub(crate) fn view_call(canvases: &mut HashMap<NodeId, Canvas>, node: NodeId, call: ViewCall, now: Instant) -> Result<proto::gpu::GpuValue, String> {
    use proto::gpu::GpuValue;
    let canvas = canvases.entry(node).or_default();
    let size = canvas.size_points();
    match call {
        ViewCall::Set(change) => {
            if let Some(s) = size {
                canvas.steering.settle(now, s);
            }
            let base = match &canvas.steering.view {
                Some(v) => v.clone(),
                None => ViewSpec::default(),
            };
            if let Some(text) = change.problem(&base) {
                return Err(text);
            }
            if let Some(ViewFit::Items(names)) = &change.fit
                && let Some(missing) = names.iter().find(|n| canvas.board.find(n).is_none())
            {
                return Err(format!("the view fits to {missing}, and no item is named {missing}"));
            }
            let made = change.applied_to(&base);
            match size.filter(|_| canvas.made.is_some()) {
                Some(s) => {
                    let to = match &change.fit {
                        Some(fit) => canvas.fitted(made, fit),
                        None => made,
                    };
                    match change.ease {
                        Some(ease) => canvas.steering.ease_to(to, ease, now, s),
                        None => canvas.steering.view = Some(to),
                    }
                }
                None => {
                    canvas.steering.view = Some(made);
                    canvas.fit_pending = change.fit.clone();
                }
            }
            canvas.version += 1;
            Ok(GpuValue::CanvasView(canvas.steering.answer(now, size)))
        }
        ViewCall::Get => Ok(GpuValue::CanvasView(canvas.steering.answer(now, size))),
        ViewCall::Remove => {
            canvas.steering.remove();
            canvas.fit_pending = None;
            canvas.version += 1;
            Ok(GpuValue::Done)
        }
    }
}

/// Whether the view of the canvas `node` takes the press `pressed` from
/// the canvas's key handler.
pub(crate) fn keeps(canvases: &HashMap<NodeId, Canvas>, node: NodeId, pressed: &proto::Shortcut) -> bool {
    canvases.get(&node).is_some_and(|c| c.steering.keeps(pressed))
}

/// Whether the canvas `node` takes keys, and so the focus: for its view's
/// controls, or for its node's `keys`.
pub(crate) fn focusable(canvases: &HashMap<NodeId, Canvas>, node: &Node) -> Result<bool, String> {
    let view_keys = canvases.get(&node.id).is_some_and(|c| c.steering.takes_keys());
    Ok(view_keys || flag(node, "keys", false)?)
}

/// Sets each canvas node's `items` in `tree` to the items its board has,
/// as a snapshot shows them.
pub(crate) fn current_items(tree: &mut Node, canvases: &HashMap<NodeId, Canvas>) {
    if let Some(canvas) = canvases.get(&tree.id) {
        tree.props.insert(String::from("items"), canvas.board.to_value());
    }
    for child in &mut tree.children {
        current_items(child, canvases);
    }
}

/// Which of its item events a canvas's node requests.
#[derive(Clone, Copy, Debug, Default)]
struct ItemEvents {
    click: bool,
    double: bool,
    enter: bool,
    leave: bool,
    drag_start: bool,
    drag_move: bool,
    drag_end: bool,
}

impl ItemEvents {
    fn of(node: &Node) -> Result<ItemEvents, String> {
        Ok(ItemEvents {
            click: flag(node, "click", false)?,
            double: flag(node, "doubleClick", false)?,
            enter: flag(node, "pointerEnter", false)?,
            leave: flag(node, "pointerLeave", false)?,
            drag_start: flag(node, "dragStart", false)?,
            drag_move: flag(node, "dragMove", false)?,
            drag_end: flag(node, "dragEnd", false)?,
        })
    }

    fn drags(self) -> bool {
        self.drag_start || self.drag_move || self.drag_end
    }
}

/// The map an item event at `at`, in points, gives: a GpuSurface
/// pointer event's, with the item's name and where it is in the items'
/// coordinates under `view`.
fn item_value(place: pointer::Place, view: Option<&ViewSpec>, at: egui::Pos2, button: Option<&str>, keys: egui::Modifiers, item: Option<&str>) -> Value {
    let mut value: BTreeMap<String, Value> = place.value_at(at, button, keys);
    value.insert(String::from("item"), item.map_or(Value::Null, |i| Value::Str(i.to_string())));
    with_scene(&mut value, place, view);
    Value::Map(value)
}

/// Adds to an event's map, whose `x` and `y` are physical pixels from the
/// canvas's top left, `scenex` and `sceney`: that point in the items'
/// coordinates under `view`, the canvas's own without one.
fn with_scene(value: &mut BTreeMap<String, Value>, place: pointer::Place, view: Option<&ViewSpec>) {
    let pixel = |key: &str| value.get(key).and_then(Value::as_number);
    let (Some(x), Some(y)) = (pixel("x"), pixel("y")) else {
        return;
    };
    let ppp = f64::from(place.ppp);
    let at = [x / ppp, y / ppp];
    let [sx, sy] = match view {
        Some(v) => v.to_items(at, [f64::from(place.across) / ppp, f64::from(place.down) / ppp]),
        None => at,
    };
    value.insert(String::from("scenex"), Value::Float(sx));
    value.insert(String::from("sceney"), Value::Float(sy));
}

/// Requests a frame from egui just after a press on `response`, still down
/// and not yet a drag, has lasted as long as a click may, so a press that
/// stays still becomes a drag then, as one that moves does past a click's
/// distance; egui wakes itself for a touch's press alone.
fn wake_when_held(ui: &egui::Ui, response: &egui::Response) {
    if !response.is_pointer_button_down_on() || response.dragged() {
        return;
    }
    let (began, time) = ui.input(|i| (i.pointer.press_start_time(), i.time));
    let longest = ui.ctx().options(|o| o.input_options.max_click_duration);
    if let Some(left) = began.map(|b| longest - (time - b)).filter(|left| *left >= 0.0) {
        ui.ctx().request_repaint_after(std::time::Duration::from_secs_f64(left + 0.001));
    }
}

/// Draws a Canvas: a box of its `width` and `height` in points, the room
/// across and the height the window leaves where absent, containing its
/// items as the paint callback draws them under its view, which the
/// gestures and keys its view's controls take move, and raises the events
/// its node requests, each naming the topmost item under the pointer. A
/// canvas that takes keys, for its view's controls or its node's `keys`,
/// takes the focus as [`crate::routed::focus_keys`] says or as
/// Set-GuiFocus gives it, a ring showing a focus the keyboard or a script
/// gave it. What could not be drawn is reported once.
pub(crate) fn draw_canvas(ui: &mut egui::Ui, node: &Node, inputs: &mut Inputs, window: WindowId, out: &mut Vec<FromHelper>) -> Result<egui::Rect, String> {
    let (width, height) = (opt_points(node, "width")?, opt_points(node, "height")?);
    let room = ui.available_size();
    let size = egui::vec2(width.unwrap_or(room.x), height.unwrap_or(room.y));
    if !size.x.is_finite() || !size.y.is_finite() {
        return Err(String::from("a Canvas without a width and a height takes the room its place leaves, and inside a Scroll that room has no end"));
    }
    let wanted = pointer::Wanted::of(node)?;
    let items = ItemEvents::of(node)?;
    let (view_drags, view_double) = match inputs.canvases.get(&node.id) {
        Some(c) => (c.steering.takes_drags(), c.steering.takes_double_clicks()),
        None => (false, false),
    };
    let focusable = focusable(&inputs.canvases, node)?;
    let take = inputs.take_focus(node.id);
    let sense = if items.drags() || view_drags {
        egui::Sense::click_and_drag()
    } else if items.click || items.double || view_double || focusable {
        egui::Sense::click()
    } else {
        egui::Sense::hover()
    };
    let (rect, response) = ui.allocate_exact_size(size, sense);
    let format = inputs.surface_format.ok_or_else(|| String::from("egui has not given the format it draws into yet"))?;
    let text = ui.visuals().text_color().to_array();
    let ctx = ui.ctx().clone();
    let ppp = ctx.pixels_per_point();
    let screen = ctx.viewport_rect().size() * ppp;
    let ((left, right), (shown_left, shown_right)) = crate::surface::edges(rect.min.x, rect.max.x, ppp, screen.x.round());
    let ((top, bottom), (shown_top, shown_bottom)) = crate::surface::edges(rect.min.y, rect.max.y, ppp, screen.y.round());
    let (across, down) = (right - left, bottom - top);
    let canvas = inputs.canvases.entry(node.id).or_default();
    if across < 1.0 || down < 1.0 {
        return Ok(rect);
    }
    let place = pointer::Place { left, top, across, down, ppp };
    canvas.place = Some(place);
    if take && focusable {
        response.request_focus();
    }
    let ring = focusable && crate::routed::focus_keys(ui, &response, &mut canvas.focus_from_press);
    if sense.senses_drag() {
        wake_when_held(ui, &response);
    }
    let now = Instant::now();
    let points = [f64::from(across / ppp), f64::from(down / ppp)];
    let fit_all = canvas.drawn_box(&ViewFit::All);
    let previous = canvas.made.as_ref().map(|(_, f)| Arc::clone(f));
    let takes = |at: egui::Pos2, taken: Taken| {
        let asks = match taken {
            Taken::Drag => items.drags(),
            Taken::DoubleClick => items.double,
        };
        let (x, y) = place.pixels(at);
        asks && previous.as_ref().is_some_and(|f| f.item_at([x, y]).is_some())
    };
    let steered = view::steer(ui, &response, place, &mut canvas.steering, &takes, fit_all, now);
    let placing = placing_of(canvas.steering.shown(now, points).as_ref(), points).0.map(f64::to_bits);
    let made = Made { version: canvas.version, size: [across as u32, down as u32], ppp, text, placing };
    let moving = canvas.motions.moving(now);
    let still = !moving && !canvas.animating;
    let reused = canvas.made.as_ref().filter(|(m, f)| *m == made && !f.pending && still).map(|(_, f)| Arc::clone(f));
    let frame = match reused {
        Some(f) => f,
        None => {
            let f = Arc::new(frame_of(canvas, [across, down], ppp, Rgba(text), Some(&ctx), now));
            canvas.made = Some((made, Arc::clone(&f)));
            f
        }
    };
    canvas.animating = moving;
    if let Some(fit) = canvas.fit_pending.take()
        && let Some(v) = canvas.steering.view.clone()
    {
        canvas.steering.view = Some(canvas.fitted(v, &fit));
        ctx.request_repaint();
    }
    if frame.pending || moving || canvas.steering.moving(now) {
        ctx.request_repaint();
    }
    let view = canvas.drawn_view.clone();
    let mut problem = frame.problems.first().cloned();
    while let Some(p) = canvas.problems.pop() {
        problem = Some(p);
    }
    if let Some(p) = problem
        && canvas.reported.as_ref() != Some(&p)
    {
        canvas.reported = Some(p.clone());
        out.push(FromHelper::Error { window: Some(window), context: String::from("draw"), text: format!("Canvas {}: {p}", node.name.as_deref().unwrap_or("")) });
    }
    let item_at = |at: egui::Pos2| {
        let (x, y) = place.pixels(at);
        frame.item_at([x, y]).map(str::to_string)
    };
    let keys = ctx.input(|i| i.modifiers);
    if items.enter || items.leave {
        let over = response.hover_pos().and_then(item_at);
        if over != canvas.hovered {
            let at = response.hover_pos().or_else(|| ctx.input(|i| i.pointer.latest_pos())).unwrap_or(rect.center());
            if let Some(gone) = canvas.hovered.take()
                && items.leave
            {
                event(out, window, node.id, EventKind::PointerLeave, item_value(place, view.as_ref(), at, None, keys, Some(&gone)));
            }
            if let Some(entered) = &over
                && items.enter
            {
                event(out, window, node.id, EventKind::PointerEnter, item_value(place, view.as_ref(), at, None, keys, Some(entered)));
            }
            canvas.hovered = over;
        }
    }
    let pressed_at = || ctx.input(|i| i.pointer.interact_pos()).unwrap_or(rect.center());
    // Space and Enter click a focused widget in egui; an item is clicked by the pointer alone.
    for (asked, happened, kind) in [(items.click, response.clicked_by(egui::PointerButton::Primary), EventKind::Click), (items.double, response.double_clicked(), EventKind::DoubleClick)] {
        if asked && happened {
            let at = pressed_at();
            if let Some(i) = item_at(at) {
                event(out, window, node.id, kind, item_value(place, view.as_ref(), at, Some("Left"), keys, Some(&i)));
            }
        }
    }
    if items.drags() {
        if response.drag_started_by(egui::PointerButton::Primary) && !steered.took_drag {
            let origin = ctx.input(|i| i.pointer.press_origin()).unwrap_or(rect.center());
            if let Some(i) = item_at(origin) {
                if items.drag_start {
                    event(out, window, node.id, EventKind::DragStart, item_value(place, view.as_ref(), origin, Some("Left"), keys, Some(&i)));
                }
                canvas.dragging = Some((i, Some(origin)));
            }
        }
        if let Some((dragged, last)) = canvas.dragging.as_mut() {
            let latest = ctx.input(|i| i.pointer.latest_pos());
            if response.dragged()
                && let Some(at) = latest
                && *last != Some(at)
            {
                *last = Some(at);
                if items.drag_move {
                    event(out, window, node.id, EventKind::DragMove, item_value(place, view.as_ref(), at, Some("Left"), keys, Some(dragged)));
                }
            }
            if response.drag_stopped() {
                if items.drag_end {
                    event(out, window, node.id, EventKind::DragEnd, item_value(place, view.as_ref(), latest.unwrap_or(rect.center()), Some("Left"), keys, Some(dragged)));
                }
                canvas.dragging = None;
            }
        }
    }
    if wanted.any() {
        let before = out.len();
        let drawn = pointer::Drawn { rect, place, over: response.contains_pointer() };
        canvas.pointer.report(&ctx, &drawn, wanted, (window, node.id), out);
        for raised in &mut out[before..] {
            if let FromHelper::Event { value: Value::Map(m), .. } = raised {
                let pixel = |key: &str| m.get(key).and_then(Value::as_number).map(|v| v as f32);
                let item = match (pixel("x"), pixel("y")) {
                    (Some(x), Some(y)) => frame.item_at([x, y]).map(|i| Value::Str(i.to_string())),
                    _ => None,
                };
                m.insert(String::from("item"), item.unwrap_or(Value::Null));
                with_scene(m, place, view.as_ref());
            }
        }
    }
    let window_uv = [(shown_left - left) / across, (shown_top - top) / down, (shown_right - left) / across, (shown_bottom - top) / down];
    let painting = Painting {
        frame,
        size: [across as u32, down as u32],
        window: window_uv,
        format,
        targets: Arc::clone(&canvas.targets),
        recorded: Arc::clone(&canvas.recorded),
        problems: Arc::clone(&canvas.problems),
    };
    ui.painter().add(egui_wgpu::Callback::new_paint_callback(rect, painting));
    if let Some(area) = steered.drawn_box {
        view::draw_box(&ui.painter_at(rect), area);
    }
    if ring {
        crate::routed::focus_ring(ui, rect);
    }
    Ok(rect)
}

/// What a test event of `kind` on the canvas `node`, named `name`, raises
/// with `given`: a map of `item`, the name of an item to put the pointer on
/// at the middle of where it was drawn, or of `u` and `v`, fractions of the
/// canvas, with what a GpuSurface's test event takes besides, its keys
/// compared without regard to case. The event names the topmost item at
/// that point, as a person's pointer there would; an item event where no
/// named item is raises nothing. An event the node does not request is
/// refused.
pub(crate) fn test_event(canvas: &Canvas, node: &Node, name: &str, kind: EventKind, given: &Value) -> Result<Option<Value>, String> {
    let item_event = matches!(kind, EventKind::Click | EventKind::DoubleClick | EventKind::PointerEnter | EventKind::PointerLeave | EventKind::DragStart | EventKind::DragMove | EventKind::DragEnd);
    let asked = match kind {
        EventKind::Click => flag(node, "click", false)?,
        EventKind::DoubleClick => flag(node, "doubleClick", false)?,
        EventKind::PointerEnter => flag(node, "pointerEnter", false)?,
        EventKind::PointerLeave => flag(node, "pointerLeave", false)?,
        EventKind::DragStart => flag(node, "dragStart", false)?,
        EventKind::DragMove => flag(node, "dragMove", false)?,
        EventKind::DragEnd => flag(node, "dragEnd", false)?,
        EventKind::PointerDown | EventKind::PointerUp | EventKind::PointerMove | EventKind::Wheel => pointer::Wanted::of(node)?.has(kind),
        other => return Err(format!("a Canvas does not raise {other:?}")),
    };
    if !asked {
        return Err(format!("{name} raises no {kind:?}; its command's -On{kind:?} gives it one"));
    }
    let place = canvas.place.ok_or_else(|| format!("Canvas {name} has not been drawn, so there is no place to put the pointer on"))?;
    let (_, frame) = canvas.made.as_ref().ok_or_else(|| format!("Canvas {name} has not been drawn, so there is no place to put the pointer on"))?;
    let mut keys = match given {
        Value::Map(m) => m.clone(),
        Value::Null => BTreeMap::new(),
        other => return Err(format!("a canvas's test event takes a hashtable of Item, or of U and V, not {other:?}")),
    };
    if let Some(key) = keys.keys().find(|k| k.eq_ignore_ascii_case("item")).cloned() {
        let wanted = match keys.remove(&key) {
            Some(Value::Str(s)) => s,
            other => return Err(format!("Item names the item to put the pointer on, and is {other:?}")),
        };
        let [u, v] = item_middle(canvas, &wanted).ok_or_else(|| format!("{name} drew no item named {wanted}"))?;
        keys.insert(String::from("u"), Value::Float(u));
        keys.insert(String::from("v"), Value::Float(v));
    }
    let mut value = match pointer::test_value(place, kind_for_surface(kind), &Value::Map(keys))? {
        Value::Map(m) => m,
        other => return Err(format!("a pointer event's value is a map, not {other:?}")),
    };
    let pixel = |key: &str| value.get(key).and_then(Value::as_number).map(|n| n as f32);
    let found = match (pixel("x"), pixel("y")) {
        (Some(x), Some(y)) => frame.item_at([x, y]).map(str::to_string),
        _ => None,
    };
    if item_event && found.is_none() {
        return Ok(None);
    }
    let held = !matches!(kind, EventKind::PointerEnter | EventKind::PointerLeave);
    if item_event && held && value.get("button").is_none_or(|b| matches!(b, Value::Null)) {
        value.insert(String::from("button"), Value::Str(String::from("Left")));
    }
    value.insert(String::from("item"), found.map_or(Value::Null, Value::Str));
    with_scene(&mut value, place, canvas.drawn_view.as_ref());
    Ok(Some(Value::Map(value)))
}

/// The GpuSurface event whose test value a canvas event of `kind` takes.
fn kind_for_surface(kind: EventKind) -> EventKind {
    match kind {
        EventKind::PointerUp | EventKind::DragEnd => EventKind::PointerUp,
        EventKind::PointerMove | EventKind::DragMove | EventKind::PointerEnter | EventKind::PointerLeave => EventKind::PointerMove,
        EventKind::Wheel => EventKind::Wheel,
        _ => EventKind::PointerDown,
    }
}

/// The middle of where the item named `name` was drawn on a canvas, as
/// fractions of the canvas's width and height, which a test event without
/// a place of its own goes to.
fn item_middle(canvas: &Canvas, name: &str) -> Option<[f64; 2]> {
    let (_, frame) = canvas.made.as_ref()?;
    let place = canvas.place?;
    let b = frame.hits.iter().rev().find(|h| h.name.eq_ignore_ascii_case(name))?.span?;
    Some([(b.x + b.width / 2.0) / f64::from(place.across), (b.y + b.height / 2.0) / f64::from(place.down)])
}

/// The texture a frame of a canvas draws into, the layers its translucent
/// and clipped groups draw into, and what the blit shows it through.
pub(crate) struct Targets {
    size: [u32; 2],
    main: Target,
    layers: Vec<Target>,
    screen: wgpu::Buffer,
    screen_group: wgpu::BindGroup,
    window: wgpu::Buffer,
    shown: wgpu::BindGroup,
}

/// A multisampled texture drawn into, the texture it resolves into, and
/// the bind group that draws from the resolved one.
struct Target {
    multisampled: wgpu::TextureView,
    resolved: wgpu::TextureView,
    sampled: wgpu::BindGroup,
}

impl Target {
    fn new(device: &wgpu::Device, gpu: &Pipelines, size: [u32; 2], label: &str) -> Target {
        let extent = wgpu::Extent3d { width: size[0], height: size[1], depth_or_array_layers: 1 };
        let make = |samples: u32, usage: wgpu::TextureUsages| {
            device
                .create_texture(&wgpu::TextureDescriptor { label: Some(label), size: extent, mip_level_count: 1, sample_count: samples, dimension: wgpu::TextureDimension::D2, format: FORMAT, usage, view_formats: &[] })
                .create_view(&wgpu::TextureViewDescriptor::default())
        };
        let multisampled = make(SAMPLES, wgpu::TextureUsages::RENDER_ATTACHMENT);
        let resolved = make(1, wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_SRC);
        let sampled = gpu.texture_group(device, &resolved, label);
        Target { multisampled, resolved, sampled }
    }
}

impl Targets {
    fn new(device: &wgpu::Device, gpu: &Pipelines, size: [u32; 2], layers: usize) -> Targets {
        let main = Target::new(device, gpu, size, "PWRS-GUI Canvas");
        let layers = (0..layers).map(|_| Target::new(device, gpu, size, "PWRS-GUI Canvas layer")).collect();
        let uniform = |label: &str| device.create_buffer(&wgpu::BufferDescriptor { label: Some(label), size: 16, usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });
        let screen = uniform("PWRS-GUI Canvas screen");
        let screen_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("PWRS-GUI Canvas screen"),
            layout: &gpu.screen_layout,
            entries: &[wgpu::BindGroupEntry { binding: 0, resource: screen.as_entire_binding() }],
        });
        let window = uniform("PWRS-GUI Canvas window");
        let shown = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("PWRS-GUI Canvas shown"),
            layout: &gpu.blit_layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&main.resolved) },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(&gpu.nearest) },
                wgpu::BindGroupEntry { binding: 2, resource: window.as_entire_binding() },
            ],
        });
        Targets { size, main, layers, screen, screen_group, window, shown }
    }
}

/// Draws a canvas's triangles and composites its layers, `screen` the
/// canvas's size in pixels: each corner's color times the texel it takes,
/// a texel of egui's own sRGB textures read back to gamma first as egui's
/// shader reads it, and a layer's texel at the pixel drawn times the mask's
/// opacity.
const SHADER: &str = "
struct Screen { size: vec2<f32>, unused: vec2<f32> }
@group(0) @binding(0) var<uniform> screen: Screen;
@group(1) @binding(0) var texels: texture_2d<f32>;
@group(1) @binding(1) var filtered: sampler;
struct Corner { @builtin(position) position: vec4<f32>, @location(0) uv: vec2<f32>, @location(1) color: vec4<f32> }
@vertex fn vs(@location(0) at: vec2<f32>, @location(1) uv: vec2<f32>, @location(2) color: vec4<f32>) -> Corner {
    var out: Corner;
    out.position = vec4<f32>(at.x / screen.size.x * 2.0 - 1.0, 1.0 - at.y / screen.size.y * 2.0, 0.0, 1.0);
    out.uv = uv;
    out.color = color;
    return out;
}
fn gamma(linear: vec3<f32>) -> vec3<f32> {
    let low = linear * 12.92;
    let high = 1.055 * pow(linear, vec3<f32>(1.0 / 2.4)) - 0.055;
    return select(high, low, linear < vec3<f32>(0.0031308));
}
@fragment fn fs_mesh(in: Corner) -> @location(0) vec4<f32> {
    let t = textureSample(texels, filtered, in.uv);
    return in.color * vec4<f32>(gamma(t.rgb), t.a);
}
@fragment fn fs_layer(in: Corner) -> @location(0) vec4<f32> {
    return textureSample(texels, filtered, in.position.xy / screen.size) * in.color.a;
}
";

/// Shows a canvas's resolved texture in egui's pass, blended over what
/// egui drew under it, the part of it the window shows mapped onto the
/// viewport egui gives the callback, texel for pixel. A target in an sRGB
/// format is given the texture's bytes as linear values, so the target's
/// encoding stores the same bytes.
const BLIT: &str = "
struct Shown { top_left: vec2<f32>, bottom_right: vec2<f32> }
@group(0) @binding(0) var texels: texture_2d<f32>;
@group(0) @binding(1) var nearest: sampler;
@group(0) @binding(2) var<uniform> shown: Shown;
struct Corner { @builtin(position) position: vec4<f32>, @location(0) uv: vec2<f32> }
@vertex fn vs(@builtin(vertex_index) i: u32) -> Corner {
    let at = vec2<f32>(f32((i << 1u) & 2u), f32(i & 2u));
    var out: Corner;
    out.position = vec4<f32>(at.x * 2.0 - 1.0, 1.0 - at.y * 2.0, 0.0, 1.0);
    out.uv = shown.top_left + at * (shown.bottom_right - shown.top_left);
    return out;
}
@fragment fn fs_gamma(in: Corner) -> @location(0) vec4<f32> {
    return textureSample(texels, nearest, in.uv);
}
@fragment fn fs_linear(in: Corner) -> @location(0) vec4<f32> {
    let c = textureSample(texels, nearest, in.uv);
    let low = c.rgb / 12.92;
    let high = pow((c.rgb + 0.055) / 1.055, vec3<f32>(2.4));
    return vec4<f32>(select(high, low, c.rgb <= vec3<f32>(0.04045)), c.a);
}
";

/// The pipelines and samplers every canvas draws with, for egui's target
/// format, stored in egui-wgpu's callback resources.
struct Pipelines {
    format: wgpu::TextureFormat,
    screen_layout: wgpu::BindGroupLayout,
    texture_layout: wgpu::BindGroupLayout,
    blit_layout: wgpu::BindGroupLayout,
    mesh: wgpu::RenderPipeline,
    layer: wgpu::RenderPipeline,
    blit: wgpu::RenderPipeline,
    smooth: wgpu::Sampler,
    nearest: wgpu::Sampler,
    white: wgpu::TextureView,
}

impl Pipelines {
    fn new(device: &wgpu::Device, queue: &wgpu::Queue, format: wgpu::TextureFormat) -> Pipelines {
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("PWRS-GUI Canvas"), source: wgpu::ShaderSource::Wgsl(SHADER.into()) });
        let blit_module = device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("PWRS-GUI Canvas blit"), source: wgpu::ShaderSource::Wgsl(BLIT.into()) });
        let uniform_entry = |binding: u32, visibility: wgpu::ShaderStages| wgpu::BindGroupLayoutEntry {
            binding,
            visibility,
            ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None },
            count: None,
        };
        let texture_entry = wgpu::BindGroupLayoutEntry {
            binding: 0,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Texture { multisampled: false, sample_type: wgpu::TextureSampleType::Float { filterable: true }, view_dimension: wgpu::TextureViewDimension::D2 },
            count: None,
        };
        let sampler_entry = wgpu::BindGroupLayoutEntry { binding: 1, visibility: wgpu::ShaderStages::FRAGMENT, ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering), count: None };
        let screen_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor { label: Some("PWRS-GUI Canvas screen"), entries: &[uniform_entry(0, wgpu::ShaderStages::VERTEX_FRAGMENT)] });
        let texture_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor { label: Some("PWRS-GUI Canvas texels"), entries: &[texture_entry, sampler_entry] });
        let blit_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("PWRS-GUI Canvas blit"),
            entries: &[texture_entry, sampler_entry, uniform_entry(2, wgpu::ShaderStages::VERTEX)],
        });
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some("PWRS-GUI Canvas"), bind_group_layouts: &[Some(&screen_layout), Some(&texture_layout)], immediate_size: 0 });
        let blit_pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some("PWRS-GUI Canvas blit"), bind_group_layouts: &[Some(&blit_layout)], immediate_size: 0 });
        let attributes = [
            wgpu::VertexAttribute { format: wgpu::VertexFormat::Float32x2, offset: 0, shader_location: 0 },
            wgpu::VertexAttribute { format: wgpu::VertexFormat::Float32x2, offset: 8, shader_location: 1 },
            wgpu::VertexAttribute { format: wgpu::VertexFormat::Float32x4, offset: 16, shader_location: 2 },
        ];
        let buffers = [Some(wgpu::VertexBufferLayout { array_stride: 32, step_mode: wgpu::VertexStepMode::Vertex, attributes: &attributes })];
        let blend = Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING);
        let pipeline = |label: &str, fragment: &str| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(label),
                layout: Some(&layout),
                vertex: wgpu::VertexState { module: &module, entry_point: Some("vs"), buffers: &buffers, compilation_options: wgpu::PipelineCompilationOptions::default() },
                primitive: wgpu::PrimitiveState::default(),
                depth_stencil: None,
                multisample: wgpu::MultisampleState { count: SAMPLES, mask: !0, alpha_to_coverage_enabled: false },
                fragment: Some(wgpu::FragmentState {
                    module: &module,
                    entry_point: Some(fragment),
                    targets: &[Some(wgpu::ColorTargetState { format: FORMAT, blend, write_mask: wgpu::ColorWrites::ALL })],
                    compilation_options: wgpu::PipelineCompilationOptions::default(),
                }),
                multiview_mask: None,
                cache: None,
            })
        };
        let mesh = pipeline("PWRS-GUI Canvas triangles", "fs_mesh");
        let layer = pipeline("PWRS-GUI Canvas layer", "fs_layer");
        let blit = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("PWRS-GUI Canvas blit"),
            layout: Some(&blit_pipeline_layout),
            vertex: wgpu::VertexState { module: &blit_module, entry_point: Some("vs"), buffers: &[], compilation_options: wgpu::PipelineCompilationOptions::default() },
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            fragment: Some(wgpu::FragmentState {
                module: &blit_module,
                entry_point: Some(if format.is_srgb() { "fs_linear" } else { "fs_gamma" }),
                targets: &[Some(wgpu::ColorTargetState { format, blend, write_mask: wgpu::ColorWrites::ALL })],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            }),
            multiview_mask: None,
            cache: None,
        });
        let sampler = |label: &str, filter: wgpu::FilterMode| {
            device.create_sampler(&wgpu::SamplerDescriptor { label: Some(label), mag_filter: filter, min_filter: filter, ..wgpu::SamplerDescriptor::default() })
        };
        let smooth = sampler("PWRS-GUI Canvas texels", wgpu::FilterMode::Linear);
        let nearest = sampler("PWRS-GUI Canvas blit", wgpu::FilterMode::Nearest);
        let white = device
            .create_texture_with_data(
                queue,
                &wgpu::TextureDescriptor {
                    label: Some("PWRS-GUI Canvas white"),
                    size: wgpu::Extent3d { width: 1, height: 1, depth_or_array_layers: 1 },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format: wgpu::TextureFormat::Rgba8UnormSrgb,
                    usage: wgpu::TextureUsages::TEXTURE_BINDING,
                    view_formats: &[],
                },
                wgpu::util::TextureDataOrder::LayerMajor,
                &[255, 255, 255, 255],
            )
            .create_view(&wgpu::TextureViewDescriptor::default());
        Pipelines { format, screen_layout, texture_layout, blit_layout, mesh, layer, blit, smooth, nearest, white }
    }

    /// The bind group a texture is drawn from with the smooth sampler.
    fn texture_group(&self, device: &wgpu::Device, view: &wgpu::TextureView, label: &str) -> wgpu::BindGroup {
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some(label),
            layout: &self.texture_layout,
            entries: &[wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(view) }, wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(&self.smooth) }],
        })
    }
}

/// One frame of a canvas as the paint callback draws it: its steps, at
/// `size` pixels, the part `window` names shown in egui's pass of format
/// `format`, with what goes wrong drawing it pushed to `problems`. The
/// frame is recorded into `targets` only when they contain another, as
/// `recorded` says.
struct Painting {
    frame: Arc<Frame>,
    size: [u32; 2],
    window: [f32; 4],
    format: wgpu::TextureFormat,
    targets: Arc<ArcSwapOption<Targets>>,
    recorded: Arc<ArcSwapOption<Frame>>,
    problems: Arc<SegQueue<String>>,
}

/// One draw a pass records: the triangles from index `first` to `first +
/// count`, a layer's mask or not, and the bind group of the texels they
/// take.
struct Draw {
    first: u32,
    count: u32,
    layer: bool,
    group: wgpu::BindGroup,
}

/// What recording a canvas reads.
struct Recording<'a> {
    device: &'a wgpu::Device,
    gpu: &'a Pipelines,
    targets: &'a Targets,
    textures: &'a egui_wgpu::EguiTextures,
    white: wgpu::BindGroup,
    problems: &'a SegQueue<String>,
}

impl Recording<'_> {
    /// Records `ops` into `target`, cleared first, after drawing each layer
    /// among them into the next of the targets' layers, `next` on.
    fn record(&self, encoder: &mut wgpu::CommandEncoder, ops: &[Op], target: &Target, next: &mut usize) {
        let mut vertices: Vec<u8> = Vec::new();
        let mut indices: Vec<u32> = Vec::new();
        let mut draws: Vec<Draw> = Vec::new();
        let mut push = |vs: &[Vertex], is: &[u32], layer: bool, group: wgpu::BindGroup| {
            let base = (vertices.len() / 32) as u32;
            for v in vs {
                for f in v.at.iter().chain(&v.uv).chain(&v.color) {
                    vertices.extend_from_slice(&f.to_le_bytes());
                }
            }
            draws.push(Draw { first: indices.len() as u32, count: is.len() as u32, layer, group });
            indices.extend(is.iter().map(|i| i + base));
        };
        for op in ops {
            match op {
                Op::Mesh(mesh) => {
                    let (vs, is) = (&mesh.vertices, &mesh.indices);
                    let group = match &mesh.texture {
                        Tex::White => self.white.clone(),
                        Tex::Egui(id) => match self.textures.0.get(id) {
                            Some(t) => self.gpu.texture_group(self.device, &t.create_view(&wgpu::TextureViewDescriptor::default()), "PWRS-GUI Canvas egui texture"),
                            None => {
                                self.problems.push(format!("egui has no texture {id:?}, which a picture is drawn from"));
                                continue;
                            }
                        },
                    };
                    push(vs, is, false, group);
                }
                Op::Layer { ops: inner, mask, mask_indices, .. } => {
                    let slot = *next;
                    *next += 1;
                    let Some(layer) = self.targets.layers.get(slot) else {
                        self.problems.push(format!("layer {slot} has no texture of its own"));
                        continue;
                    };
                    self.record(encoder, inner, layer, next);
                    push(mask, mask_indices, true, layer.sampled.clone());
                }
            }
        }
        let made = |label: &str, contents: &[u8], usage: wgpu::BufferUsages| self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor { label: Some(label), contents, usage });
        let index_bytes: Vec<u8> = indices.iter().flat_map(|i| i.to_le_bytes()).collect();
        let buffers = (!vertices.is_empty() && !index_bytes.is_empty())
            .then(|| (made("PWRS-GUI Canvas vertices", &vertices, wgpu::BufferUsages::VERTEX), made("PWRS-GUI Canvas indices", &index_bytes, wgpu::BufferUsages::INDEX)));
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("PWRS-GUI Canvas"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &target.multisampled,
                depth_slice: None,
                resolve_target: Some(&target.resolved),
                ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT), store: wgpu::StoreOp::Discard },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        let Some((vertex_buffer, index_buffer)) = &buffers else {
            return;
        };
        pass.set_bind_group(0, &self.targets.screen_group, &[]);
        pass.set_vertex_buffer(0, vertex_buffer.slice(..));
        pass.set_index_buffer(index_buffer.slice(..), wgpu::IndexFormat::Uint32);
        for d in draws.iter().filter(|d| d.count > 0) {
            pass.set_pipeline(if d.layer { &self.gpu.layer } else { &self.gpu.mesh });
            pass.set_bind_group(1, &d.group, &[]);
            pass.draw_indexed(d.first..d.first + d.count, 0, 0..1);
        }
    }
}

impl egui_wgpu::CallbackTrait for Painting {
    fn prepare(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        _screen: &egui_wgpu::ScreenDescriptor,
        _encoder: &mut wgpu::CommandEncoder,
        resources: &mut egui_wgpu::CallbackResources,
    ) -> Vec<wgpu::CommandBuffer> {
        if resources.get::<Pipelines>().is_none_or(|p| p.format != self.format) {
            resources.insert(Pipelines::new(device, queue, self.format));
        }
        let Some(gpu) = resources.get::<Pipelines>() else {
            return Vec::new();
        };
        let layers = layer_count(&self.frame.ops);
        let targets = match self.targets.load_full() {
            Some(t) if t.size == self.size && t.layers.len() >= layers => t,
            _ => {
                let made = Arc::new(Targets::new(device, gpu, self.size, layers));
                self.targets.store(Some(Arc::clone(&made)));
                self.recorded.store(None);
                made
            }
        };
        let floats = |values: [f32; 4]| -> Vec<u8> { values.iter().flat_map(|v| v.to_le_bytes()).collect() };
        queue.write_buffer(&targets.window, 0, &floats(self.window));
        if self.recorded.load().as_ref().is_some_and(|held| Arc::ptr_eq(held, &self.frame)) {
            return Vec::new();
        }
        queue.write_buffer(&targets.screen, 0, &floats([self.size[0] as f32, self.size[1] as f32, 0.0, 0.0]));
        let none = egui_wgpu::EguiTextures(Default::default());
        let textures = match resources.get::<egui_wgpu::EguiTextures>() {
            Some(t) => t,
            None => {
                if self.frame.takes_egui_textures() {
                    self.problems.push(String::from("egui-wgpu gave the canvas none of egui's textures, so its pictures are not drawn"));
                }
                &none
            }
        };
        let recording = Recording { device, gpu, targets: &targets, textures, white: gpu.texture_group(device, &gpu.white, "PWRS-GUI Canvas white"), problems: &self.problems };
        // The canvas records into an encoder of its own and submits it here,
        // so a frame counts as recorded only once its recording is on the
        // queue, whatever becomes of the encoder egui-wgpu prepares callbacks
        // into.
        let mut own = device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("PWRS-GUI Canvas") });
        recording.record(&mut own, &self.frame.ops, &targets.main, &mut 0);
        queue.submit([own.finish()]);
        self.recorded.store(Some(Arc::clone(&self.frame)));
        Vec::new()
    }

    fn paint(&self, _info: egui::PaintCallbackInfo, pass: &mut wgpu::RenderPass<'static>, resources: &egui_wgpu::CallbackResources) {
        let (Some(gpu), Some(targets)) = (resources.get::<Pipelines>(), self.targets.load_full()) else {
            return;
        };
        pass.set_pipeline(&gpu.blit);
        pass.set_bind_group(0, &targets.shown, &[]);
        pass.draw(0..3, 0..1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui_wgpu::CallbackTrait;
    use pwrs_gui_gpu::{Gpu, GpuOptions};

    /// A canvas with `items`, their animations started at `now`.
    fn canvas_at(items: &[Value], now: Instant) -> Canvas {
        let board = match Board::of_items(&Value::List(items.to_vec())) {
            Ok(b) => b,
            Err(why) => panic!("the board reads: {why}"),
        };
        let mut canvas = Canvas::default();
        canvas.motions.started(&board, &board.all(), now);
        canvas.board = board;
        canvas
    }

    fn canvas(items: &[Value]) -> Canvas {
        canvas_at(items, Instant::now())
    }

    /// A frame of `items` `size` pixels across and down at `ppp` pixels a
    /// point, in black text, without egui.
    fn made(items: &[Value], size: [f32; 2], ppp: f32) -> Frame {
        frame_of(&mut canvas(items), size, ppp, BLACK, None, Instant::now())
    }

    fn item(pairs: &[(&str, Value)]) -> Value {
        Value::Map(pairs.iter().map(|(k, v)| (k.to_string(), v.clone())).collect())
    }

    fn n(v: f64) -> Value {
        Value::Float(v)
    }

    fn s(v: &str) -> Value {
        Value::Str(v.to_string())
    }

    const BLACK: Rgba = Rgba([0, 0, 0, 255]);

    fn first_mesh(ops: &[Op]) -> &[Vertex] {
        match ops.first() {
            Some(Op::Mesh(mesh)) => &mesh.vertices,
            other => panic!("triangles first: {other:?}"),
        }
    }

    #[test]
    fn a_transform_turns_clockwise_skews_scales_and_inverts() {
        let turned = Affine::rotate(90.0).apply([1.0, 0.0]);
        assert!(turned[0].abs() < 1e-12 && (turned[1] - 1.0).abs() < 1e-12, "a quarter turn clockwise takes +x to +y, y growing down: {turned:?}");
        let pivoted = placed(&Style { rotate: 180.0, ..Style::default() }, [10.0, 10.0]).apply([12.0, 10.0]);
        assert!((pivoted[0] - 8.0).abs() < 1e-9 && (pivoted[1] - 10.0).abs() < 1e-9, "a half turn about the pivot: {pivoted:?}");
        let skewed = Affine::skew(45.0, 0.0).apply([0.0, 2.0]);
        assert!((skewed[0] - 2.0).abs() < 1e-9, "a skew of 45 degrees along x moves x by y: {skewed:?}");
        let any = Affine::translate(3.0, -4.0).then_inner(Affine::rotate(30.0)).then_inner(Affine::scale(2.0, 0.5));
        let back = any.inverse().map(|i| i.apply(any.apply([7.0, 11.0])));
        assert!(back.is_some_and(|b| (b[0] - 7.0).abs() < 1e-9 && (b[1] - 11.0).abs() < 1e-9), "{back:?}");
    }

    #[test]
    fn a_gradient_blends_between_the_stops_around_a_point() {
        let paint = Paint::Linear { colors: vec![Rgba([255, 0, 0, 255]), Rgba([0, 0, 255, 255])], stops: vec![0.0, 1.0], from: [0.0, 0.0], to: [1.0, 0.0] };
        let middle = paint_at(&paint, [0.5, 0.3], 1.0);
        assert!((middle[0] - 0.5).abs() < 1e-6 && (middle[2] - 0.5).abs() < 1e-6 && (middle[3] - 1.0).abs() < 1e-6, "{middle:?}");
        assert_eq!(paint_at(&paint, [-1.0, 0.0], 1.0), [1.0, 0.0, 0.0, 1.0], "before the first stop it takes the first color");
        let radial = Paint::Radial { colors: vec![Rgba([255, 255, 255, 255]), Rgba([0, 0, 0, 255])], stops: vec![0.0, 1.0], center: [0.5, 0.5], radius: 0.5 };
        assert_eq!(paint_at(&radial, [1.0, 0.5], 1.0), [0.0, 0.0, 0.0, 1.0], "at the radius it takes the last color");
        assert_eq!(vertex_color(Rgba([255, 255, 255, 255]), 0.5), [0.5; 4], "opacity premultiplies");
    }

    #[test]
    fn a_gradient_s_triangles_are_cut_until_each_edge_is_short() {
        let (positions, indices) = subdivided(vec![[0.0, 0.0], [20.0, 0.0], [0.0, 20.0]], vec![0, 1, 2]);
        let length = |a: [f32; 2], b: [f32; 2]| ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2)).sqrt();
        assert!(indices.len() > 3 && indices.len() % 3 == 0);
        for t in indices.as_chunks::<3>().0 {
            for (a, b) in [(t[0], t[1]), (t[1], t[2]), (t[2], t[0])] {
                assert!(length(positions[a as usize], positions[b as usize]) <= GRADIENT_STEP + 1e-4);
            }
        }
    }

    #[test]
    fn a_dash_pattern_keeps_the_dashes_along_a_line() {
        let line = svg_path(&Shape::Line([0.0, 0.0], [10.0, 0.0]).outline().expect("a line's outline"));
        let begins = |p: &Path| p.iter().filter(|e| matches!(e, PathEvent::Begin { .. })).count();
        assert_eq!(begins(&dashes(&line, &[2.0, 3.0], 0.01)), 2, "dashes at 0 to 2 and 5 to 7 of 10");
        assert_eq!(begins(&dashes(&line, &[4.0], 0.01)), 2, "one length is dash and gap alike, as SVG doubles it");
    }

    #[test]
    fn a_frame_fills_strokes_and_finds_the_topmost_named_item() {
        let items = [
            item(&[("kind", s("Rect")), ("name", s("Under")), ("x", n(0.0)), ("y", n(0.0)), ("width", n(40.0)), ("height", n(40.0)), ("fill", s("Red"))]),
            item(&[("kind", s("Rect")), ("name", s("Over")), ("x", n(20.0)), ("y", n(20.0)), ("width", n(40.0)), ("height", n(40.0)), ("fill", s("Blue")), ("stroke", s("Black"))]),
            item(&[("kind", s("Line")), ("name", s("Rule")), ("x1", n(0.0)), ("y1", n(70.0)), ("x2", n(60.0)), ("y2", n(70.0))]),
        ];
        let frame = made(&items, [100.0, 100.0], 1.0);
        assert_eq!(frame.problems, Vec::<String>::new());
        assert_eq!(frame.ops.len(), 4, "a fill, a fill and a stroke, and the line's default stroke");
        assert_eq!(frame.item_at([10.0, 10.0]), Some("Under"));
        assert_eq!(frame.item_at([30.0, 30.0]), Some("Over"), "the later item is on top");
        assert_eq!(frame.item_at([30.0, 73.0]), Some("Rule"), "within 5 points of a line counts as on it");
        assert_eq!(frame.item_at([90.0, 90.0]), None);
        assert!(first_mesh(&frame.ops).iter().all(|v| v.color == [1.0, 0.0, 0.0, 1.0]), "red, opaque");
        let doubled = made(&items, [200.0, 200.0], 2.0);
        assert_eq!(doubled.item_at([20.0, 20.0]), Some("Under"), "points are doubled into pixels at two pixels a point");
    }

    #[test]
    fn an_outline_takes_events_inside_where_it_closes_or_where_it_is_filled() {
        let points = |p: &[f64]| Value::List(p.iter().map(|v| n(*v)).collect());
        let items = [
            item(&[("kind", s("Rect")), ("name", s("Outlined")), ("x", n(0.0)), ("y", n(0.0)), ("width", n(40.0)), ("height", n(40.0)), ("stroke", s("Black"))]),
            item(&[("kind", s("Polyline")), ("name", s("Open")), ("points", points(&[60.0, 0.0, 100.0, 0.0, 100.0, 40.0])), ("stroke", s("Black"))]),
            item(&[("kind", s("Polyline")), ("name", s("Filled")), ("points", points(&[0.0, 60.0, 40.0, 60.0, 40.0, 100.0])), ("fill", s("Red"))]),
            item(&[("kind", s("Path")), ("name", s("Mixed")), ("data", s("M60 60 L100 60 L100 100 Z M120 60 L160 60 L160 100")), ("stroke", s("Black"))]),
        ];
        let frame = made(&items, [200.0, 120.0], 1.0);
        assert_eq!(frame.item_at([20.0, 20.0]), Some("Outlined"), "an outlined box takes events inside, filled or not");
        assert_eq!(frame.item_at([90.0, 15.0]), None, "an open line takes none across what it would enclose");
        assert_eq!(frame.item_at([100.0, 20.0]), Some("Open"), "it takes them on its line");
        assert_eq!(frame.item_at([30.0, 75.0]), Some("Filled"), "a filled one takes them where its fill paints");
        assert_eq!(frame.item_at([90.0, 75.0]), Some("Mixed"), "a path's closed subpath counts inside");
        assert_eq!(frame.item_at([150.0, 75.0]), None, "its open subpath does not");
    }

    #[test]
    fn a_translucent_or_clipped_group_draws_into_a_layer_and_clips_its_hits() {
        let clip = item(&[("kind", s("Ellipse")), ("x", n(0.0)), ("y", n(0.0)), ("width", n(20.0)), ("height", n(20.0))]);
        let child = item(&[("kind", s("Rect")), ("x", n(0.0)), ("y", n(0.0)), ("width", n(20.0)), ("height", n(20.0)), ("fill", s("Green"))]);
        let items = [item(&[("kind", s("Group")), ("name", s("Badge")), ("x", n(10.0)), ("y", n(10.0)), ("opacity", n(0.5)), ("clip", clip), ("children", child)])];
        let frame = made(&items, [50.0, 50.0], 1.0);
        assert_eq!(layer_count(&frame.ops), 1);
        let Some(Op::Layer { opacity, ops, .. }) = frame.ops.first() else { panic!("a layer: {:?}", frame.ops) };
        assert_eq!((*opacity, ops.len()), (0.5, 1));
        assert_eq!(frame.item_at([20.0, 20.0]), Some("Badge"), "an unnamed item takes its group's name");
        assert_eq!(frame.item_at([11.0, 11.0]), None, "the box's corner is outside the circle that clips it");
        let plain = [item(&[("kind", s("Group")), ("children", item(&[("kind", s("Rect")), ("x", n(0.0)), ("y", n(0.0)), ("width", n(5.0)), ("height", n(5.0))]))])];
        assert_eq!(layer_count(&made(&plain, [10.0, 10.0], 1.0).ops), 0, "an opaque group without a clip draws straight onto the canvas");
    }

    fn near_box(b: Option<Bounds>, want: [f64; 4]) -> bool {
        b.is_some_and(|b| [b.x, b.y, b.width, b.height].iter().zip(want).all(|(got, want)| (got - want).abs() < 1e-6))
    }

    #[test]
    fn a_fit_counts_each_item_s_exact_box_grown_by_half_its_stroke_and_cut_to_its_clips() {
        let stroked = item(&[("kind", s("Rect")), ("name", s("Framed")), ("x", n(10.0)), ("y", n(10.0)), ("width", n(10.0)), ("height", n(10.0)), ("stroke", s("Black")), ("strokewidth", n(4.0))]);
        let turned = item(&[("kind", s("Ellipse")), ("name", s("Turned")), ("x", n(50.0)), ("y", n(0.0)), ("width", n(20.0)), ("height", n(10.0)), ("rotate", n(45.0))]);
        let clipped = item(&[
            ("kind", s("Group")),
            ("name", s("Window")),
            ("clip", item(&[("kind", s("Rect")), ("x", n(0.0)), ("y", n(50.0)), ("width", n(20.0)), ("height", n(20.0))])),
            ("children", item(&[("kind", s("Rect")), ("x", n(0.0)), ("y", n(50.0)), ("width", n(40.0)), ("height", n(40.0)), ("fill", s("Gray"))])),
        ]);
        let frame = made(&[stroked, turned, clipped], [100.0, 100.0], 1.0);
        let span = |name: &str| frame.hits.iter().find(|h| h.name.as_ref() == name).and_then(|h| h.span);
        assert!(near_box(span("Framed"), [8.0, 8.0, 14.0, 14.0]), "half the 4-point stroke beyond the box: {:?}", span("Framed"));
        let half = (50.0 + 12.5f64).sqrt() + 0.5;
        assert!(near_box(span("Turned"), [60.0 - half, 5.0 - half, 2.0 * half, 2.0 * half]), "the turned ellipse's own box, not its corners', with the theme outline's half point: {:?}", span("Turned"));
        assert!(near_box(span("Window"), [0.0, 50.0, 20.0, 20.0]), "cut to the group's clip: {:?}", span("Window"));
        assert!(near_box(drawn_bounds(&frame, &ViewFit::All), [0.0, 5.0 - half, 60.0 + half, 70.0 - (5.0 - half)]), "every item together: {:?}", drawn_bounds(&frame, &ViewFit::All));
    }

    #[test]
    fn a_path_turns_about_its_exact_box_and_a_large_circle_keeps_to_its_circle() {
        let arch = item(&[("kind", s("Path")), ("data", s("M0 0 C0 -100 100 -100 100 0")), ("fill", s("Red")), ("rotate", n(180.0))]);
        let frame = made(&[arch], [100.0, 100.0], 1.0);
        assert!(near_box(frame.spans.first().copied(), [0.0, -75.0, 100.0, 75.0]), "a half turn about the middle of the arch's own box leaves the box where it was: {:?}", frame.spans);
        let circle = item(&[("kind", s("Ellipse")), ("x", n(0.0)), ("y", n(0.0)), ("width", n(1000.0)), ("height", n(1000.0)), ("fill", s("Blue"))]);
        let big = made(&[circle], [1000.0, 1000.0], 1.0);
        let off = first_mesh(&big.ops).iter().map(|v| (f64::from(v.at[0] - 500.0).hypot(f64::from(v.at[1] - 500.0)) - 500.0).abs()).fold(0.0, f64::max);
        assert!(off < 0.02, "every corner of the fill lies on the circle of radius 500 pixels: {off} off");
    }

    #[test]
    fn a_group_passes_its_style_and_turns_about_its_place() {
        let child = item(&[("kind", s("Rect")), ("name", s("Arm")), ("x", n(0.0)), ("y", n(0.0)), ("width", n(10.0)), ("height", n(2.0))]);
        let items = [item(&[("kind", s("Group")), ("x", n(50.0)), ("y", n(50.0)), ("rotate", n(90.0)), ("fill", s("White")), ("children", child)])];
        let frame = made(&items, [100.0, 100.0], 1.0);
        assert!(first_mesh(&frame.ops).iter().all(|v| v.color == [1.0; 4]), "the group's fill passes to its item");
        assert_eq!(frame.item_at([49.0, 55.0]), Some("Arm"), "turned a quarter clockwise about the group's place, the arm points down");
        assert_eq!(frame.item_at([55.0, 49.0]), None);
    }

    fn mesh_at(ops: &[Op], at: usize) -> &Arc<Mesh> {
        match ops.get(at) {
            Some(Op::Mesh(mesh)) => mesh,
            other => panic!("triangles at {at}: {other:?}"),
        }
    }

    /// The box `ops`' triangles at `at` span, in pixels.
    fn mesh_box(ops: &[Op], at: usize) -> Bounds {
        match spanned(mesh_at(ops, at).vertices.iter().map(|v| [f64::from(v.at[0]), f64::from(v.at[1])])) {
            Some(b) => b,
            None => panic!("triangles at {at} have corners"),
        }
    }

    fn props(pairs: &[(&str, Value)]) -> BTreeMap<String, Value> {
        pairs.iter().map(|(k, v)| (k.to_string(), v.clone())).collect()
    }

    fn applied(result: Result<(), String>) {
        if let Err(why) = result {
            panic!("the canvas takes the change: {why}");
        }
    }

    #[test]
    fn an_item_drawn_from_what_it_was_drawn_from_takes_what_it_drew_and_a_changed_one_is_drawn_anew() {
        let rect = |name: &str, x: f64| item(&[("kind", s("Rect")), ("name", s(name)), ("x", n(x)), ("y", n(0.0)), ("width", n(10.0)), ("height", n(10.0)), ("fill", s("Red"))]);
        let mut canvases = HashMap::from([(1, canvas(&[rect("A", 0.0), rect("B", 20.0)]))]);
        let now = Instant::now();
        let draw = |canvases: &mut HashMap<NodeId, Canvas>| match canvases.get_mut(&1) {
            Some(c) => frame_of(c, [100.0, 100.0], 1.0, BLACK, None, now),
            None => panic!("canvas 1"),
        };
        let first = draw(&mut canvases);
        applied(set_item(&mut canvases, 1, "b", &props(&[("x", n(50.0))]), now));
        let second = draw(&mut canvases);
        assert!(Arc::ptr_eq(mesh_at(&first.ops, 0), mesh_at(&second.ops, 0)), "A, unchanged, takes the triangles it drew before");
        assert!(!Arc::ptr_eq(mesh_at(&first.ops, 1), mesh_at(&second.ops, 1)), "B, changed, is drawn anew");
        assert_eq!(mesh_box(&second.ops, 1).x, 50.0);
        assert_eq!(second.item_at([55.0, 5.0]), Some("B"), "its events follow it");
        let doubled = match canvases.get_mut(&1) {
            Some(c) => frame_of(c, [200.0, 200.0], 2.0, BLACK, None, now),
            None => panic!("canvas 1"),
        };
        assert_eq!(mesh_box(&doubled.ops, 0).width, 20.0, "a new scale draws every item anew");
        applied(remove_items(&mut canvases, 1, &[String::from("A")]));
        assert_eq!(draw(&mut canvases).ops.len(), 1);
    }

    #[test]
    fn an_animation_moves_its_property_holds_its_end_and_a_direct_set_ends_it() {
        let start = Instant::now();
        let after = |seconds: f64| start + std::time::Duration::from_secs_f64(seconds);
        let slide = item(&[("property", s("X")), ("to", n(100.0)), ("duration", n(1.0)), ("easing", s("Linear"))]);
        let pulse = item(&[("property", s("Opacity")), ("to", n(0.5)), ("duration", n(1.0)), ("repeat", s("Forever")), ("reverse", Value::Bool(true))]);
        let rect = item(&[("kind", s("Rect")), ("name", s("Box")), ("x", n(0.0)), ("y", n(0.0)), ("width", n(10.0)), ("height", n(10.0)), ("fill", s("Red")), ("animate", Value::List(vec![slide, pulse]))]);
        let mut canvases = HashMap::from([(1, canvas_at(&[rect], start))]);
        let x_at = |canvases: &mut HashMap<NodeId, Canvas>, seconds: f64| match canvases.get_mut(&1) {
            Some(c) => mesh_box(&frame_of(c, [200.0, 50.0], 1.0, BLACK, None, after(seconds)).ops, 0).x,
            None => panic!("canvas 1"),
        };
        assert!((x_at(&mut canvases, 0.5) - 50.0).abs() < 1e-3, "halfway along a linear slide");
        assert!((x_at(&mut canvases, 5.0) - 100.0).abs() < 1e-3, "past its end it stays at To");
        let moving = |canvases: &HashMap<NodeId, Canvas>, seconds: f64| canvases.get(&1).is_some_and(|c| c.motions.moving(after(seconds)));
        assert!(moving(&canvases, 5.0), "the pulse repeats forever");
        let back = item(&[("property", s("X")), ("to", n(0.0)), ("duration", n(1.0)), ("easing", s("Linear"))]);
        applied(set_item(&mut canvases, 1, "Box", &props(&[("animate", back)]), after(5.0)));
        assert!((x_at(&mut canvases, 5.5) - 50.0).abs() < 1e-3, "a new Animate starts from the value drawn, at To");
        let id = canvases.get(&1).and_then(|c| c.board.find("Box"));
        let kept = canvases.get(&1).zip(id).and_then(|(c, id)| c.motions.of(id)).map(|m| m.keys().cloned().collect::<Vec<_>>());
        assert_eq!(kept, Some(vec![String::from("opacity"), String::from("x")]), "the pulse goes on beside the new slide");
        applied(set_item(&mut canvases, 1, "Box", &props(&[("x", n(10.0))]), after(5.5)));
        assert!((x_at(&mut canvases, 5.6) - 10.0).abs() < 1e-3, "setting X ends its animation");
        applied(set_item(&mut canvases, 1, "Box", &props(&[("animate", Value::Null)]), after(6.0)));
        assert!(!moving(&canvases, 6.0), "an Animate of null ends them all");
    }

    /// A canvas named Board, 200 by 100 points at a window's top left, a
    /// red box over all of it named Box, and the node's `flags` set, drawn
    /// one egui frame at a time: what the frames raised, and how long egui
    /// was told to wait before the next.
    struct Drawn {
        ctx: egui::Context,
        node: Node,
        inputs: Inputs,
        raised: Vec<FromHelper>,
    }

    impl Drawn {
        fn new(flags: &[&str]) -> Drawn {
            let rect = item(&[("kind", s("Rect")), ("name", s("Box")), ("x", n(0.0)), ("y", n(0.0)), ("width", n(200.0)), ("height", n(100.0)), ("fill", s("Red"))]);
            let mut node = Node::new(2, proto::Kind::Canvas).named("Board").prop("width", n(200.0)).prop("height", n(100.0)).prop("items", Value::List(vec![rect]));
            for flag in flags {
                node = node.prop(*flag, Value::Bool(true));
            }
            let mut inputs = Inputs { surface_format: Some(wgpu::TextureFormat::Bgra8Unorm), ..Inputs::default() };
            applied(seed(&node, &mut inputs));
            Drawn { ctx: egui::Context::default(), node, inputs, raised: Vec::new() }
        }

        /// A frame at `time` seconds given `events`, returning how long egui
        /// was told to wait before the next.
        fn frame(&mut self, time: f64, events: Vec<egui::Event>) -> std::time::Duration {
            let input = egui::RawInput { events, time: Some(time), screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(400.0, 300.0))), ..egui::RawInput::default() };
            let (node, inputs, raised) = (&self.node, &mut self.inputs, &mut self.raised);
            let output = self.ctx.run_ui(input, |ui| {
                if let Err(why) = draw_canvas(ui, node, inputs, 1, raised) {
                    panic!("the canvas draws: {why}");
                }
            });
            let wait = output.viewport_output.get(&egui::ViewportId::ROOT).map_or(std::time::Duration::MAX, |v| v.repaint_delay);
            output.drop_without_applying_deltas();
            wait
        }

        /// The kinds of the events raised since the last call.
        fn kinds(&mut self) -> Vec<EventKind> {
            self.raised.drain(..).filter_map(|e| if let FromHelper::Event { kind, .. } = e { Some(kind) } else { None }).collect()
        }
    }

    fn press(down: bool) -> egui::Event {
        egui::Event::PointerButton { pos: egui::pos2(50.0, 50.0), button: egui::PointerButton::Primary, pressed: down, modifiers: egui::Modifiers::NONE }
    }

    fn key(pressed: egui::Key) -> egui::Event {
        egui::Event::Key { key: pressed, physical_key: Some(pressed), pressed: true, repeat: false, modifiers: egui::Modifiers::NONE }
    }

    #[test]
    fn an_item_is_clicked_by_the_pointer_and_not_by_a_key_that_clicks_a_focused_widget() {
        let mut drawn = Drawn::new(&["click", "keys"]);
        drawn.frame(0.0, vec![egui::Event::PointerMoved(egui::pos2(50.0, 50.0))]);
        drawn.frame(0.1, vec![press(true)]);
        drawn.frame(0.15, vec![press(false)]);
        assert_eq!(drawn.kinds(), vec![EventKind::Click], "a click on Box");
        drawn.frame(1.0, vec![key(egui::Key::Space)]);
        drawn.frame(1.1, vec![key(egui::Key::Enter)]);
        assert_eq!(drawn.kinds(), Vec::new(), "Space and Enter on the focused canvas click no item");
    }

    #[test]
    fn a_press_held_still_wakes_egui_when_it_becomes_a_drag() {
        let mut drawn = Drawn::new(&["dragStart"]);
        drawn.frame(0.0, vec![egui::Event::PointerMoved(egui::pos2(50.0, 50.0))]);
        drawn.frame(0.1, vec![press(true)]);
        // egui draws once more at once after a frame given input.
        drawn.frame(0.4, Vec::new());
        let wait = drawn.frame(0.45, Vec::new()).as_secs_f64();
        assert!((wait - 0.451).abs() < 0.005, "egui is woken just after the press has lasted 0.8 seconds: {wait}");
        assert_eq!(drawn.kinds(), Vec::new());
        drawn.frame(0.901, Vec::new());
        assert_eq!(drawn.kinds(), vec![EventKind::DragStart], "then the still press is a drag of Box");
    }

    /// An egui context whose fonts its first pass has made.
    fn context() -> egui::Context {
        let ctx = egui::Context::default();
        ctx.run_ui(egui::RawInput::default(), |_| {}).drop_without_applying_deltas();
        ctx
    }

    #[test]
    fn a_text_fills_and_strokes_the_outlines_of_the_glyphs_egui_lays_out() {
        let ctx = context();
        let text = |extra: &[(&str, Value)]| {
            let mut pairs = vec![("kind", s("Text")), ("name", s("Label")), ("x", n(10.0)), ("y", n(20.0)), ("text", s("Hi")), ("size", n(20.0))];
            pairs.extend(extra.iter().cloned());
            canvas(&[item(&pairs)])
        };
        let framed = |extra: &[(&str, Value)]| frame_of(&mut text(extra), [200.0, 100.0], 1.0, BLACK, Some(&ctx), Instant::now());
        let plain = framed(&[]);
        assert_eq!(plain.problems, Vec::<String>::new());
        assert_eq!(plain.ops.len(), 1, "a Text giving neither Fill nor Stroke is filled in the theme's text color");
        assert!(first_mesh(&plain.ops).iter().all(|v| v.color == [0.0, 0.0, 0.0, 1.0]));
        let laid = ctx.fonts_mut(|f| f.layout_no_wrap(String::from("Hi"), egui::FontId::proportional(20.0), egui::Color32::WHITE)).size();
        let Some(glyphs) = spanned(first_mesh(&plain.ops).iter().map(|v| [f64::from(v.at[0]), f64::from(v.at[1])])) else { panic!("the glyphs' triangles") };
        let inside = glyphs.x >= 10.0 && glyphs.y >= 20.0 && glyphs.x + glyphs.width <= 10.0 + f64::from(laid.x) + 1.0 && glyphs.y + glyphs.height <= 20.0 + f64::from(laid.y);
        assert!(inside && glyphs.width > 5.0 && glyphs.height > 5.0, "the outlines lie inside the box egui lays the text out in: {glyphs:?} in {laid:?}");
        let outlined = framed(&[("stroke", s("Red")), ("strokewidth", n(2.0))]);
        assert_eq!(outlined.ops.len(), 1, "a Stroke alone outlines the glyphs and fills none");
        assert!(first_mesh(&outlined.ops).iter().all(|v| v.color == [1.0, 0.0, 0.0, 1.0]));
        let both = framed(&[("fill", s("Blue")), ("stroke", s("Red"))]);
        assert_eq!(both.ops.len(), 2, "a fill, then the stroke over it");
        assert!(first_mesh(&both.ops).iter().all(|v| v.color == [0.0, 0.0, 1.0, 1.0]), "the fill first");
        assert_eq!(plain.item_at([12.0, 30.0]), Some("Label"), "the text takes events across the box it is laid out in");
        assert_eq!(plain.item_at([5.0, 5.0]), None);
    }

    #[test]
    fn a_text_holding_line_breaks_takes_a_row_for_each_line() {
        let ctx = context();
        let row = ctx.fonts_mut(|f| f.layout_no_wrap(String::from("Hi"), egui::FontId::proportional(20.0), egui::Color32::WHITE)).size().y;
        let mut lines = canvas(&[item(&[("kind", s("Text")), ("name", s("Label")), ("x", n(10.0)), ("y", n(20.0)), ("text", s("Hi\nHi\n")), ("size", n(20.0))])]);
        let rows = frame_of(&mut lines, [200.0, 200.0], 1.0, BLACK, Some(&ctx), Instant::now());
        assert_eq!(rows.problems, Vec::<String>::new());
        let Some(glyphs) = spanned(first_mesh(&rows.ops).iter().map(|v| [f64::from(v.at[0]), f64::from(v.at[1])])) else { panic!("the glyphs' triangles") };
        let bottom = glyphs.y + glyphs.height;
        assert!(bottom > 20.0 + f64::from(row) && bottom <= 20.0 + 2.0 * f64::from(row), "the second line draws in the second row: {glyphs:?}, rows {row} tall");
        assert_eq!(rows.item_at([12.0, 20.0 + 2.5 * row]), Some("Label"), "the empty third line takes a row of the box too");
        assert_eq!(rows.item_at([12.0, 20.0 + 3.0 * row + 2.0]), None, "and the box ends with it");
    }

    /// `frame` prepared as egui-wgpu prepares a paint callback and the
    /// canvas's resolved texture read back, `size` pixels across and down.
    fn drawn(frame: Frame, size: [u32; 2]) -> Vec<u8> {
        let gpu = Gpu::open(&GpuOptions::default()).expect("a device opens");
        let mut resources = egui_wgpu::CallbackResources::default();
        resources.insert(egui_wgpu::EguiTextures(Default::default()));
        let paint = Painting {
            frame: Arc::new(frame),
            size,
            window: [0.0, 0.0, 1.0, 1.0],
            format: wgpu::TextureFormat::Rgba8Unorm,
            targets: Arc::default(),
            recorded: Arc::default(),
            problems: Arc::default(),
        };
        let screen = egui_wgpu::ScreenDescriptor { size_in_pixels: size, pixels_per_point: 1.0 };
        let mut encoder = gpu.device().create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("drawn") });
        let mut commands = paint.prepare(gpu.device(), gpu.queue(), &screen, &mut encoder, &mut resources);
        let targets = paint.targets.load_full().expect("the canvas's textures");
        let read = gpu.create_texture(size[0], size[1], FORMAT, Some(wgpu::TextureUsages::COPY_DST | wgpu::TextureUsages::COPY_SRC), "drawn").expect("a texture to read");
        let resolved = targets.main.resolved.texture().clone();
        encoder.copy_texture_to_texture(resolved.as_image_copy(), read.as_image_copy(), wgpu::Extent3d { width: size[0], height: size[1], depth_or_array_layers: 1 });
        commands.push(encoder.finish());
        gpu.queue().submit(commands);
        assert_eq!(paint.problems.pop(), None, "the frame drew");
        assert!(paint.recorded.load().as_ref().is_some_and(|held| Arc::ptr_eq(held, &paint.frame)), "the textures contain the frame recorded, so the next paint of it records nothing");
        gpu.read_texture(&read).expect("the pixels")
    }

    fn pixel(pixels: &[u8], width: u32, [x, y]: [u32; 2]) -> [u8; 4] {
        let at = ((y * width + x) * 4) as usize;
        [pixels[at], pixels[at + 1], pixels[at + 2], pixels[at + 3]]
    }

    #[test]
    fn a_canvas_draws_its_items_multisampled_and_a_translucent_group_as_one_layer() {
        let items = [
            item(&[("kind", s("Rect")), ("x", n(0.0)), ("y", n(0.0)), ("width", n(16.0)), ("height", n(16.0)), ("fill", s("#ff0000"))]),
            item(&[
                ("kind", s("Group")),
                ("opacity", n(0.5)),
                ("children", Value::List(vec![
                    item(&[("kind", s("Rect")), ("x", n(16.0)), ("y", n(0.0)), ("width", n(16.0)), ("height", n(16.0)), ("fill", s("#0000ff"))]),
                    item(&[("kind", s("Rect")), ("x", n(16.0)), ("y", n(0.0)), ("width", n(16.0)), ("height", n(16.0)), ("fill", s("#0000ff"))]),
                ])),
            ]),
            item(&[("kind", s("Ellipse")), ("x", n(0.0)), ("y", n(16.0)), ("width", n(16.0)), ("height", n(16.0)), ("fill", s("#00ff00"))]),
        ];
        let pixels = drawn(made(&items, [32.0, 32.0], 1.0), [32, 32]);
        assert_eq!(pixel(&pixels, 32, [8, 8]), [255, 0, 0, 255], "an opaque fill");
        let layered = pixel(&pixels, 32, [24, 8]);
        let half = |v: u8| v.abs_diff(128) <= 1;
        assert!(layered[0] == 0 && layered[1] == 0 && half(layered[2]) && half(layered[3]), "two blue rects in a half-opaque group show as one layer at half, not three quarters: {layered:?}");
        assert_eq!(pixel(&pixels, 32, [8, 24]), [0, 255, 0, 255], "inside the circle");
        assert_eq!(pixel(&pixels, 32, [0, 16]), [0, 0, 0, 0], "outside it, at the box's corner");
        let edge = pixel(&pixels, 32, [2, 18]);
        assert!(edge[3] > 0 && edge[3] < 255, "a pixel the circle's edge crosses at 45 degrees is partly covered, from its samples: {edge:?}");
    }
}
