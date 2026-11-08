//! `SharedLinkedList<T>` - a doubly linked list in a mapped file that any
//! number of threads and processes push to, pop from and remove from at
//! once, with O(1) removal by handle.
//!
//! # Handles
//!
//! A push returns a [`NodeHandle`]: the node's slot and generation. It
//! names the same node in every process until that node is removed or
//! popped. A handle used after that names a generation the slot no longer
//! has, and the list refuses it: `get` and `remove` answer `None`, `set`
//! answers `InvalidHandle`.
//!
//! # Concurrency
//!
//! Lock-free: pushes, pops, removals and sets run at once from any number
//! of threads and processes, and none waits on another, live or dead. A
//! read or a walk sees each node as some writer left it. The algorithm and
//! the file layout are shared with
//! [`RawLinkedList`](crate::raw_linked_list::RawLinkedList).

use std::marker::PhantomData;
use std::mem::{align_of, size_of, MaybeUninit};
use std::num::NonZeroU64;
use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};

use crate::list_core::{node_layout, ListCore, NodeLayout, Nodes, SENTINEL};
use crate::shared_region::RegionError;

/// The sentinel's handle: a walk that reaches it has passed the last node.
pub const HEAD_INDEX: u64 = SENTINEL;
/// The handle that names no node.
pub const NIL_INDEX: u64 = u64::MAX;

/// A node's slot and generation, valid until the node is removed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(C)]
pub struct NodeHandle<T> {
    pub index: u32,
    pub generation: u32,
    _phantom: PhantomData<T>,
}

impl<T> NodeHandle<T> {
    pub const NIL: Self = Self { index: u32::MAX, generation: u32::MAX, _phantom: PhantomData };

    #[inline]
    pub fn new(index: u32, generation: u32) -> Self {
        Self { index, generation, _phantom: PhantomData }
    }

    /// The handle a [`to_raw`](Self::to_raw) word, or a C or raw-list
    /// handle, names.
    #[inline]
    pub fn from_raw(raw: u64) -> Self {
        Self::new(raw as u32, (raw >> 32) as u32)
    }

    /// The handle as one word: the slot in the low half, the generation in
    /// the high half, as [`RawLinkedList`](crate::raw_linked_list::RawLinkedList)
    /// and the C list take it.
    #[inline]
    pub fn to_raw(self) -> u64 {
        u64::from(self.index) | (u64::from(self.generation) << 32)
    }

    #[inline]
    pub fn is_nil(self) -> bool {
        self.to_raw() == NIL_INDEX
    }
}

#[derive(Debug)]
pub enum LinkedListError {
    Region(RegionError),
    InvalidHandle,
    LayoutMismatch(crate::layout::Mismatch),
    /// The operating system refused to open, size or map the list file, or
    /// the processor has no lock-free 16-byte compare-and-swap
    /// (`ErrorKind::Unsupported`); the error carries the code and message.
    IoError(std::io::Error),
}

impl std::fmt::Display for LinkedListError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Region(e) => write!(f, "region: {e}"),
            Self::InvalidHandle => write!(f, "the handle names no live node"),
            Self::LayoutMismatch(m) => write!(f, "a list of another shape: {m}"),
            Self::IoError(e) => write!(f, "io: {e}"),
        }
    }
}

impl std::error::Error for LinkedListError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Region(e) => Some(e),
            Self::IoError(e) => Some(e),
            Self::InvalidHandle | Self::LayoutMismatch(_) => None,
        }
    }
}

impl From<RegionError> for LinkedListError {
    fn from(e: RegionError) -> Self {
        Self::Region(e)
    }
}
impl From<std::io::Error> for LinkedListError {
    fn from(e: std::io::Error) -> Self {
        Self::IoError(e)
    }
}

pub struct SharedLinkedList<T: Copy + Default + 'static> {
    core: ListCore,
    /// How many nodes the last walk found: the next walk's starting room.
    last_len: AtomicUsize,
    _phantom: PhantomData<T>,
    header_sidecar: subetha_core::HandshakeHeader,
    ring_sidecar: Box<subetha_core::ObservationRing>,
}

impl<T: Copy + Default + Send + Sync + 'static> subetha_sidecar::AdaptiveInstance for SharedLinkedList<T> {
    fn header(&self) -> &subetha_core::HandshakeHeader {
        &self.header_sidecar
    }
    fn ring(&self) -> &subetha_core::ObservationRing {
        &self.ring_sidecar
    }
    fn make_policy(&self) -> Box<dyn subetha_sidecar::Policy> {
        Box::new(subetha_sidecar::NoMigrationPolicy)
    }
}

/// Room for one more entry in `out`, which a walk has written to its
/// capacity: the place of the next entry and the room left from it.
#[cold]
#[inline(never)]
fn grow<O>(out: &mut Vec<O>) -> (*mut O, usize) {
    let len = out.capacity();
    // SAFETY: the walk wrote every entry up to the capacity.
    unsafe { out.set_len(len) };
    out.reserve(1);
    // SAFETY: the grown allocation holds more than `len` entries.
    (unsafe { out.as_mut_ptr().add(len) }, out.capacity() - len)
}

/// Copy a `T` from a list's value place, which the list checks after, a
/// whole element at a time, so a copy racing a writer is no data race.
///
/// # Safety
/// `p` is 8-byte aligned and addresses `size_of::<T>()` bytes, rounded up
/// to a whole word, that are readable.
#[inline(always)]
unsafe fn read_value<T>(p: *const u8) -> MaybeUninit<T> {
    // SAFETY: the caller's contract; a value another writer is replacing reads
    // as some bytes, which `MaybeUninit` holds until the list's check passes.
    unsafe { crate::atomic_copy::load(p as *const T) }
}

impl<T: Copy + Default + 'static> SharedLinkedList<T> {
    /// Where the nodes of a list of `T` lie, fixed by `T`.
    const LAYOUT: NodeLayout = match node_layout(size_of::<T>(), align_of::<T>()) {
        Some(layout) => layout,
        None => panic!("no list of this value type fits in memory"),
    };

    /// The nodes' place for one operation, by `T`'s layout.
    #[inline(always)]
    fn nodes(&self) -> Nodes<'_> {
        self.core.nodes_by(Self::LAYOUT)
    }

    /// Obtain the list at `path` holding up to `capacity` nodes, the
    /// sentinel's slot among them: an empty one is laid out when the file
    /// does not exist, an existing one is attached with its nodes in place.
    pub fn create(path: impl AsRef<Path>, capacity: usize) -> Result<Self, LinkedListError> {
        assert!(capacity >= 2, "capacity must include sentinel head + at least one node");
        Ok(Self::assemble(ListCore::create(path.as_ref(), capacity, size_of::<T>(), align_of::<T>(), 0)?))
    }

    /// Attach to the list at `path`, which must exist with this capacity and
    /// value type.
    pub fn open(path: impl AsRef<Path>, capacity: usize) -> Result<Self, LinkedListError> {
        Ok(Self::assemble(ListCore::open(path.as_ref(), capacity, size_of::<T>(), align_of::<T>(), 0)?))
    }

    fn assemble(core: ListCore) -> Self {
        Self {
            core,
            last_len: AtomicUsize::new(0),
            _phantom: PhantomData,
            header_sidecar: subetha_core::HandshakeHeader::new(),
            ring_sidecar: Box::new(subetha_core::ObservationRing::new()),
        }
    }

    /// A writer of `value` into the list's value place, a whole element at a
    /// time, so a reader copying a reused place meanwhile races no plain
    /// store.
    #[inline(always)]
    fn writer(value: T) -> impl FnOnce(*mut u8) {
        // SAFETY: the list hands a place nobody else writes, 8-byte aligned and
        // at least `size_of::<T>()` bytes.
        move |p| unsafe { crate::atomic_copy::store(p as *mut T, &value) }
    }

    /// Nodes the list can hold, the sentinel's slot among them.
    #[inline]
    pub fn capacity(&self) -> usize {
        self.core.capacity()
    }

    /// Nodes in the list, counted by one walk from the front.
    pub fn len(&self) -> usize {
        self.core.len()
    }

    pub fn is_empty(&self) -> bool {
        self.core.is_empty()
    }

    /// Bytes one node takes, its links included.
    pub fn node_size(&self) -> usize {
        self.core.node_size()
    }

    fn observe(&self, op: u16, status: u16) {
        self.ring_sidecar.push_op(op, status);
    }

    /// Push to the front of the list. Returns the node's handle.
    pub fn push_front(&self, value: T) -> Result<NodeHandle<T>, LinkedListError> {
        self.pushed(crate::sidecar_ops::linked_list::OP_PUSH_FRONT, self.core.push_front(Self::writer(value)))
    }

    /// Push to the back of the list. Returns the node's handle.
    pub fn push_back(&self, value: T) -> Result<NodeHandle<T>, LinkedListError> {
        self.pushed(crate::sidecar_ops::linked_list::OP_PUSH_BACK, self.core.push_back_by(self.nodes(), Self::writer(value)))
    }

    /// A push's answer, observed as `op`: the handle, or `Full` when the list
    /// had no node left.
    #[inline(always)]
    fn pushed(&self, op: u16, h: Option<NonZeroU64>) -> Result<NodeHandle<T>, LinkedListError> {
        match h {
            Some(h) => {
                self.observe(op, 0);
                Ok(NodeHandle::from_raw(h.get()))
            }
            None => {
                self.observe(op, 1);
                Err(LinkedListError::Region(RegionError::Full))
            }
        }
    }

    /// Remove and return the first element.
    pub fn pop_front(&self) -> Option<T> {
        // SAFETY: the list hands each read a value place of `T`'s size and
        // alignment.
        let r = self.core.pop_front_by(self.nodes(), |p| unsafe { read_value::<T>(p) });
        self.observe(crate::sidecar_ops::linked_list::OP_POP_FRONT, if r.is_none() { 2 } else { 0 });
        // SAFETY: the list returns a copy its checks confirmed whole.
        r.map(|v| unsafe { v.assume_init() })
    }

    /// Remove and return the last element.
    pub fn pop_back(&self) -> Option<T> {
        // SAFETY: as in `pop_front`.
        let r = self.core.pop_back(|p| unsafe { read_value::<T>(p) });
        self.observe(crate::sidecar_ops::linked_list::OP_POP_BACK, if r.is_none() { 2 } else { 0 });
        // SAFETY: as in `pop_front`.
        r.map(|v| unsafe { v.assume_init() })
    }

    /// Remove the node `handle` names in O(1) and return its value; None
    /// when the handle names no node in the list.
    pub fn remove(&self, handle: NodeHandle<T>) -> Option<T> {
        // SAFETY: as in `pop_front`.
        let r = self.core.remove(handle.to_raw(), |p| unsafe { read_value::<T>(p) });
        self.observe(crate::sidecar_ops::linked_list::OP_REMOVE, if r.is_none() { 2 } else { 0 });
        // SAFETY: as in `pop_front`.
        r.map(|v| unsafe { v.assume_init() })
    }

    /// The value at `handle`; None when the handle names no node in the
    /// list.
    pub fn get(&self, handle: NodeHandle<T>) -> Option<T> {
        // SAFETY: as in `pop_front`.
        let r = self.core.get(handle.to_raw(), |p| unsafe { read_value::<T>(p) });
        self.observe(crate::sidecar_ops::linked_list::OP_ITER, if r.is_none() { 2 } else { 0 });
        // SAFETY: as in `pop_front`.
        r.map(|v| unsafe { v.assume_init() })
    }

    /// Overwrite the value at `handle`, keeping the node's place in the
    /// list. `InvalidHandle` when the handle names no node in the list.
    pub fn set(&self, handle: NodeHandle<T>, value: T) -> Result<(), LinkedListError> {
        let r = self.core.set(handle.to_raw(), Self::writer(value));
        self.observe(crate::sidecar_ops::linked_list::OP_PUSH_BACK, u16::from(r.is_err()));
        r
    }

    /// First node's value (None if empty).
    pub fn first(&self) -> Option<T> {
        let r = self.value_at(self.core.first());
        self.observe(crate::sidecar_ops::linked_list::OP_ITER, if r.is_none() { 2 } else { 0 });
        r
    }

    /// Last node's value (None if empty).
    pub fn last(&self) -> Option<T> {
        let r = self.value_at(self.core.last());
        self.observe(crate::sidecar_ops::linked_list::OP_ITER, if r.is_none() { 2 } else { 0 });
        r
    }

    /// The value of the node `raw` names, which a removal may have taken
    /// since it was found.
    fn value_at(&self, raw: u64) -> Option<T> {
        if raw == HEAD_INDEX {
            return None;
        }
        // SAFETY: as in `pop_front`.
        self.core.get(raw, |p| unsafe { read_value::<T>(p) }).map(|v| unsafe { v.assume_init() })
    }

    /// Every node, front to back, from one walk, as `make` builds each
    /// entry from the node's handle and value, into one vector sized from
    /// the last walk.
    fn walk<O>(&self, mut make: impl FnMut(u64, T) -> O) -> Vec<O> {
        let mut out: Vec<O> = Vec::with_capacity(self.last_len.load(Ordering::Relaxed));
        // Where the next entry goes and the room left, as locals the walk's
        // loop keeps in registers; the entries written are the capacity less
        // that room. The vector itself is touched only to grow or begin again.
        let mut at = out.as_mut_ptr();
        let mut left = out.capacity();
        self.core.walk_by(
            self.nodes(),
            // SAFETY: as in `pop_front`.
            |p| unsafe { read_value::<T>(p) },
            |seen| match seen {
                None => {
                    // SAFETY: the entries before `at` were written below.
                    unsafe { out.set_len(out.capacity() - left) };
                    out.clear();
                    at = out.as_mut_ptr();
                    left = out.capacity();
                }
                Some((h, v)) => {
                    if left == 0 {
                        (at, left) = grow(&mut out);
                    }
                    // SAFETY: `at` has room for one entry, and the list returns a
                    // copy its checks confirmed whole.
                    unsafe {
                        at.write(make(h, v.assume_init()));
                        at = at.add(1);
                    }
                    left -= 1;
                }
            },
        );
        let len = out.capacity() - left;
        // SAFETY: the first `len` entries were written above.
        unsafe { out.set_len(len) };
        self.last_len.store(len, Ordering::Relaxed);
        self.observe(crate::sidecar_ops::linked_list::OP_ITER, 0);
        out
    }

    /// Snapshot of all values in forward order.
    pub fn iter_forward(&self) -> Vec<T> {
        self.walk(|_, v| v)
    }

    /// Snapshot of all values in reverse order (back to front).
    pub fn iter_backward(&self) -> Vec<T> {
        let mut values = self.iter_forward();
        values.reverse();
        values
    }

    /// Snapshot of all (handle, value) pairs in forward order.
    /// Useful for finding a handle by predicate, then removing it.
    pub fn iter_forward_with_handles(&self) -> Vec<(NodeHandle<T>, T)> {
        self.walk(|h, v| (NodeHandle::from_raw(h), v))
    }

    /// Free what processes killed mid-operation left in the list now: each
    /// node one took and never linked or swapped out and never freed, and
    /// each frame one took for a set. How many nodes and frames came back.
    /// Attaching does the same, and so does the next push into a full list
    /// once the operating system reports a registered process's exit.
    pub fn reclaim(&self) -> usize {
        self.core.reclaim()
    }

    pub fn flush(&self) -> Result<(), LinkedListError> {
        Ok(self.core.flush()?)
    }

    pub fn flush_async(&self) -> Result<(), LinkedListError> {
        Ok(self.core.flush_async()?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;
    use std::sync::Arc;

    /// A file for one test, removed when the test ends; declared before the
    /// primitive that maps it so it drops after it.
    fn tmp(name: &str) -> crate::test_paths::TmpFile {
        crate::test_paths::TmpFile::new(format!("subetha-linkedlist-{name}-{}.bin", std::process::id()))
    }

    #[test]
    fn create_initial_state_is_empty() {
        let p = tmp("init");
        let l: SharedLinkedList<u32> = SharedLinkedList::create(&p, 32).unwrap();
        assert!(l.is_empty());
        assert_eq!(l.len(), 0);
        assert_eq!(l.first(), None);
        assert_eq!(l.last(), None);
        assert_eq!(l.iter_forward(), Vec::<u32>::new());
    }

    #[test]
    fn push_back_and_iterate_forward() {
        let p = tmp("push-back");
        let l: SharedLinkedList<u32> = SharedLinkedList::create(&p, 32).unwrap();
        for i in [10u32, 20, 30, 40, 50] {
            l.push_back(i).unwrap();
        }
        assert_eq!(l.len(), 5);
        assert_eq!(l.iter_forward(), vec![10, 20, 30, 40, 50]);
        assert_eq!(l.iter_backward(), vec![50, 40, 30, 20, 10]);
        assert_eq!(l.first(), Some(10));
        assert_eq!(l.last(), Some(50));
    }

    #[test]
    fn push_front_inserts_at_head() {
        let p = tmp("push-front");
        let l: SharedLinkedList<u32> = SharedLinkedList::create(&p, 32).unwrap();
        for i in [10u32, 20, 30] {
            l.push_front(i).unwrap();
        }
        assert_eq!(l.iter_forward(), vec![30, 20, 10]);
        assert_eq!(l.last(), Some(10), "the first push to an empty list is its last node too");
    }

    #[test]
    fn pop_front_and_back_round_trip() {
        let p = tmp("pop");
        let l: SharedLinkedList<u32> = SharedLinkedList::create(&p, 32).unwrap();
        for i in [10u32, 20, 30, 40] {
            l.push_back(i).unwrap();
        }
        assert_eq!(l.pop_front(), Some(10));
        assert_eq!(l.pop_back(), Some(40));
        assert_eq!(l.iter_forward(), vec![20, 30]);
        assert_eq!(l.pop_front(), Some(20));
        assert_eq!(l.pop_back(), Some(30));
        assert!(l.is_empty());
        assert_eq!(l.pop_front(), None);
        assert_eq!(l.pop_back(), None);
    }

    #[test]
    fn remove_by_handle_in_middle_preserves_integrity() {
        let p = tmp("remove-middle");
        let l: SharedLinkedList<u32> = SharedLinkedList::create(&p, 32).unwrap();
        let h1 = l.push_back(10).unwrap();
        let h2 = l.push_back(20).unwrap();
        let h3 = l.push_back(30).unwrap();
        let h4 = l.push_back(40).unwrap();
        let h5 = l.push_back(50).unwrap();
        assert_eq!(l.remove(h3), Some(30));
        assert_eq!(l.len(), 4);
        assert_eq!(l.iter_forward(), vec![10, 20, 40, 50]);
        assert_eq!(l.iter_backward(), vec![50, 40, 20, 10]);
        assert_eq!(l.remove(h1), Some(10), "the first node");
        assert_eq!(l.iter_forward(), vec![20, 40, 50]);
        assert_eq!(l.remove(h5), Some(50), "the last node");
        assert_eq!(l.iter_forward(), vec![20, 40]);
        assert_eq!(l.last(), Some(40));
        assert_eq!(l.remove(h2), Some(20));
        assert_eq!(l.remove(h4), Some(40));
        assert!(l.is_empty());
    }

    #[test]
    fn remove_nil_or_head_returns_none() {
        let p = tmp("remove-nil");
        let l: SharedLinkedList<u32> = SharedLinkedList::create(&p, 8).unwrap();
        l.push_back(1).unwrap();
        assert_eq!(l.remove(NodeHandle::NIL), None);
        assert_eq!(l.remove(NodeHandle::from_raw(HEAD_INDEX)), None);
        assert_eq!(l.iter_forward(), vec![1]);
    }

    #[test]
    fn get_and_set_via_handle() {
        let p = tmp("get-set");
        let l: SharedLinkedList<u32> = SharedLinkedList::create(&p, 8).unwrap();
        let h = l.push_back(42).unwrap();
        assert_eq!(l.get(h), Some(42));
        l.set(h, 100).unwrap();
        assert_eq!(l.get(h), Some(100));
        l.set(h, 101).unwrap();
        assert_eq!(l.get(h), Some(101), "a second set replaces the first one's frame");
        assert_eq!(l.iter_forward(), vec![101]);
        assert_eq!(l.get(NodeHandle::NIL), None);
        assert!(matches!(l.set(NodeHandle::NIL, 0), Err(LinkedListError::InvalidHandle)));
        assert_eq!(l.pop_front(), Some(101), "a pop takes the value a set left");
    }

    #[test]
    fn full_capacity_returns_error() {
        let p = tmp("full");
        // capacity 4 = the sentinel's slot + 3 nodes.
        let l: SharedLinkedList<u32> = SharedLinkedList::create(&p, 4).unwrap();
        l.push_back(1).unwrap();
        l.push_back(2).unwrap();
        l.push_back(3).unwrap();
        assert!(matches!(l.push_back(4), Err(LinkedListError::Region(RegionError::Full))));
        l.pop_front().unwrap();
        l.push_back(4).unwrap();
        assert_eq!(l.iter_forward(), vec![2, 3, 4]);
    }

    #[test]
    fn iter_forward_with_handles_returns_pairs() {
        let p = tmp("iter-handles");
        let l: SharedLinkedList<u32> = SharedLinkedList::create(&p, 16).unwrap();
        let h1 = l.push_back(10).unwrap();
        let h2 = l.push_back(20).unwrap();
        let h3 = l.push_back(30).unwrap();
        let pairs = l.iter_forward_with_handles();
        assert_eq!(pairs, vec![(h1, 10), (h2, 20), (h3, 30)]);
    }

    #[test]
    fn cross_handle_visibility() {
        let p = tmp("cross-handle");
        let writer: SharedLinkedList<u32> = SharedLinkedList::create(&p, 16).unwrap();
        let reader: SharedLinkedList<u32> = SharedLinkedList::open(&p, 16).unwrap();
        writer.push_back(100).unwrap();
        writer.push_back(200).unwrap();
        assert_eq!(reader.iter_forward(), vec![100, 200]);
        let h = writer.push_back(300).unwrap();
        assert_eq!(reader.iter_forward(), vec![100, 200, 300]);
        assert_eq!(reader.remove(h), Some(300), "a handle names the node in every handle of the file");
        assert_eq!(writer.iter_forward(), vec![100, 200]);
    }

    #[test]
    fn struct_payload_round_trip() {
        #[derive(Clone, Copy, Debug, PartialEq, Default)]
        #[repr(C)]
        struct Event {
            ts_us: u64,
            code: u32,
        }
        let p = tmp("struct");
        let l: SharedLinkedList<Event> = SharedLinkedList::create(&p, 16).unwrap();
        let h1 = l.push_back(Event { ts_us: 100, code: 1 }).unwrap();
        l.push_back(Event { ts_us: 200, code: 2 }).unwrap();
        assert_eq!(l.get(h1), Some(Event { ts_us: 100, code: 1 }));
        assert_eq!(l.iter_forward(), vec![Event { ts_us: 100, code: 1 }, Event { ts_us: 200, code: 2 }]);
    }

    #[test]
    fn disk_persistence_survives_reopen() {
        let p = tmp("disk");
        {
            let l: SharedLinkedList<u32> = SharedLinkedList::create(&p, 16).unwrap();
            for i in [10u32, 20, 30, 40] {
                l.push_back(i).unwrap();
            }
            l.flush().unwrap();
        }
        let l2: SharedLinkedList<u32> = SharedLinkedList::open(&p, 16).unwrap();
        assert_eq!(l2.iter_forward(), vec![10, 20, 30, 40]);
    }

    #[test]
    fn lru_pattern_move_to_front() {
        // An access moves a node to the front: a removal by handle and a push
        // to the front, after which a pop from the back takes the least
        // recently used.
        let p = tmp("lru");
        let l: SharedLinkedList<u32> = SharedLinkedList::create(&p, 32).unwrap();
        l.push_back(1).unwrap();
        let h_b = l.push_back(2).unwrap();
        l.push_back(3).unwrap();
        let b = l.remove(h_b).unwrap();
        l.push_front(b).unwrap();
        assert_eq!(l.iter_forward(), vec![2, 1, 3]);
        assert_eq!(l.pop_back(), Some(3));
        assert_eq!(l.iter_forward(), vec![2, 1]);
    }

    #[test]
    fn free_list_pattern_uses_handles() {
        let p = tmp("free-list");
        let l: SharedLinkedList<u32> = SharedLinkedList::create(&p, 16).unwrap();
        let handles: Vec<_> = (0..10u32).map(|i| l.push_back(i).unwrap()).collect();
        for (idx, h) in handles.iter().enumerate() {
            if idx % 2 == 0 {
                assert_eq!(l.remove(*h), Some(idx as u32));
            }
        }
        assert_eq!(l.len(), 5);
        assert_eq!(l.iter_forward(), vec![1, 3, 5, 7, 9]);
    }

    /// A handle whose node was removed is refused, also once its slot holds
    /// another node.
    #[test]
    fn a_stale_handle_is_refused_after_its_slot_is_reused() {
        let p = tmp("stale");
        let l: SharedLinkedList<u32> = SharedLinkedList::create(&p, 4).unwrap();
        let old = l.push_back(7).unwrap();
        assert_eq!(l.remove(old), Some(7));
        assert_eq!(l.remove(old), None, "a second removal finds nothing");
        let new = l.push_back(8).unwrap();
        assert_eq!(new.index, old.index, "the freed slot is handed out again");
        assert_ne!(new.generation, old.generation);
        assert_eq!(l.get(old), None);
        assert_eq!(l.remove(old), None);
        assert!(matches!(l.set(old, 9), Err(LinkedListError::InvalidHandle)));
        assert_eq!(l.iter_forward(), vec![8], "the stale handle left the new node alone");
    }

    /// Threads pushing at both ends and popping from both ends at once lose
    /// no value and take none twice.
    #[test]
    fn writers_at_both_ends_lose_and_repeat_nothing() {
        let p = tmp("both-ends");
        let l = Arc::new(SharedLinkedList::<u64>::create(&p, 4096).unwrap());
        const PER: u64 = 20_000;
        let taken: Vec<Vec<u64>> = std::thread::scope(|s| {
            let handles: Vec<_> = (0..8u64)
                .map(|t| {
                    let l = Arc::clone(&l);
                    s.spawn(move || {
                        let mut got = Vec::new();
                        for i in 0..PER {
                            let v = (t << 32) | i;
                            loop {
                                let pushed = if i % 2 == 0 { l.push_back(v) } else { l.push_front(v) };
                                match pushed {
                                    Ok(_) => break,
                                    Err(LinkedListError::Region(RegionError::Full)) => got.extend(l.pop_front()),
                                    Err(e) => panic!("push: {e}"),
                                }
                            }
                            if i % 3 == 0 {
                                got.extend(l.pop_back());
                            } else if i % 3 == 1 {
                                got.extend(l.pop_front());
                            }
                        }
                        got
                    })
                })
                .collect();
            handles.into_iter().map(|h| h.join().expect("a writer")).collect()
        });
        let mut seen = HashSet::new();
        for v in taken.into_iter().flatten().chain(l.iter_forward()) {
            assert!(seen.insert(v), "value {v:#x} came out twice");
        }
        assert_eq!(seen.len() as u64, 8 * PER, "every value pushed is popped or still held");
        while l.pop_front().is_some() {}
        assert!(l.is_empty());
    }

    /// Threads racing to remove the same handles each take a node once, and
    /// removals of neighbors at once leave every other node in place.
    #[test]
    fn removers_of_neighbors_and_of_one_node_each_win_once() {
        let p = tmp("removers");
        let l = Arc::new(SharedLinkedList::<u64>::create(&p, 1024).unwrap());
        for round in 0..50u64 {
            let handles: Vec<_> = (0..512u64).map(|i| l.push_back(round << 32 | i).unwrap()).collect();
            let won: Vec<Vec<u64>> = std::thread::scope(|s| {
                let workers: Vec<_> = (0..4usize)
                    .map(|t| {
                        let (l, handles) = (Arc::clone(&l), &handles);
                        s.spawn(move || {
                            // Two threads walk the handles upward and two
                            // downward, so neighbors and the same node race.
                            let order: Vec<usize> = if t % 2 == 0 { (0..handles.len()).collect() } else { (0..handles.len()).rev().collect() };
                            order.into_iter().filter(|i| i % 4 != 3).filter_map(|i| l.remove(handles[i])).collect::<Vec<u64>>()
                        })
                    })
                    .collect();
                workers.into_iter().map(|w| w.join().expect("a remover")).collect()
            });
            let mut removed: Vec<u64> = won.into_iter().flatten().collect();
            removed.sort_unstable();
            let wanted: Vec<u64> = (0..512u64).filter(|i| i % 4 != 3).map(|i| round << 32 | i).collect();
            assert_eq!(removed, wanted, "round {round}: each removed node is taken exactly once");
            let left: Vec<u64> = (0..512u64).filter(|i| i % 4 == 3).map(|i| round << 32 | i).collect();
            assert_eq!(l.iter_forward(), left, "round {round}: the nodes nobody removed stay in order");
            while l.pop_front().is_some() {}
        }
    }

    /// Readers never see a value two sets mixed, while setters replace it and
    /// a walker copies every node.
    #[test]
    fn sets_never_tear_a_value_for_readers() {
        let p = tmp("sets");
        let l = Arc::new(SharedLinkedList::<[u64; 4]>::create(&p, 64).unwrap());
        let handles: Vec<_> = (0..16u64).map(|i| l.push_back([i; 4]).unwrap()).collect();
        let stop = std::sync::atomic::AtomicBool::new(false);
        std::thread::scope(|s| {
            for t in 0..2u64 {
                let (l, handles, stop) = (Arc::clone(&l), &handles, &stop);
                s.spawn(move || {
                    let mut i = 0u64;
                    while !stop.load(std::sync::atomic::Ordering::Relaxed) {
                        i += 1;
                        let v = (t << 40) | i;
                        l.set(handles[(i % 16) as usize], [v; 4]).unwrap();
                    }
                });
            }
            for _ in 0..2 {
                let (l, handles) = (Arc::clone(&l), &handles);
                s.spawn(move || {
                    for i in 0..20_000usize {
                        let v = l.get(handles[i % 16]).expect("the node stays");
                        assert!(v.iter().all(|&x| x == v[0]), "a get mixed two sets: {v:x?}");
                        if i % 100 == 0 {
                            for w in l.iter_forward() {
                                assert!(w.iter().all(|&x| x == w[0]), "a walk mixed two sets: {w:x?}");
                            }
                        }
                    }
                });
            }
            s.spawn(|| {
                std::thread::sleep(std::time::Duration::from_millis(300));
                stop.store(true, std::sync::atomic::Ordering::Relaxed);
            });
        });
        assert_eq!(l.len(), 16);
    }
}
