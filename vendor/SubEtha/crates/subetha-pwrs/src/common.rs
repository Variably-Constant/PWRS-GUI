//! What every family shares: the error shapes, path resolution against
//! the session's location, the byte payloads that cross as arrays, and
//! the small records methods hand back where Rust would use a tuple.

use std::ops::Deref;
use std::time::Duration;

use pwrs::prelude::*;
use pwrs::Pinned;

/// The error for a structure that could not be obtained at `path`.
pub(crate) fn open_err(what: &str, path: &str, e: impl std::fmt::Display) -> PsError {
    PsError::new(ErrorCategory::OpenError, "SubEthaOpen", format!("cannot obtain {what} at {path}: {e}"))
}

/// The error for an operation the structure refused or could not do.
pub(crate) fn op_err(what: &str, e: impl std::fmt::Display) -> PsError {
    PsError::new(ErrorCategory::InvalidOperation, "SubEthaOperation", format!("{what}: {e}"))
}

/// The error for a frame receive whose next item is a plain item, which
/// stays on the ring for a receive that takes it.
pub(crate) fn not_a_frame_err(what: &str) -> PsError {
    PsError::new(
        ErrorCategory::InvalidData,
        "SubEthaNotAFrame",
        format!("{what}: the next item is a plain item, not a frame"),
    )
}

/// The error for an argument the structure cannot take.
pub(crate) fn arg_err(message: impl Into<String>) -> PsError {
    PsError::new(ErrorCategory::InvalidArgument, "SubEthaArgument", message.into())
}

/// What a removal found: how many of a structure's files it removed and
/// how many were not there.
#[psclass(name = "SubEtha.UnlinkReport")]
#[derive(Clone, Default)]
pub struct UnlinkReport {
    /// How many files were removed.
    pub removed: u64,
    /// How many were not there.
    pub missing: u64,
}

/// A removal's counts, or its first refusal as the error naming the file
/// refused and how many were.
pub(crate) fn unlinked(what: &str, found: subetha_cxc::adaptive_ring::UnlinkReport) -> PsResult<UnlinkReport> {
    if let Some((first, _, text)) = found.first_failure {
        let message = format!("removing {what}, {} refused, the first {}: {text}", found.failed, first.display());
        return Err(PsError::new(ErrorCategory::WriteError, "SubEthaRemove", message));
    }
    Ok(UnlinkReport { removed: found.removed as u64, missing: found.missing as u64 })
}

/// The provider path `path` names, relative to the session's current
/// location rather than the process's working directory, whether or
/// not the file exists yet.
pub(crate) fn full_path(ps: &Pipeline<'_>, path: &str) -> PsResult<String> {
    let mut resolved = ps.resolve_path(path, true)?;
    match resolved.pop() {
        Some(p) => Ok(p),
        None => Err(arg_err(format!("{path} names no path"))),
    }
}

/// The bytes of an argument: a `byte[]` borrowed where it lies, or the
/// UTF-8 of a string, so a script can pass either.
pub(crate) enum Bytes<'a> {
    Pinned(Pinned<'a, u8>),
    Owned(Vec<u8>),
}

impl Deref for Bytes<'_> {
    type Target = [u8];

    fn deref(&self) -> &[u8] {
        match self {
            Bytes::Pinned(p) => &p[..],
            Bytes::Owned(v) => v,
        }
    }
}

/// Borrows `obj` as bytes. A `byte[]` is pinned in place and copied
/// nowhere; a string is encoded; any other array of numbers, which is
/// what a script's `1, 2, 3` or an unrolled pipeline hands over, is
/// converted element by element; anything else is refused by name.
pub(crate) fn bytes(obj: &PsObject) -> PsResult<Bytes<'_>> {
    if obj.is_null() {
        return Err(arg_err("the bytes are null; pass a byte[] or a string"));
    }
    match obj.pin::<u8>() {
        Ok(pinned) => Ok(Bytes::Pinned(pinned)),
        Err(not_bytes) => {
            let name = obj.type_name()?;
            if name == "System.String" {
                return Ok(Bytes::Owned(String::from_ps(obj)?.into_bytes()));
            }
            if name.ends_with("[]") || name.contains("Collections") {
                return match Vec::<u8>::from_ps(obj) {
                    Ok(converted) => Ok(Bytes::Owned(converted)),
                    Err(e) => Err(arg_err(format!("a {name} is not bytes: {e}"))),
                };
            }
            Err(arg_err(format!("a {name} is not a byte[] or a string: {not_bytes}")))
        }
    }
}

/// A `byte[]` holding `data`, filled through one pin.
pub(crate) fn out_bytes(data: &[u8]) -> PsResult<PsObject> {
    PsObject::from_slice(data)
}

/// A timeout in seconds as a duration; a negative one is refused.
pub(crate) fn seconds(timeout: f64) -> PsResult<Duration> {
    if !timeout.is_finite() || timeout < 0.0 {
        return Err(arg_err("the timeout must be a non-negative number of seconds"));
    }
    Ok(Duration::from_secs_f64(timeout))
}

/// Seconds as a duration, or nothing to wait as long as it takes.
pub(crate) fn optional_duration(timeout: Option<f64>) -> PsResult<Option<Duration>> {
    match timeout {
        None => Ok(None),
        Some(t) => Ok(Some(seconds(t)?)),
    }
}

/// A count or size argument as the `usize` the structure takes.
pub(crate) fn size(value: u64, what: &str) -> PsResult<usize> {
    usize::try_from(value).map_err(|_| arg_err(format!("{what} is more than this platform addresses")))
}

/// A capacity that must be a power of two and at least two, which the
/// ring arithmetic relies on.
pub(crate) fn power_of_two(capacity: u64) -> PsResult<usize> {
    if !capacity.is_power_of_two() || capacity < 2 {
        return Err(arg_err("the capacity must be a power of two and at least two"));
    }
    size(capacity, "the capacity")
}

/// The process id an argument names, this process's own when absent.
pub(crate) fn pid(pid: Option<u32>) -> u32 {
    pid.unwrap_or_else(std::process::id)
}

/// Fails the build when a class's value cannot leave the thread it was
/// made on, because the runtime frees a collected object's value from
/// the finalizer thread.
macro_rules! assert_send {
    ($($ty:ty),* $(,)?) => {
        $( const _: fn() = || { fn needs_send<T: Send>() {} needs_send::<$ty>(); }; )*
    };
}
pub(crate) use assert_send;

/// An item and the stamp its sender gave it.
#[psclass(name = "SubEtha.StampedItem")]
#[derive(Clone, Default)]
pub struct StampedItem {
    /// The stamp, which orders the item among everything its sender
    /// and the other senders made.
    pub stamp: u64,
    /// The item's bytes, a `byte[]`.
    pub bytes: PsObject,
}

impl StampedItem {
    pub(crate) fn new(stamp: u64, data: &[u8]) -> PsResult<Self> {
        Ok(Self { stamp, bytes: out_bytes(data)? })
    }
}

pub(crate) use subetha_cxc::byte_value::{LeaseValue, SlotValue, LEASE_VALUE_BYTES, SLOT_VALUE_BYTES};

/// A script's bytes into the value a structure holds, a value past its
/// room refused as an argument error, and the value back out as a
/// `byte[]`.
pub(crate) trait ItemBytes: Sized {
    fn from_item(value: &[u8]) -> PsResult<Self>;
    fn to_ps(self) -> PsResult<PsObject>;
}

impl<const N: usize> ItemBytes for subetha_cxc::byte_value::ByteValue<N> {
    fn from_item(value: &[u8]) -> PsResult<Self> {
        Self::from_bytes(value).map_err(|e| arg_err(e.to_string()))
    }

    fn to_ps(self) -> PsResult<PsObject> {
        out_bytes(self.as_bytes())
    }
}

/// A reading of a hybrid logical clock: the physical microseconds and
/// the logical counter that breaks ties within a microsecond.
#[psclass(name = "SubEtha.ClockReading")]
#[derive(Clone, Default)]
pub struct ClockReading {
    /// Microseconds of physical time.
    pub physical_us: u64,
    /// The counter that orders events sharing a microsecond.
    pub logical: u64,
}
