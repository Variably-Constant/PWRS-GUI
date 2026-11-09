//! A canvas's -SharedMemory as the renderer reads it: a hashtable naming a
//! SubEtha slab, vec or cell under Source, or under Path a file another
//! process writes, with the byte Offset of its values, their Length, the
//! byte Sequence of its counter and, when given, their Shape; the Type of
//! the values, the Item the rows go to and the Property each column sets;
//! and Notifier, the SubEtha notifier set the writer signals, or Interval,
//! the seconds between reads, or both. A mistake is refused here, when the
//! canvas is made, naming the key.

use crate::canvas::TICKS_PER_SECOND;
use crate::widgets::{full_path, strings};
use pwrs::prelude::*;
use pwrs::sys::PS_TYPE_TIMESPAN;
use pwrs_gui_client::proto::Value;
use pwrs_gui_client::proto::canvas::{ONE_NUMBER, shown_key};
use pwrs_gui_client::proto::gpu::ElementType;
use pwrs_gui_client::proto::shared::{SharedSource, Store};

/// The keys a canvas's -SharedMemory takes, as a script writes them.
const KEYS: [&str; 11] = ["Source", "Path", "Offset", "Length", "Shape", "Sequence", "Type", "Item", "Property", "Notifier", "Interval"];

/// The keys that name a file, which a SubEtha structure under Source
/// takes the place of.
const FILE_KEYS: [&str; 5] = ["Path", "Offset", "Length", "Shape", "Sequence"];

fn message(e: PsError) -> String {
    e.message
}

/// The canvas's shared memory the hashtable `given` names, as its node's
/// `shared` property has it.
pub(crate) fn canvas_shared(ps: &Pipeline<'_>, given: &PsObject) -> Result<Value, String> {
    let table = PsReadOnlyTable::over(given).map_err(|e| format!("it is a hashtable of {}: {}", KEYS.join(", "), e.message))?;
    let mut keys: Vec<(&'static str, PsObject)> = Vec::new();
    for written in table.keys().map_err(message)? {
        let key = KEYS.into_iter().find(|k| k.eq_ignore_ascii_case(&written)).ok_or_else(|| format!("it takes {}; it was given {written}", KEYS.join(", ")))?;
        if keys.iter().any(|(k, _)| *k == key) {
            return Err(format!("it gives {key} twice"));
        }
        keys.push((key, given.call("get_Item", &[written.as_str().into_ps().map_err(message)?]).map_err(message)?));
    }
    let get = |key: &str| keys.iter().find(|(k, _)| *k == key).map(|(_, v)| v).filter(|v| !v.is_null());
    let need = |key: &str| get(key).ok_or_else(|| format!("it names no {key}"));

    let written = String::from_ps(need("Type")?).map_err(|e| format!("Type: {}", e.message))?;
    let names: Vec<&str> = ElementType::ALL.iter().map(|t| t.name()).collect();
    let element = ElementType::named(written.trim()).ok_or_else(|| format!("Type is {written}; it is one of {}", names.join(", ")))?;

    let items = strings(ps, need("Item")?).map_err(|e| format!("Item: {}", e.message))?;
    if items.is_empty() || items.iter().any(|i| i.trim().is_empty()) {
        return Err(String::from("Item names no item; it takes the names of items and wildcards"));
    }
    let mut properties: Vec<String> = Vec::new();
    for written in strings(ps, need("Property")?).map_err(|e| format!("Property: {}", e.message))? {
        let key = written.trim().to_ascii_lowercase();
        if !ONE_NUMBER.contains(&key.as_str()) {
            let takes: Vec<&str> = ONE_NUMBER.iter().map(|k| shown_key(k)).collect();
            return Err(format!("Property names {written}, which takes no single number; a column sets one of {}", takes.join(", ")));
        }
        if properties.contains(&key) {
            return Err(format!("Property names {} twice", shown_key(&key)));
        }
        properties.push(key);
    }
    if properties.is_empty() {
        return Err(String::from("Property names no key for the columns to set"));
    }
    let size = element.size();
    let columns = properties.len() as u64;
    let row = columns * size;

    let store = match get("Source") {
        Some(source) => {
            if let Some(key) = FILE_KEYS.into_iter().find(|k| get(k).is_some()) {
                return Err(format!("it gives Source and {key}; a SubEtha structure goes under Source, and a file under Path with its Offset, Length and Sequence"));
            }
            subetha(source, row, columns, element)?
        }
        None => {
            let count = |key: &str| -> Result<u64, String> { u64::from_ps(need(key)?).map_err(|e| format!("{key}: {}", e.message)) };
            let signed = |key: &str, n: u64| i64::try_from(n).map_err(|e| format!("{key}, {n}, is past any file: {e}"));
            let written = String::from_ps(need("Path")?).map_err(|e| format!("Path: {}", e.message))?;
            let path = full_path(ps, &written).map_err(message)?;
            let (offset, length, sequence) = (count("Offset")?, count("Length")?, count("Sequence")?);
            if length == 0 || length % columns != 0 {
                return Err(format!("Length is {length}, which is no whole number of rows of {columns} values, one for each name in Property"));
            }
            if let Some(shape) = get("Shape") {
                let given = Vec::<u64>::from_ps(shape).map_err(|e| format!("Shape: {}", e.message))?;
                if !matches!(given.as_slice(), [rows, cols] if *cols == columns && rows.checked_mul(*cols) == Some(length)) {
                    return Err(format!("Shape is {}; it is the rows and the columns of Length's {length} values, {columns} columns, one for each name in Property", given.iter().map(u64::to_string).collect::<Vec<_>>().join(",")));
                }
            }
            let values_end = length.checked_mul(size).and_then(|b| b.checked_add(offset)).ok_or_else(|| format!("Offset {offset} and Length {length} reach past any file"))?;
            let counter_end = sequence.checked_add(8).ok_or_else(|| format!("Sequence {sequence} is past any file"))?;
            if sequence < values_end && offset < counter_end {
                return Err(format!("Sequence is {sequence}, among the values from byte {offset} to {values_end}; the counter has eight bytes of its own"));
            }
            let end = values_end.max(counter_end);
            let held = std::fs::metadata(&path).map_err(|e| format!("Path {path} cannot be read: {e}"))?.len();
            if held < end {
                let size = if held == 1 { String::from("1 byte") } else { format!("{held} bytes") };
                return Err(format!("the file {path} has {size}, shorter than its values and counter, which end at byte {end}"));
            }
            Store::File { path, offset: signed("Offset", offset)?, length: signed("Length", length)?, sequence: signed("Sequence", sequence)? }
        }
    };

    let notifier = match get("Notifier") {
        None => None,
        Some(set) => {
            let kind = set.type_name().map_err(message)?;
            if kind != "SubEtha.NotifierSet" {
                return Err(format!("Notifier is a {kind}; it takes the SubEtha.NotifierSet the writer signals after each change"));
            }
            Some(String::from_ps(&set.get("Path").map_err(message)?).map_err(|e| format!("Notifier's Path: {}", e.message))?)
        }
    };
    let interval = match get("Interval") {
        None => None,
        Some(v) => {
            let seconds = if v.type_tag().map_err(message)? == PS_TYPE_TIMESPAN {
                PsTimeSpan::from_ps(v).map_err(message)?.ticks as f64 / TICKS_PER_SECOND
            } else {
                f64::from_ps(v).map_err(|e| format!("Interval: {}", e.message))?
            };
            if !(seconds.is_finite() && seconds > 0.0) {
                return Err(format!("Interval is {seconds}; it is the seconds between reads, above 0"));
            }
            Some(seconds)
        }
    };
    if notifier.is_none() && interval.is_none() {
        return Err(String::from("it names no Notifier and no Interval, so nothing would ever read it; it takes Notifier, the SubEtha.NotifierSet the writer signals after each change, Interval, the seconds between reads, or both"));
    }
    Ok(SharedSource { store, element, items, properties, notifier, interval }.value())
}

/// The SubEtha structure `source` is, a `SubEtha.Slab` or `SubEtha.Vec`
/// whose slots each have one row of `row` bytes, `columns` values of
/// `element`, or a `SubEtha.Cell` containing whole rows. A slab's or vec's
/// layout is its Alignment and Tag where the object's type has them, and
/// otherwise alignment 1 and tag 0, which New-SubEthaSlab and
/// New-SubEthaVec give when not told others. A vec's writer table is its
/// Writers where the object's type has them, and otherwise none, as a vec
/// whose type names no writer table was made without one.
fn subetha(source: &PsObject, row: u64, columns: u64, element: ElementType) -> Result<Store, String> {
    let kind = source.type_name().map_err(message)?;
    let read = |name: &str| -> Result<u64, String> { u64::from_ps(&source.get(name).map_err(message)?).map_err(|e| format!("Source's {name}: {}", e.message)) };
    let signed = |name: &str, n: u64| i64::try_from(n).map_err(|e| format!("Source's {name}, {n}, is past any structure: {e}"));
    let path = || String::from_ps(&source.get("Path").map_err(message)?).map_err(|e| format!("Source's Path: {}", e.message));
    let has = |name: &str| -> Result<bool, String> {
        let ty = source.call("GetType", &[]).map_err(message)?;
        Ok(!ty.call("GetProperty", &[name.into_ps().map_err(message)?]).map_err(message)?.is_null())
    };
    match kind.as_str() {
        "SubEtha.Slab" | "SubEtha.Vec" => {
            let (capacity, slot) = (read("Capacity")?, read("ElementSize")?);
            if slot != row {
                return Err(format!("each slot of Source has {slot} bytes, where {columns} values of {}, one for each name in Property, take {row}", element.name()));
            }
            let alignment = if has("Alignment")? { read("Alignment")? } else { 1 };
            let tag = if has("Tag")? { read("Tag")? } else { 0 };
            let (path, capacity, slot, alignment) = (path()?, signed("Capacity", capacity)?, signed("ElementSize", slot)?, signed("Alignment", alignment)?);
            if kind == "SubEtha.Slab" {
                return Ok(Store::Slab { path, capacity, slot, alignment, tag });
            }
            let writers = if has("Writers")? { read("Writers")? } else { 0 };
            Ok(Store::Vec { path, capacity, writers: signed("Writers", writers)?, slot, alignment, tag })
        }
        "SubEtha.Cell" => {
            let size = read("ValueSize")?;
            if size % row != 0 {
                return Err(format!("Source has {size} bytes, which is no whole number of rows of {row} bytes, {columns} values of {}, one for each name in Property", element.name()));
            }
            Ok(Store::Cell { path: path()?, size: signed("ValueSize", size)? })
        }
        other => Err(format!("Source is a {other}; it takes a SubEtha.Slab, SubEtha.Vec or SubEtha.Cell")),
    }
}
