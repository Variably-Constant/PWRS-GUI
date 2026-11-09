//! A widget's SubEtha ring: one that another process writes and a
//! ProgressBar, LogView, Chart or Canvas reads in the renderer, without
//! passing through PowerShell. The module names it in the node's `ring`
//! property; [`Sources`] reads each such ring on a thread of its own, woken
//! by the ring's waker, and passes each item to the renderer, which
//! [`item_ops`] turns into the patch it makes. A canvas's items are read on
//! that thread into the changes they make, the files of their pictures with
//! them ([`canvas_changes`]), and [`canvas_ops`] checks those against the
//! canvas's items on the renderer's. A widget's shared memory, which the
//! node's `shared` property names, is read the same way on a thread of its
//! own ([`crate::shared`]).

use crate::canvas::{Board, Item, ItemId, ItemKind};
use crate::renderer::WakeLoop;
use crate::shared::{Reader, SharedSource};
use crate::{date, Kind, Node, NodeId, PatchOp, Value, WindowId};
use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::time::Duration;
use subetha_cxc::adaptive_ring::AdaptiveRing;
use subetha_cxc::blocking_adaptive_ring::{BlockingAdaptiveRing, MAX_WAITERS_DEFAULT};
use subetha_cxc::blocking_broadcast_ring::BlockingBroadcastRing;
use subetha_cxc::blocking_locale_ring::BlockingLocaleRing;
use subetha_cxc::blocking_spsc_ring::BlockingError;
use subetha_cxc::blocking_wait::BlockingWaitError;
use subetha_cxc::cross_process_notifier::{Notifier, NotifierSet};
use subetha_cxc::locale_adaptive_ring::{Locale, LocaleAdaptiveRing};
use subetha_cxc::shared_broadcast_ring::{SharedBroadcastRing, BROADCAST_PAYLOAD_BYTES};
use subetha_cxc::shared_ring::RingError;
use subetha_cxc::spsc_ring::SPSC_PAYLOAD_BYTES;

/// The rings a widget reads.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum RingKind {
    /// `SubEtha.Ring`, SubEtha's adaptive ring, read a frame at a time.
    Ring,
    /// `SubEtha.LocaleRing`, the adaptive ring that moves between locales,
    /// read a slot at a time.
    LocaleRing,
    /// `SubEtha.BroadcastRing`, read a slot at a time, every reader getting
    /// every item.
    BroadcastRing,
}

impl RingKind {
    /// The kind's name, as the `ring` property names it.
    pub fn name(self) -> &'static str {
        match self {
            RingKind::Ring => "Ring",
            RingKind::LocaleRing => "LocaleRing",
            RingKind::BroadcastRing => "BroadcastRing",
        }
    }

    /// The kind named `name`.
    pub fn named(name: &str) -> Option<RingKind> {
        [RingKind::Ring, RingKind::LocaleRing, RingKind::BroadcastRing].into_iter().find(|k| k.name() == name)
    }
}

/// A widget's ring as the module names it in the node's `ring` property:
/// its kind, the path it was made at, and the sizes it was made with, which
/// opening it again must match, each zero or more. A broadcast ring has one
/// producer and counts no consumers.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RingSource {
    pub kind: RingKind,
    pub path: String,
    pub capacity: i64,
    pub producers: i64,
    pub consumers: i64,
}

impl RingSource {
    /// The node's `ring` property for this ring.
    pub fn value(&self) -> Value {
        Value::Map(BTreeMap::from([
            (String::from("kind"), Value::Str(self.kind.name().to_string())),
            (String::from("path"), Value::Str(self.path.clone())),
            (String::from("capacity"), Value::Int(self.capacity)),
            (String::from("producers"), Value::Int(self.producers)),
            (String::from("consumers"), Value::Int(self.consumers)),
        ]))
    }

    /// The ring `node` reads, when its `ring` property names one, or why the
    /// property names none rightly.
    pub fn of(node: &Node) -> Option<Result<RingSource, String>> {
        match node.props.get("ring") {
            None | Some(Value::Null) => None,
            Some(Value::Map(m)) => Some(RingSource::from_map(m)),
            Some(other) => Some(Err(format!("the ring is {other:?}, not a map of its kind, path and sizes"))),
        }
    }

    fn from_map(m: &BTreeMap<String, Value>) -> Result<RingSource, String> {
        let text = |key: &str| m.get(key).and_then(Value::as_str).map(str::to_string).ok_or_else(|| format!("the ring names no {key}"));
        let count = |key: &str| match m.get(key).and_then(Value::as_int) {
            Some(n) if n >= 0 => Ok(n),
            _ => Err(format!("the ring names no {key}, a count of zero or more")),
        };
        let kind = text("kind")?;
        let kind = RingKind::named(&kind).ok_or_else(|| format!("{kind} is no kind of ring a widget reads"))?;
        Ok(RingSource { kind, path: text("path")?, capacity: count("capacity")?, producers: count("producers")?, consumers: count("consumers")? })
    }
}

/// What an item read from `node`'s ring changes: a log takes it as one
/// line, the line break at its end dropped; a progress bar reads a JSON
/// object of its Value, Maximum and Text, and a chart one of a point's
/// Series, Y and X, a number or a date and time written as ISO 8601, whose
/// zone, when it names one, is passed over, keys in any case for both. A
/// slot's zero bytes after the item are not part of it. An item the widget
/// cannot take is refused with the reason. A canvas's items come read on
/// its ring's thread instead ([`News::Changes`]), so one that reaches here
/// was read before the canvas took its node's place.
pub fn item_ops(node: &Node, item: &[u8]) -> Result<Vec<PatchOp>, String> {
    let text = item_text(item)?;
    match node.kind {
        Kind::LogView => {
            let line = text.strip_suffix('\n').map_or(text, |l| l.strip_suffix('\r').unwrap_or(l));
            Ok(vec![PatchOp::Append { node: node.id, prop: String::from("lines"), values: vec![Value::Str(line.to_string())] }])
        }
        Kind::ProgressBar => progress_ops(node, text),
        Kind::Chart => chart_ops(node, text),
        Kind::Canvas => Err(String::from("the item was read for the widget the canvas took the place of")),
        other => Err(format!("a {other:?} reads no ring")),
    }
}

/// An item's text: its bytes up to a slot's zeros after it, as UTF-8.
fn item_text(item: &[u8]) -> Result<&str, String> {
    let end = item.iter().rposition(|b| *b != 0).map_or(0, |i| i + 1);
    std::str::from_utf8(&item[..end]).map_err(|e| format!("the item is not UTF-8: {e}"))
}

/// The warning an item `node` refused makes: the widget, why, and the item
/// as text, without a slot's zeros after it.
pub fn refused(node: &Node, why: &str, item: &[u8]) -> String {
    let end = item.iter().rposition(|b| *b != 0).map_or(0, |i| i + 1);
    format!("{} took no item from its ring: {why}; the item was {:?}", widget(node), String::from_utf8_lossy(&item[..end]))
}

/// The warning the reader of node `id`'s ring or shared memory, as `feed`
/// says, makes when it cannot read or stops: `node` is the widget, when the
/// window still has it.
pub fn stopped(node: Option<&Node>, id: NodeId, feed: Feed, text: &str) -> String {
    let who = node.map_or_else(|| format!("node {id}"), widget);
    format!("{who}'s {}: {text}", feed.name())
}

/// A widget as a warning names it: its kind and name, or its node's number.
fn widget(node: &Node) -> String {
    match &node.name {
        Some(name) => format!("{:?} {name}", node.kind),
        None => format!("{:?} node {}", node.kind, node.id),
    }
}

/// The fields of the JSON object in `text`.
fn object(text: &str, what: &str) -> Result<serde_json::Map<String, serde_json::Value>, String> {
    match serde_json::from_str::<serde_json::Value>(text) {
        Ok(serde_json::Value::Object(fields)) => Ok(fields),
        Ok(_) => Err(format!("the item is not a JSON object of {what}")),
        Err(e) => Err(format!("the item is not JSON: {e}")),
    }
}

/// The finite number in field `key`.
fn finite(key: &str, v: &serde_json::Value) -> Result<f64, String> {
    match v.as_f64() {
        Some(n) if n.is_finite() => Ok(n),
        _ => Err(format!("{key} is {v}; it must be a finite number")),
    }
}

/// A progress bar's Value, Maximum and Text from an item. The Maximum must
/// be above 0 and the Value from 0 to the Maximum, those the item does not
/// give being the bar's own.
fn progress_ops(node: &Node, text: &str) -> Result<Vec<PatchOp>, String> {
    let mut props = BTreeMap::new();
    for (key, v) in object(text, "Value, Maximum and Text")? {
        match key.to_ascii_lowercase().as_str() {
            "value" => {
                props.insert(String::from("value"), Value::Float(finite(&key, &v)?));
            }
            "maximum" => {
                props.insert(String::from("maximum"), Value::Float(finite(&key, &v)?));
            }
            "text" => {
                let t = match v {
                    serde_json::Value::Null => Value::Null,
                    serde_json::Value::String(s) => Value::Str(s),
                    other => return Err(format!("{key} is {other}; it must be text or null")),
                };
                props.insert(String::from("text"), t);
            }
            _ => return Err(format!("a progress bar's item takes Value, Maximum and Text; it was given {key}")),
        }
    }
    let now = |key: &str, otherwise: f64| props.get(key).or_else(|| node.props.get(key)).and_then(Value::as_number).unwrap_or(otherwise);
    let maximum = now("maximum", 100.0);
    if maximum <= 0.0 {
        return Err(format!("a progress bar's Maximum is {maximum}; it must be above 0"));
    }
    let value = now("value", 0.0);
    if !(0.0..=maximum).contains(&value) {
        return Err(format!("a progress bar's Value is {value}; it must lie from 0 to its Maximum, {maximum}"));
    }
    Ok(vec![PatchOp::SetProps { node: node.id, props }])
}

/// A chart's point from an item, as Add-GuiItem adds one: its Series and Y,
/// and its X when it gives one. The first point settles whether the chart's
/// X are dates when nothing has, and every point must agree with it.
fn chart_ops(node: &Node, text: &str) -> Result<Vec<PatchOp>, String> {
    let mut point = BTreeMap::new();
    let mut is_date = false;
    for (key, v) in object(text, "Series, X and Y")? {
        match key.to_ascii_lowercase().as_str() {
            "series" => match v {
                serde_json::Value::String(s) => {
                    point.insert(String::from("series"), Value::Str(s));
                }
                other => return Err(format!("{key} is {other}; it must be the name of a series")),
            },
            "y" => {
                point.insert(String::from("y"), Value::Float(finite(&key, &v)?));
            }
            "x" => match v {
                serde_json::Value::String(s) => {
                    let ticks = date::parse(without_zone(&s)).map_err(|e| format!("{key}: {e}"))?;
                    point.insert(String::from("x"), Value::Str(date::full(ticks)));
                    is_date = true;
                }
                other => {
                    point.insert(String::from("x"), Value::Float(finite(&key, &other)?));
                }
            },
            _ => return Err(format!("a chart's point takes Series, X and Y; it was given {key}")),
        }
    }
    if !point.contains_key("series") {
        return Err(String::from("a point needs the Series it joins"));
    }
    if !point.contains_key("y") {
        return Err(String::from("a point needs its Y"));
    }
    let mut ops = Vec::with_capacity(2);
    match node.props.get("dates").and_then(Value::as_bool) {
        Some(dates) if dates != is_date => {
            return Err(String::from("a chart's X are numbers or dates, every point alike, and a point without X counts in numbers"));
        }
        Some(_) => {}
        None => ops.push(PatchOp::SetProps { node: node.id, props: BTreeMap::from([(String::from("dates"), Value::Bool(is_date))]) }),
    }
    ops.push(PatchOp::Append { node: node.id, prop: String::from("points"), values: vec![Value::Map(point)] });
    Ok(ops)
}

/// `text` without the zone ISO 8601 lets a date and time end with: `Z`, or
/// an offset `+HH:MM` or `-HH:MM` after the time.
fn without_zone(text: &str) -> &str {
    if let Some(rest) = text.strip_suffix(['Z', 'z']) {
        return rest;
    }
    let Some(t) = text.find(['T', 't', ' ']) else {
        return text;
    };
    match text[t..].rfind(['+', '-']) {
        Some(i) if text[t + i + 1..].len() == 5 && text.as_bytes()[t + i + 3] == b':' => &text[..t + i],
        _ => text,
    }
}

/// One change a canvas's ring item makes to one of its items, as
/// [`canvas_changes`] reads it: the item it names, as written, the keys it
/// changes but Path and Bytes, in lower case at any depth, and the picture
/// those give.
#[derive(Clone, Debug, PartialEq)]
pub struct CanvasChange {
    pub item: String,
    pub props: BTreeMap<String, Value>,
    pub picture: Option<PictureKeys>,
}

/// The Path or Bytes a change gives: the keys, which an item other than an
/// Image reads as its own, and the picture they give an Image, or why they
/// give none.
#[derive(Clone, Debug, PartialEq)]
pub struct PictureKeys {
    pub keys: BTreeMap<String, Value>,
    pub image: Result<Value, String>,
}

/// The changes a canvas's ring item makes, read as Update-GuiWidget -Item
/// reads one: a JSON object, or an array of them, each naming the item it
/// changes as Item and giving the keys it changes, in any case at any
/// depth, two alike but for case refused, a whole number an integer and a
/// null taking a key away. An Image's Path, a full path, is read here, and
/// its Bytes, written as base64, decoded, so a ring's reader does it and
/// not the renderer's thread. A slot's zero bytes after the item are not
/// part of it.
pub fn canvas_changes(item: &[u8]) -> Result<Vec<CanvasChange>, String> {
    let text = item_text(item)?;
    let (changes, several) = match serde_json::from_str::<serde_json::Value>(text) {
        Ok(serde_json::Value::Array(all)) => (all, true),
        Ok(one @ serde_json::Value::Object(_)) => (vec![one], false),
        Ok(_) => return Err(String::from("the item is not a JSON object of the Item it changes and its keys, nor an array of them")),
        Err(e) => return Err(format!("the item is not JSON: {e}")),
    };
    let place = |i: usize, e: String| if several { format!("change {}: {e}", i + 1) } else { e };
    changes.into_iter().enumerate().map(|(i, c)| canvas_change(c).map_err(|e| place(i, e))).collect()
}

/// One change of a canvas's ring item, as [`canvas_changes`] reads it, a
/// mistake named by the item the change names.
fn canvas_change(change: serde_json::Value) -> Result<CanvasChange, String> {
    let fields = match change {
        serde_json::Value::Object(fields) => fields,
        other => return Err(format!("{other} is not a JSON object of the Item it changes and its keys")),
    };
    let named = fields.iter().find(|(k, _)| k.eq_ignore_ascii_case("item")).and_then(|(_, v)| v.as_str()).map(str::to_string);
    let here = |e: String| match &named {
        Some(n) => format!("{n}: {e}"),
        None => e,
    };
    let (mut item, mut path, mut bytes, mut props) = (None, None, None, BTreeMap::new());
    for (key, written, v) in lowered(fields).map_err(here)? {
        match key.as_str() {
            "item" => match v {
                serde_json::Value::String(s) => item = Some(s),
                other => return Err(format!("{written} is {other}; it is the name of the item the change changes")),
            },
            "path" => path = Some(v),
            "bytes" => bytes = Some(v),
            _ => {
                props.insert(key, json_value(v).map_err(|e| here(format!("{written}: {e}")))?);
            }
        }
    }
    let item = item.ok_or_else(|| String::from("a change names the item it changes as Item"))?;
    let picture = if path.is_none() && bytes.is_none() {
        None
    } else {
        let image = picture_of(path.as_ref(), bytes.as_ref());
        let mut keys = BTreeMap::new();
        for (key, v) in [("path", path), ("bytes", bytes)] {
            if let Some(v) = v {
                keys.insert(String::from(key), json_value(v).map_err(here)?);
            }
        }
        Some(PictureKeys { keys, image })
    };
    Ok(CanvasChange { item, props, picture })
}

/// The picture an Image's Path or Bytes give: the file at the full path
/// Path names, or the bytes Bytes writes as base64, in a format the Image
/// widget takes. A null beside the other is passed over, and alone takes
/// the picture away.
fn picture_of(path: Option<&serde_json::Value>, bytes: Option<&serde_json::Value>) -> Result<Value, String> {
    match (path.filter(|v| !v.is_null()), bytes.filter(|v| !v.is_null())) {
        (Some(_), Some(_)) => Err(String::from("an Image takes Path or Bytes, not both")),
        (Some(serde_json::Value::String(p)), None) => {
            if !std::path::Path::new(p).is_absolute() {
                return Err(format!("Path: {p} is no full path, and the renderer reading the ring has no current location to find it from"));
            }
            let read = std::fs::read(p).map_err(|e| format!("Path: {p} could not be read: {e}"))?;
            crate::picture::value(read).map_err(|e| format!("Path {p}: {e}"))
        }
        (None, Some(serde_json::Value::String(b))) => {
            let read = crate::base64_bytes(b).map_err(|c| format!("Bytes contains {c:?}, which is not base64"))?;
            crate::picture::value(read).map_err(|e| format!("Bytes: {e}"))
        }
        (Some(other), None) => Err(format!("Path is {other}; it is the full path of the picture's file")),
        (None, Some(other)) => Err(format!("Bytes is {other}; it is the picture's bytes written as base64")),
        (None, None) => Ok(Value::Null),
    }
}

/// The fields of a JSON object, each its key in lower case, the key as
/// written and its value; two keys alike but for case are refused.
fn lowered(fields: serde_json::Map<String, serde_json::Value>) -> Result<Vec<(String, String, serde_json::Value)>, String> {
    let mut out: Vec<(String, String, serde_json::Value)> = Vec::with_capacity(fields.len());
    for (written, v) in fields {
        let key = written.to_ascii_lowercase();
        if let Some((_, first, _)) = out.iter().find(|(k, ..)| *k == key) {
            return Err(format!("{first} and {written} are one key, given twice"));
        }
        out.push((key, written, v));
    }
    Ok(out)
}

/// A JSON value as a canvas item's value: a whole number as an integer and
/// any other number as a float, as PowerShell's numbers reach a canvas, and
/// an object's keys in lower case at any depth.
fn json_value(v: serde_json::Value) -> Result<Value, String> {
    Ok(match v {
        serde_json::Value::Null => Value::Null,
        serde_json::Value::Bool(b) => Value::Bool(b),
        serde_json::Value::Number(n) => match n.as_i64() {
            Some(i) => Value::Int(i),
            None => Value::Float(n.as_f64().ok_or_else(|| format!("{n} is no number a canvas reads"))?),
        },
        serde_json::Value::String(s) => Value::Str(s),
        serde_json::Value::Array(items) => Value::List(items.into_iter().map(json_value).collect::<Result<_, _>>()?),
        serde_json::Value::Object(fields) => {
            let mut out = BTreeMap::new();
            for (key, written, v) in lowered(fields)? {
                out.insert(key, json_value(v).map_err(|e| format!("{written}: {e}"))?);
            }
            Value::Map(out)
        }
    })
}

/// The patch the `changes` of a ring item of the canvas `node` make to its
/// items, which are on `board`, as Update-GuiWidget -Item makes one: the
/// item each names must be on the board, an Image takes its picture from
/// its Path or Bytes alone, and each change must leave an item the canvas
/// reads, a later change to the same item counting the ones before it. The
/// first mistake refuses the ring item whole, so none of its changes is
/// made.
pub fn canvas_ops(node: &Node, board: Option<&Board>, changes: Vec<CanvasChange>) -> Result<Vec<PatchOp>, String> {
    if node.kind != Kind::Canvas {
        return Err(format!("the item was read for a canvas, and the widget is a {:?} now", node.kind));
    }
    let board = board.ok_or_else(|| String::from("the canvas's items cannot be read, so they take no change"))?;
    let mut made: HashMap<ItemId, Item> = HashMap::new();
    let mut ops = Vec::with_capacity(changes.len());
    for CanvasChange { item, mut props, picture } in changes {
        let id = board.find(&item).ok_or_else(|| format!("no item is named {item}"))?;
        let now = match made.get(&id) {
            Some(changed) => changed,
            None => &board.entry(id).ok_or_else(|| format!("no item is named {item}"))?.item,
        };
        let kind = now.kind();
        let here = format!("{} {}", kind.name(), now.name.as_deref().unwrap_or(&item));
        if kind == ItemKind::Image && props.contains_key("image") {
            return Err(format!("{here}: an Image takes its picture as Path or Bytes"));
        }
        match picture {
            Some(PictureKeys { image, .. }) if kind == ItemKind::Image => {
                props.insert(String::from("image"), image.map_err(|e| format!("{here}: {e}"))?);
            }
            Some(PictureKeys { keys, .. }) => props.extend(keys),
            None => {}
        }
        let changed = now.changed(&props)?;
        made.insert(id, changed);
        ops.push(PatchOp::SetItem { node: node.id, item, props });
    }
    Ok(ops)
}

/// What a reader reads for a node: the ring its `ring` property names, or
/// the shared memory its `shared` property names.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Feed {
    Ring,
    SharedMemory,
}

impl Feed {
    /// The feed as a warning names it.
    pub fn name(self) -> &'static str {
        match self {
            Feed::Ring => "ring",
            Feed::SharedMemory => "shared memory",
        }
    }
}

/// What a ring's or shared memory's reader sends.
#[derive(Debug, PartialEq)]
pub enum News {
    /// An item read from node `node`'s ring in window `window`.
    Item { window: WindowId, node: NodeId, item: Vec<u8> },
    /// An item read from the ring of the canvas `node` in window `window`,
    /// with the changes it makes as the reader read them
    /// ([`canvas_changes`]), or why it makes none.
    Changes { window: WindowId, node: NodeId, item: Vec<u8>, changes: Result<Vec<CanvasChange>, String> },
    /// The rows of node `node`'s shared memory in window `window` that
    /// changed, each its number and its values ([`Reader::read`]).
    Rows { window: WindowId, node: NodeId, rows: Vec<(u32, Vec<f64>)> },
    /// Why node `node`'s ring or shared memory cannot be read, or why
    /// reading it stopped.
    Problem { window: WindowId, node: NodeId, feed: Feed, text: String },
}

/// A ring a reader has open.
#[derive(Clone)]
enum RingHandle {
    Adaptive(Arc<BlockingAdaptiveRing>),
    Locale(Arc<BlockingLocaleRing>),
    Broadcast(Arc<BlockingBroadcastRing>),
}

/// What a reader has open, closed to end its reader: a ring, or the stop
/// of a shared memory's reader, with the notifier set its reader sleeps on
/// and the thread it reads on, which closing wakes.
#[derive(Clone)]
enum Handle {
    Ring(RingHandle),
    Shared { stop: Arc<AtomicBool>, set: Option<Arc<NotifierSet>>, thread: std::thread::Thread },
}

impl Handle {
    /// Ends every wait through this handle, its reader's among them. A
    /// shared memory's reader is woken through the notifier set it sleeps
    /// on, which wakes every notifier on the set once, or through its
    /// thread, and stops.
    fn close(&self) {
        match self {
            Handle::Ring(RingHandle::Adaptive(r)) => r.close(),
            Handle::Ring(RingHandle::Locale(r)) => r.close(),
            Handle::Ring(RingHandle::Broadcast(r)) => r.close(),
            Handle::Shared { stop, set, thread } => {
                stop.store(true, Ordering::Release);
                if let Some(s) = set {
                    s.signal();
                }
                thread.unpark();
            }
        }
    }
}

/// What a node's feed names, as the reader of it was started for.
#[derive(Clone, Debug, PartialEq)]
enum Watched {
    /// The ring, and whether the node is a canvas.
    Ring(Result<RingSource, String>, bool),
    Shared(Result<SharedSource, String>),
}

/// A node's reader, or why it has none, recorded so the reason is told once.
struct Reading {
    watched: Watched,
    reader: Option<Running>,
}

/// A reader's thread, which has its ring or shared memory open until it
/// returns, and what closing ends it through.
struct Running {
    handle: Handle,
    thread: std::thread::JoinHandle<()>,
}

/// Stops `readers`: closes every one first, so they all end at once, then
/// waits for each thread to return, after which nothing in this process
/// has their rings or shared memory open and a writer can remove the
/// files. A reader whose thread panicked is written to stderr, which a
/// helper's log keeps.
fn stop(readers: Vec<((WindowId, NodeId, Feed), Reading)>) {
    let running: Vec<((WindowId, NodeId, Feed), Running)> = readers.into_iter().filter_map(|(key, r)| r.reader.map(|run| (key, run))).collect();
    for (_, run) in &running {
        run.handle.close();
    }
    for ((window, node, _), run) in running {
        if let Err(panic) = run.thread.join() {
            let why = match (panic.downcast_ref::<&str>(), panic.downcast_ref::<String>()) {
                (Some(text), _) => text.to_string(),
                (None, Some(text)) => text.clone(),
                (None, None) => String::from("a panic with no message"),
            };
            eprintln!("PWRS-GUI: the reader of node {node} in window {window} panicked: {why}");
        }
    }
}

/// The rings and shared memory a renderer's widgets read, each on a thread
/// of its own, which sends what it reads to [`Sources::take`] and wakes the
/// renderer's loop.
pub struct Sources {
    wake: WakeLoop,
    tx: mpsc::Sender<News>,
    rx: mpsc::Receiver<News>,
    readers: HashMap<(WindowId, NodeId, Feed), Reading>,
    /// The last refusal of each shared memory's rows told, so one that
    /// recurs at every read is told once.
    refusals: HashMap<(WindowId, NodeId), String>,
}

impl Sources {
    pub fn new(wake: WakeLoop) -> Sources {
        let (tx, rx) = mpsc::channel();
        Sources { wake, tx, rx, readers: HashMap::new(), refusals: HashMap::new() }
    }

    /// Reads the ring and the shared memory of each node of `tree`, window
    /// `window`'s tree, that names one, and stops reading those whose node
    /// has gone, names another now, or has become a canvas or stopped being
    /// one; call it when a window opens and after each patch.
    pub fn follow(&mut self, window: WindowId, tree: &Node) {
        let mut wanted = HashMap::new();
        feeds_in(tree, &mut wanted);
        let gone: Vec<(NodeId, Feed)> = self
            .readers
            .iter()
            .filter(|((w, n, f), r)| *w == window && wanted.get(&(*n, *f)) != Some(&r.watched))
            .map(|((_, n, f), _)| (*n, *f))
            .collect();
        let mut stopped = Vec::new();
        for (node, feed) in gone {
            self.refusals.remove(&(window, node));
            if let Some(r) = self.readers.remove(&(window, node, feed)) {
                stopped.push(((window, node, feed), r));
            }
        }
        stop(stopped);
        for ((node, feed), watched) in wanted {
            if self.readers.contains_key(&(window, node, feed)) {
                continue;
            }
            let outlet = |canvas: bool| Outlet { window, node, feed, canvas, tx: self.tx.clone(), wake: Arc::clone(&self.wake) };
            let started = match &watched {
                Watched::Ring(Ok(s), canvas) => start(s, outlet(*canvas)),
                Watched::Shared(Ok(s)) => start_shared(s, outlet(false)),
                Watched::Ring(Err(text), _) | Watched::Shared(Err(text)) => Err(text.clone()),
            };
            let reader = match started {
                Ok(r) => Some(r),
                Err(text) => {
                    self.tell(window, node, feed, text);
                    None
                }
            };
            self.readers.insert((window, node, feed), Reading { watched, reader });
        }
    }

    /// Stops reading every ring and shared memory window `window`'s widgets
    /// read, returning once each reader's thread has, so a script that
    /// removes a ring as the window closes can delete its files.
    pub fn forget(&mut self, window: WindowId) {
        let ended: Vec<(WindowId, NodeId, Feed)> = self.readers.keys().filter(|(w, ..)| *w == window).copied().collect();
        stop(ended.into_iter().filter_map(|key| self.readers.remove(&key).map(|r| (key, r))).collect());
        self.refusals.retain(|(w, _), _| *w != window);
    }

    /// What the readers have read and said since this was last called, in
    /// the order each reader sent it, the rows a shared memory's reader sent
    /// several times joined in the place of its first, each row at its
    /// latest values.
    pub fn take(&self) -> Vec<News> {
        let mut out: Vec<News> = Vec::new();
        let mut held: HashMap<(WindowId, NodeId), (usize, HashMap<u32, usize>)> = HashMap::new();
        for news in self.rx.try_iter() {
            match news {
                News::Rows { window, node, rows } => match held.get_mut(&(window, node)) {
                    Some((at, places)) => {
                        if let Some(News::Rows { rows: joined, .. }) = out.get_mut(*at) {
                            for (row, values) in rows {
                                match places.get(&row) {
                                    Some(i) => joined[*i].1 = values,
                                    None => {
                                        places.insert(row, joined.len());
                                        joined.push((row, values));
                                    }
                                }
                            }
                        }
                    }
                    None => {
                        let places = rows.iter().enumerate().map(|(i, (row, _))| (*row, i)).collect();
                        held.insert((window, node), (out.len(), places));
                        out.push(News::Rows { window, node, rows });
                    }
                },
                other => out.push(other),
            }
        }
        out
    }

    /// Whether `text`, why node `node`'s rows in window `window` changed
    /// none of its items, differs from the refusal last told for it, and so
    /// is to be told; `None` says its rows were taken, so the next refusal
    /// is told whatever it is.
    pub fn refusal(&mut self, window: WindowId, node: NodeId, text: Option<&str>) -> bool {
        match text {
            None => {
                self.refusals.remove(&(window, node));
                false
            }
            Some(t) => {
                if self.refusals.get(&(window, node)).is_some_and(|told| told == t) {
                    return false;
                }
                self.refusals.insert((window, node), t.to_string());
                true
            }
        }
    }

    fn tell(&self, window: WindowId, node: NodeId, feed: Feed, text: String) {
        if self.tx.send(News::Problem { window, node, feed, text }).is_ok() {
            (self.wake)();
        }
    }
}

impl Drop for Sources {
    fn drop(&mut self) {
        stop(self.readers.drain().collect());
    }
}

/// The ring and the shared memory each node of `node`'s subtree names, by
/// node and feed.
fn feeds_in(node: &Node, out: &mut HashMap<(NodeId, Feed), Watched>) {
    if let Some(source) = RingSource::of(node) {
        out.insert((node.id, Feed::Ring), Watched::Ring(source, node.kind == Kind::Canvas));
    }
    if let Some(source) = SharedSource::of(node) {
        out.insert((node.id, Feed::SharedMemory), Watched::Shared(source));
    }
    for c in &node.children {
        feeds_in(c, out);
    }
}

/// Where a reader sends what it reads, and whether it reads a canvas's
/// ring, whose items it reads into their changes before it sends them.
struct Outlet {
    window: WindowId,
    node: NodeId,
    feed: Feed,
    canvas: bool,
    tx: mpsc::Sender<News>,
    wake: WakeLoop,
}

impl Outlet {
    /// Sends an item, a canvas's with the changes it makes; false when the
    /// renderer has stopped taking them.
    fn item(&self, item: Vec<u8>) -> bool {
        let news = if self.canvas {
            let changes = canvas_changes(&item);
            News::Changes { window: self.window, node: self.node, item, changes }
        } else {
            News::Item { window: self.window, node: self.node, item }
        };
        self.send(news)
    }

    /// Sends the rows of a shared memory that changed; false when the
    /// renderer has stopped taking them.
    fn rows(&self, rows: Vec<(u32, Vec<f64>)>) -> bool {
        self.send(News::Rows { window: self.window, node: self.node, rows })
    }

    fn problem(&self, text: String) {
        self.send(News::Problem { window: self.window, node: self.node, feed: self.feed, text });
    }

    fn send(&self, news: News) -> bool {
        let sent = self.tx.send(news).is_ok();
        if sent {
            (self.wake)();
        }
        sent
    }
}

/// Opens `source` and starts its reader.
fn start(source: &RingSource, outlet: Outlet) -> Result<Running, String> {
    let size = |n: i64, what: &str| usize::try_from(n).map_err(|e| format!("the ring's {what}, {n}, is no size this machine takes: {e}"));
    let capacity = size(source.capacity, "capacity")?;
    if capacity < 2 || !capacity.is_power_of_two() || u32::try_from(capacity).is_err() {
        return Err(format!("the ring's capacity, {capacity}, is no ring's: a ring has a power of two of slots, from 2 to 2^31"));
    }
    let path = &source.path;
    let ring = match source.kind {
        RingKind::Ring | RingKind::LocaleRing => {
            let (producers, consumers) = (size(source.producers, "producer count")?, size(source.consumers, "consumer count")?);
            if producers == 0 || consumers == 0 {
                return Err(String::from("the ring names no producer or no consumer; a ring has at least one of each"));
            }
            if source.kind == RingKind::Ring {
                let ring = AdaptiveRing::open(path, producers, consumers, capacity).map_err(|e| format!("the ring at {path} cannot be opened: {e}"))?;
                let ring = BlockingAdaptiveRing::beside_files(Arc::new(ring), path, MAX_WAITERS_DEFAULT).map_err(|e| format!("the wakers of the ring at {path} cannot be opened: {e}"))?;
                RingHandle::Adaptive(Arc::new(ring))
            } else {
                let ring = LocaleAdaptiveRing::open(path.as_str(), producers, consumers, capacity, false).map_err(|e| format!("the ring at {path} cannot be opened: {e}"))?;
                let ring = BlockingLocaleRing::beside_file(Arc::new(ring), path, MAX_WAITERS_DEFAULT).map_err(|e| format!("the wakers of the ring at {path} cannot be opened: {e}"))?;
                RingHandle::Locale(Arc::new(ring))
            }
        }
        RingKind::BroadcastRing => {
            let ring = SharedBroadcastRing::open(path, capacity).map_err(|e| format!("the broadcast ring at {path} cannot be opened: {e}"))?;
            let ring = BlockingBroadcastRing::beside_file(Arc::new(ring), path, MAX_WAITERS_DEFAULT).map_err(|e| format!("the wakers of the broadcast ring at {path} cannot be opened: {e}"))?;
            RingHandle::Broadcast(Arc::new(ring))
        }
    };
    let reading = ring.clone();
    let thread = std::thread::Builder::new()
        .name(String::from("pwrs-gui-ring"))
        .spawn(move || match reading {
            RingHandle::Adaptive(r) => read_frames(&r, &outlet),
            RingHandle::Locale(r) => read_locale_slots(&r, &outlet),
            RingHandle::Broadcast(r) => read_broadcast(&r, &outlet),
        })
        .map_err(|e| format!("the ring's reader cannot start: {e}"))?;
    Ok(Running { handle: Handle::Ring(ring), thread })
}

/// Opens the shared memory `source` and starts its reader, attached to the
/// notifier set the source names, when it names one.
fn start_shared(source: &SharedSource, outlet: Outlet) -> Result<Running, String> {
    let reader = Reader::open(source)?;
    let set = match &source.notifier {
        Some(path) => Some(Arc::new(NotifierSet::file(path).map_err(|e| format!("the notifier set at {path} cannot be opened: {e}"))?)),
        None => None,
    };
    let notifier = match (&set, &source.notifier) {
        (Some(s), Some(path)) => Some(s.attach().map_err(|e| format!("the notifier set at {path} takes no notifier for this window: {e}"))?),
        _ => None,
    };
    let interval = match source.interval {
        Some(seconds) => Some(Duration::try_from_secs_f64(seconds).map_err(|e| format!("the shared memory's interval, {seconds} s, is no time this machine waits: {e}"))?),
        None => None,
    };
    if notifier.is_none() && interval.is_none() {
        return Err(String::from("the shared memory names no notifier set and no interval, so nothing would ever read it"));
    }
    let stop = Arc::new(AtomicBool::new(false));
    let stopped = Arc::clone(&stop);
    let thread = std::thread::Builder::new()
        .name(String::from("pwrs-gui-shared"))
        .spawn(move || read_shared(reader, notifier.as_ref(), interval, &stopped, &outlet))
        .map_err(|e| format!("the shared memory's reader cannot start: {e}"))?;
    let handle = Handle::Shared { stop, set, thread: thread.thread().clone() };
    Ok(Running { handle, thread })
}

/// Reads a shared memory until its handle is closed: once at the start,
/// then at each signal of its notifier and at least every `interval`, or
/// every `interval` alone, sending the rows that changed and telling a
/// reason it cannot read once until a read succeeds.
fn read_shared(mut reader: Reader, notifier: Option<&Notifier>, interval: Option<Duration>, stop: &AtomicBool, out: &Outlet) {
    let mut told: Option<String> = None;
    loop {
        match reader.read() {
            Ok(rows) => {
                told = None;
                if !rows.is_empty() && !out.rows(rows) {
                    break;
                }
            }
            Err(text) => {
                if told.as_deref() != Some(text.as_str()) {
                    out.problem(text.clone());
                    told = Some(text);
                }
            }
        }
        if stop.load(Ordering::Acquire) {
            break;
        }
        match (notifier, interval) {
            (Some(n), wait) => {
                let ms = wait.map_or(-1, |d| d.as_millis().min(i32::MAX as u128) as i32);
                if n.wait(ms) {
                    n.drain();
                }
            }
            (None, Some(wait)) => std::thread::park_timeout(wait),
            (None, None) => break,
        }
        if stop.load(Ordering::Acquire) {
            break;
        }
    }
}

/// Reads the frames of an adaptive ring until its handle is closed.
fn read_frames(ring: &BlockingAdaptiveRing, out: &Outlet) {
    let consumer = match ring.ring().register_consumer() {
        Ok(c) => c,
        Err(e) => return out.problem(format!("the ring takes no more readers: {e}")),
    };
    let mut item = Vec::new();
    loop {
        match ring.recv_frame_blocking(consumer, &mut item, None) {
            Ok(_) => {
                if !out.item(std::mem::take(&mut item)) {
                    break;
                }
            }
            Err(BlockingError::Closed) => break,
            Err(e) => {
                out.problem(format!("reading the ring stopped: {e}"));
                break;
            }
        }
    }
    ring.ring().unregister_consumer(consumer);
}

/// Reads a locale ring's slots until its handle is closed, waiting on the
/// ring's waker, which an item, a move to another locale and a close all
/// wake, and saying so when the ring moves into its writer's own memory,
/// where no other process can read it.
fn read_locale_slots(ring: &BlockingLocaleRing, out: &Outlet) {
    let consumer = match ring.ring().register_consumer() {
        Ok(c) => c,
        Err(e) => return out.problem(format!("the ring takes no more readers: {e}")),
    };
    let mut slot = [0u8; SPSC_PAYLOAD_BYTES];
    let mut away = false;
    'reading: loop {
        loop {
            match ring.try_recv(consumer, &mut slot) {
                Ok(n) => {
                    if !out.item(slot[..n].to_vec()) {
                        break 'reading;
                    }
                }
                Err(RingError::Empty) => break,
                Err(e) => {
                    out.problem(format!("reading the ring stopped: {e}"));
                    break 'reading;
                }
            }
        }
        if ring.is_closed() {
            break;
        }
        let anon = ring.ring().current_locale() == Locale::Anon;
        if anon && !away {
            out.problem(String::from("the ring moved into its writer's own memory, where no other process can read it, so its items no longer reach the window until it moves back"));
        }
        away = anon;
        let waker = ring.consumer_waker();
        let token = match waker.try_park(0) {
            Ok(t) => t,
            Err(e) => {
                out.problem(format!("the ring's reader cannot wait: {e}"));
                break;
            }
        };
        match ring.try_recv(consumer, &mut slot) {
            Ok(n) => {
                waker.release(token);
                if !out.item(slot[..n].to_vec()) {
                    break;
                }
                continue;
            }
            Err(RingError::Empty) => {}
            Err(e) => {
                waker.release(token);
                out.problem(format!("reading the ring stopped: {e}"));
                break;
            }
        }
        if ring.is_closed() {
            waker.release(token);
            break;
        }
        if let Err(e) = waker.wait(token, None) {
            out.problem(format!("the ring's reader cannot wait: {e}"));
            break;
        }
    }
    // A locale ring claims a consumer's id on each of its three backings and
    // gives none back itself, so the reader leaves each one.
    let locales = ring.ring();
    for backing in [locales.file_ring(), locales.shmfs_ring(), locales.anon_ring()] {
        backing.unregister_consumer(consumer);
    }
}

/// Reads a broadcast ring's slots until its handle is closed.
fn read_broadcast(ring: &BlockingBroadcastRing, out: &Outlet) {
    let consumer = match ring.ring().register_consumer() {
        Ok(c) => c,
        Err(e) => return out.problem(format!("the broadcast ring takes no more readers: {e}")),
    };
    let mut slot = [0u8; BROADCAST_PAYLOAD_BYTES];
    loop {
        match ring.recv_blocking(consumer, &mut slot, None) {
            Ok(n) => {
                if !out.item(slot[..n].to_vec()) {
                    break;
                }
            }
            Err(BlockingWaitError::Closed) => break,
            Err(e) => {
                out.problem(format!("reading the broadcast ring stopped: {e}"));
                break;
            }
        }
    }
    ring.unregister_consumer(consumer);
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    fn source(kind: RingKind, path: &str) -> RingSource {
        RingSource { kind, path: path.to_string(), capacity: 64, producers: 1, consumers: 1 }
    }

    fn reading(kind: Kind, source: &RingSource) -> Node {
        Node::new(1, Kind::Column).child(Node::new(2, kind).named("Feed").prop("ring", source.value()))
    }

    /// The patch an item makes, which the test expects the widget to take.
    fn taken(node: &Node, item: &[u8]) -> Vec<PatchOp> {
        item_ops(node, item).expect("the widget takes the item")
    }

    /// Why the widget refuses an item, which the test expects it to.
    fn refused(node: &Node, item: &[u8]) -> String {
        item_ops(node, item).expect_err("the widget refuses the item")
    }

    /// The next piece of news `sources` has, within 20 seconds.
    fn news(sources: &Sources) -> News {
        let deadline = Instant::now() + Duration::from_secs(20);
        loop {
            match sources.rx.try_recv() {
                Ok(n) => return n,
                Err(mpsc::TryRecvError::Empty) => {}
                Err(mpsc::TryRecvError::Disconnected) => panic!("the readers' channel closed with no news"),
            }
            assert!(Instant::now() < deadline, "no news came within 20 seconds");
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    /// A folder for a test's ring, removed with what is in it when the test
    /// ends, whether it passed or not.
    struct Folder(std::path::PathBuf);

    impl Folder {
        fn new(name: &str) -> Folder {
            let dir = std::env::temp_dir().join(format!("pwrs-gui-source-{name}-{}", std::process::id()));
            std::fs::create_dir_all(&dir).expect("a folder for the ring");
            Folder(dir)
        }

        fn path(&self, file: &str) -> String {
            self.0.join(file).display().to_string()
        }
    }

    impl Drop for Folder {
        fn drop(&mut self) {
            if let Err(e) = std::fs::remove_dir_all(&self.0) {
                eprintln!("the ring's folder {} was not removed: {e}", self.0.display());
            }
        }
    }

    fn quiet() -> WakeLoop {
        Arc::new(|| {})
    }

    #[test]
    fn a_ring_crosses_as_its_property_and_back() {
        let s = RingSource { kind: RingKind::LocaleRing, path: String::from("/tmp/feed"), capacity: 256, producers: 2, consumers: 1 };
        let node = Node::new(3, Kind::LogView).prop("ring", s.value());
        assert_eq!(RingSource::of(&node).expect("a ring named").expect("named rightly"), s);
        assert!(RingSource::of(&Node::new(3, Kind::LogView)).is_none());
        assert!(RingSource::of(&Node::new(3, Kind::LogView).prop("ring", Value::Null)).is_none());
    }

    #[test]
    fn a_ring_property_that_names_no_ring_rightly_says_why() {
        let why = |v: Value| RingSource::of(&Node::new(3, Kind::LogView).prop("ring", v)).expect("a ring named").expect_err("named wrongly");
        let mut m = match source(RingKind::Ring, "/tmp/feed").value() {
            Value::Map(m) => m,
            other => panic!("a ring's property is a map, not {other:?}"),
        };
        m.insert(String::from("kind"), Value::Str(String::from("PubSub")));
        assert_eq!(why(Value::Map(m.clone())), "PubSub is no kind of ring a widget reads");
        m.insert(String::from("kind"), Value::Str(String::from("Ring")));
        m.insert(String::from("capacity"), Value::Int(-1));
        assert_eq!(why(Value::Map(m.clone())), "the ring names no capacity, a count of zero or more");
        m.remove("path");
        assert!(why(Value::Map(m)).contains("path"));
        assert!(why(Value::Int(4)).contains("not a map"));
    }

    #[test]
    fn a_log_takes_each_item_as_one_line_without_its_line_break() {
        let log = Node::new(4, Kind::LogView);
        let appended = |text: &str| vec![PatchOp::Append { node: 4, prop: String::from("lines"), values: vec![Value::Str(text.to_string())] }];
        assert_eq!(taken(&log, b"copied a.txt\r\n"), appended("copied a.txt"));
        assert_eq!(taken(&log, b"two\nlines"), appended("two\nlines"));
        assert_eq!(taken(&log, b"padded\0\0\0"), appended("padded"));
        assert!(refused(&log, &[0xff, 0xfe]).contains("not UTF-8"));
    }

    #[test]
    fn a_progress_bar_takes_its_value_maximum_and_text_in_any_case() {
        let bar = Node::new(5, Kind::ProgressBar).prop("maximum", Value::Float(200.0));
        let set = |pairs: &[(&str, Value)]| vec![PatchOp::SetProps { node: 5, props: pairs.iter().map(|(k, v)| (k.to_string(), v.clone())).collect() }];
        assert_eq!(taken(&bar, br#"{"value": 150, "text": "copying"}"#), set(&[("value", Value::Float(150.0)), ("text", Value::Str(String::from("copying")))]));
        assert_eq!(taken(&bar, br#"{"Text": null}"#), set(&[("text", Value::Null)]));
        assert_eq!(taken(&bar, br#"{"Maximum": 50, "Value": 50}"#), set(&[("maximum", Value::Float(50.0)), ("value", Value::Float(50.0))]));
        assert!(refused(&bar, br#"{"Value": 201}"#).contains("from 0 to its Maximum, 200"));
        assert!(refused(&bar, br#"{"Value": -1}"#).contains("from 0 to its Maximum"));
        assert!(refused(&bar, br#"{"Maximum": 0}"#).contains("above 0"));
        assert!(refused(&bar, br#"{"Speed": 3}"#).contains("takes Value, Maximum and Text; it was given Speed"));
        assert!(refused(&bar, br#"{"Value": "half"}"#).contains("finite number"));
        assert!(refused(&bar, b"42").contains("not a JSON object"));
        assert!(refused(&bar, b"{value").contains("not JSON"));
        assert!(refused(&Node::new(5, Kind::ProgressBar), br#"{"Value": 101}"#).contains("Maximum, 100"));
    }

    #[test]
    fn a_chart_takes_points_that_settle_whether_its_x_are_dates() {
        let cpu = |rest: &[(&str, Value)]| {
            let mut point: BTreeMap<String, Value> = BTreeMap::from([(String::from("series"), Value::Str(String::from("cpu")))]);
            point.extend(rest.iter().map(|(k, v)| (k.to_string(), v.clone())));
            Value::Map(point)
        };
        let appended = |p: Value| PatchOp::Append { node: 6, prop: String::from("points"), values: vec![p] };
        let settled = |dates: bool| PatchOp::SetProps { node: 6, props: BTreeMap::from([(String::from("dates"), Value::Bool(dates))]) };
        let fresh = Node::new(6, Kind::Chart);
        assert_eq!(taken(&fresh, br#"{"Series": "cpu", "y": 2}"#), vec![settled(false), appended(cpu(&[("y", Value::Float(2.0))]))]);
        let written = taken(&fresh, br#"{"series": "cpu", "X": "2026-10-08T14:31:00.25-04:00", "Y": 3}"#);
        assert_eq!(written, vec![settled(true), appended(cpu(&[("x", Value::Str(String::from("2026-10-08T14:31:00.25"))), ("y", Value::Float(3.0))]))]);
        let dated = Node::new(6, Kind::Chart).prop("dates", Value::Bool(true));
        let utc = taken(&dated, br#"{"Series": "cpu", "X": "2026-10-08T14:31Z", "Y": 1}"#);
        assert_eq!(utc, vec![appended(cpu(&[("x", Value::Str(String::from("2026-10-08T14:31:00"))), ("y", Value::Float(1.0))]))]);
        assert!(refused(&dated, br#"{"Series": "cpu", "X": 4, "Y": 1}"#).contains("numbers or dates"));
        assert!(refused(&dated, br#"{"Series": "cpu", "X": "yesterday", "Y": 1}"#).contains("ISO 8601"));
        let numbered = Node::new(6, Kind::Chart).prop("dates", Value::Bool(false));
        assert_eq!(taken(&numbered, br#"{"Series": "cpu", "X": 4, "Y": 1}"#), vec![appended(cpu(&[("x", Value::Float(4.0)), ("y", Value::Float(1.0))]))]);
        assert!(refused(&numbered, br#"{"X": 4, "Y": 1}"#).contains("needs the Series"));
        assert!(refused(&numbered, br#"{"Series": "cpu"}"#).contains("needs its Y"));
        assert!(refused(&numbered, br#"{"Series": "cpu", "Y": 1, "Z": 2}"#).contains("it was given Z"));
    }

    /// A picture's bytes, an SVG of the fewest a format can be told from,
    /// and the same written as base64.
    const SVG: &[u8] = b"<svg/>";
    const SVG_BASE64: &str = "PHN2Zy8+";

    fn picture_of_bytes(bytes: &[u8]) -> Value {
        crate::picture::value(bytes.to_vec()).expect("a picture in a format the Image widget takes")
    }

    /// Canvas node 3, with a Rect named Box and an Image named Logo, and
    /// the board of its items.
    fn canvas() -> (Node, Board) {
        let item = |kind: &str, name: &str, rest: &[(&str, Value)]| {
            let mut map = BTreeMap::from([(String::from("kind"), Value::Str(kind.to_string())), (String::from("name"), Value::Str(name.to_string()))]);
            for key in ["x", "y", "width", "height"] {
                map.insert(key.to_string(), Value::Int(10));
            }
            map.extend(rest.iter().map(|(k, v)| (k.to_string(), v.clone())));
            Value::Map(map)
        };
        let items = Value::List(vec![item("Rect", "Box", &[]), item("Image", "Logo", &[("image", picture_of_bytes(SVG))])]);
        let node = Node::new(3, Kind::Canvas).named("Board").prop("items", items);
        let board = Board::of(&node).expect("the canvas's items read");
        (node, board)
    }

    /// The patch a ring item makes to `canvas()`, which the test expects the
    /// canvas to take.
    fn changed(item: &[u8]) -> Vec<PatchOp> {
        let (node, board) = canvas();
        canvas_changes(item).and_then(|c| canvas_ops(&node, Some(&board), c)).expect("the canvas takes the item")
    }

    /// Why `canvas()` refuses a ring item, which the test expects it to.
    fn unchanged(item: &[u8]) -> String {
        let (node, board) = canvas();
        canvas_changes(item).and_then(|c| canvas_ops(&node, Some(&board), c)).expect_err("the canvas refuses the item")
    }

    fn set(item: &str, pairs: &[(&str, Value)]) -> PatchOp {
        PatchOp::SetItem { node: 3, item: item.to_string(), props: pairs.iter().map(|(k, v)| (k.to_string(), v.clone())).collect() }
    }

    fn list(values: &[Value]) -> Value {
        Value::List(values.to_vec())
    }

    fn text(t: &str) -> Value {
        Value::Str(t.to_string())
    }

    #[test]
    fn a_canvas_takes_changes_to_its_items_in_any_case_as_update_gui_widget_item_makes_them() {
        assert_eq!(changed(br#"{"Item": "Box", "X": 20}"#), vec![set("Box", &[("x", Value::Int(20))])]);
        assert_eq!(changed(b"{\"item\": \"box\", \"WIDTH\": 2.5}\0\0"), vec![set("box", &[("width", Value::Float(2.5))])], "a slot's zeros after the item are not part of it");
        let fill = Value::Map(BTreeMap::from([
            (String::from("linear"), list(&[text("Red"), text("Blue")])),
            (String::from("from"), list(&[Value::Int(0), Value::Int(0)])),
            (String::from("to"), list(&[Value::Int(1), Value::Int(0)])),
        ]));
        let gradient = changed(br#"{"Item": "Box", "Fill": {"Linear": ["Red", "Blue"], "From": [0, 0], "To": [1, 0]}}"#);
        assert_eq!(gradient, vec![set("Box", &[("fill", fill)])], "an object's keys are in lower case at any depth");
        assert_eq!(changed(br#"{"Item": "Box", "Fill": null}"#), vec![set("Box", &[("fill", Value::Null)])], "a null takes a key away");
        let animate = Value::Map(BTreeMap::from([(String::from("property"), text("X")), (String::from("to"), Value::Int(100)), (String::from("duration"), Value::Float(0.5))]));
        assert_eq!(changed(br#"{"Item": "Box", "Animate": {"Property": "X", "To": 100, "Duration": 0.5}}"#), vec![set("Box", &[("animate", animate)])]);
        let several = changed(br#"[{"Item": "Box", "Width": 5}, {"Item": "Box", "X": 3}, {"Item": "Logo", "Y": 1}]"#);
        assert_eq!(several, vec![set("Box", &[("width", Value::Int(5))]), set("Box", &[("x", Value::Int(3))]), set("Logo", &[("y", Value::Int(1))])]);
        assert_eq!(changed(b"[]"), Vec::new(), "an empty array changes nothing");
    }

    #[test]
    fn a_canvas_refuses_a_ring_item_whole_for_its_first_mistake() {
        assert_eq!(unchanged(br#"{"Item": "Nobody", "X": 1}"#), "no item is named Nobody");
        assert!(unchanged(br#"{"Item": "Box", "Kind": "Ellipse"}"#).starts_with("Rect Box: a change sets no Kind"));
        assert!(unchanged(br#"{"Item": "Box", "Name": "Crate"}"#).starts_with("Rect Box: a change sets no Name"));
        assert!(unchanged(br#"{"Item": "Box", "Fill": "Plaid"}"#).starts_with("Rect Box: Fill"));
        assert!(unchanged(br#"{"Item": "Box", "Radius": 4}"#).starts_with("Rect Box: a Rect takes no Radius"));
        assert!(unchanged(br#"[{"Item": "Box", "X": 1}, {"Item": "Box", "Fill": "Plaid"}]"#).starts_with("Rect Box: Fill"), "a later change's mistake refuses the earlier ones too");
        assert!(unchanged(br#"{"Item": "Box", "X": 1, "x": 2}"#).ends_with("are one key, given twice"));
        assert!(unchanged(br#"{"Item": "Box", "Fill": {"Linear": ["Red", "Blue"], "linear": ["Red"]}}"#).starts_with("Box: Fill: "));
        assert_eq!(unchanged(br#"[{"Item": "Box", "X": 1}, {"X": 2}]"#), "change 2: a change names the item it changes as Item");
        assert!(unchanged(br#"{"Item": 4}"#).contains("it is the name of the item the change changes"));
        assert!(unchanged(b"[4]").starts_with("change 1: 4 is not a JSON object"));
        assert!(unchanged(b"42").contains("nor an array of them"));
        assert!(unchanged(b"{item").contains("not JSON"));
        assert!(unchanged(&[0xff, 0xfe]).contains("not UTF-8"));
    }

    #[test]
    fn an_image_takes_its_picture_from_the_full_path_of_its_file_or_its_bytes_in_base64() {
        let dir = Folder::new("canvas-picture");
        let file = dir.path("logo.svg");
        let drawn = b"<svg width=\"8\"/>";
        std::fs::write(&file, drawn).expect("the picture's file");
        let by_path = format!(r#"{{"Item": "Logo", "Path": {}}}"#, serde_json::Value::String(file.clone()));
        assert_eq!(changed(by_path.as_bytes()), vec![set("Logo", &[("image", picture_of_bytes(drawn))])]);
        let by_bytes = format!(r#"{{"item": "logo", "bytes": "{SVG_BASE64}"}}"#);
        assert_eq!(changed(by_bytes.as_bytes()), vec![set("logo", &[("image", picture_of_bytes(SVG))])]);
        let beside_null = format!(r#"{{"Item": "Logo", "Path": null, "Bytes": "{SVG_BASE64}"}}"#);
        assert_eq!(changed(beside_null.as_bytes()), vec![set("Logo", &[("image", picture_of_bytes(SVG))])], "a null beside the other is passed over");
        assert!(unchanged(br#"{"Item": "Logo", "Path": "logo.svg"}"#).starts_with("Image Logo: Path: logo.svg is no full path"));
        let missing = format!(r#"{{"Item": "Logo", "Path": {}}}"#, serde_json::Value::String(dir.path("missing.svg")));
        assert!(unchanged(missing.as_bytes()).contains("missing.svg could not be read"));
        let both = format!(r#"{{"Item": "Logo", "Path": {}, "Bytes": "{SVG_BASE64}"}}"#, serde_json::Value::String(file.clone()));
        assert_eq!(unchanged(both.as_bytes()), "Image Logo: an Image takes Path or Bytes, not both");
        assert_eq!(unchanged(br#"{"Item": "Logo", "Image": null}"#), "Image Logo: an Image takes its picture as Path or Bytes");
        assert_eq!(unchanged(br#"{"Item": "Logo", "Bytes": "*"}"#), "Image Logo: Bytes contains '*', which is not base64");
        assert!(unchanged(br#"{"Item": "Logo", "Bytes": "AAAA"}"#).starts_with("Image Logo: Bytes: the bytes begin 00 00 00"));
        assert!(unchanged(br#"{"Item": "Logo", "Path": 4}"#).starts_with("Image Logo: Path is 4; it is the full path"));
        assert!(unchanged(br#"{"Item": "Box", "Path": "logo.svg"}"#).starts_with("Rect Box: a Rect takes no Path"), "only an Image takes a picture");
    }

    #[test]
    fn a_canvas_takes_ring_items_only_with_its_items_read_and_only_while_it_is_one() {
        let (node, board) = canvas();
        let changes = || canvas_changes(br#"{"Item": "Box", "X": 1}"#).expect("the item reads");
        assert!(canvas_ops(&node, None, changes()).expect_err("no board").contains("cannot be read"));
        let log = Node::new(3, Kind::LogView);
        assert!(canvas_ops(&log, Some(&board), changes()).expect_err("no canvas").contains("the widget is a LogView now"));
        assert!(item_ops(&node, br#"{"Item": "Box", "X": 1}"#).expect_err("read for another kind").contains("took the place of"));
    }

    #[test]
    fn a_zone_after_the_time_is_passed_over() {
        assert_eq!(without_zone("2026-10-08T14:31:00Z"), "2026-10-08T14:31:00");
        assert_eq!(without_zone("2026-10-08T14:31:00+05:30"), "2026-10-08T14:31:00");
        assert_eq!(without_zone("2026-10-08T14:31:00.1234567-04:00"), "2026-10-08T14:31:00.1234567");
        assert_eq!(without_zone("2026-10-08T14:31"), "2026-10-08T14:31");
        assert_eq!(without_zone("2026-10-08"), "2026-10-08");
    }

    #[test]
    fn an_adaptive_ring_reader_hands_over_each_frame_and_ends_with_its_window() {
        let dir = Folder::new("ring");
        let path = dir.path("feed");
        {
            let writer = AdaptiveRing::create(&path, 1, 1, 64).expect("the ring");
            let writer = BlockingAdaptiveRing::beside_files(Arc::new(writer), &path, MAX_WAITERS_DEFAULT).expect("its wakers");
            let producer = writer.ring().register_producer().expect("a producer");
            let mut sources = Sources::new(quiet());
            sources.follow(7, &reading(Kind::LogView, &source(RingKind::Ring, &path)));
            let long = "a line far longer than one slot of the ring takes, so it travels in a block of the frame region";
            writer.send_frame_as(producer, long.as_bytes(), subetha_cxc::frame_ring::LayoutHint::Auto).expect("a frame sent");
            assert_eq!(news(&sources), News::Item { window: 7, node: 2, item: long.as_bytes().to_vec() });
            sources.forget(7);
        }
    }

    #[test]
    fn a_canvas_rings_reader_hands_over_each_item_read_into_its_changes() {
        let dir = Folder::new("canvas");
        let path = dir.path("feed");
        {
            let writer = AdaptiveRing::create(&path, 1, 1, 64).expect("the ring");
            let writer = BlockingAdaptiveRing::beside_files(Arc::new(writer), &path, MAX_WAITERS_DEFAULT).expect("its wakers");
            let producer = writer.ring().register_producer().expect("a producer");
            let mut sources = Sources::new(quiet());
            sources.follow(11, &reading(Kind::Canvas, &source(RingKind::Ring, &path)));
            let item = format!(r#"[{{"Item": "Box", "X": 20}}, {{"Item": "Logo", "Bytes": "{SVG_BASE64}"}}]"#);
            writer.send_frame_as(producer, item.as_bytes(), subetha_cxc::frame_ring::LayoutHint::Auto).expect("a frame sent");
            let moved = CanvasChange { item: String::from("Box"), props: BTreeMap::from([(String::from("x"), Value::Int(20))]), picture: None };
            let keys = BTreeMap::from([(String::from("bytes"), text(SVG_BASE64))]);
            let pictured = CanvasChange { item: String::from("Logo"), props: BTreeMap::new(), picture: Some(PictureKeys { keys, image: Ok(picture_of_bytes(SVG)) }) };
            assert_eq!(news(&sources), News::Changes { window: 11, node: 2, item: item.into_bytes(), changes: Ok(vec![moved, pictured]) });
            sources.forget(11);
        }
    }

    #[test]
    fn a_broadcast_ring_reader_sees_what_is_pushed_once_it_reads() {
        let dir = Folder::new("broadcast");
        let path = dir.path("feed");
        {
            let writer = SharedBroadcastRing::create(&path, 8).expect("the broadcast ring");
            let writer = BlockingBroadcastRing::beside_file(Arc::new(writer), &path, MAX_WAITERS_DEFAULT).expect("its wakers");
            let mut sources = Sources::new(quiet());
            sources.follow(8, &reading(Kind::ProgressBar, &RingSource { capacity: 8, ..source(RingKind::BroadcastRing, &path) }));
            assert_eq!(writer.ring().wait_for_consumers(1, Duration::from_secs(20)), 1, "the reader registers");
            writer.try_push(br#"{"Value": 40}"#).expect("an item pushed");
            let item = match news(&sources) {
                News::Item { item, .. } => item,
                other => panic!("{other:?}"),
            };
            let set = vec![PatchOp::SetProps { node: 2, props: BTreeMap::from([(String::from("value"), Value::Float(40.0))]) }];
            assert_eq!(taken(&Node::new(2, Kind::ProgressBar), &item), set);
            sources.forget(8);
        }
    }

    #[test]
    fn a_locale_ring_reader_hands_over_items_and_says_when_the_ring_moves_where_it_cannot_follow() {
        let dir = Folder::new("locale");
        let path = dir.path("feed");
        {
            let writer = LocaleAdaptiveRing::create(path.as_str(), 1, 1, 64).expect("the locale ring");
            let writer = BlockingLocaleRing::beside_file(Arc::new(writer), &path, MAX_WAITERS_DEFAULT).expect("its wakers");
            let producer = writer.ring().register_producer().expect("a producer");
            writer.ring().migrate_to(Locale::File).expect("the move out of the writer's memory, where a locale ring starts");
            let mut sources = Sources::new(quiet());
            sources.follow(9, &reading(Kind::Chart, &source(RingKind::LocaleRing, &path)));
            writer.try_send(producer, br#"{"Series": "a", "Y": 1}"#).expect("an item sent");
            let item = match news(&sources) {
                News::Item { window: 9, node: 2, item } => item,
                other => panic!("{other:?}"),
            };
            let point = Value::Map(BTreeMap::from([(String::from("series"), Value::Str(String::from("a"))), (String::from("y"), Value::Float(1.0))]));
            let chart = Node::new(2, Kind::Chart).prop("dates", Value::Bool(false));
            assert_eq!(taken(&chart, &item), vec![PatchOp::Append { node: 2, prop: String::from("points"), values: vec![point] }], "a slot's zeros after the item are not part of it");
            writer.ring().migrate_to(Locale::Anon).expect("the move to the writer's memory");
            let moved = news(&sources);
            assert!(matches!(&moved, News::Problem { window: 9, node: 2, feed: Feed::Ring, text } if text.contains("writer's own memory")), "{moved:?}");
            sources.forget(9);
        }
    }

    #[test]
    fn a_shared_memory_reader_wakes_at_its_notifier_and_hands_over_the_rows_that_changed() {
        let dir = Folder::new("shared");
        let (path, notify) = (dir.path("rows"), dir.path("notify"));
        {
            let writer = subetha_cxc::RawSlab::create(&path, 4, subetha_cxc::ElementLayout { slot_size: 16, alignment: 1, tag: 0 }).expect("the slab");
            let set = NotifierSet::file(&notify).expect("the notifier set");
            let shared = SharedSource {
                store: crate::shared::Store::Slab { path: path.clone(), capacity: 4, slot: 16, alignment: 1, tag: 0 },
                element: crate::gpu::ElementType::Float64,
                items: vec![String::from("*")],
                properties: vec![String::from("x"), String::from("y")],
                notifier: Some(notify.clone()),
                interval: None,
            };
            let mut sources = Sources::new(quiet());
            sources.follow(12, &Node::new(1, Kind::Column).child(Node::new(2, Kind::Canvas).named("Board").prop("shared", shared.value())));
            assert_eq!(set.attached(), 1, "the reader sleeps on a notifier of the set");
            let bytes: Vec<u8> = [3.0f64, 4.0].iter().flat_map(|v| v.to_le_bytes()).collect();
            writer.set(2, &bytes).expect("slot 2 written");
            set.signal();
            assert_eq!(news(&sources), News::Rows { window: 12, node: 2, rows: vec![(2, vec![3.0, 4.0])] });
            sources.forget(12);
        }
    }

    #[test]
    fn rows_a_shared_memory_sends_twice_are_taken_once_at_their_latest_values() {
        let sources = Sources::new(quiet());
        let send = |news: News| sources.tx.send(news).expect("the news sent");
        send(News::Rows { window: 1, node: 2, rows: vec![(0, vec![1.0]), (1, vec![2.0])] });
        send(News::Problem { window: 1, node: 3, feed: Feed::Ring, text: String::from("stopped") });
        send(News::Rows { window: 1, node: 2, rows: vec![(1, vec![5.0]), (4, vec![6.0])] });
        let joined = News::Rows { window: 1, node: 2, rows: vec![(0, vec![1.0]), (1, vec![5.0]), (4, vec![6.0])] };
        assert_eq!(sources.take(), vec![joined, News::Problem { window: 1, node: 3, feed: Feed::Ring, text: String::from("stopped") }]);
    }

    #[test]
    fn a_refusal_of_a_shared_memorys_rows_is_told_once_until_rows_are_taken() {
        let mut sources = Sources::new(quiet());
        assert!(sources.refusal(1, 2, Some("row 0: no")));
        assert!(!sources.refusal(1, 2, Some("row 0: no")), "the same refusal again is not told");
        assert!(sources.refusal(1, 2, Some("row 1: no")), "another is");
        assert!(!sources.refusal(1, 2, None));
        assert!(sources.refusal(1, 2, Some("row 1: no")), "after rows were taken the refusal is told again");
        assert_eq!(stopped(None, 4, Feed::SharedMemory, "gone"), "node 4's shared memory: gone");
    }

    #[test]
    fn a_ring_that_cannot_be_opened_is_told_once() {
        let mut sources = Sources::new(quiet());
        let missing = std::env::temp_dir().join(format!("pwrs-gui-source-missing-{}", std::process::id())).join("feed").display().to_string();
        let tree = reading(Kind::LogView, &source(RingKind::Ring, &missing));
        sources.follow(10, &tree);
        sources.follow(10, &tree);
        let told: Vec<News> = sources.take();
        assert_eq!(told.len(), 1, "{told:?}");
        assert!(matches!(&told[0], News::Problem { window: 10, node: 2, feed: Feed::Ring, text } if text.contains("cannot be opened")), "{told:?}");
        sources.follow(10, &reading(Kind::LogView, &RingSource { capacity: 48, ..source(RingKind::Ring, &missing) }));
        assert!(matches!(sources.take().as_slice(), [News::Problem { text, .. }] if text.contains("power of two")));
    }
}
