//! The front door's adaptive queue through the C ABI: a queue that changes
//! what it is underneath as its traffic changes, moving between a ring and
//! a work-stealing deque without either end reconnecting. It is the queue
//! Python's `AdaptiveQueue` and PowerShell's `New-SubEthaAdaptiveQueue`
//! make. An item is up to `SUBETHA_ADAPTIVE_QUEUE_ITEM_MAX` bytes and keeps
//! its length.
//!
//! The queue's files live while the handle that made them does: destroying
//! the handle removes them, and `subetha_adaptive_queue_unlink` removes
//! what a process that ended without destroying one left behind. What the
//! queue has counted of its traffic, which a change of shape is decided
//! on, is this handle's own.
//!
//! Mode: strict starts no thread, and the queue weighs a change of shape,
//! and an `auto_order` rate, only when
//! `subetha_adaptive_queue_maybe_change_shape` asks. Managed starts a
//! scanner that weighs them every `SUBETHA_SCAN_INTERVAL_DEFAULT_US`
//! microseconds, and the stats count the changes it made.

use std::ffi::c_char;
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use subetha_cxc::adaptive_ipc::{AdaptiveIpc, AdaptiveIpcSidecar};
use subetha_cxc::api::BLOCKING_HEAL;
use subetha_cxc::byte_value::{SlotValue, SLOT_VALUE_BYTES};
use subetha_cxc::dispatch_deque::DequeVariant;
use subetha_cxc::mmf_dispatcher::{MmfFamily, MmfWorkloadShape};
use subetha_cxc::qos_policy::Ordering as QosOrdering;

use crate::batch::run_pop_many_indexed;
use crate::batch::{batch_arguments, batch_result};
use crate::error::{
    api_code, fail, SUBETHA_E_INVALID_ARGUMENT, SUBETHA_E_RING_FULL, SUBETHA_E_RING_PAYLOAD_TOO_LARGE,
    SUBETHA_E_TIMEOUT, SUBETHA_E_WRONG_KIND, SUBETHA_OK,
};
use crate::handle::{subetha_handle, Object, SUBETHA_KIND_ADAPTIVE_QUEUE};
use crate::qos_policy::{ordering_of, SUBETHA_QOS_GLOBAL_FIFO, SUBETHA_QOS_PER_PRODUCER};
use crate::ring::{bytes, deadline_from, finish_unlink, out_buffer, subetha_unlink_report, text, SUBETHA_SCAN_INTERVAL_DEFAULT_US};
use crate::runtime::{entry, issue, require_initialized, resolve_mode, with_kind, SUBETHA_MODE_MANAGED};
use crate::wait::{slice_until, Waiting};

/// The most an item may be, in bytes. A receive buffer must be at least
/// this large.
pub const SUBETHA_ADAPTIVE_QUEUE_ITEM_MAX: usize = 52;
/// Shape: the adaptive ring, which streaming traffic starts on.
pub const SUBETHA_ADAPTIVE_SHAPE_RING: u32 = 0;
/// Shape: the work-stealing deque, which batched traffic moves to.
pub const SUBETHA_ADAPTIVE_SHAPE_WORK_STEALING: u32 = 1;
/// `auto_order`: the queue never turns global ordering on by itself.
pub const SUBETHA_AUTO_ORDER_NEVER: f64 = -1.0;

const _: () = assert!(SUBETHA_ADAPTIVE_QUEUE_ITEM_MAX == SLOT_VALUE_BYTES);

/// A snapshot of an adaptive queue handle.
#[repr(C)]
#[allow(non_camel_case_types)]
#[derive(Clone, Copy, Default)]
pub struct subetha_adaptive_queue_stats {
    /// The mode this handle was created in.
    pub mode: u32,
    /// `SUBETHA_ADAPTIVE_SHAPE_RING` or
    /// `SUBETHA_ADAPTIVE_SHAPE_WORK_STEALING`, what the queue is now.
    pub shape: u32,
    /// Steps every time the shape changes, so a holder can tell that what
    /// it looked at has been superseded.
    pub shape_generation: u64,
    /// `SUBETHA_QOS_PER_PRODUCER` or `SUBETHA_QOS_GLOBAL_FIFO`, whose order
    /// a reader gets now.
    pub ordering: u32,
    /// Cross-sender inversions seen so far, which say whether the ordering
    /// asked for is being met.
    pub inversions: u64,
    /// Items a send carried on average, this handle's sends counted.
    pub average_batch: u64,
    /// The share of this handle's sends that were batches, from zero to
    /// one.
    pub batch_share: f64,
    /// Changes of shape the managed scanner made; zero in strict mode.
    pub scanner_changes: u64,
}

pub(crate) struct AdaptiveQueueObject {
    /// The scanner managed mode starts. It holds the queue as well, and is
    /// declared first so it stops before the queue's last holder goes.
    scanner: Option<AdaptiveIpcSidecar>,
    queue: Arc<AdaptiveIpc<SlotValue>>,
    mode: u32,
    waiting: Waiting,
}

impl AdaptiveQueueObject {
    /// Raise the flag every wait reads between slices, so a destroy can
    /// proceed; each wait returns `SUBETHA_E_DESTROYED` within one slice.
    pub(crate) fn interrupt(&self) {
        self.waiting.close();
    }

    /// The queue's own blocking send, one heal tick at a time, so the
    /// destroy flag is read between ticks.
    fn send_wait(&self, item: &SlotValue, deadline: Option<Instant>) -> Result<(), i32> {
        slice_until(&self.waiting, BLOCKING_HEAL, deadline, SUBETHA_E_TIMEOUT, |slice| {
            self.queue.send_blocking(item, Some(slice)).map_err(api_code)
        })
    }

    /// The queue's own blocking receive, one heal tick at a time.
    fn recv_wait(&self, deadline: Option<Instant>) -> Result<SlotValue, i32> {
        slice_until(&self.waiting, BLOCKING_HEAL, deadline, SUBETHA_E_TIMEOUT, |slice| {
            self.queue.recv_blocking(Some(slice)).map_err(api_code)
        })
    }
}

fn with_queue(handle: subetha_handle, f: impl FnOnce(&AdaptiveQueueObject) -> i32) -> i32 {
    with_kind(handle, SUBETHA_KIND_ADAPTIVE_QUEUE, |object| match object {
        Object::AdaptiveQueue(q) => f(q),
        _ => fail(SUBETHA_E_WRONG_KIND, "the handle does not name an adaptive queue"),
    })
}

fn shape_of(family: MmfFamily) -> u32 {
    match family {
        MmfFamily::SharedDeque(_) => SUBETHA_ADAPTIVE_SHAPE_WORK_STEALING,
        MmfFamily::SharedRing | MmfFamily::SharedHashMap => SUBETHA_ADAPTIVE_SHAPE_RING,
    }
}

fn family_of(shape: u32) -> Result<MmfFamily, i32> {
    match shape {
        SUBETHA_ADAPTIVE_SHAPE_RING => Ok(MmfFamily::SharedRing),
        SUBETHA_ADAPTIVE_SHAPE_WORK_STEALING => Ok(MmfFamily::SharedDeque(DequeVariant::ChaseLev)),
        other => Err(fail(SUBETHA_E_INVALID_ARGUMENT, format!("shape {other} names none"))),
    }
}

fn ordering_tag(ordering: QosOrdering) -> u32 {
    match ordering {
        QosOrdering::PerProducer => SUBETHA_QOS_PER_PRODUCER,
        QosOrdering::GlobalFifo => SUBETHA_QOS_GLOBAL_FIFO,
    }
}

/// The `auto_order` rate a C caller's double names: none for
/// `SUBETHA_AUTO_ORDER_NEVER`, the rate for any finite value of zero or
/// more.
fn auto_order_rate(auto_order: f64) -> Result<Option<f64>, i32> {
    if auto_order == SUBETHA_AUTO_ORDER_NEVER {
        Ok(None)
    } else if auto_order.is_finite() && auto_order >= 0.0 {
        Ok(Some(auto_order))
    } else {
        Err(fail(
            SUBETHA_E_INVALID_ARGUMENT,
            format!(
                "auto_order {auto_order} is neither SUBETHA_AUTO_ORDER_NEVER nor a number of inversions a second that is not negative"
            ),
        ))
    }
}

/// The item the `len` bytes at `data` make.
///
/// # Safety
/// `data` is null or points to `len` readable bytes.
unsafe fn item(data: *const u8, len: usize) -> Result<SlotValue, i32> {
    let payload = unsafe { bytes(data, len) }?;
    SlotValue::from_bytes(payload).map_err(|e| fail(SUBETHA_E_RING_PAYLOAD_TOO_LARGE, e.to_string()))
}

/// Copy `held`'s bytes to the front of `buf` and its length to `out_len`.
///
/// # Safety
/// `out_len` is a valid pointer, as `out_buffer` checked.
unsafe fn deliver(held: &SlotValue, buf: &mut [u8], out_len: *mut usize) -> i32 {
    let payload = held.as_bytes();
    buf[..payload.len()].copy_from_slice(payload);
    // SAFETY: checked non-null; the caller guarantees it is writable.
    unsafe { *out_len = payload.len() };
    SUBETHA_OK
}

/// Make the queue at `path` for `senders` senders and `readers` readers,
/// with `capacity` items in flight rounded up to a power of two of at least
/// 2, as the Python and PowerShell queues round it. It starts as a ring and
/// changes shape as its traffic says. `ordering` is
/// `SUBETHA_QOS_PER_PRODUCER`, each sender's own order, or
/// `SUBETHA_QOS_GLOBAL_FIFO`, one order across them all; it is the caller's
/// setting, never inferred from the traffic. `auto_order` is
/// `SUBETHA_AUTO_ORDER_NEVER`, or the cross-sender inversions per second
/// past which the queue may turn global ordering on by itself; any other
/// negative or non-finite value is refused. Zero senders, readers or
/// capacity are refused.
///
/// The queue's deque is made afresh and its other files are attached when
/// they are there; they are removed when this handle is destroyed.
///
/// # Safety
/// `path` is a NUL-terminated UTF-8 string; `out` is a valid pointer.
#[unsafe(no_mangle)]
#[allow(clippy::too_many_arguments)]
pub unsafe extern "C" fn subetha_adaptive_queue_create(
    path: *const c_char,
    capacity: u32,
    senders: u32,
    readers: u32,
    ordering: u32,
    auto_order: f64,
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
        if capacity == 0 {
            return fail(SUBETHA_E_INVALID_ARGUMENT, "a queue's capacity must be at least 1");
        }
        let Some(rounded) = (capacity as usize).max(2).checked_next_power_of_two() else {
            return fail(SUBETHA_E_INVALID_ARGUMENT, format!("capacity {capacity} rounds past what this platform addresses"));
        };
        if senders == 0 || readers == 0 {
            return fail(SUBETHA_E_INVALID_ARGUMENT, "a queue needs at least one sender and one reader");
        }
        let ordering = match ordering_of(ordering) {
            Ok(o) => o,
            Err(code) => return code,
        };
        let auto_order = match auto_order_rate(auto_order) {
            Ok(rate) => rate,
            Err(code) => return code,
        };
        let mode = match resolve_mode(mode) {
            Ok(m) => m,
            Err(code) => return code,
        };
        let shape = MmfWorkloadShape::StreamingMpmc { n_producers: senders as usize, n_consumers: readers as usize };
        let queue = match AdaptiveIpc::<SlotValue>::create_with_ordering(path, shape, rounded, readers as usize, ordering, auto_order) {
            Ok(q) => Arc::new(q),
            Err(e) => return api_code(e),
        };
        let scanner = (mode == SUBETHA_MODE_MANAGED).then(|| {
            AdaptiveIpcSidecar::spawn(Arc::clone(&queue), Duration::from_micros(SUBETHA_SCAN_INTERVAL_DEFAULT_US))
        });
        unsafe { issue(Object::AdaptiveQueue(AdaptiveQueueObject { scanner, queue, mode, waiting: Waiting::new() }), out) }
    })
}

/// Send the `len` bytes at `data` without waiting. `SUBETHA_E_RING_FULL`
/// when the queue is full; `SUBETHA_E_RING_PAYLOAD_TOO_LARGE` for more than
/// `SUBETHA_ADAPTIVE_QUEUE_ITEM_MAX` bytes.
///
/// # Safety
/// `data` is null or points to `len` readable bytes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn subetha_adaptive_queue_try_send(handle: subetha_handle, data: *const u8, len: usize) -> i32 {
    with_queue(handle, |q| match unsafe { item(data, len) } {
        Ok(held) => match q.queue.send(&held) {
            Ok(()) => SUBETHA_OK,
            Err(e) => api_code(e),
        },
        Err(code) => code,
    })
}

/// Send, waiting for room until `timeout_ms` has passed
/// (`SUBETHA_E_TIMEOUT`); the waiting contract is
/// `subetha_channel_send_wait`'s.
///
/// # Safety
/// `data` is null or points to `len` readable bytes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn subetha_adaptive_queue_send_wait(handle: subetha_handle, data: *const u8, len: usize, timeout_ms: i64) -> i32 {
    with_queue(handle, |q| {
        let held = match unsafe { item(data, len) } {
            Ok(h) => h,
            Err(code) => return code,
        };
        let deadline = match deadline_from(timeout_ms) {
            Ok(d) => d,
            Err(code) => return code,
        };
        match q.send_wait(&held, deadline) {
            Ok(()) => SUBETHA_OK,
            Err(code) => code,
        }
    })
}

/// Receive the next item into `out` and its length into `out_len`, without
/// waiting. `cap` must be at least `SUBETHA_ADAPTIVE_QUEUE_ITEM_MAX`.
/// `SUBETHA_E_RING_EMPTY` when nothing is there.
///
/// # Safety
/// `out` points to `cap` writable bytes; `out_len` is a valid pointer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn subetha_adaptive_queue_try_recv(handle: subetha_handle, out: *mut u8, cap: usize, out_len: *mut usize) -> i32 {
    with_queue(handle, |q| {
        let buf = match unsafe { out_buffer(out, cap, out_len, SUBETHA_ADAPTIVE_QUEUE_ITEM_MAX) } {
            Ok(b) => b,
            Err(code) => return code,
        };
        match q.queue.recv() {
            Ok(held) => unsafe { deliver(&held, buf, out_len) },
            Err(e) => api_code(e),
        }
    })
}

/// Receive, waiting for an item until `timeout_ms` has passed; the buffer
/// contract is `subetha_adaptive_queue_try_recv`'s and the waiting contract
/// `subetha_channel_send_wait`'s.
///
/// # Safety
/// `out` points to `cap` writable bytes; `out_len` is a valid pointer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn subetha_adaptive_queue_recv_wait(
    handle: subetha_handle,
    out: *mut u8,
    cap: usize,
    out_len: *mut usize,
    timeout_ms: i64,
) -> i32 {
    with_queue(handle, |q| {
        let buf = match unsafe { out_buffer(out, cap, out_len, SUBETHA_ADAPTIVE_QUEUE_ITEM_MAX) } {
            Ok(b) => b,
            Err(code) => return code,
        };
        let deadline = match deadline_from(timeout_ms) {
            Ok(d) => d,
            Err(code) => return code,
        };
        match q.recv_wait(deadline) {
            Ok(held) => unsafe { deliver(&held, buf, out_len) },
            Err(code) => code,
        }
    })
}

/// Send `count` items as one batch without waiting, the first at `items`
/// and each next one `stride` bytes on, item `i` being `lens[i]` bytes. The
/// run is also what tells the queue its traffic comes in batches. It stops
/// at the first item a full queue refuses: `SUBETHA_E_RING_FULL` when that
/// was the first, otherwise `SUBETHA_OK` with `out_done` counting what went.
/// A length past `stride` or past `SUBETHA_ADAPTIVE_QUEUE_ITEM_MAX` is
/// refused before anything is sent.
///
/// # Safety
/// `items` addresses `count * stride` readable bytes, `lens` `count`
/// readable lengths, and `out_done` is a valid pointer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn subetha_adaptive_queue_try_send_many(
    handle: subetha_handle,
    items: *const u8,
    stride: usize,
    lens: *const usize,
    count: usize,
    out_done: *mut usize,
) -> i32 {
    with_queue(handle, |q| {
        if let Err(code) = batch_arguments(items.is_null(), stride, 0, count, out_done.is_null()) {
            return code;
        }
        if count != 0 && lens.is_null() {
            return fail(SUBETHA_E_INVALID_ARGUMENT, "lens is null with a non-zero count");
        }
        let mut held = Vec::with_capacity(count);
        for i in 0..count {
            // SAFETY: `i` is below `count`, which the caller guarantees is
            // readable at `lens`.
            let len = unsafe { *lens.add(i) };
            if len > stride {
                // SAFETY: `batch_arguments` refused a null `out_done`.
                let refusal = fail(SUBETHA_E_INVALID_ARGUMENT, format!("item {i} is {len} bytes, past the stride of {stride}"));
                return unsafe { batch_result(0, Some(refusal), out_done) };
            }
            // SAFETY: the caller guarantees the array spans this item.
            let payload = unsafe { std::slice::from_raw_parts(items.add(i * stride), len) };
            match SlotValue::from_bytes(payload) {
                Ok(h) => held.push(h),
                Err(e) => {
                    let refusal = fail(SUBETHA_E_RING_PAYLOAD_TOO_LARGE, format!("item {i}: {e}"));
                    // SAFETY: `batch_arguments` refused a null `out_done`.
                    return unsafe { batch_result(0, Some(refusal), out_done) };
                }
            }
        }
        let (done, refusal) = match q.queue.try_send_batch(&held) {
            Ok(landed) if landed < count => (landed, Some(SUBETHA_E_RING_FULL)),
            Ok(landed) => (landed, None),
            Err(e) => (0, Some(api_code(e))),
        };
        // SAFETY: `batch_arguments` refused a null `out_done`.
        unsafe { batch_result(done, refusal, out_done) }
    })
}

/// Receive up to `count` items without waiting, the first written at `out`
/// and each next one `stride` bytes on, and each one's length into
/// `out_lens`; `stride` is at least `SUBETHA_ADAPTIVE_QUEUE_ITEM_MAX`. The
/// batch contract is `subetha_channel_try_recv_many`'s.
///
/// # Safety
/// `out` addresses `count * stride` writable bytes, `out_lens` `count`
/// writable lengths, and `out_done` is a valid pointer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn subetha_adaptive_queue_try_recv_many(
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
            run_pop_many_indexed(out, stride, SUBETHA_ADAPTIVE_QUEUE_ITEM_MAX, count, out_done, |i, buf| match q.queue.recv() {
                Ok(held) => {
                    let payload = held.as_bytes();
                    buf[..payload.len()].copy_from_slice(payload);
                    lens[i] = payload.len();
                    SUBETHA_OK
                }
                Err(e) => api_code(e),
            })
        }
    })
}

/// Weigh this handle's traffic so far and move the queue to the shape that
/// fits it, which is also when an `auto_order` rate is checked.
/// `out_changed`, when not null, receives whether it moved, and
/// `out_shape`, when not null, the shape it is in afterward.
///
/// # Safety
/// `out_changed` and `out_shape` are null or valid pointers.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn subetha_adaptive_queue_maybe_change_shape(
    handle: subetha_handle,
    out_changed: *mut bool,
    out_shape: *mut u32,
) -> i32 {
    with_queue(handle, |q| match q.queue.maybe_promote() {
        Ok(moved) => {
            // SAFETY: each is checked non-null; the caller guarantees it is writable.
            unsafe {
                if !out_changed.is_null() {
                    *out_changed = moved.is_some();
                }
                if !out_shape.is_null() {
                    *out_shape = shape_of(q.queue.active_family());
                }
            }
            SUBETHA_OK
        }
        Err(e) => api_code(e),
    })
}

/// Move the queue to `shape`, whatever its traffic says; a move to the
/// shape it is already in changes nothing.
#[unsafe(no_mangle)]
pub extern "C" fn subetha_adaptive_queue_change_shape_to(handle: subetha_handle, shape: u32) -> i32 {
    with_queue(handle, |q| {
        let family = match family_of(shape) {
            Ok(f) => f,
            Err(code) => return code,
        };
        match q.queue.migrate_to(family) {
            Ok(()) => SUBETHA_OK,
            Err(e) => api_code(e),
        }
    })
}

/// Set whose order a reader gets: `SUBETHA_QOS_PER_PRODUCER` or
/// `SUBETHA_QOS_GLOBAL_FIFO`.
#[unsafe(no_mangle)]
pub extern "C" fn subetha_adaptive_queue_set_ordering(handle: subetha_handle, ordering: u32) -> i32 {
    with_queue(handle, |q| {
        let ordering = match ordering_of(ordering) {
            Ok(o) => o,
            Err(code) => return code,
        };
        match q.queue.set_ordering(ordering) {
            Ok(()) => SUBETHA_OK,
            Err(e) => api_code(e),
        }
    })
}

/// A snapshot of this handle into `out`.
///
/// # Safety
/// `out` is a valid pointer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn subetha_adaptive_queue_read_stats(handle: subetha_handle, out: *mut subetha_adaptive_queue_stats) -> i32 {
    with_queue(handle, |q| {
        if out.is_null() {
            return fail(SUBETHA_E_INVALID_ARGUMENT, "out is null");
        }
        let seen = q.queue.profile_snapshot();
        let stats = subetha_adaptive_queue_stats {
            mode: q.mode,
            shape: shape_of(q.queue.active_family()),
            shape_generation: q.queue.pin_generation(),
            ordering: ordering_tag(q.queue.ordering()),
            inversions: q.queue.inversions(),
            average_batch: seen.avg_batch_size(),
            batch_share: seen.batch_ratio(),
            scanner_changes: q.scanner.as_ref().map_or(0, AdaptiveIpcSidecar::promotions_triggered),
        };
        // SAFETY: checked non-null; the caller guarantees it is writable.
        unsafe { *out = stats };
        SUBETHA_OK
    })
}

/// Remove every file a queue at `path` lays out, for a process that ended
/// without destroying its handle; the ring's per-producer files are counted
/// from its peer directory. The contract is `subetha_ring_unlink`'s.
///
/// # Safety
/// `path` is a NUL-terminated UTF-8 string; `report` is null or a valid
/// pointer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn subetha_adaptive_queue_unlink(path: *const c_char, report: *mut subetha_unlink_report) -> i32 {
    entry(|| {
        if let Err(code) = require_initialized() {
            return code;
        }
        let path = match unsafe { text(path, "path") } {
            Ok(p) => Path::new(p),
            Err(code) => return code,
        };
        let found = subetha_cxc::adaptive_ipc::unlink(path, 1);
        unsafe { finish_unlink(found, report) }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::{SUBETHA_E_BUFFER_TOO_SMALL, SUBETHA_E_DESTROYED, SUBETHA_E_INVALID_HANDLE, SUBETHA_E_RING_EMPTY};
    use crate::ring::SUBETHA_WAIT_FOREVER;
    use crate::runtime::{subetha_handle_destroy, subetha_init, SUBETHA_MODE_STRICT};
    use std::ffi::CString;
    use std::path::PathBuf;

    /// A folder of the test's own, so what a queue leaves behind can be
    /// listed; it is removed when the guard drops.
    struct Folder(PathBuf);

    impl Folder {
        fn new(name: &str) -> Self {
            let dir = std::env::temp_dir().join(format!("subetha-ffi-adaptive-queue-{name}-{}", std::process::id()));
            std::fs::create_dir(&dir).expect("a folder of the test's own");
            Self(dir)
        }

        fn queue(&self) -> CString {
            CString::new(self.0.join("queue").to_str().expect("a UTF-8 temp path")).expect("no NUL in the path")
        }

        fn left(&self) -> Vec<std::ffi::OsString> {
            std::fs::read_dir(&self.0).expect("the folder lists").map(|e| e.expect("an entry").file_name()).collect()
        }
    }

    impl Drop for Folder {
        fn drop(&mut self) {
            std::fs::remove_dir(&self.0).expect("the folder is empty and goes");
        }
    }

    /// Items travel with their lengths; a capacity rounds up; the queue
    /// moves shape by hand and reports it; an ordering and an auto-order
    /// rate are checked; and destroying the handle removes every file.
    #[test]
    fn a_queue_carries_items_moves_shape_and_cleans_up() {
        assert_eq!(subetha_init(SUBETHA_MODE_STRICT), SUBETHA_OK);
        let folder = Folder::new("entry");
        let path = folder.queue();
        let (mut q, mut other) = (0, 0);
        let mut buf = [0u8; SUBETHA_ADAPTIVE_QUEUE_ITEM_MAX];
        let mut len = 0usize;
        // SAFETY: valid paths, buffers and out pointers throughout.
        unsafe {
            let never = SUBETHA_AUTO_ORDER_NEVER;
            let per = SUBETHA_QOS_PER_PRODUCER;
            assert_eq!(subetha_adaptive_queue_create(path.as_ptr(), 3, 1, 1, per, never, SUBETHA_MODE_STRICT, &mut q), SUBETHA_OK);
            for bad in [-0.5, f64::NAN, f64::INFINITY] {
                assert_eq!(subetha_adaptive_queue_create(path.as_ptr(), 4, 1, 1, per, bad, SUBETHA_MODE_STRICT, &mut other), SUBETHA_E_INVALID_ARGUMENT);
            }
            assert_eq!(subetha_adaptive_queue_create(path.as_ptr(), 4, 1, 1, 9, never, SUBETHA_MODE_STRICT, &mut other), SUBETHA_E_INVALID_ARGUMENT);
            assert_eq!(subetha_adaptive_queue_create(path.as_ptr(), 0, 1, 1, per, never, SUBETHA_MODE_STRICT, &mut other), SUBETHA_E_INVALID_ARGUMENT);

            assert_eq!(subetha_adaptive_queue_try_send(q, b"first".as_ptr(), 5), SUBETHA_OK);
            let long = [7u8; SUBETHA_ADAPTIVE_QUEUE_ITEM_MAX + 1];
            assert_eq!(subetha_adaptive_queue_try_send(q, long.as_ptr(), long.len()), SUBETHA_E_RING_PAYLOAD_TOO_LARGE);
            assert_eq!(subetha_adaptive_queue_try_recv(q, buf.as_mut_ptr(), buf.len() - 1, &mut len), SUBETHA_E_BUFFER_TOO_SMALL);
            assert_eq!(subetha_adaptive_queue_try_recv(q, buf.as_mut_ptr(), buf.len(), &mut len), SUBETHA_OK);
            assert_eq!(&buf[..len], b"first");
            assert_eq!(subetha_adaptive_queue_try_recv(q, buf.as_mut_ptr(), buf.len(), &mut len), SUBETHA_E_RING_EMPTY);

            let mut stats = subetha_adaptive_queue_stats::default();
            assert_eq!(subetha_adaptive_queue_read_stats(q, &mut stats), SUBETHA_OK);
            assert_eq!((stats.mode, stats.shape, stats.ordering, stats.scanner_changes), (SUBETHA_MODE_STRICT, SUBETHA_ADAPTIVE_SHAPE_RING, per, 0));
            let before = stats.shape_generation;
            assert_eq!(subetha_adaptive_queue_change_shape_to(q, SUBETHA_ADAPTIVE_SHAPE_WORK_STEALING), SUBETHA_OK);
            assert_eq!(subetha_adaptive_queue_change_shape_to(q, 7), SUBETHA_E_INVALID_ARGUMENT);
            assert_eq!(subetha_adaptive_queue_read_stats(q, &mut stats), SUBETHA_OK);
            assert_eq!(stats.shape, SUBETHA_ADAPTIVE_SHAPE_WORK_STEALING);
            assert!(stats.shape_generation > before, "a change is visible to a holder");
            assert_eq!(subetha_adaptive_queue_send_wait(q, b"after".as_ptr(), 5, 1000), SUBETHA_OK);
            assert_eq!(subetha_adaptive_queue_recv_wait(q, buf.as_mut_ptr(), buf.len(), &mut len, 1000), SUBETHA_OK);
            assert_eq!(&buf[..len], b"after", "the new shape carries items");
            assert_eq!(subetha_adaptive_queue_recv_wait(q, buf.as_mut_ptr(), buf.len(), &mut len, 0), SUBETHA_E_TIMEOUT);

            let (mut changed, mut shape) = (true, u32::MAX);
            assert_eq!(subetha_adaptive_queue_maybe_change_shape(q, &mut changed, &mut shape), SUBETHA_OK);
            assert!(shape == SUBETHA_ADAPTIVE_SHAPE_RING || shape == SUBETHA_ADAPTIVE_SHAPE_WORK_STEALING);
            assert_eq!(subetha_adaptive_queue_set_ordering(q, SUBETHA_QOS_GLOBAL_FIFO), SUBETHA_OK);
            assert_eq!(subetha_adaptive_queue_read_stats(q, &mut stats), SUBETHA_OK);
            assert_eq!(stats.ordering, SUBETHA_QOS_GLOBAL_FIFO);
            assert_eq!(subetha_adaptive_queue_set_ordering(q, 9), SUBETHA_E_INVALID_ARGUMENT);

            assert!(!folder.left().is_empty(), "a live queue holds its files");
            assert_eq!(subetha_handle_destroy(q), SUBETHA_OK);
            assert_eq!(folder.left(), Vec::<std::ffi::OsString>::new(), "destroying the handle removes every file");
        }
    }

    /// A batch stops at a full queue and reports how many went, a batch
    /// receive hands each item back with its length, and a wait with
    /// nothing coming ends when the handle is destroyed.
    #[test]
    fn a_batch_stops_at_a_full_queue_and_a_wait_ends_on_a_destroy() {
        assert_eq!(subetha_init(SUBETHA_MODE_STRICT), SUBETHA_OK);
        let folder = Folder::new("batch");
        let path = folder.queue();
        let mut q = 0;
        const STRIDE: usize = 4;
        let items: Vec<u8> = (0..10u8).flat_map(|n| [b'a' + n, 0, 0, 0]).collect();
        let lens = [1usize; 10];
        let mut done = 0usize;
        let stride = SUBETHA_ADAPTIVE_QUEUE_ITEM_MAX;
        let mut out = vec![0u8; 16 * stride];
        let mut out_lens = [usize::MAX; 16];
        // SAFETY: valid paths, arrays and out pointers throughout.
        unsafe {
            let never = SUBETHA_AUTO_ORDER_NEVER;
            assert_eq!(subetha_adaptive_queue_create(path.as_ptr(), 4, 1, 1, SUBETHA_QOS_PER_PRODUCER, never, SUBETHA_MODE_STRICT, &mut q), SUBETHA_OK);
            assert_eq!(subetha_adaptive_queue_try_send_many(q, items.as_ptr(), STRIDE, lens.as_ptr(), 10, &mut done), SUBETHA_OK);
            assert!(done > 0 && done < 10, "a queue of four takes part of ten, took {done}");
            let sent = done;
            assert_eq!(subetha_adaptive_queue_try_send_many(q, items.as_ptr(), STRIDE, lens.as_ptr(), 1, &mut done), SUBETHA_E_RING_FULL);
            assert_eq!(done, 0);
            let mut stats = subetha_adaptive_queue_stats::default();
            assert_eq!(subetha_adaptive_queue_read_stats(q, &mut stats), SUBETHA_OK);
            assert!(stats.batch_share > 0.0, "the run was counted as a batch");

            assert_eq!(subetha_adaptive_queue_try_recv_many(q, out.as_mut_ptr(), stride, out_lens.as_mut_ptr(), 16, &mut done), SUBETHA_OK);
            assert_eq!(done, sent);
            for (i, chunk) in out.chunks(stride).take(sent).enumerate() {
                assert_eq!((chunk[0], out_lens[i]), (b'a' + i as u8, 1));
            }
        }
        let q_handle = q;
        let admitted = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let waiter = {
            let admitted = Arc::clone(&admitted);
            std::thread::spawn(move || {
                // Admitted as an entry point admits its call, so the destroy
                // below can only end this wait; one that came before the
                // admission would refuse the call instead.
                let borrowed = crate::runtime::table()
                    .borrow(q_handle, SUBETHA_KIND_ADAPTIVE_QUEUE)
                    .expect("the queue is live");
                admitted.store(true, std::sync::atomic::Ordering::Release);
                match borrowed.object() {
                    Object::AdaptiveQueue(queue) => queue.recv_wait(None).err(),
                    _ => None,
                }
            })
        };
        let deadline = Instant::now() + Duration::from_secs(30);
        while !admitted.load(std::sync::atomic::Ordering::Acquire) {
            assert!(Instant::now() < deadline, "the waiter was admitted within 30 s");
            std::thread::yield_now();
        }
        assert_eq!(subetha_handle_destroy(q), SUBETHA_OK);
        assert_eq!(waiter.join().expect("the waiter ends"), Some(SUBETHA_E_DESTROYED));
        let mut buf = [0u8; SUBETHA_ADAPTIVE_QUEUE_ITEM_MAX];
        let mut len = 0usize;
        // SAFETY: a valid buffer and out pointer.
        let refused = unsafe { subetha_adaptive_queue_recv_wait(q, buf.as_mut_ptr(), buf.len(), &mut len, SUBETHA_WAIT_FOREVER) };
        assert_eq!(refused, SUBETHA_E_INVALID_HANDLE, "a call after the destroy is refused");
        assert!(folder.left().is_empty(), "destroying the handle removes every file");
    }

    /// A managed queue runs a scanner, which stops when the handle is
    /// destroyed, and a queue a process left behind is removed by unlink.
    #[test]
    fn a_managed_queue_scans_and_unlink_removes_what_is_left() {
        assert_eq!(subetha_init(SUBETHA_MODE_STRICT), SUBETHA_OK);
        let folder = Folder::new("managed");
        let path = folder.queue();
        let mut q = 0;
        // SAFETY: valid paths and out pointers throughout.
        unsafe {
            let never = SUBETHA_AUTO_ORDER_NEVER;
            assert_eq!(subetha_adaptive_queue_create(path.as_ptr(), 8, 1, 1, SUBETHA_QOS_PER_PRODUCER, never, SUBETHA_MODE_MANAGED, &mut q), SUBETHA_OK);
            let mut stats = subetha_adaptive_queue_stats::default();
            assert_eq!(subetha_adaptive_queue_read_stats(q, &mut stats), SUBETHA_OK);
            assert_eq!(stats.mode, SUBETHA_MODE_MANAGED);
            assert_eq!(subetha_handle_destroy(q), SUBETHA_OK);
        }
        assert!(folder.left().is_empty(), "the scanner stops and the files go with the handle");

        let left = AdaptiveIpc::<SlotValue>::create(
            folder.0.join("queue"),
            MmfWorkloadShape::StreamingMpmc { n_producers: 1, n_consumers: 1 },
            8,
            1,
        )
        .expect("a queue to leave behind");
        std::mem::forget(left);
        assert!(!folder.left().is_empty());
        let mut report = subetha_unlink_report::default();
        // SAFETY: a valid path and report pointer.
        assert_eq!(unsafe { subetha_adaptive_queue_unlink(path.as_ptr(), &mut report) }, SUBETHA_OK);
        assert_eq!(report.failed, 0);
        assert!(report.removed > 0);
        assert!(folder.left().is_empty(), "unlink removes every file a queue lays out");
    }
}
