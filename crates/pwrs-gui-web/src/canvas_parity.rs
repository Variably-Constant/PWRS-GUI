//! The page's canvas script, run in boa, against the protocol crate's own
//! arithmetic. Every easing curve, view move, blend, animation rule, item
//! value, outline, closed part, box, text row and stack of a text's rows the
//! page makes must agree with egui's within [`CLOSE`]. Its pointer's clicks
//! and drags are checked against egui's own context, frame by frame, and its
//! hit test against the hit test egui runs over the same widgets.

use boa_engine::{Context, Source};
use pwrs_gui_proto::Value;
use pwrs_gui_proto::canvas::{Board, Easing, Motions, Rgba, Shape, Target};
use pwrs_gui_proto::svg_path;
use pwrs_gui_proto::typeface::FaceMetrics;
use pwrs_gui_proto::view::{self, ViewGesture, ViewKey, ViewMove, ViewSpec};
use serde_json::Value as Json;
use std::time::{Duration, Instant};

const CANVAS_JS: &str = include_str!("runtime/canvas.js");

/// How near the script's results and the protocol crate's must be.
const CLOSE: f64 = 1e-9;

/// What `script` returns in a context that has run the page's canvas
/// script, given the numbers a view moves by as `k`, the result written as
/// JSON by the script and read back.
fn answer(script: &str) -> Json {
    let k = format!(
        "var k = {{ zoomStep: {:?}, zoomInStep: {:?}, scrollStep: {:?}, keyStep: {:?}, glideDeceleration: {:?}, glideLinearity: {:?} }};",
        view::ZOOM_STEP,
        view::ZOOM_IN_STEP,
        view::SCROLL_STEP,
        view::KEY_STEP,
        view::GLIDE_DECELERATION,
        view::GLIDE_LINEARITY
    );
    let whole = format!("{CANVAS_JS}\n{k}\nJSON.stringify((function () {{ {script} }})());");
    let mut context = Context::default();
    let value = match context.eval(Source::from_bytes(&whole)) {
        Ok(v) => v,
        Err(e) => panic!("the page's canvas script runs: {e}"),
    };
    let text = match value.as_string() {
        Some(s) => s.to_std_string_escaped(),
        None => panic!("the script returned no JSON: {value:?}"),
    };
    match serde_json::from_str(&text) {
        Ok(json) => json,
        Err(e) => panic!("the script's result reads as JSON: {e}: {text}"),
    }
}

/// `spec` as the page stores a view.
fn script_view(spec: &ViewSpec) -> String {
    let center = match spec.center {
        Some([x, y]) => format!("[{x:?}, {y:?}]"),
        None => String::from("null"),
    };
    let max = if spec.max_zoom.is_infinite() { String::from("Infinity") } else { format!("{:?}", spec.max_zoom) };
    format!("{{ center: {center}, zoom: {:?}, minZoom: {:?}, maxZoom: {max}, bindings: {{}} }}", spec.zoom, spec.min_zoom)
}

fn number(json: &Json) -> f64 {
    match json.as_f64() {
        Some(n) => n,
        None => panic!("{json} is no number"),
    }
}

/// Whether the page's view `page`, as [center x, center y, zoom], is
/// `rust` on a canvas `size` points across and down.
fn same_view(page: &Json, rust: &ViewSpec, size: [f64; 2]) -> bool {
    let c = rust.center_in(size);
    [c[0], c[1], rust.zoom].iter().enumerate().all(|(i, want)| (number(&page[i]) - want).abs() < CLOSE)
}

#[test]
fn every_curve_the_page_holds_is_the_protocol_crates_at_every_point() {
    let page = answer("return pwrsCanvas.easings.map(function (name) { var at = []; for (var i = 0; i <= 200; i++) at.push(pwrsCanvas.ease(name, i / 200)); return [name, at]; });");
    let curves = match page.as_array() {
        Some(c) => c,
        None => panic!("the page returned no list of curves: {page}"),
    };
    let names: Vec<&str> = curves.iter().filter_map(|c| c[0].as_str()).collect();
    let rust: Vec<&str> = Easing::ALL.iter().map(|e| e.name()).collect();
    assert_eq!(names, rust, "the page has every curve, by the same names");
    for (curve, easing) in curves.iter().zip(Easing::ALL) {
        for i in 0..=200u32 {
            let t = f64::from(i) / 200.0;
            let (on_page, in_rust) = (number(&curve[1][i as usize]), easing.at(t));
            assert!((on_page - in_rust).abs() < CLOSE, "{} at {t}: the page gives {on_page}, the protocol crate {in_rust}", easing.name());
        }
    }
}

#[test]
fn every_view_move_the_page_makes_is_the_protocol_crates() {
    let size = [200.0, 100.0];
    let start = ViewSpec { center: Some([12.5, -7.0]), zoom: 1.5, ..ViewSpec::default() };
    let mut panned = start.clone();
    panned.pan(20.0, -10.0, size);
    let mut zoomed = start.clone();
    zoomed.zoom_about(2.0, [150.0, 60.0], size);
    let mut wheeled = start.clone();
    wheeled.wheel(ViewMove::Zoom, [0.0, 250.0], [40.0, 30.0], size);
    let mut across = start.clone();
    across.wheel(ViewMove::ScrollAcross, [0.0, 100.0], [40.0, 30.0], size);
    let mut dragged = start.clone();
    dragged.dragged(ViewMove::Zoom, 0.0, 130.0, [80.0, 20.0], size);
    let out = start.stepped(ViewMove::ZoomOut, [150.0, 50.0], None, None, size);
    let up = start.stepped(ViewMove::Pan, [100.0, 50.0], Some(ViewKey::ArrowUp), None, size);
    let fitted = start.stepped(ViewMove::Fit, [100.0, 50.0], None, Some([0.0, 0.0, 400.0, 100.0]), size);
    let actual = start.stepped(ViewMove::ActualSize, [30.0, 70.0], None, None, size);
    let boxed = start.boxed([10.0, 10.0], [90.0, 70.0], size);
    let between = start.toward(&zoomed, 0.37, size);
    let v = script_view(&start);
    let script = format!(
        "var s = [200, 100], v = function () {{ return {v}; }}, z = function (x) {{ return x ? [pwrsCanvas.centerIn(x, s)[0], pwrsCanvas.centerIn(x, s)[1], x.zoom] : null; }};
         var a = v(); pwrsCanvas.pan(a, 20, -10, s);
         var b = v(); pwrsCanvas.zoomAbout(b, 2, [150, 60], s);
         var c = v(); pwrsCanvas.wheel(c, 'Zoom', [0, 250], [40, 30], s, k);
         var d = v(); pwrsCanvas.wheel(d, 'ScrollAcross', [0, 100], [40, 30], s, k);
         var e = v(); pwrsCanvas.dragged(e, 'Zoom', 0, 130, [80, 20], s, k);
         return [z(a), z(b), z(c), z(d), z(e),
           z(pwrsCanvas.stepped(v(), 'ZoomOut', [150, 50], null, null, s, k)),
           z(pwrsCanvas.stepped(v(), 'Pan', [100, 50], [0, -1], null, s, k)),
           z(pwrsCanvas.stepped(v(), 'Fit', [100, 50], null, [0, 0, 400, 100], s, k)),
           z(pwrsCanvas.stepped(v(), 'ActualSize', [30, 70], null, null, s, k)),
           z(pwrsCanvas.boxed(v(), [10, 10], [90, 70], s)),
           z(pwrsCanvas.toward(v(), b, 0.37, s))];"
    );
    let page = answer(&script);
    let rust = [Some(panned), Some(zoomed), Some(wheeled), Some(across), Some(dragged), out, up, fitted, actual, boxed, Some(between)];
    for (i, want) in rust.iter().enumerate() {
        match want {
            Some(spec) => assert!(same_view(&page[i], spec, size), "move {i}: the page gives {}, the protocol crate {spec:?}", page[i]),
            None => assert!(page[i].is_null(), "move {i} makes no view in the protocol crate, and the page gives {}", page[i]),
        }
    }
}

#[test]
fn a_glide_a_release_and_a_written_gesture_are_the_protocol_crates() {
    let page = answer(
        "var g = pwrsCanvas.glide([1234, -567], k), samples = [[0, [0, 0]], [0.96875, [75, 10]], [1, [100, 20]]];
         return [g.by, g.seconds, pwrsCanvas.glide([50, 0], k), pwrsCanvas.releaseVelocity(samples, 1, 0.05), pwrsCanvas.releaseVelocity(samples, 1.25, 0.05),
           pwrsCanvas.written('Left', { shift: true, space: true }), pwrsCanvas.written('Plus', { shift: true, ctrl: false }), pwrsCanvas.written('1', { shift: true }), pwrsCanvas.written('Wheel', { ctrl: true })];",
    );
    let (by, seconds) = match view::glide([1234.0, -567.0]) {
        Some(g) => g,
        None => panic!("the protocol crate glides at 1234 points a second"),
    };
    assert!((number(&page[0][0]) - by[0]).abs() < CLOSE && (number(&page[0][1]) - by[1]).abs() < CLOSE && (number(&page[1]) - seconds).abs() < CLOSE, "the glide: {page}");
    assert!(page[2].is_null() && view::glide([50.0, 0.0]).is_none(), "too slow to glide in both");
    let samples = [(0.0, [0.0, 0.0]), (0.96875, [75.0, 10.0]), (1.0, [100.0, 20.0])];
    let velocity = match view::release_velocity(&samples, 1.0) {
        Some(v) => v,
        None => panic!("the protocol crate measures the release"),
    };
    assert!((number(&page[3][0]) - velocity[0]).abs() < CLOSE && (number(&page[3][1]) - velocity[1]).abs() < CLOSE, "the release: {}", page[3]);
    assert!(page[4].is_null() && view::release_velocity(&samples, 1.25).is_none(), "a rest before the release in both");
    for (i, written) in ["Shift+Space+Left", "Plus", "Shift+1", "Ctrl+Wheel"].iter().enumerate() {
        assert_eq!(page[5 + i].as_str(), Some(*written), "the page writes the gesture");
        assert!(ViewGesture::named(written).is_ok_and(|g| g.written() == *written), "the protocol crate writes {written} alike");
    }
}

fn map(pairs: &[(&str, Value)]) -> Value {
    Value::Map(pairs.iter().map(|(k, v)| (k.to_string(), v.clone())).collect())
}

fn text(s: &str) -> Value {
    Value::Str(s.to_string())
}

/// A board of `items`, which must read.
fn board(items: Vec<Value>) -> Board {
    match Board::of_items(&Value::List(items)) {
        Ok(b) => b,
        Err(why) => panic!("the items read: {why}"),
    }
}

/// `board`'s items as a page takes them, with no animations under way.
fn scene(board: &Board) -> String {
    let pictures = super::Pictures::default();
    Json::Array(board.top().iter().map(|id| super::item_json(board, *id, &pictures, true, None)).collect()).to_string()
}

/// Whether two JSON values are alike, numbers compared by their value,
/// which JSON written by the page and by serde writes differently for a
/// whole number.
fn alike(a: &Json, b: &Json) -> bool {
    match (a, b) {
        (Json::Number(x), Json::Number(y)) => x.as_f64().zip(y.as_f64()).is_some_and(|(x, y)| x == y),
        (Json::Array(x), Json::Array(y)) => x.len() == y.len() && x.iter().zip(y).all(|(x, y)| alike(x, y)),
        (Json::Object(x), Json::Object(y)) => x.len() == y.len() && x.iter().all(|(k, v)| y.get(k).is_some_and(|w| alike(v, w))),
        _ => a == b,
    }
}

/// Whether the page's `page` is `rust`, a number, a pair or a color.
fn same_target(page: &Json, rust: Option<Target>) -> bool {
    match rust {
        None => page.is_null(),
        Some(Target::Number(v)) => page.as_f64().is_some_and(|x| (x - v).abs() < CLOSE),
        Some(Target::Pair([a, b])) => page[0].as_f64().is_some_and(|x| (x - a).abs() < CLOSE) && page[1].as_f64().is_some_and(|y| (y - b).abs() < CLOSE),
        Some(Target::Color(c)) => page.as_str() == Some(c.hex().as_str()),
    }
}

#[test]
fn a_blend_and_an_animations_passes_are_the_protocol_crates() {
    let pairs = [
        (Target::Number(2.0), Target::Number(-6.0)),
        (Target::Pair([1.0, 2.0]), Target::Pair([5.0, -2.0])),
        (Target::Color(Rgba([255, 0, 0, 255])), Target::Color(Rgba([0, 0, 255, 64]))),
        (Target::Color(Rgba([0, 0, 0, 0])), Target::Color(Rgba([20, 200, 90, 255]))),
    ];
    let written = |t: Target| super::target_json(t).to_string();
    let shares = [0.0, 0.3, 0.5, 1.0, 1.2, -0.1];
    let calls: Vec<String> = pairs.iter().flat_map(|(a, b)| shares.iter().map(move |s| format!("pwrsCanvas.mix({}, {}, {s:?})", written(*a), written(*b)))).collect();
    let page = answer(&format!("return [{}];", calls.join(", ")));
    let mut at = 0;
    for (a, b) in pairs {
        for s in shares {
            assert!(same_target(&page[at], Some(a.toward(b, s))), "{a:?} to {b:?} at {s}: the page gives {}", page[at]);
            at += 1;
        }
    }
    let animate = map(&[
        ("kind", text("Rect")),
        ("x", Value::Float(0.0)),
        ("y", Value::Float(0.0)),
        ("width", Value::Float(1.0)),
        ("height", Value::Float(1.0)),
        (
            "animate",
            Value::List(vec![
                map(&[("property", text("X")), ("to", Value::Float(1.0)), ("duration", Value::Float(2.0)), ("easing", text("EaseInOutCubic")), ("repeat", Value::Int(3)), ("reverse", Value::Bool(true))]),
                map(&[("property", text("Y")), ("to", Value::Float(1.0)), ("duration", Value::Float(0.7)), ("easing", text("EaseOutBounce")), ("repeat", text("Forever"))]),
            ]),
        ),
    ]);
    let held = board(vec![animate]);
    let Some(entry) = held.entry(held.top()[0]) else { panic!("the item") };
    let times = [0.0, 0.35, 1.0, 2.9, 4.0, 11.9, 12.0, 30.0];
    for animation in &entry.item.animations {
        let page = answer(&format!("var a = {}; return {times:?}.map(function (t) {{ return pwrsCanvas.share(a, t); }});", super::animation_json(animation)));
        for (i, t) in times.iter().enumerate() {
            let rust = animation.share(*t);
            assert!((number(&page[i]) - rust).abs() < CLOSE, "{} at {t}: the page gives {}, the protocol crate {rust}", animation.property, page[i]);
        }
    }
}

#[test]
fn an_items_values_and_its_animations_rules_are_the_protocol_crates() {
    let slide = |to: f64| map(&[("property", text("X")), ("to", Value::Float(to)), ("duration", Value::Float(1.0)), ("easing", text("Linear"))]);
    let fade = map(&[("property", text("Fill")), ("to", text("#00ff0080")), ("duration", Value::Float(2.0)), ("easing", text("EaseIn"))]);
    let child = map(&[("kind", text("Rect")), ("name", text("Box")), ("x", Value::Float(4.0)), ("y", Value::Float(5.0)), ("width", Value::Float(6.0)), ("height", Value::Float(7.0)), ("animate", Value::List(vec![slide(40.0), fade]))]);
    let line = map(&[("kind", text("Line")), ("name", text("Rule")), ("x1", Value::Float(0.0)), ("y1", Value::Float(0.0)), ("x2", Value::Float(9.0)), ("y2", Value::Float(1.0)), ("strokewidth", Value::Float(3.0))]);
    let group = map(&[("kind", text("Group")), ("name", text("G")), ("fill", text("Tomato")), ("stroke", text("Navy")), ("children", Value::List(vec![child, line]))]);
    let mut held = board(vec![group]);
    let t0 = Instant::now();
    let at = |s: f64| t0 + Duration::from_secs_f64(s);
    let mut motions = Motions::default();
    motions.started(&held, &held.all(), t0);
    let (Some(boxed), Some(rule)) = (held.find("Box"), held.find("Rule")) else { panic!("named items") };
    let properties = ["x", "y", "width", "fill", "stroke", "strokewidth", "opacity", "scale", "origin", "x1", "radius"];
    let check = |held: &Board, motions: &Motions, page: &Json, times: &[f64], label: &str| {
        let mut i = 0;
        for t in times {
            for id in [boxed, rule] {
                for p in properties {
                    assert!(same_target(&page[i], motions.drawn(held, id, p, at(*t))), "{label}: item {id}'s {p} at {t}: the page gives {}", page[i]);
                    i += 1;
                }
            }
        }
    };
    let original = scene(&held);
    let ask = |times: &[f64], steps: &str| {
        format!(
            "var b = pwrsCanvas.boardOf({original}), m = {{}}; pwrsCanvas.started(m, b, Object.keys(b.items).map(Number), 0); {steps}
             var out = []; {times:?}.forEach(function (t) {{ [{boxed}, {rule}].forEach(function (id) {{ {properties:?}.forEach(function (p) {{ out.push(pwrsCanvas.drawn(m, b, id, p, t)); }}); }}); }}); return out;"
        )
    };
    let early = [0.0, 0.25, 0.5, 1.5, 3.0];
    check(&held, &motions, &answer(&ask(&early, "")), &early, "as they start");
    let set = std::collections::BTreeMap::from([(String::from("x"), Value::Float(9.0))]);
    let changed = match held.set("Box", &set) {
        Ok(id) => id,
        Err(why) => panic!("the change reads: {why}"),
    };
    motions.changed(&held, changed, &set, at(0.5));
    let again = std::collections::BTreeMap::from([(String::from("animate"), slide(100.0))]);
    if let Err(why) = held.set("Box", &again) {
        panic!("the Animate reads: {why}");
    }
    motions.changed(&held, changed, &again, at(0.75));
    let later = [0.75, 1.0, 1.5, 2.5];
    let steps = format!(
        "var e = b.items[{boxed}]; e.shape = Object.assign({{}}, e.shape, {{ x: 9 }}); pwrsCanvas.changed(m, b, {boxed}, ['x'], null, 0.5);
         e.animations = [{}]; pwrsCanvas.changed(m, b, {boxed}, [], 'start', 0.75);",
        super::animation_json(&held.entry(boxed).map(|e| e.item.animations[0].clone()).expect("the new animation"))
    );
    check(&held, &motions, &answer(&ask(&later, &steps)), &later, "a direct set ends X's animation and a new Animate starts from the value drawn");
    let page = answer(&format!(
        "var b = pwrsCanvas.boardOf({}), e = b.items[{boxed}], copy = {{ shape: Object.assign({{}}, e.shape), style: Object.assign({{}}, e.style) }};
         pwrsCanvas.setValue(copy, 'skew', [120, -5]); pwrsCanvas.setValue(copy, 'opacity', 1.4); pwrsCanvas.setValue(copy, 'width', -3); pwrsCanvas.setValue(copy, 'x', 'Red');
         return [copy.style.skew, copy.style.opacity, copy.shape.width, copy.shape.x];",
        scene(&held)
    ));
    let Some(mut item) = held.entry(boxed).map(|e| e.item.clone()) else { panic!("the item") };
    item.set_value("skew", Target::Pair([120.0, -5.0]));
    item.set_value("opacity", Target::Number(1.4));
    item.set_value("width", Target::Number(-3.0));
    item.set_value("x", Target::Color(Rgba([255, 0, 0, 255])));
    for (i, p) in ["skew", "opacity", "width", "x"].iter().enumerate() {
        assert!(same_target(&page[i], item.value_of(p)), "{p} as set: the page gives {}", page[i]);
    }
}

#[test]
fn outlines_arcs_and_exact_boxes_are_the_protocol_crates() {
    let shapes = [
        map(&[("kind", text("Rect")), ("x", Value::Float(1.0)), ("y", Value::Float(2.0)), ("width", Value::Float(30.0)), ("height", Value::Float(20.0))]),
        map(&[("kind", text("RoundedRect")), ("x", Value::Float(1.0)), ("y", Value::Float(2.0)), ("width", Value::Float(30.0)), ("height", Value::Float(20.0)), ("radius", Value::Float(40.0))]),
        map(&[("kind", text("Ellipse")), ("x", Value::Float(-5.0)), ("y", Value::Float(2.0)), ("width", Value::Float(30.0)), ("height", Value::Float(12.0))]),
        map(&[("kind", text("Ellipse")), ("x", Value::Float(0.0)), ("y", Value::Float(0.0)), ("width", Value::Float(10.0)), ("height", Value::Float(0.0))]),
        map(&[("kind", text("Polygon")), ("points", Value::List([0.0, 0.0, 9.0, 1.0, 4.0, 7.0].map(Value::Float).to_vec()))]),
        map(&[("kind", text("Path")), ("data", text("M0 0 C0 -100 100 -100 100 0 Q 150 50 120 90 A 30 20 25 1 0 10 60 a 5 5 0 0 1 -3 4 z M 200 0 L 210 5"))]),
    ];
    let held = board(shapes.to_vec());
    let turns = [svg_path::IDENTITY, [0.0, 1.0, -1.0, 0.0, 3.0, -2.0], [2.0, 0.5, -0.7, 1.5, -10.0, 4.0]];
    for id in held.all() {
        let Some(entry) = held.entry(id) else { panic!("an item") };
        let Some(segments) = entry.item.shape.outline() else { panic!("an outline") };
        let shape = super::shape_json(&entry.item.shape, &super::Pictures::default());
        let page = answer(&format!("var s = {shape}; return [pwrsCanvas.outline(s), {turns:?}.map(function (m) {{ return pwrsCanvas.bounds(pwrsCanvas.outline(s), m); }})];"));
        let rust = super::segments_json(&segments);
        assert!(alike(&page[0], &rust), "the outline of {:?}: the page gives {}, the protocol crate {rust}", entry.item.shape, page[0]);
        for (i, m) in turns.iter().enumerate() {
            let rust = svg_path::bounds(&segments, *m);
            match rust {
                Some(b) => assert!(b.iter().enumerate().all(|(j, v)| (number(&page[1][i][j]) - v).abs() < 1e-9 * (1.0 + v.abs())), "{:?} under {m:?}: the page gives {}, the protocol crate {b:?}", entry.item.shape, page[1][i]),
                None => assert!(page[1][i].is_null(), "{:?} under {m:?} draws nothing in the protocol crate", entry.item.shape),
            }
        }
    }
    let page = answer("return [pwrsCanvas.centerArc([0, 0], [5, 5], 0, false, true, [10, 0]), pwrsCanvas.centerArc([0, 0], [1, 2], 30, true, false, [10, 3]), pwrsCanvas.centerArc([0, 0], [0, 5], 0, false, true, [10, 0])];");
    for (i, (radii, rotation, large, sweep, to)) in [([5.0, 5.0], 0.0, false, true, [10.0, 0.0]), ([1.0, 2.0], 30.0, true, false, [10.0, 3.0]), ([0.0, 5.0], 0.0, false, true, [10.0, 0.0])].into_iter().enumerate() {
        match svg_path::center_arc([0.0, 0.0], radii, rotation, large, sweep, to) {
            Some(arc) => {
                let got = &page[i];
                let close = |key: &str, at: Option<usize>, want: f64| {
                    let v = match at {
                        Some(j) => number(&got[key][j]),
                        None => number(&got[key]),
                    };
                    (v - want).abs() < CLOSE
                };
                assert!(
                    close("center", Some(0), arc.center[0]) && close("center", Some(1), arc.center[1]) && close("radii", Some(0), arc.radii[0]) && close("radii", Some(1), arc.radii[1]) && close("rotation", None, arc.rotation) && close("start", None, arc.start) && close("sweep", None, arc.sweep),
                    "arc {i}: the page gives {got}, the protocol crate {arc:?}"
                );
            }
            None => assert!(page[i].is_null(), "arc {i} is no arc in the protocol crate"),
        }
    }
    assert!(Shape::Ellipse(pwrs_gui_proto::canvas::Bounds { x: 0.0, y: 0.0, width: 2.0, height: 2.0 }).outline().is_some());
}

#[test]
fn a_gradient_s_stops_blend_premultiplied_and_a_text_row_is_egui_s() {
    let colors = [Rgba([255, 0, 0, 255]), Rgba([0, 0, 255, 0]), Rgba([0, 128, 0, 128]), Rgba([0, 128, 0, 128])];
    let hexes: Vec<String> = colors.iter().map(|c| c.hex()).collect();
    let stops = [0.0, 0.4, 0.6, 1.0];
    let page = answer(&format!("return pwrsCanvas.gradientStops({hexes:?}, {stops:?});"));
    let Some(given) = page.as_array() else { panic!("the page returned no stops: {page}") };
    let bytes = |hex: &str| -> [u8; 4] {
        let at = |k: usize| match u8::from_str_radix(&hex[k..k + 2], 16) {
            Ok(b) => b,
            Err(why) => panic!("{hex} is no color: {why}"),
        };
        [at(1), at(3), at(5), at(7)]
    };
    let mut last = f64::NEG_INFINITY;
    for stop in given {
        let (offset, hex) = (number(&stop[0]), stop[1].as_str().unwrap_or("#00000000"));
        assert!(offset >= last, "the stops go forward: {page}");
        last = offset;
        let segment = stops.windows(2).position(|w| offset >= w[0] && offset <= w[1]).unwrap_or(0);
        let t = (offset - stops[segment]) / (stops[segment + 1] - stops[segment]);
        let Target::Color(want) = Target::Color(colors[segment]).toward(Target::Color(colors[segment + 1]), t) else { panic!("a color") };
        let got = bytes(hex);
        let near = |a: u8, b: u8| a.abs_diff(b) <= 1;
        assert!(near(got[3], want.0[3]), "at {offset} the page's alpha is {hex}, the premultiplied blend {}", want.hex());
        if got[3] > 0 {
            assert!(got.iter().zip(want.0).all(|(a, b)| near(*a, b)), "at {offset} the page gives {hex}, the premultiplied blend {}", want.hex());
        }
    }
    let between = |from: f64, to: f64| given.iter().filter(|s| number(&s[0]) > from && number(&s[0]) < to).count();
    assert_eq!((between(0.0, 0.4), between(0.4, 0.6), between(0.6, 1.0)), (15, 15, 0), "fifteen between stops whose alphas differ, none where they agree");
    let face = FaceMetrics { units: 2048.0, ascent: 2210.0, descent: -514.0, gap: 67.0 };
    let sizes = [9.0f32, 13.0, 14.0, 17.5, 23.9375];
    let ppps = [1.0f32, 1.25, 1.5, 2.0];
    let page = answer(&format!(
        "var f = {{ units: 2048, ascent: 2210, descent: -514, gap: 67 }}; var out = []; {sizes:?}.forEach(function (s) {{ {ppps:?}.forEach(function (p) {{ var r = pwrsCanvas.textRow(f, s, p); out.push([r.height, r.baseline]); }}); }}); return out;"
    ));
    let mut at = 0;
    for size in sizes {
        for ppp in ppps {
            let [ascent, descent, gap] = face.at(size);
            let pixel = |v: f32| (v * ppp).round() / ppp;
            let (height, baseline) = (pixel(ascent - descent + gap), pixel(ascent));
            let single = |json: &Json| number(json) as f32;
            assert!(single(&page[at][0]) == height && single(&page[at][1]) == baseline, "{size} points at {ppp}: the page gives {}, egui {height} and {baseline}", page[at]);
            at += 1;
        }
    }
}

#[test]
fn a_text_s_lines_are_rows_stacked_as_egui_stacks_them() {
    let face = FaceMetrics { units: 2048.0, ascent: 2210.0, descent: -514.0, gap: 67.0 };
    for (size, ppp) in [(14.0f32, 1.0f32), (17.5, 1.25), (23.9375, 1.5), (13.0, 2.0)] {
        let [ascent, descent, gap] = face.at(size);
        let pixel = |v: f32| (v * ppp).round() / ppp;
        let (height, baseline) = (pixel(ascent - descent + gap), pixel(ascent));
        for text in ["One", "One\nTwo", "a\n\nb\n", "\n"] {
            let page = answer(&format!(
                "var f = {{ units: 2048, ascent: 2210, descent: -514, gap: 67 }}; var r = pwrsCanvas.textLines({text:?}, pwrsCanvas.textRow(f, {size:?}, {ppp:?}), {ppp:?}); return [r.height, r.lines.map(function (l) {{ return [l.text, l.baseline]; }})];"
            ));
            let Some(rows) = page[1].as_array() else { panic!("the page laid {text:?} out in no rows: {page}") };
            let lines: Vec<&str> = text.split('\n').collect();
            assert_eq!(rows.len(), lines.len(), "{text:?}: a row for each line, an empty one included: {page}");
            let mut top = 0.0f32;
            for (row, line) in rows.iter().zip(&lines) {
                assert_eq!(row[0].as_str(), Some(*line), "{text:?}: {page}");
                assert!(number(&row[1]) as f32 == top + baseline, "{text:?} at {size} points and {ppp}: the page puts {line:?}'s baseline at {}, egui at {}", row[1], top + baseline);
                top = pixel(top + height);
            }
            assert!(number(&page[0]) as f32 == top, "{text:?} at {size} points and {ppp}: the page's rows are {} tall, egui's {top}", page[0]);
        }
    }
}

#[test]
fn the_closed_part_of_an_outline_is_the_protocol_crates() {
    for data in [
        "M0 0 L10 0 L10 10 Z",
        "M0 0 L10 0 L10 10",
        "M0 0 L5 5 M20 0 L30 0 L30 10 Z M40 0 L50 0",
        "M0 0 L10 0 L10 10 Z L0 20 L20 20 Z",
        "M0 0 L10 0 Z Z",
        "M0 0 C0 -100 100 -100 100 0 Q 150 50 120 90 A 30 20 25 1 0 10 60 a 5 5 0 0 1 -3 4 z M 200 0 L 210 5",
    ] {
        let segments = match svg_path::segments(data) {
            Ok(s) => s,
            Err(why) => panic!("{data} reads: {why}"),
        };
        let page = answer(&format!("return pwrsCanvas.closed({});", super::segments_json(&segments)));
        let rust = super::segments_json(&svg_path::closed(&segments));
        assert!(alike(&page, &rust), "the closed part of {data}: the page gives {page}, the protocol crate {rust}");
    }
}

/// One of a frame's events: the pointer moved to a point, a button, 0 Left,
/// 1 Right or 2 Middle, pressed or let go at a point, or Escape pressed.
#[derive(Clone, Copy, Debug)]
enum Input {
    Move([f32; 2]),
    Press(usize, [f32; 2]),
    Release(usize, [f32; 2]),
    Escape,
}

/// Frames of input, each at its time in seconds.
type Frames = Vec<(f64, Vec<Input>)>;

/// What each frame of `frames`, each at its time in seconds, made of a
/// widget 200 by 100 points at the window's top left sensing clicks and
/// drags as `click` and `drag` say, as egui's own context works it out:
/// whether Left, Right and Middle clicked it, whether each double-clicked
/// it, and whether its drag started, went on and stopped.
fn egui_frames(click: bool, drag: bool, frames: &[(f64, Vec<Input>)]) -> Vec<[bool; 9]> {
    let ctx = egui::Context::default();
    let sense = match (click, drag) {
        (true, true) => egui::Sense::click_and_drag(),
        (true, false) => egui::Sense::click(),
        (false, true) => egui::Sense::drag(),
        (false, false) => egui::Sense::hover(),
    };
    let buttons = [egui::PointerButton::Primary, egui::PointerButton::Secondary, egui::PointerButton::Middle];
    frames
        .iter()
        .map(|(time, inputs)| {
            let button = |b: usize, [x, y]: [f32; 2], pressed| egui::Event::PointerButton { pos: egui::pos2(x, y), button: buttons[b], pressed, modifiers: egui::Modifiers::NONE };
            let events = inputs
                .iter()
                .map(|i| match *i {
                    Input::Move([x, y]) => egui::Event::PointerMoved(egui::pos2(x, y)),
                    Input::Press(b, at) => button(b, at, true),
                    Input::Release(b, at) => button(b, at, false),
                    Input::Escape => egui::Event::Key { key: egui::Key::Escape, physical_key: Some(egui::Key::Escape), pressed: true, repeat: false, modifiers: egui::Modifiers::NONE },
                })
                .collect();
            let input = egui::RawInput { events, time: Some(*time), screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(400.0, 300.0))), ..egui::RawInput::default() };
            let mut seen = [false; 9];
            let output = ctx.run_ui(input, |ui| {
                let r = ui.allocate_rect(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(200.0, 100.0)), sense);
                for (i, b) in buttons.iter().enumerate() {
                    seen[i] = r.clicked_by(*b);
                    seen[3 + i] = r.double_clicked_by(*b);
                }
                seen[6] = r.drag_started();
                seen[7] = r.dragged();
                seen[8] = r.drag_stopped();
            });
            output.drop_without_applying_deltas();
            seen
        })
        .collect()
}

/// What each frame of `frames` made of the same widget in the page's
/// script: its pointer fed each frame's events in order, then its
/// interaction run over them with the page's hit test of the widget at the
/// pointer's last place.
fn page_frames(click: bool, drag: bool, frames: &[(f64, Vec<Input>)]) -> Vec<[bool; 9]> {
    let at = |[x, y]: [f32; 2]| format!("[{x:?}, {y:?}]");
    let written: Vec<String> = frames
        .iter()
        .map(|(time, inputs)| {
            let events: Vec<String> = inputs
                .iter()
                .map(|i| match *i {
                    Input::Move(p) => format!("['move', {}]", at(p)),
                    Input::Press(b, p) => format!("['press', {b}, {}]", at(p)),
                    Input::Release(b, p) => format!("['release', {b}, {}]", at(p)),
                    Input::Escape => String::from("['escape']"),
                })
                .collect();
            format!("[{time:?}, [{}]]", events.join(", "))
        })
        .collect();
    let page = answer(&format!(
        "var p = pwrsCanvas.pointer(), w = {{ potentialClick: false, potentialDrag: false, dragging: false }}, latest = null;
         return [{}].map(function (f) {{
           var events = [], escape = false;
           f[1].forEach(function (i) {{
             if (i[0] === 'move') {{ pwrsCanvas.moved(p, i[1]); latest = i[1]; }}
             else if (i[0] === 'press') {{ pwrsCanvas.pressed(p, i[1], i[2], f[0]); latest = i[2]; events.push({{ press: i[1] }}); }}
             else if (i[0] === 'release') {{ var click = pwrsCanvas.released(p, i[1], i[2], f[0]); latest = i[2]; events.push({{ release: i[1], click: click }}); }}
             else escape = true;
           }});
           var hits = latest === null ? {{ contains: [], click: null, drag: null }} : pwrsCanvas.hitTest([{{ id: 1, layer: 0, rect: [0, 0, 200, 100], click: {click}, drag: {drag}, enabled: true }}], latest, pwrsCanvas.interactRadius);
           var r = pwrsCanvas.interact(w, p, {{ events: events, escape: escape, inside: hits.contains.indexOf(1) >= 0, hitClick: hits.click === 1, hitDrag: hits.drag === 1, click: {click}, drag: {drag} }}, f[0]);
           var by = function (b, count) {{ return !!pwrsCanvas.clickBy(r, events, b, count); }};
           return [by(0), by(1), by(2), by(0, 2), by(1, 2), by(2, 2), r.started, w.dragging, r.stopped];
         }});",
        written.join(", ")
    ));
    let Some(rows) = page.as_array() else { panic!("the page returned no frames: {page}") };
    rows.iter()
        .map(|row| {
            let mut seen = [false; 9];
            for (i, s) in seen.iter_mut().enumerate() {
                *s = row[i].as_bool().unwrap_or_else(|| panic!("{row} has no true or false at {i}"));
            }
            seen
        })
        .collect()
}

#[test]
fn the_pages_clicks_and_drags_are_egui_s_frame_by_frame() {
    use Input::{Escape, Move, Press, Release};
    let (inside, near, outside) = ([50.0, 50.0], [60.0, 50.0], [250.0, 50.0]);
    let click = |t: f64, b: usize, at: [f32; 2]| [(t, vec![Press(b, at)]), (t + 0.05, vec![Release(b, at)])];
    let start = (0.0, vec![Move(inside)]);
    let clicks = |times: &[f64]| -> Frames { std::iter::once(start.clone()).chain(times.iter().flat_map(|t| click(*t, 0, inside))).collect() };
    let cases: Vec<(&str, bool, bool, Frames)> = vec![
        ("a click", true, true, clicks(&[0.1])),
        ("a double-click", true, true, clicks(&[0.1, 0.3])),
        ("a triple-click, then a fourth", true, true, clicks(&[0.1, 0.3, 0.5, 0.7])),
        ("two clicks too far apart in time", true, true, clicks(&[0.1, 0.5])),
        (
            "two clicks too far apart in place",
            true,
            true,
            [start.clone()].into_iter().chain(click(0.1, 0, inside)).chain([(0.2, vec![Move([58.0, 50.0])])]).chain(click(0.25, 0, [58.0, 50.0])).collect(),
        ),
        (
            "a drag past a click's distance",
            true,
            true,
            vec![start.clone(), (0.1, vec![Press(0, inside)]), (0.12, vec![Move([53.0, 50.0])]), (0.14, vec![Move(near)]), (0.16, vec![Move([80.0, 50.0])]), (0.2, vec![Release(0, [80.0, 50.0])])],
        ),
        ("a press lasting past a click's time", true, true, vec![start.clone(), (0.1, vec![Press(0, inside)]), (0.5, Vec::new()), (0.95, Vec::new()), (1.0, vec![Release(0, inside)])]),
        (
            "a press that leaves the widget within a click's distance",
            true,
            true,
            vec![(0.0, vec![Move([199.5, 50.0])]), (0.1, vec![Press(0, [199.5, 50.0])]), (0.12, vec![Move([205.5, 50.0])]), (0.15, vec![Release(0, [205.5, 50.0])])],
        ),
        (
            "a second button pressed and let go while the first is down",
            true,
            true,
            vec![start.clone(), (0.1, vec![Press(0, inside)]), (0.2, vec![Press(1, inside)]), (0.25, vec![Release(1, inside)]), (0.3, vec![Release(0, inside)])],
        ),
        (
            "Escape during a drag",
            true,
            true,
            vec![start.clone(), (0.1, vec![Press(0, inside)]), (0.12, vec![Move([80.0, 50.0])]), (0.14, vec![Escape]), (0.16, vec![Move([90.0, 50.0])]), (0.2, vec![Release(0, [90.0, 50.0])])],
        ),
        ("a drag on a widget sensing clicks alone", true, false, vec![start.clone(), (0.1, vec![Press(0, inside)]), (0.12, vec![Move([80.0, 50.0])]), (0.2, vec![Release(0, [80.0, 50.0])])]),
        ("a click on a widget sensing neither", false, false, clicks(&[0.1])),
        ("a press on a widget sensing drags alone", false, true, vec![start.clone(), (0.1, vec![Press(0, inside)]), (0.12, vec![Move(near)]), (0.2, vec![Release(0, near)])]),
        ("a right double-click", true, true, [start.clone()].into_iter().chain(click(0.1, 1, inside)).chain(click(0.3, 1, inside)).collect()),
        ("a middle click", true, true, [start.clone()].into_iter().chain(click(0.1, 2, inside)).collect()),
        (
            "a press outside the widget that moves onto it",
            true,
            true,
            vec![(0.0, vec![Move(outside)]), (0.1, vec![Press(0, outside)]), (0.15, vec![Move(inside)]), (0.2, vec![Release(0, inside)])],
        ),
        (
            "a click, then a long press",
            true,
            true,
            [start.clone()].into_iter().chain(click(0.1, 0, inside)).chain([(0.3, vec![Press(0, inside)]), (1.2, Vec::new()), (1.3, vec![Release(0, inside)])]).collect(),
        ),
    ];
    // Presses near the widget's edge, which egui's hit test gives the widget
    // within 5 points of it and not past them.
    let near = |at: [f32; 2]| -> Frames { [(0.0, vec![Move(at)])].into_iter().chain(click(0.1, 0, at)).collect() };
    let edges: Vec<(&str, bool, bool, Frames)> = vec![
        ("a click 3 points right of the widget", true, true, near([203.0, 50.0])),
        ("a click 6 points right of the widget", true, true, near([206.0, 50.0])),
        ("a click 3 points above the widget's middle", true, true, near([100.0, -3.0])),
        ("a click 3 points below and 3 right of the widget's corner", true, true, near([203.0, 103.0])),
        ("a click 4 points below and 5 right of the widget's corner", true, true, near([205.0, 104.0])),
        ("a right click 3 points right of the widget", true, true, [(0.0, vec![Move([203.0, 50.0])])].into_iter().chain(click(0.1, 1, [203.0, 50.0])).collect()),
        ("a click 3 points right of a widget sensing clicks alone", true, false, near([203.0, 50.0])),
        ("a click 6 points right of a widget sensing clicks alone", true, false, near([206.0, 50.0])),
        (
            "a press 3 points below the widget dragged along it",
            true,
            true,
            vec![(0.0, vec![Move([100.0, 103.0])]), (0.1, vec![Press(0, [100.0, 103.0])]), (0.12, vec![Move([100.0, 101.0])]), (0.14, vec![Move([120.0, 101.0])]), (0.2, vec![Release(0, [120.0, 101.0])])],
        ),
        (
            "a press 3 points right of the widget that moves past 5 points of it within a click's distance",
            true,
            true,
            vec![(0.0, vec![Move([203.0, 50.0])]), (0.1, vec![Press(0, [203.0, 50.0])]), (0.12, vec![Move([207.0, 50.0])]), (0.15, vec![Release(0, [207.0, 50.0])])],
        ),
        (
            "a press 3 points right of a widget sensing drags alone",
            false,
            true,
            vec![(0.0, vec![Move([203.0, 50.0])]), (0.1, vec![Press(0, [203.0, 50.0])]), (0.12, vec![Move([204.0, 50.0])]), (0.2, vec![Release(0, [204.0, 50.0])])],
        ),
        (
            "a press 6 points right of a widget sensing drags alone",
            false,
            true,
            vec![(0.0, vec![Move([206.0, 50.0])]), (0.1, vec![Press(0, [206.0, 50.0])]), (0.12, vec![Move([190.0, 50.0])]), (0.2, vec![Release(0, [190.0, 50.0])])],
        ),
    ];
    for (name, click, drag, frames) in cases.into_iter().chain(edges) {
        let (by_egui, by_page) = (egui_frames(click, drag, &frames), page_frames(click, drag, &frames));
        assert_eq!(by_page, by_egui, "{name}: each frame's clicks by button, double-clicks by button, and drag started, going on and stopped; the page's first, egui's second, for {frames:?}");
    }
}

/// A widget a hit-test case places: its layer, 0 the window's own and 1
/// and 2 areas of egui's middle and foreground orders over it, its box
/// [left, top, right, bottom] in points, what it senses, and whether it is
/// enabled.
#[derive(Clone, Copy, Debug)]
struct Placed {
    layer: usize,
    rect: [f32; 4],
    sense: egui::Sense,
    enabled: bool,
}

/// Places `w` in `ui`, inside a disabled scope of its own where it is not
/// enabled.
fn place(ui: &mut egui::Ui, w: &Placed) {
    let rect = egui::Rect::from_min_max(egui::pos2(w.rect[0], w.rect[1]), egui::pos2(w.rect[2], w.rect[3]));
    if w.enabled {
        ui.allocate_rect(rect, w.sense);
    } else {
        ui.add_enabled_ui(false, |ui| ui.allocate_rect(rect, w.sense));
    }
}

/// Runs one frame of `ctx` given `events`, placing `widgets`: the window's
/// own in its root, each other layer's in an area of its order at the top
/// left of its widgets. Returns what `read` read of the context during the
/// frame, the hit test egui ran as it began.
fn hit_frame<R>(ctx: &egui::Context, widgets: &[Placed], events: Vec<egui::Event>, read: impl Fn(&egui::Context) -> R) -> R {
    let input = egui::RawInput { events, screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(400.0, 300.0))), ..egui::RawInput::default() };
    let mut seen = None;
    let output = ctx.run_ui(input, |ui| {
        seen = Some(read(ui.ctx()));
        for w in widgets.iter().filter(|w| w.layer == 0) {
            place(ui, w);
        }
        for (layer, order) in [(1, egui::Order::Middle), (2, egui::Order::Foreground)] {
            let mine: Vec<&Placed> = widgets.iter().filter(|w| w.layer == layer).collect();
            let Some(left) = mine.iter().map(|w| w.rect[0]).reduce(f32::min) else { continue };
            let top = mine.iter().map(|w| w.rect[1]).fold(f32::INFINITY, f32::min);
            egui::Area::new(egui::Id::unique(("hit layer", layer))).order(order).fixed_pos(egui::pos2(left, top)).show(ui.ctx(), |ui| {
                for w in &mine {
                    place(ui, w);
                }
            });
        }
    });
    output.drop_without_applying_deltas();
    match seen {
        Some(r) => r,
        None => panic!("the frame ran no ui"),
    }
}

/// What egui's own hit test found among `widgets` at `at`, in a frame of
/// its own once the widgets have settled. The first part is every widget it
/// tested, the Ui and Area widgets egui registers among them. They run back
/// to front by layer as the page's hitTest takes them, each with its place
/// in that list as its id. The second is egui's close, contains, click and
/// drag as places in that list.
fn egui_hits(ctx: &egui::Context, widgets: &[Placed], at: [f32; 2]) -> (Json, Json) {
    let layers = [egui::LayerId::background(), egui::LayerId::new(egui::Order::Middle, egui::Id::unique(("hit layer", 1))), egui::LayerId::new(egui::Order::Foreground, egui::Id::unique(("hit layer", 2)))];
    hit_frame(ctx, widgets, vec![egui::Event::PointerMoved(egui::pos2(at[0], at[1]))], |ctx| {
        ctx.viewport(|vp| {
            let mut ids = Vec::new();
            let mut tested = Vec::new();
            for (layer, layer_id) in layers.iter().enumerate() {
                for w in vp.prev_pass.widgets.get_layer(*layer_id) {
                    let r = w.interact_rect;
                    tested.push(serde_json::json!({ "id": ids.len(), "layer": layer, "rect": [f64::from(r.min.x), f64::from(r.min.y), f64::from(r.max.x), f64::from(r.max.y)], "click": w.sense.senses_click(), "drag": w.sense.senses_drag(), "enabled": w.enabled }));
                    ids.push(w.id);
                }
            }
            let place = |id: egui::Id| match ids.iter().position(|i| *i == id) {
                Some(at) => Json::from(at),
                None => panic!("egui's hit test found widget {id:?}, which it was not given"),
            };
            let hits = &vp.hits;
            let found = serde_json::json!({
                "close": hits.close.iter().map(|w| place(w.id)).collect::<Vec<_>>(),
                "contains": hits.contains_pointer.iter().map(|w| place(w.id)).collect::<Vec<_>>(),
                "click": hits.click.map_or(Json::Null, |w| place(w.id)),
                "drag": hits.drag.map_or(Json::Null, |w| place(w.id)),
            });
            (Json::Array(tested), found)
        })
    })
}

/// A case of the hit test's parity: what it shows, its widgets, and the
/// points the pointer is put at.
type HitCase = (&'static str, Vec<Placed>, Vec<[f32; 2]>);

#[test]
fn the_pages_hit_test_is_egui_s() {
    let w = |layer: usize, rect: [f32; 4], sense: egui::Sense| Placed { layer, rect, sense, enabled: true };
    let (click, drag, both, hover) = (egui::Sense::click(), egui::Sense::drag(), egui::Sense::click_and_drag(), egui::Sense::hover());
    let cases: Vec<HitCase> = vec![
        (
            "a button alone, at and 3 and 6 points past each edge and corner",
            vec![w(0, [100.0, 100.0, 180.0, 124.0], click)],
            vec![[140.0, 112.0], [97.0, 112.0], [94.0, 112.0], [183.0, 112.0], [186.0, 112.0], [140.0, 97.0], [140.0, 94.0], [140.0, 127.0], [140.0, 130.0], [97.0, 97.0], [96.0, 96.0], [183.0, 127.0]],
        ),
        (
            "two buttons 8 points apart, the pointer between them",
            vec![w(0, [10.0, 10.0, 50.0, 30.0], click), w(0, [58.0, 10.0, 98.0, 30.0], click)],
            vec![[53.0, 20.0], [54.0, 20.0], [55.0, 20.0], [56.0, 20.0], [54.0, 4.0]],
        ),
        (
            "buttons on a drag background, as egui's own test has them",
            vec![w(0, [0.0, 0.0, 100.0, 100.0], drag), w(0, [10.0, 10.0, 20.0, 20.0], click), w(0, [100.0, 10.0, 110.0, 20.0], both)],
            vec![[15.0, 15.0], [5.0, 5.0], [105.0, 15.0], [105.0, 5.0], [50.0, 50.0], [97.0, 15.0], [113.0, 15.0]],
        ),
        (
            "a thin drag handle between labels, as egui's own test has them",
            vec![w(0, [0.0, 0.0, 100.0, 100.0], drag), w(0, [0.0, 0.0, 40.0, 100.0], both), w(0, [30.0, 0.0, 100.0, 100.0], drag), w(0, [60.0, 0.0, 110.0, 100.0], both)],
            vec![[25.0, 50.0], [35.0, 50.0], [50.0, 50.0], [65.0, 50.0], [113.0, 50.0]],
        ),
        (
            "a label beside a button, and a splitter beside a label",
            vec![w(0, [10.0, 10.0, 60.0, 30.0], both), w(0, [63.0, 10.0, 120.0, 30.0], click), w(0, [200.0, 0.0, 206.0, 200.0], drag), w(0, [209.0, 10.0, 260.0, 30.0], both)],
            vec![[61.0, 20.0], [62.0, 20.0], [58.0, 20.0], [65.0, 34.0], [207.0, 20.0], [208.0, 20.0], [198.0, 50.0], [210.0, 50.0]],
        ),
        (
            "a disabled button beside an enabled one, and a drag value",
            vec![Placed { layer: 0, rect: [10.0, 10.0, 50.0, 30.0], sense: click, enabled: false }, w(0, [54.0, 10.0, 94.0, 30.0], click), w(0, [10.0, 40.0, 50.0, 60.0], both), w(0, [10.0, 70.0, 300.0, 90.0], hover)],
            vec![[51.0, 20.0], [30.0, 20.0], [30.0, 36.0], [30.0, 64.0], [30.0, 80.0], [53.0, 50.0]],
        ),
        (
            "a popup over the edge of a button, and a button the popup hides",
            vec![w(0, [10.0, 10.0, 110.0, 40.0], click), w(0, [70.0, 50.0, 90.0, 60.0], click), w(2, [60.0, 30.0, 160.0, 70.0], click)],
            vec![[70.0, 25.0], [70.0, 29.0], [70.0, 32.0], [80.0, 55.0], [165.0, 50.0], [100.0, 20.0], [55.0, 35.0]],
        ),
        (
            "a menu and a popup over the window, each over the one before",
            vec![w(0, [0.0, 0.0, 200.0, 200.0], both), w(1, [20.0, 20.0, 120.0, 60.0], click), w(2, [100.0, 40.0, 180.0, 90.0], click)],
            vec![[50.0, 40.0], [110.0, 50.0], [117.0, 63.0], [97.0, 50.0], [185.0, 60.0], [10.0, 10.0]],
        ),
    ];
    for (name, widgets, points) in cases {
        let ctx = egui::Context::default();
        for _ in 0..3 {
            hit_frame(&ctx, &widgets, Vec::new(), |_| ());
        }
        for at in points {
            let (tested, by_egui) = egui_hits(&ctx, &widgets, at);
            let by_page = answer(&format!("var h = pwrsCanvas.hitTest({tested}, [{:?}, {:?}], pwrsCanvas.interactRadius); return {{ close: h.close, contains: h.contains, click: h.click, drag: h.drag }};", f64::from(at[0]), f64::from(at[1])));
            assert!(alike(&by_page, &by_egui), "{name}, the pointer at {at:?}: the page finds {by_page}, egui {by_egui}, among {tested}");
        }
    }
    let radius = answer("return pwrsCanvas.interactRadius;");
    assert_eq!(number(&radius), f64::from(egui::Style::default().interaction.interact_radius), "the page reaches as far as egui's interact radius");
}
