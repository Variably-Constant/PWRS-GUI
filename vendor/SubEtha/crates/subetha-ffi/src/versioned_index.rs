//! The shared versioned index through the C ABI: an ordered map that any
//! number of threads and processes write at once, keeping every version
//! of a key that a pinned reader can still need.
//!
//! A statement takes a pin and a ticket from an epoch table handle, writes
//! under the ticket, reads at the pin, and publishes the ticket when it is
//! done, so everything it wrote appears at once. The index is built from
//! that epoch table handle and keeps its own reference to the table, as an
//! epoch barrier does to its heartbeat table, so the calls that take no
//! tokens keep working after the handle is destroyed. A call that takes
//! tokens takes the handle that issued them too, and that must be the
//! handle the index was built from: a token from any other handle is
//! refused, since a ticket from another table stamps nothing this index
//! can take back and a pin from one keeps nothing it reads in place.
//! Indexes built from one handle share its tickets, so one ticket stamps
//! them all. A ticket of zero means none, for a read that sees only what
//! the pin sees.
//!
//! One compare-and-swap decides each write, and a writer that loses is
//! refused rather than made to wait: a key whose newest version was born
//! after the statement's pin by another epoch, published or not, answers
//! `SUBETHA_E_WRITE_CONFLICT` with that epoch. The statement takes back what
//! it wrote with `subetha_versioned_index_void_epoch` on its own ticket's
//! epoch, publishes, and runs again under a fresh pin.
//!
//! Keys order as unsigned bytes, so a caller that wants numeric order
//! stores its integers big-endian. Key and value sizes come from the caller
//! at create, and an index is attached at the sizes it was made with.

use std::ffi::c_char;
use std::ops::Bound;
use std::path::Path;
use std::sync::Arc;

use subetha_cxc::raw_shared_versioned_index::RawSharedVersionedIndex;
use subetha_cxc::shared_versioned_index::{IndexError, DEFAULT_CHUNK_BITS};

use crate::epochs::{statement, table_of, Statement};
use crate::error::{
    epochs_code, fail, SUBETHA_E_BUFFER_TOO_SMALL, SUBETHA_E_INVALID_ARGUMENT, SUBETHA_E_IO,
    SUBETHA_E_LAYOUT_MISMATCH, SUBETHA_E_MAP_FULL, SUBETHA_E_MAP_KEY_ABSENT, SUBETHA_E_WRITE_CONFLICT,
    SUBETHA_E_WRONG_KIND, SUBETHA_OK,
};
use crate::handle::{subetha_handle, Object, SUBETHA_KIND_VERSIONED_INDEX};
use crate::ring::{bytes, finish_unlink, subetha_unlink_report, text};
use crate::runtime::{entry, issue, require_initialized, resolve_mode, with_kind};

/// The log2 of the slots one chunk file holds when the caller passes zero.
///
/// Spelled as a literal because cbindgen copies the right-hand side into
/// the header verbatim, and a name from another crate does not exist
/// there; the assertion below holds it to the value it mirrors.
pub const SUBETHA_VERSIONED_INDEX_CHUNK_BITS_DEFAULT: u32 = 16;

const _: () = assert!(SUBETHA_VERSIONED_INDEX_CHUNK_BITS_DEFAULT == DEFAULT_CHUNK_BITS);

/// A snapshot of a shared versioned index.
#[repr(C)]
#[allow(non_camel_case_types)]
#[derive(Clone, Copy, Default)]
pub struct subetha_versioned_index_stats {
    /// The mode this handle was created in.
    pub mode: u32,
    /// Bytes a key takes.
    pub key_size: u64,
    /// Bytes a value takes.
    pub value_size: u64,
    /// The epoch the shared table stands at now.
    pub epoch: u64,
}

pub(crate) struct VersionedIndexObject {
    index: RawSharedVersionedIndex,
    mode: u32,
}

impl VersionedIndexObject {
    /// An index parks nothing inside a call, so a destroy has nobody to
    /// wake.
    pub(crate) fn interrupt(&self) {}
}

fn code_for(e: IndexError) -> i32 {
    match e {
        e @ IndexError::Conflict { .. } => fail(SUBETHA_E_WRITE_CONFLICT, e.to_string()),
        IndexError::Full => fail(SUBETHA_E_MAP_FULL, "the index has no free slot after reclaiming what the horizon allows"),
        e @ IndexError::LayoutMismatch(_) => fail(SUBETHA_E_LAYOUT_MISMATCH, e.to_string()),
        IndexError::InvalidConfig(why) => fail(SUBETHA_E_INVALID_ARGUMENT, why),
        IndexError::Epochs(e) => epochs_code(e),
        IndexError::IoError(e) => fail(SUBETHA_E_IO, format!("versioned index io: {e}")),
    }
}

fn with_index(handle: subetha_handle, f: impl FnOnce(&VersionedIndexObject) -> i32) -> i32 {
    with_kind(handle, SUBETHA_KIND_VERSIONED_INDEX, |object| match object {
        Object::VersionedIndex(i) => f(i),
        _ => fail(SUBETHA_E_WRONG_KIND, "the handle does not name a versioned index"),
    })
}

/// Refuse tokens from any epoch table handle but the one the index was
/// built from.
fn same_table(index: &VersionedIndexObject, statement: &Statement) -> Result<(), i32> {
    if Arc::ptr_eq(index.index.epochs(), &statement.table) {
        Ok(())
    } else {
        Err(fail(
            SUBETHA_E_INVALID_ARGUMENT,
            "the epoch table handle is not the one this index was built from",
        ))
    }
}

/// The key the caller passed, which must be the index's key size.
///
/// # Safety
/// `key` points to `key_len` readable bytes.
unsafe fn key_of<'a>(index: &VersionedIndexObject, key: *const u8, key_len: usize) -> Result<&'a [u8], i32> {
    let size = index.index.key_size();
    if key_len != size {
        return Err(fail(SUBETHA_E_INVALID_ARGUMENT, format!("a key is {size} bytes, not {key_len}")));
    }
    unsafe { bytes(key, key_len) }
}

/// One end of a range: no bound when `key` is null, otherwise a key of the
/// index's key size, left out when `excluded`.
///
/// # Safety
/// `key` is null or points to `key_size` readable bytes.
unsafe fn bound_of<'a>(index: &VersionedIndexObject, key: *const u8, excluded: bool) -> Result<Bound<&'a [u8]>, i32> {
    if key.is_null() {
        return Ok(Bound::Unbounded);
    }
    let key = unsafe { bytes(key, index.index.key_size()) }?;
    Ok(if excluded { Bound::Excluded(key) } else { Bound::Included(key) })
}

/// The arguments both constructors read after the epoch table, in order,
/// so the first refusal names the argument at fault.
///
/// # Safety
/// `path` is a NUL-terminated UTF-8 string; `out` is a valid pointer.
unsafe fn read_arguments<'a>(
    path: *const c_char,
    key_size: usize,
    value_size: usize,
    mode: u32,
    out: *mut subetha_handle,
) -> Result<(&'a Path, u32), i32> {
    if out.is_null() {
        return Err(fail(SUBETHA_E_INVALID_ARGUMENT, "out is null"));
    }
    let path = Path::new(unsafe { text(path, "path") }?);
    if key_size == 0 {
        return Err(fail(SUBETHA_E_INVALID_ARGUMENT, "key_size is zero"));
    }
    if value_size == 0 {
        return Err(fail(SUBETHA_E_INVALID_ARGUMENT, "value_size is zero"));
    }
    let mode = resolve_mode(mode)?;
    Ok((path, mode))
}

fn place(index: Result<RawSharedVersionedIndex, IndexError>, mode: u32, out: *mut subetha_handle) -> i32 {
    match index {
        Ok(index) => unsafe { issue(Object::VersionedIndex(VersionedIndexObject { index, mode }), out) },
        Err(e) => code_for(e),
    }
}

/// Obtain the index at `path` for `key_size`-byte keys and `value_size`-byte
/// values over the epoch table `epochs` names, making it when the file does
/// not exist and attaching to it when it does. Keys, versions and write
/// lists live in chunk files beside `path`, named after it, of
/// `1 << chunk_bits` slots each, 4 to 20 bits; a `chunk_bits` of zero takes
/// `SUBETHA_VERSIONED_INDEX_CHUNK_BITS_DEFAULT`. The index keeps its own
/// reference to the table; the calls that take tokens take them from
/// `epochs` and no other handle.
///
/// # Safety
/// `path` is a NUL-terminated UTF-8 string; `out` is a valid pointer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn subetha_versioned_index_create(
    path: *const c_char,
    epochs: subetha_handle,
    key_size: usize,
    value_size: usize,
    chunk_bits: u32,
    mode: u32,
    out: *mut subetha_handle,
) -> i32 {
    let table = match table_of(epochs) {
        Ok(t) => t,
        Err(code) => return code,
    };
    entry(|| {
        let (path, mode) = match unsafe { read_arguments(path, key_size, value_size, mode, out) } {
            Ok(a) => a,
            Err(code) => return code,
        };
        let bits = if chunk_bits == 0 { SUBETHA_VERSIONED_INDEX_CHUNK_BITS_DEFAULT } else { chunk_bits };
        place(RawSharedVersionedIndex::create_with_chunks(path, table, key_size, value_size, bits), mode, out)
    })
}

/// Attach to the index another process made at `path` over the epoch table
/// `epochs` names, at the key and value sizes it was made with; the file
/// must exist, and `SUBETHA_E_IO` names an absent one.
///
/// # Safety
/// `path` is a NUL-terminated UTF-8 string; `out` is a valid pointer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn subetha_versioned_index_open(
    path: *const c_char,
    epochs: subetha_handle,
    key_size: usize,
    value_size: usize,
    mode: u32,
    out: *mut subetha_handle,
) -> i32 {
    let table = match table_of(epochs) {
        Ok(t) => t,
        Err(code) => return code,
    };
    entry(|| {
        let (path, mode) = match unsafe { read_arguments(path, key_size, value_size, mode, out) } {
            Ok(a) => a,
            Err(code) => return code,
        };
        place(RawSharedVersionedIndex::open(path, table, key_size, value_size), mode, out)
    })
}

/// Write the value at `value`, or a deletion for `None`, in the statement
/// the tokens name.
///
/// # Safety
/// `key` points to `key_len` readable bytes, a value's pointer to its
/// length's; `out_conflict` is null or a valid pointer.
#[allow(clippy::too_many_arguments)]
unsafe fn write_in(
    handle: subetha_handle,
    epochs: subetha_handle,
    ticket: u64,
    pin: u64,
    key: *const u8,
    key_len: usize,
    value: Option<(*const u8, usize)>,
    out_conflict: *mut u64,
) -> i32 {
    if ticket == 0 {
        return entry(|| fail(SUBETHA_E_INVALID_ARGUMENT, "ticket is zero, and a write is stamped with one"));
    }
    let held = match statement(epochs, ticket, Some(pin)) {
        Ok(s) => s,
        Err(code) => return code,
    };
    with_index(handle, |ix| {
        if let Err(code) = same_table(ix, &held) {
            return code;
        }
        let (Some(ticket), Some(pin)) = (held.ticket, held.pin) else {
            return fail(SUBETHA_E_INVALID_ARGUMENT, "a write needs both a ticket and a pin");
        };
        let key = match unsafe { key_of(ix, key, key_len) } {
            Ok(k) => k,
            Err(code) => return code,
        };
        let value = match value {
            None => None,
            Some((data, len)) => {
                let size = ix.index.value_size();
                if len != size {
                    return fail(SUBETHA_E_INVALID_ARGUMENT, format!("a value is {size} bytes, not {len}"));
                }
                match unsafe { bytes(data, len) } {
                    Ok(v) => Some(v),
                    Err(code) => return code,
                }
            }
        };
        match ix.index.write(key, value, ticket, pin) {
            Ok(()) => SUBETHA_OK,
            Err(IndexError::Conflict { epoch }) => {
                if !out_conflict.is_null() {
                    // Checked non-null; the caller guarantees it is writable.
                    unsafe { *out_conflict = epoch };
                }
                code_for(IndexError::Conflict { epoch })
            }
            Err(e) => code_for(e),
        }
    })
}

/// Make `value` current at `key` in the statement holding the ticket
/// `ticket`, whose snapshot is the pin `pin`, both tokens from the epoch
/// table `epochs`. A second write under the same ticket replaces its own
/// version. `SUBETHA_E_WRITE_CONFLICT` when the key's newest version was
/// born after the pin by another epoch, which `out_conflict` receives when
/// it is not null.
///
/// # Safety
/// `key` points to `key_len` readable bytes and `value` to `value_len`;
/// `out_conflict` is null or a valid pointer.
#[unsafe(no_mangle)]
#[allow(clippy::too_many_arguments)]
pub unsafe extern "C" fn subetha_versioned_index_write(
    handle: subetha_handle,
    epochs: subetha_handle,
    ticket: u64,
    pin: u64,
    key: *const u8,
    key_len: usize,
    value: *const u8,
    value_len: usize,
    out_conflict: *mut u64,
) -> i32 {
    unsafe { write_in(handle, epochs, ticket, pin, key, key_len, Some((value, value_len)), out_conflict) }
}

/// Delete `key` in the statement holding `ticket`, whose snapshot is `pin`:
/// a version with no value, which a reader pinned earlier steps past.
/// Deleting a version the ticket wrote leaves a deletion when that version
/// superseded a value, and otherwise puts the key back as it was before the
/// ticket wrote it. `SUBETHA_E_WRITE_CONFLICT` as for a write.
///
/// # Safety
/// `key` points to `key_len` readable bytes; `out_conflict` is null or a
/// valid pointer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn subetha_versioned_index_delete(
    handle: subetha_handle,
    epochs: subetha_handle,
    ticket: u64,
    pin: u64,
    key: *const u8,
    key_len: usize,
    out_conflict: *mut u64,
) -> i32 {
    unsafe { write_in(handle, epochs, ticket, pin, key, key_len, None, out_conflict) }
}

/// Take back what `epoch` wrote to `key`: its version goes and the one it
/// superseded is current again, and `out_voided` says whether there was
/// one. An epoch's version is the newest on its key until the epoch
/// publishes; once a later version supersedes it, it is history a reader
/// may already have seen, and it stays.
///
/// # Safety
/// `key` points to `key_len` readable bytes; `out_voided` is null or a
/// valid pointer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn subetha_versioned_index_void(
    handle: subetha_handle,
    key: *const u8,
    key_len: usize,
    epoch: u64,
    out_voided: *mut bool,
) -> i32 {
    with_index(handle, |ix| {
        let key = match unsafe { key_of(ix, key, key_len) } {
            Ok(k) => k,
            Err(code) => return code,
        };
        match ix.index.void(key, epoch) {
            Ok(voided) => {
                if !out_voided.is_null() {
                    // Checked non-null; the caller guarantees it is writable.
                    unsafe { *out_voided = voided };
                }
                SUBETHA_OK
            }
            Err(e) => code_for(e),
        }
    })
}

/// Take back everything `epoch` wrote, key by key, from the write list of
/// the ticket holding it, reporting how many keys gave a version back
/// through `out_voided` when that is not null. This is how the epoch of a
/// dead ticket, which `subetha_epochs_dead_tickets` names, is taken back
/// before `subetha_epochs_free_dead_ticket` releases its slot, and how a
/// statement refused with `SUBETHA_E_WRITE_CONFLICT` takes back its own
/// writes before it publishes.
///
/// # Safety
/// `out_voided` is null or a valid pointer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn subetha_versioned_index_void_epoch(
    handle: subetha_handle,
    epoch: u64,
    out_voided: *mut u64,
) -> i32 {
    with_index(handle, |ix| match ix.index.void_epoch(epoch) {
        Ok(voided) => {
            if !out_voided.is_null() {
                // Checked non-null; the caller guarantees it is writable.
                unsafe { *out_voided = voided as u64 };
            }
            SUBETHA_OK
        }
        Err(e) => code_for(e),
    })
}

/// The value of `key` the statement reading at the pin `pin` sees, into
/// `out`, or the one its own ticket wrote when `ticket` is not zero; both
/// tokens from `epochs`. `out_len` receives the value's size.
/// `SUBETHA_E_MAP_KEY_ABSENT` when the statement sees no value there.
///
/// # Safety
/// `key` points to `key_len` readable bytes; `out` points to `cap`
/// writable bytes; `out_len` is a valid pointer.
#[unsafe(no_mangle)]
#[allow(clippy::too_many_arguments)]
pub unsafe extern "C" fn subetha_versioned_index_get(
    handle: subetha_handle,
    epochs: subetha_handle,
    ticket: u64,
    pin: u64,
    key: *const u8,
    key_len: usize,
    out: *mut u8,
    cap: usize,
    out_len: *mut usize,
) -> i32 {
    let held = match statement(epochs, ticket, Some(pin)) {
        Ok(s) => s,
        Err(code) => return code,
    };
    with_index(handle, |ix| {
        if let Err(code) = same_table(ix, &held) {
            return code;
        }
        let Some(pin) = held.pin else {
            return fail(SUBETHA_E_INVALID_ARGUMENT, "a read needs a pin");
        };
        if out_len.is_null() {
            return fail(SUBETHA_E_INVALID_ARGUMENT, "out_len is null");
        }
        let key = match unsafe { key_of(ix, key, key_len) } {
            Ok(k) => k,
            Err(code) => return code,
        };
        let size = ix.index.value_size();
        // Checked non-null; the caller guarantees it is writable.
        unsafe { *out_len = size };
        if out.is_null() || cap < size {
            return SUBETHA_E_BUFFER_TOO_SMALL;
        }
        // The caller guarantees `cap` writable bytes, and cap is enough here.
        let buf = unsafe { std::slice::from_raw_parts_mut(out, size) };
        match ix.index.get_at(key, pin, held.ticket.map(|(_, epoch)| epoch), buf) {
            Ok(true) => SUBETHA_OK,
            Ok(false) => SUBETHA_E_MAP_KEY_ABSENT,
            Err(e) => code_for(e),
        }
    })
}

/// One page of the entries the statement reading at the pin `pin` sees,
/// those its own ticket wrote included when `ticket` is not zero, in key
/// order, packed into `out` as key bytes then value bytes per entry; both
/// tokens from `epochs`. `out_count` receives the entries and `out_len`
/// the bytes they take.
///
/// `low` and `high` bound the range, each null for no bound there and
/// otherwise a key of the index's key size; `low_excluded` and
/// `high_excluded` leave the bound key itself out. `limit` counts versions
/// examined. When it stops the page, `out_has_cursor` is true and
/// `out_cursor` receives the last key examined, and the next page passes
/// that key as `low` with `low_excluded` set. A key's versions are never
/// split across two pages, every page examines at least one key, and
/// bounds that cross read as no rows. A page holds at most `limit` entries,
/// or one when `limit` is zero, so an `out` of that many entries is never
/// too small. `SUBETHA_E_BUFFER_TOO_SMALL` leaves `out` and `out_cursor`
/// untouched with the bytes needed reported.
///
/// # Safety
/// `low`, `high` and `out_cursor` are null or point to `key_size` bytes;
/// `out` points to `cap` writable bytes; `out_len`, `out_count` and
/// `out_has_cursor` are valid pointers.
#[unsafe(no_mangle)]
#[allow(clippy::too_many_arguments)]
pub unsafe extern "C" fn subetha_versioned_index_range(
    handle: subetha_handle,
    epochs: subetha_handle,
    ticket: u64,
    pin: u64,
    low: *const u8,
    low_excluded: bool,
    high: *const u8,
    high_excluded: bool,
    limit: usize,
    out: *mut u8,
    cap: usize,
    out_len: *mut usize,
    out_count: *mut usize,
    out_cursor: *mut u8,
    out_has_cursor: *mut bool,
) -> i32 {
    let held = match statement(epochs, ticket, Some(pin)) {
        Ok(s) => s,
        Err(code) => return code,
    };
    with_index(handle, |ix| {
        if let Err(code) = same_table(ix, &held) {
            return code;
        }
        let Some(pin) = held.pin else {
            return fail(SUBETHA_E_INVALID_ARGUMENT, "a read needs a pin");
        };
        if out_len.is_null() || out_count.is_null() || out_has_cursor.is_null() {
            return fail(SUBETHA_E_INVALID_ARGUMENT, "out_len, out_count or out_has_cursor is null");
        }
        let low = match unsafe { bound_of(ix, low, low_excluded) } {
            Ok(b) => b,
            Err(code) => return code,
        };
        let high = match unsafe { bound_of(ix, high, high_excluded) } {
            Ok(b) => b,
            Err(code) => return code,
        };
        let (rows, cursor) = match ix.index.range_at(low, high, limit, pin, held.ticket.map(|(_, epoch)| epoch)) {
            Ok(page) => page,
            Err(e) => return code_for(e),
        };
        let key_size = ix.index.key_size();
        let stride = key_size + ix.index.value_size();
        let needed = rows.len() * stride;
        // Checked non-null; the caller guarantees they are writable.
        unsafe {
            *out_len = needed;
            *out_count = rows.len();
            *out_has_cursor = cursor.is_some();
        }
        if needed > 0 && (out.is_null() || cap < needed) {
            return SUBETHA_E_BUFFER_TOO_SMALL;
        }
        if needed > 0 {
            // The caller guarantees `cap` writable bytes, and cap >= needed.
            let buf = unsafe { std::slice::from_raw_parts_mut(out, needed) };
            for (i, (k, v)) in rows.iter().enumerate() {
                let at = i * stride;
                buf[at..at + key_size].copy_from_slice(k);
                buf[at + key_size..at + stride].copy_from_slice(v);
            }
        }
        if let Some(c) = cursor
            && !out_cursor.is_null()
        {
            // The caller guarantees `key_size` writable bytes there.
            let slot = unsafe { std::slice::from_raw_parts_mut(out_cursor, key_size) };
            slot.copy_from_slice(&c);
        }
        SUBETHA_OK
    })
}

/// Whether any key in the range gained a version after `since` from an
/// epoch other than the one of the ticket `ticket`, a deletion included,
/// into `out_changed`: what a statement validates for each range it
/// scanned before it publishes. `ticket` is a token from `epochs`, or zero
/// for none; the bounds read as `subetha_versioned_index_range` reads them.
///
/// # Safety
/// `low` and `high` are null or point to `key_size` bytes; `out_changed`
/// is a valid pointer.
#[unsafe(no_mangle)]
#[allow(clippy::too_many_arguments)]
pub unsafe extern "C" fn subetha_versioned_index_changed_since(
    handle: subetha_handle,
    epochs: subetha_handle,
    ticket: u64,
    low: *const u8,
    low_excluded: bool,
    high: *const u8,
    high_excluded: bool,
    since: u64,
    out_changed: *mut bool,
) -> i32 {
    let held = match statement(epochs, ticket, None) {
        Ok(s) => s,
        Err(code) => return code,
    };
    with_index(handle, |ix| {
        if let Err(code) = same_table(ix, &held) {
            return code;
        }
        if out_changed.is_null() {
            return fail(SUBETHA_E_INVALID_ARGUMENT, "out_changed is null");
        }
        let low = match unsafe { bound_of(ix, low, low_excluded) } {
            Ok(b) => b,
            Err(code) => return code,
        };
        let high = match unsafe { bound_of(ix, high, high_excluded) } {
            Ok(b) => b,
            Err(code) => return code,
        };
        match ix.index.changed_since(low, high, since, held.ticket.map(|(_, epoch)| epoch)) {
            Ok(changed) => {
                // Checked non-null; the caller guarantees it is writable.
                unsafe { *out_changed = changed };
                SUBETHA_OK
            }
            Err(e) => code_for(e),
        }
    })
}

/// Drop every version superseded at or below the epoch table's reclaim
/// horizon and every key whose newest version is a deletion at or below
/// it, reporting how many versions holding a value went through
/// `out_dropped` when that is not null. What goes is retired, and its slot
/// is handed out again only once the horizon has passed the epoch it was
/// retired at.
///
/// # Safety
/// `out_dropped` is null or a valid pointer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn subetha_versioned_index_sweep(handle: subetha_handle, out_dropped: *mut u64) -> i32 {
    with_index(handle, |ix| match ix.index.sweep() {
        Ok(dropped) => {
            if !out_dropped.is_null() {
                // Checked non-null; the caller guarantees it is writable.
                unsafe { *out_dropped = dropped as u64 };
            }
            SUBETHA_OK
        }
        Err(e) => code_for(e),
    })
}

/// The versions the index holds, current and superseded, deletions
/// included, into `out`. It walks every key.
///
/// # Safety
/// `out` is a valid pointer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn subetha_versioned_index_version_count(handle: subetha_handle, out: *mut u64) -> i32 {
    with_index(handle, |ix| {
        if out.is_null() {
            return fail(SUBETHA_E_INVALID_ARGUMENT, "out is null");
        }
        match ix.index.version_count() {
            Ok(count) => {
                // Checked non-null; the caller guarantees it is writable.
                unsafe { *out = count as u64 };
                SUBETHA_OK
            }
            Err(e) => code_for(e),
        }
    })
}

/// The keys whose newest version holds a value, into `out`. It walks every
/// key.
///
/// # Safety
/// `out` is a valid pointer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn subetha_versioned_index_live_count(handle: subetha_handle, out: *mut u64) -> i32 {
    with_index(handle, |ix| {
        if out.is_null() {
            return fail(SUBETHA_E_INVALID_ARGUMENT, "out is null");
        }
        match ix.index.live_count() {
            Ok(count) => {
                // Checked non-null; the caller guarantees it is writable.
                unsafe { *out = count as u64 };
                SUBETHA_OK
            }
            Err(e) => code_for(e),
        }
    })
}

/// A snapshot of the index into `out`.
///
/// # Safety
/// `out` is a valid pointer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn subetha_versioned_index_read_stats(
    handle: subetha_handle,
    out: *mut subetha_versioned_index_stats,
) -> i32 {
    with_index(handle, |ix| {
        if out.is_null() {
            return fail(SUBETHA_E_INVALID_ARGUMENT, "out is null");
        }
        let stats = subetha_versioned_index_stats {
            mode: ix.mode,
            key_size: ix.index.key_size() as u64,
            value_size: ix.index.value_size() as u64,
            epoch: ix.index.epochs().now(),
        };
        // Checked non-null; the caller guarantees it is writable.
        unsafe { *out = stats };
        SUBETHA_OK
    })
}

/// Remove every file of the index at `path`: its main file and each chunk
/// file beside it. The contract is `subetha_ring_unlink`'s. The epoch
/// table is a file of its own, removed with `subetha_epochs_unlink`.
///
/// # Safety
/// `path` is a NUL-terminated UTF-8 string; `report` is null or a valid
/// pointer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn subetha_versioned_index_unlink(
    path: *const c_char,
    report: *mut subetha_unlink_report,
) -> i32 {
    entry(|| {
        if let Err(code) = require_initialized() {
            return code;
        }
        let path = match unsafe { text(path, "path") } {
            Ok(p) => p,
            Err(code) => return code,
        };
        unsafe { finish_unlink(RawSharedVersionedIndex::unlink(path), report) }
    })
}
