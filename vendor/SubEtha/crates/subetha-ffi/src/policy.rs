//! A policy of the caller's own, written in C. The sidecar's scan thread
//! calls `decide` with the observed object's statistics and its current
//! strategy tag, one call at a time per registration, and moves the tag to
//! what it answers. This is the callback shape SubEtha's C ABI uses: the
//! caller's context first and an `int32_t` answer.
//!
//! The context stays the caller's to manage. `subetha_unobserve` and
//! `subetha_handle_destroy` return only once no scan is inside `decide`,
//! after which it is never called again, and a `release` the caller gave
//! is called once, after the last `decide`, by whichever thread ended the
//! registration.

use std::cell::Cell;
use std::ffi::c_void;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use subetha_sidecar::{InstanceStats, Policy};

use crate::handle::{subetha_handle, SUBETHA_HANDLE_NONE};
use crate::observe::subetha_instance_stats;

/// From a policy's `decide`: keep the current tag.
pub const SUBETHA_POLICY_STAY: i32 = 0;
/// From a policy's `decide`: move to the tag written into `out_tag`.
pub const SUBETHA_POLICY_MOVE: i32 = 1;

/// A policy's decision: given the observed object's statistics and its
/// current tag, answer `SUBETHA_POLICY_STAY`, or `SUBETHA_POLICY_MOVE` with
/// the tag to move to written into `out_tag`, which holds the current tag
/// when the call begins. Any other answer is counted as a policy error and
/// leaves the tag where it is. Called on the sidecar's scan thread; it must
/// not unwind, and it must not unobserve or destroy the handle it decides
/// for, which is refused.
#[allow(non_camel_case_types)]
pub type subetha_policy_decide =
    Option<unsafe extern "C" fn(context: *mut c_void, stats: *const subetha_instance_stats, current_tag: u32, out_tag: *mut u32) -> i32>;

/// Called once with the policy's context after its last `decide`, by
/// whichever thread ended the registration; it must not unwind.
#[allow(non_camel_case_types)]
pub type subetha_policy_release = Option<unsafe extern "C" fn(context: *mut c_void)>;

thread_local! {
    /// The handle whose policy this thread is running now, or
    /// `SUBETHA_HANDLE_NONE`.
    static DECIDING: Cell<subetha_handle> = const { Cell::new(SUBETHA_HANDLE_NONE) };
}

/// Whether this thread is inside the policy of `handle`, which ending that
/// registration from here would wait on forever.
pub(crate) fn deciding_for(handle: subetha_handle) -> bool {
    handle != SUBETHA_HANDLE_NONE && DECIDING.with(|d| d.get() == handle)
}

/// A policy's `decide`, checked non-null.
pub(crate) type DecideFn = unsafe extern "C" fn(*mut c_void, *const subetha_instance_stats, u32, *mut u32) -> i32;

/// What `subetha_observe_with_policy` was given, turned into a policy only
/// once the registration is certain to be made, so a refused call never
/// calls `release`.
pub(crate) struct Custom {
    pub(crate) decide: DecideFn,
    pub(crate) release: subetha_policy_release,
    pub(crate) context: *mut c_void,
}

/// The caller's context, carried to the scan thread.
struct Context(*mut c_void);

// SAFETY: the caller of `subetha_observe_with_policy` promises `decide` and
// `release` may be called with this context from the sidecar's scan thread
// and from whichever thread ends the registration.
unsafe impl Send for Context {}
// SAFETY: as above; the sidecar calls `decide` one call at a time per
// registration.
unsafe impl Sync for Context {}

pub(crate) struct CPolicy {
    handle: subetha_handle,
    decide: DecideFn,
    release: subetha_policy_release,
    context: Context,
    errors: Arc<AtomicU64>,
}

impl CPolicy {
    /// The policy `custom` describes for the object `handle` names,
    /// counting its errors into `errors`.
    pub(crate) fn new(handle: subetha_handle, custom: Custom, errors: Arc<AtomicU64>) -> Self {
        Self { handle, decide: custom.decide, release: custom.release, context: Context(custom.context), errors }
    }
}

impl Policy for CPolicy {
    fn decide(&self, stats: &InstanceStats, current_tag: u32) -> Option<u32> {
        let stats = subetha_instance_stats::from(*stats);
        let mut out_tag = current_tag;
        let outer = DECIDING.with(|d| d.replace(self.handle));
        // SAFETY: the registering caller promised `decide` is callable with
        // its context from this thread; both pointers name locals that live
        // across the call.
        let answer = unsafe { (self.decide)(self.context.0, &stats, current_tag, &mut out_tag) };
        DECIDING.with(|d| d.set(outer));
        match answer {
            SUBETHA_POLICY_STAY => None,
            SUBETHA_POLICY_MOVE => Some(out_tag),
            _ => {
                self.errors.fetch_add(1, Ordering::AcqRel);
                None
            }
        }
    }
}

impl Drop for CPolicy {
    fn drop(&mut self) {
        if let Some(release) = self.release {
            // SAFETY: the registering caller promised `release` takes its
            // context once; the sidecar drops a policy only after its last
            // `decide` has returned, on the thread ending the registration.
            unsafe { release(self.context.0) };
        }
    }
}
