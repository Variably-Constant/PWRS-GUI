//! A widget's shared memory: values another process writes, which the
//! renderer reads row by row without passing through PowerShell. They live
//! in a SubEtha slab or vec, each slot a row, in a SubEtha cell, its value
//! the rows end to end, or in a plain file whose 64-bit counter the writer
//! makes odd before it writes and even after, a seqlock, so no half-written
//! array is read. The module names the source in the node's `shared`
//! property ([`SharedSource`]); [`Sources`](crate::source::Sources) reads
//! each on a thread of its own through a [`Reader`], woken by the source's
//! SubEtha notifier set or at its interval, and gives the renderer the rows
//! that changed, which [`canvas_ops`] turns into a canvas's changes: row i
//! to the i-th item the source names and column j to property j.

use crate::canvas::{Board, ItemId};
use crate::gpu::ElementType;
use crate::{Node, PatchOp, Value};
use std::collections::{BTreeMap, HashSet};
use std::time::{Duration, Instant};
use subetha_cxc::{ElementLayout, RawCell, RawSlab, RawVec};

/// How often a file's array is read again within one read when its counter
/// moved while it was copied.
const FILE_TRIES: usize = 8;

/// How long a file's counter stays odd at one value before the writer is
/// taken to have left a write unfinished.
const HELD_ODD: Duration = Duration::from_secs(1);

/// Where a source's values live, its counts and offsets each zero or more.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Store {
    /// A SubEtha slab of `capacity` slots of `slot` bytes, each slot a row,
    /// made with the slot `alignment` and the `tag` its layout records.
    Slab { path: String, capacity: i64, slot: i64, alignment: i64, tag: u64 },
    /// A SubEtha vec of up to `capacity` elements of `slot` bytes, each
    /// live element a row, made with a writer table of `writers` entries
    /// and the `alignment` and `tag` its layout records.
    Vec { path: String, capacity: i64, writers: i64, slot: i64, alignment: i64, tag: u64 },
    /// A SubEtha cell of `size` bytes, the rows end to end.
    Cell { path: String, size: i64 },
    /// A file with `length` values from byte `offset`, the rows end to end,
    /// and the writer's 64-bit counter at byte `sequence`.
    File { path: String, offset: i64, length: i64, sequence: i64 },
}

impl Store {
    /// The kind's name, as the `shared` property names it.
    fn kind(&self) -> &'static str {
        match self {
            Store::Slab { .. } => "slab",
            Store::Vec { .. } => "vec",
            Store::Cell { .. } => "cell",
            Store::File { .. } => "file",
        }
    }

    /// The file the values live in.
    pub fn path(&self) -> &str {
        match self {
            Store::Slab { path, .. } | Store::Vec { path, .. } | Store::Cell { path, .. } | Store::File { path, .. } => path,
        }
    }
}

/// A widget's shared memory as the module names it in the node's `shared`
/// property.
#[derive(Clone, Debug, PartialEq)]
pub struct SharedSource {
    pub store: Store,
    /// The type of the values.
    pub element: ElementType,
    /// The items the rows go to, in order: names, each taking one place
    /// whether or not the item is in the scene now, and wildcards, each
    /// taking a place for each item it matches, in the scene's order.
    pub items: Vec<String>,
    /// The key each column sets, in lower case.
    pub properties: Vec<String>,
    /// The path of the SubEtha notifier set the writer signals after each
    /// change.
    pub notifier: Option<String>,
    /// The seconds between reads, whether or not a signal comes.
    pub interval: Option<f64>,
}

impl SharedSource {
    /// The node's `shared` property for this source.
    pub fn value(&self) -> Value {
        let texts = |all: &[String]| Value::List(all.iter().cloned().map(Value::Str).collect());
        let mut m = BTreeMap::from([
            (String::from("kind"), Value::Str(self.store.kind().to_string())),
            (String::from("path"), Value::Str(self.store.path().to_string())),
            (String::from("type"), Value::Str(self.element.name().to_string())),
            (String::from("items"), texts(&self.items)),
            (String::from("properties"), texts(&self.properties)),
            (String::from("notifier"), self.notifier.clone().map_or(Value::Null, Value::Str)),
            (String::from("interval"), self.interval.map_or(Value::Null, Value::Float)),
        ]);
        let counts = match &self.store {
            Store::Slab { capacity, slot, alignment, .. } => vec![("capacity", *capacity), ("slot", *slot), ("alignment", *alignment)],
            Store::Vec { capacity, writers, slot, alignment, .. } => vec![("capacity", *capacity), ("writers", *writers), ("slot", *slot), ("alignment", *alignment)],
            Store::Cell { size, .. } => vec![("size", *size)],
            Store::File { offset, length, sequence, .. } => vec![("offset", *offset), ("length", *length), ("sequence", *sequence)],
        };
        for (key, n) in counts {
            m.insert(key.to_string(), Value::Int(n));
        }
        // The tag's 64 bits ride in the protocol's signed integer as they are.
        if let Store::Slab { tag, .. } | Store::Vec { tag, .. } = &self.store {
            m.insert(String::from("tag"), Value::Int(*tag as i64));
        }
        Value::Map(m)
    }

    /// The shared memory `node` reads, when its `shared` property names
    /// one, or why the property names none rightly.
    pub fn of(node: &Node) -> Option<Result<SharedSource, String>> {
        match node.props.get("shared") {
            None | Some(Value::Null) => None,
            Some(Value::Map(m)) => Some(SharedSource::from_map(m)),
            Some(other) => Some(Err(format!("the shared memory is {other:?}, not a map of its store and keys"))),
        }
    }

    fn from_map(m: &BTreeMap<String, Value>) -> Result<SharedSource, String> {
        let text = |key: &str| m.get(key).and_then(Value::as_str).map(str::to_string).ok_or_else(|| format!("the shared memory names no {key}"));
        let count = |key: &str| match m.get(key).and_then(Value::as_int) {
            Some(n) if n >= 0 => Ok(n),
            _ => Err(format!("the shared memory names no {key}, a count of zero or more")),
        };
        let texts = |key: &str| match m.get(key) {
            Some(Value::List(all)) => all.iter().map(|v| v.as_str().map(str::to_string).ok_or_else(|| format!("the shared memory's {key} are not all text"))).collect::<Result<Vec<_>, _>>(),
            _ => Err(format!("the shared memory names no {key}")),
        };
        let tag = || match m.get("tag") {
            Some(Value::Int(bits)) => Ok(*bits as u64),
            _ => Err(String::from("the shared memory names no tag")),
        };
        let path = text("path")?;
        let kind = text("kind")?;
        let store = match kind.as_str() {
            "slab" => Store::Slab { path, capacity: count("capacity")?, slot: count("slot")?, alignment: count("alignment")?, tag: tag()? },
            "vec" => Store::Vec { path, capacity: count("capacity")?, writers: count("writers")?, slot: count("slot")?, alignment: count("alignment")?, tag: tag()? },
            "cell" => Store::Cell { path, size: count("size")? },
            "file" => Store::File { path, offset: count("offset")?, length: count("length")?, sequence: count("sequence")? },
            _ => return Err(format!("{kind} is no kind of shared memory a widget reads")),
        };
        let written = text("type")?;
        let element = ElementType::named(&written).ok_or_else(|| format!("{written} is no element type"))?;
        let notifier = match m.get("notifier") {
            None | Some(Value::Null) => None,
            Some(Value::Str(p)) => Some(p.clone()),
            Some(other) => return Err(format!("the shared memory's notifier is {other:?}, not the path of a notifier set")),
        };
        let interval = match m.get("interval") {
            None | Some(Value::Null) => None,
            Some(v) => match v.as_number() {
                Some(n) if n.is_finite() && n > 0.0 => Some(n),
                _ => return Err(format!("the shared memory's interval is {v:?}, not seconds above 0")),
            },
        };
        Ok(SharedSource { store, element, items: texts("items")?, properties: texts("properties")?, notifier, interval })
    }
}

/// What a reader records of the rows it has passed on.
struct Rows {
    element: ElementType,
    /// A row's bytes.
    bytes: usize,
    /// Every row's bytes as last passed on.
    last: Vec<u8>,
    /// Which rows have been passed on.
    given: Vec<bool>,
}

impl Rows {
    /// Passes on row `i`, gathered in `changed`, when `row` differs from
    /// what was last passed on for it.
    fn take(&mut self, i: u32, row: &[u8], changed: &mut Vec<(u32, Vec<f64>)>) {
        let at = i as usize * self.bytes;
        let held = &mut self.last[at..at + self.bytes];
        let given = &mut self.given[i as usize];
        if *given && held == row {
            return;
        }
        held.copy_from_slice(row);
        *given = true;
        changed.push((i, values(self.element, row)));
    }
}

/// A source opened.
enum Opened {
    Slab(RawSlab),
    Vec(RawVec),
    Cell(RawCell),
    File { file: std::fs::File, path: String, offset: u64, sequence: u64, end: u64 },
}

/// A shared-memory source opened for reading, with what it last passed on
/// of each row.
pub struct Reader {
    opened: Opened,
    rows: Rows,
    /// The most rows the source has.
    count: u32,
    /// A slab's slot versions as last read, zero for a slot never written.
    versions: Vec<u32>,
    /// A cell's version, or a file's counter, as last read.
    generation: u64,
    /// A file's counter as last seen odd, and since when.
    odd: Option<(u64, Instant)>,
}

impl Reader {
    /// Opens `source` for reading: a SubEtha structure read-only, with the
    /// layout the source names, which must be the one its file records, and
    /// a file by its path. Each slot, or each row of a cell or file, has one
    /// value of the source's type for each of its properties.
    pub fn open(source: &SharedSource) -> Result<Reader, String> {
        let columns = source.properties.len();
        let size = usize::try_from(source.element.size()).map_err(|e| format!("a {} is no size this machine takes: {e}", source.element.name()))?;
        let row = columns.checked_mul(size).filter(|r| *r > 0).ok_or_else(|| String::from("the shared memory sets no property, so its rows have no values"))?;
        let at = |n: i64, what: &str| usize::try_from(n).map_err(|e| format!("the shared memory's {what}, {n}, is no size this machine takes: {e}"));
        let unsigned = |n: i64, what: &str| u64::try_from(n).map_err(|e| format!("the shared memory's {what}, {n}, is no place in a file: {e}"));
        let layout = |slot: usize, alignment: i64, tag: u64| -> Result<ElementLayout, String> { Ok(ElementLayout { slot_size: slot, alignment: at(alignment, "alignment")?, tag }) };
        let slots = |slot: usize, what: &str| {
            if slot == row {
                Ok(())
            } else {
                Err(format!("each {what} has {slot} bytes, where {columns} values of {} take {row}", source.element.name()))
            }
        };
        let path = source.store.path();
        let (opened, count) = match &source.store {
            Store::Slab { capacity, slot, alignment, tag, .. } => {
                let (capacity, slot) = (at(*capacity, "capacity")?, at(*slot, "slot size")?);
                slots(slot, "slot of the slab")?;
                let slab = RawSlab::open_read_only(path, capacity, layout(slot, *alignment, *tag)?).map_err(|e| format!("the slab at {path} cannot be opened: {e}"))?;
                (Opened::Slab(slab), capacity)
            }
            Store::Vec { capacity, writers, slot, alignment, tag, .. } => {
                let (capacity, writers, slot) = (at(*capacity, "capacity")?, at(*writers, "writer table")?, at(*slot, "element size")?);
                slots(slot, "element of the vec")?;
                let elements = RawVec::open_read_only(path, capacity, writers, layout(slot, *alignment, *tag)?).map_err(|e| format!("the vec at {path} cannot be opened: {e}"))?;
                (Opened::Vec(elements), capacity)
            }
            Store::Cell { size, .. } => {
                let size = at(*size, "value size")?;
                if size % row != 0 {
                    return Err(format!("the cell has {size} bytes, which is no whole number of rows of {row} bytes"));
                }
                let cell = RawCell::open(path, size).map_err(|e| format!("the cell at {path} cannot be opened: {e}"))?;
                (Opened::Cell(cell), size / row)
            }
            Store::File { offset, length, sequence, .. } => {
                let length = at(*length, "length")?;
                if length % columns != 0 {
                    return Err(format!("the file has {length} values, which is no whole number of rows of {columns}"));
                }
                let (offset, sequence) = (unsigned(*offset, "offset")?, unsigned(*sequence, "sequence")?);
                let bytes = u64::try_from(length * size).map_err(|e| format!("the file's values are no size this machine takes: {e}"))?;
                let end = (offset + bytes).max(sequence + 8);
                let file = std::fs::File::open(path).map_err(|e| format!("the file {path} cannot be opened: {e}"))?;
                (Opened::File { file, path: path.to_string(), offset, sequence, end }, length / columns)
            }
        };
        let count = u32::try_from(count).map_err(|e| format!("the shared memory has {count} rows, more than a widget reads: {e}"))?;
        let rows = Rows { element: source.element, bytes: row, last: vec![0; count as usize * row], given: vec![false; count as usize] };
        Ok(Reader { opened, rows, count, versions: vec![0; count as usize], generation: 0, odd: None })
    }

    /// The rows that changed since the last read, each its number and its
    /// values: a slab's slots whose version moved, a vec's live elements, a
    /// cell's rows when its version moved, and a file's rows when its
    /// counter moved, each read whole. A slot or a file never written gives
    /// no rows. When the source cannot be read now, the result says why,
    /// and the rows last passed on stand.
    pub fn read(&mut self) -> Result<Vec<(u32, Vec<f64>)>, String> {
        let mut changed = Vec::new();
        let bytes = self.rows.bytes;
        match &self.opened {
            Opened::Slab(slab) => {
                let mut row = vec![0; bytes];
                for i in 0..self.count {
                    let slot = i as usize;
                    let version = slab.slot_version(slot).map_err(|e| format!("slot {i} of the slab cannot be read: {e}"))?;
                    if version == self.versions[slot] || version % 2 == 1 {
                        continue;
                    }
                    slab.get(slot, &mut row).map_err(|e| format!("slot {i} of the slab cannot be read: {e}"))?;
                    self.versions[slot] = version;
                    self.rows.take(i, &row, &mut changed);
                }
            }
            Opened::Vec(elements) => {
                let mut row = vec![0; bytes];
                for i in 0..self.count {
                    if !elements.get(i as usize, &mut row).map_err(|e| format!("element {i} of the vec cannot be read: {e}"))? {
                        break;
                    }
                    self.rows.take(i, &row, &mut changed);
                }
            }
            Opened::Cell(cell) => {
                let version = cell.version();
                if version != 0 && version != self.generation {
                    let mut all = vec![0; self.count as usize * bytes];
                    self.generation = cell.load(&mut all).map_err(|e| format!("the cell cannot be read: {e}"))?;
                    for (i, row) in (0..self.count).zip(all.chunks_exact(bytes)) {
                        self.rows.take(i, row, &mut changed);
                    }
                }
            }
            Opened::File { file, path, offset, sequence, end } => {
                let failed = |e: std::io::Error| {
                    if e.kind() == std::io::ErrorKind::UnexpectedEof {
                        format!("the file {path} is shorter than its values and counter, which end at byte {end}; the values last read stay")
                    } else {
                        format!("the file {path} cannot be read: {e}")
                    }
                };
                for _ in 0..FILE_TRIES {
                    let before = counter(file, *sequence).map_err(failed)?;
                    if before == 0 {
                        break;
                    }
                    if before % 2 == 1 {
                        match self.odd {
                            Some((seen, since)) if seen == before => {
                                if since.elapsed() >= HELD_ODD {
                                    return Err(format!("the counter of {path} has stayed odd at {before} for a second or more, a write the writer has not finished; the values last read stay"));
                                }
                            }
                            _ => self.odd = Some((before, Instant::now())),
                        }
                        break;
                    }
                    self.odd = None;
                    if before == self.generation {
                        break;
                    }
                    let mut all = vec![0; self.count as usize * bytes];
                    read_at(file, &mut all, *offset).map_err(failed)?;
                    if counter(file, *sequence).map_err(failed)? != before {
                        continue;
                    }
                    self.generation = before;
                    for (i, row) in (0..self.count).zip(all.chunks_exact(bytes)) {
                        self.rows.take(i, row, &mut changed);
                    }
                    break;
                }
            }
        }
        Ok(changed)
    }
}

/// The 64-bit counter of `file` at byte `at`, in the byte order of every
/// machine PWRS-GUI runs on, low byte first.
fn counter(file: &std::fs::File, at: u64) -> std::io::Result<u64> {
    let mut word = [0; 8];
    read_at(file, &mut word, at)?;
    Ok(u64::from_le_bytes(word))
}

/// Fills `buf` from `file` at byte `at`; a file that ends first is an
/// `UnexpectedEof`.
#[cfg(unix)]
fn read_at(file: &std::fs::File, buf: &mut [u8], at: u64) -> std::io::Result<()> {
    use std::os::unix::fs::FileExt;
    file.read_exact_at(buf, at)
}

/// Fills `buf` from `file` at byte `at`; a file that ends first is an
/// `UnexpectedEof`.
#[cfg(windows)]
fn read_at(file: &std::fs::File, buf: &mut [u8], at: u64) -> std::io::Result<()> {
    use std::os::windows::fs::FileExt;
    let mut done = 0;
    while done < buf.len() {
        match file.seek_read(&mut buf[done..], at + done as u64)? {
            0 => return Err(std::io::Error::from(std::io::ErrorKind::UnexpectedEof)),
            n => done += n,
        }
    }
    Ok(())
}

/// The numbers in `bytes`, values of `element` end to end, each low byte
/// first.
fn values(element: ElementType, bytes: &[u8]) -> Vec<f64> {
    let size = element.size() as usize;
    bytes.chunks_exact(size).map(|b| number(element, b)).collect()
}

/// The number in `b`, one value of `element`.
fn number(element: ElementType, b: &[u8]) -> f64 {
    let mut word = [0u8; 8];
    word[..b.len()].copy_from_slice(b);
    let [w0, w1, w2, w3, ..] = word;
    match element {
        ElementType::Float64 => f64::from_le_bytes(word),
        ElementType::Float32 => f64::from(f32::from_le_bytes([w0, w1, w2, w3])),
        ElementType::Float16 => half(u16::from_le_bytes([w0, w1])),
        ElementType::Int8 => f64::from(i8::from_le_bytes([w0])),
        ElementType::UInt8 => f64::from(w0),
        ElementType::Int16 => f64::from(i16::from_le_bytes([w0, w1])),
        ElementType::UInt16 => f64::from(u16::from_le_bytes([w0, w1])),
        ElementType::Int32 => f64::from(i32::from_le_bytes([w0, w1, w2, w3])),
        ElementType::UInt32 => f64::from(u32::from_le_bytes([w0, w1, w2, w3])),
        ElementType::Int64 => i64::from_le_bytes(word) as f64,
        ElementType::UInt64 => u64::from_le_bytes(word) as f64,
    }
}

/// The value of an IEEE 754 half-precision float's `bits`.
fn half(bits: u16) -> f64 {
    let sign = if bits & 0x8000 == 0 { 1.0 } else { -1.0 };
    let exponent = i32::from((bits >> 10) & 0x1f);
    let fraction = f64::from(bits & 0x03ff);
    match exponent {
        0 => sign * fraction * 2f64.powi(-24),
        31 if bits & 0x03ff == 0 => sign * f64::INFINITY,
        31 => f64::NAN,
        e => sign * (1.0 + fraction / 1024.0) * 2f64.powi(e - 15),
    }
}

/// The changes `rows`, read from the shared memory `source` of the canvas
/// `node`, make to its items, which are on `board`: row i sets the keys of
/// the i-th item the source names, column j key j, as numbers. A row past
/// the items, or for a name not in the scene now, changes nothing.
/// Each change is checked once, by the board as it takes it, so a row an
/// item cannot take is refused there, the others taken.
pub fn canvas_ops(node: &Node, board: Option<&Board>, source: &SharedSource, rows: &[(u32, Vec<f64>)]) -> Result<Vec<PatchOp>, String> {
    let board = board.ok_or_else(|| String::from("the canvas's items cannot be read, so its shared memory changes none of them"))?;
    let places = places(board, &source.items);
    Ok(rows
        .iter()
        .filter_map(|(row, values)| {
            let name = places.get(*row as usize)?.as_ref()?;
            let props: BTreeMap<String, Value> = source.properties.iter().cloned().zip(values.iter().map(|v| Value::Float(*v))).collect();
            Some(PatchOp::SetItem { node: node.id, item: name.clone(), props })
        })
        .collect())
}

/// The item each row goes to, by row, as its name: each name of `items`
/// takes one place, empty when the scene has no such item now, and each
/// wildcard a place for each item it matches, in the scene's order; an item
/// matched twice stays in its first place, and a name matching it again
/// takes an empty one.
fn places(board: &Board, items: &[String]) -> Vec<Option<String>> {
    let mut out = Vec::new();
    let mut taken: HashSet<ItemId> = HashSet::new();
    for entry in items {
        if is_pattern(entry) {
            for id in board.all() {
                if let Some(name) = board.entry(id).and_then(|e| e.item.name.clone())
                    && like(entry, &name)
                    && taken.insert(id)
                {
                    out.push(Some(name));
                }
            }
        } else {
            let found = board.find(entry).filter(|id| taken.insert(*id)).and_then(|id| board.entry(id).and_then(|e| e.item.name.clone()));
            out.push(found);
        }
    }
    out
}

/// Whether `entry` is a wildcard: it has `*`, `?` or `[` in it.
fn is_pattern(entry: &str) -> bool {
    entry.contains(['*', '?', '['])
}

/// Whether `name` matches the wildcard `pattern` as PowerShell matches one,
/// without regard to case: `*` any run of characters, `?` any one, and
/// `[...]` one of a set of characters and ranges.
fn like(pattern: &str, name: &str) -> bool {
    let p: Vec<char> = pattern.chars().map(|c| c.to_ascii_lowercase()).collect();
    let n: Vec<char> = name.chars().map(|c| c.to_ascii_lowercase()).collect();
    matches_from(&p, &n)
}

fn matches_from(p: &[char], n: &[char]) -> bool {
    match p.split_first() {
        None => n.is_empty(),
        Some(('*', rest)) => (0..=n.len()).any(|i| matches_from(rest, &n[i..])),
        Some(('?', rest)) => !n.is_empty() && matches_from(rest, &n[1..]),
        Some(('[', rest)) => match (rest.iter().position(|c| *c == ']'), n.split_first()) {
            (Some(end), Some((c, tail))) => in_set(&rest[..end], *c) && matches_from(&rest[end + 1..], tail),
            (Some(_), None) => false,
            (None, _) => n.first() == Some(&'[') && matches_from(rest, &n[1..]),
        },
        Some((c, rest)) => n.first() == Some(c) && matches_from(rest, &n[1..]),
    }
}

/// Whether `c` is among `set`, a wildcard's bracketed characters and
/// ranges such as `a-z`.
fn in_set(set: &[char], c: char) -> bool {
    let mut i = 0;
    while i < set.len() {
        if i + 2 < set.len() && set[i + 1] == '-' {
            if (set[i]..=set[i + 2]).contains(&c) {
                return true;
            }
            i += 3;
        } else {
            if set[i] == c {
                return true;
            }
            i += 1;
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Kind;

    /// A folder for a test's files, removed with what is in it when the
    /// test ends, whether it passed or not.
    struct Folder(std::path::PathBuf);

    impl Folder {
        fn new(name: &str) -> Folder {
            let dir = std::env::temp_dir().join(format!("pwrs-gui-shared-{name}-{}", std::process::id()));
            std::fs::create_dir_all(&dir).expect("a folder for the test's files");
            Folder(dir)
        }

        fn path(&self, file: &str) -> String {
            self.0.join(file).display().to_string()
        }
    }

    impl Drop for Folder {
        fn drop(&mut self) {
            if let Err(e) = std::fs::remove_dir_all(&self.0) {
                eprintln!("the test's folder {} was not removed: {e}", self.0.display());
            }
        }
    }

    fn source(store: Store, element: ElementType, properties: &[&str]) -> SharedSource {
        SharedSource {
            store,
            element,
            items: vec![String::from("*")],
            properties: properties.iter().map(|p| p.to_string()).collect(),
            notifier: None,
            interval: Some(0.05),
        }
    }

    /// Pairs of 64-bit floats as a slot's or a file's bytes.
    fn pairs(values: &[(f64, f64)]) -> Vec<u8> {
        values.iter().flat_map(|(x, y)| x.to_le_bytes().into_iter().chain(y.to_le_bytes())).collect()
    }

    #[test]
    fn a_source_crosses_as_its_property_and_back() {
        let stores = [
            Store::Slab { path: String::from("/tmp/s"), capacity: 64, slot: 16, alignment: 16, tag: u64::MAX },
            Store::Vec { path: String::from("/tmp/v"), capacity: 8, writers: 4, slot: 16, alignment: 1, tag: 0 },
            Store::Cell { path: String::from("/tmp/c"), size: 32 },
            Store::File { path: String::from("/tmp/f"), offset: 64, length: 100, sequence: 0 },
        ];
        for store in stores {
            let mut s = source(store, ElementType::Float64, &["x", "y"]);
            s.notifier = Some(String::from("/tmp/n"));
            let node = Node::new(3, Kind::Canvas).prop("shared", s.value());
            assert_eq!(SharedSource::of(&node).expect("a source named").expect("named rightly"), s);
        }
        assert!(SharedSource::of(&Node::new(3, Kind::Canvas)).is_none());
    }

    #[test]
    fn a_shared_property_that_names_no_source_rightly_says_why() {
        let why = |v: Value| SharedSource::of(&Node::new(3, Kind::Canvas).prop("shared", v)).expect("a source named").expect_err("named wrongly");
        let mut m = match source(Store::Cell { path: String::from("/tmp/c"), size: 16 }, ElementType::Float32, &["x"]).value() {
            Value::Map(m) => m,
            other => panic!("a source's property is a map, not {other:?}"),
        };
        m.insert(String::from("type"), Value::Str(String::from("Float128")));
        assert_eq!(why(Value::Map(m.clone())), "Float128 is no element type");
        m.insert(String::from("type"), Value::Str(String::from("Float32")));
        m.insert(String::from("kind"), Value::Str(String::from("ring")));
        assert_eq!(why(Value::Map(m.clone())), "ring is no kind of shared memory a widget reads");
        m.insert(String::from("kind"), Value::Str(String::from("cell")));
        m.insert(String::from("interval"), Value::Float(0.0));
        assert!(why(Value::Map(m)).contains("not seconds above 0"));
        assert!(why(Value::Int(4)).contains("not a map"));
    }

    #[test]
    fn every_element_type_reads_as_its_number() {
        let read = |t: ElementType, bytes: &[u8]| values(t, bytes);
        assert_eq!(read(ElementType::Float64, &2.5f64.to_le_bytes()), vec![2.5]);
        assert_eq!(read(ElementType::Float32, &[(-1.5f32).to_le_bytes(), 4.0f32.to_le_bytes()].concat()), vec![-1.5, 4.0]);
        assert_eq!(read(ElementType::Int8, &[0xff, 0x7f]), vec![-1.0, 127.0]);
        assert_eq!(read(ElementType::UInt8, &[0xff]), vec![255.0]);
        assert_eq!(read(ElementType::Int16, &(-300i16).to_le_bytes()), vec![-300.0]);
        assert_eq!(read(ElementType::UInt16, &60_000u16.to_le_bytes()), vec![60_000.0]);
        assert_eq!(read(ElementType::Int32, &(-70_000i32).to_le_bytes()), vec![-70_000.0]);
        assert_eq!(read(ElementType::UInt32, &4_000_000_000u32.to_le_bytes()), vec![4_000_000_000.0]);
        assert_eq!(read(ElementType::Int64, &(-5i64).to_le_bytes()), vec![-5.0]);
        assert_eq!(read(ElementType::UInt64, &7u64.to_le_bytes()), vec![7.0]);
        let halves = [0x3c00u16, 0xc000, 0x3800, 0x7bff, 0x0001, 0x7c00, 0xfc00];
        let bytes: Vec<u8> = halves.iter().flat_map(|h| h.to_le_bytes()).collect();
        assert_eq!(read(ElementType::Float16, &bytes), vec![1.0, -2.0, 0.5, 65504.0, 2f64.powi(-24), f64::INFINITY, f64::NEG_INFINITY]);
        assert!(half(0x7e00).is_nan());
    }

    #[test]
    fn a_wildcard_matches_names_as_powershell_does_without_regard_to_case() {
        assert!(like("Dot*", "dot3"));
        assert!(like("*", ""));
        assert!(like("?ox", "Box"));
        assert!(!like("?ox", "Bx"));
        assert!(like("[a-c]at", "Bat"));
        assert!(!like("[a-c]at", "dat"));
        assert!(like("[xyz]*", "Zone"));
        assert!(like("Moon*", "MoonB"));
        assert!(!like("Moon*", "Earth"));
        assert!(like("a*b*c", "aXXbYYc"));
        assert!(!like("a*b*c", "aXXbYY"));
    }

    /// A canvas of Rects named `names`, each 10 by 10 at the origin, and the
    /// board of its items.
    fn canvas(names: &[&str]) -> (Node, Board) {
        let items = names
            .iter()
            .map(|n| {
                let mut m = BTreeMap::from([(String::from("kind"), Value::Str(String::from("Rect"))), (String::from("name"), Value::Str(n.to_string()))]);
                for key in ["x", "y", "width", "height"] {
                    m.insert(key.to_string(), Value::Int(10));
                }
                Value::Map(m)
            })
            .collect();
        let node = Node::new(3, Kind::Canvas).prop("items", Value::List(items));
        let board = Board::of(&node).expect("the items read");
        (node, board)
    }

    fn placed(board: &Board, items: &[&str]) -> Vec<Option<String>> {
        let items: Vec<String> = items.iter().map(|i| i.to_string()).collect();
        places(board, &items)
    }

    #[test]
    fn rows_go_to_the_entries_in_order_each_wildcard_in_the_scenes_order() {
        let (_, board) = canvas(&["Mars", "MoonB", "Earth", "MoonA", "Sun"]);
        let some = |n: &str| Some(n.to_string());
        assert_eq!(placed(&board, &["Sun", "Moon*", "Earth"]), vec![some("Sun"), some("MoonB"), some("MoonA"), some("Earth")]);
        assert_eq!(placed(&board, &["sun", "Pluto", "earth"]), vec![some("Sun"), None, some("Earth")], "a name not in the scene still takes its place");
        assert_eq!(placed(&board, &["Earth", "*"]), vec![some("Earth"), some("Mars"), some("MoonB"), some("MoonA"), some("Sun")], "an item matched twice stays in its first place");
        assert_eq!(placed(&board, &["Earth", "Earth", "Sun"]), vec![some("Earth"), None, some("Sun")], "a name matching an item again takes an empty place");
    }

    #[test]
    fn a_canvas_takes_its_rows_as_changes_to_the_items_they_go_to() {
        let (node, board) = canvas(&["A", "B"]);
        let mut s = source(Store::Cell { path: String::new(), size: 16 }, ElementType::Float64, &["x", "width"]);
        s.items = vec![String::from("A"), String::from("Pluto"), String::from("B")];
        let set = |item: &str, x: f64, width: f64| PatchOp::SetItem {
            node: 3,
            item: item.to_string(),
            props: BTreeMap::from([(String::from("x"), Value::Float(x)), (String::from("width"), Value::Float(width))]),
        };
        let rows = [(0, vec![5.0, 20.0]), (1, vec![6.0, 30.0]), (2, vec![7.0, 40.0]), (3, vec![8.0, 50.0])];
        let ops = canvas_ops(&node, Some(&board), &s, &rows).expect("the rows go to the items");
        assert_eq!(ops, vec![set("A", 5.0, 20.0), set("B", 7.0, 40.0)], "a row for a name not in the scene, or past the items, changes nothing");
        assert!(canvas_ops(&node, None, &s, &rows).expect_err("no board").contains("cannot be read"));
    }

    #[test]
    fn a_slab_is_read_by_the_slots_whose_version_moved_never_written_ones_passed_over() {
        let dir = Folder::new("slab");
        let path = dir.path("rows");
        let layout = ElementLayout { slot_size: 16, alignment: 1, tag: 0 };
        let writer = RawSlab::create(&path, 4, layout).expect("the slab");
        let mut reader = Reader::open(&source(Store::Slab { path: path.clone(), capacity: 4, slot: 16, alignment: 1, tag: 0 }, ElementType::Float64, &["x", "y"])).expect("the slab opened");
        assert_eq!(reader.read().expect("read"), Vec::new(), "no slot is written yet");
        writer.set(1, &pairs(&[(1.0, 2.0)])).expect("slot 1 written");
        assert_eq!(reader.read().expect("read"), vec![(1, vec![1.0, 2.0])]);
        assert_eq!(reader.read().expect("read"), Vec::new(), "nothing moved since");
        writer.set(1, &pairs(&[(1.0, 2.0)])).expect("slot 1 written again alike");
        assert_eq!(reader.read().expect("read"), Vec::new(), "a write of the same values changes nothing");
        writer.set(3, &pairs(&[(3.0, 4.0)])).expect("slot 3 written");
        writer.set(1, &pairs(&[(5.0, 6.0)])).expect("slot 1 written anew");
        assert_eq!(reader.read().expect("read"), vec![(1, vec![5.0, 6.0]), (3, vec![3.0, 4.0])]);
        let other = Reader::open(&source(Store::Slab { path, capacity: 4, slot: 16, alignment: 1, tag: 0 }, ElementType::Float32, &["x", "y"])).err().expect("a slot of another size is refused");
        assert!(other.contains("each slot of the slab has 16 bytes, where 2 values of Float32 take 8"), "{other}");
    }

    #[test]
    fn a_vec_is_read_by_its_live_elements() {
        let dir = Folder::new("vec");
        let path = dir.path("rows");
        let layout = ElementLayout { slot_size: 16, alignment: 1, tag: 0 };
        let writer = RawVec::create(&path, 8, 1, layout).expect("the vec");
        let mut reader = Reader::open(&source(Store::Vec { path, capacity: 8, writers: 1, slot: 16, alignment: 1, tag: 0 }, ElementType::Float64, &["x", "y"])).expect("the vec opened");
        writer.push_back(&pairs(&[(1.0, 1.0)])).expect("pushed");
        writer.push_back(&pairs(&[(2.0, 2.0)])).expect("pushed");
        assert_eq!(reader.read().expect("read"), vec![(0, vec![1.0, 1.0]), (1, vec![2.0, 2.0])]);
        writer.set(0, &pairs(&[(9.0, 9.0)])).expect("set");
        assert_eq!(reader.read().expect("read"), vec![(0, vec![9.0, 9.0])]);
    }

    #[test]
    fn a_slab_and_a_vec_are_opened_with_the_layout_they_were_made_with_and_another_is_refused() {
        let dir = Folder::new("layout");
        let (slab_path, vec_path) = (dir.path("slab"), dir.path("vec"));
        let layout = ElementLayout { slot_size: 16, alignment: 64, tag: 7 };
        let slab = RawSlab::create(&slab_path, 2, layout).expect("the slab");
        let elements = RawVec::create(&vec_path, 2, 3, layout).expect("the vec");
        slab.set(0, &pairs(&[(1.0, 2.0)])).expect("slot 0 written");
        elements.push_back(&pairs(&[(3.0, 4.0)])).expect("pushed");
        let open_vec = |path: &str, writers: i64, alignment: i64, tag: u64| {
            Reader::open(&source(Store::Vec { path: path.to_string(), capacity: 2, writers, slot: 16, alignment, tag }, ElementType::Float64, &["x", "y"]))
        };
        let open = |path: &str, vec: bool, alignment: i64, tag: u64| {
            if vec {
                return open_vec(path, 3, alignment, tag);
            }
            Reader::open(&source(Store::Slab { path: path.to_string(), capacity: 2, slot: 16, alignment, tag }, ElementType::Float64, &["x", "y"]))
        };
        for writers in [1, 4] {
            let table = open_vec(&vec_path, writers, 64, 7).err().expect("another writer table is refused");
            assert!(table.contains("writer slots is 3"), "{table}");
        }
        let mut from_slab = open(&slab_path, false, 64, 7).expect("the slab opened with its layout");
        assert_eq!(from_slab.read().expect("read"), vec![(0, vec![1.0, 2.0])]);
        let mut from_vec = open(&vec_path, true, 64, 7).expect("the vec opened with its layout");
        assert_eq!(from_vec.read().expect("read"), vec![(0, vec![3.0, 4.0])]);
        for (path, vec) in [(&slab_path, false), (&vec_path, true)] {
            let tag = open(path, vec, 64, 0).err().expect("another tag is refused");
            assert!(tag.contains("the file's layout tag is 7 where 0 was asked for"), "{tag}");
            let alignment = open(path, vec, 1, 7).err().expect("an alignment that lays the slots out otherwise is refused");
            assert!(alignment.contains("cannot be opened") && alignment.contains("was asked for"), "{alignment}");
        }
    }

    #[test]
    fn a_cell_is_read_whole_when_its_version_moves() {
        let dir = Folder::new("cell");
        let path = dir.path("rows");
        let writer = RawCell::create(&path, 32).expect("the cell");
        let mut reader = Reader::open(&source(Store::Cell { path, size: 32 }, ElementType::Float64, &["x", "y"])).expect("the cell opened");
        assert_eq!(reader.read().expect("read"), Vec::new(), "a cell never written gives no rows");
        writer.set(&pairs(&[(1.0, 2.0), (3.0, 4.0)])).expect("set");
        assert_eq!(reader.read().expect("read"), vec![(0, vec![1.0, 2.0]), (1, vec![3.0, 4.0])]);
        writer.set(&pairs(&[(1.0, 2.0), (7.0, 8.0)])).expect("set");
        assert_eq!(reader.read().expect("read"), vec![(1, vec![7.0, 8.0])], "only the row that changed");
    }

    #[test]
    fn a_file_is_read_only_at_an_even_counter_that_moved_and_says_when_it_is_short() {
        let dir = Folder::new("file");
        let path = dir.path("rows.bin");
        let write = |counter: u64, rows: &[(f64, f64)]| {
            let mut bytes = counter.to_le_bytes().to_vec();
            bytes.extend(pairs(rows));
            std::fs::write(&path, bytes).expect("the file written");
        };
        write(0, &[(1.0, 1.0), (2.0, 2.0)]);
        let mut reader = Reader::open(&source(Store::File { path: path.clone(), offset: 8, length: 4, sequence: 0 }, ElementType::Float64, &["x", "y"])).expect("the file opened");
        assert_eq!(reader.read().expect("read"), Vec::new(), "a counter of 0 has had no write");
        write(2, &[(1.0, 1.0), (2.0, 2.0)]);
        assert_eq!(reader.read().expect("read"), vec![(0, vec![1.0, 1.0]), (1, vec![2.0, 2.0])]);
        write(3, &[(5.0, 5.0), (2.0, 2.0)]);
        assert_eq!(reader.read().expect("read"), Vec::new(), "an odd counter is a write under way");
        write(4, &[(5.0, 5.0), (2.0, 2.0)]);
        assert_eq!(reader.read().expect("read"), vec![(0, vec![5.0, 5.0])]);
        std::fs::write(&path, 6u64.to_le_bytes()).expect("the file cut short");
        let short = reader.read().expect_err("a file shorter than its values is told");
        assert!(short.contains("is shorter than its values and counter, which end at byte 40"), "{short}");
    }
}
