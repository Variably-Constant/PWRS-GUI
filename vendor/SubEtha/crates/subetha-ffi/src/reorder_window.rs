//! The reorder window through the C ABI, on its own: items go in with the
//! stamp their sender gave them and come out smallest stamp first once
//! more than `window` of them are held. The window grows by itself when an
//! item turns up further out of order than it covered, and `corrections`
//! counts how often that happened; zero means the starting window was wide
//! enough. `subetha_ordered_` uses this window against a stamped ring; this
//! is the same window over items from anywhere else, Python's
//! `ReorderWindow` and PowerShell's `New-SubEthaReorderWindow`. It lives in
//! this process's memory, behind one atomic claim every call takes, and runs no
//! background work, so strict and managed modes are the same.

use subetha_cxc::claim::Claim;
use subetha_cxc::ordering::STAMPED_PAYLOAD_BYTES;
use subetha_cxc::reorder::{ReorderBuffer, DEFAULT_CAP, DEFAULT_FLOOR};

use crate::batch::{batch_arguments, batch_result};
use crate::error::{
    fail, SUBETHA_E_INVALID_ARGUMENT, SUBETHA_E_RING_EMPTY, SUBETHA_E_RING_PAYLOAD_TOO_LARGE, SUBETHA_E_WRONG_KIND,
    SUBETHA_OK,
};
use crate::handle::{subetha_handle, Object, SUBETHA_KIND_REORDER_WINDOW};
use crate::ring::{bytes, out_buffer};
use crate::runtime::{entry, issue, require_initialized, resolve_mode, with_kind};

/// The most an item may be, in bytes. A take or flush buffer must be at
/// least this large.
pub const SUBETHA_REORDER_ITEM_MAX: usize = 56;
/// The window a caller with no measurement of its own starts at.
pub const SUBETHA_REORDER_DEFAULT_FLOOR: u32 = 8;
/// The widest the window grows to unless the caller names another cap.
pub const SUBETHA_REORDER_DEFAULT_CAP: u32 = 1024;

const _: () = assert!(SUBETHA_REORDER_ITEM_MAX == STAMPED_PAYLOAD_BYTES);
const _: () = assert!(SUBETHA_REORDER_DEFAULT_FLOOR as usize == DEFAULT_FLOOR);
const _: () = assert!(SUBETHA_REORDER_DEFAULT_CAP as usize == DEFAULT_CAP);

/// A snapshot of a reorder window.
#[repr(C)]
#[allow(non_camel_case_types)]
#[derive(Clone, Copy, Default)]
pub struct subetha_reorder_window_stats {
    /// The mode this handle was created in.
    pub mode: u32,
    /// The window right now, which growth changes.
    pub window: u64,
    /// Times an item arrived further out of order than the window covered,
    /// each of which widened it.
    pub corrections: u64,
    /// Items held waiting for a gap to fill, which is not how many have
    /// passed through.
    pub held: u64,
}

pub(crate) struct ReorderWindowObject {
    window: Claim<ReorderBuffer>,
    mode: u32,
}

impl ReorderWindowObject {
    /// A window parks nothing inside a call, so a destroy has nobody to
    /// wake.
    pub(crate) fn interrupt(&self) {}
}

fn with_window(handle: subetha_handle, f: impl FnOnce(&ReorderWindowObject) -> i32) -> i32 {
    with_kind(handle, SUBETHA_KIND_REORDER_WINDOW, |object| match object {
        Object::ReorderWindow(w) => f(w),
        _ => fail(SUBETHA_E_WRONG_KIND, "the handle does not name a reorder window"),
    })
}

/// Refuse a payload the window would cut short.
fn fits(payload: &[u8]) -> Result<(), i32> {
    if payload.len() > SUBETHA_REORDER_ITEM_MAX {
        Err(fail(
            SUBETHA_E_RING_PAYLOAD_TOO_LARGE,
            format!("an item of {} bytes is past the {SUBETHA_REORDER_ITEM_MAX} a window holds", payload.len()),
        ))
    } else {
        Ok(())
    }
}

/// Write a released item's length and stamp, or report
/// `SUBETHA_E_RING_EMPTY` for nothing released.
///
/// # Safety
/// `out_len` and `out_stamp` are valid pointers.
unsafe fn report(released: Option<(u64, usize)>, out_len: *mut usize, out_stamp: *mut u64) -> i32 {
    match released {
        Some((stamp, len)) => {
            // SAFETY: the caller guarantees both are writable.
            unsafe {
                *out_len = len;
                *out_stamp = stamp;
            }
            SUBETHA_OK
        }
        None => SUBETHA_E_RING_EMPTY,
    }
}

/// Make a window that starts at `floor` items and grows to at most `cap`;
/// a `cap` below `floor` is raised to it. A window at least as wide as the
/// number of senders puts every item back in order.
/// `SUBETHA_REORDER_DEFAULT_FLOOR` and `SUBETHA_REORDER_DEFAULT_CAP` are
/// the window the ordered receiver starts from.
///
/// # Safety
/// `out` is a valid pointer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn subetha_reorder_window_create(floor: u32, cap: u32, mode: u32, out: *mut subetha_handle) -> i32 {
    entry(|| {
        if let Err(code) = require_initialized() {
            return code;
        }
        if out.is_null() {
            return fail(SUBETHA_E_INVALID_ARGUMENT, "out is null");
        }
        let mode = match resolve_mode(mode) {
            Ok(m) => m,
            Err(code) => return code,
        };
        let window = Claim::new(ReorderBuffer::with_window(floor as usize, cap as usize));
        unsafe { issue(Object::ReorderWindow(ReorderWindowObject { window, mode }), out) }
    })
}

/// Hold the `len` bytes at `data` with the stamp `stamp` their sender gave
/// them. `SUBETHA_E_RING_PAYLOAD_TOO_LARGE` for more than
/// `SUBETHA_REORDER_ITEM_MAX` bytes.
///
/// # Safety
/// `data` is null or points to `len` readable bytes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn subetha_reorder_window_push(handle: subetha_handle, stamp: u64, data: *const u8, len: usize) -> i32 {
    with_window(handle, |w| {
        let payload = match unsafe { bytes(data, len) } {
            Ok(p) => p,
            Err(code) => return code,
        };
        if let Err(code) = fits(payload) {
            return code;
        }
        w.window.claim().push(stamp, payload);
        SUBETHA_OK
    })
}

/// The next item in stamp order into `out`, its length into `out_len` and
/// its stamp into `out_stamp`. `cap` must be at least
/// `SUBETHA_REORDER_ITEM_MAX`. `SUBETHA_E_RING_EMPTY` while the window holds
/// `window` items or fewer.
///
/// # Safety
/// `out` points to `cap` writable bytes; `out_len` and `out_stamp` are
/// valid pointers.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn subetha_reorder_window_take(
    handle: subetha_handle,
    out: *mut u8,
    cap: usize,
    out_len: *mut usize,
    out_stamp: *mut u64,
) -> i32 {
    with_window(handle, |w| {
        if out_stamp.is_null() {
            return fail(SUBETHA_E_INVALID_ARGUMENT, "out_stamp is null");
        }
        let buf = match unsafe { out_buffer(out, cap, out_len, SUBETHA_REORDER_ITEM_MAX) } {
            Ok(b) => b,
            Err(code) => return code,
        };
        let released = w.window.claim().try_take(buf);
        unsafe { report(released, out_len, out_stamp) }
    })
}

/// The next item in stamp order however full the window is, as
/// `subetha_reorder_window_take` writes it; call in a loop at the end of a
/// stream until `SUBETHA_E_RING_EMPTY`, which means nothing is held.
///
/// # Safety
/// `out` points to `cap` writable bytes; `out_len` and `out_stamp` are
/// valid pointers.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn subetha_reorder_window_flush(
    handle: subetha_handle,
    out: *mut u8,
    cap: usize,
    out_len: *mut usize,
    out_stamp: *mut u64,
) -> i32 {
    with_window(handle, |w| {
        if out_stamp.is_null() {
            return fail(SUBETHA_E_INVALID_ARGUMENT, "out_stamp is null");
        }
        let buf = match unsafe { out_buffer(out, cap, out_len, SUBETHA_REORDER_ITEM_MAX) } {
            Ok(b) => b,
            Err(code) => return code,
        };
        let released = w.window.claim().flush_one(buf);
        unsafe { report(released, out_len, out_stamp) }
    })
}

/// Hold `count` items under one handle lookup, one panic guard and one
/// lock: item `i` is the `lens[i]` bytes `i * stride` past `items`, with
/// the stamp `stamps[i]`. A length past `stride` or past
/// `SUBETHA_REORDER_ITEM_MAX` stops the batch, which returns `SUBETHA_OK`
/// when any went in; `out_done` counts them.
///
/// # Safety
/// `stamps` addresses `count` readable words, `items` `count * stride`
/// readable bytes and `lens` `count` readable lengths; `out_done` is a
/// valid pointer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn subetha_reorder_window_push_many(
    handle: subetha_handle,
    stamps: *const u64,
    items: *const u8,
    stride: usize,
    lens: *const usize,
    count: usize,
    out_done: *mut usize,
) -> i32 {
    with_window(handle, |w| {
        if let Err(code) = batch_arguments(items.is_null(), stride, 0, count, out_done.is_null()) {
            return code;
        }
        if count != 0 && (stamps.is_null() || lens.is_null()) {
            return fail(SUBETHA_E_INVALID_ARGUMENT, "stamps or lens is null with a non-zero count");
        }
        let (done, refusal) = {
            let mut window = w.window.claim();
            let mut done = 0usize;
            let mut refusal = None;
            while done < count {
                // SAFETY: `done` is below `count`, which the caller
                // guarantees is readable at `stamps` and `lens`.
                let (stamp, len) = unsafe { (*stamps.add(done), *lens.add(done)) };
                if len > stride {
                    refusal = Some(fail(SUBETHA_E_INVALID_ARGUMENT, format!("item {done} is {len} bytes, past the stride of {stride}")));
                    break;
                }
                // SAFETY: the caller guarantees the array spans this item.
                let payload = unsafe { std::slice::from_raw_parts(items.add(done * stride), len) };
                if let Err(code) = fits(payload) {
                    refusal = Some(code);
                    break;
                }
                window.push(stamp, payload);
                done += 1;
            }
            (done, refusal)
        };
        // SAFETY: `batch_arguments` refused a null `out_done`.
        unsafe { batch_result(done, refusal, out_done) }
    })
}

/// Flush up to `count` items in stamp order under one claim, the first
/// written at `out` and each next one `stride` bytes on, each one's length
/// into `out_lens` and stamp into `out_stamps`; `stride` is at least
/// `SUBETHA_REORDER_ITEM_MAX`. `SUBETHA_E_RING_EMPTY` when nothing was
/// held, and otherwise `out_done` counts what came.
///
/// # Safety
/// `out` addresses `count * stride` writable bytes, `out_lens` `count`
/// writable lengths and `out_stamps` `count` writable words; `out_done` is
/// a valid pointer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn subetha_reorder_window_flush_many(
    handle: subetha_handle,
    out: *mut u8,
    stride: usize,
    out_lens: *mut usize,
    out_stamps: *mut u64,
    count: usize,
    out_done: *mut usize,
) -> i32 {
    with_window(handle, |w| {
        if let Err(code) = batch_arguments(out.is_null(), stride, SUBETHA_REORDER_ITEM_MAX, count, out_done.is_null()) {
            return code;
        }
        if count != 0 && (out_lens.is_null() || out_stamps.is_null()) {
            return fail(SUBETHA_E_INVALID_ARGUMENT, "out_lens or out_stamps is null with a non-zero count");
        }
        let (done, refusal) = {
            let mut window = w.window.claim();
            let mut done = 0usize;
            let mut refusal = None;
            while done < count {
                // SAFETY: the caller guarantees the array spans this slot.
                let buf = unsafe { std::slice::from_raw_parts_mut(out.add(done * stride), stride) };
                match window.flush_one(buf) {
                    Some((stamp, len)) => {
                        // SAFETY: `done` is below `count`, which the caller
                        // guarantees is writable at both arrays.
                        unsafe {
                            *out_lens.add(done) = len;
                            *out_stamps.add(done) = stamp;
                        }
                        done += 1;
                    }
                    None => {
                        refusal = Some(SUBETHA_E_RING_EMPTY);
                        break;
                    }
                }
            }
            (done, refusal)
        };
        // SAFETY: `batch_arguments` refused a null `out_done`.
        unsafe { batch_result(done, refusal, out_done) }
    })
}

/// Widen the window to at least `window`, which is what a caller does when
/// the number of senders grows; a narrower `window` changes nothing.
#[unsafe(no_mangle)]
pub extern "C" fn subetha_reorder_window_widen_to(handle: subetha_handle, window: u32) -> i32 {
    with_window(handle, |w| {
        w.window.claim().widen_to(window as usize);
        SUBETHA_OK
    })
}

/// A snapshot of the window into `out`.
///
/// # Safety
/// `out` is a valid pointer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn subetha_reorder_window_read_stats(handle: subetha_handle, out: *mut subetha_reorder_window_stats) -> i32 {
    with_window(handle, |w| {
        if out.is_null() {
            return fail(SUBETHA_E_INVALID_ARGUMENT, "out is null");
        }
        let stats = {
            let window = w.window.claim();
            subetha_reorder_window_stats {
                mode: w.mode,
                window: window.window() as u64,
                corrections: window.corrections(),
                held: window.len() as u64,
            }
        };
        // SAFETY: checked non-null; the caller guarantees it is writable.
        unsafe { *out = stats };
        SUBETHA_OK
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::SUBETHA_E_BUFFER_TOO_SMALL;
    use crate::runtime::{subetha_handle_destroy, subetha_init, SUBETHA_MODE_STRICT};

    /// Items pushed out of order come back in stamp order once the window
    /// holds more than its width, and a flush drains the rest; an item
    /// further out of order than the window widens it and is counted; an
    /// over-long item and a short buffer are refused.
    #[test]
    fn a_window_puts_items_back_in_stamp_order() {
        assert_eq!(subetha_init(SUBETHA_MODE_STRICT), SUBETHA_OK);
        let mut w = 0;
        let mut buf = [0u8; SUBETHA_REORDER_ITEM_MAX];
        let (mut len, mut stamp) = (0usize, 0u64);
        // SAFETY: valid buffers and out pointers throughout.
        unsafe {
            assert_eq!(subetha_reorder_window_create(2, 8, SUBETHA_MODE_STRICT, &mut w), SUBETHA_OK);
            assert_eq!(subetha_reorder_window_push(w, 20, b"b".as_ptr(), 1), SUBETHA_OK);
            assert_eq!(subetha_reorder_window_push(w, 10, b"a".as_ptr(), 1), SUBETHA_OK);
            assert_eq!(subetha_reorder_window_take(w, buf.as_mut_ptr(), buf.len(), &mut len, &mut stamp), SUBETHA_E_RING_EMPTY, "two held, window two");
            assert_eq!(subetha_reorder_window_push(w, 30, b"cc".as_ptr(), 2), SUBETHA_OK);
            assert_eq!(subetha_reorder_window_take(w, buf.as_mut_ptr(), buf.len() - 1, &mut len, &mut stamp), SUBETHA_E_BUFFER_TOO_SMALL);
            assert_eq!(subetha_reorder_window_take(w, buf.as_mut_ptr(), buf.len(), &mut len, &mut stamp), SUBETHA_OK);
            assert_eq!((stamp, &buf[..len]), (10, &b"a"[..]));

            assert_eq!(subetha_reorder_window_push(w, 5, b"late".as_ptr(), 4), SUBETHA_OK);
            assert_eq!(subetha_reorder_window_take(w, buf.as_mut_ptr(), buf.len(), &mut len, &mut stamp), SUBETHA_OK);
            assert_eq!(stamp, 5, "the smallest stamp held comes out next");
            let mut stats = subetha_reorder_window_stats::default();
            assert_eq!(subetha_reorder_window_read_stats(w, &mut stats), SUBETHA_OK);
            assert_eq!(stats.corrections, 1, "a stamp below one already released widened the window");
            assert!(stats.window > 2);

            let long = [7u8; SUBETHA_REORDER_ITEM_MAX + 1];
            assert_eq!(subetha_reorder_window_push(w, 99, long.as_ptr(), long.len()), SUBETHA_E_RING_PAYLOAD_TOO_LARGE);
            let mut flushed = Vec::new();
            while subetha_reorder_window_flush(w, buf.as_mut_ptr(), buf.len(), &mut len, &mut stamp) == SUBETHA_OK {
                flushed.push(stamp);
            }
            assert_eq!(flushed, vec![20, 30]);
            assert_eq!(subetha_reorder_window_read_stats(w, &mut stats), SUBETHA_OK);
            assert_eq!((stats.mode, stats.held), (SUBETHA_MODE_STRICT, 0));
            assert_eq!(subetha_reorder_window_widen_to(w, 64), SUBETHA_OK);
            assert_eq!(subetha_reorder_window_read_stats(w, &mut stats), SUBETHA_OK);
            assert_eq!(stats.window, 64);
            assert_eq!(subetha_handle_destroy(w), SUBETHA_OK);
        }
    }

    /// A batch push holds each item with its stamp and length, and a batch
    /// flush hands them back in stamp order with both.
    #[test]
    fn the_batches_keep_stamps_and_lengths() {
        assert_eq!(subetha_init(SUBETHA_MODE_STRICT), SUBETHA_OK);
        let mut w = 0;
        const STRIDE: usize = 4;
        let items: [u8; 3 * STRIDE] = *b"ccc\0a\0\0\0bb\0\0";
        let stamps = [3u64, 1, 2];
        let lens = [3usize, 1, 2];
        let mut done = 0usize;
        let stride = SUBETHA_REORDER_ITEM_MAX;
        let mut out = [0u8; 4 * SUBETHA_REORDER_ITEM_MAX];
        let mut out_lens = [usize::MAX; 4];
        let mut out_stamps = [u64::MAX; 4];
        // SAFETY: valid arrays and out pointers throughout.
        unsafe {
            assert_eq!(
                subetha_reorder_window_create(SUBETHA_REORDER_DEFAULT_FLOOR, SUBETHA_REORDER_DEFAULT_CAP, SUBETHA_MODE_STRICT, &mut w),
                SUBETHA_OK
            );
            assert_eq!(subetha_reorder_window_push_many(w, stamps.as_ptr(), items.as_ptr(), STRIDE, lens.as_ptr(), 3, &mut done), SUBETHA_OK);
            assert_eq!(done, 3);
            assert_eq!(
                subetha_reorder_window_flush_many(w, out.as_mut_ptr(), stride, out_lens.as_mut_ptr(), out_stamps.as_mut_ptr(), 4, &mut done),
                SUBETHA_OK
            );
            assert_eq!(done, 3);
            assert_eq!(&out_stamps[..3], &[1, 2, 3]);
            assert_eq!(&out_lens[..3], &[1, 2, 3]);
            assert_eq!((out_stamps[3], out_lens[3]), (u64::MAX, usize::MAX), "a slot nothing filled is left alone");
            assert_eq!(&out[..1], b"a");
            assert_eq!(&out[stride..stride + 2], b"bb");
            assert_eq!(&out[2 * stride..2 * stride + 3], b"ccc");
            assert_eq!(
                subetha_reorder_window_flush_many(w, out.as_mut_ptr(), stride, out_lens.as_mut_ptr(), out_stamps.as_mut_ptr(), 4, &mut done),
                SUBETHA_E_RING_EMPTY
            );
            assert_eq!(subetha_handle_destroy(w), SUBETHA_OK);
        }
    }
}
