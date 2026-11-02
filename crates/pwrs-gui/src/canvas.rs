//! A canvas's items as a script writes them, hashtables of each one's Kind,
//! Name and keys, as the renderer's item maps: every key in lower case at
//! any depth, a Group's items read as items, a time span as its seconds,
//! and an Image's Path or Bytes read into its picture as the Image widget's
//! are. The protocol crate's scene reads the maps made, so a mistake is
//! refused before a window gets it, naming the item as the scene does.

use crate::values::to_value;
use crate::widgets::{picture, picture_file};
use pwrs::prelude::*;
use pwrs::sys::{PS_TYPE_OBJECT, PS_TYPE_TIMESPAN};
use pwrs_gui_client::proto::Value;
use pwrs_gui_client::proto::canvas::{Board, ItemKind, Scene};
use std::collections::BTreeMap;

/// Ticks in a second; a tick is 100 ns.
pub(crate) const TICKS_PER_SECOND: f64 = 10_000_000.0;

fn message(e: PsError) -> String {
    e.message
}

/// What a value is to a canvas: nothing, a dictionary, a list, or any
/// other value, a scalar among them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Shape {
    Empty,
    Map,
    List,
    Other,
}

/// The shape of `given`.
fn shape(given: &PsObject) -> Result<Shape, String> {
    if given.is_null() {
        return Ok(Shape::Empty);
    }
    if given.type_tag().map_err(message)? != PS_TYPE_OBJECT {
        return Ok(Shape::Other);
    }
    object_shape(given)
}

/// The shape of `given`, an object of no scalar type, told by the
/// interfaces its type implements, so a dictionary or list of any type
/// reads as one.
fn object_shape(given: &PsObject) -> Result<Shape, String> {
    let ty = given.call("GetType", &[]).map_err(message)?;
    let implements = |interface: &str| -> Result<bool, String> {
        let found = ty.call("GetInterface", &[interface.into_ps().map_err(message)?]).map_err(message)?;
        Ok(!found.is_null())
    };
    if implements("System.Collections.IDictionary")? {
        Ok(Shape::Map)
    } else if implements("System.Collections.IList")? {
        Ok(Shape::List)
    } else {
        Ok(Shape::Other)
    }
}

/// The items in `given`, one hashtable alone being a list of one, as
/// the renderer's list of item maps, refused unless the scene they make
/// reads.
pub(crate) fn items(ps: &Pipeline<'_>, given: &PsObject) -> Result<Value, String> {
    let items = Value::List(listed(ps, given, "item")?);
    Scene::of_items(&items)?;
    Ok(items)
}

/// The item maps Add-GuiItem adds, one for each of `given`, which the
/// canvas's scene reads once they join it.
pub(crate) fn added(ps: &Pipeline<'_>, given: &[PsObject]) -> Result<Vec<Value>, String> {
    given.iter().enumerate().map(|(i, one)| item_of(ps, one, &format!("item {}", i + 1))).collect()
}

/// The changes the hashtable `given` makes to the item named `name` on
/// `board`, a canvas's items, as the renderer takes them: its keys in lower
/// case, a null taking a key away, and for an Image a Path or Bytes read
/// into its picture.
pub(crate) fn changes(ps: &Pipeline<'_>, board: &Board, name: &str, given: &PsObject) -> Result<BTreeMap<String, Value>, String> {
    let entry = board.find(name).and_then(|id| board.entry(id)).ok_or_else(|| format!("no item is named {name}"))?;
    let kind = entry.item.kind();
    let here = format!("{} {}", kind.name(), entry.item.name.as_deref().unwrap_or(name));
    if shape(given)? != Shape::Map {
        return Err(format!("{here}: an item's change is a hashtable of its keys"));
    }
    let entries = lowered(given).map_err(|e| format!("{here}: {e}"))?;
    fields(ps, &entries, Some(kind), &here)
}

/// The item maps in `given`, one alone being a list of one and nothing
/// none, each named in an error by `prefix` and its place among them until
/// it has a name of its own.
fn listed(ps: &Pipeline<'_>, given: &PsObject, prefix: &str) -> Result<Vec<Value>, String> {
    let each = match shape(given)? {
        Shape::Empty => return Ok(Vec::new()),
        Shape::List => Vec::<PsObject>::from_ps(given).map_err(message)?,
        Shape::Map | Shape::Other => vec![given.clone()],
    };
    each.iter().enumerate().map(|(i, one)| item_of(ps, one, &format!("{prefix} {}", i + 1))).collect()
}

/// The item map the hashtable `given` makes, `place` naming it until it has
/// a name; any other value is passed on for the scene to refuse.
fn item_of(ps: &Pipeline<'_>, given: &PsObject, place: &str) -> Result<Value, String> {
    if shape(given)? != Shape::Map {
        return value_of(given).map_err(|e| format!("{place}: {e}"));
    }
    let entries = lowered(given).map_err(|e| format!("{place}: {e}"))?;
    let text = |key: &str| -> Result<Option<String>, String> {
        match entries.iter().find(|(k, ..)| k == key) {
            Some((_, _, v)) => match value_of(v)? {
                Value::Str(s) => Ok(Some(s)),
                _ => Ok(None),
            },
            None => Ok(None),
        }
    };
    let kind = text("kind")?.and_then(|written| ItemKind::ALL.into_iter().find(|k| k.name().eq_ignore_ascii_case(&written)));
    let name = text("name")?.filter(|n| !n.is_empty());
    let here = match (kind, &name) {
        (Some(k), Some(n)) => format!("{} {n}", k.name()),
        (Some(k), None) => format!("{place}, {}", k.article()),
        (None, _) => place.to_string(),
    };
    fields(ps, &entries, kind, &here).map(Value::Map)
}

/// The renderer's map of an item's `entries`, `kind` its kind where it
/// names one a canvas draws and `here` naming it in an error: a Group's
/// items read as items, and an Image's Path or Bytes read into its picture.
/// A null stays under its key, which a change reads as taking it away.
fn fields(ps: &Pipeline<'_>, entries: &[(String, String, PsObject)], kind: Option<ItemKind>, here: &str) -> Result<BTreeMap<String, Value>, String> {
    let image = kind == Some(ItemKind::Image);
    let given = |key: &str| entries.iter().any(|(k, _, v)| k == key && !v.is_null());
    if image && given("path") && given("bytes") {
        return Err(format!("{here}: an Image takes Path or Bytes, not both"));
    }
    let mut out = BTreeMap::new();
    for (key, written, v) in entries {
        match key.as_str() {
            "image" if image => return Err(format!("{here}: an Image takes its picture as Path or Bytes")),
            "path" | "bytes" if image => {
                if v.is_null() {
                    out.entry(String::from("image")).or_insert(Value::Null);
                    continue;
                }
                let shown = if key == "path" {
                    let path = String::from_ps(v).map_err(|e| format!("{here}: Path: {}", e.message))?;
                    let bytes = picture_file(ps, &path).map_err(|e| format!("{here}: Path: {e}"))?;
                    picture(bytes).map_err(|e| format!("{here}: Path {path}: {e}"))?
                } else {
                    let bytes = Vec::<u8>::from_ps(v).map_err(|e| format!("{here}: Bytes: {}", e.message))?;
                    picture(bytes).map_err(|e| format!("{here}: Bytes: {e}"))?
                };
                out.insert(String::from("image"), shown);
            }
            "children" if !v.is_null() => {
                out.insert(key.clone(), Value::List(listed(ps, v, &format!("{here}'s item"))?));
            }
            _ => {
                out.insert(key.clone(), value_of(v).map_err(|e| format!("{here}: {written}: {e}"))?);
            }
        }
    }
    Ok(out)
}

/// The entries of the dictionary `given`, each its key in lower case, the
/// key as written and the value; two keys alike but for case are refused.
fn lowered(given: &PsObject) -> Result<Vec<(String, String, PsObject)>, String> {
    let keys = Vec::<String>::from_ps(&given.get("Keys").map_err(message)?).map_err(message)?;
    let mut out: Vec<(String, String, PsObject)> = Vec::with_capacity(keys.len());
    for written in keys {
        let key = written.to_ascii_lowercase();
        if let Some((_, first, _)) = out.iter().find(|(k, ..)| *k == key) {
            return Err(format!("{first} and {written} are one key, given twice"));
        }
        let v = given.call("get_Item", &[written.as_str().into_ps().map_err(message)?]).map_err(message)?;
        out.push((key, written, v));
    }
    Ok(out)
}

/// A key's value as the renderer takes it: a dictionary's keys in lower
/// case and its values read alike, a list's items read alike, a time span
/// as its seconds, and anything else as the window's values are.
fn value_of(given: &PsObject) -> Result<Value, String> {
    if given.is_null() {
        return Ok(Value::Null);
    }
    let tag = given.type_tag().map_err(message)?;
    if tag == PS_TYPE_TIMESPAN {
        let span = PsTimeSpan::from_ps(given).map_err(message)?;
        return Ok(Value::Float(span.ticks as f64 / TICKS_PER_SECOND));
    }
    if tag != PS_TYPE_OBJECT {
        return to_value(given).map_err(message);
    }
    match object_shape(given)? {
        Shape::Map => {
            let mut out = BTreeMap::new();
            for (key, written, v) in lowered(given)? {
                out.insert(key, value_of(&v).map_err(|e| format!("{written}: {e}"))?);
            }
            Ok(Value::Map(out))
        }
        Shape::List => Vec::<PsObject>::from_ps(given).map_err(message)?.iter().map(value_of).collect::<Result<Vec<_>, _>>().map(Value::List),
        Shape::Empty | Shape::Other => to_value(given).map_err(message),
    }
}
