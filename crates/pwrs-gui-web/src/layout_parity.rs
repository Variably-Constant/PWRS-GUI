//! The page's layout script, run in boa, against the protocol crate's own
//! rules: which widgets take room where they stand and which fill down, how
//! a Column, Row or Grid places its widgets across, a button's role, the
//! order a button bar shows its buttons in on each system, the keys a list,
//! table or tree uses itself, a chart's X and a date axis's labels as text,
//! and when a password field offers its peek button. Each node is given to
//! the script as the helper gives it to a page.

use crate::{Pictures, Showing, node_json};
use boa_engine::{Context, Source};
use pwrs_gui_proto as proto;
use proto::button_bar::{self, Role};
use proto::rows::{self, RowKey, RowMove, RowSelect, RowState, Selection};
use proto::{Align, Kind, Node, Value, chart};
use serde_json::{Value as Json, json};
use std::collections::{BTreeMap, BTreeSet};
use std::time::Instant;

const LAYOUT_JS: &str = include_str!("runtime/layout.js");

/// What `script` returns in a context that has run the page's layout
/// script, written as JSON by the script and read back.
fn answer(script: &str) -> Json {
    let whole = format!("{LAYOUT_JS}\nJSON.stringify((function () {{ {script} }})());");
    let mut context = Context::default();
    let value = match context.eval(Source::from_bytes(&whole)) {
        Ok(v) => v,
        Err(e) => panic!("the page's layout script runs: {e}"),
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

/// `node` as the helper gives it to a page.
fn page_node(node: &Node) -> String {
    let pictures = Pictures::default();
    let canvases = BTreeMap::new();
    node_json(node, &Showing { pictures: &pictures, canvases: &canvases, now: Instant::now() }).to_string()
}

/// Every node of `tree`, the tree first.
fn every(tree: &Node) -> Vec<&Node> {
    let mut out = vec![tree];
    for c in &tree.children {
        out.extend(every(c));
    }
    out
}

fn node(kind: Kind) -> Node {
    Node::new(1, kind)
}

fn text() -> Node {
    node(Kind::Text).prop("text", Value::Str(String::from("words")))
}

/// Trees with every kind of widget the rules tell apart, sized and not.
fn trees() -> Vec<Node> {
    let tall = |kind: Kind| node(kind).prop("height", Value::Float(80.0));
    let fill = node(Kind::Spacer).prop("fill", Value::Bool(true));
    vec![
        node(Kind::Column).child(text()).child(node(Kind::ListBox)).child(node(Kind::ButtonBar).child(node(Kind::Button))),
        node(Kind::Column).child(text()).child(fill.clone()).child(node(Kind::Spacer)),
        node(Kind::Row).child(text()).child(fill.clone()),
        node(Kind::Row).child(text()).child(node(Kind::TreeView)),
        node(Kind::Grid).child(text()).child(node(Kind::Table)),
        node(Kind::Grid).child(text()).child(fill),
        node(Kind::Group).child(node(Kind::LogView)),
        node(Kind::Group).child(tall(Kind::LogView)),
        node(Kind::Tabs).child(node(Kind::Tab).child(node(Kind::Scroll))),
        node(Kind::Tabs).child(node(Kind::Tab).child(tall(Kind::Scroll))),
        node(Kind::Split).prop("orientation", Value::Str(String::from("vertical"))).child(text()).child(text()),
        node(Kind::Split).child(text()).child(node(Kind::GpuSurface)),
        node(Kind::Split).child(text()).child(tall(Kind::GpuSurface)),
        node(Kind::Column).child(node(Kind::Chart)).child(tall(Kind::Chart)).child(node(Kind::LogView).prop("height", Value::Null)),
        node(Kind::Column).child(node(Kind::MenuBar)).child(node(Kind::StatusBar)).child(node(Kind::ContextMenu)).child(node(Kind::Button)),
    ]
}

#[test]
fn the_page_fills_down_and_flows_as_the_protocol_crate_says() {
    let mut checked = 0;
    for tree in trees() {
        for n in every(&tree) {
            let page = answer(&format!("var n = {}; return [pwrsLayout.fillsDown(n), pwrsLayout.inFlow(n)];", page_node(n)));
            assert_eq!(page, json!([proto::fills_down(n), proto::in_flow(n)]), "{n:?}");
            checked += 1;
        }
    }
    assert!(checked > 40, "every node of every tree was checked: {checked}");
}

#[test]
fn the_page_reads_align_as_the_protocol_crate_does() {
    let named = |a: Align| match a {
        Align::Start => "start",
        Align::Center => "center",
        Align::End => "end",
        Align::Stretch => "stretch",
    };
    let given = [None, Some(Value::Null), Some(Value::Str(String::from("start"))), Some(Value::Str(String::from("center"))), Some(Value::Str(String::from("end"))), Some(Value::Str(String::from("stretch"))), Some(Value::Str(String::from("middle"))), Some(Value::Int(3))];
    for align in given {
        let mut n = node(Kind::Row);
        if let Some(a) = &align {
            n = n.prop("align", a.clone());
        }
        let page = answer(&format!("var n = {}; try {{ return pwrsLayout.align(n); }} catch (e) {{ return {{ error: e.message }}; }}", page_node(&n)));
        match (Align::of(&n), align) {
            (Ok(a), _) => assert_eq!(page, json!(named(a)), "{n:?}"),
            (Err(why), Some(Value::Str(_))) => assert_eq!(page, json!({ "error": why }), "a word is refused in the same words"),
            (Err(_), _) => assert!(page.get("error").is_some(), "{n:?} is refused: {page}"),
        }
    }
}

#[test]
fn the_page_reads_a_buttons_role_and_orders_a_bar_as_the_protocol_crate_does() {
    let flags = [None, Some(true), Some(false)];
    for default in flags {
        for cancel in flags {
            let mut b = node(Kind::Button).prop("text", Value::Str(String::from("OK")));
            if let Some(d) = default {
                b = b.prop("default", Value::Bool(d));
            }
            if let Some(c) = cancel {
                b = b.prop("cancel", Value::Bool(c));
            }
            let page = answer(&format!("var n = {}; try {{ return pwrsLayout.role(n); }} catch (e) {{ return {{ error: e.message }}; }}", page_node(&b)));
            let want = match Role::of(&b) {
                Ok(Role::Default) => json!("default"),
                Ok(Role::Cancel) => json!("cancel"),
                Ok(Role::Other) => json!("other"),
                Err(why) => json!({ "error": why }),
            };
            assert_eq!(page, want, "{b:?}");
        }
    }
    let written = |r: &Role| match r {
        Role::Default => "default",
        Role::Cancel => "cancel",
        Role::Other => "other",
    };
    use Role::{Cancel, Default, Other};
    for roles in [vec![Default, Cancel], vec![Cancel, Default], vec![Default, Other, Cancel], vec![Other, Cancel, Other, Default], vec![Other], vec![]] {
        for windows in [true, false] {
            let names: Vec<&str> = roles.iter().map(written).collect();
            let page = answer(&format!("return pwrsLayout.order({}, {windows});", json!(names)));
            assert_eq!(page, json!(button_bar::order(&roles, windows)), "{names:?} with the Windows order {windows}");
        }
    }
}

/// Each row key with the name the page's script gives it.
const ROW_KEYS: [(RowKey, &str); 9] = [
    (RowKey::Up, "Up"),
    (RowKey::Down, "Down"),
    (RowKey::Home, "Home"),
    (RowKey::End, "End"),
    (RowKey::PageUp, "PageUp"),
    (RowKey::PageDown, "PageDown"),
    (RowKey::Space, "Space"),
    (RowKey::Enter, "Enter"),
    (RowKey::All, "All"),
];

/// Each way of selecting rows with the name the page's script gives it.
const SELECTIONS: [(Selection, &str); 4] = [(Selection::None, "none"), (Selection::Single, "single"), (Selection::Multiple, "multiple"), (Selection::Checks, "checks")];

/// A selection as the page's script writes it.
fn select_json(s: RowSelect) -> Json {
    match s {
        RowSelect::Same => json!({ "kind": "same" }),
        RowSelect::Only(at) => json!({ "kind": "only", "at": at }),
        RowSelect::Range(at, to) => json!({ "kind": "range", "at": at, "to": to }),
        RowSelect::Toggle(at) => json!({ "kind": "toggle", "at": at }),
        RowSelect::All => json!({ "kind": "all" }),
    }
}

/// What a key does as the page's script writes it.
fn move_json(m: Option<RowMove>) -> Json {
    match m {
        None => Json::Null,
        Some(m) => json!({ "current": m.current, "anchor": m.anchor, "select": select_json(m.select), "activate": m.activate }),
    }
}

#[test]
fn the_page_moves_and_selects_rows_by_key_as_the_protocol_crate_does() {
    let mut cases = Vec::new();
    let mut want = Vec::new();
    for (key, key_name) in ROW_KEYS {
        for shift in [false, true] {
            for ctrl in [false, true] {
                for (selection, named) in SELECTIONS {
                    for count in [0, 1, 10] {
                        for current in [None, Some(0), Some(3), Some(9), Some(40)] {
                            for current_selected in [false, true] {
                                for anchor in [None, Some(2), Some(8)] {
                                    for page in [0, 4] {
                                        cases.push(json!({ "key": key_name, "shift": shift, "ctrl": ctrl, "state": { "count": count, "selection": named, "current": current, "currentSelected": current_selected, "anchor": anchor, "page": page } }));
                                        want.push(move_json(rows::row_key(key, shift, ctrl, RowState { count, selection, current, current_selected, anchor, page })));
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
    let script = format!("return {}.map(function (c) {{ return pwrsLayout.rowKey(c.key, c.shift, c.ctrl, c.state); }});", Json::Array(cases.clone()));
    let page = answer(&script);
    let got = page.as_array().expect("the page returns a list");
    assert_eq!(got.len(), want.len(), "the page returns every case");
    for ((case, got), want) in cases.iter().zip(got).zip(&want) {
        assert_eq!(got, want, "{case}");
    }
}

#[test]
fn the_page_names_row_keys_as_the_protocol_crate_does() {
    let mut keys: Vec<&str> = proto::Shortcut::KEYS.to_vec();
    keys.extend(["A", "B", "Z", "7", "F5"]);
    let mut cases = Vec::new();
    let mut want = Vec::new();
    for key in keys {
        for ctrl in [false, true] {
            for shift in [false, true] {
                for alt in [false, true] {
                    cases.push(json!([key, ctrl, shift, alt]));
                    let named = RowKey::named(key, ctrl, shift, alt).and_then(|k| ROW_KEYS.iter().find(|(r, _)| *r == k).map(|(_, n)| *n));
                    want.push(json!(named));
                }
            }
        }
    }
    let page = answer(&format!("return {}.map(function (c) {{ return pwrsLayout.rowKeyNamed(c[0], c[1], c[2], c[3]); }});", Json::Array(cases)));
    assert_eq!(page, Json::Array(want));
}

#[test]
fn the_page_applies_a_selection_and_sizes_a_page_as_the_protocol_crate_does() {
    let shown = [10_u32, 20, 30, 40];
    let selects = [RowSelect::Same, RowSelect::Only(0), RowSelect::Only(3), RowSelect::Only(9), RowSelect::Range(3, 1), RowSelect::Range(2, 9), RowSelect::Toggle(1), RowSelect::Toggle(2), RowSelect::Toggle(9), RowSelect::All];
    for start in [vec![], vec![20], vec![10, 30], vec![99]] {
        for select in selects {
            let mut selected: BTreeSet<u32> = start.iter().copied().collect();
            let changed = select.apply(&mut selected, &shown);
            let script = format!("var s = new Set({}); var changed = pwrsLayout.applyRows({}, s, {}); return [Array.from(s).sort(function (a, b) {{ return a - b; }}), changed];", json!(start), select_json(select), json!(shown));
            assert_eq!(answer(&script), json!([selected.into_iter().collect::<Vec<_>>(), changed]), "{select:?} on {start:?}");
        }
    }
    for (shown, row, written) in [(200.0, 20.0, "200, 20"), (209.0, 20.0, "209, 20"), (30.0, 20.0, "30, 20"), (f32::INFINITY, 20.0, "Infinity, 20"), (200.0, 0.0, "200, 0"), (-5.0, 20.0, "-5, 20"), (f32::NAN, 20.0, "NaN, 20"), (200.0, f32::NAN, "200, NaN")] {
        assert_eq!(answer(&format!("return pwrsLayout.pageRows({written});")), json!(rows::page_rows(shown, row)), "{written}");
    }
}

#[test]
fn the_page_copies_rows_as_the_protocol_crate_does() {
    let cases: [&[&[&str]]; 4] = [&[], &[&["web01", "10"]], &[&["web01", "10"], &["web 02", ""], &["tab\there", "line\nbreak"]], &[&[]]];
    for rows_given in cases {
        for newline in ["\r\n", "\n"] {
            let script = format!("return pwrsLayout.copied({}, {});", json!(rows_given), json!(newline));
            assert_eq!(answer(&script), json!(rows::copied(rows_given, newline)), "{rows_given:?} with {newline:?}");
        }
    }
}

#[test]
fn the_page_writes_a_charts_x_and_a_date_axiss_labels_as_the_protocol_crate_does() {
    // Seconds from the Unix epoch, whole and with a fraction, at midnight
    // and not, under every kind of step a date axis labels.
    let xs = [0.0, 1_791_417_600.0, 1_791_417_630.0, 1_791_469_860.0, 1_791_469_860.25, 1_791_469_860.125, 1_792_488_900.0];
    let steps = [0.001, 0.01, 1.0, 15.0, 60.0, 300.0, 3_600.0, 86_400.0, 604_800.0];
    for x in xs {
        assert_eq!(answer(&format!("return pwrsLayout.xText({x}, true);")), json!(chart::x_text(x, true)), "{x}");
        for step in steps {
            let written = answer(&format!("return [pwrsLayout.dateTick({x}, {step}), pwrsLayout.dateSample({step})];"));
            assert_eq!(written, json!([chart::date_tick(x, step), chart::date_sample(step)]), "{x} at {step}");
        }
    }
}

/// One step of a password field's life: text a script sets, or the
/// field's focus and text as a frame finds them.
#[derive(Clone, Copy, Debug)]
enum PeekStep {
    Set(&'static str),
    Update(bool, &'static str),
}

#[test]
fn the_page_offers_a_password_fields_peek_button_as_the_protocol_crate_does() {
    let texts = ["", "a", "ab"];
    let mut steps: Vec<PeekStep> = texts.into_iter().map(PeekStep::Set).collect();
    steps.extend(texts.into_iter().flat_map(|t| [PeekStep::Update(true, t), PeekStep::Update(false, t)]));
    let mut runs: Vec<Vec<PeekStep>> = vec![Vec::new()];
    for _ in 0..4 {
        runs = runs.iter().flat_map(|run| steps.iter().map(move |s| run.iter().copied().chain([*s]).collect())).collect();
    }
    let mut cases = Vec::new();
    let mut want = Vec::new();
    for start in ["", "a"] {
        for run in &runs {
            let mut peek = proto::peek::Peek::new(start);
            let offered: Vec<bool> = run
                .iter()
                .map(|s| match *s {
                    PeekStep::Set(t) => {
                        peek.set(t);
                        peek.offered()
                    }
                    PeekStep::Update(focused, t) => peek.update(focused, t),
                })
                .collect();
            let written: Vec<Json> = run
                .iter()
                .map(|s| match *s {
                    PeekStep::Set(t) => json!({ "set": t }),
                    PeekStep::Update(focused, t) => json!({ "focused": focused, "text": t }),
                })
                .collect();
            cases.push(json!({ "start": start, "steps": written }));
            want.push(json!(offered));
        }
    }
    let script = format!(
        "return {}.map(function (c) {{ var p = pwrsLayout.peek(c.start); return c.steps.map(function (s) {{ if (s.set !== undefined) {{ p.set(s.set); return p.offered(); }} return p.update(s.focused, s.text); }}); }});",
        Json::Array(cases.clone())
    );
    let page = answer(&script);
    let got = page.as_array().expect("the page returns a list");
    assert_eq!(got.len(), want.len(), "the page returns every case");
    for ((case, got), want) in cases.iter().zip(got).zip(&want) {
        assert_eq!(got, want, "{case}");
    }
}
