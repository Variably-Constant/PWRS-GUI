//! Element types, and the conversions between PowerShell's arrays and the
//! bytes a GPU buffer holds.

use half::f16;
use pwrs::prelude::*;

use crate::error::invalid;

/// The scalar type of a buffer's elements.
#[psenum(name = "Wgpu.ElementType")]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, Hash)]
pub enum ElementType {
    /// 32-bit float: WGSL `f32`, .NET `float`.
    #[default]
    Float32,
    /// 64-bit float: WGSL `f64` (feature ShaderF64), .NET `double`.
    Float64,
    /// 16-bit float: WGSL `f16` (feature ShaderF16), read and written as .NET `float`.
    Float16,
    /// 8-bit signed integer, .NET `sbyte`.
    Int8,
    /// 8-bit unsigned integer, .NET `byte`.
    UInt8,
    /// 16-bit signed integer, .NET `short`.
    Int16,
    /// 16-bit unsigned integer, .NET `ushort`.
    UInt16,
    /// 32-bit signed integer: WGSL `i32`, .NET `int`.
    Int32,
    /// 32-bit unsigned integer: WGSL `u32`, .NET `uint`.
    UInt32,
    /// 64-bit signed integer: WGSL `i64` (feature ShaderInt64), .NET `long`.
    Int64,
    /// 64-bit unsigned integer: WGSL `u64` (feature ShaderInt64), .NET `ulong`.
    UInt64,
}

impl ElementType {
    /// Bytes per element.
    pub const fn size(self) -> u64 {
        match self {
            ElementType::Int8 | ElementType::UInt8 => 1,
            ElementType::Float16 | ElementType::Int16 | ElementType::UInt16 => 2,
            ElementType::Float32 | ElementType::Int32 | ElementType::UInt32 => 4,
            ElementType::Float64 | ElementType::Int64 | ElementType::UInt64 => 8,
        }
    }

    /// The WGSL scalar, for the types WGSL has.
    pub(crate) const fn wgsl(self) -> Option<&'static str> {
        match self {
            ElementType::Float32 => Some("f32"),
            ElementType::Float64 => Some("f64"),
            ElementType::Float16 => Some("f16"),
            ElementType::Int32 => Some("i32"),
            ElementType::UInt32 => Some("u32"),
            ElementType::Int64 => Some("i64"),
            ElementType::UInt64 => Some("u64"),
            ElementType::Int8 | ElementType::UInt8 | ElementType::Int16 | ElementType::UInt16 => None,
        }
    }

    pub(crate) const fn is_signed(self) -> bool {
        matches!(self, ElementType::Int8 | ElementType::Int16 | ElementType::Int32 | ElementType::Int64)
    }

    /// The smallest and largest value of an integer type; `None` for a
    /// float type.
    pub(crate) const fn range(self) -> Option<(i128, i128)> {
        match self {
            ElementType::Int8 => Some((i8::MIN as i128, i8::MAX as i128)),
            ElementType::UInt8 => Some((0, u8::MAX as i128)),
            ElementType::Int16 => Some((i16::MIN as i128, i16::MAX as i128)),
            ElementType::UInt16 => Some((0, u16::MAX as i128)),
            ElementType::Int32 => Some((i32::MIN as i128, i32::MAX as i128)),
            ElementType::UInt32 => Some((0, u32::MAX as i128)),
            ElementType::Int64 => Some((i64::MIN as i128, i64::MAX as i128)),
            ElementType::UInt64 => Some((0, u64::MAX as i128)),
            ElementType::Float16 | ElementType::Float32 | ElementType::Float64 => None,
        }
    }

    /// The element type of a .NET array type name such as `System.Single[]`.
    pub(crate) fn from_clr_array(type_name: &str) -> Option<ElementType> {
        Some(match type_name {
            "System.Single[]" => ElementType::Float32,
            "System.Double[]" => ElementType::Float64,
            "System.SByte[]" => ElementType::Int8,
            "System.Byte[]" => ElementType::UInt8,
            "System.Int16[]" => ElementType::Int16,
            "System.UInt16[]" => ElementType::UInt16,
            "System.Int32[]" => ElementType::Int32,
            "System.UInt32[]" => ElementType::UInt32,
            "System.Int64[]" => ElementType::Int64,
            "System.UInt64[]" => ElementType::UInt64,
            _ => return None,
        })
    }

    /// The type name of the .NET array a read of this type returns, such
    /// as `System.Single[]`, which a read of Float16 returns as well.
    pub(crate) const fn clr_array(self) -> &'static str {
        match self {
            ElementType::Float32 | ElementType::Float16 => "System.Single[]",
            ElementType::Float64 => "System.Double[]",
            ElementType::Int8 => "System.SByte[]",
            ElementType::UInt8 => "System.Byte[]",
            ElementType::Int16 => "System.Int16[]",
            ElementType::UInt16 => "System.UInt16[]",
            ElementType::Int32 => "System.Int32[]",
            ElementType::UInt32 => "System.UInt32[]",
            ElementType::Int64 => "System.Int64[]",
            ElementType::UInt64 => "System.UInt64[]",
        }
    }

    /// `bytes`, elements of this type, as elements of `to`, each converted
    /// by value as the module converts a typed array: a float truncates
    /// toward zero and saturates into an integer type, NaN becoming 0; an
    /// integer wraps into a narrower integer type; and a float type takes
    /// the nearest value it holds. A trailing partial element is left out.
    pub fn convert(self, bytes: &[u8], to: ElementType) -> Vec<u8> {
        let whole = bytes.len() - bytes.len() % self.size() as usize;
        convert(self, &bytes[..whole], to)
    }

    /// The element type of a naga scalar.
    pub(crate) fn from_scalar(scalar: naga::Scalar) -> Option<ElementType> {
        use naga::ScalarKind as K;
        Some(match (scalar.kind, scalar.width) {
            (K::Float, 2) => ElementType::Float16,
            (K::Float, 4) => ElementType::Float32,
            (K::Float, 8) => ElementType::Float64,
            (K::Sint, 4) => ElementType::Int32,
            (K::Uint, 4) => ElementType::UInt32,
            (K::Sint, 8) => ElementType::Int64,
            (K::Uint, 8) => ElementType::UInt64,
            _ => return None,
        })
    }
}

/// One number on its way between two element types.
#[derive(Clone, Copy)]
enum Num {
    Float(f64),
    Int(i128),
}

impl Num {
    fn as_f64(self) -> f64 {
        match self {
            Num::Float(v) => v,
            Num::Int(v) => v as f64,
        }
    }
}

fn decode(ty: ElementType, bytes: &[u8]) -> Num {
    let mut b = [0u8; 8];
    b[..bytes.len()].copy_from_slice(bytes);
    match ty {
        ElementType::Float32 => Num::Float(f32::from_le_bytes([b[0], b[1], b[2], b[3]]) as f64),
        ElementType::Float64 => Num::Float(f64::from_le_bytes(b)),
        ElementType::Float16 => Num::Float(f16::from_le_bytes([b[0], b[1]]).to_f64()),
        ElementType::Int8 => Num::Int(b[0] as i8 as i128),
        ElementType::UInt8 => Num::Int(b[0] as i128),
        ElementType::Int16 => Num::Int(i16::from_le_bytes([b[0], b[1]]) as i128),
        ElementType::UInt16 => Num::Int(u16::from_le_bytes([b[0], b[1]]) as i128),
        ElementType::Int32 => Num::Int(i32::from_le_bytes([b[0], b[1], b[2], b[3]]) as i128),
        ElementType::UInt32 => Num::Int(u32::from_le_bytes([b[0], b[1], b[2], b[3]]) as i128),
        ElementType::Int64 => Num::Int(i64::from_le_bytes(b) as i128),
        ElementType::UInt64 => Num::Int(u64::from_le_bytes(b) as i128),
    }
}

/// Writes `n` as one element of `ty`. A float becomes an integer type's
/// value by Rust's float-to-integer cast, which truncates toward zero and
/// saturates at the type's range (NaN becomes 0); an integer wraps into a
/// narrower integer type.
fn encode(ty: ElementType, n: Num, out: &mut Vec<u8>) {
    macro_rules! integer {
        ($t:ty) => {
            match n {
                Num::Float(v) => v as $t,
                Num::Int(v) => v as $t,
            }
        };
    }
    match ty {
        ElementType::Float32 => out.extend_from_slice(&(n.as_f64() as f32).to_le_bytes()),
        ElementType::Float64 => out.extend_from_slice(&n.as_f64().to_le_bytes()),
        ElementType::Float16 => out.extend_from_slice(&f16::from_f64(n.as_f64()).to_le_bytes()),
        ElementType::Int8 => out.extend_from_slice(&integer!(i8).to_le_bytes()),
        ElementType::UInt8 => out.push(integer!(u8)),
        ElementType::Int16 => out.extend_from_slice(&integer!(i16).to_le_bytes()),
        ElementType::UInt16 => out.extend_from_slice(&integer!(u16).to_le_bytes()),
        ElementType::Int32 => out.extend_from_slice(&integer!(i32).to_le_bytes()),
        ElementType::UInt32 => out.extend_from_slice(&integer!(u32).to_le_bytes()),
        ElementType::Int64 => out.extend_from_slice(&integer!(i64).to_le_bytes()),
        ElementType::UInt64 => out.extend_from_slice(&integer!(u64).to_le_bytes()),
    }
}

/// `bytes` of `from` elements as `to` elements, each converted by value:
/// floats truncate toward zero and saturate into integers, integers wrap
/// into narrower integers.
pub(crate) fn convert(from: ElementType, bytes: &[u8], to: ElementType) -> Vec<u8> {
    if from == to {
        return bytes.to_vec();
    }
    let step = from.size() as usize;
    let mut out = Vec::with_capacity(bytes.len() / step * to.size() as usize);
    for chunk in bytes.chunks_exact(step) {
        encode(to, decode(from, chunk), &mut out);
    }
    out
}

/// One untyped number as an element of `ty`, converted as PowerShell
/// converts it: a float type takes it as a double, and an integer type
/// takes it rounded half to even, exact to 64 bits. A number outside an
/// integer type's range is refused as WgpuOutOfRange.
fn untyped(item: &PsObject, ty: ElementType) -> PsResult<Num> {
    let Some((min, max)) = ty.range() else {
        return Ok(Num::Float(f64::from_ps(item)?));
    };
    let converted = if ty.is_signed() { i64::from_ps(item).map(i128::from) } else { u64::from_ps(item).map(i128::from) };
    let value = match converted {
        Ok(value) => value,
        // A number no 64-bit integer of the type's sign holds is outside
        // the type's range; anything else is not a number.
        Err(conversion) => {
            if is_scalar_type(&item.type_name()?) {
                return Err(out_of_range(&String::from_ps(item)?, ty, min, max));
            }
            return Err(conversion);
        }
    };
    if value < min || value > max {
        return Err(out_of_range(&value.to_string(), ty, min, max));
    }
    Ok(Num::Int(value))
}

/// The error for `value`, a number outside `ty`'s range from `min` to `max`.
fn out_of_range(value: &str, ty: ElementType, min: i128, max: i128) -> PsError {
    invalid("WgpuOutOfRange", format!("{value} does not fit in {ty:?}, which holds {min} to {max}"))
}

/// One untyped number as the bytes of one element of `ty`, converted as
/// each number of an untyped array is.
pub(crate) fn scalar_bytes(ty: ElementType, value: &PsObject) -> PsResult<Vec<u8>> {
    let mut out = Vec::with_capacity(ty.size() as usize);
    encode(ty, untyped(value, ty)?, &mut out);
    Ok(out)
}

/// Calls `f` with the element type and the bytes of `obj`: a .NET array of
/// any numeric type (read in place, without a copy, when its type is
/// the one wanted), any other collection of numbers, or one number. `want`
/// converts the values to that type; without it a typed array keeps its
/// own type and everything else becomes Float32.
pub(crate) fn with_host_bytes<R>(obj: &PsObject, want: Option<ElementType>, f: impl FnOnce(ElementType, &[u8]) -> PsResult<R>) -> PsResult<R> {
    if obj.is_null() {
        return Err(invalid("WgpuNoData", "no data was given"));
    }
    let type_name = obj.type_name()?;
    if let Some(own) = ElementType::from_clr_array(&type_name) {
        macro_rules! pinned {
            ($t:ty) => {{
                let pin = obj.pin::<$t>()?;
                let bytes: &[u8] = bytemuck::cast_slice(&pin[..]);
                match want {
                    Some(to) if to != own => f(to, &convert(own, bytes, to)),
                    _ => f(own, bytes),
                }
            }};
        }
        return match own {
            ElementType::Float32 => pinned!(f32),
            ElementType::Float64 => pinned!(f64),
            ElementType::Int8 => pinned!(i8),
            ElementType::UInt8 => pinned!(u8),
            ElementType::Int16 => pinned!(i16),
            ElementType::UInt16 => pinned!(u16),
            ElementType::Int32 => pinned!(i32),
            ElementType::UInt32 => pinned!(u32),
            ElementType::Int64 => pinned!(i64),
            ElementType::UInt64 => pinned!(u64),
            ElementType::Float16 => unreachable!("no .NET array pins as f16"),
        };
    }
    if type_name == "System.String" {
        return Err(invalid("WgpuNotNumeric", "a string is not numeric data; give an array of numbers"));
    }
    let to = want.unwrap_or(ElementType::Float32);
    let items = if is_scalar_type(&type_name) { vec![obj.clone()] } else { Vec::<PsObject>::from_ps(obj)? };
    let mut out = Vec::with_capacity(items.len() * to.size() as usize);
    for item in &items {
        encode(to, untyped(item, to)?, &mut out);
    }
    f(to, &out)
}

/// The element type and bytes of `obj`, read as the cmdlets read numeric
/// data: a .NET array of a numeric type keeps its own type, or becomes
/// `want` by [`ElementType::convert`]'s rule; any other collection of
/// numbers, or one number, becomes `want` (Float32 when `None`) by
/// PowerShell's casts, an integer type taking a number rounded half to
/// even and refusing one outside its range as WgpuOutOfRange. A string is
/// refused as WgpuNotNumeric and null as WgpuNoData. It needs a PowerShell
/// host, as a PWRS module has.
pub fn host_bytes(obj: &PsObject, want: Option<ElementType>) -> PsResult<(ElementType, Vec<u8>)> {
    with_host_bytes(obj, want, |ty, bytes| Ok((ty, bytes.to_vec())))
}

fn is_scalar_type(type_name: &str) -> bool {
    matches!(
        type_name,
        "System.Single"
            | "System.Double"
            | "System.Half"
            | "System.Decimal"
            | "System.SByte"
            | "System.Byte"
            | "System.Int16"
            | "System.UInt16"
            | "System.Int32"
            | "System.UInt32"
            | "System.Int64"
            | "System.UInt64"
            | "System.Boolean"
            | "System.Numerics.BigInteger"
    )
}

/// The values of `bytes` as `T`, borrowed where `bytes` is aligned for
/// `T` and copied where it is not.
fn typed<T: bytemuck::Pod>(bytes: &[u8]) -> std::borrow::Cow<'_, [T]> {
    if bytes.as_ptr().cast::<T>().is_aligned() {
        std::borrow::Cow::Borrowed(bytemuck::cast_slice(bytes))
    } else {
        std::borrow::Cow::Owned(bytemuck::pod_collect_to_vec(bytes))
    }
}

/// A .NET array of the values in `bytes`: `float[]` for Float32 and
/// Float16, `double[]`, `sbyte[]`, `byte[]`, `short[]`, `ushort[]`,
/// `int[]`, `uint[]`, `long[]` or `ulong[]`. A trailing partial element is
/// left out. It needs a PowerShell host, as a PWRS module has.
pub fn to_clr_array(ty: ElementType, bytes: &[u8]) -> PsResult<PsObject> {
    let whole = bytes.len() - bytes.len() % ty.size() as usize;
    let bytes = &bytes[..whole];
    match ty {
        ElementType::Float32 => PsObject::from_slice::<f32>(&typed(bytes)),
        ElementType::Float64 => PsObject::from_slice::<f64>(&typed(bytes)),
        ElementType::Int8 => PsObject::from_slice::<i8>(&typed(bytes)),
        ElementType::UInt8 => PsObject::from_slice::<u8>(bytes),
        ElementType::Int16 => PsObject::from_slice::<i16>(&typed(bytes)),
        ElementType::UInt16 => PsObject::from_slice::<u16>(&typed(bytes)),
        ElementType::Int32 => PsObject::from_slice::<i32>(&typed(bytes)),
        ElementType::UInt32 => PsObject::from_slice::<u32>(&typed(bytes)),
        ElementType::Int64 => PsObject::from_slice::<i64>(&typed(bytes)),
        ElementType::UInt64 => PsObject::from_slice::<u64>(&typed(bytes)),
        ElementType::Float16 => {
            let values: Vec<f32> = bytes.as_chunks::<2>().0.iter().map(|c| f16::from_le_bytes(*c).to_f32()).collect();
            PsObject::from_slice::<f32>(&values)
        }
    }
}

/// A `Memory<T>` over the values in `bytes`, handed to .NET without a
/// second copy (a `T[]` on .NET Framework). Float16 arrives as `float`. It
/// needs a PowerShell host, as a PWRS module has.
pub fn to_clr_memory(ty: ElementType, bytes: &[u8]) -> PsResult<PsObject> {
    let whole = bytes.len() - bytes.len() % ty.size() as usize;
    let bytes = &bytes[..whole];
    macro_rules! memory {
        ($t:ty) => {{
            let mut memory = PsMemory::<$t>::zeroed(bytes.len() / std::mem::size_of::<$t>())?;
            bytemuck::cast_slice_mut::<$t, u8>(&mut memory[..]).copy_from_slice(bytes);
            memory.into_ps()
        }};
    }
    match ty {
        ElementType::Float32 => memory!(f32),
        ElementType::Float64 => memory!(f64),
        ElementType::Int8 => memory!(i8),
        ElementType::UInt8 => memory!(u8),
        ElementType::Int16 => memory!(i16),
        ElementType::UInt16 => memory!(u16),
        ElementType::Int32 => memory!(i32),
        ElementType::UInt32 => memory!(u32),
        ElementType::Int64 => memory!(i64),
        ElementType::UInt64 => memory!(u64),
        ElementType::Float16 => {
            let mut memory = PsMemory::<f32>::zeroed(bytes.len() / 2)?;
            for (slot, c) in memory.iter_mut().zip(bytes.as_chunks::<2>().0) {
                *slot = f16::from_le_bytes(*c).to_f32();
            }
            memory.into_ps()
        }
    }
}

/// Refuses `destination` as WgpuDestination unless it is an array of the
/// type [`to_clr_array`] returns for `ty` holding at least `count`
/// elements, the read [`into_clr_array`] fills it with. Each refusal names
/// what the read gives and what the destination is. It needs a PowerShell
/// host, as a PWRS module has.
pub(crate) fn check_destination(ty: ElementType, count: u64, destination: &PsObject) -> PsResult<()> {
    if destination.is_null() {
        return Err(invalid("WgpuDestination", "no array was given to read into"));
    }
    let array_type = ty.clr_array();
    let destination_type = destination.type_name()?;
    if destination_type != array_type {
        return Err(invalid("WgpuDestination", format!("a read of {ty:?} elements fills a {array_type}, and the destination is a {destination_type}")));
    }
    let length = u64::from_ps(&destination.get("Length")?)?;
    if length < count {
        return Err(too_short(count, length));
    }
    Ok(())
}

/// The error for a destination of `length` elements, fewer than the
/// `count` a read gives.
fn too_short(count: u64, length: u64) -> PsError {
    invalid("WgpuDestination", format!("the read gives {count} elements, and the destination holds {length}"))
}

/// Copies the values in `bytes`, elements of `ty`, into `destination` from
/// its index 0, as [`to_clr_array`] returns them, Float16 as `float`; the
/// elements past them keep their values. A trailing partial element is
/// left out. `destination` has passed [`check_destination`]; an array that
/// has not is refused by the pin, or as WgpuDestination when it is too
/// short. It needs a PowerShell host, as a PWRS module has.
pub(crate) fn into_clr_array(ty: ElementType, bytes: &[u8], destination: &PsObject) -> PsResult<()> {
    let count = bytes.len() / ty.size() as usize;
    let bytes = &bytes[..count * ty.size() as usize];
    if count == 0 {
        return Ok(());
    }
    macro_rules! fill {
        ($t:ty) => {{
            let mut pin = destination.pin::<$t>()?;
            let length = pin.len();
            let Some(slots) = pin.get_mut(..count) else {
                return Err(too_short(count as u64, length as u64));
            };
            bytemuck::cast_slice_mut::<$t, u8>(slots).copy_from_slice(bytes);
        }};
    }
    match ty {
        ElementType::Float32 => fill!(f32),
        ElementType::Float64 => fill!(f64),
        ElementType::Int8 => fill!(i8),
        ElementType::UInt8 => fill!(u8),
        ElementType::Int16 => fill!(i16),
        ElementType::UInt16 => fill!(u16),
        ElementType::Int32 => fill!(i32),
        ElementType::UInt32 => fill!(u32),
        ElementType::Int64 => fill!(i64),
        ElementType::UInt64 => fill!(u64),
        ElementType::Float16 => {
            let mut pin = destination.pin::<f32>()?;
            let length = pin.len();
            let Some(slots) = pin.get_mut(..count) else {
                return Err(too_short(count as u64, length as u64));
            };
            for (slot, c) in slots.iter_mut().zip(bytes.as_chunks::<2>().0) {
                *slot = f16::from_le_bytes(*c).to_f32();
            }
        }
    }
    Ok(())
}

/// Where a read copies its elements, made or checked before the read so
/// the read can place its bytes where the elements begin within a page: a
/// .NET array of the type [`to_clr_array`] returns, new or the caller's,
/// or the native elements of a `Memory<T>` as [`to_clr_memory`] returns,
/// not yet handed to .NET.
pub(crate) struct Target {
    ty: ElementType,
    place: Place,
}

/// What a [`Target`] holds.
enum Place {
    /// A .NET array.
    Array(PsObject),
    /// The elements of a `Memory<T>`.
    Memory(Memory),
}

/// The elements of a `Memory<T>`, of the .NET type a read of each element
/// type gives.
enum Memory {
    Float32(PsMemory<f32>),
    Float64(PsMemory<f64>),
    Int8(PsMemory<i8>),
    UInt8(PsMemory<u8>),
    Int16(PsMemory<i16>),
    UInt16(PsMemory<u16>),
    Int32(PsMemory<i32>),
    UInt32(PsMemory<u32>),
    Int64(PsMemory<i64>),
    UInt64(PsMemory<u64>),
}

/// `$body` with `$m` bound to the `PsMemory` in `$memory`, whatever its
/// element type.
macro_rules! each_memory {
    ($memory:expr, $m:ident => $body:expr) => {
        match $memory {
            Memory::Float32($m) => $body,
            Memory::Float64($m) => $body,
            Memory::Int8($m) => $body,
            Memory::UInt8($m) => $body,
            Memory::Int16($m) => $body,
            Memory::UInt16($m) => $body,
            Memory::Int32($m) => $body,
            Memory::UInt32($m) => $body,
            Memory::Int64($m) => $body,
            Memory::UInt64($m) => $body,
        }
    };
}

impl Target {
    /// A new array of `count` zeros, made by `Array.CreateInstance`; more
    /// elements than a .NET array holds are refused as WgpuBufferTooLarge.
    /// It needs a PowerShell host, as a PWRS module has.
    pub(crate) fn array(ty: ElementType, count: u64) -> PsResult<Target> {
        let length = i32::try_from(count).map_err(|e| invalid("WgpuBufferTooLarge", format!("the read gives {count} elements, more than a .NET array holds: {e}")))?;
        let element = PsType::from_name("System.Type").call_static("GetType", &[ty.clr_array().trim_end_matches("[]").into_ps()?])?;
        let array = PsType::from_name("System.Array").call_static("CreateInstance", &[element, length.into_ps()?])?;
        Ok(Target { ty, place: Place::Array(array) })
    }

    /// The elements of a new `Memory<T>` of `count` zeros.
    pub(crate) fn memory(ty: ElementType, count: u64) -> PsResult<Target> {
        let count = count as usize;
        let memory = match ty {
            ElementType::Float32 | ElementType::Float16 => Memory::Float32(PsMemory::zeroed(count)?),
            ElementType::Float64 => Memory::Float64(PsMemory::zeroed(count)?),
            ElementType::Int8 => Memory::Int8(PsMemory::zeroed(count)?),
            ElementType::UInt8 => Memory::UInt8(PsMemory::zeroed(count)?),
            ElementType::Int16 => Memory::Int16(PsMemory::zeroed(count)?),
            ElementType::UInt16 => Memory::UInt16(PsMemory::zeroed(count)?),
            ElementType::Int32 => Memory::Int32(PsMemory::zeroed(count)?),
            ElementType::UInt32 => Memory::UInt32(PsMemory::zeroed(count)?),
            ElementType::Int64 => Memory::Int64(PsMemory::zeroed(count)?),
            ElementType::UInt64 => Memory::UInt64(PsMemory::zeroed(count)?),
        };
        Ok(Target { ty, place: Place::Memory(memory) })
    }

    /// The caller's `destination`, refused as [`check_destination`] refuses
    /// it unless it holds `count` elements of `ty`. It needs a PowerShell
    /// host, as a PWRS module has.
    pub(crate) fn given(ty: ElementType, count: u64, destination: PsObject) -> PsResult<Target> {
        check_destination(ty, count, &destination)?;
        Ok(Target { ty, place: Place::Array(destination) })
    }

    /// The address of the first element. An array is pinned to read it,
    /// so a small array, which the collector may move once it is unpinned,
    /// may be elsewhere by the time it is filled. It needs a
    /// PowerShell host for an array.
    pub(crate) fn address(&self) -> PsResult<usize> {
        let array = match &self.place {
            Place::Array(array) => array,
            Place::Memory(memory) => return Ok(each_memory!(memory, m => m.as_ptr() as usize)),
        };
        macro_rules! at {
            ($t:ty) => {
                array.pin::<$t>()?.as_ptr() as usize
            };
        }
        Ok(match self.ty {
            ElementType::Float32 | ElementType::Float16 => at!(f32),
            ElementType::Float64 => at!(f64),
            ElementType::Int8 => at!(i8),
            ElementType::UInt8 => at!(u8),
            ElementType::Int16 => at!(i16),
            ElementType::UInt16 => at!(u16),
            ElementType::Int32 => at!(i32),
            ElementType::UInt32 => at!(u32),
            ElementType::Int64 => at!(i64),
            ElementType::UInt64 => at!(u64),
        })
    }

    /// Copies the values in `bytes` in from index 0, as [`into_clr_array`]
    /// copies them into an array. It needs a PowerShell host for an array.
    pub(crate) fn fill(&mut self, bytes: &[u8]) -> PsResult<()> {
        match &mut self.place {
            Place::Array(array) => into_clr_array(self.ty, bytes, array),
            Place::Memory(memory) => fill_memory(self.ty, bytes, memory),
        }
    }

    /// The array, or the `Memory<T>` handed to .NET (a `T[]` on .NET
    /// Framework). It needs a PowerShell host, as a PWRS module has.
    pub(crate) fn into_ps(self) -> PsResult<PsObject> {
        match self.place {
            Place::Array(array) => Ok(array),
            Place::Memory(memory) => each_memory!(memory, m => m.into_ps()),
        }
    }
}

/// Copies the values in `bytes`, elements of `ty`, into `memory` from its
/// index 0, Float16 converted to `float`. A trailing partial element is
/// left out, and memory too short for the values is refused as
/// WgpuDestination.
fn fill_memory(ty: ElementType, bytes: &[u8], memory: &mut Memory) -> PsResult<()> {
    let count = bytes.len() / ty.size() as usize;
    let bytes = &bytes[..count * ty.size() as usize];
    if let (ElementType::Float16, Memory::Float32(m)) = (ty, &mut *memory) {
        let length = m.len();
        let Some(slots) = m.get_mut(..count) else {
            return Err(too_short(count as u64, length as u64));
        };
        for (slot, c) in slots.iter_mut().zip(bytes.as_chunks::<2>().0) {
            *slot = f16::from_le_bytes(*c).to_f32();
        }
        return Ok(());
    }
    each_memory!(memory, m => {
        let length = m.len();
        let Some(slots) = m.get_mut(..count) else {
            return Err(too_short(count as u64, length as u64));
        };
        bytemuck::cast_slice_mut::<_, u8>(slots).copy_from_slice(bytes);
    });
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn floats_truncate_and_saturate_into_integers() {
        let floats: Vec<u8> = [1.7f64, -2.5, 300.0, -300.0, f64::NAN].iter().flat_map(|v| v.to_le_bytes()).collect();
        assert_eq!(convert(ElementType::Float64, &floats, ElementType::UInt8), vec![1, 0, 255, 0, 0]);
        let signed = convert(ElementType::Float64, &floats, ElementType::Int8);
        assert_eq!(signed.iter().map(|b| *b as i8).collect::<Vec<_>>(), vec![1, -2, 127, -128, 0]);
    }

    #[test]
    fn integers_wrap_into_narrower_integers() {
        let ints: Vec<u8> = [300i32, -1, 255].iter().flat_map(|v| v.to_le_bytes()).collect();
        assert_eq!(convert(ElementType::Int32, &ints, ElementType::UInt8), vec![44, 255, 255]);
        let widened = convert(ElementType::Int32, &ints, ElementType::Int64);
        let back: Vec<i64> = widened.as_chunks::<8>().0.iter().map(|c| i64::from_le_bytes(*c)).collect();
        assert_eq!(back, vec![300, -1, 255]);
    }

    /// The public conversion converts as the module does, and leaves out a
    /// trailing partial element whether or not the type changes.
    #[test]
    fn the_public_conversion_converts_whole_elements() {
        let floats: Vec<u8> = [1.7f32, -2.5, 3e9].iter().flat_map(|v| v.to_le_bytes()).collect();
        let ints = ElementType::Float32.convert(&floats, ElementType::Int32);
        assert_eq!(ints.as_chunks::<4>().0.iter().map(|c| i32::from_le_bytes(*c)).collect::<Vec<_>>(), [1, -2, i32::MAX]);
        assert_eq!(ElementType::Int32.size(), 4);
        assert_eq!(ElementType::UInt16.convert(&[1, 0, 2], ElementType::UInt16), [1, 0]);
        assert_eq!(ElementType::UInt16.convert(&[1, 0, 2], ElementType::UInt8), [1]);
    }
}
