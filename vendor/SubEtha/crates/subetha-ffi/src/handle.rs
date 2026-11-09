//! The handle table: a 64-bit handle names one live object through an index
//! and a generation, so a stale or forged handle is refused rather than
//! dereferenced.
//!
//! The table grows without bound; there is no capacity to run out of.
//! Slots live in chunks that are placed once and never freed or moved,
//! behind a directory of one pointer per chunk, so a reader reaches a
//! slot through one load and never locks.
//!
//! A call publishes the slot it is inside, in a word of its own thread's;
//! `crate::epoch` holds that protocol. `destroy` marks the slot closing,
//! interrupts any waiter, and waits out every call that began before it
//! did, so a call in flight on another thread can never see freed memory.
//! A panic caught inside a call poisons the slot: every later call on that
//! handle fails until it is destroyed.

use std::marker::PhantomData;
use std::ptr::NonNull;
use std::sync::atomic::{AtomicPtr, AtomicU32, AtomicU64, AtomicU8, Ordering};
use std::sync::Arc;

use subetha_core::SwapCellOption;
use subetha_cxc::claim::Claim;
use subetha_sidecar::{AdaptiveInstance, InstanceStats};

use crate::error::{
    fail, SUBETHA_E_HANDLE_POISONED, SUBETHA_E_INVALID_ARGUMENT, SUBETHA_E_INVALID_HANDLE, SUBETHA_E_NOT_SUPPORTED,
    SUBETHA_E_RING_FULL, SUBETHA_E_WRONG_KIND, SUBETHA_OK,
};

/// A handle: zero is never issued.
#[allow(non_camel_case_types)]
pub type subetha_handle = u64;

/// The handle no object has.
pub const SUBETHA_HANDLE_NONE: subetha_handle = 0;

/// The handle names an adaptive ring.
pub const SUBETHA_KIND_RING: u32 = 1;
/// The handle names a blocking single-producer single-consumer ring.
pub const SUBETHA_KIND_SPSC: u32 = 2;
/// The handle names one producer of a blocking MPSC pool.
pub const SUBETHA_KIND_MPSC_PRODUCER: u32 = 3;
/// The handle names the consumer of a blocking MPSC pool.
pub const SUBETHA_KIND_MPSC_CONSUMER: u32 = 4;
/// The handle names one producer of a blocking MPMC grid.
pub const SUBETHA_KIND_MPMC_PRODUCER: u32 = 5;
/// The handle names one consumer of a blocking MPMC grid.
pub const SUBETHA_KIND_MPMC_CONSUMER: u32 = 6;
/// The handle names a Vyukov MPMC ring.
pub const SUBETHA_KIND_VYUKOV: u32 = 7;
/// The handle names the producer side of a Lamport SPSC pair.
pub const SUBETHA_KIND_LAMPORT_PRODUCER: u32 = 8;
/// The handle names the consumer side of a Lamport SPSC pair.
pub const SUBETHA_KIND_LAMPORT_CONSUMER: u32 = 9;
/// The handle names a broadcast ring.
pub const SUBETHA_KIND_BROADCAST: u32 = 10;
/// The handle names a pub/sub ring.
pub const SUBETHA_KIND_PUBSUB: u32 = 11;
/// The handle names a subscriber of a pub/sub ring.
pub const SUBETHA_KIND_SUBSCRIBER: u32 = 12;
/// The handle names a capacity-adaptive ring.
pub const SUBETHA_KIND_CAPACITY_RING: u32 = 13;
/// The handle names a locale-adaptive ring.
pub const SUBETHA_KIND_LOCALE_RING: u32 = 14;
/// The handle names a capacity-adaptive broadcast ring.
pub const SUBETHA_KIND_CAPACITY_BROADCAST: u32 = 15;
/// The handle names a capacity-adaptive pub/sub ring.
pub const SUBETHA_KIND_CAPACITY_PUBSUB: u32 = 16;
/// The handle names a subscriber of a capacity-adaptive pub/sub ring.
pub const SUBETHA_KIND_CAPACITY_SUBSCRIBER: u32 = 17;
/// The handle names an exact-delivery receiver on a stamped ring.
pub const SUBETHA_KIND_ORDERED_RECEIVER: u32 = 18;
/// The handle names a shared Treiber stack.
pub const SUBETHA_KIND_STACK: u32 = 19;
/// The handle names a work-stealing deque, as its owner or as a thief.
pub const SUBETHA_KIND_DEQUE: u32 = 20;
/// The handle names a pollable notifier attached to a ring.
pub const SUBETHA_KIND_NOTIFIER: u32 = 21;
/// The handle names a shared hash map.
pub const SUBETHA_KIND_HASHMAP: u32 = 22;
/// The handle names a shared string arena.
pub const SUBETHA_KIND_ARENA: u32 = 23;
/// The handle names a shared vec.
pub const SUBETHA_KIND_VEC: u32 = 24;
/// The handle names a shared slab.
pub const SUBETHA_KIND_SLAB: u32 = 25;
/// The handle names a shared region.
pub const SUBETHA_KIND_REGION: u32 = 26;
/// The handle names a shared atomic of one of the three widths.
pub const SUBETHA_KIND_ATOMIC: u32 = 27;
/// The handle names a shared doubly-linked list.
pub const SUBETHA_KIND_LIST: u32 = 28;
/// The handle names a shared B-tree map.
pub const SUBETHA_KIND_BTREE: u32 = 29;
/// The handle names a shared cell.
pub const SUBETHA_KIND_CELL: u32 = 30;
/// The handle names a frame region of fixed-size blocks.
pub const SUBETHA_KIND_FRAME_REGION: u32 = 31;
/// The handle names an epoch table.
pub const SUBETHA_KIND_EPOCHS: u32 = 32;
/// Names no object: a pin held against an epoch table is a token, not a
/// handle. The number stays out of use, so a handle carrying it is
/// refused rather than resolved to another kind.
pub const SUBETHA_KIND_PIN: u32 = 33;
/// Names no object: an open ticket on an epoch table is a token, like a
/// pin. The number stays out of use.
pub const SUBETHA_KIND_TICKET: u32 = 34;
// Kinds 35 to 38 and 40 are out of use, and no new kind takes them.
/// The handle names a condition variable.
pub const SUBETHA_KIND_CONDVAR: u32 = 39;
/// The handle names a leader election.
pub const SUBETHA_KIND_LEADER: u32 = 41;
/// The handle names a heartbeat table.
pub const SUBETHA_KIND_HEARTBEAT: u32 = 42;
/// The handle names a holder table.
pub const SUBETHA_KIND_HOLDERS: u32 = 43;
/// The handle names a shared fence clock.
pub const SUBETHA_KIND_FENCE_CLOCK: u32 = 44;
/// The handle names an epoch barrier.
pub const SUBETHA_KIND_EPOCH_BARRIER: u32 = 45;
/// The handle names a shared value kept alive by its holders.
pub const SUBETHA_KIND_SHARED_ARC: u32 = 46;
/// The handle names a value produced once across every process.
pub const SUBETHA_KIND_LAZY_VALUE: u32 = 47;
/// The handle names a cross-process waker.
pub const SUBETHA_KIND_WAKER: u32 = 48;
/// The handle names one half of a TCP bridge.
pub const SUBETHA_KIND_TCP_BRIDGE: u32 = 49;
/// The handle names one half of a QUIC bridge.
pub const SUBETHA_KIND_QUIC_BRIDGE: u32 = 50;
/// The handle names one half of a Sens-O-Matic link.
pub const SUBETHA_KIND_SENS: u32 = 51;
/// The handle names a shared Bloom filter.
pub const SUBETHA_KIND_BLOOM: u32 = 52;
/// The handle names a shared count-min sketch.
pub const SUBETHA_KIND_CMS: u32 = 53;
/// The handle names a shared histogram.
pub const SUBETHA_KIND_HISTOGRAM: u32 = 54;
/// The handle names a shared rate limiter.
pub const SUBETHA_KIND_RATE_LIMITER: u32 = 55;
/// The handle names a shared blocked Bloom filter.
pub const SUBETHA_KIND_BLOCKED_BLOOM: u32 = 56;
/// The handle names a shared reservoir sampler.
pub const SUBETHA_KIND_RESERVOIR: u32 = 57;
/// The handle names a shared topology map.
pub const SUBETHA_KIND_TOPOLOGY: u32 = 58;
/// The handle names a shared versioned chain.
pub const SUBETHA_KIND_VERSIONED_CHAIN: u32 = 59;
/// The handle names a shared versioned slab.
pub const SUBETHA_KIND_VERSIONED_SLAB: u32 = 60;
/// The handle names a shared versioned map.
pub const SUBETHA_KIND_VERSIONED_MAP: u32 = 61;
/// The handle names a shared laned versioned map.
pub const SUBETHA_KIND_LANED_MAP: u32 = 62;
/// The handle names a virtual endpoint registry.
pub const SUBETHA_KIND_ENDPOINT_REGISTRY: u32 = 63;
/// The handle names a runtime-mutable quality-of-service policy.
pub const SUBETHA_KIND_QOS_POLICY: u32 = 64;
/// The handle names a shared bit vector.
pub const SUBETHA_KIND_BIT_VEC: u32 = 65;
/// The handle names a shared HyperLogLog.
pub const SUBETHA_KIND_HLL: u32 = 66;
/// The handle names a least-recently-used cache.
pub const SUBETHA_KIND_LRU_CACHE: u32 = 67;
/// The handle names a directed graph.
pub const SUBETHA_KIND_GRAPH: u32 = 68;
/// The handle names a versioned time-point tile.
pub const SUBETHA_KIND_TIME_POINT: u32 = 69;
/// The handle names a strategy-switching set.
pub const SUBETHA_KIND_UNIVERSAL: u32 = 70;
/// The handle names a cascade tower.
pub const SUBETHA_KIND_K_TOWER: u32 = 71;
/// The handle names a blocking bridge: a blocking SPSC ring carried
/// across a socket connection.
pub const SUBETHA_KIND_BLOCKING_TCP_BRIDGE: u32 = 72;
/// The handle names a front-door channel: a queue between processes of
/// items up to `SUBETHA_CHANNEL_ITEM_MAX` bytes.
pub const SUBETHA_KIND_CHANNEL: u32 = 73;
/// The handle names a front-door key-value map: 64-bit keys mapped to
/// 64-bit values.
pub const SUBETHA_KIND_KV_MAP: u32 = 74;
/// The handle names a front-door work queue, as its owner or as a thief.
pub const SUBETHA_KIND_WORK_QUEUE: u32 = 75;
/// The handle names a reorder window in this process's memory.
pub const SUBETHA_KIND_REORDER_WINDOW: u32 = 76;
/// The handle names a front-door adaptive queue.
pub const SUBETHA_KIND_ADAPTIVE_QUEUE: u32 = 77;
/// The handle names a handle table: values reached by a generation-checked
/// table handle rather than by a place.
pub const SUBETHA_KIND_HANDLE_TABLE: u32 = 78;
/// The handle names a link sensor telling wireless loss from congestion.
pub const SUBETHA_KIND_LINK_LOSS_KIND: u32 = 79;
/// The handle names a link sensor fitting runs of loss.
pub const SUBETHA_KIND_LINK_LOSS_BURSTS: u32 = 80;
/// The handle names a link sensor reading jitter, spacing and delay trend.
pub const SUBETHA_KIND_LINK_TIMING: u32 = 81;
/// The handle names a link sensor looking for two groups of round trips.
pub const SUBETHA_KIND_LINK_ROUND_TRIP_SHAPE: u32 = 82;
/// The handle names a link sensor looking for a beat in the delays.
pub const SUBETHA_KIND_LINK_PERIODICITY: u32 = 83;
/// The handle names a link sensor estimating capacity from probes.
pub const SUBETHA_KIND_LINK_CAPACITY: u32 = 84;
/// The handle names a link sensor forecasting the next interval's rate.
pub const SUBETHA_KIND_LINK_FORECAST: u32 = 85;
/// The handle names a link sensor watching the route for changes.
pub const SUBETHA_KIND_LINK_PATH_CHANGES: u32 = 86;
/// The handle names an adaptive object of the caller's own: a strategy tag
/// and a ring the caller records its operations into.
pub const SUBETHA_KIND_ADAPTIVE: u32 = 87;
/// The handle names a shared versioned index.
pub const SUBETHA_KIND_VERSIONED_INDEX: u32 = 88;

/// A slot's object has no sidecar registration.
const NOT_OBSERVED: u64 = u64::MAX;
/// A call is making a slot's object's sidecar registration right now.
const CLAIMING: u64 = u64::MAX - 1;

const STATE_FREE: u8 = 0;
const STATE_LIVE: u8 = 1;
const STATE_POISONED: u8 = 2;
const STATE_CLOSING: u8 = 3;

/// Everything a handle can name.
pub(crate) enum Object {
    Ring(crate::ring::RingObject),
    Spsc(crate::spsc::SpscObject),
    MpscProducer(crate::mpsc::MpscProducerObject),
    MpscConsumer(crate::mpsc::MpscConsumerObject),
    MpmcProducer(crate::mpmc::MpmcProducerObject),
    MpmcConsumer(crate::mpmc::MpmcConsumerObject),
    Vyukov(crate::vyukov::VyukovObject),
    LamportProducer(crate::lamport::LamportProducerObject),
    LamportConsumer(crate::lamport::LamportConsumerObject),
    Broadcast(crate::broadcast::BroadcastObject),
    PubSub(crate::pubsub::PubSubObject),
    Subscriber(crate::pubsub::SubscriberObject),
    Capacity(crate::capacity::CapacityObject),
    LocaleRing(crate::locale::LocaleObject),
    EndpointRegistry(crate::virtual_endpoint::EndpointRegistryObject),
    QosPolicy(crate::qos_policy::QosPolicyObject),
    Lru(crate::lru_cache::LruObject),
    Graph(crate::graph::GraphObject),
    Tile(crate::time_point::TileObject),
    Universal(crate::universal::UniversalObject),
    Tower(crate::k_tower::TowerObject),
    BitVec(crate::bit_vec::BitVecObject),
    Hll(crate::hyper_log_log::HllObject),
    CapacityBroadcast(crate::capacity_broadcast::CapacityBroadcastObject),
    CapacityPubSub(crate::capacity_pubsub::CapacityPubSubObject),
    CapacitySubscriber(crate::capacity_pubsub::CapacitySubscriberObject),
    Ordered(crate::ordered::OrderedObject),
    Stack(crate::stack::StackObject),
    Deque(crate::deque::DequeObject),
    Notifier(crate::notifier::NotifierObject),
    HashMap(crate::hashmap::HashMapObject),
    Arena(crate::arena::ArenaObject),
    Vec(crate::vec::VecObject),
    Slab(crate::slab::SlabObject),
    Region(crate::region::RegionObject),
    Atomic(crate::atomic::AtomicObject),
    List(crate::list::ListObject),
    BTree(crate::btree::BTreeObject),
    Cell(crate::cell::CellObject),
    FrameRegion(crate::frame_region::FrameRegionObject),
    Epochs(crate::epochs::EpochsObject),
    Condvar(crate::condvar::CondvarObject),
    Leader(crate::leader::LeaderObject),
    Heartbeat(crate::heartbeat::HeartbeatObject),
    Holders(crate::holders::HoldersObject),
    FenceClock(crate::fence_clock::FenceClockObject),
    EpochBarrier(crate::epoch_barrier::EpochBarrierObject),
    SharedArc(crate::shared_arc::SharedArcObject),
    LazyValue(crate::lazy_value::LazyValueObject),
    Waker(crate::waker::WakerObject),
    /// In a build without the transports the type this carries holds an
    /// `Infallible`, so the variant cannot be constructed. It stays in the
    /// enum regardless, so `kind` and the borrow path have one shape in
    /// every build and a handle means the same thing either way.
    #[cfg_attr(not(feature = "tcp-bridge"), allow(dead_code))]
    TcpBridge(crate::tcp_bridge::TcpBridgeObject),
    /// As with `TcpBridge`, unconstructable without its feature and
    /// present in every build so a handle means the same thing either way.
    #[cfg_attr(not(feature = "quic-bridge"), allow(dead_code))]
    QuicBridge(crate::quic_bridge::QuicBridgeObject),
    /// As with `TcpBridge`: the blocking bridge rides the same feature and
    /// is unconstructable without it.
    #[cfg_attr(not(feature = "tcp-bridge"), allow(dead_code))]
    BlockingTcpBridge(crate::blocking_tcp_bridge::BlockingTcpBridgeObject),
    Sens(crate::sens::SensObject),
    Bloom(crate::bloom::BloomObject),
    Cms(crate::cms::CmsObject),
    Histogram(crate::histogram::HistogramObject),
    RateLimiter(crate::rate_limiter::RateLimiterObject),
    BlockedBloom(crate::blocked_bloom::BlockedBloomObject),
    Reservoir(crate::reservoir::ReservoirObject),
    Topology(crate::topology::TopologyObject),
    VersionedChain(crate::versioned_chain::VersionedChainObject),
    VersionedSlab(crate::versioned_slab::VersionedSlabObject),
    VersionedMap(crate::versioned_map::VersionedMapObject),
    LanedMap(crate::laned_map::LanedMapObject),
    Channel(crate::channel::ChannelObject),
    KvMap(crate::kv_map::KvMapObject),
    WorkQueue(crate::work_queue::WorkQueueObject),
    ReorderWindow(crate::reorder_window::ReorderWindowObject),
    AdaptiveQueue(crate::adaptive_queue::AdaptiveQueueObject),
    HandleTable(crate::handle_table::HandleTableObject),
    /// Any of the eight link sensors; the object carries which.
    LinkSensor(crate::link_sensor::LinkSensorObject),
    Adaptive(crate::adaptive::AdaptiveObject),
    VersionedIndex(crate::versioned_index::VersionedIndexObject),
}

impl Object {
    pub(crate) fn kind(&self) -> u32 {
        match self {
            Object::Ring(_) => SUBETHA_KIND_RING,
            Object::Spsc(_) => SUBETHA_KIND_SPSC,
            Object::MpscProducer(_) => SUBETHA_KIND_MPSC_PRODUCER,
            Object::MpscConsumer(_) => SUBETHA_KIND_MPSC_CONSUMER,
            Object::MpmcProducer(_) => SUBETHA_KIND_MPMC_PRODUCER,
            Object::MpmcConsumer(_) => SUBETHA_KIND_MPMC_CONSUMER,
            Object::Vyukov(_) => SUBETHA_KIND_VYUKOV,
            Object::LamportProducer(_) => SUBETHA_KIND_LAMPORT_PRODUCER,
            Object::LamportConsumer(_) => SUBETHA_KIND_LAMPORT_CONSUMER,
            Object::Broadcast(_) => SUBETHA_KIND_BROADCAST,
            Object::PubSub(_) => SUBETHA_KIND_PUBSUB,
            Object::Subscriber(_) => SUBETHA_KIND_SUBSCRIBER,
            Object::Capacity(_) => SUBETHA_KIND_CAPACITY_RING,
            Object::LocaleRing(_) => SUBETHA_KIND_LOCALE_RING,
            Object::EndpointRegistry(_) => SUBETHA_KIND_ENDPOINT_REGISTRY,
            Object::QosPolicy(_) => SUBETHA_KIND_QOS_POLICY,
            Object::Lru(_) => SUBETHA_KIND_LRU_CACHE,
            Object::Graph(_) => SUBETHA_KIND_GRAPH,
            Object::Tile(_) => SUBETHA_KIND_TIME_POINT,
            Object::Universal(_) => SUBETHA_KIND_UNIVERSAL,
            Object::Tower(_) => SUBETHA_KIND_K_TOWER,
            Object::BitVec(_) => SUBETHA_KIND_BIT_VEC,
            Object::Hll(_) => SUBETHA_KIND_HLL,
            Object::CapacityBroadcast(_) => SUBETHA_KIND_CAPACITY_BROADCAST,
            Object::CapacityPubSub(_) => SUBETHA_KIND_CAPACITY_PUBSUB,
            Object::CapacitySubscriber(_) => SUBETHA_KIND_CAPACITY_SUBSCRIBER,
            Object::Ordered(_) => SUBETHA_KIND_ORDERED_RECEIVER,
            Object::Stack(_) => SUBETHA_KIND_STACK,
            Object::Deque(_) => SUBETHA_KIND_DEQUE,
            Object::Notifier(_) => SUBETHA_KIND_NOTIFIER,
            Object::HashMap(_) => SUBETHA_KIND_HASHMAP,
            Object::Arena(_) => SUBETHA_KIND_ARENA,
            Object::Vec(_) => SUBETHA_KIND_VEC,
            Object::Slab(_) => SUBETHA_KIND_SLAB,
            Object::Region(_) => SUBETHA_KIND_REGION,
            Object::Atomic(_) => SUBETHA_KIND_ATOMIC,
            Object::List(_) => SUBETHA_KIND_LIST,
            Object::BTree(_) => SUBETHA_KIND_BTREE,
            Object::Cell(_) => SUBETHA_KIND_CELL,
            Object::FrameRegion(_) => SUBETHA_KIND_FRAME_REGION,
            Object::Epochs(_) => SUBETHA_KIND_EPOCHS,
            Object::Condvar(_) => SUBETHA_KIND_CONDVAR,
            Object::Leader(_) => SUBETHA_KIND_LEADER,
            Object::Heartbeat(_) => SUBETHA_KIND_HEARTBEAT,
            Object::Holders(_) => SUBETHA_KIND_HOLDERS,
            Object::FenceClock(_) => SUBETHA_KIND_FENCE_CLOCK,
            Object::EpochBarrier(_) => SUBETHA_KIND_EPOCH_BARRIER,
            Object::SharedArc(_) => SUBETHA_KIND_SHARED_ARC,
            Object::LazyValue(_) => SUBETHA_KIND_LAZY_VALUE,
            Object::Waker(_) => SUBETHA_KIND_WAKER,
            Object::TcpBridge(_) => SUBETHA_KIND_TCP_BRIDGE,
            Object::QuicBridge(_) => SUBETHA_KIND_QUIC_BRIDGE,
            Object::BlockingTcpBridge(_) => SUBETHA_KIND_BLOCKING_TCP_BRIDGE,
            Object::Sens(_) => SUBETHA_KIND_SENS,
            Object::Bloom(_) => SUBETHA_KIND_BLOOM,
            Object::Cms(_) => SUBETHA_KIND_CMS,
            Object::Histogram(_) => SUBETHA_KIND_HISTOGRAM,
            Object::RateLimiter(_) => SUBETHA_KIND_RATE_LIMITER,
            Object::BlockedBloom(_) => SUBETHA_KIND_BLOCKED_BLOOM,
            Object::Reservoir(_) => SUBETHA_KIND_RESERVOIR,
            Object::Topology(_) => SUBETHA_KIND_TOPOLOGY,
            Object::VersionedChain(_) => SUBETHA_KIND_VERSIONED_CHAIN,
            Object::VersionedSlab(_) => SUBETHA_KIND_VERSIONED_SLAB,
            Object::VersionedMap(_) => SUBETHA_KIND_VERSIONED_MAP,
            Object::LanedMap(_) => SUBETHA_KIND_LANED_MAP,
            Object::Channel(_) => SUBETHA_KIND_CHANNEL,
            Object::KvMap(_) => SUBETHA_KIND_KV_MAP,
            Object::WorkQueue(_) => SUBETHA_KIND_WORK_QUEUE,
            Object::ReorderWindow(_) => SUBETHA_KIND_REORDER_WINDOW,
            Object::AdaptiveQueue(_) => SUBETHA_KIND_ADAPTIVE_QUEUE,
            Object::HandleTable(_) => SUBETHA_KIND_HANDLE_TABLE,
            Object::LinkSensor(s) => s.kind(),
            Object::Adaptive(_) => SUBETHA_KIND_ADAPTIVE,
            Object::VersionedIndex(_) => SUBETHA_KIND_VERSIONED_INDEX,
        }
    }

    /// The object as the sidecar observes it, for the kinds whose Rust type
    /// records observations; none for the rest.
    fn adaptive(&self) -> Option<&(dyn AdaptiveInstance + 'static)> {
        match self {
            Object::HashMap(m) => Some(m.adaptive()),
            Object::Bloom(b) => Some(b.adaptive()),
            Object::BlockedBloom(b) => Some(b.adaptive()),
            Object::BitVec(b) => Some(b.adaptive()),
            Object::Broadcast(b) => Some(b.adaptive()),
            Object::EpochBarrier(b) => Some(b.adaptive()),
            Object::Heartbeat(h) => Some(h.adaptive()),
            Object::Hll(h) => Some(h.adaptive()),
            Object::Arena(a) => Some(a.adaptive()),
            Object::Leader(l) => Some(l.adaptive()),
            Object::FenceClock(c) => Some(c.adaptive()),
            Object::Cms(c) => Some(c.adaptive()),
            Object::Histogram(h) => Some(h.adaptive()),
            Object::RateLimiter(r) => Some(r.adaptive()),
            Object::Topology(t) => Some(t.adaptive()),
            Object::Atomic(a) => Some(a.adaptive()),
            Object::Reservoir(r) => Some(r.adaptive()),
            Object::VersionedChain(c) => Some(c.adaptive()),
            Object::Vyukov(v) => Some(v.adaptive()),
            Object::Ring(r) => Some(r.adaptive()),
            Object::HandleTable(t) => Some(t.adaptive()),
            Object::Adaptive(a) => Some(a.adaptive()),
            _ => None,
        }
    }

    /// Wake every thread parked inside a call on this object, so a destroy
    /// can wait for them to leave.
    fn interrupt(&self) {
        match self {
            Object::Ring(r) => r.interrupt(),
            Object::Spsc(s) => s.interrupt(),
            Object::MpscProducer(p) => p.interrupt(),
            Object::MpscConsumer(c) => c.interrupt(),
            Object::MpmcProducer(p) => p.interrupt(),
            Object::MpmcConsumer(c) => c.interrupt(),
            Object::Vyukov(v) => v.interrupt(),
            Object::LamportProducer(p) => p.interrupt(),
            Object::LamportConsumer(c) => c.interrupt(),
            Object::Broadcast(b) => b.interrupt(),
            Object::PubSub(p) => p.interrupt(),
            Object::Subscriber(s) => s.interrupt(),
            Object::Capacity(c) => c.interrupt(),
            Object::LocaleRing(l) => l.interrupt(),
            Object::EndpointRegistry(r) => r.interrupt(),
            Object::QosPolicy(q) => q.interrupt(),
            Object::Lru(c) => c.interrupt(),
            Object::Graph(g) => g.interrupt(),
            Object::Tile(t) => t.interrupt(),
            Object::Universal(u) => u.interrupt(),
            Object::Tower(t) => t.interrupt(),
            Object::BitVec(b) => b.interrupt(),
            Object::Hll(h) => h.interrupt(),
            Object::CapacityBroadcast(b) => b.interrupt(),
            Object::CapacityPubSub(p) => p.interrupt(),
            Object::CapacitySubscriber(s) => s.interrupt(),
            Object::Ordered(o) => o.interrupt(),
            Object::Stack(s) => s.interrupt(),
            Object::Deque(d) => d.interrupt(),
            Object::Notifier(n) => n.interrupt(),
            Object::HashMap(m) => m.interrupt(),
            Object::Arena(a) => a.interrupt(),
            Object::Vec(v) => v.interrupt(),
            Object::Slab(s) => s.interrupt(),
            Object::Region(r) => r.interrupt(),
            Object::Atomic(a) => a.interrupt(),
            Object::List(l) => l.interrupt(),
            Object::BTree(b) => b.interrupt(),
            Object::Cell(c) => c.interrupt(),
            Object::FrameRegion(r) => r.interrupt(),
            Object::Epochs(e) => e.interrupt(),
            Object::Condvar(c) => c.interrupt(),
            Object::Leader(l) => l.interrupt(),
            Object::Heartbeat(h) => h.interrupt(),
            Object::Holders(h) => h.interrupt(),
            Object::FenceClock(c) => c.interrupt(),
            Object::EpochBarrier(b) => b.interrupt(),
            Object::SharedArc(a) => a.interrupt(),
            Object::LazyValue(l) => l.interrupt(),
            Object::Waker(w) => w.interrupt(),
            Object::TcpBridge(b) => b.interrupt(),
            Object::QuicBridge(b) => b.interrupt(),
            Object::BlockingTcpBridge(b) => b.interrupt(),
            Object::Sens(s) => s.interrupt(),
            Object::Bloom(b) => b.interrupt(),
            Object::Cms(c) => c.interrupt(),
            Object::Histogram(h) => h.interrupt(),
            Object::RateLimiter(l) => l.interrupt(),
            Object::BlockedBloom(b) => b.interrupt(),
            Object::Reservoir(r) => r.interrupt(),
            Object::Topology(t) => t.interrupt(),
            Object::VersionedChain(c) => c.interrupt(),
            Object::VersionedSlab(s) => s.interrupt(),
            Object::VersionedMap(m) => m.interrupt(),
            Object::LanedMap(m) => m.interrupt(),
            Object::Channel(c) => c.interrupt(),
            Object::KvMap(m) => m.interrupt(),
            Object::WorkQueue(q) => q.interrupt(),
            Object::ReorderWindow(w) => w.interrupt(),
            Object::AdaptiveQueue(q) => q.interrupt(),
            Object::HandleTable(t) => t.interrupt(),
            Object::LinkSensor(s) => s.interrupt(),
            Object::Adaptive(a) => a.interrupt(),
            Object::VersionedIndex(i) => i.interrupt(),
        }
    }
}

pub(crate) struct Slot {
    generation: AtomicU32,
    state: AtomicU8,
    /// The object, as the raw form of the `Box` that owns it; null while
    /// the slot holds none. A call reads it after publishing the slot it
    /// is inside, and a destroy frees it only after every such call has
    /// left, so one Acquire load is the whole of a reader's protection.
    object: AtomicPtr<Object>,
    /// The object's sidecar registration as its instance id, or
    /// `NOT_OBSERVED`, or `CLAIMING` while a call makes one.
    observed: AtomicU64,
    /// How often the registration's policy answered something other than
    /// stay or move; set before `observed` names the registration.
    policy_errors: SwapCellOption<AtomicU64>,
    /// The text of the panic that poisoned the slot's object, read without
    /// waiting by the call it refuses.
    panic_message: SwapCellOption<String>,
    /// The slot owns the object the pointer names.
    _owns: PhantomData<Box<Object>>,
}

impl Slot {
    fn new() -> Self {
        Self {
            generation: AtomicU32::new(1),
            state: AtomicU8::new(STATE_FREE),
            object: AtomicPtr::new(std::ptr::null_mut()),
            observed: AtomicU64::new(NOT_OBSERVED),
            policy_errors: SwapCellOption::empty(),
            panic_message: SwapCellOption::empty(),
            _owns: PhantomData,
        }
    }
}

impl Drop for Slot {
    fn drop(&mut self) {
        let object = self.object.swap(std::ptr::null_mut(), Ordering::AcqRel);
        if !object.is_null() {
            // SAFETY: a non-null pointer here is the raw form of the Box
            // `insert` placed, and nothing else frees it.
            drop(unsafe { Box::from_raw(object) });
        }
    }
}

/// A live object borrowed for the duration of one call. Dropping it lets a
/// pending destroy proceed.
///
/// The object's address is read out of the table rather than counted,
/// which is what keeps a call to plain loads:
///
/// - A slot, once placed, is never freed or moved. Its chunk stays where
///   it was put for as long as the table does, which is the process, and
///   a destroy only recycles the index.
/// - The object is held by the call this thread published before it read
///   the slot's state. A destroy marks the slot closing, waits out every
///   call that began before that, and frees the object only afterward,
///   so a borrow that published and then read the state as live holds an
///   object no destroy can take away underneath it.
pub(crate) struct Borrowed {
    object: *const Object,
    /// Published for as long as this borrow lives, and cleared when it
    /// drops. Declared last so it is cleared after the pointer is done
    /// with.
    _inside: crate::epoch::Inside,
}

impl std::fmt::Debug for Borrowed {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Borrowed(kind {})", self.object().kind())
    }
}

impl Borrowed {
    pub(crate) fn object(&self) -> &Object {
        // SAFETY: the slot this borrow published keeps the object in place
        // for the borrow's lifetime.
        unsafe { &*self.object }
    }
}

/// Chunk `k` holds `1 << k` slots, so this many chunks cover every index
/// a handle can carry.
const CHUNKS: usize = 32;

/// The chunk an index lives in and its place there. Index `i` is slot
/// number `i + 1` overall, and chunk `k` holds the slots numbered from
/// `1 << k` up to but not including `1 << (k + 1)`.
fn locate(index: u32) -> (usize, usize) {
    let n = u64::from(index) + 1;
    let k = (63 - n.leading_zeros()) as usize;
    (k, (n - (1u64 << k)) as usize)
}

/// What the issue claim guards: the indices handed back by destroy, and
/// one past the highest index ever issued.
struct Issued {
    free: Vec<u32>,
    high: u64,
}

pub(crate) struct Table {
    /// One pointer per chunk, null until that chunk is placed. A placed
    /// chunk is never freed or moved while the table exists.
    chunks: [AtomicPtr<Slot>; CHUNKS],
    issued: Claim<Issued>,
    live: AtomicU64,
}

fn encode(index: u32, generation: u32) -> subetha_handle {
    (u64::from(index) << 32) | u64::from(generation)
}

fn decode(handle: subetha_handle) -> (u32, u32) {
    ((handle >> 32) as u32, handle as u32)
}

impl Table {
    pub(crate) fn new() -> Self {
        Self {
            chunks: std::array::from_fn(|_| AtomicPtr::new(std::ptr::null_mut())),
            issued: Claim::new(Issued { free: Vec::new(), high: 0 }),
            live: AtomicU64::new(0),
        }
    }

    /// The slot at `index`, if its chunk has been placed.
    fn slot(&self, index: u32) -> Option<&Slot> {
        let (k, offset) = locate(index);
        let base = self.chunks.get(k)?.load(Ordering::Acquire);
        if base.is_null() {
            return None;
        }
        // SAFETY: a placed chunk `k` holds `1 << k` slots, `offset` is
        // below that by construction, and the chunk is never freed or
        // moved while the table exists.
        Some(unsafe { &*base.add(offset) })
    }

    /// Objects currently held by a handle, poisoned ones included.
    pub(crate) fn live(&self) -> u64 {
        self.live.load(Ordering::Acquire)
    }

    /// Place an object and issue its handle.
    pub(crate) fn insert(&self, object: Object) -> subetha_handle {
        let mut issued = self.issued.claim();
        let index = match issued.free.pop() {
            Some(i) => i,
            None => {
                // A handle carries a 32-bit index, and the directory reaches
                // every one of those except the last, whose chunk would be
                // the thirty-third.
                let index = match u32::try_from(issued.high) {
                    Ok(i) if i != u32::MAX => i,
                    Ok(i) => panic!("index {i} is the one a handle cannot carry: every index below it is issued"),
                    Err(e) => panic!("the handle table has issued more indices than a handle can carry: {e}"),
                };
                let (k, _offset) = locate(index);
                if self.chunks[k].load(Ordering::Acquire).is_null() {
                    // The first index of a chunk places it, under the issue
                    // claim, before that index is handed out.
                    let chunk: Box<[Slot]> = (0..1usize << k).map(|_| Slot::new()).collect();
                    self.chunks[k].store(Box::into_raw(chunk).cast::<Slot>(), Ordering::Release);
                }
                issued.high += 1;
                index
            }
        };
        let slot = self.slot(index).expect("an issued index has a placed chunk");
        let generation = slot.generation.load(Ordering::Acquire);
        let object = Box::into_raw(Box::new(object));
        let previous = slot.object.swap(object, Ordering::AcqRel);
        debug_assert!(previous.is_null(), "a free slot holds no object");
        slot.panic_message.store(None);
        slot.state.store(STATE_LIVE, Ordering::Release);
        self.live.fetch_add(1, Ordering::AcqRel);
        encode(index, generation)
    }

    /// Borrow the object a handle names for one call. Refuses a handle that
    /// was never issued, has been destroyed, is being destroyed, or is
    /// poisoned; refuses a live handle of another kind.
    pub(crate) fn borrow(&self, handle: subetha_handle, kind: u32) -> Result<Borrowed, i32> {
        let (object, inside) = self.admit(handle, false)?;
        // SAFETY: this call is published and the slot was read as live, so
        // the object stays in place for this borrow.
        let kind_of = unsafe { (*object).kind() };
        if kind_of != kind {
            return Err(fail(
                SUBETHA_E_WRONG_KIND,
                format!("handle {handle:#x} is kind {kind_of}, not {kind}"),
            ));
        }
        Ok(Borrowed { object, _inside: inside })
    }

    /// The object a handle names, with this thread published as inside a
    /// call on that handle's slot for as long as the returned guard lives.
    /// A poisoned handle is admitted only when `poisoned_too` says so.
    fn admit(
        &self,
        handle: subetha_handle,
        poisoned_too: bool,
    ) -> Result<(*const Object, crate::epoch::Inside), i32> {
        if crate::holds::is_token(handle) {
            return Err(fail(
                SUBETHA_E_INVALID_HANDLE,
                format!("handle {handle:#x}: that is a hold token, not a handle"),
            ));
        }
        let (index, generation) = decode(handle);
        let Some(slot) = self.slot(index) else {
            return Err(fail(SUBETHA_E_INVALID_HANDLE, format!("handle {handle:#x}: no such slot")));
        };
        if slot.generation.load(Ordering::Acquire) != generation {
            return Err(fail(
                SUBETHA_E_INVALID_HANDLE,
                format!("handle {handle:#x}: its object was destroyed"),
            ));
        }
        // Published before the state is read, so a destroy either waits for
        // this call or this call sees the slot closing.
        let inside = crate::epoch::enter(index)?;
        match slot.state.load(Ordering::Acquire) {
            STATE_LIVE => {}
            STATE_POISONED if poisoned_too => {}
            STATE_POISONED => {
                return Err(fail(
                    SUBETHA_E_HANDLE_POISONED,
                    match slot.panic_message.load() {
                        Some(m) => format!("handle {handle:#x} poisoned by: {}", m.as_str()),
                        None => format!("handle {handle:#x} poisoned"),
                    },
                ));
            }
            _ => return Err(fail(SUBETHA_E_INVALID_HANDLE, format!("handle {handle:#x}: not live"))),
        }
        // A destroy that raced this call has bumped the generation; a call
        // that came in after that must not run.
        if slot.generation.load(Ordering::Acquire) != generation {
            return Err(fail(SUBETHA_E_INVALID_HANDLE, format!("handle {handle:#x}: destroyed")));
        }
        // The address, not a share of anything: the slot published above
        // is what holds the object for this call.
        let object = slot.object.load(Ordering::Acquire);
        if object.is_null() {
            return Err(fail(SUBETHA_E_INVALID_HANDLE, format!("handle {handle:#x}: empty")));
        }
        Ok((object as *const Object, inside))
    }

    /// The kind a live or poisoned handle names.
    pub(crate) fn kind_of(&self, handle: subetha_handle) -> Result<u32, i32> {
        let (object, _inside) = self.admit(handle, true)?;
        // SAFETY: this call is published and the slot was read as live or
        // poisoned, so the object stays in place until the guard drops.
        Ok(unsafe { (*object).kind() })
    }

    /// Register the object a handle names with the process's sidecar, under
    /// the policy its type makes. Refused for a kind that records no
    /// observations, an object already observed, and a sidecar holding its
    /// instance cap.
    pub(crate) fn observe(&self, handle: subetha_handle) -> i32 {
        self.observe_with(handle, None)
    }

    /// [`observe`](Self::observe), under the caller's own policy when
    /// `custom` is given. The policy is made only once every refusal has
    /// been ruled out, so a refused call never calls its `release`.
    pub(crate) fn observe_with(&self, handle: subetha_handle, custom: Option<crate::policy::Custom>) -> i32 {
        let (object, _inside) = match self.admit(handle, false) {
            Ok(pair) => pair,
            Err(code) => return code,
        };
        let slot = self.slot(decode(handle).0).expect("an admitted handle names a placed slot");
        // SAFETY: this call is published and the slot was read as live, so
        // the object stays in place until the guard drops.
        let object = unsafe { &*object };
        let Some(instance) = object.adaptive() else {
            return fail(SUBETHA_E_NOT_SUPPORTED, format!("a handle of kind {} records no observations", object.kind()));
        };
        if let Err(held) = slot.observed.compare_exchange(NOT_OBSERVED, CLAIMING, Ordering::AcqRel, Ordering::Acquire) {
            return fail(
                SUBETHA_E_INVALID_ARGUMENT,
                if held == CLAIMING {
                    "the object is being observed by another call right now".to_owned()
                } else {
                    format!("the object is already observed as instance {held}; subetha_unobserve it first")
                },
            );
        }
        let sidecar = subetha_sidecar::global();
        let cap = sidecar.max_instances();
        if sidecar.instance_count() >= cap {
            slot.observed.store(NOT_OBSERVED, Ordering::Release);
            return fail(
                SUBETHA_E_RING_FULL,
                format!("the sidecar holds its cap of {cap} instances; subetha_sidecar_set_max_instances raises it"),
            );
        }
        let errors = Arc::new(AtomicU64::new(0));
        let policy: Box<dyn subetha_sidecar::Policy> = match custom {
            Some(custom) => Box::new(crate::policy::CPolicy::new(handle, custom, Arc::clone(&errors))),
            None => instance.make_policy(),
        };
        slot.policy_errors.store(Some(errors));
        // SAFETY: the object stays at this address until `destroy`, which
        // ends this registration before it frees the object, and the claim
        // above makes this the object's one registration.
        let id = unsafe {
            sidecar.register_raw(
                NonNull::from(instance.header()),
                NonNull::from(instance.ring()),
                Some(NonNull::from(instance)),
                policy,
            )
        };
        slot.observed.store(u64::from(id), Ordering::Release);
        SUBETHA_OK
    }

    /// How often the observed object's policy answered something other
    /// than stay or move.
    pub(crate) fn policy_errors(&self, handle: subetha_handle) -> Result<u64, i32> {
        let (_object, _inside) = self.admit(handle, false)?;
        let slot = self.slot(decode(handle).0).expect("an admitted handle names a placed slot");
        if slot.observed.load(Ordering::Acquire) >= CLAIMING {
            return Err(fail(SUBETHA_E_INVALID_ARGUMENT, "the object is not observed; subetha_observe it first"));
        }
        match slot.policy_errors.load_full() {
            Some(errors) => Ok(errors.load(Ordering::Acquire)),
            None => Err(fail(SUBETHA_E_INVALID_ARGUMENT, "the object's registration has ended")),
        }
    }

    /// End the sidecar registration of the object a handle names.
    pub(crate) fn unobserve(&self, handle: subetha_handle) -> i32 {
        let (_object, _inside) = match self.admit(handle, false) {
            Ok(pair) => pair,
            Err(code) => return code,
        };
        let slot = self.slot(decode(handle).0).expect("an admitted handle names a placed slot");
        if crate::policy::deciding_for(handle) {
            return fail(
                SUBETHA_E_INVALID_ARGUMENT,
                "a policy cannot end its own registration from inside decide, which would wait on itself",
            );
        }
        let held = slot.observed.load(Ordering::Acquire);
        if held == NOT_OBSERVED {
            return fail(SUBETHA_E_INVALID_ARGUMENT, "the object is not observed");
        }
        if held == CLAIMING
            || slot.observed.compare_exchange(held, NOT_OBSERVED, Ordering::AcqRel, Ordering::Acquire).is_err()
        {
            return fail(SUBETHA_E_INVALID_ARGUMENT, "another call is changing the object's registration right now");
        }
        subetha_sidecar::global().unregister(held as u32);
        slot.policy_errors.store(None);
        SUBETHA_OK
    }

    /// The sidecar's statistics for the object a handle names, as the last
    /// scan that drained it left them.
    pub(crate) fn observation_stats(&self, handle: subetha_handle) -> Result<InstanceStats, i32> {
        let (_object, _inside) = self.admit(handle, false)?;
        let slot = self.slot(decode(handle).0).expect("an admitted handle names a placed slot");
        let held = slot.observed.load(Ordering::Acquire);
        if held >= CLAIMING {
            return Err(fail(SUBETHA_E_INVALID_ARGUMENT, "the object is not observed; subetha_observe it first"));
        }
        subetha_sidecar::global()
            .stats(held as u32)
            .ok_or_else(|| fail(SUBETHA_E_INVALID_ARGUMENT, "the object's registration has ended"))
    }

    /// Whether a live handle is poisoned.
    pub(crate) fn is_poisoned(&self, handle: subetha_handle) -> Result<bool, i32> {
        let (index, generation) = decode(handle);
        let Some(slot) = self.slot(index) else {
            return Err(fail(SUBETHA_E_INVALID_HANDLE, format!("handle {handle:#x}: no such slot")));
        };
        if slot.generation.load(Ordering::Acquire) != generation {
            return Err(fail(SUBETHA_E_INVALID_HANDLE, format!("handle {handle:#x}: destroyed")));
        }
        match slot.state.load(Ordering::Acquire) {
            STATE_LIVE => Ok(false),
            STATE_POISONED => Ok(true),
            _ => Err(fail(SUBETHA_E_INVALID_HANDLE, format!("handle {handle:#x}: not live"))),
        }
    }

    /// Mark the slot a handle names poisoned and keep the panic's text, so
    /// every later call on the handle fails until it is destroyed. The
    /// caller's borrow has already been released, so a destroy may have
    /// closed or recycled the slot meanwhile; a handle that no longer names
    /// a live object of its own generation is left as it is, since the
    /// object it named is gone or going.
    ///
    /// This thread publishes itself inside the slot first, as a call does,
    /// so a destroy that starts now waits for the mark before it can free
    /// the slot, and no insert can reissue the slot, between the generation
    /// check and the mark, to a handle this panic never touched. Only a
    /// thread being torn down is refused that, and the panicking call ran
    /// here with its locals in place, so a refusal leaves the mark checked
    /// against the generation alone.
    pub(crate) fn poison(&self, handle: subetha_handle, message: String) {
        let (index, generation) = decode(handle);
        let Some(slot) = self.slot(index) else {
            return;
        };
        let _published = crate::epoch::try_enter(index);
        if slot.generation.load(Ordering::Acquire) != generation {
            return;
        }
        #[cfg(test)]
        tests::while_marking_poison(index);
        if slot
            .state
            .compare_exchange(STATE_LIVE, STATE_POISONED, Ordering::AcqRel, Ordering::Acquire)
            .is_ok()
        {
            slot.panic_message.store(Some(Arc::new(message)));
        }
    }

    /// Release a handle: refuse new calls, interrupt any waiter, wait out
    /// every call that began before the refusal, drop the object, and
    /// retire the generation so the handle can never name anything again.
    /// Poisoned handles are destroyed the same way; that is how they are
    /// recovered.
    pub(crate) fn destroy(&self, handle: subetha_handle) -> i32 {
        if crate::holds::is_token(handle) {
            return fail(
                SUBETHA_E_INVALID_HANDLE,
                format!("handle {handle:#x}: that is a hold token, not a handle - release it on the object that issued it"),
            );
        }
        let (index, generation) = decode(handle);
        let Some(slot) = self.slot(index) else {
            return fail(SUBETHA_E_INVALID_HANDLE, format!("handle {handle:#x}: no such slot"));
        };
        if slot.generation.load(Ordering::Acquire) != generation {
            return fail(SUBETHA_E_INVALID_HANDLE, format!("handle {handle:#x}: already destroyed"));
        }
        if crate::policy::deciding_for(handle) {
            return fail(
                SUBETHA_E_INVALID_ARGUMENT,
                "a policy cannot destroy the handle it decides for from inside decide, which would wait on itself",
            );
        }
        let claimed = slot
            .state
            .compare_exchange(STATE_LIVE, STATE_CLOSING, Ordering::AcqRel, Ordering::Acquire)
            .or_else(|_| {
                slot.state.compare_exchange(
                    STATE_POISONED,
                    STATE_CLOSING,
                    Ordering::AcqRel,
                    Ordering::Acquire,
                )
            });
        if claimed.is_err() {
            return fail(
                SUBETHA_E_INVALID_HANDLE,
                format!("handle {handle:#x}: not live, or being destroyed by another thread"),
            );
        }
        // The state is closing under this destroy alone, so nothing frees
        // the object before the swap below.
        let object = slot.object.load(Ordering::Acquire);
        if !object.is_null() {
            // SAFETY: the pointer names the object `insert` placed, and only
            // this destroy will free it.
            unsafe { (*object).interrupt() };
        }
        // The slot is closing, so a call that begins from here on is
        // refused; this waits out the ones that began before it was.
        crate::epoch::wait_for_calls_on(index);
        // No call can start and none is inside, so this is the only
        // reference: dropping it runs the object's own cleanup, including
        // joining any thread it owns. The sidecar holds pointers into the
        // object, so its registration ends first.
        let observed = slot.observed.swap(NOT_OBSERVED, Ordering::AcqRel);
        if observed < CLAIMING {
            subetha_sidecar::global().unregister(observed as u32);
        }
        slot.policy_errors.store(None);
        let object = slot.object.swap(std::ptr::null_mut(), Ordering::AcqRel);
        if !object.is_null() {
            // SAFETY: the raw form of the Box `insert` placed, taken by this
            // destroy alone.
            drop(unsafe { Box::from_raw(object) });
        }
        slot.panic_message.store(None);
        let next = match generation.wrapping_add(1) {
            0 => 1,
            g => g,
        };
        slot.generation.store(next, Ordering::Release);
        slot.state.store(STATE_FREE, Ordering::Release);
        self.live.fetch_sub(1, Ordering::AcqRel);
        self.issued.claim().free.push(index);
        SUBETHA_OK
    }

    /// Destroy every handle still held. Returns how many there were.
    pub(crate) fn destroy_all(&self) -> u64 {
        let high = self.issued.claim().high;
        let mut closed = 0u64;
        for index in 0..high {
            // `insert` issues only indices a handle can carry, and places
            // the chunk before it issues the index.
            let index = index as u32;
            let slot = self.slot(index).expect("every index below high has a placed chunk");
            let state = slot.state.load(Ordering::Acquire);
            if state == STATE_LIVE || state == STATE_POISONED {
                let handle = encode(index, slot.generation.load(Ordering::Acquire));
                if self.destroy(handle) == SUBETHA_OK {
                    closed += 1;
                }
            }
        }
        closed
    }
}

impl Drop for Table {
    fn drop(&mut self) {
        for (k, chunk) in self.chunks.iter_mut().enumerate() {
            let base = *chunk.get_mut();
            if base.is_null() {
                continue;
            }
            // SAFETY: chunk `k` was placed by `insert` as a boxed slice of
            // `1 << k` slots and nothing else frees it.
            drop(unsafe { Box::from_raw(std::ptr::slice_from_raw_parts_mut(base, 1 << k)) });
        }
    }
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;
    use std::rc::Rc;
    use std::sync::Arc;

    use super::*;

    /// What `poison` runs, given the slot index, just before it marks the
    /// slot.
    type PoisonHook = Box<dyn FnOnce(u32)>;

    thread_local! {
        /// Run once by `poison` on this thread.
        static WHILE_MARKING_POISON: Cell<Option<PoisonHook>> = const { Cell::new(None) };
    }

    pub(super) fn while_marking_poison(index: u32) {
        if let Some(hook) = WHILE_MARKING_POISON.take() {
            hook(index);
        }
    }

    fn ring() -> Object {
        Object::Ring(crate::ring::RingObject::anon_for_tests())
    }

    /// A panic's mark is made with this thread published inside the slot,
    /// which is what makes a destroy wait for the mark before the slot can
    /// be freed and reissued to another handle.
    #[test]
    fn poison_marks_the_slot_while_published_inside_it() {
        let table = Table::new();
        let handle = table.insert(ring());
        let seen = Rc::new(Cell::new(None));
        let hook = {
            let seen = Rc::clone(&seen);
            move |index| seen.set(Some(crate::epoch::this_thread_is_inside(index)))
        };
        WHILE_MARKING_POISON.set(Some(Box::new(hook)));
        table.poison(handle, "boom".to_owned());
        assert_eq!(seen.get(), Some(true), "the mark was made with this thread inside the slot");
        assert!(table.is_poisoned(handle).expect("the handle is still issued"), "the mark landed");
        assert!(!crate::epoch::this_thread_is_inside(decode(handle).0), "the thread left the slot after the mark");
        assert_eq!(table.destroy(handle), SUBETHA_OK);
    }

    #[test]
    fn a_handle_is_refused_after_destroy_and_its_slot_is_reissued_with_a_new_generation() {
        let table = Table::new();
        let a = table.insert(ring());
        assert!(table.borrow(a, SUBETHA_KIND_RING).is_ok());
        assert_eq!(table.destroy(a), SUBETHA_OK);
        assert_eq!(table.borrow(a, SUBETHA_KIND_RING).unwrap_err(), SUBETHA_E_INVALID_HANDLE);
        assert_eq!(table.destroy(a), SUBETHA_E_INVALID_HANDLE, "a second destroy is refused");
        let b = table.insert(ring());
        assert_eq!(decode(b).0, decode(a).0, "the slot is reused");
        assert_ne!(decode(b).1, decode(a).1, "under a new generation");
        assert!(table.borrow(a, SUBETHA_KIND_RING).is_err(), "the old handle still names nothing");
        assert!(table.borrow(b, SUBETHA_KIND_RING).is_ok());
        assert_eq!(table.destroy(b), SUBETHA_OK);
    }

    #[test]
    fn zero_and_forged_handles_name_nothing() {
        let table = Table::new();
        assert_eq!(table.borrow(SUBETHA_HANDLE_NONE, SUBETHA_KIND_RING).unwrap_err(), SUBETHA_E_INVALID_HANDLE);
        assert_eq!(table.borrow(u64::MAX, SUBETHA_KIND_RING).unwrap_err(), SUBETHA_E_INVALID_HANDLE);
        let a = table.insert(ring());
        let forged = encode(decode(a).0, decode(a).1.wrapping_add(7));
        assert_eq!(table.borrow(forged, SUBETHA_KIND_RING).unwrap_err(), SUBETHA_E_INVALID_HANDLE);
        assert_eq!(table.destroy(a), SUBETHA_OK);
    }

    #[test]
    fn a_poisoned_handle_refuses_calls_until_it_is_destroyed() {
        let table = Table::new();
        let a = table.insert(ring());
        assert!(table.borrow(a, SUBETHA_KIND_RING).is_ok(), "live before the poison");
        table.poison(a, "boom".into());
        assert_eq!(table.borrow(a, SUBETHA_KIND_RING).unwrap_err(), SUBETHA_E_HANDLE_POISONED);
        assert!(table.is_poisoned(a).unwrap());
        assert_eq!(table.kind_of(a).unwrap(), SUBETHA_KIND_RING, "the kind is still readable");
        assert_eq!(table.destroy(a), SUBETHA_OK);
        assert_eq!(table.live(), 0);
    }

    #[test]
    fn the_table_grows_without_a_ceiling_and_every_handle_stays_distinct() {
        let table = Table::new();
        let handles: Vec<_> = (0..1000).map(|_| table.insert(ring())).collect();
        let mut sorted = handles.clone();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(sorted.len(), handles.len());
        assert_eq!(table.live(), 1000);
        for h in &handles {
            assert!(table.borrow(*h, SUBETHA_KIND_RING).is_ok());
        }
        assert_eq!(table.destroy_all(), 1000);
        assert_eq!(table.live(), 0);
    }

    #[test]
    fn destroy_waits_for_a_call_in_flight_and_that_call_sees_the_object_alive() {
        let table = Arc::new(Table::new());
        let a = table.insert(ring());
        let borrowed = table.borrow(a, SUBETHA_KIND_RING).unwrap();
        let t = {
            let table = Arc::clone(&table);
            std::thread::spawn(move || table.destroy(a))
        };
        // The destroyer is waiting out this borrow; the object must still
        // be reachable through it.
        std::thread::sleep(std::time::Duration::from_millis(20));
        assert!(!t.is_finished(), "destroy must not complete while a borrow is live");
        assert_eq!(borrowed.object().kind(), SUBETHA_KIND_RING);
        drop(borrowed);
        assert_eq!(t.join().unwrap(), SUBETHA_OK);
    }
}
