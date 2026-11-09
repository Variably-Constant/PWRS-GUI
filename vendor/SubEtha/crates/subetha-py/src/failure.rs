//! How a SubEtha failure reaches Python: as the exception Python's own
//! calls raise for the same failure. An operating-system failure is an
//! `OSError` carrying the code, which CPython turns into the subclass
//! that code names, with the path the call opened as `filename`; every
//! other failure raises the type its [`Class`] names.

use std::io::ErrorKind;
use std::path::Path;

use pyo3::exceptions::{
    PyBlockingIOError, PyFileExistsError, PyFileNotFoundError, PyIndexError,
    PyInterruptedError, PyKeyError, PyOSError, PyPermissionError, PyTimeoutError, PyValueError,
};
use pyo3::PyErr;

use subetha_core::MarshalError;
use subetha_cxc::adaptive_ring::AdaptiveError;
use subetha_cxc::api::ApiError;
use subetha_cxc::blocking_spsc_ring::BlockingError;
use subetha_cxc::blocking_wait::BlockingWaitError;
use subetha_cxc::capacity_adaptive_ring::CapacityMorphError;
use subetha_cxc::cross_process_notifier::NotifyError;
use subetha_cxc::cross_process_waker::WakerError;
use subetha_cxc::epoch_barrier::BarrierError;
use subetha_cxc::heartbeat::HeartbeatError;
use subetha_cxc::laned_versioned_map::LanedError;
use subetha_cxc::layout::Mismatch;
use subetha_cxc::message_transport::TransportError;
use subetha_cxc::raw_k_tower::RawTowerError;
use subetha_cxc::raw_lru_cache::RawLruError;
use subetha_cxc::shared_arc::ArcError;
use subetha_cxc::shared_atomic::SharedAtomicError;
use subetha_cxc::shared_bit_vec::BitVecError;
use subetha_cxc::shared_blocked_bloom_filter::BlockedBloomError;
use subetha_cxc::shared_bloom_filter::BloomError;
use subetha_cxc::shared_broadcast_ring::BroadcastError;
use subetha_cxc::shared_btree_map::BTreeError;
use subetha_cxc::shared_cell::SharedCellError;
use subetha_cxc::shared_condvar::CondvarError;
use subetha_cxc::shared_count_min_sketch::CMSError;
use subetha_cxc::shared_deque::DequeError;
use subetha_cxc::shared_epochs::EpochError;
use subetha_cxc::shared_fence_clock::FenceClockError;
use subetha_cxc::shared_graph::GraphError;
use subetha_cxc::shared_handle_table::HandleTableError;
use subetha_cxc::shared_hash_map::MapError;
use subetha_cxc::shared_histogram::HistogramError;
use subetha_cxc::shared_holder_table::SharedHolderError;
use subetha_cxc::shared_hyper_log_log::HLLError;
use subetha_cxc::shared_leader_election::LeaderError;
use subetha_cxc::shared_linked_list::LinkedListError;
use subetha_cxc::shared_once_cell::SharedOnceError;
use subetha_cxc::shared_rate_limiter::RateLimiterError;
use subetha_cxc::shared_region::RegionError;
use subetha_cxc::shared_reservoir_sampler::ReservoirError;
use subetha_cxc::shared_ring::RingError;
use subetha_cxc::shared_slab::SlabError;
use subetha_cxc::shared_string_arena::ArenaError;
use subetha_cxc::shared_time_point::TileError;
use subetha_cxc::shared_topology_map::TopologyError;
use subetha_cxc::shared_treiber_stack::StackError;
use subetha_cxc::shared_universal::UniversalError;
use subetha_cxc::shared_vec::VecError;
use subetha_cxc::shared_versioned_chain::ChainError;
use subetha_cxc::shared_versioned_index::IndexError;
use subetha_cxc::shared_versioned_slab::VersionedSlabError;
use subetha_cxc::versioned_btree_map::VersionedError;

pyo3::create_exception!(
    subetha,
    LayoutMismatch,
    PyOSError,
    "The file is too short, or holds a structure of another shape than the \
     call asked for: another kind, or the same kind built with another \
     capacity or element size. An OSError, so `except OSError` catches it \
     with every other failure to open a file."
);

pyo3::create_exception!(
    subetha,
    NotAFrame,
    pyo3::exceptions::PyException,
    "The next item on the ring is a plain item where a frame was asked for. \
     It stays on the ring, for `recv_any` or a plain receive to take."
);

pyo3::import_exception!(io, UnsupportedOperation);

/// What a SubEtha failure is, in the terms Python's own exceptions draw.
pub(crate) enum Class {
    /// The file holds a structure of another shape: [`LayoutMismatch`].
    Layout,
    /// The call passed what the structure cannot take: `ValueError`.
    Argument,
    /// An index past the end: `IndexError`.
    Index,
    /// No entry for the key: `KeyError`.
    Key,
    /// A wait ran out before what it waited for: `TimeoutError`.
    Timeout,
    /// An operation the structure was not opened or built for, such as a
    /// write to one opened read-only: `io.UnsupportedOperation`.
    Unsupported,
    /// The structure's own state refused the call, such as every slot
    /// taken: `OSError`.
    State,
    /// A ring's next item is a plain item where a frame was asked for:
    /// [`NotAFrame`].
    NotAFrame,
}

/// Sorts one SubEtha error type's variants into a [`Class`], by what each
/// variant's documentation says it means. A variant holding an
/// `io::Error` is raised from that error before `class` is asked, so it
/// answers [`Class::State`].
pub(crate) trait Classify: std::error::Error + 'static {
    fn class(&self) -> Class;
}

/// Raise a SubEtha failure from a call that opened no file.
pub(crate) fn os_err<E: Classify>(what: &str, e: E) -> PyErr {
    raise(what, None, &e)
}

/// Raise a SubEtha failure from a call that opened `path`. An
/// operating-system failure carries the path as `filename`; every other
/// failure ends its message with it.
pub(crate) fn open_err<E: Classify>(what: &str, path: impl AsRef<Path>, e: E) -> PyErr {
    raise(what, Some(path.as_ref()), &e)
}

fn raise<E: Classify>(what: &str, path: Option<&Path>, e: &E) -> PyErr {
    if let Some(io) = io_source(e) {
        return io_err(what, path, io);
    }
    let text = with_path(format!("{what}: {e}"), path);
    match e.class() {
        Class::Layout => LayoutMismatch::new_err(text),
        Class::Argument => PyValueError::new_err(text),
        Class::Index => PyIndexError::new_err(text),
        Class::Key => PyKeyError::new_err(text),
        Class::Timeout => PyTimeoutError::new_err(text),
        Class::Unsupported => UnsupportedOperation::new_err(text),
        Class::State => PyOSError::new_err(text),
        Class::NotAFrame => NotAFrame::new_err(text),
    }
}

/// The operating system's error under a SubEtha one, found by following
/// `source()` down from the error itself.
fn io_source<'a>(e: &'a (dyn std::error::Error + 'static)) -> Option<&'a std::io::Error> {
    let mut next = Some(e);
    while let Some(err) = next {
        if let Some(io) = err.downcast_ref::<std::io::Error>() {
            return Some(io);
        }
        next = err.source();
    }
    None
}

/// An operating-system failure, raised the way Python raises its own:
/// `OSError` with the code, which CPython turns into the subclass that
/// code names (`FileNotFoundError`, `PermissionError` and the rest), and
/// the path the call opened as `filename`. On Windows the code is a
/// Windows one, so it goes in as `winerror` and CPython derives `errno`
/// from it. A failure SubEtha detected itself has no code: a file of
/// another shape, which carries a [`Mismatch`], raises
/// [`LayoutMismatch`]; any other raises the subclass its kind names, with
/// the path in the message, and a refused argument raises `ValueError`,
/// as Python's own calls do for a name holding a NUL.
fn io_err(what: &str, path: Option<&Path>, io: &std::io::Error) -> PyErr {
    if Mismatch::carried_by(io).is_some() {
        return LayoutMismatch::new_err(with_path(format!("{what}: {io}"), path));
    }
    let Some(code) = io.raw_os_error() else {
        let text = with_path(format!("{what}: {io}"), path);
        return match io.kind() {
            ErrorKind::InvalidInput => PyValueError::new_err(text),
            ErrorKind::NotFound => PyFileNotFoundError::new_err(text),
            ErrorKind::PermissionDenied => PyPermissionError::new_err(text),
            ErrorKind::AlreadyExists => PyFileExistsError::new_err(text),
            ErrorKind::TimedOut => PyTimeoutError::new_err(text),
            ErrorKind::WouldBlock => PyBlockingIOError::new_err(text),
            ErrorKind::Interrupted => PyInterruptedError::new_err(text),
            _ => PyOSError::new_err(text),
        };
    };
    let text = format!("{what}: {}", os_text(io, code));
    let filename = path.map(|p| p.as_os_str().to_os_string());
    #[cfg(windows)]
    let args = (code, text, filename, code);
    #[cfg(not(windows))]
    let args = (code, text, filename);
    PyOSError::new_err(args)
}

/// The system's text for a failure, without the " (os error N)" the
/// standard library appends or the full stop that ends a Windows message:
/// Python prints the code itself, as `[Errno N]` or `[WinError N]`, and
/// trims that stop from its own Windows messages.
fn os_text(io: &std::io::Error, code: i32) -> String {
    let text = io.to_string();
    let bare = text.strip_suffix(&format!(" (os error {code})")).unwrap_or(&text);
    bare.trim_end_matches('.').to_string()
}

/// A message ending in the path the call opened, set out as Python ends
/// an `OSError`'s: `message: 'path'`.
fn with_path(text: String, path: Option<&Path>) -> String {
    match path {
        Some(p) => format!("{text}: '{}'", p.display()),
        None => text,
    }
}

impl Classify for std::io::Error {
    fn class(&self) -> Class {
        Class::State
    }
}

impl Classify for MarshalError {
    fn class(&self) -> Class {
        match self {
            Self::ShortBuffer { .. } | Self::InvalidEncoding => Class::Argument,
        }
    }
}

impl Classify for SharedAtomicError {
    fn class(&self) -> Class {
        match self {
            Self::LayoutMismatch(_) => Class::Layout,
            Self::IoError(_) => Class::State,
        }
    }
}

impl Classify for RegionError {
    fn class(&self) -> Class {
        match self {
            Self::InvalidPtr | Self::PayloadTooLarge | Self::InvalidArgument(_) => Class::Argument,
            Self::LayoutMismatch(_) => Class::Layout,
            Self::Full | Self::IoError(_) => Class::State,
        }
    }
}

impl Classify for RingError {
    fn class(&self) -> Class {
        match self {
            Self::PayloadTooLarge | Self::InvalidArgument(_) => Class::Argument,
            Self::LayoutMismatch(_) | Self::UnknownStampKind(_) => Class::Layout,
            Self::NotStamped | Self::Stamped => Class::Unsupported,
            Self::Full | Self::Empty | Self::NotDrainer | Self::StaleBacklog | Self::IoError(_) => {
                Class::State
            }
            Self::NotAFrame => Class::NotAFrame,
        }
    }
}

impl Classify for BroadcastError {
    fn class(&self) -> Class {
        match self {
            Self::InvalidConsumer | Self::PayloadTooLarge => Class::Argument,
            Self::LayoutMismatch(_) => Class::Layout,
            Self::Full | Self::Empty | Self::NoConsumerSlot | Self::IoError(_) => Class::State,
        }
    }
}

impl Classify for SharedCellError {
    fn class(&self) -> Class {
        match self {
            Self::PayloadTooLarge => Class::Argument,
            Self::LayoutMismatch(_) => Class::Layout,
            Self::NotInitialized | Self::IoError(_) => Class::State,
        }
    }
}

impl Classify for VecError {
    fn class(&self) -> Class {
        match self {
            Self::PayloadTooLarge | Self::InvalidArgument(_) => Class::Argument,
            Self::OutOfBounds => Class::Index,
            Self::LayoutMismatch(_) => Class::Layout,
            Self::ReadOnly => Class::Unsupported,
            Self::Full | Self::IoError(_) => Class::State,
        }
    }
}

impl Classify for NotifyError {
    fn class(&self) -> Class {
        match self {
            Self::LayoutMismatch(_) => Class::Layout,
            Self::Io(_) => Class::State,
        }
    }
}

impl Classify for BlockingError {
    fn class(&self) -> Class {
        match self {
            Self::Ring(e) => e.class(),
            Self::Notify(e) => e.class(),
            Self::WakerLayout(_) => Class::Layout,
            Self::Timeout => Class::Timeout,
            Self::WakerFull | Self::Closed | Self::Io(_) => Class::State,
        }
    }
}

impl<E: Classify> Classify for BlockingWaitError<E> {
    fn class(&self) -> Class {
        match self {
            Self::Op(e) => e.class(),
            Self::WakerLayout(_) => Class::Layout,
            Self::Timeout => Class::Timeout,
            Self::WakerFull | Self::Closed | Self::Io(_) => Class::State,
        }
    }
}

impl Classify for ArcError {
    fn class(&self) -> Class {
        match self {
            Self::LayoutMismatch(_) => Class::Layout,
            Self::OutOfRange { .. } => Class::Index,
            Self::HoldersExhausted | Self::IoError(_) => Class::State,
        }
    }
}

impl Classify for LeaderError {
    fn class(&self) -> Class {
        match self {
            Self::LayoutMismatch(_) => Class::Layout,
            Self::IoError(_) => Class::State,
        }
    }
}

impl Classify for SharedHolderError {
    fn class(&self) -> Class {
        match self {
            Self::EmptyCapacity => Class::Argument,
            Self::LayoutMismatch(_) => Class::Layout,
            Self::IoError(_) => Class::State,
        }
    }
}

impl Classify for HeartbeatError {
    fn class(&self) -> Class {
        match self {
            Self::LayoutMismatch(_) => Class::Layout,
            Self::TableFull | Self::IoError(_) => Class::State,
        }
    }
}

impl Classify for BarrierError {
    fn class(&self) -> Class {
        match self {
            Self::Atomic(e) => e.class(),
            Self::EpochTooFarBehind => Class::Argument,
            Self::Timeout => Class::Timeout,
            Self::NoLivePeers => Class::State,
        }
    }
}

impl Classify for CondvarError {
    fn class(&self) -> Class {
        match self {
            Self::LayoutMismatch(_) => Class::Layout,
            Self::Timeout => Class::Timeout,
            Self::WakerFull | Self::Io(_) => Class::State,
        }
    }
}

impl Classify for SharedOnceError {
    fn class(&self) -> Class {
        match self {
            Self::PayloadTooLarge => Class::Argument,
            Self::LayoutMismatch(_) => Class::Layout,
            Self::IoError(_) => Class::State,
        }
    }
}

impl Classify for FenceClockError {
    fn class(&self) -> Class {
        match self {
            Self::InvalidSlot => Class::Argument,
            Self::LayoutMismatch(_) => Class::Layout,
            Self::Full | Self::IoError(_) => Class::State,
        }
    }
}

impl Classify for BloomError {
    fn class(&self) -> Class {
        match self {
            Self::BitVec(e) => e.class(),
            Self::InvalidConfig => Class::Argument,
            Self::LayoutMismatch(_) => Class::Layout,
            Self::IoError(_) => Class::State,
        }
    }
}

impl Classify for CapacityMorphError {
    fn class(&self) -> Class {
        match self {
            Self::Ring(e) => e.class(),
            Self::Adaptive(e) => e.class(),
            Self::InvalidCapacity => Class::Argument,
            Self::CannotShrinkInFlight { .. } => Class::State,
        }
    }
}

impl Classify for AdaptiveError {
    fn class(&self) -> Class {
        match self {
            Self::GrowthFailed(e) => e.class(),
            Self::TooManyProducers | Self::TooManyConsumers => Class::State,
        }
    }
}

impl Classify for EpochError {
    fn class(&self) -> Class {
        match self {
            Self::LayoutMismatch(_) => Class::Layout,
            Self::PinsExhausted | Self::TicketsExhausted | Self::IoError(_) => Class::State,
        }
    }
}

impl Classify for RawLruError {
    fn class(&self) -> Class {
        match self {
            Self::WrongSize { .. } | Self::ZeroCapacity => Class::Argument,
            Self::LayoutMismatch(_) => Class::Layout,
            Self::IoError(_) | Self::Full => Class::State,
        }
    }
}

impl Classify for HLLError {
    fn class(&self) -> Class {
        match self {
            Self::InvalidPrecision => Class::Argument,
            Self::LayoutMismatch(_) => Class::Layout,
            Self::IoError(_) => Class::State,
        }
    }
}

impl Classify for CMSError {
    fn class(&self) -> Class {
        match self {
            Self::InvalidConfig => Class::Argument,
            Self::LayoutMismatch(_) => Class::Layout,
            Self::IoError(_) => Class::State,
        }
    }
}

impl Classify for BitVecError {
    fn class(&self) -> Class {
        match self {
            Self::OutOfBounds => Class::Index,
            Self::LayoutMismatch(_) => Class::Layout,
            Self::IoError(_) => Class::State,
        }
    }
}

impl Classify for HistogramError {
    fn class(&self) -> Class {
        match self {
            Self::EmptyBoundaries | Self::NonMonotonicBoundaries => Class::Argument,
            Self::OutOfBounds => Class::Index,
            Self::LayoutMismatch(_) => Class::Layout,
            Self::IoError(_) => Class::State,
        }
    }
}

impl Classify for RateLimiterError {
    fn class(&self) -> Class {
        match self {
            Self::InvalidConfig => Class::Argument,
            Self::LayoutMismatch(_) => Class::Layout,
            Self::Timeout => Class::Timeout,
            Self::InsufficientTokens { .. } | Self::IoError(_) => Class::State,
        }
    }
}

impl Classify for StackError {
    fn class(&self) -> Class {
        match self {
            Self::PayloadTooLarge => Class::Argument,
            Self::LayoutMismatch(_) => Class::Layout,
            Self::Full | Self::IoError(_) => Class::State,
        }
    }
}

impl Classify for DequeError {
    fn class(&self) -> Class {
        match self {
            Self::InvalidCapacity | Self::Marshal(_) => Class::Argument,
            Self::LayoutMismatch(_) => Class::Layout,
            Self::Full | Self::Io(_) => Class::State,
        }
    }
}

impl Classify for ReservoirError {
    fn class(&self) -> Class {
        match self {
            Self::PayloadTooLarge => Class::Argument,
            Self::LayoutMismatch(_) => Class::Layout,
            Self::IoError(_) => Class::State,
        }
    }
}

impl Classify for BlockedBloomError {
    fn class(&self) -> Class {
        match self {
            Self::InvalidConfig => Class::Argument,
            Self::LayoutMismatch(_) => Class::Layout,
            Self::IoError(_) => Class::State,
        }
    }
}

impl Classify for HandleTableError {
    fn class(&self) -> Class {
        match self {
            Self::PayloadTooLarge => Class::Argument,
            Self::LayoutMismatch(_) => Class::Layout,
            Self::Full | Self::IoError(_) => Class::State,
        }
    }
}

impl Classify for TileError {
    fn class(&self) -> Class {
        match self {
            Self::PayloadTooLarge => Class::Argument,
            Self::LayoutMismatch(_) => Class::Layout,
            Self::Full | Self::IoError(_) => Class::State,
        }
    }
}

impl Classify for ChainError {
    fn class(&self) -> Class {
        match self {
            Self::PayloadTooLarge | Self::NonMonotonicVersion => Class::Argument,
            Self::LayoutMismatch(_) => Class::Layout,
            Self::Full | Self::IoError(_) => Class::State,
        }
    }
}

impl Classify for VersionedSlabError {
    fn class(&self) -> Class {
        match self {
            Self::Slab(e) => e.class(),
            Self::Epochs(e) => e.class(),
            Self::Pinned => Class::State,
        }
    }
}

impl Classify for VersionedError {
    fn class(&self) -> Class {
        match self {
            Self::Epochs(e) => e.class(),
            Self::InvalidConfig => Class::Argument,
            Self::LayoutMismatch(_) => Class::Layout,
            Self::RebornUnderPin | Self::Full | Self::IoError(_) => Class::State,
        }
    }
}

impl Classify for IndexError {
    fn class(&self) -> Class {
        match self {
            Self::Epochs(e) => e.class(),
            Self::InvalidConfig(_) => Class::Argument,
            Self::LayoutMismatch(_) => Class::Layout,
            Self::Conflict { .. } | Self::Full | Self::IoError(_) => Class::State,
        }
    }
}

impl Classify for TopologyError {
    fn class(&self) -> Class {
        match self {
            Self::NodeIndexOutOfBounds => Class::Index,
            Self::LayoutMismatch(_) => Class::Layout,
            Self::IoError(_) => Class::State,
        }
    }
}

impl Classify for GraphError {
    fn class(&self) -> Class {
        match self {
            Self::Region(e) => e.class(),
            Self::InvalidNode | Self::InvalidEdge => Class::Argument,
            Self::IoError(_) => Class::State,
        }
    }
}

impl Classify for UniversalError {
    fn class(&self) -> Class {
        match self {
            // The state word read from the file names no strategy.
            Self::InvalidStrategy | Self::LayoutMismatch(_) => Class::Layout,
            Self::Full | Self::VersionExhausted | Self::MigrationRaced { .. } | Self::IoError(_) => {
                Class::State
            }
        }
    }
}

impl Classify for ArenaError {
    fn class(&self) -> Class {
        match self {
            Self::InvalidRef | Self::InvalidUtf8 | Self::InvalidArgument(_) => Class::Argument,
            Self::LayoutMismatch(_) => Class::Layout,
            Self::ReadOnly => Class::Unsupported,
            Self::Full | Self::IoError(_) => Class::State,
        }
    }
}

impl Classify for LinkedListError {
    fn class(&self) -> Class {
        match self {
            Self::Region(e) => e.class(),
            Self::InvalidHandle => Class::Argument,
            Self::LayoutMismatch(_) => Class::Layout,
            Self::IoError(_) => Class::State,
        }
    }
}

impl Classify for SlabError {
    fn class(&self) -> Class {
        match self {
            Self::PayloadTooLarge | Self::InvalidArgument(_) => Class::Argument,
            Self::OutOfBounds => Class::Index,
            Self::LayoutMismatch(_) => Class::Layout,
            Self::ReadOnly => Class::Unsupported,
            Self::IoError(_) => Class::State,
        }
    }
}

impl Classify for BTreeError {
    fn class(&self) -> Class {
        match self {
            Self::InvalidConfig => Class::Argument,
            Self::LayoutMismatch(_) => Class::Layout,
            Self::Full | Self::IoError(_) => Class::State,
        }
    }
}

impl Classify for MapError {
    fn class(&self) -> Class {
        match self {
            Self::PayloadTooLarge => Class::Argument,
            Self::KeyAbsent => Class::Key,
            Self::LayoutMismatch(_) => Class::Layout,
            Self::Full | Self::IoError(_) => Class::State,
        }
    }
}

impl Classify for WakerError {
    fn class(&self) -> Class {
        match self {
            Self::LayoutMismatch(_) => Class::Layout,
            Self::Timeout => Class::Timeout,
            Self::Full | Self::IoError(_) => Class::State,
        }
    }
}

impl Classify for LanedError {
    fn class(&self) -> Class {
        match self {
            Self::Versioned(e) => e.class(),
            Self::Epochs(e) => e.class(),
            Self::KeyInAnotherLane(_) => Class::Argument,
            Self::LaneOutOfRange { .. } => Class::Index,
            Self::KeyAbsent => Class::Key,
            Self::LayoutMismatch(_) => Class::Layout,
            Self::NoFreeLane | Self::LaneBusy(_) | Self::Io(_) => Class::State,
        }
    }
}

impl Classify for RawTowerError {
    fn class(&self) -> Class {
        match self {
            Self::Region(e) => e.class(),
            Self::WrongSize { .. } | Self::ZeroDepth | Self::NilAtLevel(_) | Self::BrokenAtLevel(_) => {
                Class::Argument
            }
        }
    }
}

impl Classify for TransportError {
    fn class(&self) -> Class {
        match self {
            Self::PayloadTooLarge | Self::OutBufferTooSmall => Class::Argument,
            Self::Full | Self::Empty | Self::Other => Class::State,
        }
    }
}

impl Classify for ApiError {
    fn class(&self) -> Class {
        match self {
            Self::Transport(e) => e.class(),
            Self::Map(e) => e.class(),
            Self::Marshal(_) | Self::WrongFamily { .. } | Self::PayloadTooLarge => Class::Argument,
            Self::Timeout => Class::Timeout,
            Self::Io(_) => Class::State,
        }
    }
}

#[cfg(feature = "tcp-bridge")]
impl Classify for subetha_cxc::tcp_bridge::TcpBridgeError {
    fn class(&self) -> Class {
        match self {
            Self::Io(_) | Self::Closed => Class::State,
        }
    }
}

#[cfg(feature = "quic-bridge")]
impl Classify for subetha_cxc::quic_bridge::QuicBridgeError {
    fn class(&self) -> Class {
        match self {
            Self::Tls(_) | Self::Quic(_) | Self::Io(_) => Class::State,
        }
    }
}
