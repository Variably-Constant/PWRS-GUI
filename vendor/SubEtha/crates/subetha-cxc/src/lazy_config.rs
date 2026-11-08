//! `LazyConfig<T>` - thundering-herd-proof distributed config fetch.
//!
//! Composite primitive built on `SharedOnceCell<T>` + (optionally)
//! the BackgroundScheduler's Pass dispatch. Guarantees that one
//! config-fetch closure runs at a time across all participating
//! processes and exactly one result is published - no matter how many
//! of them call `get_or_fetch` concurrently. For a caller that finds a
//! fetch in flight, `get_or_fetch` returns [`Fetch::Pending`] at once
//! rather than waiting for it, and the caller reads the published value
//! on a later call. A fetcher whose process dies, or whose fetch panics,
//! before publishing is replaced by the next caller, whose fetch then
//! runs, so a fetch runs more than once only after one ended unpublished.
//!
//! # Why this exists
//!
//! Distributed services often want shared config (Consul, etcd,
//! DNS, an internal config service) loaded into every process once.
//! Naive implementations have every process independently fetch:
//! N processes -> N backend requests at startup, a classic
//! thundering herd against the config service. With LazyConfig,
//! one process fetches; all others see the result.
//!
//! # Two access modes
//!
//! 1. **Local fetcher**: `get_or_fetch(|| { ... })` runs the
//!    closure in the calling process if it takes the fetch claim;
//!    for other callers it returns Pending until the value is published.
//!
//! 2. **Scheduler-dispatched fetcher** (extension): submit a Pass
//!    via the BackgroundScheduler with a registered closure_id.
//!    Any process registered for that closure_id can serve as the
//!    fetcher; the result is published via the SharedOnceCell. This
//!    allows the fetch to happen in a process dedicated to that
//!    role (e.g., a privileged process with network access) while
//!    other processes only read the cell.
//!
//! Both modes share the same fetch claim, so the
//! thundering-herd-prevention property holds either way.

use std::path::Path;
use std::sync::Arc;

pub use crate::shared_once_cell::Fetch;
use crate::shared_once_cell::{SharedOnceCell, SharedOnceError};

#[derive(Debug)]
pub enum LazyConfigError {
    Once(SharedOnceError),
}

impl std::fmt::Display for LazyConfigError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Once(e) => write!(f, "once cell: {e}"),
        }
    }
}

impl std::error::Error for LazyConfigError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Once(e) => Some(e),
        }
    }
}

impl From<SharedOnceError> for LazyConfigError {
    fn from(e: SharedOnceError) -> Self { Self::Once(e) }
}

pub struct LazyConfig<T: Copy + Send + Sync + 'static> {
    cell: Arc<SharedOnceCell<T>>,
    header_sidecar: subetha_core::HandshakeHeader,
    ring_sidecar: Box<subetha_core::ObservationRing>,
}

impl<T: Copy + Send + Sync + 'static> subetha_sidecar::AdaptiveInstance for LazyConfig<T> {
    fn header(&self) -> &subetha_core::HandshakeHeader { &self.header_sidecar }
    fn ring(&self) -> &subetha_core::ObservationRing { &self.ring_sidecar }
    fn make_policy(&self) -> Box<dyn subetha_sidecar::Policy> {
        Box::new(subetha_sidecar::NoMigrationPolicy)
    }
}

impl<T: Copy + Send + Sync + 'static> LazyConfig<T> {
    pub fn create(path: impl AsRef<Path>) -> Result<Self, LazyConfigError> {
        Ok(Self {
            cell: Arc::new(SharedOnceCell::create(path)?),
            header_sidecar: subetha_core::HandshakeHeader::new(),
            ring_sidecar: Box::new(subetha_core::ObservationRing::new()),
        })
    }

    pub fn open(path: impl AsRef<Path>) -> Result<Self, LazyConfigError> {
        Ok(Self {
            cell: Arc::new(SharedOnceCell::open(path)?),
            header_sidecar: subetha_core::HandshakeHeader::new(),
            ring_sidecar: Box::new(subetha_core::ObservationRing::new()),
        })
    }

    /// Fast path: returns the value when loaded, otherwise None.
    pub fn try_get(&self) -> Option<T> {
        let r = self.cell.get();
        self.ring_sidecar.push_op(
            crate::sidecar_ops::lazy_config::OP_GET,
            if r.is_none() { 2 } else { 0 },
        );
        r
    }

    /// True when the config has been loaded.
    pub fn is_loaded(&self) -> bool {
        self.cell.is_initialized()
    }

    /// The loaded config, or `fetcher`'s, run under the cell's one fetch
    /// claim so that across every process mapping this file at most one
    /// fetch runs at a time. For a caller that finds a live caller's fetch
    /// in flight it returns [`Fetch::Pending`] at once, and that caller
    /// reads the value on a later call; a fetch claim whose holder is gone
    /// is taken over, and a fetch that panics gives the claim back.
    pub fn get_or_fetch<F: FnOnce() -> T>(&self, fetcher: F) -> Fetch<T> {
        if let Some(v) = self.cell.get() {
            self.ring_sidecar.push_op(crate::sidecar_ops::lazy_config::OP_FETCH, 0);
            return Fetch::Ready(v);
        }
        self.fetch_unloaded(fetcher)
    }

    /// [`get_or_fetch`](Self::get_or_fetch) when the config was not loaded
    /// at its first look.
    #[cold]
    #[inline(never)]
    fn fetch_unloaded<F: FnOnce() -> T>(&self, fetcher: F) -> Fetch<T> {
        let v = self.cell.get_or_fetch(fetcher);
        self.ring_sidecar.push_op(crate::sidecar_ops::lazy_config::OP_FETCH, 1); // cold-fetch path
        v
    }

    /// Force-set the value without going through a fetcher. Useful
    /// for testing or for an admin-set-config workflow. Returns
    /// `true` if this caller's value became canonical (it won the
    /// CAS), `false` when the cell was already initialized.
    pub fn force_set(&self, value: T) -> bool {
        let ok = self.cell.set(value);
        self.ring_sidecar.push_op(
            crate::sidecar_ops::lazy_config::OP_FETCH,
            if ok { 0 } else { 1 }, // lost the race
        );
        ok
    }

    pub fn flush(&self) -> Result<(), LazyConfigError> {
        Ok(self.cell.flush()?)
    }

    /// Non-blocking flush: schedules a writeback via the OS.
    /// Note: Windows is only partially async (sync to page cache,
    /// not to disk).
    pub fn flush_async(&self) -> Result<(), LazyConfigError> {
        Ok(self.cell.flush_async()?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::thread;
    use std::time::Duration;

    /// A file for one test, removed when the test ends; declared before the
    /// primitive that maps it so it drops after it.
    fn tmp(name: &str) -> crate::test_paths::TmpFile {
        crate::test_paths::TmpFile::new(format!("subetha-lazyconf-{name}-{}.bin", std::process::id()))
    }

    #[test]
    fn unloaded_try_get_returns_none() {
        let p = tmp("unloaded");
        let c: LazyConfig<u64> = LazyConfig::create(&p).unwrap();
        assert_eq!(c.try_get(), None);
        assert!(!c.is_loaded());
    }

    #[test]
    fn first_fetch_loads_value() {
        let p = tmp("first");
        let c: LazyConfig<u64> = LazyConfig::create(&p).unwrap();
        assert_eq!(c.get_or_fetch(|| 12345), Fetch::Ready(12345));
        assert!(c.is_loaded());
        assert_eq!(c.try_get(), Some(12345));
    }

    #[test]
    fn second_fetch_returns_cached() {
        let p = tmp("second");
        let c: LazyConfig<u64> = LazyConfig::create(&p).unwrap();
        // Prime the cache; this fetcher must run.
        assert_eq!(c.get_or_fetch(|| 100), Fetch::Ready(100));
        // Second fetcher must not run; if it ran, the panic fires.
        assert_eq!(c.get_or_fetch(|| panic!("must not run on loaded config")), Fetch::Ready(100));
    }

    /// Sixteen callers fetching together run one fetch between them: the
    /// call returns Pending to the rest, none of them waiting, and every
    /// caller reads the one value once it is published.
    #[test]
    fn fetch_runs_at_most_once_under_concurrency() {
        let p = tmp("concurrent");
        let c: Arc<LazyConfig<u64>> = Arc::new(LazyConfig::create(&p).unwrap());
        let runs = Arc::new(AtomicU32::new(0));
        let gate = Arc::new(std::sync::Barrier::new(16));
        let handles: Vec<_> = (0..16)
            .map(|_| {
                let (c, runs, gate) = (Arc::clone(&c), Arc::clone(&runs), Arc::clone(&gate));
                thread::spawn(move || {
                    gate.wait();
                    c.get_or_fetch(|| {
                        runs.fetch_add(1, Ordering::AcqRel);
                        // A slow fetch, so the other callers arrive while it runs.
                        thread::sleep(Duration::from_millis(50));
                        9999u64
                    })
                })
            })
            .collect();
        let results: Vec<Fetch<u64>> = handles.into_iter().map(|h| h.join().unwrap()).collect();
        assert_eq!(runs.load(Ordering::Acquire), 1, "one fetch ran across 16 concurrent callers");
        assert!(
            results.iter().all(|r| matches!(r, Fetch::Ready(9999) | Fetch::Pending)),
            "a caller got something else: {results:?}"
        );
        assert!(results.contains(&Fetch::Ready(9999)), "the fetcher got its value");
        assert_eq!(c.try_get(), Some(9999), "every later caller reads it");
    }

    #[test]
    fn cross_handle_load_visible() {
        let p = tmp("cross-handle");
        let a: LazyConfig<u64> = LazyConfig::create(&p).unwrap();
        let b: LazyConfig<u64> = LazyConfig::open(&p).unwrap();
        // Process A fetches; B sees the result without running.
        assert_eq!(a.get_or_fetch(|| 7777), Fetch::Ready(7777));
        assert_eq!(b.get_or_fetch(|| panic!("must not run after A loaded")), Fetch::Ready(7777));
        assert!(b.is_loaded());
    }

    #[test]
    fn force_set_wins_first_then_loses() {
        let p = tmp("force-set");
        let c: LazyConfig<u64> = LazyConfig::create(&p).unwrap();
        assert!(c.force_set(42));
        assert!(!c.force_set(99));
        assert_eq!(c.try_get(), Some(42));
    }

    #[test]
    fn disk_persistence_loaded_config_survives_reopen() {
        let p = tmp("disk-persist");
        {
            let c: LazyConfig<u64> = LazyConfig::create(&p).unwrap();
            assert_eq!(c.get_or_fetch(|| 8888), Fetch::Ready(8888));
            c.flush().unwrap();
        }
        let c2: LazyConfig<u64> = LazyConfig::open(&p).unwrap();
        assert!(c2.is_loaded());
        assert_eq!(c2.try_get(), Some(8888));
        // Subsequent fetch returns cached value.
        assert_eq!(c2.get_or_fetch(|| panic!("must not run on already-loaded config")), Fetch::Ready(8888));
    }

    #[test]
    fn struct_config_round_trip() {
        #[derive(Clone, Copy, Debug, PartialEq)]
        #[repr(C)]
        struct ConfigBytes { max_conns: u32, ttl_ms: u32, debug: u32 }
        let p = tmp("struct-config");
        let c: LazyConfig<ConfigBytes> = LazyConfig::create(&p).unwrap();
        let loaded = c.get_or_fetch(|| ConfigBytes {
            max_conns: 256,
            ttl_ms: 60_000,
            debug: 1,
        });
        assert_eq!(loaded, Fetch::Ready(ConfigBytes { max_conns: 256, ttl_ms: 60_000, debug: 1 }));
    }

    /// A fetch claimed by a process that died before publishing is taken
    /// over: the next caller's fetch runs and its result is the config.
    #[test]
    fn a_fetch_claimed_by_a_process_that_is_gone_is_taken_over() {
        let p = tmp("gone");
        let c: LazyConfig<u64> = LazyConfig::create(&p).unwrap();
        // No system hands out pid u32::MAX - 1, and a stamp of 0 leaves
        // liveness to the pid alone.
        c.cell.set_fetch_claim_for_tests(crate::process_identity::holder(u32::MAX - 1, 0));
        assert_eq!(c.get_or_fetch(|| 31), Fetch::Ready(31));
        assert_eq!(c.try_get(), Some(31));
    }

    /// A fetch that panics leaves the config unloaded and its claim free,
    /// so the next caller's fetch loads it.
    #[test]
    fn a_fetch_that_panics_leaves_the_config_for_the_next_caller() {
        let p = tmp("panic");
        let c: LazyConfig<u64> = LazyConfig::create(&p).unwrap();
        let failed = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            c.get_or_fetch(|| -> u64 { panic!("the config service refused") })
        }));
        assert!(failed.is_err(), "the panic reached the caller");
        assert!(!c.is_loaded());
        assert_eq!(c.get_or_fetch(|| 64), Fetch::Ready(64));
    }
}
