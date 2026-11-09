//! The front door's channel through the C ABI: a queue between processes
//! whose shape is picked from the senders and readers named, with a try
//! and a waiting form of send and receive. An item is up to
//! `SUBETHA_CHANNEL_ITEM_MAX` bytes and travels with its length, encoded
//! as Python's and PowerShell's channels encode it, so a channel any of
//! them made at a path is the same channel to C. The channel lives in a
//! file beside two waker files, `<path>.cw` and `<path>.pw`; strict and
//! managed modes are the same here, the channel runs no background work.

use std::ffi::c_char;
use std::path::{Path, PathBuf};
use std::time::Instant;

use subetha_cxc::adaptive_ring::UnlinkReport;
use subetha_cxc::api::{ApiError, Channel, BLOCKING_HEAL};
use subetha_cxc::byte_value::{SlotValue, SLOT_VALUE_BYTES};
use subetha_cxc::mmf_dispatcher::MmfWorkloadShape;

use crate::error::{
    api_code, fail, SUBETHA_E_INVALID_ARGUMENT, SUBETHA_E_RING_PAYLOAD_TOO_LARGE, SUBETHA_E_TIMEOUT,
    SUBETHA_E_WRONG_KIND, SUBETHA_OK,
};
use crate::batch::{run_pop_many_indexed, run_push_many_lens};
use crate::handle::{subetha_handle, Object, SUBETHA_KIND_CHANNEL};
use crate::ring::{bytes, deadline_from, finish_unlink, out_buffer, subetha_unlink_report, text, with_suffix};
use crate::runtime::{entry, issue, require_initialized, resolve_mode, with_kind};
use crate::wait::{slice_until, Waiting};

/// The most an item may be, in bytes. A receive buffer must be at least
/// this large.
pub const SUBETHA_CHANNEL_ITEM_MAX: usize = 52;

const _: () = assert!(SUBETHA_CHANNEL_ITEM_MAX == SLOT_VALUE_BYTES);

/// The files beside the channel's own: its consumer and producer wakers.
const WAKER_SUFFIXES: [&str; 2] = [".cw", ".pw"];

pub(crate) struct ChannelObject {
    channel: Channel<SlotValue>,
    waiting: Waiting,
}

impl ChannelObject {
    /// Raise the flag every wait reads between slices, so a destroy can
    /// proceed; each wait returns `SUBETHA_E_DESTROYED` within one slice.
    pub(crate) fn interrupt(&self) {
        self.waiting.close();
    }

    fn try_send(&self, item: &SlotValue) -> Result<(), i32> {
        self.channel.send(item).map_err(api_code)
    }

    fn try_recv(&self) -> Result<SlotValue, i32> {
        self.channel.recv().map_err(api_code)
    }

    /// The channel's own blocking send, one heal tick at a time: the
    /// destroy flag is read between ticks, at no wakeup the send would not
    /// make anyway.
    fn send_wait(&self, item: &SlotValue, deadline: Option<Instant>) -> Result<(), i32> {
        slice_until(&self.waiting, BLOCKING_HEAL, deadline, SUBETHA_E_TIMEOUT, |slice| {
            self.channel.send_blocking(item, Some(slice)).map_err(api_code)
        })
    }

    /// The channel's own blocking receive, one heal tick at a time.
    fn recv_wait(&self, deadline: Option<Instant>) -> Result<SlotValue, i32> {
        slice_until(&self.waiting, BLOCKING_HEAL, deadline, SUBETHA_E_TIMEOUT, |slice| {
            self.channel.recv_blocking(Some(slice)).map_err(api_code)
        })
    }
}

fn with_channel(handle: subetha_handle, f: impl FnOnce(&ChannelObject) -> i32) -> i32 {
    with_kind(handle, SUBETHA_KIND_CHANNEL, |object| match object {
        Object::Channel(c) => f(c),
        _ => fail(SUBETHA_E_WRONG_KIND, "the handle does not name a channel"),
    })
}

/// The ring capacity a requested `capacity` makes: at least 2 and a power
/// of two, rounded up as the Python and PowerShell channels round it, so
/// the same request reaches the same file from any of them.
fn channel_capacity(capacity: u32) -> Result<usize, i32> {
    if capacity == 0 {
        return Err(fail(SUBETHA_E_INVALID_ARGUMENT, "a channel's capacity must be at least 1"));
    }
    (capacity as usize).max(2).checked_next_power_of_two().ok_or_else(|| {
        fail(SUBETHA_E_INVALID_ARGUMENT, format!("capacity {capacity} rounds past what this platform addresses"))
    })
}

/// The arguments both constructors read, in order, so the first refusal
/// names the argument at fault.
///
/// # Safety
/// `path` is a NUL-terminated UTF-8 string; `out` is a valid pointer.
unsafe fn read_arguments<'a>(
    path: *const c_char,
    capacity: u32,
    mode: u32,
    out: *mut subetha_handle,
) -> Result<(&'a Path, usize), i32> {
    require_initialized()?;
    if out.is_null() {
        return Err(fail(SUBETHA_E_INVALID_ARGUMENT, "out is null"));
    }
    let path = Path::new(unsafe { text(path, "path") }?);
    let capacity = channel_capacity(capacity)?;
    // Both modes behave alike here; an unknown one is still refused.
    resolve_mode(mode)?;
    Ok((path, capacity))
}

fn place(channel: Result<Channel<SlotValue>, ApiError>, out: *mut subetha_handle) -> i32 {
    match channel {
        Ok(channel) => unsafe { issue(Object::Channel(ChannelObject { channel, waiting: Waiting::new() }), out) },
        Err(e) => api_code(e),
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

/// Copy `held`'s bytes to the front of `buf`, which `out_buffer` sized to
/// at least `SUBETHA_CHANNEL_ITEM_MAX`, and its length to `out_len`.
///
/// # Safety
/// `out_len` is a valid pointer, as `out_buffer` checked.
unsafe fn deliver(held: &SlotValue, buf: &mut [u8], out_len: *mut usize) -> i32 {
    let taken = held.as_bytes();
    buf[..taken.len()].copy_from_slice(taken);
    // SAFETY: checked non-null; the caller guarantees it is writable.
    unsafe { *out_len = taken.len() };
    SUBETHA_OK
}

/// Make the channel at `path` for `senders` senders and `readers` readers,
/// with `capacity` items in flight rounded up to a power of two. The file
/// is made afresh, so the items of a channel already there are lost; a
/// process joining a channel another made calls `subetha_channel_open`.
/// The shape underneath is picked from `senders` and `readers`.
///
/// # Safety
/// `path` is a NUL-terminated UTF-8 string; `out` is a valid pointer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn subetha_channel_create(
    path: *const c_char,
    capacity: u32,
    senders: u32,
    readers: u32,
    mode: u32,
    out: *mut subetha_handle,
) -> i32 {
    entry(|| {
        let (path, capacity) = match unsafe { read_arguments(path, capacity, mode, out) } {
            Ok(a) => a,
            Err(code) => return code,
        };
        if senders == 0 || readers == 0 {
            return fail(SUBETHA_E_INVALID_ARGUMENT, "a channel needs at least one sender and one reader");
        }
        let shape = MmfWorkloadShape::StreamingMpmc { n_producers: senders as usize, n_consumers: readers as usize };
        place(Channel::create(path, shape, capacity), out)
    })
}

/// Attach to the channel another process made at `path`, which must exist,
/// asking for the `capacity` it was made with; the rounding is
/// `subetha_channel_create`'s, so the same number reaches the same file.
/// `SUBETHA_E_IO` names an absent file and `SUBETHA_E_LAYOUT_MISMATCH` one
/// of another capacity.
///
/// # Safety
/// `path` is a NUL-terminated UTF-8 string; `out` is a valid pointer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn subetha_channel_open(path: *const c_char, capacity: u32, mode: u32, out: *mut subetha_handle) -> i32 {
    entry(|| {
        let (path, capacity) = match unsafe { read_arguments(path, capacity, mode, out) } {
            Ok(a) => a,
            Err(code) => return code,
        };
        place(Channel::open(path, capacity), out)
    })
}

/// Send the `len` bytes at `data` without waiting. `SUBETHA_E_RING_FULL`
/// when the channel is full; `SUBETHA_E_RING_PAYLOAD_TOO_LARGE` for more
/// than `SUBETHA_CHANNEL_ITEM_MAX` bytes.
///
/// # Safety
/// `data` is null or points to `len` readable bytes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn subetha_channel_try_send(handle: subetha_handle, data: *const u8, len: usize) -> i32 {
    with_channel(handle, |c| {
        let held = match unsafe { item(data, len) } {
            Ok(h) => h,
            Err(code) => return code,
        };
        match c.try_send(&held) {
            Ok(()) => SUBETHA_OK,
            Err(code) => code,
        }
    })
}

/// Send, waiting for room until `timeout_ms` has passed
/// (`SUBETHA_E_TIMEOUT`); `SUBETHA_WAIT_FOREVER` waits without a deadline,
/// and a timeout of zero tries once. `SUBETHA_E_DESTROYED` means the handle
/// was destroyed while this call waited.
///
/// # Safety
/// `data` is null or points to `len` readable bytes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn subetha_channel_send_wait(handle: subetha_handle, data: *const u8, len: usize, timeout_ms: i64) -> i32 {
    with_channel(handle, |c| {
        let held = match unsafe { item(data, len) } {
            Ok(h) => h,
            Err(code) => return code,
        };
        let deadline = match deadline_from(timeout_ms) {
            Ok(d) => d,
            Err(code) => return code,
        };
        match c.send_wait(&held, deadline) {
            Ok(()) => SUBETHA_OK,
            Err(code) => code,
        }
    })
}

/// Receive the next item into `out` and its length into `out_len`, without
/// waiting. `cap` must be at least `SUBETHA_CHANNEL_ITEM_MAX`, whatever the
/// item's length. `SUBETHA_E_RING_EMPTY` when nothing is there.
///
/// # Safety
/// `out` points to `cap` writable bytes; `out_len` is a valid pointer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn subetha_channel_try_recv(handle: subetha_handle, out: *mut u8, cap: usize, out_len: *mut usize) -> i32 {
    with_channel(handle, |c| {
        let buf = match unsafe { out_buffer(out, cap, out_len, SUBETHA_CHANNEL_ITEM_MAX) } {
            Ok(b) => b,
            Err(code) => return code,
        };
        match c.try_recv() {
            Ok(held) => unsafe { deliver(&held, buf, out_len) },
            Err(code) => code,
        }
    })
}

/// Receive, waiting for an item until `timeout_ms` has passed
/// (`SUBETHA_E_TIMEOUT`); the buffer contract is
/// `subetha_channel_try_recv`'s and the waiting contract
/// `subetha_channel_send_wait`'s.
///
/// # Safety
/// `out` points to `cap` writable bytes; `out_len` is a valid pointer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn subetha_channel_recv_wait(
    handle: subetha_handle,
    out: *mut u8,
    cap: usize,
    out_len: *mut usize,
    timeout_ms: i64,
) -> i32 {
    with_channel(handle, |c| {
        let buf = match unsafe { out_buffer(out, cap, out_len, SUBETHA_CHANNEL_ITEM_MAX) } {
            Ok(b) => b,
            Err(code) => return code,
        };
        let deadline = match deadline_from(timeout_ms) {
            Ok(d) => d,
            Err(code) => return code,
        };
        match c.recv_wait(deadline) {
            Ok(held) => unsafe { deliver(&held, buf, out_len) },
            Err(code) => code,
        }
    })
}

/// Send `count` items without waiting, the first at `items` and each next
/// one `stride` bytes on, item `i` being `lens[i]` bytes, under one handle
/// lookup and one panic guard. A length past `stride` or past
/// `SUBETHA_CHANNEL_ITEM_MAX` is refused. The batch contract is
/// `subetha_ring_try_push_many`'s: it stops at the first item refused,
/// returns `SUBETHA_OK` when any went, and `out_done` counts them.
///
/// # Safety
/// `items` addresses `count * stride` readable bytes, `lens` `count`
/// readable lengths, and `out_done` is a valid pointer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn subetha_channel_try_send_many(
    handle: subetha_handle,
    items: *const u8,
    stride: usize,
    lens: *const usize,
    count: usize,
    out_done: *mut usize,
) -> i32 {
    with_channel(handle, |c| {
        // SAFETY: the caller guarantees the array, the lengths and the count word.
        unsafe {
            run_push_many_lens(items, stride, lens, count, out_done, |payload| {
                let held = match SlotValue::from_bytes(payload) {
                    Ok(h) => h,
                    Err(e) => return fail(SUBETHA_E_RING_PAYLOAD_TOO_LARGE, e.to_string()),
                };
                match c.try_send(&held) {
                    Ok(()) => SUBETHA_OK,
                    Err(code) => code,
                }
            })
        }
    })
}

/// Receive up to `count` items without waiting, the first written at `out`
/// and each next one `stride` bytes on, and each one's length into
/// `out_lens`; `stride` is at least `SUBETHA_CHANNEL_ITEM_MAX`. The batch
/// contract is `subetha_ring_try_pop_many`'s: `SUBETHA_E_RING_EMPTY` when
/// nothing was there, and otherwise `out_done` counts what came.
///
/// # Safety
/// `out` addresses `count * stride` writable bytes, `out_lens` `count`
/// writable lengths, and `out_done` is a valid pointer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn subetha_channel_try_recv_many(
    handle: subetha_handle,
    out: *mut u8,
    stride: usize,
    out_lens: *mut usize,
    count: usize,
    out_done: *mut usize,
) -> i32 {
    with_channel(handle, |c| {
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
            run_pop_many_indexed(out, stride, SUBETHA_CHANNEL_ITEM_MAX, count, out_done, |i, buf| match c.try_recv() {
                Ok(held) => {
                    let taken = held.as_bytes();
                    buf[..taken.len()].copy_from_slice(taken);
                    lens[i] = taken.len();
                    SUBETHA_OK
                }
                Err(code) => code,
            })
        }
    })
}

/// Remove the channel file at `path` and its two waker files. The contract
/// is `subetha_ring_unlink`'s.
///
/// # Safety
/// `path` is a NUL-terminated UTF-8 string; `report` is null or a valid
/// pointer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn subetha_channel_unlink(path: *const c_char, report: *mut subetha_unlink_report) -> i32 {
    entry(|| {
        if let Err(code) = require_initialized() {
            return code;
        }
        let path = match unsafe { text(path, "path") } {
            Ok(p) => Path::new(p),
            Err(code) => return code,
        };
        let mut found = UnlinkReport::default();
        found.remove(PathBuf::from(path));
        for suffix in WAKER_SUFFIXES {
            found.remove(with_suffix(path, suffix));
        }
        unsafe { finish_unlink(found, report) }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::{SUBETHA_E_BUFFER_TOO_SMALL, SUBETHA_E_DESTROYED, SUBETHA_E_LAYOUT_MISMATCH, SUBETHA_E_RING_EMPTY};
    use crate::runtime::{subetha_handle_destroy, subetha_init, SUBETHA_MODE_STRICT};
    use std::ffi::CString;
    use std::sync::Arc;
    use std::time::Duration;

    /// A channel's path for one test under the temp directory; its three
    /// files are removed when the guard drops.
    struct Scratch(PathBuf);

    impl Scratch {
        fn new(name: &str) -> Self {
            Self(std::env::temp_dir().join(format!("subetha-ffi-channel-{name}-{}.bin", std::process::id())))
        }

        fn c_path(&self) -> CString {
            CString::new(self.0.to_str().expect("a UTF-8 temp path")).expect("no NUL in the path")
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let mut found = UnlinkReport::default();
            found.remove(self.0.clone());
            for suffix in WAKER_SUFFIXES {
                found.remove(with_suffix(&self.0, suffix));
            }
            assert_eq!(found.failed, 0, "the channel's files were removed: {:?}", found.first_failure);
        }
    }

    fn object(channel: Channel<SlotValue>) -> ChannelObject {
        ChannelObject { channel, waiting: Waiting::new() }
    }

    fn one_to_one() -> MmfWorkloadShape {
        MmfWorkloadShape::StreamingMpmc { n_producers: 1, n_consumers: 1 }
    }

    /// A channel the C side opens and a Rust channel of the same items on
    /// the same file are one channel: what either sends, the other
    /// receives, with its length.
    #[test]
    fn a_c_channel_and_a_rust_channel_on_one_file_are_one_channel() {
        let scratch = Scratch::new("interop");
        let rust = Channel::<SlotValue>::create(&scratch.0, one_to_one(), 64).expect("the Rust channel");
        let c = object(Channel::<SlotValue>::open(&scratch.0, 64).expect("the C side's channel"));
        rust.send(&SlotValue::from_bytes(b"from rust").expect("fits")).expect("send");
        let held = c.recv_wait(Some(Instant::now() + Duration::from_secs(5))).expect("the C side receives");
        let mut buf = [0u8; SUBETHA_CHANNEL_ITEM_MAX];
        let mut len = 0usize;
        assert_eq!(unsafe { deliver(&held, &mut buf, &mut len) }, SUBETHA_OK);
        assert_eq!(&buf[..len], b"from rust");
        c.try_send(&SlotValue::from_bytes(b"from c").expect("fits")).expect("send");
        assert_eq!(rust.recv().expect("the Rust side receives").as_bytes(), b"from c");
        assert_eq!(c.try_recv().err(), Some(SUBETHA_E_RING_EMPTY));
        drop(c);
        drop(rust);
    }

    /// A wait with nothing coming ends on its deadline, and one with no
    /// deadline ends when the handle is destroyed.
    #[test]
    fn a_wait_ends_on_its_deadline_and_on_a_destroy() {
        let scratch = Scratch::new("waits");
        let c = Arc::new(object(Channel::<SlotValue>::create(&scratch.0, one_to_one(), 2).expect("a channel")));
        assert_eq!(c.recv_wait(Some(Instant::now())).err(), Some(SUBETHA_E_TIMEOUT), "a zero timeout tries once");
        let soon = Some(Instant::now() + Duration::from_millis(20));
        assert_eq!(c.recv_wait(soon).err(), Some(SUBETHA_E_TIMEOUT));
        let one = SlotValue::from_bytes(b"x").expect("fits");
        c.try_send(&one).expect("first slot");
        c.try_send(&one).expect("second slot");
        let soon = Some(Instant::now() + Duration::from_millis(20));
        assert_eq!(c.send_wait(&one, soon).err(), Some(SUBETHA_E_TIMEOUT), "a full channel holds a send to its deadline");
        assert_eq!(c.try_recv().expect("an item").as_bytes(), b"x");
        assert_eq!(c.try_recv().expect("an item").as_bytes(), b"x");
        let waiter = {
            let c = Arc::clone(&c);
            std::thread::spawn(move || c.recv_wait(None).err())
        };
        std::thread::sleep(Duration::from_millis(20));
        c.interrupt();
        assert_eq!(waiter.join().expect("the waiter ends"), Some(SUBETHA_E_DESTROYED));
        drop(c);
    }

    /// A batch send takes a length per item and a batch receive hands each
    /// back with its own; a length past the stride is refused, and so is a
    /// receive stride narrower than the longest item.
    #[test]
    fn a_batch_keeps_each_items_length() {
        assert_eq!(subetha_init(SUBETHA_MODE_STRICT), SUBETHA_OK);
        let scratch = Scratch::new("batch");
        let path = scratch.c_path();
        let mut h = 0;
        const STRIDE: usize = 8;
        let items: [u8; 3 * STRIDE] = *b"a\0\0\0\0\0\0\0bb\0\0\0\0\0\0ccc\0\0\0\0\0";
        let lens = [1usize, 2, 3];
        let past = [STRIDE + 1];
        let mut done = 0usize;
        let mut out = [0u8; 4 * SUBETHA_CHANNEL_ITEM_MAX];
        let mut out_lens = [usize::MAX; 4];
        // SAFETY: valid paths, arrays and out pointers throughout.
        unsafe {
            assert_eq!(subetha_channel_create(path.as_ptr(), 8, 1, 1, SUBETHA_MODE_STRICT, &mut h), SUBETHA_OK);
            assert_eq!(subetha_channel_try_send_many(h, items.as_ptr(), STRIDE, lens.as_ptr(), 3, &mut done), SUBETHA_OK);
            assert_eq!(done, 3);
            assert_eq!(
                subetha_channel_try_send_many(h, items.as_ptr(), STRIDE, past.as_ptr(), 1, &mut done),
                SUBETHA_E_INVALID_ARGUMENT
            );
            assert_eq!(done, 0);

            let narrow = SUBETHA_CHANNEL_ITEM_MAX - 1;
            assert_eq!(
                subetha_channel_try_recv_many(h, out.as_mut_ptr(), narrow, out_lens.as_mut_ptr(), 4, &mut done),
                SUBETHA_E_INVALID_ARGUMENT
            );
            let stride = SUBETHA_CHANNEL_ITEM_MAX;
            assert_eq!(subetha_channel_try_recv_many(h, out.as_mut_ptr(), stride, out_lens.as_mut_ptr(), 4, &mut done), SUBETHA_OK);
            assert_eq!(done, 3, "three items were there");
            assert_eq!(&out_lens[..3], &[1, 2, 3]);
            assert_eq!(out_lens[3], usize::MAX, "a slot nothing filled is left alone");
            assert_eq!(&out[..1], b"a");
            assert_eq!(&out[stride..stride + 2], b"bb");
            assert_eq!(&out[2 * stride..2 * stride + 3], b"ccc");
            assert_eq!(
                subetha_channel_try_recv_many(h, out.as_mut_ptr(), stride, out_lens.as_mut_ptr(), 4, &mut done),
                SUBETHA_E_RING_EMPTY
            );
            assert_eq!(done, 0);
            assert_eq!(subetha_handle_destroy(h), SUBETHA_OK);
        }
    }

    /// Through the entry points: a capacity is rounded up, so an open
    /// asking for the same number reaches the file; another capacity, an
    /// over-long item and a short buffer are each refused with their own
    /// code; an empty item travels; and an unlink removes the channel's
    /// three files.
    #[test]
    fn the_entry_points_round_refuse_and_unlink() {
        assert_eq!(subetha_init(SUBETHA_MODE_STRICT), SUBETHA_OK);
        let scratch = Scratch::new("entry");
        let path = scratch.c_path();
        let (mut sender, mut reader, mut other) = (0, 0, 0);
        // SAFETY: valid paths, buffers and out pointers throughout.
        unsafe {
            assert_eq!(subetha_channel_create(path.as_ptr(), 1000, 1, 1, SUBETHA_MODE_STRICT, &mut sender), SUBETHA_OK);
            assert_eq!(subetha_channel_open(path.as_ptr(), 1000, SUBETHA_MODE_STRICT, &mut reader), SUBETHA_OK);
            assert_eq!(subetha_channel_open(path.as_ptr(), 1024, SUBETHA_MODE_STRICT, &mut other), SUBETHA_OK);
            assert_eq!(subetha_handle_destroy(other), SUBETHA_OK);
            assert_eq!(subetha_channel_open(path.as_ptr(), 64, SUBETHA_MODE_STRICT, &mut other), SUBETHA_E_LAYOUT_MISMATCH);
            assert_eq!(subetha_channel_create(path.as_ptr(), 0, 1, 1, SUBETHA_MODE_STRICT, &mut other), SUBETHA_E_INVALID_ARGUMENT);
            assert_eq!(subetha_channel_create(path.as_ptr(), 8, 0, 1, SUBETHA_MODE_STRICT, &mut other), SUBETHA_E_INVALID_ARGUMENT);

            let long = [7u8; SUBETHA_CHANNEL_ITEM_MAX + 1];
            assert_eq!(subetha_channel_try_send(sender, long.as_ptr(), long.len()), SUBETHA_E_RING_PAYLOAD_TOO_LARGE);
            let most = [7u8; SUBETHA_CHANNEL_ITEM_MAX];
            assert_eq!(subetha_channel_try_send(sender, most.as_ptr(), most.len()), SUBETHA_OK);
            assert_eq!(subetha_channel_send_wait(sender, b"two".as_ptr(), 3, 1000), SUBETHA_OK);
            assert_eq!(subetha_channel_try_send(sender, std::ptr::null(), 0), SUBETHA_OK, "an empty item is an item");

            let mut buf = [0u8; SUBETHA_CHANNEL_ITEM_MAX];
            let mut len = usize::MAX;
            assert_eq!(subetha_channel_try_recv(reader, buf.as_mut_ptr(), buf.len() - 1, &mut len), SUBETHA_E_BUFFER_TOO_SMALL);
            assert_eq!(subetha_channel_try_recv(reader, buf.as_mut_ptr(), buf.len(), &mut len), SUBETHA_OK);
            assert_eq!(&buf[..len], &most[..]);
            assert_eq!(subetha_channel_recv_wait(reader, buf.as_mut_ptr(), buf.len(), &mut len, 1000), SUBETHA_OK);
            assert_eq!(&buf[..len], b"two");
            assert_eq!(subetha_channel_recv_wait(reader, buf.as_mut_ptr(), buf.len(), &mut len, 0), SUBETHA_OK);
            assert_eq!(len, 0);
            assert_eq!(subetha_channel_recv_wait(reader, buf.as_mut_ptr(), buf.len(), &mut len, 0), SUBETHA_E_TIMEOUT);
            assert_eq!(subetha_channel_try_recv(sender, buf.as_mut_ptr(), buf.len(), &mut len), SUBETHA_E_RING_EMPTY);

            assert_eq!(subetha_handle_destroy(sender), SUBETHA_OK);
            assert_eq!(subetha_handle_destroy(reader), SUBETHA_OK);
            let mut report = subetha_unlink_report::default();
            assert_eq!(subetha_channel_unlink(path.as_ptr(), &mut report), SUBETHA_OK);
            assert_eq!((report.removed, report.missing, report.failed), (3, 0, 0), "the ring and both waker files go");
        }
    }
}
