//! Conversions between PowerShell objects and protocol values. Runs on the
//! pipeline thread, where PowerShell objects live.

use pwrs::prelude::*;
use pwrs::sys::{
    PS_TYPE_BOOL, PS_TYPE_CHAR, PS_TYPE_DATETIME, PS_TYPE_DECIMAL, PS_TYPE_F32, PS_TYPE_F64, PS_TYPE_GUID, PS_TYPE_I16, PS_TYPE_I32, PS_TYPE_I64, PS_TYPE_I8,
    PS_TYPE_STRING, PS_TYPE_TIMESPAN, PS_TYPE_U16, PS_TYPE_U32, PS_TYPE_U64, PS_TYPE_U8,
};
use pwrs_gui_client::proto::{Node, Value, date};
use std::collections::BTreeMap;

/// A PowerShell value as a protocol value. Booleans, integers, floating
/// point, strings, arrays and hashtables stay as they are; a `UInt64`
/// above `Int64`'s range travels as its digits; a date as ISO 8601 with no
/// zone, as [`date::full`] writes it, its clock time unchanged; a time span in
/// .NET's constant format; any other object as its display string.
pub fn to_value(obj: &PsObject) -> PsResult<Value> {
    if obj.is_null() {
        return Ok(Value::Null);
    }
    let tag = obj.type_tag()?;
    Ok(match tag {
        PS_TYPE_BOOL => Value::Bool(bool::from_ps(obj)?),
        PS_TYPE_I8 | PS_TYPE_I16 | PS_TYPE_I32 | PS_TYPE_I64 | PS_TYPE_U8 | PS_TYPE_U16 | PS_TYPE_U32 => Value::Int(i64::from_ps(obj)?),
        PS_TYPE_U64 => {
            let n = u64::from_ps(obj)?;
            if n <= i64::MAX as u64 { Value::Int(n as i64) } else { Value::Str(n.to_string()) }
        }
        PS_TYPE_F32 | PS_TYPE_F64 | PS_TYPE_DECIMAL => Value::Float(f64::from_ps(obj)?),
        PS_TYPE_DATETIME => Value::Str(date::full(PsDateTime::from_ps(obj)?.ticks)),
        PS_TYPE_TIMESPAN => Value::Str(String::from_ps(&obj.call("ToString", &["c".into_ps()?])?)?),
        PS_TYPE_STRING | PS_TYPE_CHAR | PS_TYPE_GUID => Value::Str(String::from_ps(obj)?),
        _ => {
            let name = obj.type_name()?;
            if name == "System.Collections.Hashtable" || name == "System.Collections.Specialized.OrderedDictionary" {
                let table = PsReadOnlyTable::over(obj)?;
                let mut map = BTreeMap::new();
                for key in table.keys()? {
                    let v = to_value(&table.get(&key)?)?;
                    map.insert(key, v);
                }
                Value::Map(map)
            } else if name.ends_with("[]") || name == "System.Collections.ArrayList" || name.starts_with("System.Collections.Generic.List") {
                let items = Vec::<PsObject>::from_ps(obj)?;
                Value::List(items.iter().map(to_value).collect::<PsResult<Vec<Value>>>()?)
            } else {
                Value::Str(String::from_ps(obj)?)
            }
        }
    })
}

/// A protocol value as a PowerShell value: lists become object arrays,
/// maps hashtables, bytes a byte array, and strings stay strings.
pub fn from_value(v: &Value) -> PsResult<PsObject> {
    match v {
        Value::Null => Ok(PsObject::null()),
        Value::Bool(b) => (*b).into_ps(),
        Value::Int(i) => (*i).into_ps(),
        Value::Float(f) => (*f).into_ps(),
        Value::Str(s) => s.clone().into_ps(),
        Value::List(items) => items.iter().map(from_value).collect::<PsResult<Vec<PsObject>>>()?.into_ps(),
        Value::Map(m) => ordered(m.iter().map(|(k, v)| Ok((k.clone(), from_value(v)?))).collect::<PsResult<Vec<(String, PsObject)>>>()?),
        Value::Bytes(b) => b.to_vec().into_ps(),
    }
}

/// An ordered dictionary of `entries`, as PowerShell's `[ordered]` makes:
/// its keys stay in the order given, where a hashtable's order changes
/// from process to process under .NET's randomized string hashing, and
/// finds a key without regard to case.
pub fn ordered(entries: Vec<(String, PsObject)>) -> PsResult<PsObject> {
    let any_case = PsType::from_name("System.StringComparer").call_static("get_OrdinalIgnoreCase", &[])?;
    let table = PsType::from_name("System.Collections.Specialized.OrderedDictionary").new(&[any_case])?;
    for (k, v) in entries {
        table.call("Add", &[k.into_ps()?, v])?;
    }
    Ok(table)
}

/// A window's tree as nested ordered dictionaries: `Id`, `Kind`, `Name`,
/// `Properties`, by name, and `Children`, in that order.
pub fn from_node(node: &Node) -> PsResult<PsObject> {
    let props = node.props.iter().map(|(k, v)| Ok((k.clone(), from_value(v)?))).collect::<PsResult<Vec<(String, PsObject)>>>()?;
    let children = node.children.iter().map(from_node).collect::<PsResult<Vec<PsObject>>>()?;
    ordered(vec![
        (String::from("Id"), i64::from(node.id).into_ps()?),
        (String::from("Kind"), format!("{:?}", node.kind).into_ps()?),
        (String::from("Name"), node.name.clone().into_ps()?),
        (String::from("Properties"), ordered(props)?),
        (String::from("Children"), children.into_ps()?),
    ])
}
