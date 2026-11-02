//! What the family blocking types share: the two wakers beside a
//! structure under the names the C ABI gives them, the error a wait comes
//! to, and the loop that parks until an operation goes through.
//!
//! A family's blocking type, such as
//! [`BlockingBroadcastRing`](crate::blocking_broadcast_ring::BlockingBroadcastRing),
//! owns its structure and these two wakers. A push or pop through it wakes
//! the other side. A wait through it tries the operation, takes a waiter
//! slot, tries once more, and only then sleeps, so a wake that landed
//! between the failed try and the park is caught without entering the
//! kernel.
//!
//! The wakers sit beside the structure under fixed names, which is how
//! every process finds the same ones: `<path>.cwaker.bin` and
//! `<path>.pwaker.bin` beside a file, the regions `{name}_cwaker` and
//! `{name}_pwaker` beside a structure in named shared memory, and in this
//! process alone for an anonymous one. Every process names the same
//! number of waiter slots, [`MAX_WAITERS_DEFAULT`] unless all of them
//! agree on another; a waker made with another number is a
//! [`BlockingWaitError::WakerLayout`].

use std::convert::Infallible;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, Instant};

pub use crate::cross_process_waker::MAX_WAITERS_DEFAULT;
use crate::cross_process_waker::{waker_region_size, CrossProcessWaker, WakerError};
use crate::layout::Mismatch;
use crate::shm_file::{ShmFile, ShmNamespace};

/// The sequence every parked waiter registers and every wake reaches: any
/// item, and any freed slot, counts.
pub(crate) const WAKE_ANY: u64 = 0;

/// What a call through a family's blocking type can come to besides
/// success: the structure's own refusal, or what waiting adds to it.
#[derive(Debug)]
pub enum BlockingWaitError<E> {
    /// The structure refused the operation for a reason waiting cannot
    /// cure.
    Op(E),
    /// Every waiter slot was in use, so the call could not wait.
    WakerFull,
    /// The timeout passed before the other side made room or an item.
    Timeout,
    /// A waker's mapping holds another layout; the error names the field
    /// that differs, what the mapping holds and what was asked for.
    WakerLayout(Mismatch),
    /// The operating system refused to map a waker, or a waker's sleep.
    Io(std::io::Error),
    /// The handle was closed while the call waited, or before it began.
    Closed,
}

impl<E: std::fmt::Display> std::fmt::Display for BlockingWaitError<E> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Op(e) => write!(f, "{e}"),
            Self::WakerFull => write!(f, "every waker slot is in use"),
            Self::Timeout => write!(f, "timed out before the other side woke this one"),
            Self::WakerLayout(m) => write!(f, "a waker mapping of another shape: {m}"),
            Self::Io(e) => write!(f, "io: {e}"),
            Self::Closed => write!(f, "the handle was closed"),
        }
    }
}

impl<E: std::error::Error + 'static> std::error::Error for BlockingWaitError<E> {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Op(e) => Some(e),
            Self::Io(e) => Some(e),
            Self::WakerFull | Self::Timeout | Self::WakerLayout(_) | Self::Closed => None,
        }
    }
}

impl<E> From<WakerError> for BlockingWaitError<E> {
    fn from(e: WakerError) -> Self {
        match e {
            WakerError::Full => Self::WakerFull,
            WakerError::Timeout => Self::Timeout,
            WakerError::LayoutMismatch(m) => Self::WakerLayout(m),
            WakerError::IoError(e) => Self::Io(e),
        }
    }
}

/// `path` with `suffix` appended to its last component.
pub(crate) fn suffixed(path: &Path, suffix: &str) -> PathBuf {
    let mut p = path.as_os_str().to_owned();
    p.push(suffix);
    PathBuf::from(p)
}

/// The consumer waker and the producer waker beside the file at `path`,
/// `<path>.cwaker.bin` and `<path>.pwaker.bin`, made when they do not
/// exist and attached to when they do.
pub(crate) fn file_wakers(path: &Path, max_waiters: usize) -> Result<(CrossProcessWaker, CrossProcessWaker), WakerError> {
    Ok((
        CrossProcessWaker::create(suffixed(path, ".cwaker.bin"), max_waiters)?,
        CrossProcessWaker::create(suffixed(path, ".pwaker.bin"), max_waiters)?,
    ))
}

/// The consumer waker and the producer waker beside the named
/// shared-memory region `name`, the regions `{name}_cwaker` and
/// `{name}_pwaker`, made with `sddl` as their security descriptor when
/// `create` and opened otherwise. Made regions' names go with this handle.
pub(crate) fn shm_wakers(
    name: &str,
    namespace: ShmNamespace,
    create: bool,
    sddl: Option<&str>,
    max_waiters: usize,
) -> Result<(CrossProcessWaker, CrossProcessWaker), WakerError> {
    Ok((
        shm_waker(name, "_cwaker", namespace, create, sddl, max_waiters)?,
        shm_waker(name, "_pwaker", namespace, create, sddl, max_waiters)?,
    ))
}

fn shm_waker(
    name: &str,
    suffix: &str,
    namespace: ShmNamespace,
    create: bool,
    sddl: Option<&str>,
    capacity: usize,
) -> Result<CrossProcessWaker, WakerError> {
    let region_name = format!("{name}{suffix}");
    let size = waker_region_size(capacity);
    let region = if create {
        ShmFile::create_named_secured(&region_name, size, namespace, sddl)
    } else {
        ShmFile::open_named_secured(&region_name, size, namespace, sddl)
    };
    let region = region.map_err(|e| match Mismatch::carried_by(&e) {
        Some(m) => WakerError::LayoutMismatch(m),
        None => WakerError::IoError(std::io::Error::new(e.kind(), format!("waker region {region_name}: {e}"))),
    })?;
    if create {
        CrossProcessWaker::create_from_shm(region, capacity)
    } else {
        CrossProcessWaker::open_from_shm(region, capacity)
    }
}

/// The instant `timeout` from now; `None` for no deadline.
pub(crate) fn deadline(timeout: Option<Duration>) -> Option<Instant> {
    timeout.map(|t| Instant::now() + t)
}

/// What one try of an operation came to.
enum Attempt<T, E> {
    /// It went through.
    Done(T),
    /// It met a refusal waiting can cure.
    Blocked,
    /// It met a refusal waiting cannot cure.
    Refused(E),
}

/// A handle's closed flag and its count of waits refused for want of a
/// waiter slot, which every family's blocking type carries.
#[derive(Default)]
pub(crate) struct WaitState {
    closed: AtomicBool,
    waiters_full: AtomicU64,
}

impl WaitState {
    pub(crate) fn is_closed(&self) -> bool {
        self.closed.load(Ordering::Acquire)
    }

    pub(crate) fn close(&self) {
        self.closed.store(true, Ordering::Release);
    }

    pub(crate) fn waiters_full(&self) -> u64 {
        self.waiters_full.load(Ordering::Acquire)
    }

    /// Run `op` until it goes through, parking on `waker` while
    /// `would_block` says its refusal is one waiting can cure, until
    /// `deadline`. Any other refusal returns at once as [`BlockingWaitError::Op`].
    pub(crate) fn wait<T, E>(
        &self,
        waker: &CrossProcessWaker,
        deadline: Option<Instant>,
        would_block: impl Fn(&E) -> bool,
        mut op: impl FnMut() -> Result<T, E>,
    ) -> Result<T, BlockingWaitError<E>> {
        self.park_until(waker, deadline, || match op() {
            Ok(v) => Attempt::Done(v),
            Err(e) if would_block(&e) => Attempt::Blocked,
            Err(e) => Attempt::Refused(e),
        })
    }

    /// Run `op` until it gives a value, parking on `waker` while it gives
    /// none, until `deadline`: the wait for an operation whose only
    /// refusal is finding nothing, as a steal or a pop does.
    pub(crate) fn wait_for<T>(
        &self,
        waker: &CrossProcessWaker,
        deadline: Option<Instant>,
        mut op: impl FnMut() -> Option<T>,
    ) -> Result<T, BlockingWaitError<Infallible>> {
        self.park_until(waker, deadline, || op().map_or(Attempt::Blocked, Attempt::Done))
    }

    /// Try `attempt`, take a waiter slot, try again, and sleep on `waker`
    /// until a wake or `deadline`, round after round, until an attempt
    /// goes through or meets a refusal waiting cannot cure.
    fn park_until<T, E>(
        &self,
        waker: &CrossProcessWaker,
        deadline: Option<Instant>,
        mut attempt: impl FnMut() -> Attempt<T, E>,
    ) -> Result<T, BlockingWaitError<E>> {
        loop {
            match attempt() {
                Attempt::Done(v) => return Ok(v),
                Attempt::Blocked => {}
                Attempt::Refused(e) => return Err(BlockingWaitError::Op(e)),
            }
            if self.is_closed() {
                return Err(BlockingWaitError::Closed);
            }
            let token = match waker.try_park(WAKE_ANY) {
                Ok(token) => token,
                Err(WakerError::Full) => {
                    self.waiters_full.fetch_add(1, Ordering::AcqRel);
                    return Err(BlockingWaitError::WakerFull);
                }
                Err(e) => return Err(e.into()),
            };
            match attempt() {
                Attempt::Done(v) => {
                    waker.release(token);
                    return Ok(v);
                }
                Attempt::Blocked => {}
                Attempt::Refused(e) => {
                    waker.release(token);
                    return Err(BlockingWaitError::Op(e));
                }
            }
            if self.is_closed() {
                waker.release(token);
                return Err(BlockingWaitError::Closed);
            }
            let remaining = match deadline {
                None => None,
                Some(d) => match d.checked_duration_since(Instant::now()) {
                    Some(left) if !left.is_zero() => Some(left),
                    _ => {
                        waker.release(token);
                        return Err(BlockingWaitError::Timeout);
                    }
                },
            };
            match waker.wait(token, remaining) {
                Ok(()) | Err(WakerError::Timeout) => {}
                Err(e) => return Err(e.into()),
            }
        }
    }
}
