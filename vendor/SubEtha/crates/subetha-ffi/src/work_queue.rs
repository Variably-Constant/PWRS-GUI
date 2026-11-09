//! The front door's work queue through the C ABI: work one process owns and
//! others take from when they are idle. The owner pushes and pops at one
//! end of a work-stealing deque, the end it has to itself; every other
//! handle steals from the far end. An item is up to
//! `SUBETHA_WORK_QUEUE_ITEM_MAX` bytes and keeps its length, encoded as
//! Python's `WorkQueue` and PowerShell's `New-SubEthaWorkQueue` encode it,
//! so a queue any of them made at a path is the same queue to C. The queue
//! runs no background work, so strict and managed modes are the same.

use std::ffi::c_char;
use std::path::{Path, PathBuf};

use subetha_cxc::adaptive_ring::UnlinkReport;
use subetha_cxc::api::{ApiError, AutoIpc, WorkStealQueue};
use subetha_cxc::byte_value::{SlotValue, SLOT_VALUE_BYTES};

use crate::batch::{run_pop_many_indexed, run_push_many_lens};
use crate::deque::{SUBETHA_DEQUE_OWNER, SUBETHA_DEQUE_THIEF};
use crate::error::{
    api_code, fail, SUBETHA_E_DEQUE_NOT_OWNER, SUBETHA_E_INVALID_ARGUMENT, SUBETHA_E_RING_EMPTY,
    SUBETHA_E_RING_PAYLOAD_TOO_LARGE, SUBETHA_E_WRONG_KIND, SUBETHA_OK,
};
use crate::handle::{subetha_handle, Object, SUBETHA_KIND_WORK_QUEUE};
use crate::ring::{bytes, finish_unlink, out_buffer, subetha_unlink_report, text};
use crate::runtime::{entry, issue, require_initialized, resolve_mode, with_kind};

/// The most an item may be, in bytes. A pop or steal buffer must be at
/// least this large.
pub const SUBETHA_WORK_QUEUE_ITEM_MAX: usize = 52;

const _: () = assert!(SUBETHA_WORK_QUEUE_ITEM_MAX == SLOT_VALUE_BYTES);

/// A snapshot of a work queue handle.
#[repr(C)]
#[allow(non_camel_case_types)]
#[derive(Clone, Copy, Default)]
pub struct subetha_work_queue_stats {
    /// The mode this handle was created in.
    pub mode: u32,
    /// `SUBETHA_DEQUE_OWNER` for the handle that made the queue,
    /// `SUBETHA_DEQUE_THIEF` for one that attached with
    /// `subetha_work_queue_steal_from`.
    pub role: u32,
}

pub(crate) struct WorkQueueObject {
    queue: WorkStealQueue<SlotValue>,
    mode: u32,
    role: u32,
}

impl WorkQueueObject {
    /// A queue parks nothing inside a call, so a destroy has nobody to
    /// wake.
    pub(crate) fn interrupt(&self) {}

    /// Refuse the owner's end to a thief, whose push or pop would break the
    /// deque's single-owner protocol.
    fn owner_only(&self) -> Result<(), i32> {
        if self.role == SUBETHA_DEQUE_OWNER {
            Ok(())
        } else {
            Err(fail(SUBETHA_E_DEQUE_NOT_OWNER, "this handle attached to the queue as a thief"))
        }
    }

    fn push(&self, held: &SlotValue) -> i32 {
        match self.queue.push(held) {
            Ok(()) => SUBETHA_OK,
            Err(e) => api_code(e),
        }
    }
}

fn with_queue(handle: subetha_handle, f: impl FnOnce(&WorkQueueObject) -> i32) -> i32 {
    with_kind(handle, SUBETHA_KIND_WORK_QUEUE, |object| match object {
        Object::WorkQueue(q) => f(q),
        _ => fail(SUBETHA_E_WRONG_KIND, "the handle does not name a work queue"),
    })
}

/// The item the `len` bytes at `data` make.
///
/// # Safety
/// `data` is null or points to `len` readable bytes.
unsafe fn item(data: *const u8, len: usize) -> Result<SlotValue, i32> {
    let payload = unsafe { bytes(data, len) }?;
    SlotValue::from_bytes(payload).map_err(|e| fail(SUBETHA_E_RING_PAYLOAD_TOO_LARGE, e.to_string()))
}

/// Copy `held`'s bytes to the front of `buf` and its length to `out_len`,
/// or report `SUBETHA_E_RING_EMPTY` for nothing taken.
///
/// # Safety
/// `out_len` is a valid pointer, as `out_buffer` checked.
unsafe fn deliver(taken: Option<SlotValue>, buf: &mut [u8], out_len: *mut usize) -> i32 {
    match taken {
        Some(held) => {
            let payload = held.as_bytes();
            buf[..payload.len()].copy_from_slice(payload);
            // SAFETY: checked non-null; the caller guarantees it is writable.
            unsafe { *out_len = payload.len() };
            SUBETHA_OK
        }
        None => SUBETHA_E_RING_EMPTY,
    }
}

fn place(queue: Result<WorkStealQueue<SlotValue>, ApiError>, mode: u32, role: u32, out: *mut subetha_handle) -> i32 {
    match queue {
        Ok(queue) => unsafe { issue(Object::WorkQueue(WorkQueueObject { queue, mode, role }), out) },
        Err(e) => api_code(e),
    }
}

/// The checks both constructors make, in order, so the first refusal names
/// the argument at fault.
///
/// # Safety
/// `path` is a NUL-terminated UTF-8 string; `out` is a valid pointer.
unsafe fn read_arguments<'a>(path: *const c_char, mode: u32, out: *mut subetha_handle) -> Result<(&'a Path, u32), i32> {
    require_initialized()?;
    if out.is_null() {
        return Err(fail(SUBETHA_E_INVALID_ARGUMENT, "out is null"));
    }
    let path = Path::new(unsafe { text(path, "path") }?);
    let mode = resolve_mode(mode)?;
    Ok((path, mode))
}

/// Make the queue at `path` and own it, with `capacity` items in flight
/// rounded up to a power of two of at least 2 as the Python and PowerShell
/// queues round it. The file is made afresh, so the work in a queue already
/// there is lost: one process makes the queue and the rest attach with
/// `subetha_work_queue_steal_from`. `thieves` is how many are expected to
/// take from it, which the shape underneath is picked from; zero is
/// refused, and so is a capacity of zero.
///
/// # Safety
/// `path` is a NUL-terminated UTF-8 string; `out` is a valid pointer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn subetha_work_queue_create(
    path: *const c_char,
    capacity: u32,
    thieves: u32,
    mode: u32,
    out: *mut subetha_handle,
) -> i32 {
    entry(|| {
        let (path, mode) = match unsafe { read_arguments(path, mode, out) } {
            Ok(a) => a,
            Err(code) => return code,
        };
        if capacity == 0 {
            return fail(SUBETHA_E_INVALID_ARGUMENT, "a queue's capacity must be at least 1");
        }
        let Some(rounded) = (capacity as usize).max(2).checked_next_power_of_two() else {
            return fail(SUBETHA_E_INVALID_ARGUMENT, format!("capacity {capacity} rounds past what this platform addresses"));
        };
        if thieves == 0 {
            return fail(SUBETHA_E_INVALID_ARGUMENT, "a queue needs at least one thief");
        }
        // A batch hint is what turns the inference toward work-stealing,
        // as the Python and PowerShell queues build it.
        let built = AutoIpc::new(path)
            .consumers(thieves as usize)
            .batch_size(2)
            .capacity(rounded)
            .build_work_steal_queue::<SlotValue>();
        place(built, mode, SUBETHA_DEQUE_OWNER, out)
    })
}

/// Attach to the queue another process made at `path`, which must exist,
/// to steal from it. `SUBETHA_E_IO` names an absent file and
/// `SUBETHA_E_LAYOUT_MISMATCH` one another kind of queue laid out.
///
/// # Safety
/// `path` is a NUL-terminated UTF-8 string; `out` is a valid pointer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn subetha_work_queue_steal_from(path: *const c_char, mode: u32, out: *mut subetha_handle) -> i32 {
    entry(|| {
        let (path, mode) = match unsafe { read_arguments(path, mode, out) } {
            Ok(a) => a,
            Err(code) => return code,
        };
        place(WorkStealQueue::open_as_thief(path), mode, SUBETHA_DEQUE_THIEF, out)
    })
}

/// Add the `len` bytes at `data` at the owner's end. `SUBETHA_E_RING_FULL`
/// when the queue is full, `SUBETHA_E_RING_PAYLOAD_TOO_LARGE` for more than
/// `SUBETHA_WORK_QUEUE_ITEM_MAX` bytes, and `SUBETHA_E_DEQUE_NOT_OWNER` from
/// a thief.
///
/// # Safety
/// `data` is null or points to `len` readable bytes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn subetha_work_queue_push(handle: subetha_handle, data: *const u8, len: usize) -> i32 {
    with_queue(handle, |q| {
        if let Err(code) = q.owner_only() {
            return code;
        }
        match unsafe { item(data, len) } {
            Ok(held) => q.push(&held),
            Err(code) => code,
        }
    })
}

/// Take the owner's own next piece of work, the most recent one it added,
/// into `out` and its length into `out_len`. `cap` must be at least
/// `SUBETHA_WORK_QUEUE_ITEM_MAX`. `SUBETHA_E_RING_EMPTY` when there is none,
/// and `SUBETHA_E_DEQUE_NOT_OWNER` from a thief.
///
/// # Safety
/// `out` points to `cap` writable bytes; `out_len` is a valid pointer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn subetha_work_queue_pop(handle: subetha_handle, out: *mut u8, cap: usize, out_len: *mut usize) -> i32 {
    with_queue(handle, |q| {
        if let Err(code) = q.owner_only() {
            return code;
        }
        let buf = match unsafe { out_buffer(out, cap, out_len, SUBETHA_WORK_QUEUE_ITEM_MAX) } {
            Ok(b) => b,
            Err(code) => return code,
        };
        unsafe { deliver(q.queue.pop(), buf, out_len) }
    })
}

/// Take a piece of work from the far end, the oldest one there, into `out`
/// and its length into `out_len`; any handle may steal, the owner's
/// included. The buffer contract is `subetha_work_queue_pop`'s, and
/// `SUBETHA_E_RING_EMPTY` means there is none to take.
///
/// # Safety
/// `out` points to `cap` writable bytes; `out_len` is a valid pointer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn subetha_work_queue_steal(handle: subetha_handle, out: *mut u8, cap: usize, out_len: *mut usize) -> i32 {
    with_queue(handle, |q| {
        let buf = match unsafe { out_buffer(out, cap, out_len, SUBETHA_WORK_QUEUE_ITEM_MAX) } {
            Ok(b) => b,
            Err(code) => return code,
        };
        unsafe { deliver(q.queue.steal(), buf, out_len) }
    })
}

/// Add `count` items at the owner's end, the first at `items` and each next
/// one `stride` bytes on, item `i` being `lens[i]` bytes, under one handle
/// lookup and one panic guard. The batch contract is
/// `subetha_channel_try_send_many`'s; a thief is refused with
/// `SUBETHA_E_DEQUE_NOT_OWNER` before anything is pushed.
///
/// # Safety
/// `items` addresses `count * stride` readable bytes, `lens` `count`
/// readable lengths, and `out_done` is a valid pointer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn subetha_work_queue_push_many(
    handle: subetha_handle,
    items: *const u8,
    stride: usize,
    lens: *const usize,
    count: usize,
    out_done: *mut usize,
) -> i32 {
    with_queue(handle, |q| {
        // SAFETY: the caller guarantees the array, the lengths and the count word.
        unsafe {
            run_push_many_lens(items, stride, lens, count, out_done, |payload| {
                if let Err(code) = q.owner_only() {
                    return code;
                }
                match SlotValue::from_bytes(payload) {
                    Ok(held) => q.push(&held),
                    Err(e) => fail(SUBETHA_E_RING_PAYLOAD_TOO_LARGE, e.to_string()),
                }
            })
        }
    })
}

/// Steal up to `count` items, the first written at `out` and each next one
/// `stride` bytes on, and each one's length into `out_lens`; `stride` is at
/// least `SUBETHA_WORK_QUEUE_ITEM_MAX`. The batch contract is
/// `subetha_channel_try_recv_many`'s.
///
/// # Safety
/// `out` addresses `count * stride` writable bytes, `out_lens` `count`
/// writable lengths, and `out_done` is a valid pointer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn subetha_work_queue_steal_many(
    handle: subetha_handle,
    out: *mut u8,
    stride: usize,
    out_lens: *mut usize,
    count: usize,
    out_done: *mut usize,
) -> i32 {
    with_queue(handle, |q| {
        if count != 0 && out_lens.is_null() {
            return fail(SUBETHA_E_INVALID_ARGUMENT, "out_lens is null with a non-zero count");
        }
        let lens: &mut [usize] = if count == 0 {
            &mut []
        } else {
            // SAFETY: checked non-null; the caller guarantees `count` writable lengths.
            unsafe { std::slice::from_raw_parts_mut(out_lens, count) }
        };
        // SAFETY: the caller guarantees the array and the count word.
        unsafe {
            run_pop_many_indexed(out, stride, SUBETHA_WORK_QUEUE_ITEM_MAX, count, out_done, |i, buf| match q.queue.steal() {
                Some(held) => {
                    let payload = held.as_bytes();
                    buf[..payload.len()].copy_from_slice(payload);
                    lens[i] = payload.len();
                    SUBETHA_OK
                }
                None => SUBETHA_E_RING_EMPTY,
            })
        }
    })
}

/// A snapshot of this handle into `out`.
///
/// # Safety
/// `out` is a valid pointer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn subetha_work_queue_read_stats(handle: subetha_handle, out: *mut subetha_work_queue_stats) -> i32 {
    with_queue(handle, |q| {
        if out.is_null() {
            return fail(SUBETHA_E_INVALID_ARGUMENT, "out is null");
        }
        // SAFETY: checked non-null; the caller guarantees it is writable.
        unsafe { *out = subetha_work_queue_stats { mode: q.mode, role: q.role } };
        SUBETHA_OK
    })
}

/// Remove the queue file at `path`. The contract is `subetha_ring_unlink`'s.
///
/// # Safety
/// `path` is a NUL-terminated UTF-8 string; `report` is null or a valid
/// pointer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn subetha_work_queue_unlink(path: *const c_char, report: *mut subetha_unlink_report) -> i32 {
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
    use crate::error::{SUBETHA_E_BUFFER_TOO_SMALL, SUBETHA_E_RING_FULL};
    use crate::runtime::{subetha_handle_destroy, subetha_init, SUBETHA_MODE_STRICT};
    use std::ffi::CString;

    fn scratch(name: &str) -> CString {
        let path = std::env::temp_dir().join(format!("subetha-ffi-work-queue-{name}-{}.bin", std::process::id()));
        CString::new(path.to_str().expect("a UTF-8 temp path")).expect("no NUL in the path")
    }

    /// The owner pops the newest item and a thief steals the oldest, each
    /// with its length; a thief may not push or pop; a full queue, an
    /// over-long item and a short buffer each have their code; and an
    /// unlink removes the file.
    #[test]
    fn the_owner_pops_the_newest_and_a_thief_steals_the_oldest() {
        assert_eq!(subetha_init(SUBETHA_MODE_STRICT), SUBETHA_OK);
        let path = scratch("entry");
        let (mut owner, mut thief, mut other) = (0, 0, 0);
        let mut buf = [0u8; SUBETHA_WORK_QUEUE_ITEM_MAX];
        let mut len = 0usize;
        // SAFETY: valid paths, buffers and out pointers throughout.
        unsafe {
            assert_eq!(subetha_work_queue_create(path.as_ptr(), 3, 1, SUBETHA_MODE_STRICT, &mut owner), SUBETHA_OK);
            assert_eq!(subetha_work_queue_steal_from(path.as_ptr(), SUBETHA_MODE_STRICT, &mut thief), SUBETHA_OK);
            assert_eq!(subetha_work_queue_create(path.as_ptr(), 0, 1, SUBETHA_MODE_STRICT, &mut other), SUBETHA_E_INVALID_ARGUMENT);
            assert_eq!(subetha_work_queue_create(path.as_ptr(), 4, 0, SUBETHA_MODE_STRICT, &mut other), SUBETHA_E_INVALID_ARGUMENT);

            for job in [&b"one"[..], b"two", b"three"] {
                assert_eq!(subetha_work_queue_push(owner, job.as_ptr(), job.len()), SUBETHA_OK);
            }
            assert_eq!(subetha_work_queue_push(owner, b"four".as_ptr(), 4), SUBETHA_OK, "three rounds up to four");
            assert_eq!(subetha_work_queue_push(owner, b"five".as_ptr(), 4), SUBETHA_E_RING_FULL);
            let long = [7u8; SUBETHA_WORK_QUEUE_ITEM_MAX + 1];
            assert_eq!(subetha_work_queue_push(owner, long.as_ptr(), long.len()), SUBETHA_E_RING_PAYLOAD_TOO_LARGE);
            assert_eq!(subetha_work_queue_push(thief, b"x".as_ptr(), 1), SUBETHA_E_DEQUE_NOT_OWNER);
            assert_eq!(subetha_work_queue_pop(thief, buf.as_mut_ptr(), buf.len(), &mut len), SUBETHA_E_DEQUE_NOT_OWNER);

            assert_eq!(subetha_work_queue_pop(owner, buf.as_mut_ptr(), buf.len() - 1, &mut len), SUBETHA_E_BUFFER_TOO_SMALL);
            assert_eq!(subetha_work_queue_pop(owner, buf.as_mut_ptr(), buf.len(), &mut len), SUBETHA_OK);
            assert_eq!(&buf[..len], b"four", "the owner takes the newest");
            assert_eq!(subetha_work_queue_steal(thief, buf.as_mut_ptr(), buf.len(), &mut len), SUBETHA_OK);
            assert_eq!(&buf[..len], b"one", "a thief takes the oldest");

            let mut stats = subetha_work_queue_stats::default();
            assert_eq!(subetha_work_queue_read_stats(thief, &mut stats), SUBETHA_OK);
            assert_eq!((stats.mode, stats.role), (SUBETHA_MODE_STRICT, SUBETHA_DEQUE_THIEF));
            assert_eq!(subetha_work_queue_read_stats(owner, &mut stats), SUBETHA_OK);
            assert_eq!(stats.role, SUBETHA_DEQUE_OWNER);

            assert_eq!(subetha_handle_destroy(thief), SUBETHA_OK);
            assert_eq!(subetha_handle_destroy(owner), SUBETHA_OK);
            let mut report = subetha_unlink_report::default();
            assert_eq!(subetha_work_queue_unlink(path.as_ptr(), &mut report), SUBETHA_OK);
            assert_eq!((report.removed, report.failed), (1, 0));
        }
    }

    /// A batch push keeps each item's length and a batch steal hands each
    /// back with its own, oldest first, until the queue is empty.
    #[test]
    fn the_batches_keep_each_items_length() {
        assert_eq!(subetha_init(SUBETHA_MODE_STRICT), SUBETHA_OK);
        let path = scratch("batch");
        let mut owner = 0;
        const STRIDE: usize = 4;
        let items: [u8; 3 * STRIDE] = *b"a\0\0\0bb\0\0ccc\0";
        let lens = [1usize, 2, 3];
        let mut done = 0usize;
        let stride = SUBETHA_WORK_QUEUE_ITEM_MAX;
        let mut out = [0u8; 4 * SUBETHA_WORK_QUEUE_ITEM_MAX];
        let mut out_lens = [usize::MAX; 4];
        // SAFETY: valid paths, arrays and out pointers throughout.
        unsafe {
            assert_eq!(subetha_work_queue_create(path.as_ptr(), 8, 2, SUBETHA_MODE_STRICT, &mut owner), SUBETHA_OK);
            assert_eq!(subetha_work_queue_push_many(owner, items.as_ptr(), STRIDE, lens.as_ptr(), 3, &mut done), SUBETHA_OK);
            assert_eq!(done, 3);
            assert_eq!(subetha_work_queue_steal_many(owner, out.as_mut_ptr(), stride, out_lens.as_mut_ptr(), 4, &mut done), SUBETHA_OK);
            assert_eq!(done, 3);
            assert_eq!(&out_lens[..3], &[1, 2, 3]);
            assert_eq!(out_lens[3], usize::MAX, "a slot nothing filled is left alone");
            assert_eq!(&out[..1], b"a");
            assert_eq!(&out[stride..stride + 2], b"bb");
            assert_eq!(&out[2 * stride..2 * stride + 3], b"ccc");
            assert_eq!(
                subetha_work_queue_steal_many(owner, out.as_mut_ptr(), stride, out_lens.as_mut_ptr(), 4, &mut done),
                SUBETHA_E_RING_EMPTY
            );
            assert_eq!(subetha_handle_destroy(owner), SUBETHA_OK);
            assert_eq!(subetha_work_queue_unlink(path.as_ptr(), std::ptr::null_mut()), SUBETHA_OK);
        }
    }
}
