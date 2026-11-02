//! The handle table through the C ABI: values of up to
//! `SUBETHA_HANDLE_TABLE_VALUE_MAX` bytes reached by a table handle rather
//! than by a place, so a slot reused after its value is removed cannot be
//! mistaken for the old one. A table handle is a `uint64_t` carrying the
//! slot in its low half and the times that slot has been reused in its
//! high half; zero is never one. It is the number Python's `HandleTable`
//! and PowerShell's `New-SubEthaHandleTable` answer and the values are
//! laid out as theirs are, so a table any of them made at a path is the
//! same table to C. A handle names one value for its whole life: a handle
//! whose value has been removed, or whose slot has since been reused,
//! answers `SUBETHA_E_MAP_KEY_ABSENT`, and a full table
//! `SUBETHA_E_MAP_FULL`. The table runs no background work, so strict and
//! managed modes are the same.

use std::ffi::c_char;
use std::path::{Path, PathBuf};

use subetha_cxc::adaptive_ring::UnlinkReport;
use subetha_cxc::shared_handle_table::{Handle, HandleTableError, SharedHandleTable};
use subetha_cxc::{LeaseValue, LEASE_VALUE_BYTES};

use crate::batch::run_push_many_lens_reporting;
use crate::error::{
    fail, handle_table_code, SUBETHA_E_BUFFER_TOO_SMALL, SUBETHA_E_INVALID_ARGUMENT, SUBETHA_E_MAP_KEY_ABSENT,
    SUBETHA_E_WRONG_KIND, SUBETHA_OK,
};
use crate::handle::{subetha_handle, Object, SUBETHA_KIND_HANDLE_TABLE};
use crate::ring::{bytes, copy_out, finish_unlink, subetha_unlink_report, text};
use crate::runtime::{entry, issue, require_initialized, resolve_mode, with_kind};

/// The most bytes a handle table value holds.
pub const SUBETHA_HANDLE_TABLE_VALUE_MAX: usize = 44;

const _: () = assert!(SUBETHA_HANDLE_TABLE_VALUE_MAX == LEASE_VALUE_BYTES);

/// A snapshot of a handle table.
#[repr(C)]
#[allow(non_camel_case_types)]
#[derive(Clone, Copy, Default)]
pub struct subetha_handle_table_stats {
    /// The mode this handle was created in.
    pub mode: u32,
    /// Values the table holds at most.
    pub capacity: u64,
    /// Values live now.
    pub len: u64,
}

pub(crate) struct HandleTableObject {
    table: SharedHandleTable<LeaseValue>,
    mode: u32,
}

impl HandleTableObject {
    /// The table as the sidecar observes it.
    pub(crate) fn adaptive(&self) -> &(dyn subetha_sidecar::AdaptiveInstance + 'static) {
        &self.table
    }
}

impl HandleTableObject {
    /// A table parks nothing inside a call, so a destroy has nobody to wake.
    pub(crate) fn interrupt(&self) {}

    /// The value `handle` names, or the refusal naming it.
    fn value(&self, handle: u64) -> Result<LeaseValue, i32> {
        self.table.get(handle_of(handle)).ok_or_else(|| absent(handle))
    }
}

fn with_table(table: subetha_handle, f: impl FnOnce(&HandleTableObject) -> i32) -> i32 {
    with_kind(table, SUBETHA_KIND_HANDLE_TABLE, |object| match object {
        Object::HandleTable(t) => f(t),
        _ => fail(SUBETHA_E_WRONG_KIND, "the handle does not name a handle table"),
    })
}

fn handle_of(raw: u64) -> Handle {
    Handle::from_parts((raw >> 32) as u32, raw as u32)
}

fn absent(handle: u64) -> i32 {
    fail(SUBETHA_E_MAP_KEY_ABSENT, format!("table handle {handle:#x} names no live value"))
}

/// The `len` bytes at `value` as a table value.
///
/// # Safety
/// `value` is null or addresses `len` readable bytes.
unsafe fn table_value(value: *const u8, len: usize) -> Result<LeaseValue, i32> {
    let read = unsafe { bytes(value, len) }?;
    LeaseValue::from_bytes(read).map_err(|e| fail(SUBETHA_E_INVALID_ARGUMENT, e.to_string()))
}

/// The arguments every constructor reads, in order, so the first refusal
/// names the argument at fault.
///
/// # Safety
/// `path` is a NUL-terminated UTF-8 string; `out` is a valid pointer.
unsafe fn read_arguments<'a>(
    path: *const c_char,
    capacity: u32,
    mode: u32,
    out: *mut subetha_handle,
) -> Result<(&'a Path, usize, u32), i32> {
    require_initialized()?;
    if out.is_null() {
        return Err(fail(SUBETHA_E_INVALID_ARGUMENT, "out is null"));
    }
    let path = Path::new(unsafe { text(path, "path") }?);
    if capacity == 0 || capacity == u32::MAX {
        return Err(fail(
            SUBETHA_E_INVALID_ARGUMENT,
            format!("a capacity of {capacity} is outside 1 to {}", u32::MAX - 1),
        ));
    }
    let mode = resolve_mode(mode)?;
    Ok((path, capacity as usize, mode))
}

fn place(table: Result<SharedHandleTable<LeaseValue>, HandleTableError>, mode: u32, out: *mut subetha_handle) -> i32 {
    match table {
        Ok(table) => unsafe { issue(Object::HandleTable(HandleTableObject { table, mode }), out) },
        Err(e) => handle_table_code(e),
    }
}

/// Obtain the handle table at `path` holding `capacity` values: an empty
/// one is initialized when the file does not exist, an existing one is
/// attached with its live handles in place. A file of another capacity is
/// `SUBETHA_E_LAYOUT_MISMATCH`. `capacity` runs from 1 to `UINT32_MAX - 1`.
///
/// # Safety
/// `path` is a NUL-terminated UTF-8 string; `out` is a valid pointer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn subetha_handle_table_create(path: *const c_char, capacity: u32, mode: u32, out: *mut subetha_handle) -> i32 {
    entry(|| {
        let (path, capacity, mode) = match unsafe { read_arguments(path, capacity, mode, out) } {
            Ok(a) => a,
            Err(code) => return code,
        };
        place(SharedHandleTable::create(path, capacity), mode, out)
    })
}

/// Attach to the handle table another process made at `path`, with the
/// capacity it was made with; the file must exist.
///
/// # Safety
/// `path` is a NUL-terminated UTF-8 string; `out` is a valid pointer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn subetha_handle_table_open(path: *const c_char, capacity: u32, mode: u32, out: *mut subetha_handle) -> i32 {
    entry(|| {
        let (path, capacity, mode) = match unsafe { read_arguments(path, capacity, mode, out) } {
            Ok(a) => a,
            Err(code) => return code,
        };
        place(SharedHandleTable::open(path, capacity), mode, out)
    })
}

/// Truncate the file at `path` and initialize an empty table there, so
/// every handle a live peer holds stops naming anything. On Windows the
/// file must not be mapped by any handle.
///
/// # Safety
/// `path` is a NUL-terminated UTF-8 string; `out` is a valid pointer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn subetha_handle_table_reset(path: *const c_char, capacity: u32, mode: u32, out: *mut subetha_handle) -> i32 {
    entry(|| {
        let (path, capacity, mode) = match unsafe { read_arguments(path, capacity, mode, out) } {
            Ok(a) => a,
            Err(code) => return code,
        };
        place(SharedHandleTable::reset(path, capacity), mode, out)
    })
}

/// Put the `len` bytes at `value` in the table and write the handle that
/// names them into `out_handle`. `SUBETHA_E_MAP_FULL` when every slot
/// holds a live value; more than `SUBETHA_HANDLE_TABLE_VALUE_MAX` bytes is
/// `SUBETHA_E_INVALID_ARGUMENT`.
///
/// # Safety
/// `value` is null or addresses `len` readable bytes; `out_handle` is a
/// valid pointer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn subetha_handle_table_insert(table: subetha_handle, value: *const u8, len: usize, out_handle: *mut u64) -> i32 {
    with_table(table, |t| {
        let held = match unsafe { table_value(value, len) } {
            Ok(v) => v,
            Err(code) => return code,
        };
        if out_handle.is_null() {
            return fail(SUBETHA_E_INVALID_ARGUMENT, "out_handle is null");
        }
        match t.table.insert(held) {
            Ok(handle) => {
                // SAFETY: checked non-null; the caller guarantees it is writable.
                unsafe { *out_handle = handle.raw() };
                SUBETHA_OK
            }
            Err(e) => handle_table_code(e),
        }
    })
}

/// The value `handle` names, copied into `out`, which holds `out_cap`
/// bytes, with its length written into `out_len`.
/// `SUBETHA_E_MAP_KEY_ABSENT` when the handle names nothing, leaving `out`
/// and `out_len` as they were; `SUBETHA_E_BUFFER_TOO_SMALL` when `out` is
/// null or shorter than the value, with the length it needs written into
/// `out_len` and nothing copied. A buffer of
/// `SUBETHA_HANDLE_TABLE_VALUE_MAX` bytes holds any value.
///
/// # Safety
/// `out` is null or addresses `out_cap` writable bytes; `out_len` is a
/// valid pointer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn subetha_handle_table_get(
    table: subetha_handle,
    handle: u64,
    out: *mut u8,
    out_cap: usize,
    out_len: *mut usize,
) -> i32 {
    with_table(table, |t| {
        if out_len.is_null() {
            return fail(SUBETHA_E_INVALID_ARGUMENT, "out_len is null");
        }
        let found = match t.value(handle) {
            Ok(v) => v,
            Err(code) => return code,
        };
        match unsafe { copy_out(found.as_bytes(), out, out_cap, out_len) } {
            Ok(()) => SUBETHA_OK,
            Err(code) => code,
        }
    })
}

/// Take the value `handle` names out of the table, copying it into `out`
/// as `subetha_handle_table_get` copies it, so the handle and every copy
/// of it stop naming anything. The refusals are `get`'s, and each leaves
/// the value in the table: a buffer too short for it is refused before
/// anything is taken out.
///
/// # Safety
/// `out` is null or addresses `out_cap` writable bytes; `out_len` is a
/// valid pointer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn subetha_handle_table_remove(
    table: subetha_handle,
    handle: u64,
    out: *mut u8,
    out_cap: usize,
    out_len: *mut usize,
) -> i32 {
    with_table(table, |t| {
        if out_len.is_null() {
            return fail(SUBETHA_E_INVALID_ARGUMENT, "out_len is null");
        }
        let found = match t.value(handle) {
            Ok(v) => v,
            Err(code) => return code,
        };
        let needed = found.as_bytes().len();
        if out.is_null() || out_cap < needed {
            // SAFETY: checked non-null; the caller guarantees it is writable.
            unsafe { *out_len = needed };
            return fail(SUBETHA_E_BUFFER_TOO_SMALL, format!("the value is {needed} bytes, {out_cap} given"));
        }
        // A handle names one value for its whole life, so what the remove
        // takes out is what the lookup saw, or nothing when another caller
        // removed it in between.
        match t.table.remove(handle_of(handle)) {
            Some(taken) => match unsafe { copy_out(taken.as_bytes(), out, out_cap, out_len) } {
                Ok(()) => SUBETHA_OK,
                Err(code) => code,
            },
            None => absent(handle),
        }
    })
}

/// Whether `handle` still names a value, into `out`.
///
/// # Safety
/// `out` is a valid pointer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn subetha_handle_table_contains(table: subetha_handle, handle: u64, out: *mut bool) -> i32 {
    with_table(table, |t| {
        if out.is_null() {
            return fail(SUBETHA_E_INVALID_ARGUMENT, "out is null");
        }
        // SAFETY: checked non-null; the caller guarantees it is writable.
        unsafe { *out = t.table.contains(handle_of(handle)) };
        SUBETHA_OK
    })
}

/// Put `count` values in under one handle lookup and one panic guard, the
/// first at `values` and each next one `stride` bytes on, value `i` being
/// `lens[i]` bytes, writing each one's handle into `out_handles`. The
/// batch stops at the first value refused, `SUBETHA_E_MAP_FULL` being the
/// one to expect, returns `SUBETHA_OK` when any went in, and `out_done`
/// counts them; `out_handles` past that is left as it was.
///
/// # Safety
/// `values` addresses `count * stride` readable bytes, `lens` `count`
/// readable lengths and `out_handles` `count` writable words; `out_done`
/// is a valid pointer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn subetha_handle_table_insert_many(
    table: subetha_handle,
    values: *const u8,
    stride: usize,
    lens: *const usize,
    count: usize,
    out_handles: *mut u64,
    out_done: *mut usize,
) -> i32 {
    with_table(table, |t| {
        // SAFETY: the caller guarantees the arrays and the count word.
        unsafe {
            run_push_many_lens_reporting(values, stride, lens, count, out_handles, out_done, |payload| {
                let held = LeaseValue::from_bytes(payload).map_err(|e| fail(SUBETHA_E_INVALID_ARGUMENT, e.to_string()))?;
                t.table.insert(held).map(|handle| handle.raw()).map_err(handle_table_code)
            })
        }
    })
}

/// Look up `count` handles under one handle lookup and one panic guard:
/// `out_present[i]` receives whether `handles[i]` names a value and, when
/// it does, the value is copied to `out` plus `i * stride` and its length
/// written into `out_lens[i]`; an absent handle's slot and length are left
/// as they were. Every handle is looked up, present or not. `stride` must
/// be at least `SUBETHA_HANDLE_TABLE_VALUE_MAX`, so every value fits.
///
/// # Safety
/// `handles` addresses `count` readable words, `out` `count * stride`
/// writable bytes, `out_lens` `count` writable lengths and `out_present`
/// `count` writable flags.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn subetha_handle_table_get_many(
    table: subetha_handle,
    handles: *const u64,
    count: usize,
    out: *mut u8,
    stride: usize,
    out_lens: *mut usize,
    out_present: *mut bool,
) -> i32 {
    with_table(table, |t| {
        if count == 0 {
            return SUBETHA_OK;
        }
        if handles.is_null() || out.is_null() || out_lens.is_null() || out_present.is_null() {
            return fail(SUBETHA_E_INVALID_ARGUMENT, "handles, out, out_lens or out_present is null with a non-zero count");
        }
        if stride < SUBETHA_HANDLE_TABLE_VALUE_MAX {
            return fail(
                SUBETHA_E_INVALID_ARGUMENT,
                format!("a stride of {stride} does not span a value of up to {SUBETHA_HANDLE_TABLE_VALUE_MAX} bytes"),
            );
        }
        if stride.checked_mul(count).is_none() {
            return fail(
                SUBETHA_E_INVALID_ARGUMENT,
                format!("a stride of {stride} over {count} values overruns this platform's address space"),
            );
        }
        for i in 0..count {
            // SAFETY: `i` is below `count`, which the caller guarantees is
            // readable at `handles` and writable at every output.
            unsafe {
                let found = t.table.get(handle_of(*handles.add(i)));
                if let Some(value) = found {
                    let read = value.as_bytes();
                    std::ptr::copy_nonoverlapping(read.as_ptr(), out.add(i * stride), read.len());
                    *out_lens.add(i) = read.len();
                }
                *out_present.add(i) = found.is_some();
            }
        }
        SUBETHA_OK
    })
}

/// A snapshot of the table into `out`.
///
/// # Safety
/// `out` is a valid pointer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn subetha_handle_table_read_stats(table: subetha_handle, out: *mut subetha_handle_table_stats) -> i32 {
    with_table(table, |t| {
        if out.is_null() {
            return fail(SUBETHA_E_INVALID_ARGUMENT, "out is null");
        }
        let stats = subetha_handle_table_stats {
            mode: t.mode,
            capacity: t.table.capacity() as u64,
            len: t.table.len() as u64,
        };
        // SAFETY: checked non-null; the caller guarantees it is writable.
        unsafe { *out = stats };
        SUBETHA_OK
    })
}

/// Push the mapped bytes to disk and wait for them.
#[unsafe(no_mangle)]
pub extern "C" fn subetha_handle_table_flush(table: subetha_handle) -> i32 {
    with_table(table, |t| match t.table.flush() {
        Ok(()) => SUBETHA_OK,
        Err(e) => handle_table_code(e),
    })
}

/// Ask for the mapped bytes to reach disk without waiting.
#[unsafe(no_mangle)]
pub extern "C" fn subetha_handle_table_flush_async(table: subetha_handle) -> i32 {
    with_table(table, |t| match t.table.flush_async() {
        Ok(()) => SUBETHA_OK,
        Err(e) => handle_table_code(e),
    })
}

/// Remove the table file at `path`. The contract is `subetha_ring_unlink`'s.
///
/// # Safety
/// `path` is a NUL-terminated UTF-8 string; `report` is null or a valid
/// pointer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn subetha_handle_table_unlink(path: *const c_char, report: *mut subetha_unlink_report) -> i32 {
    entry(|| {
        if let Err(code) = require_initialized() {
            return code;
        }
        let path = match unsafe { text(path, "path") } {
            Ok(p) => PathBuf::from(p),
            Err(code) => return code,
        };
        let mut found = UnlinkReport::default();
        found.remove(path);
        unsafe { finish_unlink(found, report) }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::{SUBETHA_E_LAYOUT_MISMATCH, SUBETHA_E_MAP_FULL};
    use crate::runtime::{subetha_handle_destroy, subetha_init, SUBETHA_MODE_STRICT};
    use std::ffi::CString;

    fn scratch(name: &str) -> (PathBuf, CString) {
        let path = std::env::temp_dir().join(format!("subetha-ffi-handle-table-{name}-{}.bin", std::process::id()));
        let c_path = CString::new(path.to_str().expect("a UTF-8 temp path")).expect("no NUL in the path");
        (path, c_path)
    }

    /// A value goes in and comes back by its handle, Rust reads the same
    /// table at the same path, a removed handle stays dead after its slot
    /// is reused, a short buffer is told the length it needs without the
    /// value leaving the table, and a full table is refused.
    #[test]
    fn a_handle_names_one_value_for_its_life() {
        assert_eq!(subetha_init(SUBETHA_MODE_STRICT), SUBETHA_OK);
        let (path, c_path) = scratch("life");
        let (mut table, mut other) = (0, 0);
        let (mut alpha, mut beta, mut gamma) = (0u64, 0u64, 0u64);
        let mut buf = [0u8; SUBETHA_HANDLE_TABLE_VALUE_MAX];
        let mut len = 0usize;
        let mut live = false;
        // SAFETY: valid paths, buffers and out pointers throughout.
        unsafe {
            assert_eq!(subetha_handle_table_create(c_path.as_ptr(), 2, SUBETHA_MODE_STRICT, &mut table), SUBETHA_OK);
            assert_eq!(subetha_handle_table_insert(table, b"alpha".as_ptr(), 5, &mut alpha), SUBETHA_OK);
            assert_ne!(alpha, 0, "zero is never a handle");
            assert_eq!(subetha_handle_table_get(table, alpha, buf.as_mut_ptr(), buf.len(), &mut len), SUBETHA_OK);
            assert_eq!(&buf[..len], b"alpha");
            let long = [7u8; SUBETHA_HANDLE_TABLE_VALUE_MAX + 1];
            assert_eq!(subetha_handle_table_insert(table, long.as_ptr(), long.len(), &mut beta), SUBETHA_E_INVALID_ARGUMENT);
            assert_eq!(subetha_handle_table_insert(table, b"beta".as_ptr(), 4, &mut beta), SUBETHA_OK);
            assert_eq!(subetha_handle_table_insert(table, b"x".as_ptr(), 1, &mut gamma), SUBETHA_E_MAP_FULL);

            let rust = SharedHandleTable::<LeaseValue>::open(&path, 2).expect("Rust attaches to the same table");
            assert_eq!(rust.get(handle_of(beta)).map(|v| v.as_bytes().to_vec()), Some(b"beta".to_vec()));
            drop(rust);

            let mut short = [0u8; 2];
            assert_eq!(subetha_handle_table_get(table, alpha, short.as_mut_ptr(), short.len(), &mut len), SUBETHA_E_BUFFER_TOO_SMALL);
            assert_eq!((len, short), (5, [0, 0]), "the length it needs, nothing copied");
            len = 0;
            assert_eq!(subetha_handle_table_remove(table, alpha, short.as_mut_ptr(), short.len(), &mut len), SUBETHA_E_BUFFER_TOO_SMALL);
            assert_eq!(len, 5);
            assert_eq!(subetha_handle_table_contains(table, alpha, &mut live), SUBETHA_OK);
            assert!(live, "a remove refused for its buffer leaves the value in the table");

            assert_eq!(subetha_handle_table_remove(table, alpha, buf.as_mut_ptr(), buf.len(), &mut len), SUBETHA_OK);
            assert_eq!(&buf[..len], b"alpha");
            assert_eq!(subetha_handle_table_insert(table, b"gamma".as_ptr(), 5, &mut gamma), SUBETHA_OK);
            assert_eq!(gamma as u32, alpha as u32, "gamma reuses alpha's slot");
            assert_ne!(gamma, alpha, "under a new generation");
            len = 99;
            assert_eq!(subetha_handle_table_get(table, alpha, buf.as_mut_ptr(), buf.len(), &mut len), SUBETHA_E_MAP_KEY_ABSENT);
            assert_eq!(len, 99, "an absent handle leaves out_len alone");
            assert_eq!(subetha_handle_table_remove(table, alpha, buf.as_mut_ptr(), buf.len(), &mut len), SUBETHA_E_MAP_KEY_ABSENT);
            assert_eq!(subetha_handle_table_get(table, 0, buf.as_mut_ptr(), buf.len(), &mut len), SUBETHA_E_MAP_KEY_ABSENT);
            assert_eq!(subetha_handle_table_contains(table, alpha, &mut live), SUBETHA_OK);
            assert!(!live);
            assert_eq!(subetha_handle_table_get(table, gamma, buf.as_mut_ptr(), buf.len(), &mut len), SUBETHA_OK);
            assert_eq!(&buf[..len], b"gamma");

            let mut stats = subetha_handle_table_stats::default();
            assert_eq!(subetha_handle_table_read_stats(table, &mut stats), SUBETHA_OK);
            assert_eq!((stats.mode, stats.capacity, stats.len), (SUBETHA_MODE_STRICT, 2, 2));
            assert_eq!(subetha_handle_table_flush(table), SUBETHA_OK);
            assert_eq!(subetha_handle_table_flush_async(table), SUBETHA_OK);

            assert_eq!(subetha_handle_table_create(c_path.as_ptr(), 0, SUBETHA_MODE_STRICT, &mut other), SUBETHA_E_INVALID_ARGUMENT);
            assert_eq!(subetha_handle_table_create(c_path.as_ptr(), u32::MAX, SUBETHA_MODE_STRICT, &mut other), SUBETHA_E_INVALID_ARGUMENT);
            assert_eq!(subetha_handle_table_open(c_path.as_ptr(), 4, SUBETHA_MODE_STRICT, &mut other), SUBETHA_E_LAYOUT_MISMATCH);
            assert_eq!(subetha_handle_destroy(table), SUBETHA_OK);
            let mut report = subetha_unlink_report::default();
            assert_eq!(subetha_handle_table_unlink(c_path.as_ptr(), &mut report), SUBETHA_OK);
            assert_eq!((report.removed, report.failed), (1, 0));
        }
    }

    /// A batch insert stops at a full table and reports the handles that
    /// went in; a batch get answers every handle, present or not.
    #[test]
    fn the_batches_stop_at_a_full_table_and_answer_every_handle() {
        assert_eq!(subetha_init(SUBETHA_MODE_STRICT), SUBETHA_OK);
        let (_path, c_path) = scratch("batch");
        let mut table = 0;
        const STRIDE: usize = 8;
        let values: [u8; 3 * STRIDE] = *b"a\0\0\0\0\0\0\0bb\0\0\0\0\0\0ccc\0\0\0\0\0";
        let lens = [1usize, 2, 3];
        let mut handles = [0u64; 3];
        let mut done = 0usize;
        let mut got = [0u8; 3 * SUBETHA_HANDLE_TABLE_VALUE_MAX];
        let mut got_lens = [usize::MAX; 3];
        let mut present = [true; 3];
        // SAFETY: valid paths, arrays and out pointers throughout.
        unsafe {
            assert_eq!(subetha_handle_table_create(c_path.as_ptr(), 2, SUBETHA_MODE_STRICT, &mut table), SUBETHA_OK);
            assert_eq!(
                subetha_handle_table_insert_many(table, values.as_ptr(), STRIDE, lens.as_ptr(), 3, handles.as_mut_ptr(), &mut done),
                SUBETHA_OK
            );
            assert_eq!(done, 2, "two slots take two values");
            assert_eq!(handles[2], 0, "past what went in is left alone");
            assert_eq!(
                subetha_handle_table_insert_many(table, values.as_ptr(), STRIDE, lens.as_ptr(), 1, handles.as_mut_ptr(), &mut done),
                SUBETHA_E_MAP_FULL
            );
            assert_eq!(done, 0);

            let asked = [handles[1], 0, handles[0]];
            assert_eq!(
                subetha_handle_table_get_many(
                    table,
                    asked.as_ptr(),
                    3,
                    got.as_mut_ptr(),
                    SUBETHA_HANDLE_TABLE_VALUE_MAX,
                    got_lens.as_mut_ptr(),
                    present.as_mut_ptr()
                ),
                SUBETHA_OK
            );
            assert_eq!(present, [true, false, true]);
            assert_eq!(got_lens, [2, usize::MAX, 1], "an absent handle's length is left alone");
            assert_eq!(&got[..2], b"bb");
            assert_eq!(&got[2 * SUBETHA_HANDLE_TABLE_VALUE_MAX..2 * SUBETHA_HANDLE_TABLE_VALUE_MAX + 1], b"a");
            assert_eq!(
                subetha_handle_table_get_many(table, asked.as_ptr(), 3, got.as_mut_ptr(), 10, got_lens.as_mut_ptr(), present.as_mut_ptr()),
                SUBETHA_E_INVALID_ARGUMENT,
                "a stride shorter than the largest value is refused"
            );
            assert_eq!(subetha_handle_destroy(table), SUBETHA_OK);
            assert_eq!(subetha_handle_table_unlink(c_path.as_ptr(), std::ptr::null_mut()), SUBETHA_OK);
        }
    }
}
