//! Photoshop's action descriptors, in which tagged blocks such as layer
//! effects (lfx2), fills (SoCo, GdFl, PtFl) and adjustments keep their
//! settings: a name, a class, then items, each a key and a typed value, as
//! Adobe's Photoshop File Formats Specification lays them out, with the
//! undocumented types psd-tools reads (UnFl, ObAr, Pth ). A key or class
//! stored with length 0 is 4 bytes, and is read as their text.

use crate::psd::Reader;

/// A descriptor's value.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Value {
    Descriptor(Descriptor),
    List(Vec<Value>),
    Double(f64),
    /// A number and its unit's 4-byte code, such as #Pxl or #Prc.
    Unit(String, f64),
    Units(String, Vec<f64>),
    Text(String),
    /// An enumeration's type and value.
    Enum(String, String),
    Integer(i64),
    Bool(bool),
    Class(String),
    Data(Vec<u8>),
    /// A reference, whose items the module does not read.
    Reference,
}

/// A descriptor: its class and items, in order.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Descriptor {
    pub(crate) class: String,
    pub(crate) items: Vec<(String, Value)>,
}

impl Descriptor {
    /// The value of the item `key`.
    pub(crate) fn get(&self, key: &str) -> Option<&Value> {
        self.items.iter().find(|(k, _)| k == key).map(|(_, v)| v)
    }

    /// The boolean item `key` holds.
    pub(crate) fn flag(&self, key: &str) -> Option<bool> {
        match self.get(key) {
            Some(Value::Bool(v)) => Some(*v),
            _ => None,
        }
    }
}

/// A descriptor block: a 4-byte version, which is 16, then a descriptor.
pub(crate) fn versioned(data: &[u8]) -> Result<Descriptor, String> {
    let mut r = Reader::new(data);
    let version = r.number(4)?;
    if version != 16 {
        return Err(format!("its descriptor's version is {version}, where the specification gives 16"));
    }
    descriptor(&mut r)
}

/// A descriptor at the reader's place.
pub(crate) fn descriptor(r: &mut Reader) -> Result<Descriptor, String> {
    text(r)?;
    let class = key(r)?;
    let count = r.length(4)?;
    let mut items = Vec::with_capacity(count.min(r.left()));
    for _ in 0..count {
        let name = key(r)?;
        let kind = r.four()?;
        items.push((name, value(r, kind)?));
    }
    Ok(Descriptor { class, items })
}

/// A key or class ID: a 4-byte length, and that many bytes, or 4 where the
/// length is 0.
fn key(r: &mut Reader) -> Result<String, String> {
    let n = r.length(4)?;
    Ok(String::from_utf8_lossy(r.take(if n == 0 { 4 } else { n })?).into_owned())
}

/// A Unicode string: a 4-byte count of UTF-16 code units and that many,
/// big-endian, a trailing NUL dropped.
fn text(r: &mut Reader) -> Result<String, String> {
    let n = r.length(4)?;
    let units: Vec<u16> = r.take(n.checked_mul(2).ok_or("a string does not fit in memory")?)?.as_chunks::<2>().0.iter().map(|c| u16::from_be_bytes(*c)).collect();
    Ok(String::from_utf16_lossy(&units).trim_end_matches('\0').to_string())
}

fn double(r: &mut Reader) -> Result<f64, String> {
    Ok(f64::from_bits(r.number(8)?))
}

/// A value of the type `kind`.
fn value(r: &mut Reader, kind: [u8; 4]) -> Result<Value, String> {
    Ok(match &kind {
        b"Objc" | b"GlbO" => Value::Descriptor(descriptor(r)?),
        b"ObAr" => {
            r.number(4)?;
            Value::Descriptor(descriptor(r)?)
        }
        b"VlLs" => {
            let count = r.length(4)?;
            let mut values = Vec::with_capacity(count.min(r.left()));
            for _ in 0..count {
                let kind = r.four()?;
                values.push(value(r, kind)?);
            }
            Value::List(values)
        }
        b"obj " => {
            let count = r.length(4)?;
            for _ in 0..count {
                let kind = r.four()?;
                reference_item(r, kind)?;
            }
            Value::Reference
        }
        b"doub" => Value::Double(double(r)?),
        b"UntF" => {
            let unit = String::from_utf8_lossy(&r.four()?).into_owned();
            Value::Unit(unit, double(r)?)
        }
        b"UnFl" => {
            let unit = String::from_utf8_lossy(&r.four()?).into_owned();
            let count = r.length(4)?;
            let mut values = Vec::with_capacity(count.min(r.left()));
            for _ in 0..count {
                values.push(double(r)?);
            }
            Value::Units(unit, values)
        }
        b"TEXT" => Value::Text(text(r)?),
        b"enum" => {
            let kind = key(r)?;
            Value::Enum(kind, key(r)?)
        }
        b"long" => Value::Integer(i64::from(r.signed32()?)),
        b"comp" => Value::Integer(i64::from_be_bytes(r.take(8)?.try_into().map_err(|e| format!("a large integer is cut short: {e}"))?)),
        b"bool" => Value::Bool(r.number(1)? != 0),
        b"type" | b"GlbC" | b"Clss" => {
            text(r)?;
            Value::Class(key(r)?)
        }
        b"alis" | b"tdta" | b"Pth " => {
            let n = r.length(4)?;
            Value::Data(r.take(n)?.to_vec())
        }
        other => return Err(format!("a descriptor holds a value of type {:?}, which the specification does not define", String::from_utf8_lossy(other))),
    })
}

/// A reference's item of the type `kind`, read past.
fn reference_item(r: &mut Reader, kind: [u8; 4]) -> Result<(), String> {
    match &kind {
        b"prop" => {
            text(r)?;
            key(r)?;
            key(r)?;
        }
        b"Clss" => {
            text(r)?;
            key(r)?;
        }
        b"Enmr" => {
            text(r)?;
            key(r)?;
            key(r)?;
            key(r)?;
        }
        b"rele" => {
            text(r)?;
            key(r)?;
            r.number(4)?;
        }
        b"Idnt" | b"indx" => {
            r.number(4)?;
        }
        b"name" => {
            text(r)?;
            key(r)?;
            text(r)?;
        }
        other => return Err(format!("a reference holds an item of type {:?}, which the specification does not define", String::from_utf8_lossy(other))),
    }
    Ok(())
}
