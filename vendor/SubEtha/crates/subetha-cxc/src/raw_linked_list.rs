//! `RawLinkedList`: the doubly linked list of
//! [`SharedLinkedList`](crate::shared_linked_list::SharedLinkedList) at an
//! element size and alignment fixed at run time instead of by a type
//! parameter, for a caller reaching the list from another language. Both
//! read and write one file format under one protocol.
//!
//! A push returns the node's handle, its slot in the low half and its
//! generation in the high half, which names the same node in every process
//! until that node is removed or popped; a handle used after that is
//! refused with `InvalidHandle`. The sentinel's handle is
//! [`HEAD_INDEX`](crate::shared_linked_list::HEAD_INDEX): a walk from
//! [`first`](RawLinkedList::first) by [`next_of`](RawLinkedList::next_of)
//! ends when it comes back to it.
//!
//! # Concurrency
//!
//! Lock-free, as the typed list is: any number of writers and readers in
//! any number of processes, none waiting on another, live or dead.

use std::num::NonZeroU64;
use std::path::Path;

use crate::list_core::ListCore;
use crate::raw_treiber_stack::ElementLayout;
use crate::set_table::{put_key, take_key};
use crate::shared_linked_list::{LinkedListError, NIL_INDEX};
use crate::shared_region::RegionError;

pub struct RawLinkedList {
    core: ListCore,
    layout: ElementLayout,
}

impl std::fmt::Debug for RawLinkedList {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RawLinkedList")
            .field("capacity", &self.core.capacity())
            .field("layout", &self.layout)
            .field("node_size", &self.core.node_size())
            .finish()
    }
}

impl RawLinkedList {
    /// A capacity with room for the sentinel's slot and at least one node.
    fn check_capacity(capacity: usize) -> Result<(), LinkedListError> {
        if capacity < 2 {
            return Err(LinkedListError::Region(RegionError::InvalidArgument(
                "a list needs a capacity of at least 2, the head and one node",
            )));
        }
        Ok(())
    }

    /// Obtain the list at `path` holding up to `capacity` nodes, the
    /// sentinel's slot among them: an empty one is laid out when the file
    /// does not exist, an existing one is attached with its nodes in place.
    /// A file built with another capacity or layout is a `LayoutMismatch`.
    pub fn create(path: impl AsRef<Path>, capacity: usize, layout: ElementLayout) -> Result<Self, LinkedListError> {
        Self::check_capacity(capacity)?;
        let core = ListCore::create(path.as_ref(), capacity, layout.slot_size, layout.alignment, layout.tag)?;
        Ok(Self { core, layout })
    }

    /// Truncate the list at `path` and lay an empty one out, invalidating
    /// every handle live peers hold.
    pub fn reset(path: impl AsRef<Path>, capacity: usize, layout: ElementLayout) -> Result<Self, LinkedListError> {
        Self::check_capacity(capacity)?;
        let core = ListCore::reset(path.as_ref(), capacity, layout.slot_size, layout.alignment, layout.tag)?;
        Ok(Self { core, layout })
    }

    /// Attach to the list at `path`, which must exist with this capacity and
    /// layout.
    pub fn open(path: impl AsRef<Path>, capacity: usize, layout: ElementLayout) -> Result<Self, LinkedListError> {
        Self::check_capacity(capacity)?;
        let core = ListCore::open(path.as_ref(), capacity, layout.slot_size, layout.alignment, layout.tag)?;
        Ok(Self { core, layout })
    }

    /// Nodes the list can hold, the sentinel's slot among them.
    pub fn capacity(&self) -> usize {
        self.core.capacity()
    }

    /// The value layout this handle opened the list with.
    pub fn layout(&self) -> ElementLayout {
        self.layout
    }

    /// Bytes one node takes, its links included.
    pub fn node_size(&self) -> usize {
        self.core.node_size()
    }

    /// Nodes in the list, counted by one walk from the front.
    pub fn len(&self) -> usize {
        self.core.len()
    }

    pub fn is_empty(&self) -> bool {
        self.core.is_empty()
    }

    /// A value argument of exactly the element size.
    fn sized(&self, value: &[u8]) -> Result<(), LinkedListError> {
        if value.len() != self.layout.slot_size {
            return Err(LinkedListError::Region(RegionError::PayloadTooLarge));
        }
        Ok(())
    }

    /// An output buffer of at least the element size, cut to it.
    fn room<'a>(&self, out: &'a mut [u8]) -> Result<&'a mut [u8], LinkedListError> {
        out.get_mut(..self.layout.slot_size).ok_or(LinkedListError::Region(RegionError::PayloadTooLarge))
    }

    /// Add `value` at the front and return its handle.
    pub fn push_front(&self, value: &[u8]) -> Result<u64, LinkedListError> {
        self.sized(value)?;
        // SAFETY: the list hands a value place nobody else writes, of the
        // element size rounded up to a whole word, eight-byte aligned.
        Self::pushed(self.core.push_front(|p| unsafe { put_key(p, value) }))
    }

    /// Add `value` at the back and return its handle.
    pub fn push_back(&self, value: &[u8]) -> Result<u64, LinkedListError> {
        self.sized(value)?;
        // SAFETY: as in `push_front`.
        Self::pushed(self.core.push_back(|p| unsafe { put_key(p, value) }))
    }

    /// A push's answer: the handle, or `Full` when the list had no node left.
    fn pushed(h: Option<NonZeroU64>) -> Result<u64, LinkedListError> {
        h.map(NonZeroU64::get).ok_or(LinkedListError::Region(RegionError::Full))
    }

    /// Remove the first node into `out`, at least one element long:
    /// `Ok(true)` with its value copied, `Ok(false)` when the list is
    /// empty.
    pub fn pop_front(&self, out: &mut [u8]) -> Result<bool, LinkedListError> {
        let out = self.room(out)?;
        // SAFETY: the list hands each read a value place of the element size
        // rounded up to a whole word, eight-byte aligned.
        Ok(self.core.pop_front(|p| unsafe { take_key(p, out) }).is_some())
    }

    /// Remove the last node into `out`, at least one element long.
    pub fn pop_back(&self, out: &mut [u8]) -> Result<bool, LinkedListError> {
        let out = self.room(out)?;
        // SAFETY: as in `pop_front`.
        Ok(self.core.pop_back(|p| unsafe { take_key(p, out) }).is_some())
    }

    /// Remove the node `handle` names into `out`, at least one element long.
    /// `InvalidHandle` when the handle names no node in the list.
    pub fn remove(&self, handle: u64, out: &mut [u8]) -> Result<(), LinkedListError> {
        let out = self.room(out)?;
        // SAFETY: as in `pop_front`.
        self.core.remove(handle, |p| unsafe { take_key(p, out) }).ok_or(LinkedListError::InvalidHandle)
    }

    /// Copy the value `handle` names into `out`, at least one element long.
    pub fn get(&self, handle: u64, out: &mut [u8]) -> Result<(), LinkedListError> {
        let out = self.room(out)?;
        // SAFETY: as in `pop_front`.
        self.core.get(handle, |p| unsafe { take_key(p, out) }).ok_or(LinkedListError::InvalidHandle)
    }

    /// Overwrite the value `handle` names, leaving the node where it is.
    pub fn set(&self, handle: u64, value: &[u8]) -> Result<(), LinkedListError> {
        self.sized(value)?;
        // SAFETY: as in `push_front`, a frame nobody else writes.
        self.core.set(handle, |p| unsafe { put_key(p, value) })
    }

    /// The first node's handle, or the head's when the list is empty.
    pub fn first(&self) -> Result<u64, LinkedListError> {
        Ok(self.core.first())
    }

    /// The last node's handle, or the head's when the list is empty.
    pub fn last(&self) -> Result<u64, LinkedListError> {
        Ok(self.core.last())
    }

    /// The handle after `handle`, which is the head's when `handle` is the
    /// last node: a walk ends when it comes back to the head. The head's
    /// handle gives the first node.
    pub fn next_of(&self, handle: u64) -> Result<u64, LinkedListError> {
        if handle == NIL_INDEX {
            return Err(LinkedListError::InvalidHandle);
        }
        self.core.next_of(handle).ok_or(LinkedListError::InvalidHandle)
    }

    /// The handle before `handle`, which is the head's when `handle` is the
    /// first node. The head's handle gives the last node.
    pub fn prev_of(&self, handle: u64) -> Result<u64, LinkedListError> {
        if handle == NIL_INDEX {
            return Err(LinkedListError::InvalidHandle);
        }
        self.core.prev_of(handle).ok_or(LinkedListError::InvalidHandle)
    }

    /// Free what processes killed mid-operation left in the list now, as
    /// [`SharedLinkedList::reclaim`](crate::shared_linked_list::SharedLinkedList::reclaim)
    /// does. How many nodes and frames came back.
    pub fn reclaim(&self) -> usize {
        self.core.reclaim()
    }

    pub fn flush(&self) -> Result<(), LinkedListError> {
        self.core.flush()?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shared_linked_list::{NodeHandle, SharedLinkedList, HEAD_INDEX};

    /// A file for one test, removed when the test ends; declared before the
    /// primitive that maps it so it drops after it.
    fn tmp(name: &str) -> crate::test_paths::TmpFile {
        crate::test_paths::TmpFile::new(format!("subetha-raw-list-{name}-{}.bin", std::process::id()))
    }

    const BYTES_16: ElementLayout = ElementLayout { slot_size: 16, alignment: 4, tag: 21 };

    fn value(i: u8) -> [u8; 16] {
        let mut v = [i; 16];
        v[15] = i.wrapping_add(1);
        v
    }

    /// Every value in the list, walked from the front.
    fn forward(list: &RawLinkedList) -> Vec<[u8; 16]> {
        let mut out = Vec::new();
        let mut at = list.first().unwrap();
        let mut buf = [0u8; 16];
        while at != HEAD_INDEX {
            list.get(at, &mut buf).unwrap();
            out.push(buf);
            at = list.next_of(at).unwrap();
        }
        out
    }

    /// Every value in the list, walked from the back.
    fn backward(list: &RawLinkedList) -> Vec<[u8; 16]> {
        let mut out = Vec::new();
        let mut at = list.last().unwrap();
        let mut buf = [0u8; 16];
        while at != HEAD_INDEX {
            list.get(at, &mut buf).unwrap();
            out.push(buf);
            at = list.prev_of(at).unwrap();
        }
        out
    }

    #[test]
    fn pushes_pops_and_a_removal_from_the_middle_keep_the_list_whole() {
        let path = tmp("ring");
        let list = RawLinkedList::create(&path, 8, BYTES_16).unwrap();
        assert!(list.is_empty());
        assert_eq!(list.first().unwrap(), HEAD_INDEX);
        assert_eq!(list.last().unwrap(), HEAD_INDEX);
        let mut out = [0u8; 16];
        assert!(!list.pop_front(&mut out).unwrap());
        assert!(!list.pop_back(&mut out).unwrap());

        let b = list.push_back(&value(2)).unwrap();
        let a = list.push_front(&value(1)).unwrap();
        list.push_back(&value(3)).unwrap();
        assert_eq!(list.len(), 3);
        assert_eq!(forward(&list), vec![value(1), value(2), value(3)]);
        assert_eq!(backward(&list), vec![value(3), value(2), value(1)]);

        list.remove(b, &mut out).unwrap();
        assert_eq!(out, value(2));
        assert_eq!(list.len(), 2);
        assert_eq!(forward(&list), vec![value(1), value(3)]);
        assert_eq!(backward(&list), vec![value(3), value(1)]);

        list.set(a, &value(9)).unwrap();
        list.get(a, &mut out).unwrap();
        assert_eq!(out, value(9));
        assert!(matches!(list.remove(HEAD_INDEX, &mut out).unwrap_err(), LinkedListError::InvalidHandle));
        assert!(matches!(list.remove(NIL_INDEX, &mut out).unwrap_err(), LinkedListError::InvalidHandle));
        assert!(matches!(list.get(8, &mut out).unwrap_err(), LinkedListError::InvalidHandle));
        assert!(matches!(list.remove(b, &mut out).unwrap_err(), LinkedListError::InvalidHandle), "a removed node's handle");
        assert!(matches!(list.set(a, &[0u8; 15]).unwrap_err(), LinkedListError::Region(RegionError::PayloadTooLarge)));

        assert!(list.pop_front(&mut out).unwrap());
        assert_eq!(out, value(9));
        assert!(list.pop_back(&mut out).unwrap());
        assert_eq!(out, value(3));
        assert!(list.is_empty());
        assert_eq!(list.first().unwrap(), HEAD_INDEX);
        assert_eq!(list.last().unwrap(), HEAD_INDEX);
    }

    #[test]
    fn a_full_list_refuses_the_next_push_and_a_removed_slot_returns() {
        let path = tmp("full");
        let list = RawLinkedList::create(&path, 3, BYTES_16).unwrap();
        let a = list.push_back(&value(1)).unwrap();
        list.push_back(&value(2)).unwrap();
        assert!(
            matches!(list.push_back(&value(3)).unwrap_err(), LinkedListError::Region(RegionError::Full)),
            "the sentinel takes one of the three slots"
        );
        let mut out = [0u8; 16];
        list.remove(a, &mut out).unwrap();
        let again = list.push_back(&value(4)).unwrap();
        assert_eq!(again as u32, a as u32, "the freed slot is handed out again");
        assert_ne!(again, a, "in its next generation");
        assert_eq!(forward(&list), vec![value(2), value(4)]);
    }

    #[test]
    fn a_typed_list_and_a_raw_one_share_a_file() {
        let path = tmp("interop");
        let typed: SharedLinkedList<[u8; 16]> = SharedLinkedList::create(&path, 8).unwrap();
        let first = typed.push_back(*b"sixteen bytes!!!").unwrap();
        let raw = RawLinkedList::open(&path, 8, ElementLayout { slot_size: 16, alignment: 1, tag: 0 }).unwrap();
        assert_eq!(raw.len(), 1);
        let mut out = [0u8; 16];
        raw.get(first.to_raw(), &mut out).unwrap();
        assert_eq!(&out, b"sixteen bytes!!!");
        let added = raw.push_back(b"raw side wrote!!").unwrap();
        assert_eq!(typed.len(), 2);
        assert_eq!(typed.get(NodeHandle::from_raw(added)), Some(*b"raw side wrote!!"));
        assert_eq!(typed.iter_forward(), vec![*b"sixteen bytes!!!", *b"raw side wrote!!"]);
        raw.remove(first.to_raw(), &mut out).unwrap();
        assert_eq!(typed.iter_forward(), vec![*b"raw side wrote!!"]);
    }

    #[test]
    fn a_list_of_fewer_than_two_slots_has_no_room_for_a_node() {
        let path = tmp("tiny");
        assert!(matches!(
            RawLinkedList::create(&path, 1, BYTES_16).unwrap_err(),
            LinkedListError::Region(RegionError::InvalidArgument(_))
        ));
        assert!(!path.exists());
    }

    /// A file holding no list, or a list of another layout, is refused, and
    /// the refusal names the field that differs.
    #[test]
    fn a_file_of_another_shape_is_refused_by_its_field() {
        let path = tmp("shape");
        drop(RawLinkedList::create(&path, 4, BYTES_16).unwrap());
        assert!(matches!(
            RawLinkedList::open(&path, 4, ElementLayout { tag: 22, ..BYTES_16 }).unwrap_err(),
            LinkedListError::LayoutMismatch(crate::layout::Mismatch { field: "tag", found: 21, wanted: 22 })
        ));
        assert!(matches!(
            RawLinkedList::open(&path, 4, ElementLayout { slot_size: 8, ..BYTES_16 }).unwrap_err(),
            LinkedListError::LayoutMismatch(crate::layout::Mismatch { field: "value size", .. })
        ));
    }
}
