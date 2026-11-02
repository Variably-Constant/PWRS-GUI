//! The front door's key-value map through the C ABI: 64-bit keys mapped to
//! 64-bit values in a file every process inserts into and looks up, with
//! the shape underneath picked from the readers and writers named. It is
//! the map Python's `KvMap` and PowerShell's `New-SubEthaKvMap` make, so a
//! map any of them made at a path is the same map to C. A key is never
//! taken out, only written over; `subetha_hashmap_` is the map whose
//! entries go away. The map runs no background work, so strict and managed
//! modes are the same.

use std::ffi::c_char;
use std::path::{Path, PathBuf};

use subetha_cxc::adaptive_ring::UnlinkReport;
use subetha_cxc::api::{AutoIpc, KvMap};
use subetha_cxc::shared_hash_map::InsertOutcome;

use crate::batch::{batch_arguments, batch_result};
use crate::error::{api_code, fail, SUBETHA_E_INVALID_ARGUMENT, SUBETHA_E_MAP_KEY_ABSENT, SUBETHA_E_WRONG_KIND, SUBETHA_OK};
use crate::handle::{subetha_handle, Object, SUBETHA_KIND_KV_MAP};
use crate::ring::{finish_unlink, subetha_unlink_report, text};
use crate::runtime::{entry, issue, require_initialized, resolve_mode, with_kind};

/// A snapshot of a key-value map.
#[repr(C)]
#[allow(non_camel_case_types)]
#[derive(Clone, Copy, Default)]
pub struct subetha_kv_map_stats {
    /// The mode this handle was created in.
    pub mode: u32,
    /// Slots in the table: the capacity asked for, rounded up to a power of
    /// two of at least 2.
    pub capacity: u64,
    /// Keys the map holds.
    pub len: u64,
}

pub(crate) struct KvMapObject {
    map: KvMap<u64, u64>,
    mode: u32,
    capacity: usize,
}

impl KvMapObject {
    /// A map parks nothing inside a call, so a destroy has nobody to wake.
    pub(crate) fn interrupt(&self) {}

    /// Insert or write over, answering whether the key was new.
    fn insert(&self, key: u64, value: u64) -> Result<bool, i32> {
        self.map
            .insert(key, value)
            .map(|outcome| matches!(outcome, InsertOutcome::Inserted))
            .map_err(api_code)
    }
}

fn with_kv_map(handle: subetha_handle, f: impl FnOnce(&KvMapObject) -> i32) -> i32 {
    with_kind(handle, SUBETHA_KIND_KV_MAP, |object| match object {
        Object::KvMap(m) => f(m),
        _ => fail(SUBETHA_E_WRONG_KIND, "the handle does not name a key-value map"),
    })
}

/// The slot count a requested `capacity` makes: at least 2 and a power of
/// two, rounded up as the Python and PowerShell maps round it, so the same
/// request reaches the same file from any of them.
fn map_capacity(capacity: u32) -> Result<usize, i32> {
    if capacity == 0 {
        return Err(fail(SUBETHA_E_INVALID_ARGUMENT, "a map's capacity must be at least 1"));
    }
    (capacity as usize).max(2).checked_next_power_of_two().ok_or_else(|| {
        fail(SUBETHA_E_INVALID_ARGUMENT, format!("capacity {capacity} rounds past what this platform addresses"))
    })
}

/// Obtain the map at `path` with `capacity` slots, rounded up to a power of
/// two of at least 2 as the Python and PowerShell maps round it, so the same
/// request reaches the same file. An empty map is initialized when the file
/// does not exist and an existing one is attached with its keys in place;
/// a file of another capacity is `SUBETHA_E_LAYOUT_MISMATCH`. `readers` and
/// `writers` are how many of each take part, and the shape underneath is
/// picked from them; zero of either is refused. Size the map at twice the
/// keys it will hold: the probe chain saturates past a load factor of 0.7.
///
/// # Safety
/// `path` is a NUL-terminated UTF-8 string; `out` is a valid pointer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn subetha_kv_map_create(
    path: *const c_char,
    capacity: u32,
    readers: u32,
    writers: u32,
    mode: u32,
    out: *mut subetha_handle,
) -> i32 {
    entry(|| {
        if let Err(code) = require_initialized() {
            return code;
        }
        if out.is_null() {
            return fail(SUBETHA_E_INVALID_ARGUMENT, "out is null");
        }
        let path = match unsafe { text(path, "path") } {
            Ok(p) => Path::new(p),
            Err(code) => return code,
        };
        let capacity = match map_capacity(capacity) {
            Ok(c) => c,
            Err(code) => return code,
        };
        if readers == 0 || writers == 0 {
            return fail(SUBETHA_E_INVALID_ARGUMENT, "a map needs at least one reader and one writer");
        }
        let mode = match resolve_mode(mode) {
            Ok(m) => m,
            Err(code) => return code,
        };
        let built = AutoIpc::new(path)
            .consumers(readers as usize)
            .producers(writers as usize)
            .capacity(capacity)
            .build_kv_map::<u64, u64>();
        match built {
            Ok(map) => unsafe { issue(Object::KvMap(KvMapObject { map, mode, capacity }), out) },
            Err(e) => api_code(e),
        }
    })
}

/// Insert `value` under `key`, or write it over what the key held;
/// `out_inserted`, when not null, receives true when the key was not there
/// before. `SUBETHA_E_MAP_FULL` when no slot is free.
///
/// # Safety
/// `out_inserted` is null or a valid pointer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn subetha_kv_map_insert(handle: subetha_handle, key: u64, value: u64, out_inserted: *mut bool) -> i32 {
    with_kv_map(handle, |m| match m.insert(key, value) {
        Ok(fresh) => {
            if !out_inserted.is_null() {
                // SAFETY: checked non-null; the caller guarantees it is writable.
                unsafe { *out_inserted = fresh };
            }
            SUBETHA_OK
        }
        Err(code) => code,
    })
}

/// The value `key` holds, into `out`. `SUBETHA_E_MAP_KEY_ABSENT` when the
/// key holds nothing, leaving `out` as it was.
///
/// # Safety
/// `out` is a valid pointer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn subetha_kv_map_get(handle: subetha_handle, key: u64, out: *mut u64) -> i32 {
    with_kv_map(handle, |m| {
        if out.is_null() {
            return fail(SUBETHA_E_INVALID_ARGUMENT, "out is null");
        }
        match m.map.get(&key) {
            Some(value) => {
                // SAFETY: checked non-null; the caller guarantees it is writable.
                unsafe { *out = value };
                SUBETHA_OK
            }
            None => SUBETHA_E_MAP_KEY_ABSENT,
        }
    })
}

/// Insert `count` entries, `values[i]` under `keys[i]`, under one handle
/// lookup and one panic guard; `out_inserted`, when not null, receives true
/// at `i` when `keys[i]` was not there before. The batch stops at the first
/// entry refused, `SUBETHA_E_MAP_FULL` being the one to expect, returns
/// `SUBETHA_OK` when any went in, and `out_done` counts them.
///
/// # Safety
/// `keys` and `values` address `count` readable words, `out_inserted` is
/// null or addresses `count` writable flags, and `out_done` is a valid
/// pointer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn subetha_kv_map_insert_many(
    handle: subetha_handle,
    keys: *const u64,
    values: *const u64,
    count: usize,
    out_inserted: *mut bool,
    out_done: *mut usize,
) -> i32 {
    with_kv_map(handle, |m| {
        if let Err(code) = batch_arguments(keys.is_null(), 1, 1, count, out_done.is_null()) {
            return code;
        }
        if count != 0 && values.is_null() {
            return fail(SUBETHA_E_INVALID_ARGUMENT, "values is null with a non-zero count");
        }
        let mut done = 0usize;
        let mut refusal = None;
        while done < count {
            // SAFETY: `done` is below `count`, which the caller guarantees
            // is readable at both arrays.
            let (key, value) = unsafe { (*keys.add(done), *values.add(done)) };
            match m.insert(key, value) {
                Ok(fresh) => {
                    if !out_inserted.is_null() {
                        // SAFETY: the caller guarantees `count` flags at `out_inserted`.
                        unsafe { *out_inserted.add(done) = fresh };
                    }
                    done += 1;
                }
                Err(code) => {
                    refusal = Some(code);
                    break;
                }
            }
        }
        // SAFETY: `batch_arguments` refused a null `out_done`.
        unsafe { batch_result(done, refusal, out_done) }
    })
}

/// Look up `count` keys under one handle lookup and one panic guard:
/// `out_present[i]` receives whether `keys[i]` holds a value and, when it
/// does, `out_values[i]` receives it; the value slot of an absent key is
/// left as it was. Every key is looked up, present or not.
///
/// # Safety
/// `keys` addresses `count` readable words, `out_values` `count` writable
/// words and `out_present` `count` writable flags.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn subetha_kv_map_get_many(
    handle: subetha_handle,
    keys: *const u64,
    count: usize,
    out_values: *mut u64,
    out_present: *mut bool,
) -> i32 {
    with_kv_map(handle, |m| {
        if count == 0 {
            return SUBETHA_OK;
        }
        if keys.is_null() || out_values.is_null() || out_present.is_null() {
            return fail(SUBETHA_E_INVALID_ARGUMENT, "keys, out_values or out_present is null with a non-zero count");
        }
        for i in 0..count {
            // SAFETY: `i` is below `count`, which the caller guarantees is
            // readable at `keys` and writable at both outputs.
            unsafe {
                let found = m.map.get(&*keys.add(i));
                if let Some(value) = found {
                    *out_values.add(i) = value;
                }
                *out_present.add(i) = found.is_some();
            }
        }
        SUBETHA_OK
    })
}

/// A snapshot of the map into `out`.
///
/// # Safety
/// `out` is a valid pointer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn subetha_kv_map_read_stats(handle: subetha_handle, out: *mut subetha_kv_map_stats) -> i32 {
    with_kv_map(handle, |m| {
        if out.is_null() {
            return fail(SUBETHA_E_INVALID_ARGUMENT, "out is null");
        }
        let stats = subetha_kv_map_stats { mode: m.mode, capacity: m.capacity as u64, len: m.map.len() as u64 };
        // SAFETY: checked non-null; the caller guarantees it is writable.
        unsafe { *out = stats };
        SUBETHA_OK
    })
}

/// Remove the map file at `path`. The contract is `subetha_ring_unlink`'s.
///
/// # Safety
/// `path` is a NUL-terminated UTF-8 string; `report` is null or a valid
/// pointer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn subetha_kv_map_unlink(path: *const c_char, report: *mut subetha_unlink_report) -> i32 {
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
        let path = std::env::temp_dir().join(format!("subetha-ffi-kv-map-{name}-{}.bin", std::process::id()));
        let c_path = CString::new(path.to_str().expect("a UTF-8 temp path")).expect("no NUL in the path");
        (path, c_path)
    }

    /// A map made from C and one Rust's front door builds at the same path
    /// and capacity are one map; a capacity rounds up so an attach asking
    /// for the same number reaches the file and another capacity is
    /// refused; an insert says whether the key was new; and a map fills.
    #[test]
    fn a_c_map_rounds_attaches_answers_and_fills() {
        assert_eq!(subetha_init(SUBETHA_MODE_STRICT), SUBETHA_OK);
        let (path, c_path) = scratch("entry");
        let (mut map, mut again, mut other) = (0, 0, 0);
        let mut fresh = false;
        let mut value = 0u64;
        // SAFETY: valid paths and out pointers throughout.
        unsafe {
            assert_eq!(subetha_kv_map_create(c_path.as_ptr(), 3, 1, 1, SUBETHA_MODE_STRICT, &mut map), SUBETHA_OK);
            assert_eq!(subetha_kv_map_insert(map, 7, 70, &mut fresh), SUBETHA_OK);
            assert!(fresh, "the first insert of a key is new");
            assert_eq!(subetha_kv_map_insert(map, 7, 71, &mut fresh), SUBETHA_OK);
            assert!(!fresh, "a second insert writes over");
            assert_eq!(subetha_kv_map_insert(map, 8, 80, std::ptr::null_mut()), SUBETHA_OK);
            assert_eq!(subetha_kv_map_get(map, 7, &mut value), SUBETHA_OK);
            assert_eq!(value, 71);
            assert_eq!(subetha_kv_map_get(map, 9, &mut value), SUBETHA_E_MAP_KEY_ABSENT);
            assert_eq!(value, 71, "an absent key leaves the output alone");

            let rust = AutoIpc::new(&path).capacity(4).build_kv_map::<u64, u64>().expect("Rust attaches at the rounded capacity");
            assert_eq!(rust.get(&8), Some(80));
            drop(rust);
            assert_eq!(subetha_kv_map_create(c_path.as_ptr(), 4, 1, 1, SUBETHA_MODE_STRICT, &mut again), SUBETHA_OK);
            assert_eq!(subetha_handle_destroy(again), SUBETHA_OK);
            assert_eq!(subetha_kv_map_create(c_path.as_ptr(), 64, 1, 1, SUBETHA_MODE_STRICT, &mut other), SUBETHA_E_LAYOUT_MISMATCH);
            assert_eq!(subetha_kv_map_create(c_path.as_ptr(), 0, 1, 1, SUBETHA_MODE_STRICT, &mut other), SUBETHA_E_INVALID_ARGUMENT);
            assert_eq!(subetha_kv_map_create(c_path.as_ptr(), 4, 0, 1, SUBETHA_MODE_STRICT, &mut other), SUBETHA_E_INVALID_ARGUMENT);

            let mut stats = subetha_kv_map_stats::default();
            assert_eq!(subetha_kv_map_read_stats(map, &mut stats), SUBETHA_OK);
            assert_eq!((stats.mode, stats.capacity, stats.len), (SUBETHA_MODE_STRICT, 4, 2));

            let mut status = SUBETHA_OK;
            for key in 100..104u64 {
                status = subetha_kv_map_insert(map, key, key, std::ptr::null_mut());
                if status != SUBETHA_OK {
                    break;
                }
            }
            assert_eq!(status, SUBETHA_E_MAP_FULL, "four slots take at most four keys");
            assert_eq!(subetha_handle_destroy(map), SUBETHA_OK);
            let mut report = subetha_unlink_report::default();
            assert_eq!(subetha_kv_map_unlink(c_path.as_ptr(), &mut report), SUBETHA_OK);
            assert_eq!((report.removed, report.failed), (1, 0));
        }
    }

    /// A batch insert flags each new key and stops at the first entry the
    /// map refuses; a batch get answers every key, present or not.
    #[test]
    fn the_batches_flag_new_keys_and_answer_every_key() {
        assert_eq!(subetha_init(SUBETHA_MODE_STRICT), SUBETHA_OK);
        let (_path, c_path) = scratch("batch");
        let mut map = 0;
        let keys = [1u64, 2, 1];
        let values = [10u64, 20, 11];
        let mut fresh = [false; 3];
        let mut done = 0usize;
        let mut got = [u64::MAX; 3];
        let mut present = [true; 3];
        // SAFETY: valid paths, arrays and out pointers throughout.
        unsafe {
            assert_eq!(subetha_kv_map_create(c_path.as_ptr(), 8, 1, 1, SUBETHA_MODE_STRICT, &mut map), SUBETHA_OK);
            assert_eq!(subetha_kv_map_insert_many(map, keys.as_ptr(), values.as_ptr(), 3, fresh.as_mut_ptr(), &mut done), SUBETHA_OK);
            assert_eq!(done, 3);
            assert_eq!(fresh, [true, true, false], "the third writes over the first");
            let asked = [1u64, 3, 2];
            assert_eq!(subetha_kv_map_get_many(map, asked.as_ptr(), 3, got.as_mut_ptr(), present.as_mut_ptr()), SUBETHA_OK);
            assert_eq!(present, [true, false, true]);
            assert_eq!(got, [11, u64::MAX, 20], "an absent key's slot is left alone");
            assert_eq!(
                subetha_kv_map_insert_many(map, keys.as_ptr(), std::ptr::null(), 3, std::ptr::null_mut(), &mut done),
                SUBETHA_E_INVALID_ARGUMENT
            );
            assert_eq!(subetha_handle_destroy(map), SUBETHA_OK);
            assert_eq!(subetha_kv_map_unlink(c_path.as_ptr(), std::ptr::null_mut()), SUBETHA_OK);
        }
    }
}
