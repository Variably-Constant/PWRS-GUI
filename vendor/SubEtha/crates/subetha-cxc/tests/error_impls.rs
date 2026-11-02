//! Every public error type in `subetha-cxc` implements `std::error::Error`,
//! is `Send + Sync + 'static` so `?` carries it into `Box<dyn Error>` or
//! `anyhow`, formats to a message, and returns the error it wraps from
//! `source()`.

use std::error::Error;

use subetha_cxc as cxc;

/// Formats `e`, failing on an empty message, which a reporter would print
/// as nothing.
fn message<E: Error + Send + Sync + 'static>(e: E) -> String {
    let text = e.to_string();
    assert!(!text.is_empty(), "{e:?} formats to an empty message");
    text
}

/// A disagreement as an open reports one.
fn mismatch() -> cxc::layout::Mismatch {
    cxc::layout::Mismatch::new("capacity", 16, 32)
}

/// The messages of `e` and every error under it, outermost first.
fn chain(e: &(dyn Error + 'static)) -> Vec<String> {
    let mut messages = vec![e.to_string()];
    let mut next = e.source();
    while let Some(inner) = next {
        messages.push(inner.to_string());
        next = inner.source();
    }
    messages
}

#[test]
fn every_error_type_formats() {
    message(cxc::adaptive_ring::AdaptiveError::TooManyProducers);
    message(cxc::adaptive_ring::LastHolderError::NoBackingFiles);
    message(cxc::api::ApiError::Timeout);
    message(cxc::blocking_spsc_ring::BlockingError::WakerFull);
    message(cxc::capacity_adaptive_ring::CapacityMorphError::InvalidCapacity);
    message(cxc::capacity_broadcast_ring::BroadcastCapacityMorphError::InvalidCapacity);
    message(cxc::capacity_pubsub_ring::PubSubCapacityMorphError::InvalidCapacity);
    message(cxc::cross_process_notifier::NotifyError::LayoutMismatch(mismatch()));
    message(cxc::cross_process_waker::WakerError::Full);
    message(cxc::dispatch_deque::DispatchError::PushFailed("the ring is full"));
    message(cxc::epoch_barrier::BarrierError::NoLivePeers);
    message(cxc::event_state_log::EventLogError::EventTooLarge);
    message(cxc::fec::FecError::BadParams);
    message(cxc::heartbeat::HeartbeatError::TableFull);
    message(cxc::k_tower_cascade::CascadeError::NilAtLevel(1));
    message(cxc::laned_versioned_map::LanedError::NoFreeLane);
    message(cxc::lazy_config::LazyConfigError::Once(
        cxc::shared_once_cell::SharedOnceError::LayoutMismatch(mismatch()),
    ));
    message(cxc::message_transport::TransportError::Other);
    message(cxc::pass_registry::PassError::UnknownClosureId(7));
    message(cxc::priority_fanout::FanoutError::NPrioritiesOutOfBounds);
    message(cxc::progress_task::ProgressTaskError::Atomic(
        cxc::shared_atomic::SharedAtomicError::LayoutMismatch(mismatch()),
    ));
    message(cxc::protocol_pubsub::PubSubReadError::Lost);
    message(cxc::raw_arena::RawArenaError::Exhausted);
    message(cxc::raw_graph::RawGraphError::WrongSize { expected: 8, found: 4 });
    message(cxc::raw_k_tower::RawTowerError::BrokenAtLevel(2));
    message(cxc::raw_lru_cache::RawLruError::ZeroCapacity);
    message(cxc::raw_time_point::RawTileError::BadPayloadSize { asked: 0, max: 56 });
    message(cxc::raw_umbra_pointer::RawUmbraError::ExtTooLong { found: 9 });
    message(cxc::raw_universal::RawUniversalError::MigrationRaced { published: 0x11 });
    message(cxc::ring_holders::HoldersError::Exhausted);
    message(cxc::scheduler::SchedError::ResultTooLarge);
    message(cxc::shared_arc::ArcError::HoldersExhausted);
    message(cxc::shared_arc::ArcError::OutOfRange { offset: 3, len: 2, value_bytes: 4 });
    message(cxc::shared_array::ArrayError::OutOfBounds { index: 9, len: 4 });
    message(cxc::shared_async_pointer::SharedAsyncError::AllWorkersDied);
    message(cxc::shared_atomic::SharedAtomicError::LayoutMismatch(mismatch()));
    message(cxc::shared_bit_vec::BitVecError::OutOfBounds);
    message(cxc::shared_blocked_bloom_filter::BlockedBloomError::InvalidConfig);
    message(cxc::shared_bloom_filter::BloomError::InvalidConfig);
    message(cxc::shared_broadcast_ring::BroadcastError::NoConsumerSlot);
    message(cxc::shared_btree_map::BTreeError::Full);
    message(cxc::shared_cell::SharedCellError::NotInitialized);
    message(cxc::shared_condvar::CondvarError::Timeout);
    message(cxc::shared_count_min_sketch::CMSError::InvalidConfig);
    message(cxc::shared_deque::DequeError::LayoutMismatch(mismatch()));
    message(cxc::shared_deque_khl::PushError::Full);
    message(cxc::shared_deque_khpd::PushError::PayloadTooLarge);
    message(cxc::shared_deque_loh::PushError::LifoFull);
    message(cxc::shared_deque_urd::PublishError::BadTarget(3));
    message(cxc::shared_epochs::EpochError::TicketsExhausted);
    message(cxc::shared_fence_clock::FenceClockError::Full);
    message(cxc::shared_graph::GraphError::InvalidEdge);
    message(cxc::shared_handle_table::HandleTableError::Full);
    message(cxc::shared_hash_map::MapError::KeyAbsent);
    message(cxc::shared_histogram::HistogramError::NonMonotonicBoundaries);
    message(cxc::shared_holder_table::SharedHolderError::EmptyCapacity);
    message(cxc::shared_hyper_log_log::HLLError::InvalidPrecision);
    message(cxc::shared_leader_election::LeaderError::LayoutMismatch(mismatch()));
    message(cxc::shared_linked_list::LinkedListError::InvalidHandle);
    message(cxc::shared_lru_cache::LRUError::LayoutMismatch(mismatch()));
    message(cxc::shared_named_values::NamedValuesError::Corrupt);
    message(cxc::shared_once_cell::SharedOnceError::PayloadTooLarge);
    message(cxc::shared_rate_limiter::RateLimiterError::InsufficientTokens {
        available: 1,
        requested: 2,
    });
    message(cxc::shared_region::RegionError::InvalidPtr);
    message(cxc::shared_reservoir_sampler::ReservoirError::PayloadTooLarge);
    message(cxc::shared_ring::RingError::NotDrainer);
    message(cxc::shared_slab::SlabError::ReadOnly);
    message(cxc::shared_string_arena::ArenaError::InvalidUtf8);
    message(cxc::shared_time_point::TileError::Full);
    message(cxc::shared_topology_map::TopologyError::NodeIndexOutOfBounds);
    message(cxc::shared_treiber_stack::StackError::Full);
    message(cxc::shared_universal::UniversalError::VersionExhausted);
    message(cxc::shared_universal::UniversalError::MigrationRaced { published: 0x21 });
    message(cxc::shared_vec::VecError::OutOfBounds);
    message(cxc::shared_versioned_chain::ChainError::NonMonotonicVersion);
    message(cxc::shared_versioned_slab::VersionedSlabError::Pinned);
    message(cxc::tagged_offset_ptr::TaggedPtrError::IndexOutOfRange);
    message(cxc::versioned_btree_map::VersionedError::RebornUnderPin);
    #[cfg(any(unix, windows))]
    message(cxc::protocol_direct_file::DirectFileError::Full);
    #[cfg(feature = "tcp-bridge")]
    message(cxc::tcp_bridge::TcpBridgeError::Closed);
    #[cfg(feature = "tcp-bridge")]
    message(cxc::blocking_tcp_bridge::BlockingTcpBridgeError::Closed);
    #[cfg(feature = "quic-bridge")]
    message(cxc::quic_bridge::QuicBridgeError::Quic("the connection closed".into()));
    #[cfg(feature = "residue-fec")]
    message(cxc::residue_fec::ResidueError::NotEnoughResidues);
}

#[test]
fn a_wrapped_error_is_printed_and_returned_as_the_source() {
    use cxc::progress_task::ProgressTaskError;
    use cxc::shared_atomic::SharedAtomicError;

    let io = std::io::Error::other("the mapping went away");
    let io_message = io.to_string();
    let atomic = SharedAtomicError::IoError(io);
    let atomic_message = atomic.to_string();
    let outer = ProgressTaskError::Atomic(atomic);

    let messages = chain(&outer);
    assert_eq!(messages.len(), 3, "three layers, each its own source: {messages:?}");
    assert_eq!(messages[1], atomic_message);
    assert_eq!(messages[2], io_message);
    assert!(messages[0].ends_with(&messages[1]), "the wrapper prints what it wraps: {messages:?}");
    assert!(messages[1].ends_with(&messages[2]), "the wrapper prints what it wraps: {messages:?}");
}

#[test]
fn a_layout_mismatch_names_the_field_and_both_values() {
    let e = cxc::shared_ring::RingError::LayoutMismatch(mismatch());
    assert_eq!(e.to_string(), "a ring of another shape: the file's capacity is 16 where 32 was asked for");
}

#[test]
fn an_api_error_carries_its_cause() {
    let map = cxc::shared_hash_map::MapError::KeyAbsent;
    let map_message = map.to_string();
    let api = cxc::api::ApiError::Map(map);
    let source = api.source().expect("ApiError::Map returns the map error");
    assert_eq!(source.to_string(), map_message);
    assert_eq!(api.to_string(), format!("map: {map_message}"));

    let io = cxc::api::ApiError::Io(std::io::Error::other("the disk went away"));
    let source = io.source().expect("ApiError::Io returns the io error");
    assert_eq!(source.to_string(), "the disk went away");

    assert!(cxc::api::ApiError::Timeout.source().is_none());
}

#[test]
fn a_universal_set_carries_the_operating_systems_error() {
    use cxc::shared_universal::{SharedUniversal, UniversalError};

    let missing = std::env::temp_dir()
        .join(format!("subetha-error-impls-universal-missing-{}", std::process::id()));
    let err = match SharedUniversal::<u64>::open(&missing, 16) {
        Ok(_) => panic!("a set opened at a path nothing created"),
        Err(e) => e,
    };
    let UniversalError::IoError(io) = &err else {
        panic!("a missing state file is an io failure: {err:?}");
    };
    assert_eq!(io.kind(), std::io::ErrorKind::NotFound);
    let source = err.source().expect("UniversalError::IoError returns the io error");
    assert_eq!(source.to_string(), io.to_string());
}

#[test]
fn an_io_failure_carries_the_operating_systems_error() {
    use cxc::shared_hash_map::{MapError, SharedHashMap};

    let missing = std::env::temp_dir()
        .join(format!("subetha-error-impls-missing-{}.bin", std::process::id()));
    let err = match SharedHashMap::<u32, u64>::open(&missing, 16) {
        Ok(_) => panic!("a map opened at a path nothing created"),
        Err(e) => e,
    };
    let MapError::IoError(io) = &err else {
        panic!("a missing file is an io failure: {err:?}");
    };
    assert_eq!(io.kind(), std::io::ErrorKind::NotFound);
    assert!(io.raw_os_error().is_some(), "the operating system's code survives: {io:?}");
    let source = err.source().expect("the io failure is the source");
    assert_eq!(source.to_string(), io.to_string());
    assert_eq!(err.to_string(), format!("io: {io}"));
}

#[test]
fn the_deque_notifier_and_growth_errors_carry_the_io_error() {
    use cxc::adaptive_ring::AdaptiveError;
    use cxc::cross_process_notifier::NotifyError;
    use cxc::shared_deque::{DequeError, SharedDeque};
    use cxc::shared_ring::RingError;

    let missing = std::env::temp_dir()
        .join(format!("subetha-error-impls-missing-deque-{}.bin", std::process::id()));
    let err = match SharedDeque::<u64>::open_as_thief(&missing) {
        Ok(_) => panic!("a deque opened at a path nothing created"),
        Err(e) => e,
    };
    let DequeError::Io(io) = &err else {
        panic!("a missing file is an io failure: {err:?}");
    };
    assert_eq!(io.kind(), std::io::ErrorKind::NotFound);
    assert!(io.raw_os_error().is_some(), "the operating system's code survives: {io:?}");
    let source = err.source().expect("the io failure is the source");
    assert_eq!(source.to_string(), io.to_string());

    let notify = NotifyError::from(std::io::Error::other("the pipe went away"));
    assert_eq!(chain(&notify), ["io: the pipe went away", "the pipe went away"]);

    let growth = AdaptiveError::GrowthFailed(RingError::IoError(std::io::Error::other("no space")));
    let messages = chain(&growth);
    assert_eq!(messages.len(), 3, "growth, the ring's error, the io error: {messages:?}");
    assert_eq!(messages[2], "no space");
    assert!(messages[0].ends_with(&messages[1]), "the wrapper prints what it wraps: {messages:?}");
}

#[test]
fn a_boxed_error_takes_any_of_them() {
    fn open() -> Result<(), Box<dyn Error + Send + Sync>> {
        Err(cxc::shared_ring::RingError::Full)?;
        Ok(())
    }
    let err = open().expect_err("the ring error propagates through ?");
    assert_eq!(err.to_string(), cxc::shared_ring::RingError::Full.to_string());
}
