//! An adaptive object of the caller's own through the C ABI: a strategy
//! tag, and a ring the caller records its operations into. Observed under a
//! policy with `subetha_observe_with_policy`, it has the process's sidecar
//! drain the ring into statistics after each scan and move the tag to what
//! the policy answers, and the caller's code reads the tag to choose how it
//! works. It is Python's `Adaptive` and PowerShell's `New-SubEthaAdaptive`,
//! and what a Rust type implementing `AdaptiveInstance` is. It lives in
//! this process's memory and runs no background work of its own, so strict
//! and managed modes are the same.

use subetha_core::{HandshakeHeader, Observation, ObservationRing};
use subetha_sidecar::{AdaptiveInstance, NoMigrationPolicy, Policy};

use crate::error::{fail, SUBETHA_E_INVALID_ARGUMENT, SUBETHA_E_WRONG_KIND, SUBETHA_OK};
use crate::handle::{subetha_handle, Object, SUBETHA_KIND_ADAPTIVE};
use crate::runtime::{entry, issue, require_initialized, resolve_mode, with_kind};

pub(crate) struct AdaptiveObject {
    header: HandshakeHeader,
    ring: ObservationRing,
}

impl AdaptiveInstance for AdaptiveObject {
    fn header(&self) -> &HandshakeHeader {
        &self.header
    }

    fn ring(&self) -> &ObservationRing {
        &self.ring
    }

    /// Observed without a policy of the caller's own, the tag moves only
    /// when the caller moves it.
    fn make_policy(&self) -> Box<dyn Policy> {
        Box::new(NoMigrationPolicy)
    }
}

impl AdaptiveObject {
    /// The object as the sidecar observes it.
    pub(crate) fn adaptive(&self) -> &(dyn AdaptiveInstance + 'static) {
        self
    }
}

impl AdaptiveObject {
    /// The object parks nothing inside a call, so a destroy has nobody to
    /// wake.
    pub(crate) fn interrupt(&self) {}
}

fn with_adaptive(handle: subetha_handle, f: impl FnOnce(&AdaptiveObject) -> i32) -> i32 {
    with_kind(handle, SUBETHA_KIND_ADAPTIVE, |object| match object {
        Object::Adaptive(a) => f(a),
        _ => fail(SUBETHA_E_WRONG_KIND, "the handle does not name an adaptive object"),
    })
}

/// Make an adaptive object at tag 0. Its ring takes nothing until the
/// object is observed.
///
/// # Safety
/// `out` is a valid pointer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn subetha_adaptive_create(mode: u32, out: *mut subetha_handle) -> i32 {
    entry(|| {
        if let Err(code) = require_initialized() {
            return code;
        }
        if out.is_null() {
            return fail(SUBETHA_E_INVALID_ARGUMENT, "out is null");
        }
        if let Err(code) = resolve_mode(mode) {
            return code;
        }
        let object = AdaptiveObject { header: HandshakeHeader::new(), ring: ObservationRing::new() };
        unsafe { issue(Object::Adaptive(object), out) }
    })
}

/// Record one operation of kind `op_kind` that took `latency_ticks`, in
/// whatever unit the policy reads. Kinds 1 to 6 are counted apart, 7 and
/// above share one count, and 0 is unspecified. `contended` marks an
/// operation that took a slow path and `empty` one that found nothing.
/// `out_taken`, when not null, receives false when the ring did not take
/// it: before the object is observed, or while the ring is full.
///
/// # Safety
/// `out_taken` is null or a valid pointer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn subetha_adaptive_record(
    handle: subetha_handle,
    op_kind: u16,
    latency_ticks: u64,
    contended: bool,
    empty: bool,
    out_taken: *mut bool,
) -> i32 {
    with_adaptive(handle, |a| {
        let flags = u16::from(contended) | (u16::from(empty) << 1);
        let taken = a.ring.push(Observation { op_kind, flags, latency_ticks, ..Observation::ZERO });
        if !out_taken.is_null() {
            // SAFETY: checked non-null; the caller guarantees it is writable.
            unsafe { *out_taken = taken };
        }
        SUBETHA_OK
    })
}

/// The strategy tag into `out`; a policy's answer moves it.
///
/// # Safety
/// `out` is a valid pointer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn subetha_adaptive_tag(handle: subetha_handle, out: *mut u32) -> i32 {
    with_adaptive(handle, |a| {
        if out.is_null() {
            return fail(SUBETHA_E_INVALID_ARGUMENT, "out is null");
        }
        // SAFETY: checked non-null; the caller guarantees it is writable.
        unsafe { *out = a.header.tag() };
        SUBETHA_OK
    })
}

/// Move the tag from the caller's own code.
#[unsafe(no_mangle)]
pub extern "C" fn subetha_adaptive_set_tag(handle: subetha_handle, tag: u32) -> i32 {
    with_adaptive(handle, |a| {
        a.header.set_tag(tag);
        SUBETHA_OK
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::observe::{
        subetha_instance_stats, subetha_observation_policy_errors, subetha_observe_with_policy, subetha_sidecar_scan_now,
        subetha_unobserve,
    };
    use crate::policy::{SUBETHA_POLICY_MOVE, SUBETHA_POLICY_STAY};
    use crate::runtime::{subetha_handle_destroy, subetha_init, SUBETHA_MODE_STRICT};
    use std::ffi::c_void;
    use std::sync::atomic::{AtomicI32, AtomicU64, Ordering};

    /// What the callbacks saw, shared with the test through the context.
    #[derive(Default)]
    struct Seen {
        handle: AtomicU64,
        decides: AtomicU64,
        releases: AtomicU64,
        unobserve_from_decide: AtomicI32,
    }

    unsafe extern "C" fn move_to_seven(
        context: *mut c_void,
        stats: *const subetha_instance_stats,
        _current_tag: u32,
        out_tag: *mut u32,
    ) -> i32 {
        // SAFETY: the test passes a `Seen` that outlives the registration.
        let seen = unsafe { &*(context as *const Seen) };
        seen.decides.fetch_add(1, Ordering::AcqRel);
        seen.unobserve_from_decide.store(subetha_unobserve(seen.handle.load(Ordering::Acquire)), Ordering::Release);
        // SAFETY: the sidecar passes valid statistics and a writable tag.
        if unsafe { (*stats).ops_observed } == 0 {
            return SUBETHA_POLICY_STAY;
        }
        unsafe { *out_tag = 7 };
        SUBETHA_POLICY_MOVE
    }

    unsafe extern "C" fn answer_badly(_: *mut c_void, _: *const subetha_instance_stats, _: u32, _: *mut u32) -> i32 {
        -1
    }

    unsafe extern "C" fn count_release(context: *mut c_void) {
        // SAFETY: as in `move_to_seven`.
        let seen = unsafe { &*(context as *const Seen) };
        seen.releases.fetch_add(1, Ordering::AcqRel);
    }

    /// A policy written in C moves the tag, cannot end its own registration
    /// from inside `decide`, has `release` called once and only after the
    /// registration ends, and has a bad answer counted with the tag left
    /// alone; a refused registration never calls `release`.
    #[test]
    fn a_c_policy_moves_the_tag_and_is_released_once_after_the_last_decide() {
        let _serial = crate::observe::SIDECAR_COUNT_TESTS.claim();
        assert_eq!(subetha_init(SUBETHA_MODE_STRICT), SUBETHA_OK);
        let seen = Seen::default();
        let context = &seen as *const Seen as *mut c_void;
        let mut adaptive = 0;
        let (mut taken, mut tag, mut errors) = (true, 0u32, 9u64);
        // SAFETY: valid out pointers throughout; `seen` outlives every
        // registration, each ended before the test returns.
        unsafe {
            assert_eq!(subetha_adaptive_create(SUBETHA_MODE_STRICT, &mut adaptive), SUBETHA_OK);
            seen.handle.store(adaptive, Ordering::Release);
            assert_eq!(subetha_adaptive_record(adaptive, 1, 10, false, false, &mut taken), SUBETHA_OK);
            assert!(!taken, "the ring takes nothing before the object is observed");

            assert_eq!(subetha_observe_with_policy(adaptive, None, None, context), SUBETHA_E_INVALID_ARGUMENT);
            assert_eq!(subetha_observe_with_policy(adaptive, Some(move_to_seven), Some(count_release), context), SUBETHA_OK);
            assert_eq!(
                subetha_observe_with_policy(adaptive, Some(move_to_seven), Some(count_release), context),
                SUBETHA_E_INVALID_ARGUMENT,
                "already observed"
            );
            assert_eq!(seen.releases.load(Ordering::Acquire), 0, "a refused registration never calls release");
            assert_eq!(subetha_adaptive_record(adaptive, 1, 10, true, false, &mut taken), SUBETHA_OK);
            assert!(taken);
            assert_eq!(subetha_sidecar_scan_now(), SUBETHA_OK);
            assert!(seen.decides.load(Ordering::Acquire) >= 1);
            assert_eq!(subetha_adaptive_tag(adaptive, &mut tag), SUBETHA_OK);
            assert_eq!(tag, 7, "the policy's answer moved the tag");
            assert_eq!(seen.unobserve_from_decide.load(Ordering::Acquire), SUBETHA_E_INVALID_ARGUMENT);
            assert_eq!(subetha_observation_policy_errors(adaptive, &mut errors), SUBETHA_OK);
            assert_eq!(errors, 0);

            assert_eq!(subetha_unobserve(adaptive), SUBETHA_OK);
            assert_eq!(seen.releases.load(Ordering::Acquire), 1, "release ran once the registration ended");
            let decided = seen.decides.load(Ordering::Acquire);
            assert_eq!(subetha_sidecar_scan_now(), SUBETHA_OK);
            assert_eq!(seen.decides.load(Ordering::Acquire), decided, "decide is never called after unobserve returns");

            assert_eq!(subetha_observe_with_policy(adaptive, Some(answer_badly), None, std::ptr::null_mut()), SUBETHA_OK);
            assert_eq!(subetha_adaptive_record(adaptive, 2, 5, false, true, &mut taken), SUBETHA_OK);
            assert_eq!(subetha_sidecar_scan_now(), SUBETHA_OK);
            assert_eq!(subetha_observation_policy_errors(adaptive, &mut errors), SUBETHA_OK);
            assert!(errors >= 1, "a bad answer is counted");
            assert_eq!(subetha_adaptive_tag(adaptive, &mut tag), SUBETHA_OK);
            assert_eq!(tag, 7, "and leaves the tag where it was");
            assert_eq!(subetha_adaptive_set_tag(adaptive, 3), SUBETHA_OK);
            assert_eq!(subetha_adaptive_tag(adaptive, &mut tag), SUBETHA_OK);
            assert_eq!(tag, 3);
            assert_eq!(subetha_handle_destroy(adaptive), SUBETHA_OK);
            assert_eq!(seen.releases.load(Ordering::Acquire), 1);
        }
    }
}
