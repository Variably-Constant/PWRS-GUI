//! Sidecar registration through the C ABI. A structure is registered
//! inside the object its handle names: `subetha_observe` starts it,
//! `subetha_unobserve` ends it, and destroying the handle ends it before
//! the object goes, so a registration never outlives what it reads. The
//! sidecar is the process's own, the one Rust's `SidecarBox` and the
//! bindings' `observe()` reach, and a registration takes the policy the
//! structure's type makes.

use std::ffi::c_void;

use subetha_sidecar::InstanceStats;

use crate::error::{fail, SUBETHA_E_INVALID_ARGUMENT, SUBETHA_OK};
use crate::handle::subetha_handle;
use crate::policy::{subetha_policy_decide, subetha_policy_release, Custom};
use crate::runtime::{entry, require_initialized, table};

/// Op kinds an instance's statistics count by; index 0 is unspecified.
pub const SUBETHA_N_OP_KINDS: usize = 8;
/// Distinct producer threads an instance's statistics track per op kind.
pub const SUBETHA_MAX_TRACKED_THREADS_PER_KIND: usize = 4;

const _: () = assert!(SUBETHA_N_OP_KINDS == subetha_sidecar::N_OP_KINDS);
const _: () = assert!(SUBETHA_MAX_TRACKED_THREADS_PER_KIND == subetha_sidecar::MAX_TRACKED_THREADS_PER_KIND);

/// What the sidecar has folded from an observed object's observations,
/// field for field the Rust `InstanceStats`.
#[repr(C)]
#[allow(non_camel_case_types)]
#[derive(Clone, Copy, Default)]
pub struct subetha_instance_stats {
    /// Observations drained.
    pub ops_observed: u64,
    /// The latency the observations carried, summed, in raw ticks.
    pub total_latency_ticks: u64,
    /// Observations that reported contention.
    pub contention_ops: u64,
    /// Observations by op kind.
    pub op_kind_counts: [u64; SUBETHA_N_OP_KINDS],
    /// Microseconds from registration to the last scan that drained one.
    pub last_drain_us: u64,
    /// Scans whose policy chose another strategy tag.
    pub migrations_triggered: u64,
    /// The distinct producer thread ids seen per op kind, as many of each
    /// row as `per_op_kind_distinct_count` says.
    pub per_op_kind_distinct_threads: [[u32; SUBETHA_MAX_TRACKED_THREADS_PER_KIND]; SUBETHA_N_OP_KINDS],
    /// Distinct producer threads per op kind, saturating one past the
    /// number tracked.
    pub per_op_kind_distinct_count: [u8; SUBETHA_N_OP_KINDS],
}

impl From<InstanceStats> for subetha_instance_stats {
    fn from(s: InstanceStats) -> Self {
        Self {
            ops_observed: s.ops_observed,
            total_latency_ticks: s.total_latency_ticks,
            contention_ops: s.contention_ops,
            op_kind_counts: s.op_kind_counts,
            last_drain_us: s.last_drain_us,
            migrations_triggered: s.migrations_triggered,
            per_op_kind_distinct_threads: s.per_op_kind_distinct_threads,
            per_op_kind_distinct_count: s.per_op_kind_distinct_count,
        }
    }
}

/// Run `f` once the library is initialized.
fn initialized(f: impl FnOnce() -> i32) -> i32 {
    entry(|| match require_initialized() {
        Ok(()) => f(),
        Err(code) => code,
    })
}

/// Write `value` through `out`.
///
/// # Safety
/// `out` is null or a valid pointer.
unsafe fn put<T>(out: *mut T, value: T) -> i32 {
    if out.is_null() {
        return fail(SUBETHA_E_INVALID_ARGUMENT, "out is null");
    }
    // SAFETY: checked non-null; the caller guarantees it is writable.
    unsafe { *out = value };
    SUBETHA_OK
}

/// Register the object `handle` names with the process's sidecar.
///
/// The kinds that record observations: the hash map, both Bloom filters,
/// the bit vector, the broadcast ring, the epoch barrier, the heartbeat
/// table, the HyperLogLog, the count-min sketch, the histogram, the rate
/// limiter, the topology map, the string arena, the leader election, the
/// fence clock, the three atomics, the
/// reservoir sampler, the versioned chain, the handle table, the adaptive
/// object, the Vyukov ring and the adaptive ring. An adaptive ring records one observation for each
/// pop whose stamp undercuts the same consumer's previous pop, so only a
/// stamped ring has any; an ordered receiver's pops are among them, so a
/// receiver is observed through its ring's handle.
///
/// `SUBETHA_E_NOT_SUPPORTED` for any other kind,
/// `SUBETHA_E_INVALID_ARGUMENT` for an object already observed, and
/// `SUBETHA_E_RING_FULL` when the sidecar holds its instance cap.
#[unsafe(no_mangle)]
pub extern "C" fn subetha_observe(handle: subetha_handle) -> i32 {
    initialized(|| table().observe(handle))
}

/// Register the object `handle` names with the process's sidecar under a
/// policy of the caller's own, as `subetha_observe` does under the one its
/// type makes. `decide` is called on the sidecar's scan thread, one call at
/// a time, with `context`; `release`, when not null, is called once with
/// `context` after the last `decide`, by whichever thread ends the
/// registration. The refusals are `subetha_observe`'s, plus a null
/// `decide`, and a refused call never calls `release`.
///
/// # Safety
/// `decide` and `release` may be called with `context` from the sidecar's
/// scan thread and from whichever thread ends the registration, until
/// `release` has been called or, without one, until `subetha_unobserve`
/// or `subetha_handle_destroy` returns.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn subetha_observe_with_policy(
    handle: subetha_handle,
    decide: subetha_policy_decide,
    release: subetha_policy_release,
    context: *mut c_void,
) -> i32 {
    initialized(|| {
        let Some(decide) = decide else {
            return fail(SUBETHA_E_INVALID_ARGUMENT, "decide is null");
        };
        table().observe_with(handle, Some(Custom { decide, release, context }))
    })
}

/// End the registration `subetha_observe` made. Destroying the handle ends
/// it too. Either returns only once no scan is inside the registration, so
/// a policy's `decide` is never called after it; a policy's own `decide`
/// cannot end its registration, which is refused.
#[unsafe(no_mangle)]
pub extern "C" fn subetha_unobserve(handle: subetha_handle) -> i32 {
    initialized(|| table().unobserve(handle))
}

/// How often the observed object's policy answered something other than
/// `SUBETHA_POLICY_STAY` or `SUBETHA_POLICY_MOVE`, into `out`; each such
/// scan left the tag where it was. Zero under the policy the object's type
/// makes.
///
/// # Safety
/// `out` is a valid pointer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn subetha_observation_policy_errors(handle: subetha_handle, out: *mut u64) -> i32 {
    initialized(|| match table().policy_errors(handle) {
        Ok(errors) => unsafe { put(out, errors) },
        Err(code) => code,
    })
}

/// The observed object's statistics into `out`, as the last scan that
/// drained it left them; `subetha_sidecar_scan_now` drains it first.
///
/// # Safety
/// `out` is a valid pointer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn subetha_observation_stats(handle: subetha_handle, out: *mut subetha_instance_stats) -> i32 {
    initialized(|| match table().observation_stats(handle) {
        Ok(stats) => unsafe { put(out, subetha_instance_stats::from(stats)) },
        Err(code) => code,
    })
}

/// Drain every observed object, from every surface in this process, and
/// fold what each held into its statistics, returning once that is done.
#[unsafe(no_mangle)]
pub extern "C" fn subetha_sidecar_scan_now() -> i32 {
    initialized(|| {
        subetha_sidecar::global().scan_now();
        SUBETHA_OK
    })
}

/// How many objects the process's sidecar observes, from every surface,
/// into `out`.
///
/// # Safety
/// `out` is a valid pointer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn subetha_sidecar_instance_count(out: *mut u64) -> i32 {
    initialized(|| unsafe { put(out, subetha_sidecar::global().instance_count() as u64) })
}

/// The most objects the sidecar observes before refusing another, into
/// `out`.
///
/// # Safety
/// `out` is a valid pointer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn subetha_sidecar_max_instances(out: *mut u64) -> i32 {
    initialized(|| unsafe { put(out, subetha_sidecar::global().max_instances() as u64) })
}

/// Set the most objects the sidecar observes before refusing another.
#[unsafe(no_mangle)]
pub extern "C" fn subetha_sidecar_set_max_instances(cap: u64) -> i32 {
    initialized(|| match usize::try_from(cap) {
        Ok(cap) => {
            subetha_sidecar::global().set_max_instances(cap);
            SUBETHA_OK
        }
        Err(e) => fail(SUBETHA_E_INVALID_ARGUMENT, format!("cap {cap} does not fit this platform: {e}")),
    })
}

/// How many scan threads the sidecar runs, one per NUMA node, into `out`.
///
/// # Safety
/// `out` is a valid pointer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn subetha_sidecar_node_count(out: *mut u64) -> i32 {
    initialized(|| unsafe { put(out, subetha_sidecar::global().node_count() as u64) })
}

/// Held for the whole of every unit test that reads the process-wide
/// sidecar's instance count or registers while another reads it, so one
/// test's registration never lands between another's two reads.
#[cfg(test)]
pub(crate) static SIDECAR_COUNT_TESTS: subetha_cxc::claim::Claim<()> = subetha_cxc::claim::Claim::new(());

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::{SUBETHA_E_NOT_SUPPORTED, SUBETHA_E_INVALID_ARGUMENT};
    use crate::hashmap::{subetha_hashmap_create, subetha_hashmap_get, subetha_hashmap_insert};
    use crate::runtime::{subetha_handle_destroy, subetha_init, SUBETHA_MODE_STRICT};
    use std::ffi::CString;

    /// A map observed through its handle reports each insert and get the
    /// sidecar drained, by kind; a second observe, a stats read after the
    /// registration ends, and a kind that records nothing are refused; and
    /// destroying an observed handle ends its registration.
    #[test]
    fn an_observed_map_reports_what_it_did_and_its_registration_ends_with_it() {
        let _serial = SIDECAR_COUNT_TESTS.claim();
        assert_eq!(subetha_init(SUBETHA_MODE_STRICT), SUBETHA_OK);
        let path = std::env::temp_dir().join(format!("subetha-ffi-observe-{}.bin", std::process::id()));
        let c_path = CString::new(path.to_str().expect("a UTF-8 temp path")).expect("no NUL in the path");
        let mut map = 0;
        // SAFETY: valid paths, buffers and out pointers throughout.
        unsafe {
            assert_eq!(subetha_hashmap_create(c_path.as_ptr(), 64, 4, 8, SUBETHA_MODE_STRICT, &mut map), SUBETHA_OK);
            assert_eq!(subetha_observe(map), SUBETHA_OK);
            assert_eq!(subetha_observe(map), SUBETHA_E_INVALID_ARGUMENT, "one registration per object");
            let mut out = [0u8; 8];
            let mut len = 0usize;
            for k in 0..10u32 {
                let (key, value) = (k.to_ne_bytes(), u64::from(k).to_ne_bytes());
                let rc = subetha_hashmap_insert(map, key.as_ptr(), key.len(), value.as_ptr(), value.len(), std::ptr::null_mut());
                assert_eq!(rc, SUBETHA_OK);
                assert_eq!(subetha_hashmap_get(map, key.as_ptr(), key.len(), out.as_mut_ptr(), out.len(), &mut len), SUBETHA_OK);
            }
            assert_eq!(subetha_sidecar_scan_now(), SUBETHA_OK);
            let mut stats = subetha_instance_stats::default();
            assert_eq!(subetha_observation_stats(map, &mut stats), SUBETHA_OK);
            assert_eq!(stats.ops_observed, 20, "ten inserts and ten gets");
            assert_eq!(stats.op_kind_counts[1], 10, "the inserts");
            assert_eq!(stats.op_kind_counts[2], 10, "the gets");

            assert_eq!(subetha_unobserve(map), SUBETHA_OK);
            assert_eq!(subetha_observation_stats(map, &mut stats), SUBETHA_E_INVALID_ARGUMENT);
            assert_eq!(subetha_unobserve(map), SUBETHA_E_INVALID_ARGUMENT, "nothing left to end");

            let mut before = 0u64;
            assert_eq!(subetha_sidecar_instance_count(&mut before), SUBETHA_OK);
            assert_eq!(subetha_observe(map), SUBETHA_OK, "a map observed again after its registration ended");
            assert_eq!(subetha_handle_destroy(map), SUBETHA_OK);
            let mut after = 0u64;
            assert_eq!(subetha_sidecar_instance_count(&mut after), SUBETHA_OK);
            assert!(after <= before, "destroying the observed map ended its registration: {before} then {after}");

            let mut cap = 0u64;
            assert_eq!(subetha_sidecar_max_instances(&mut cap), SUBETHA_OK);
            assert!(cap > 0);
            let mut nodes = 0u64;
            assert_eq!(subetha_sidecar_node_count(&mut nodes), SUBETHA_OK);
            assert!(nodes >= 1);
        }
        std::fs::remove_file(&path).expect("the map file goes");

        let mut channel = 0;
        let channel_path = std::env::temp_dir().join(format!("subetha-ffi-observe-chan-{}.bin", std::process::id()));
        let c_channel = CString::new(channel_path.to_str().expect("a UTF-8 temp path")).expect("no NUL in the path");
        // SAFETY: a valid path and out pointer.
        unsafe {
            assert_eq!(
                crate::channel::subetha_channel_create(c_channel.as_ptr(), 2, 1, 1, SUBETHA_MODE_STRICT, &mut channel),
                SUBETHA_OK
            );
        }
        assert_eq!(subetha_observe(channel), SUBETHA_E_NOT_SUPPORTED, "a channel records no observations");
        assert_eq!(subetha_handle_destroy(channel), SUBETHA_OK);
        let mut report = crate::ring::subetha_unlink_report::default();
        // SAFETY: a valid path and report pointer.
        assert_eq!(unsafe { crate::channel::subetha_channel_unlink(c_channel.as_ptr(), &mut report) }, SUBETHA_OK);
    }

    /// Every other kind that records observations registers, and one call
    /// made through its handle reaches the sidecar; a registration that
    /// drained nothing would mean the handle's calls feed another
    /// structure. An adaptive ring whose pops undercut no stamp registers
    /// and records nothing.
    #[test]
    fn each_observable_kind_reports_a_call_made_through_its_handle() {
        let _serial = SIDECAR_COUNT_TESTS.claim();
        use crate::reservoir::SUBETHA_RESERVOIR_ITEM_BYTES;
        use crate::ring::{subetha_ring_options, SUBETHA_RING_SLOT_BYTES};
        use crate::versioned_chain::SUBETHA_VERSIONED_CHAIN_ITEM_BYTES;
        use std::path::{Path, PathBuf};

        assert_eq!(subetha_init(SUBETHA_MODE_STRICT), SUBETHA_OK);
        let file = |kind: &str| -> (PathBuf, CString) {
            let path = std::env::temp_dir().join(format!("subetha-ffi-observe-{kind}-{}.bin", std::process::id()));
            let c_path = CString::new(path.to_str().expect("a UTF-8 temp path")).expect("no NUL in the path");
            (path, c_path)
        };
        // Observe `handle`, make `call` through it, destroy it, and answer
        // how many observations the sidecar drained in between.
        let drained = |kind: &str, handle: subetha_handle, call: &dyn Fn(subetha_handle) -> i32| -> u64 {
            assert_eq!(subetha_observe(handle), SUBETHA_OK, "{kind} registers");
            assert_eq!(call(handle), SUBETHA_OK, "{kind}'s call");
            assert_eq!(subetha_sidecar_scan_now(), SUBETHA_OK);
            let mut stats = subetha_instance_stats::default();
            // SAFETY: a valid out pointer.
            assert_eq!(unsafe { subetha_observation_stats(handle, &mut stats) }, SUBETHA_OK);
            assert_eq!(subetha_handle_destroy(handle), SUBETHA_OK);
            stats.ops_observed
        };
        let removed = |path: &Path| std::fs::remove_file(path).expect("the object's file goes");
        let mode = SUBETHA_MODE_STRICT;
        let pid = std::process::id();

        // SAFETY: valid paths, buffers and out pointers throughout; each
        // handle is used only before `drained` destroys it.
        unsafe {
            let mut h = 0;

            let (path, c) = file("cms");
            assert_eq!(crate::cms::subetha_cms_create(c.as_ptr(), 4, 64, mode, &mut h), SUBETHA_OK);
            assert_ne!(drained("the count-min sketch", h, &|h| crate::cms::subetha_cms_insert(h, b"item".as_ptr(), 4)), 0);
            removed(&path);

            let (path, c) = file("hll");
            assert_eq!(crate::hyper_log_log::subetha_hll_create(c.as_ptr(), 10, mode, &mut h), SUBETHA_OK);
            let insert = |h| crate::hyper_log_log::subetha_hll_insert(h, b"item".as_ptr(), 4);
            assert_ne!(drained("the HyperLogLog", h, &insert), 0);
            removed(&path);

            let (path, c) = file("histogram");
            let boundaries = [10u64, 100];
            let rc = crate::histogram::subetha_histogram_create(c.as_ptr(), boundaries.as_ptr(), boundaries.len(), mode, &mut h);
            assert_eq!(rc, SUBETHA_OK);
            let record = |h| crate::histogram::subetha_histogram_record(h, 50, &mut 0);
            assert_ne!(drained("the histogram", h, &record), 0);
            removed(&path);

            let (path, c) = file("rate-limiter");
            assert_eq!(crate::rate_limiter::subetha_rate_limiter_create(c.as_ptr(), 10, 10, mode, &mut h), SUBETHA_OK);
            assert_ne!(drained("the rate limiter", h, &|h| crate::rate_limiter::subetha_rate_limiter_try_acquire(h, 1)), 0);
            removed(&path);

            let (path, c) = file("topology");
            assert_eq!(crate::topology::subetha_topology_create(c.as_ptr(), 4, 2, 2, mode, &mut h), SUBETHA_OK);
            let send = |h| crate::topology::subetha_topology_record_send(h, 0, 1, &mut 0);
            assert_ne!(drained("the topology map", h, &send), 0);
            removed(&path);

            let (path, c) = file("atomic");
            assert_eq!(crate::atomic::subetha_atomic_u64_create(c.as_ptr(), 0, mode, &mut h), SUBETHA_OK);
            assert_ne!(drained("the 64-bit atomic", h, &|h| crate::atomic::subetha_atomic_u64_fetch_add(h, 1, &mut 0)), 0);
            removed(&path);

            let (path, c) = file("reservoir");
            assert_eq!(crate::reservoir::subetha_reservoir_create(c.as_ptr(), 4, mode, &mut h), SUBETHA_OK);
            let item = [5u8; SUBETHA_RESERVOIR_ITEM_BYTES];
            let offer = |h| crate::reservoir::subetha_reservoir_record(h, item.as_ptr(), item.len(), &mut 0);
            assert_ne!(drained("the reservoir sampler", h, &offer), 0);
            removed(&path);

            let (path, c) = file("versioned-chain");
            assert_eq!(crate::versioned_chain::subetha_versioned_chain_create(c.as_ptr(), 4, mode, &mut h), SUBETHA_OK);
            let value = [3u8; SUBETHA_VERSIONED_CHAIN_ITEM_BYTES];
            let push = |h| crate::versioned_chain::subetha_versioned_chain_push(h, 1, value.as_ptr(), value.len());
            assert_ne!(drained("the versioned chain", h, &push), 0);
            removed(&path);

            let (_, c) = file("arena");
            assert_eq!(crate::arena::subetha_arena_create(c.as_ptr(), 4096, mode, &mut h), SUBETHA_OK);
            let intern = |h| crate::arena::subetha_arena_intern(h, b"name".as_ptr(), 4, &mut 0);
            assert_ne!(drained("the string arena", h, &intern), 0);
            assert_eq!(crate::arena::subetha_arena_unlink(c.as_ptr(), std::ptr::null_mut()), SUBETHA_OK);

            let (_, c) = file("leader");
            assert_eq!(crate::leader::subetha_leader_create(c.as_ptr(), mode, &mut h), SUBETHA_OK);
            let claim = |h| crate::leader::subetha_leader_try_claim(h, pid, 2, &mut false);
            assert_ne!(drained("the leader election", h, &claim), 0);
            assert_eq!(crate::leader::subetha_leader_unlink(c.as_ptr(), std::ptr::null_mut()), SUBETHA_OK);

            let (_, c) = file("fence-clock");
            assert_eq!(crate::fence_clock::subetha_fence_clock_create(c.as_ptr(), 4, mode, &mut h), SUBETHA_OK);
            let tick = |h| {
                let mut slot = 0;
                let rc = crate::fence_clock::subetha_fence_clock_register(h, pid, &mut slot);
                if rc != SUBETHA_OK {
                    return rc;
                }
                crate::fence_clock::subetha_fence_clock_tick(h, slot, &mut crate::fence_clock::subetha_hlc::default())
            };
            assert_ne!(drained("the fence clock", h, &tick), 0);
            assert_eq!(crate::fence_clock::subetha_fence_clock_unlink(c.as_ptr(), std::ptr::null_mut()), SUBETHA_OK);

            let options = subetha_ring_options::default();
            assert_eq!(crate::vyukov::subetha_vyukov_create_anon(8, &options, &mut h), SUBETHA_OK);
            let push = |h| crate::vyukov::subetha_vyukov_try_push(h, b"job".as_ptr(), 3);
            assert_ne!(drained("the Vyukov ring", h, &push), 0);

            assert_eq!(crate::ring::subetha_ring_create_anon(1, 1, 8, &options, &mut h), SUBETHA_OK);
            let (mut producer, mut consumer) = (0, 0);
            assert_eq!(crate::ring::subetha_ring_register_producer(h, &mut producer), SUBETHA_OK);
            assert_eq!(crate::ring::subetha_ring_register_consumer(h, &mut consumer), SUBETHA_OK);
            let round_trip = |h| {
                let rc = crate::ring::subetha_ring_try_push(h, producer, b"job".as_ptr(), 3);
                if rc != SUBETHA_OK {
                    return rc;
                }
                let mut out = [0u8; SUBETHA_RING_SLOT_BYTES];
                crate::ring::subetha_ring_try_pop(h, consumer, out.as_mut_ptr(), out.len(), &mut 0)
            };
            assert_eq!(drained("the adaptive ring", h, &round_trip), 0, "a pop that undercuts no stamp records nothing");
        }
    }
}
