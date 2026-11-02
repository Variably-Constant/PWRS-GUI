//! Codes, their names, and the per-thread detail behind a failing call.
//!
//! One integer space: the hundreds digit is the domain. Codes below 100
//! belong to the runtime and the handle table, 100 to 199 to rings. Later
//! tiers take the next hundreds. A code alone is enough to act on;
//! [`subetha_last_error_detail`] carries the specifics (a path, two sizes
//! that disagreed) for the call that just failed on this thread.

use std::cell::{Cell, RefCell};
use std::ffi::{c_char, CStr};

/// The call succeeded.
pub const SUBETHA_OK: i32 = 0;

/// No call may precede `subetha_init`.
pub const SUBETHA_E_NOT_INITIALIZED: i32 = 1;
/// `subetha_init` was called again with a different default mode.
pub const SUBETHA_E_INIT_CONFLICT: i32 = 2;
/// An argument was null, zero where a count is required, or out of range.
pub const SUBETHA_E_INVALID_ARGUMENT: i32 = 3;
/// The handle names no live object: never issued, already destroyed, or
/// issued by an earlier occupant of the same table slot.
pub const SUBETHA_E_INVALID_HANDLE: i32 = 4;
/// A panic inside an earlier call on this handle left the object
/// unusable. Destroy it and create another.
pub const SUBETHA_E_HANDLE_POISONED: i32 = 5;
/// This call panicked. The handle it ran on, if any, is now poisoned;
/// `subetha_last_panic_message` has the text.
pub const SUBETHA_E_PANIC: i32 = 6;
/// The handle is live but names an object of another kind.
pub const SUBETHA_E_WRONG_KIND: i32 = 7;
/// An allocation failed.
pub const SUBETHA_E_OUT_OF_MEMORY: i32 = 8;
/// A caller-provided buffer is too small; the detail names the size needed.
pub const SUBETHA_E_BUFFER_TOO_SMALL: i32 = 9;
/// A string argument is not valid UTF-8.
pub const SUBETHA_E_INVALID_UTF8: i32 = 10;
/// The timeout elapsed before the condition held.
pub const SUBETHA_E_TIMEOUT: i32 = 11;
/// The library has been shut down; `subetha_init` again before any other call.
pub const SUBETHA_E_SHUT_DOWN: i32 = 12;
/// `subetha_shutdown` found live handles and closed them; the detail says how many.
pub const SUBETHA_E_HANDLES_WERE_LIVE: i32 = 13;
/// The requested behavior exists in the library but not on this platform
/// or in this mode.
pub const SUBETHA_E_NOT_SUPPORTED: i32 = 14;
/// The object was destroyed while this call was waiting on it.
pub const SUBETHA_E_DESTROYED: i32 = 15;

/// The ring is full.
pub const SUBETHA_E_RING_FULL: i32 = 100;
/// The ring is empty.
pub const SUBETHA_E_RING_EMPTY: i32 = 101;
/// A backing exists but its magic or capacity does not match the request.
pub const SUBETHA_E_RING_LAYOUT_MISMATCH: i32 = 102;
/// The payload exceeds what one slot carries.
pub const SUBETHA_E_RING_PAYLOAD_TOO_LARGE: i32 = 103;
/// The operation needs ordering stamps the ring was not built with.
pub const SUBETHA_E_RING_NOT_STAMPED: i32 = 104;
/// Another consumer holds the drainer lease; back off and retry.
pub const SUBETHA_E_RING_NOT_DRAINER: i32 = 105;
/// A morph was asked for while the previous shape still holds a backlog.
pub const SUBETHA_E_RING_STALE_BACKLOG: i32 = 106;
/// The OS refused an open, map, or remove; the detail carries the
/// operating system's message.
pub const SUBETHA_E_RING_IO: i32 = 107;
/// A backing of any family exists with a layout other than the one asked
/// for: the code `SUBETHA_E_RING_LAYOUT_MISMATCH` names, under a name for
/// the structures that are not rings.
pub const SUBETHA_E_LAYOUT_MISMATCH: i32 = 102;
/// The OS refused an open, map, or remove for a structure of any family:
/// the code `SUBETHA_E_RING_IO` names, under a name for the structures
/// that are not rings.
pub const SUBETHA_E_IO: i32 = 107;

const _: () = {
    assert!(SUBETHA_E_LAYOUT_MISMATCH == SUBETHA_E_RING_LAYOUT_MISMATCH);
    assert!(SUBETHA_E_IO == SUBETHA_E_RING_IO);
};
/// Producer registration refused by a declared contract or the slot ceiling.
pub const SUBETHA_E_RING_TOO_MANY_PRODUCERS: i32 = 108;
/// Consumer registration refused by a declared contract or the slot ceiling.
pub const SUBETHA_E_RING_TOO_MANY_CONSUMERS: i32 = 109;
/// Registration claimed a slot but the grown backing could not be created.
pub const SUBETHA_E_RING_GROWTH_FAILED: i32 = 110;
/// Every waiter slot is in use; poll with the try variant instead.
pub const SUBETHA_E_RING_WAKER_FULL: i32 = 111;
/// A waker region exists but does not match the ring's request.
pub const SUBETHA_E_RING_WAKER_LAYOUT: i32 = 112;
/// Every consumer slot of the broadcast ring is taken.
pub const SUBETHA_E_BROADCAST_NO_CONSUMER_SLOT: i32 = 113;
/// The consumer index is out of range or not registered.
pub const SUBETHA_E_BROADCAST_INVALID_CONSUMER: i32 = 114;
/// The position has not been published yet.
pub const SUBETHA_E_PUBSUB_PENDING: i32 = 115;
/// The position was overwritten before it was read; the subscriber's
/// position has moved to the ring's head.
pub const SUBETHA_E_PUBSUB_LOST: i32 = 116;
/// The deque handle is a thief's; push and pop belong to the owner.
pub const SUBETHA_E_DEQUE_NOT_OWNER: i32 = 117;
/// The map has no free slot for the key.
pub const SUBETHA_E_MAP_FULL: i32 = 118;
/// The key has no entry in the map.
pub const SUBETHA_E_MAP_KEY_ABSENT: i32 = 119;
/// The arena has no room for the whole value.
pub const SUBETHA_E_ARENA_FULL: i32 = 120;
/// The reference reaches past what the arena holds.
pub const SUBETHA_E_ARENA_INVALID_REF: i32 = 121;
/// The handle was opened read-only and the call writes.
pub const SUBETHA_E_READ_ONLY: i32 = 122;
/// The index is at or past the length or the capacity.
pub const SUBETHA_E_OUT_OF_BOUNDS: i32 = 123;
/// Every pin slot in the epoch table is held by a live process.
pub const SUBETHA_E_EPOCHS_PINS_EXHAUSTED: i32 = 124;
/// Every ticket slot in the epoch table is held by a live process.
pub const SUBETHA_E_EPOCHS_TICKETS_EXHAUSTED: i32 = 125;
/// The call would have to wait to succeed: something it needs is held by
/// another participant or not free yet, and the same call can land later.
pub const SUBETHA_E_WOULD_BLOCK: i32 = 126;
// Code 127 is out of use, and no new code takes it.
/// Every slot in the fence clock is registered.
pub const SUBETHA_E_FENCE_CLOCK_FULL: i32 = 128;
/// The barrier has no live peer to wait for, so a wait would never be
/// released by anyone.
pub const SUBETHA_E_BARRIER_NO_LIVE_PEERS: i32 = 129;
/// The epoch waited on is behind the one the barrier has reached, so the
/// caller is asking to join a rendezvous that is already over.
pub const SUBETHA_E_BARRIER_EPOCH_PASSED: i32 = 130;
/// Every holder slot on the shared value is held by a live process.
pub const SUBETHA_E_ARC_HOLDERS_EXHAUSTED: i32 = 131;
// Code 132 is out of use, and no new code takes it.
/// The lazy value is claimed by a live process and not yet published.
pub const SUBETHA_E_LAZY_NOT_CLAIMED: i32 = 133;
/// A newer version than the one the write was made against landed first,
/// so the write did not. On a versioned index the key's newest version was
/// born after the statement's pin by another epoch: the statement takes
/// back what it wrote, publishes, and runs again under a fresh pin. On a
/// cell another write moved the generation the write named.
pub const SUBETHA_E_WRITE_CONFLICT: i32 = 134;
/// The next item on the ring is a plain item where a frame was asked for;
/// it stays on the ring for a receive that takes it.
pub const SUBETHA_E_NOT_A_FRAME: i32 = 135;

/// The name of a code, as a static NUL-terminated string. Safe from any
/// thread at any time; an unknown code names itself as unknown.
#[unsafe(no_mangle)]
pub extern "C" fn subetha_strerror(code: i32) -> *const c_char {
    code_name(code).as_ptr()
}

/// The name [`subetha_strerror`] gives a code, which is also the detail of
/// a failing call that wrote none of its own.
fn code_name(code: i32) -> &'static CStr {
    match code {
        SUBETHA_OK => c"ok",
        SUBETHA_E_NOT_INITIALIZED => c"subetha_init has not been called",
        SUBETHA_E_INIT_CONFLICT => c"subetha_init was called again with a different default mode",
        SUBETHA_E_INVALID_ARGUMENT => c"an argument is null, zero, or out of range",
        SUBETHA_E_INVALID_HANDLE => c"the handle names no live object",
        SUBETHA_E_HANDLE_POISONED => c"an earlier panic left this handle's object unusable",
        SUBETHA_E_PANIC => c"the call panicked",
        SUBETHA_E_WRONG_KIND => c"the handle names an object of another kind",
        SUBETHA_E_OUT_OF_MEMORY => c"an allocation failed",
        SUBETHA_E_BUFFER_TOO_SMALL => c"the caller's buffer is too small",
        SUBETHA_E_INVALID_UTF8 => c"a string argument is not valid UTF-8",
        SUBETHA_E_TIMEOUT => c"the timeout elapsed",
        SUBETHA_E_SHUT_DOWN => c"the library has been shut down",
        SUBETHA_E_HANDLES_WERE_LIVE => c"shutdown closed handles that were still live",
        SUBETHA_E_NOT_SUPPORTED => c"not supported on this platform or in this mode",
        SUBETHA_E_DESTROYED => c"the object was destroyed while this call waited on it",
        SUBETHA_E_RING_FULL => c"the ring is full",
        SUBETHA_E_RING_EMPTY => c"the ring is empty",
        SUBETHA_E_RING_LAYOUT_MISMATCH => c"a backing exists with a different layout",
        SUBETHA_E_RING_PAYLOAD_TOO_LARGE => c"the payload exceeds one slot",
        SUBETHA_E_RING_NOT_STAMPED => c"the ring carries no ordering stamps",
        SUBETHA_E_RING_NOT_DRAINER => c"another consumer holds the drainer lease",
        SUBETHA_E_RING_STALE_BACKLOG => c"the previous shape still holds a backlog",
        SUBETHA_E_RING_IO => c"the OS refused an open, map or remove",
        SUBETHA_E_RING_TOO_MANY_PRODUCERS => c"producer registration refused",
        SUBETHA_E_RING_TOO_MANY_CONSUMERS => c"consumer registration refused",
        SUBETHA_E_RING_GROWTH_FAILED => c"the grown backing could not be created",
        SUBETHA_E_RING_WAKER_FULL => c"every waiter slot is in use",
        SUBETHA_E_RING_WAKER_LAYOUT => c"a waker region exists with a different layout",
        SUBETHA_E_BROADCAST_NO_CONSUMER_SLOT => c"every consumer slot of the broadcast ring is taken",
        SUBETHA_E_BROADCAST_INVALID_CONSUMER => c"the consumer index is out of range or not registered",
        SUBETHA_E_PUBSUB_PENDING => c"the position has not been published yet",
        SUBETHA_E_PUBSUB_LOST => c"the position was overwritten before it was read",
        SUBETHA_E_DEQUE_NOT_OWNER => c"the deque handle is a thief's; push and pop belong to the owner",
        SUBETHA_E_MAP_FULL => c"the map has no free slot for the key",
        SUBETHA_E_MAP_KEY_ABSENT => c"the key has no entry in the map",
        SUBETHA_E_ARENA_FULL => c"the arena has no room for the whole value",
        SUBETHA_E_ARENA_INVALID_REF => c"the reference reaches past what the arena holds",
        SUBETHA_E_READ_ONLY => c"the handle was opened read-only",
        SUBETHA_E_OUT_OF_BOUNDS => c"the index is at or past the length or the capacity",
        SUBETHA_E_EPOCHS_PINS_EXHAUSTED => c"every pin slot is held by a live process",
        SUBETHA_E_EPOCHS_TICKETS_EXHAUSTED => c"every ticket slot is held by a live process",
        SUBETHA_E_FENCE_CLOCK_FULL => c"every slot in the fence clock is registered",
        SUBETHA_E_BARRIER_NO_LIVE_PEERS => c"the barrier has no live peer to wait for",
        SUBETHA_E_BARRIER_EPOCH_PASSED => c"the barrier has already passed the epoch waited on",
        SUBETHA_E_ARC_HOLDERS_EXHAUSTED => c"every holder slot on the shared value is held",
        SUBETHA_E_LAZY_NOT_CLAIMED => c"the caller does not hold the claim on the lazy value",
        SUBETHA_E_WRITE_CONFLICT => c"a newer version than the one the write was made against landed first",
        SUBETHA_E_NOT_A_FRAME => c"the next item is a plain item, not a frame",
        SUBETHA_E_WOULD_BLOCK => c"something the call needs is held by another participant or not free yet",
        _ => c"unknown subetha code",
    }
}

/// The text of a call's failure: a code's own name, which a failing call
/// stores without allocating, or the specifics an arm formatted.
enum DetailText {
    Name(&'static CStr),
    Owned(String),
}

/// No code: nothing written during the current call, or a detail of
/// formatted specifics rather than a code's name.
const CODE_NONE: i32 = i32::MIN;

/// The code of specifics whose writer did not name one, which stand for
/// whatever code the call returns.
const CODE_ANY: i32 = i32::MIN + 1;

thread_local! {
    static DETAIL: RefCell<DetailText> = const { RefCell::new(DetailText::Name(c"")) };
    /// The code the detail was written for during the current call, and
    /// the code whose own name the detail holds. One copyable cell, so a
    /// polling caller's repeated refusal reads it once and writes nothing.
    static CODES: Cell<(i32, i32)> = const { Cell::new((CODE_NONE, CODE_NONE)) };
    /// The paths the current call was given, quoted and in order, which a
    /// failing call names ahead of its detail.
    static OPENED: RefCell<String> = const { RefCell::new(String::new()) };
    /// Whether `OPENED` holds anything, so a call that opened nothing
    /// reads one cell.
    static HAS_OPENED: Cell<bool> = const { Cell::new(false) };
}

/// Record a path the current call was given, so its failure names it.
pub(crate) fn opened(path: &str) {
    OPENED.with(|o| {
        let mut o = o.borrow_mut();
        if !o.is_empty() {
            o.push_str(", ");
        }
        o.push('\'');
        o.push_str(path);
        o.push('\'');
    });
    HAS_OPENED.set(true);
}

/// Drop the paths an earlier call recorded, at the start of a call.
pub(crate) fn forget_opened() {
    if HAS_OPENED.get() {
        HAS_OPENED.set(false);
        OPENED.with(|o| o.borrow_mut().clear());
    }
}

/// Record the specifics of the failure the current call is about to report.
pub(crate) fn set_detail(text: impl Into<String>) {
    DETAIL.with(|d| *d.borrow_mut() = DetailText::Owned(text.into()));
    CODES.set((CODE_ANY, CODE_NONE));
}

/// Record a failure's specifics and return its code, so a failing arm reads
/// as one expression.
pub(crate) fn fail(code: i32, text: impl Into<String>) -> i32 {
    DETAIL.with(|d| *d.borrow_mut() = DetailText::Owned(text.into()));
    CODES.set((code, CODE_NONE));
    code
}

/// Finish an entry point's call: a failing code whose call wrote no detail
/// for it gets the code's own name, so the detail always describes this
/// call's failure and never an earlier one's. A successful call is passed
/// through untouched, and a refusal whose name the detail already holds
/// writes nothing.
#[inline]
pub(crate) fn finish(code: i32) -> i32 {
    if code != SUBETHA_OK {
        let (written_for, named) = CODES.get();
        if written_for == code || written_for == CODE_ANY {
            CODES.set((CODE_NONE, named));
        } else if named != code {
            DETAIL.with(|d| *d.borrow_mut() = DetailText::Name(code_name(code)));
            CODES.set((CODE_NONE, code));
        } else if written_for != CODE_NONE {
            CODES.set((CODE_NONE, named));
        }
    }
    if HAS_OPENED.get() {
        HAS_OPENED.set(false);
        let paths = OPENED.with(|o| std::mem::take(&mut *o.borrow_mut()));
        if code != SUBETHA_OK {
            DETAIL.with(|d| {
                let mut d = d.borrow_mut();
                let text = match &*d {
                    DetailText::Name(name) => name.to_string_lossy().into_owned(),
                    DetailText::Owned(text) => text.clone(),
                };
                *d = DetailText::Owned(format!("{paths}: {text}"));
            });
            // The detail now holds more than a code's name, so the next
            // refusal writes its own.
            CODES.set((CODE_NONE, CODE_NONE));
        }
    }
    code
}

/// Copy the detail of the last failing call on this thread into `buf`,
/// NUL-terminated, and return the number of bytes the whole detail needs
/// including the terminator. A return larger than `cap` means the copy was
/// cut short; call again with a buffer of the returned size. With `buf` null
/// or `cap` zero nothing is copied and only the size is returned. The detail
/// describes the most recent failure and is meaningful only right after a
/// call returned a code other than SUBETHA_OK.
///
/// # Safety
/// `buf` is null or points to `cap` writable bytes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn subetha_last_error_detail(buf: *mut c_char, cap: usize) -> usize {
    DETAIL.with(|d| {
        let d = d.borrow();
        let bytes = match &*d {
            DetailText::Name(name) => name.to_bytes(),
            DetailText::Owned(text) => text.as_bytes(),
        };
        let needed = bytes.len() + 1;
        if buf.is_null() || cap == 0 {
            return needed;
        }
        let n = bytes.len().min(cap - 1);
        // SAFETY: the caller guarantees `cap` writable bytes at `buf`, and
        // `n + 1 <= cap`.
        unsafe {
            std::ptr::copy_nonoverlapping(bytes.as_ptr(), buf.cast::<u8>(), n);
            *buf.add(n) = 0;
        }
        needed
    })
}

/// The map from a ring's own error to the code that names it.
pub(crate) fn ring_code(e: subetha_cxc::shared_ring::RingError) -> i32 {
    use subetha_cxc::shared_ring::RingError;
    match e {
        RingError::Full => SUBETHA_E_RING_FULL,
        RingError::Empty => SUBETHA_E_RING_EMPTY,
        e @ (RingError::LayoutMismatch(_) | RingError::UnknownStampKind(_) | RingError::Stamped) => {
            fail(SUBETHA_E_RING_LAYOUT_MISMATCH, e.to_string())
        }
        RingError::PayloadTooLarge => SUBETHA_E_RING_PAYLOAD_TOO_LARGE,
        RingError::NotStamped => SUBETHA_E_RING_NOT_STAMPED,
        RingError::InvalidArgument(what) => fail(SUBETHA_E_INVALID_ARGUMENT, what),
        RingError::NotDrainer => SUBETHA_E_RING_NOT_DRAINER,
        RingError::StaleBacklog => SUBETHA_E_RING_STALE_BACKLOG,
        RingError::NotAFrame => SUBETHA_E_NOT_A_FRAME,
        RingError::IoError(e) => fail(SUBETHA_E_RING_IO, e.to_string()),
    }
}

/// The map from a notifier's error to the code that names it.
pub(crate) fn notify_code(e: subetha_cxc::cross_process_notifier::NotifyError) -> i32 {
    use subetha_cxc::cross_process_notifier::NotifyError;
    match e {
        NotifyError::Io(e) => fail(SUBETHA_E_RING_IO, format!("notifier: {e}")),
        e @ NotifyError::LayoutMismatch(_) => fail(SUBETHA_E_RING_LAYOUT_MISMATCH, e.to_string()),
    }
}

/// The map from a refused hold on a ring's backings to the code that
/// names it.
///
/// A locale with no files to remove and a region built for another
/// holder count are both the caller's argument being wrong about the
/// ring in front of it, which is what the invalid-argument code says;
/// an exhausted table is the ring's own capacity being reached, and
/// reads like a full ring rather than a bad call.
pub(crate) fn last_holder_code(e: subetha_cxc::adaptive_ring::LastHolderError) -> i32 {
    use subetha_cxc::adaptive_ring::LastHolderError;
    use subetha_cxc::ring_holders::HoldersError;
    match e {
        LastHolderError::NoBackingFiles => fail(
            SUBETHA_E_INVALID_ARGUMENT,
            "an anonymous ring has no backing files or names for a last holder to remove",
        ),
        LastHolderError::Region(HoldersError::Exhausted) => fail(
            SUBETHA_E_RING_FULL,
            "every holder slot on this ring is held by a live process",
        ),
        LastHolderError::Region(e @ HoldersError::LayoutMismatch(_)) => {
            fail(SUBETHA_E_RING_LAYOUT_MISMATCH, e.to_string())
        }
        LastHolderError::Region(HoldersError::IoError(e)) => {
            fail(SUBETHA_E_RING_IO, format!("holders region: {e}"))
        }
    }
}

/// The map from a shared hash map's error to the code that names it.
pub(crate) fn map_code(e: subetha_cxc::shared_hash_map::MapError) -> i32 {
    use subetha_cxc::shared_hash_map::MapError;
    match e {
        MapError::Full => SUBETHA_E_MAP_FULL,
        MapError::PayloadTooLarge => fail(SUBETHA_E_INVALID_ARGUMENT, "a key or value is not its declared size, or the sizes exceed one entry"),
        e @ MapError::LayoutMismatch(_) => fail(SUBETHA_E_RING_LAYOUT_MISMATCH, e.to_string()),
        MapError::KeyAbsent => SUBETHA_E_MAP_KEY_ABSENT,
        MapError::IoError(e) => fail(SUBETHA_E_RING_IO, e.to_string()),
    }
}

/// The map from a handle table's error to the code that names it.
pub(crate) fn handle_table_code(e: subetha_cxc::shared_handle_table::HandleTableError) -> i32 {
    use subetha_cxc::shared_handle_table::HandleTableError;
    match e {
        HandleTableError::Full => fail(SUBETHA_E_MAP_FULL, "every slot in the table holds a live value"),
        HandleTableError::PayloadTooLarge => fail(SUBETHA_E_INVALID_ARGUMENT, "the value type is larger than a slot"),
        e @ HandleTableError::LayoutMismatch(_) => fail(SUBETHA_E_RING_LAYOUT_MISMATCH, e.to_string()),
        HandleTableError::IoError(e) => fail(SUBETHA_E_RING_IO, e.to_string()),
    }
}

/// The map from a string arena's error to the code that names it.
pub(crate) fn arena_code(e: subetha_cxc::shared_string_arena::ArenaError) -> i32 {
    use subetha_cxc::shared_string_arena::ArenaError;
    match e {
        ArenaError::Full => SUBETHA_E_ARENA_FULL,
        ArenaError::InvalidRef => SUBETHA_E_ARENA_INVALID_REF,
        ArenaError::InvalidUtf8 => fail(SUBETHA_E_INVALID_UTF8, "the value is not UTF-8"),
        e @ ArenaError::LayoutMismatch(_) => fail(SUBETHA_E_RING_LAYOUT_MISMATCH, e.to_string()),
        ArenaError::InvalidArgument(what) => fail(SUBETHA_E_INVALID_ARGUMENT, what),
        ArenaError::ReadOnly => fail(SUBETHA_E_READ_ONLY, "the arena was opened read-only"),
        ArenaError::IoError(e) => fail(SUBETHA_E_RING_IO, e.to_string()),
    }
}

/// The map from a holder table's error to the code that names it.
pub(crate) fn holders_code(e: subetha_cxc::shared_holder_table::SharedHolderError) -> i32 {
    use subetha_cxc::shared_holder_table::SharedHolderError;
    match e {
        e @ SharedHolderError::LayoutMismatch(_) => fail(SUBETHA_E_RING_LAYOUT_MISMATCH, e.to_string()),
        SharedHolderError::EmptyCapacity => fail(SUBETHA_E_INVALID_ARGUMENT, "a capacity of zero has no slot to claim"),
        SharedHolderError::IoError(e) => fail(SUBETHA_E_RING_IO, e.to_string()),
    }
}

/// The map from a heartbeat table's error to the code that names it.
pub(crate) fn heartbeat_code(e: subetha_cxc::heartbeat::HeartbeatError) -> i32 {
    use subetha_cxc::heartbeat::HeartbeatError;
    match e {
        HeartbeatError::TableFull => fail(SUBETHA_E_RING_FULL, "every slot in the table is held"),
        e @ HeartbeatError::LayoutMismatch(_) => fail(SUBETHA_E_RING_LAYOUT_MISMATCH, e.to_string()),
        HeartbeatError::IoError(e) => fail(SUBETHA_E_RING_IO, e.to_string()),
    }
}

/// The map from a leader election's error to the code that names it.
pub(crate) fn leader_code(e: subetha_cxc::shared_leader_election::LeaderError) -> i32 {
    use subetha_cxc::shared_leader_election::LeaderError;
    match e {
        e @ LeaderError::LayoutMismatch(_) => fail(SUBETHA_E_RING_LAYOUT_MISMATCH, e.to_string()),
        LeaderError::IoError(e) => fail(SUBETHA_E_RING_IO, e.to_string()),
    }
}

/// The map from a condition variable's error to the code that names it.
pub(crate) fn condvar_code(e: subetha_cxc::shared_condvar::CondvarError) -> i32 {
    use subetha_cxc::shared_condvar::CondvarError;
    match e {
        CondvarError::WakerFull => SUBETHA_E_RING_WAKER_FULL,
        CondvarError::Timeout => SUBETHA_E_TIMEOUT,
        e @ CondvarError::LayoutMismatch(_) => fail(SUBETHA_E_RING_LAYOUT_MISMATCH, e.to_string()),
        CondvarError::Io(e) => fail(SUBETHA_E_RING_IO, e.to_string()),
    }
}

/// The map from a fence clock's error to the code that names it.
pub(crate) fn fence_clock_code(e: subetha_cxc::shared_fence_clock::FenceClockError) -> i32 {
    use subetha_cxc::shared_fence_clock::FenceClockError;
    match e {
        FenceClockError::Full => SUBETHA_E_FENCE_CLOCK_FULL,
        e @ FenceClockError::LayoutMismatch(_) => fail(SUBETHA_E_RING_LAYOUT_MISMATCH, e.to_string()),
        FenceClockError::InvalidSlot => fail(SUBETHA_E_INVALID_ARGUMENT, "the slot is not one this clock holds"),
        FenceClockError::IoError(e) => fail(SUBETHA_E_RING_IO, e.to_string()),
    }
}

/// The map from an epoch table's error to the code that names it.
pub(crate) fn epochs_code(e: subetha_cxc::shared_epochs::EpochError) -> i32 {
    use subetha_cxc::shared_epochs::EpochError;
    match e {
        EpochError::PinsExhausted => SUBETHA_E_EPOCHS_PINS_EXHAUSTED,
        EpochError::TicketsExhausted => SUBETHA_E_EPOCHS_TICKETS_EXHAUSTED,
        e @ EpochError::LayoutMismatch(_) => fail(SUBETHA_E_RING_LAYOUT_MISMATCH, e.to_string()),
        EpochError::IoError(e) => fail(SUBETHA_E_RING_IO, e.to_string()),
    }
}

/// The map from a shared cell's error to the code that names it.
pub(crate) fn cell_code(e: subetha_cxc::shared_cell::SharedCellError) -> i32 {
    use subetha_cxc::shared_cell::SharedCellError;
    match e {
        e @ SharedCellError::LayoutMismatch(_) => fail(SUBETHA_E_RING_LAYOUT_MISMATCH, e.to_string()),
        SharedCellError::PayloadTooLarge => SUBETHA_E_RING_PAYLOAD_TOO_LARGE,
        SharedCellError::NotInitialized => fail(SUBETHA_E_INVALID_HANDLE, "the cell holds no value yet"),
        SharedCellError::IoError(e) => fail(SUBETHA_E_RING_IO, e.to_string()),
    }
}

/// The map from a shared B-tree map's error to the code that names it.
pub(crate) fn btree_code(e: subetha_cxc::shared_btree_map::BTreeError) -> i32 {
    use subetha_cxc::shared_btree_map::BTreeError;
    match e {
        BTreeError::Full => fail(SUBETHA_E_RING_FULL, "the map has no node for the split this insert needs"),
        e @ BTreeError::LayoutMismatch(_) => fail(SUBETHA_E_RING_LAYOUT_MISMATCH, e.to_string()),
        BTreeError::InvalidConfig => fail(SUBETHA_E_INVALID_ARGUMENT, "a key or value is not its declared size, or the capacity is zero"),
        BTreeError::IoError(e) => fail(SUBETHA_E_RING_IO, e.to_string()),
    }
}

/// The map from a shared linked list's error to the code that names it.
pub(crate) fn list_code(e: subetha_cxc::shared_linked_list::LinkedListError) -> i32 {
    use subetha_cxc::shared_linked_list::LinkedListError;
    match e {
        LinkedListError::Region(r) => region_code(r),
        LinkedListError::InvalidHandle => fail(SUBETHA_E_OUT_OF_BOUNDS, "the index names no node of this list"),
        e @ LinkedListError::LayoutMismatch(_) => fail(SUBETHA_E_RING_LAYOUT_MISMATCH, e.to_string()),
        LinkedListError::IoError(e) => fail(SUBETHA_E_RING_IO, e.to_string()),
    }
}

/// The map from a shared atomic's error to the code that names it.
pub(crate) fn atomic_code(e: subetha_cxc::shared_atomic::SharedAtomicError) -> i32 {
    use subetha_cxc::shared_atomic::SharedAtomicError;
    match e {
        e @ SharedAtomicError::LayoutMismatch(_) => fail(SUBETHA_E_RING_LAYOUT_MISMATCH, e.to_string()),
        SharedAtomicError::IoError(e) => fail(SUBETHA_E_RING_IO, e.to_string()),
    }
}

/// The map from a once cell's error to the code that names it.
pub(crate) fn once_code(e: subetha_cxc::shared_once_cell::SharedOnceError) -> i32 {
    use subetha_cxc::shared_once_cell::SharedOnceError;
    match e {
        e @ SharedOnceError::LayoutMismatch(_) => fail(SUBETHA_E_RING_LAYOUT_MISMATCH, e.to_string()),
        SharedOnceError::PayloadTooLarge => fail(
            SUBETHA_E_INVALID_ARGUMENT,
            "value_bytes is zero or past what a lazy value holds",
        ),
        SharedOnceError::IoError(e) => fail(SUBETHA_E_RING_IO, e.to_string()),
    }
}

/// The map from a shared value's error to the code that names it.
pub(crate) fn arc_code(e: subetha_cxc::shared_arc::ArcError) -> i32 {
    use subetha_cxc::shared_arc::ArcError;
    match e {
        ArcError::HoldersExhausted => SUBETHA_E_ARC_HOLDERS_EXHAUSTED,
        e @ ArcError::LayoutMismatch(_) => fail(SUBETHA_E_RING_LAYOUT_MISMATCH, e.to_string()),
        e @ ArcError::OutOfRange { .. } => fail(SUBETHA_E_OUT_OF_BOUNDS, e.to_string()),
        ArcError::IoError(e) => fail(SUBETHA_E_RING_IO, e.to_string()),
    }
}

/// The map from an epoch barrier's error to the code that names it.
pub(crate) fn barrier_code(e: subetha_cxc::epoch_barrier::BarrierError) -> i32 {
    use subetha_cxc::epoch_barrier::BarrierError;
    match e {
        BarrierError::Atomic(inner) => atomic_code(inner),
        BarrierError::EpochTooFarBehind => SUBETHA_E_BARRIER_EPOCH_PASSED,
        BarrierError::Timeout => SUBETHA_E_TIMEOUT,
        BarrierError::NoLivePeers => SUBETHA_E_BARRIER_NO_LIVE_PEERS,
    }
}

/// The map from a shared region's error to the code that names it.
pub(crate) fn region_code(e: subetha_cxc::shared_region::RegionError) -> i32 {
    use subetha_cxc::shared_region::RegionError;
    match e {
        RegionError::Full => SUBETHA_E_RING_FULL,
        RegionError::InvalidPtr => SUBETHA_E_OUT_OF_BOUNDS,
        RegionError::PayloadTooLarge => fail(SUBETHA_E_INVALID_ARGUMENT, "the bytes are not the element size"),
        e @ RegionError::LayoutMismatch(_) => fail(SUBETHA_E_RING_LAYOUT_MISMATCH, e.to_string()),
        RegionError::InvalidArgument(what) => fail(SUBETHA_E_INVALID_ARGUMENT, what),
        RegionError::IoError(e) => fail(SUBETHA_E_RING_IO, e.to_string()),
    }
}

/// The map from a shared vec's error to the code that names it.
pub(crate) fn vec_code(e: subetha_cxc::shared_vec::VecError) -> i32 {
    use subetha_cxc::shared_vec::VecError;
    match e {
        VecError::Full => SUBETHA_E_RING_FULL,
        VecError::OutOfBounds => SUBETHA_E_OUT_OF_BOUNDS,
        e @ VecError::LayoutMismatch(_) => fail(SUBETHA_E_RING_LAYOUT_MISMATCH, e.to_string()),
        VecError::InvalidArgument(what) => fail(SUBETHA_E_INVALID_ARGUMENT, what),
        VecError::PayloadTooLarge => fail(SUBETHA_E_INVALID_ARGUMENT, "the bytes are not the element size"),
        VecError::ReadOnly => fail(SUBETHA_E_READ_ONLY, "the vec was opened read-only"),
        VecError::IoError(e) => fail(SUBETHA_E_RING_IO, e.to_string()),
    }
}

/// The map from a shared slab's error to the code that names it.
pub(crate) fn slab_code(e: subetha_cxc::shared_slab::SlabError) -> i32 {
    use subetha_cxc::shared_slab::SlabError;
    match e {
        SlabError::OutOfBounds => SUBETHA_E_OUT_OF_BOUNDS,
        e @ SlabError::LayoutMismatch(_) => fail(SUBETHA_E_RING_LAYOUT_MISMATCH, e.to_string()),
        SlabError::InvalidArgument(what) => fail(SUBETHA_E_INVALID_ARGUMENT, what),
        SlabError::ReadOnly => fail(SUBETHA_E_READ_ONLY, "the slab was opened read-only"),
        SlabError::PayloadTooLarge => fail(SUBETHA_E_INVALID_ARGUMENT, "the bytes are not the record size"),
        SlabError::IoError(e) => fail(SUBETHA_E_RING_IO, e.to_string()),
    }
}

/// The map from a Treiber stack's error to the code that names it.
pub(crate) fn stack_code(e: subetha_cxc::shared_treiber_stack::StackError) -> i32 {
    use subetha_cxc::shared_treiber_stack::StackError;
    match e {
        StackError::Full => SUBETHA_E_RING_FULL,
        e @ StackError::LayoutMismatch(_) => fail(SUBETHA_E_RING_LAYOUT_MISMATCH, e.to_string()),
        e @ StackError::PayloadTooLarge => fail(SUBETHA_E_RING_PAYLOAD_TOO_LARGE, e.to_string()),
        StackError::IoError(e) => fail(SUBETHA_E_RING_IO, e.to_string()),
    }
}

/// The map from a work-stealing deque's error to the code that names it.
pub(crate) fn deque_code(e: subetha_cxc::shared_deque::DequeError) -> i32 {
    use subetha_cxc::shared_deque::DequeError;
    match e {
        DequeError::Io(e) => fail(SUBETHA_E_RING_IO, e.to_string()),
        DequeError::InvalidCapacity => fail(SUBETHA_E_INVALID_ARGUMENT, "the capacity is not a power of two of at least 1, or the layout is empty or misaligned"),
        e @ DequeError::LayoutMismatch(_) => fail(SUBETHA_E_RING_LAYOUT_MISMATCH, e.to_string()),
        DequeError::Full => SUBETHA_E_RING_FULL,
        DequeError::Marshal(e) => fail(SUBETHA_E_RING_PAYLOAD_TOO_LARGE, format!("{e:?}")),
    }
}

/// The map from a registration error to the code that names it.
pub(crate) fn adaptive_code(e: subetha_cxc::adaptive_ring::AdaptiveError) -> i32 {
    use subetha_cxc::adaptive_ring::AdaptiveError;
    match e {
        AdaptiveError::TooManyProducers => SUBETHA_E_RING_TOO_MANY_PRODUCERS,
        AdaptiveError::TooManyConsumers => SUBETHA_E_RING_TOO_MANY_CONSUMERS,
        e @ AdaptiveError::GrowthFailed(_) => fail(SUBETHA_E_RING_GROWTH_FAILED, e.to_string()),
    }
}

/// The map from a broadcast ring's error to the code that names it.
pub(crate) fn broadcast_code(e: subetha_cxc::shared_broadcast_ring::BroadcastError) -> i32 {
    use subetha_cxc::shared_broadcast_ring::BroadcastError;
    match e {
        BroadcastError::Full => SUBETHA_E_RING_FULL,
        BroadcastError::Empty => SUBETHA_E_RING_EMPTY,
        BroadcastError::NoConsumerSlot => SUBETHA_E_BROADCAST_NO_CONSUMER_SLOT,
        BroadcastError::InvalidConsumer => SUBETHA_E_BROADCAST_INVALID_CONSUMER,
        BroadcastError::PayloadTooLarge => SUBETHA_E_RING_PAYLOAD_TOO_LARGE,
        e @ BroadcastError::LayoutMismatch(_) => fail(SUBETHA_E_RING_LAYOUT_MISMATCH, e.to_string()),
        BroadcastError::IoError(e) => fail(SUBETHA_E_RING_IO, e.to_string()),
    }
}

/// The map from an I/O error a ring constructor reports to the code that
/// names it: invalid data is a layout that does not match, anything else
/// the OS's refusal.
pub(crate) fn io_code(e: std::io::Error) -> i32 {
    if e.kind() == std::io::ErrorKind::InvalidData {
        fail(SUBETHA_E_RING_LAYOUT_MISMATCH, e.to_string())
    } else {
        fail(SUBETHA_E_RING_IO, e.to_string())
    }
}

/// The map from a capacity morph's error to the code that names it.
pub(crate) fn capacity_code(e: subetha_cxc::capacity_adaptive_ring::CapacityMorphError) -> i32 {
    use subetha_cxc::capacity_adaptive_ring::CapacityMorphError;
    match e {
        CapacityMorphError::InvalidCapacity => fail(SUBETHA_E_INVALID_ARGUMENT, "the capacity is not a power of two of at least 2"),
        CapacityMorphError::CannotShrinkInFlight { in_flight, new_capacity } => fail(
            SUBETHA_E_RING_STALE_BACKLOG,
            format!("{in_flight} items in flight exceed the new capacity {new_capacity}"),
        ),
        CapacityMorphError::Ring(e) => ring_code(e),
        CapacityMorphError::Adaptive(e) => adaptive_code(e),
    }
}

/// The map from a broadcast capacity morph's error to the code that names it.
pub(crate) fn broadcast_capacity_code(e: subetha_cxc::capacity_broadcast_ring::BroadcastCapacityMorphError) -> i32 {
    use subetha_cxc::capacity_broadcast_ring::BroadcastCapacityMorphError;
    match e {
        BroadcastCapacityMorphError::InvalidCapacity => fail(SUBETHA_E_INVALID_ARGUMENT, "the capacity is not a power of two of at least 2"),
        BroadcastCapacityMorphError::Broadcast(e) => broadcast_code(e),
        BroadcastCapacityMorphError::Io(e) => io_code(e),
    }
}

/// The map from a pub/sub capacity morph's error to the code that names it.
pub(crate) fn pubsub_capacity_code(e: subetha_cxc::capacity_pubsub_ring::PubSubCapacityMorphError) -> i32 {
    use subetha_cxc::capacity_pubsub_ring::PubSubCapacityMorphError;
    match e {
        PubSubCapacityMorphError::InvalidCapacity => fail(SUBETHA_E_INVALID_ARGUMENT, "the capacity is not a power of two of at least 2"),
        PubSubCapacityMorphError::Io(e) => io_code(e),
    }
}

/// The map from a blocking ring's error to the code that names it.
pub(crate) fn blocking_code(e: subetha_cxc::blocking_spsc_ring::BlockingError) -> i32 {
    use subetha_cxc::blocking_spsc_ring::BlockingError;
    match e {
        BlockingError::Ring(e) => ring_code(e),
        BlockingError::WakerFull => SUBETHA_E_RING_WAKER_FULL,
        BlockingError::Timeout => SUBETHA_E_TIMEOUT,
        e @ BlockingError::WakerLayout(_) => fail(SUBETHA_E_RING_WAKER_LAYOUT, e.to_string()),
        BlockingError::Io(e) => fail(SUBETHA_E_RING_IO, e.to_string()),
        BlockingError::Notify(e) => notify_code(e),
        BlockingError::Closed => fail(SUBETHA_E_DESTROYED, "the object is being destroyed"),
    }
}

/// The map from a front-door error to the code that names it.
pub(crate) fn api_code(e: subetha_cxc::api::ApiError) -> i32 {
    use subetha_cxc::api::ApiError;
    use subetha_cxc::message_transport::TransportError;
    match e {
        ApiError::Transport(TransportError::Full) => SUBETHA_E_RING_FULL,
        ApiError::Transport(TransportError::Empty) => SUBETHA_E_RING_EMPTY,
        e @ (ApiError::Transport(TransportError::PayloadTooLarge) | ApiError::PayloadTooLarge) => {
            fail(SUBETHA_E_RING_PAYLOAD_TOO_LARGE, e.to_string())
        }
        e @ ApiError::Transport(TransportError::OutBufferTooSmall) => fail(SUBETHA_E_BUFFER_TOO_SMALL, e.to_string()),
        e @ ApiError::Transport(TransportError::Other) => fail(SUBETHA_E_IO, e.to_string()),
        e @ (ApiError::Marshal(_) | ApiError::WrongFamily { .. }) => fail(SUBETHA_E_LAYOUT_MISMATCH, e.to_string()),
        ApiError::Io(e) => io_code(e),
        ApiError::Map(e) => map_code(e),
        ApiError::Timeout => SUBETHA_E_TIMEOUT,
    }
}

/// The map from a waker error to the code that names it.
pub(crate) fn waker_code(e: subetha_cxc::cross_process_waker::WakerError) -> i32 {
    use subetha_cxc::cross_process_waker::WakerError;
    match e {
        WakerError::Full => SUBETHA_E_RING_WAKER_FULL,
        WakerError::Timeout => SUBETHA_E_TIMEOUT,
        e @ WakerError::LayoutMismatch(_) => fail(SUBETHA_E_RING_WAKER_LAYOUT, e.to_string()),
        WakerError::IoError(e) => fail(SUBETHA_E_RING_IO, format!("waker: {e}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The text `subetha_last_error_detail` hands back on this thread now.
    fn detail() -> String {
        let mut buf = [0u8; 512];
        // SAFETY: `buf` has `buf.len()` writable bytes.
        let needed = unsafe { subetha_last_error_detail(buf.as_mut_ptr().cast(), buf.len()) };
        String::from_utf8(buf[..needed.min(buf.len()) - 1].to_vec()).expect("the detail is UTF-8")
    }

    fn name(code: i32) -> &'static str {
        code_name(code).to_str().expect("every code name is UTF-8")
    }

    /// Each failing call leaves a detail of its own: a handle used after it
    /// was destroyed, a map opened at another capacity, a ring with nothing
    /// in it, a key the map does not hold. None reads as the call before.
    #[test]
    fn every_failing_call_leaves_a_detail_of_its_own() {
        use crate::hashmap::{
            subetha_hashmap_create, subetha_hashmap_get, subetha_hashmap_insert, subetha_hashmap_open,
        };
        use crate::ring::{
            subetha_ring_create_anon, subetha_ring_options, subetha_ring_register_consumer,
            subetha_ring_try_pop,
        };
        use crate::runtime::{subetha_handle_destroy, subetha_init};

        assert_eq!(subetha_init(crate::runtime::SUBETHA_MODE_STRICT), SUBETHA_OK);
        let path = std::env::temp_dir().join(format!("subetha-ffi-detail-{}.bin", std::process::id()));
        let c_path = std::ffi::CString::new(path.to_str().expect("a UTF-8 temp path")).expect("no NUL in the path");
        let mut map = 0;
        // SAFETY: a valid path and out pointer.
        let rc = unsafe { subetha_hashmap_create(c_path.as_ptr(), 1024, 4, 8, 2, &mut map) };
        assert_eq!(rc, SUBETHA_OK);
        assert_eq!(subetha_handle_destroy(map), SUBETHA_OK);
        let (key, value) = (7u32.to_ne_bytes(), 9u64.to_ne_bytes());
        // SAFETY: the key and value are their declared sizes.
        let rc = unsafe {
            subetha_hashmap_insert(map, key.as_ptr(), key.len(), value.as_ptr(), value.len(), std::ptr::null_mut())
        };
        assert_eq!(rc, SUBETHA_E_INVALID_HANDLE);
        let destroyed = detail();
        assert!(destroyed.contains("destroyed"), "{destroyed}");

        let mut reopened = 0;
        // SAFETY: a valid path and out pointer.
        let rc = unsafe { subetha_hashmap_open(c_path.as_ptr(), 512, 4, 8, 2, &mut reopened) };
        assert_eq!(rc, SUBETHA_E_RING_LAYOUT_MISMATCH);
        assert_eq!(
            detail(),
            format!(
                "'{}': a map of another shape: the file's capacity is 1024 where 512 was asked for",
                path.display()
            ),
            "the open's own detail, naming the path, what the file holds and what was asked for",
        );

        let options = subetha_ring_options { max_waiters: 0, scan_interval_us: 0, ..Default::default() };
        let (mut ring, mut consumer, mut len) = (0, 0u32, 0usize);
        // SAFETY: valid options and out pointers.
        assert_eq!(unsafe { subetha_ring_create_anon(1, 1, 8, &options, &mut ring) }, SUBETHA_OK);
        // SAFETY: a valid out pointer.
        assert_eq!(unsafe { subetha_ring_register_consumer(ring, &mut consumer) }, SUBETHA_OK);
        let mut slot = [0u8; 64];
        // SAFETY: `slot` has `slot.len()` writable bytes.
        let rc = unsafe { subetha_ring_try_pop(ring, consumer, slot.as_mut_ptr(), slot.len(), &mut len) };
        assert_eq!(rc, SUBETHA_E_RING_EMPTY);
        assert_eq!(detail(), name(SUBETHA_E_RING_EMPTY), "the pop's own detail, not the open's");

        let mut live = 0;
        // SAFETY: a valid path and out pointer.
        assert_eq!(unsafe { subetha_hashmap_open(c_path.as_ptr(), 1024, 4, 8, 2, &mut live) }, SUBETHA_OK);
        let mut out = [0u8; 8];
        // SAFETY: the key is its declared size and `out` has room for a value.
        let rc = unsafe { subetha_hashmap_get(live, key.as_ptr(), key.len(), out.as_mut_ptr(), out.len(), &mut len) };
        assert_eq!(rc, SUBETHA_E_MAP_KEY_ABSENT);
        assert_eq!(detail(), name(SUBETHA_E_MAP_KEY_ABSENT), "the lookup's own detail, not the pop's");

        assert_eq!(subetha_handle_destroy(live), SUBETHA_OK);
        assert_eq!(subetha_handle_destroy(ring), SUBETHA_OK);
        std::fs::remove_file(&path).expect("the map file goes");
    }

    /// An open of a file that is not there names the path ahead of the
    /// operating system's message.
    #[test]
    fn a_failing_open_names_the_path_it_was_given() {
        use crate::hashmap::subetha_hashmap_open;
        use crate::runtime::subetha_init;

        assert_eq!(subetha_init(crate::runtime::SUBETHA_MODE_STRICT), SUBETHA_OK);
        let path = std::env::temp_dir().join(format!("subetha-ffi-absent-{}.bin", std::process::id()));
        let text = path.to_str().expect("a UTF-8 temp path");
        let c_path = std::ffi::CString::new(text).expect("no NUL in the path");
        let mut map = 0;
        // SAFETY: a valid path and out pointer.
        let rc = unsafe { subetha_hashmap_open(c_path.as_ptr(), 16, 4, 8, 2, &mut map) };
        assert_eq!(rc, SUBETHA_E_RING_IO);
        let said = detail();
        assert!(said.starts_with(&format!("'{text}': ")), "{said}");
    }

    #[test]
    fn every_code_has_a_name_and_the_unknown_one_says_so() {
        for code in [
            SUBETHA_OK,
            SUBETHA_E_NOT_INITIALIZED,
            SUBETHA_E_INIT_CONFLICT,
            SUBETHA_E_INVALID_ARGUMENT,
            SUBETHA_E_INVALID_HANDLE,
            SUBETHA_E_HANDLE_POISONED,
            SUBETHA_E_PANIC,
            SUBETHA_E_WRONG_KIND,
            SUBETHA_E_OUT_OF_MEMORY,
            SUBETHA_E_BUFFER_TOO_SMALL,
            SUBETHA_E_INVALID_UTF8,
            SUBETHA_E_TIMEOUT,
            SUBETHA_E_SHUT_DOWN,
            SUBETHA_E_HANDLES_WERE_LIVE,
            SUBETHA_E_NOT_SUPPORTED,
            SUBETHA_E_DESTROYED,
            SUBETHA_E_RING_FULL,
            SUBETHA_E_RING_EMPTY,
            SUBETHA_E_RING_LAYOUT_MISMATCH,
            SUBETHA_E_RING_PAYLOAD_TOO_LARGE,
            SUBETHA_E_RING_NOT_STAMPED,
            SUBETHA_E_RING_NOT_DRAINER,
            SUBETHA_E_RING_STALE_BACKLOG,
            SUBETHA_E_RING_IO,
            SUBETHA_E_RING_TOO_MANY_PRODUCERS,
            SUBETHA_E_RING_TOO_MANY_CONSUMERS,
            SUBETHA_E_RING_GROWTH_FAILED,
            SUBETHA_E_RING_WAKER_FULL,
            SUBETHA_E_RING_WAKER_LAYOUT,
            SUBETHA_E_BROADCAST_NO_CONSUMER_SLOT,
            SUBETHA_E_BROADCAST_INVALID_CONSUMER,
            SUBETHA_E_PUBSUB_PENDING,
            SUBETHA_E_PUBSUB_LOST,
            SUBETHA_E_DEQUE_NOT_OWNER,
            SUBETHA_E_MAP_FULL,
            SUBETHA_E_MAP_KEY_ABSENT,
            SUBETHA_E_ARENA_FULL,
            SUBETHA_E_ARENA_INVALID_REF,
            SUBETHA_E_READ_ONLY,
            SUBETHA_E_OUT_OF_BOUNDS,
            SUBETHA_E_NOT_A_FRAME,
        ] {
            let s = unsafe { CStr::from_ptr(subetha_strerror(code)) };
            assert_ne!(s.to_str().unwrap(), "unknown subetha code", "code {code} has no name");
        }
        let s = unsafe { CStr::from_ptr(subetha_strerror(9_999)) };
        assert_eq!(s.to_str().unwrap(), "unknown subetha code");
    }

    #[test]
    fn detail_reports_the_size_it_needs_and_never_overruns() {
        set_detail("a path and two sizes");
        let needed = unsafe { subetha_last_error_detail(std::ptr::null_mut(), 0) };
        assert_eq!(needed, "a path and two sizes".len() + 1);
        let mut small = [0x7f as c_char; 8];
        let got = unsafe { subetha_last_error_detail(small.as_mut_ptr(), small.len()) };
        assert_eq!(got, needed, "a short buffer still reports the full size");
        assert_eq!(small[7], 0, "the copy is NUL-terminated inside the buffer");
        let text = unsafe { CStr::from_ptr(small.as_ptr()) }.to_str().unwrap();
        assert_eq!(text, "a path ");
        let mut full = vec![0 as c_char; needed];
        let got = unsafe { subetha_last_error_detail(full.as_mut_ptr(), full.len()) };
        assert_eq!(got, needed);
        let text = unsafe { CStr::from_ptr(full.as_ptr()) }.to_str().unwrap();
        assert_eq!(text, "a path and two sizes");
    }
}
