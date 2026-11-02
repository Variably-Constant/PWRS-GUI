//! The lock-free doubly linked list under
//! [`SharedLinkedList`](crate::shared_linked_list::SharedLinkedList) and
//! [`RawLinkedList`](crate::raw_linked_list::RawLinkedList): one mapped file
//! that any number of processes push to, pop from and remove from at once,
//! none of them waiting on another, live or dead.
//!
//! # Nodes and links
//!
//! A node holds a value and two 16-byte links, `next` and `prev`. A link's
//! low half names a target node by slot and generation; its high half holds
//! the node's own generation, a mark, a free flag and where the value lies.
//! `next` is the truth: a remover marks it, which freezes it, and then swaps
//! the predecessor's `next` past the node. `prev` is a hint toward the
//! predecessor, trusted only once the predecessor's `next` is seen to name
//! the node, and replaced by a walk from the front when it is not. Every
//! swap of a link names the node's own generation, so a swap aimed at a
//! node that was freed and handed out again fails.
//!
//! A link is read a half at a time wherever something else checks the
//! pair: a swap that names it whole, or a high half read after the low half
//! that still names the node's generation, which only rises. The mark and
//! the free swap the high half alone, since every swap of the whole word
//! expects it unmarked and the low half holds still from the mark on. A
//! node just taken, which no list reaches yet, has its own links written a
//! half at a time.
//!
//! The sentinel is no node: its `next` and `prev` are 8-byte words in the
//! header naming slot 0, generation 0, and it is never marked or reused. A
//! handle is a node's slot and generation, so a handle whose node is gone
//! names a generation the node no longer has.
//!
//! # Values
//!
//! A value lives in its node until a [`set`](crate::list_core::ListCore::set), which fills a
//! frame from a second pool and swaps the frame's number into the node's
//! `next`, so a set and a removal of one node order on that one word. A
//! frame's generation is odd while the frame is free and while its writer
//! fills it, and even while a node may name it. A reader copies a value,
//! then checks the node's `next` and the frame's generation, and copies
//! again when either moved or the generation was odd.
//!
//! # Free nodes and dead writers
//!
//! Free nodes and free frames are Treiber stacks in the header, above a bump
//! counter for the never-used ones. A handle holding a registry entry marks
//! a node whose value is at home with its entry's number, and frees that
//! node itself, by a store that steps its generation, once it is out of the
//! list; no other writer and no sweep frees it while that handle's process
//! lives. Any other marked node is freed by whoever swaps it out of the list,
//! by a swap of its `next` that steps its generation.
//!
//! A sweep frees what a killed process left: each node it took and never
//! linked, each node its mark names it on, swapped out first where the list
//! still reaches it, each other marked node the list no longer reaches, and
//! each frame it took that no node names. A gone process's registry entry is
//! freed only after the sweep, so an entry handed to another process never
//! names a node a gone process marked. A node's owner, a frame's taker and a registry
//! entry each record the process's pid and start time, and a process is
//! gone when its pid names no process or names one that started after it,
//! so a pid handed to another process never keeps what a dead one left.
//! Every handle registers its process in the
//! header and watches the other registered processes through the operating
//! system's exit event ([`exit_watch`](crate::exit_watch)); an exit owes a
//! sweep, which the next push or set that finds nothing free runs. A push
//! that finds nothing free and owes no sweep is refused as `Full`, reading
//! two words. Attaching sweeps, as does [`reclaim`](crate::list_core::ListCore::reclaim), so a
//! handle past the registry's entries is covered too.

use std::fs::File;
use std::io;
use std::mem::size_of;
use std::num::NonZeroU64;
use std::path::Path;
use std::sync::atomic::{fence, AtomicBool, AtomicU64, Ordering};

use memmap2::{MmapMut, MmapOptions};

use crate::atomic_pair::{self, AtomicPair};
use crate::layout::Mismatch;
use crate::process_identity;
use crate::shared_linked_list::LinkedListError;
use crate::shared_region::RegionError;

/// Record an event about `$list` in the test-build trace, when a test named
/// the list: `$slot` a node, `$link` the node's handle, which the trace's
/// stories follow. Outside a test build it is nothing.
macro_rules! note {
    ($list:expr, $what:ident, $slot:expr, $link:expr, $word:expr, $extra:expr) => {
        #[cfg(test)]
        crate::set_trace::record(
            $list.base as usize,
            crate::set_trace::What::$what,
            $slot as usize,
            0,
            $link,
            $word,
            $extra,
        );
    };
}

const LIST_MAGIC: u32 = 0x4C4B_4C53;
const LIST_FORMAT: u32 = 2;

/// The sentinel as a link: slot 0, generation 0. A walk that reaches it has
/// passed the last node.
pub(crate) const SENTINEL: u64 = 0;

/// High-half flags of a link: the node is removed, or it is free.
const MARK: u64 = 1 << 32;
const FREE: u64 = 1 << 33;
/// Where the value lies, in the high half's top 30 bits: zero for the node
/// itself, else one past a frame, below NAMED.
const VLOC_SHIFT: u32 = 34;
const VLOC_MASK: u64 = !0 << VLOC_SHIFT;
const HOME: u64 = 0;
/// The top bit of a marked node whose value is at home, set when its mark
/// names the registry entry of the handle that made it, in the bits from
/// VLOC_SHIFT up. That handle frees the node, by stores.
const NAMED: u64 = 1 << 63;

const _: () = assert!(REGISTRY.is_power_of_two() && (REGISTRY as u64) << VLOC_SHIFT < NAMED);

/// Where a node's fields lie: the two links, the process that took it, then
/// the value at the shape's `value_at`.
const NEXT_AT: usize = 0;
const PREV_AT: usize = 16;
const OWNER_AT: usize = 32;
const LINKS_END: usize = 40;

/// Frames beyond one per node, for sets in flight.
const FRAME_SLACK: usize = 64;

/// A free-stack head's low half when the stack is empty and its bump
/// counter has items left.
const EMPTY: u32 = 0;
/// A free-stack head's low half when the stack is empty and its bump
/// counter is spent. It is one past no item, since a list has fewer than
/// 2^30 nodes and frames. An item freed onto it takes it as its link, so
/// the pop of the last free item leaves it in the head again.
const EXHAUSTED: u32 = u32::MAX;

/// Where a frame's fields lie: its word, its taker's holder word, then the
/// value at the shape's `frame_value_at`, from the first multiple of the
/// value's alignment at or past FRAME_HEAD.
const FRAME_TAKER_AT: usize = 8;
const FRAME_HEAD: usize = 16;

fn link(slot: u32, generation: u32) -> u64 {
    u64::from(slot) | (u64::from(generation) << 32)
}

fn slot_of(l: u64) -> u32 {
    l as u32
}

fn gen_of(l: u64) -> u32 {
    (l >> 32) as u32
}

fn pair(lo: u64, hi: u64) -> u128 {
    u128::from(lo) | (u128::from(hi) << 64)
}

fn lo(w: u128) -> u64 {
    w as u64
}

fn hi(w: u128) -> u64 {
    (w >> 64) as u64
}

/// The node's own generation in a link's high half.
fn own(h: u64) -> u32 {
    h as u32
}

/// Where the value lies: HOME, or one past a frame. A named mark was made on
/// a value at home.
fn vloc(h: u64) -> u64 {
    if h & NAMED != 0 { HOME } else { h >> VLOC_SHIFT }
}

/// The high half `h` as it read before its mark.
fn unmarked(h: u64) -> u64 {
    if h & NAMED != 0 { h & !(MARK | VLOC_MASK) } else { h & !MARK }
}

/// The registry entry a named mark names.
fn marker_entry(h: u64) -> usize {
    (h >> VLOC_SHIFT) as usize & (REGISTRY - 1)
}

/// Whether the process a holder word names is gone: its pid names no
/// process, or names one that started after it. An unset owner, and this
/// process, never are.
fn holder_gone(holder: u64) -> bool {
    holder != 0 && holder != process_identity::current() && process_identity::holder_gone(holder)
}

/// The list file's first five lines.
#[repr(C, align(64))]
struct ListHeader {
    magic: u32,
    format: u32,
    value_size: u32,
    value_align: u32,
    capacity: u64,
    frame_count: u64,
    tag: u64,
    _line0: [u8; 24],
    /// The sentinel's `next`: the first node, or the sentinel itself.
    head_next: AtomicU64,
    _line1: [u8; 56],
    /// The sentinel's `prev`: a hint toward the last node.
    head_prev: AtomicU64,
    _line2: [u8; 56],
    /// Free nodes: a tag in the high half, and in the low one past the top
    /// node, [`EMPTY`] or [`EXHAUSTED`].
    node_free: AtomicU64,
    /// The first never-used node. Reads past the capacity while a taker
    /// that found none left gives its add back.
    node_bump: AtomicU64,
    _line3: [u8; 48],
    /// Free frames, laid out as `node_free`.
    frame_free: AtomicU64,
    /// The first never-used frame, as `node_bump`.
    frame_bump: AtomicU64,
    _line4: [u8; 48],
    /// Steps each time a handle registers or leaves the registry.
    registry_epoch: AtomicU64,
    _line5: [u8; 56],
    /// The process id each attached handle registered, zero for a free
    /// entry.
    registry: [AtomicU64; REGISTRY],
}

/// Handles the registry can name; a handle past them is covered by the
/// sweeps of attaching and of `reclaim` alone.
const REGISTRY: usize = 64;

const _: () = assert!(size_of::<ListHeader>() == 384 + REGISTRY * 8);

/// Where a list's nodes lie: their offset in the file, the stride from one
/// node to the next, the value's offset in a node and the bytes a value
/// takes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct NodeLayout {
    nodes_at: usize,
    stride: usize,
    value_at: usize,
    value_stride: usize,
}

/// The [`NodeLayout`] of a list of `value_size`-byte values aligned to
/// `value_align`, a power of two; None when it does not fit in a `usize`.
/// Every list file is laid out by it, and a typed list takes it as a
/// constant of its value type.
pub(crate) const fn node_layout(value_size: usize, value_align: usize) -> Option<NodeLayout> {
    let align = if value_align > 8 { value_align } else { 8 };
    let wide = if align > 16 { align } else { 16 };
    let Some(value_stride) = (if value_size > 8 { value_size } else { 8 }).checked_next_multiple_of(align) else {
        return None;
    };
    let Some(value_at) = LINKS_END.checked_next_multiple_of(align) else {
        return None;
    };
    let Some(end) = value_at.checked_add(value_stride) else {
        return None;
    };
    let Some(stride) = end.checked_next_multiple_of(wide) else {
        return None;
    };
    // The smallest node, 48 bytes, takes 64, so its address is its slot
    // shifted once; a value of up to 24 bytes fills a 64-byte node either way.
    let stride = if stride < 64 { 64 } else { stride };
    let Some(nodes_at) = size_of::<ListHeader>().checked_next_multiple_of(wide) else {
        return None;
    };
    Some(NodeLayout { nodes_at, stride, value_at, value_stride })
}

/// Where everything in a list file lies.
#[derive(Debug, Clone, Copy)]
struct Shape {
    value_size: usize,
    value_align: usize,
    tag: u64,
    capacity: usize,
    frame_count: usize,
    value_at: usize,
    node_stride: usize,
    frame_value_at: usize,
    frame_stride: usize,
    nodes_at: usize,
    frames_at: usize,
    total: usize,
}

impl Shape {
    /// The shape of a list of `capacity` nodes of `value_size`-byte values;
    /// the reason when there is none.
    fn new(capacity: usize, value_size: usize, value_align: usize, tag: u64) -> Result<Self, &'static str> {
        if capacity < 2 {
            return Err("a list needs a capacity of at least 2, the head and one node");
        }
        // A link names a frame by its number plus one in the 29 bits between
        // its flags and NAMED, and the frames number the nodes and
        // FRAME_SLACK more.
        if capacity >= (1 << (63 - VLOC_SHIFT)) - FRAME_SLACK {
            return Err("a list holds fewer than 536870848 nodes");
        }
        if !value_align.is_power_of_two() {
            return Err("a value's alignment is a power of two");
        }
        if u32::try_from(value_size).is_err() {
            return Err("a value is under four gigabytes");
        }
        let too_large = || "the list for this capacity and value size does not fit in memory";
        let NodeLayout { nodes_at, stride: node_stride, value_at, value_stride } =
            node_layout(value_size, value_align).ok_or_else(too_large)?;
        let align = value_align.max(8);
        let frame_value_at = FRAME_HEAD.next_multiple_of(align);
        let frame_stride = (frame_value_at + value_stride).checked_next_multiple_of(align).ok_or_else(too_large)?;
        let frame_count = capacity + FRAME_SLACK;
        let frames_at = capacity
            .checked_mul(node_stride)
            .and_then(|n| n.checked_add(nodes_at))
            .and_then(|n| n.checked_next_multiple_of(align))
            .ok_or_else(too_large)?;
        let total = frame_count.checked_mul(frame_stride).and_then(|n| n.checked_add(frames_at)).ok_or_else(too_large)?;
        Ok(Self {
            value_size,
            value_align,
            tag,
            capacity,
            frame_count,
            value_at,
            node_stride,
            frame_value_at,
            frame_stride,
            nodes_at,
            frames_at,
            total,
        })
    }

    /// Lay a fresh file out, the magic last, because an attacher spins on
    /// it. Node 0 is the sentinel's slot and is never handed out.
    ///
    /// # Safety
    /// `ptr` addresses at least `self.total` writable zeroed bytes, aligned
    /// to a page.
    unsafe fn init(&self, ptr: *mut u8) {
        let hdr = ptr as *mut ListHeader;
        // SAFETY: the caller's contract; the header is the file's start.
        unsafe {
            (*hdr).format = LIST_FORMAT;
            (*hdr).value_size = self.value_size as u32;
            (*hdr).value_align = self.value_align as u32;
            (*hdr).capacity = self.capacity as u64;
            (*hdr).frame_count = self.frame_count as u64;
            (*hdr).tag = self.tag;
            (*hdr).node_bump.store(1, Ordering::Relaxed);
            std::ptr::write_volatile(&raw mut (*hdr).magic, LIST_MAGIC);
        }
    }

    /// The first header field of the list at `base` that differs from this
    /// shape.
    fn check(&self, base: *const u8) -> io::Result<()> {
        // SAFETY: the caller mapped at least a header's length at `base`.
        let hdr = unsafe { &*(base as *const ListHeader) };
        Mismatch::first(&[
            ("magic", u64::from(hdr.magic), u64::from(LIST_MAGIC)),
            ("format", u64::from(hdr.format), u64::from(LIST_FORMAT)),
            ("value size", u64::from(hdr.value_size), self.value_size as u64),
            ("value alignment", u64::from(hdr.value_align), self.value_align as u64),
            ("capacity", hdr.capacity, self.capacity as u64),
            ("tag", hdr.tag, self.tag),
        ])
        .map_err(io::Error::from)
    }
}

/// The node whose `next` names the node a walk is at: the sentinel, or a
/// node with the `next` the walk read from it.
#[derive(Clone, Copy)]
enum Pred {
    Sentinel,
    Node(u64, u128),
}

impl Pred {
    fn link(self) -> u64 {
        match self {
            Pred::Sentinel => SENTINEL,
            Pred::Node(l, _) => l,
        }
    }
}

/// Why one take of a node came back without one.
#[derive(Clone, Copy)]
enum Missed {
    /// The free stack's top moved under the take: another try may find one.
    Moved,
    /// The free stack and the bump counter are both empty.
    Empty,
}

/// What a pop of a free stack found.
#[derive(Clone, Copy)]
enum Popped {
    /// The item it took.
    Item(u32),
    /// No item, and the stack's bump counter has items left.
    Empty,
    /// No item, and the stack's bump counter is spent.
    Exhausted,
}

/// What a walk does at a node whose `next` high half is not its generation
/// alone.
enum Passed<V> {
    /// Visit the node with this value, then step to this link.
    Value(V, u64),
    /// Pass over the node, a removal under way, to this link.
    Marked(u64),
    /// Begin again from the front: the node was freed under the walk.
    Freed,
}

/// Where a list's nodes lie, copied out of its shape once per operation so
/// that the operation keeps it in registers across the swaps, which the
/// compiler treats as touching all memory; or built from a typed list's
/// constant [`NodeLayout`], which it folds into each address.
#[derive(Clone, Copy)]
pub(crate) struct Nodes<'a> {
    first: *mut u8,
    stride: usize,
    value_at: usize,
    _list: std::marker::PhantomData<&'a ListCore>,
}

impl<'a> Nodes<'a> {
    #[inline(always)]
    fn at(self, s: u32) -> *mut u8 {
        // SAFETY: callers pass a slot below the capacity, and the nodes region
        // holds `capacity` nodes of `stride`.
        unsafe { self.first.add(s as usize * self.stride) }
    }

    #[inline(always)]
    fn next(self, s: u32) -> &'a AtomicPair {
        // SAFETY: as for `ListCore::next`.
        unsafe { AtomicPair::at(self.at(s).add(NEXT_AT)) }
    }

    #[inline(always)]
    fn prev(self, s: u32) -> &'a AtomicPair {
        // SAFETY: as for `ListCore::prev`.
        unsafe { AtomicPair::at(self.at(s).add(PREV_AT)) }
    }

    #[inline(always)]
    fn owner(self, s: u32) -> &'a AtomicU64 {
        // SAFETY: as for `ListCore::owner`.
        unsafe { &*(self.at(s).add(OWNER_AT) as *const AtomicU64) }
    }

    #[inline(always)]
    fn home(self, s: u32) -> *mut u8 {
        // SAFETY: as for `ListCore::home`.
        unsafe { self.at(s).add(self.value_at) }
    }

    /// The free-stack link of a free node, in its value's first word.
    #[inline(always)]
    fn free_link(self, s: u32) -> &'a AtomicU64 {
        // SAFETY: the value is at least eight bytes, eight-byte aligned.
        unsafe { &*(self.home(s) as *const AtomicU64) }
    }
}

/// One list, mapped.
pub(crate) struct ListCore {
    _file: File,
    mmap: MmapMut,
    base: *mut u8,
    shape: Shape,
    /// `node_bump`'s limit, the capacity: the word a push reads for it.
    node_limit: crate::bump::Limit,
    /// `frame_bump`'s limit, the frame count.
    frame_limit: crate::bump::Limit,
    /// The registry entry this handle holds, when it found one free.
    entry: Option<usize>,
    /// The bits this handle's mark sets in the high half of a node whose
    /// value is at home: MARK, and NAMED with the entry when it holds one.
    home_mark: u64,
    /// Set when a registrant reads gone as this handle watches it: a sweep
    /// is owed.
    owed: AtomicBool,
    /// The count of exits this process's watches were told of when this
    /// handle last swept or attached: another count owes a sweep.
    seen_exits: AtomicU64,
    /// The registry epoch whose processes this handle watches.
    seen_epoch: AtomicU64,
}

// SAFETY: every shared access goes through the header's atomic words, the
// nodes' atomic links and owner words and the frames' generation words; a
// value is written only by the writer holding its node or frame before the
// swap that publishes it, and read under the generation checks above.
unsafe impl Send for ListCore {}
// SAFETY: as for Send.
unsafe impl Sync for ListCore {}

impl Drop for ListCore {
    fn drop(&mut self) {
        if let Some(i) = self.entry {
            let hdr = self.header();
            if hdr.registry[i].compare_exchange(process_identity::current(), 0, Ordering::AcqRel, Ordering::Relaxed).is_ok() {
                hdr.registry_epoch.fetch_add(1, Ordering::AcqRel);
            }
        }
    }
}

impl ListCore {
    /// The handle over a mapped list: registered, swept when `sweep`, and
    /// watching every other registered process.
    fn from_parts(file: File, mut mmap: MmapMut, shape: Shape, sweep: bool) -> Self {
        let base = mmap.as_mut_ptr();
        let mut core = Self {
            _file: file,
            mmap,
            base,
            shape,
            node_limit: crate::bump::Limit::new(shape.capacity as u64),
            frame_limit: crate::bump::Limit::new(shape.frame_count as u64),
            entry: None,
            home_mark: MARK,
            owed: AtomicBool::new(false),
            seen_exits: AtomicU64::new(crate::exit_watch::exits()),
            seen_epoch: AtomicU64::new(u64::MAX),
        };
        core.entry = core.register();
        if let Some(e) = core.entry {
            core.home_mark = MARK | NAMED | ((e as u64) << VLOC_SHIFT);
        }
        if sweep {
            core.reclaim();
        }
        core.watch_registered();
        core
    }

    /// Take a free registry entry for this process: its index, or none when
    /// every entry is held.
    fn register(&self) -> Option<usize> {
        let hdr = self.header();
        let me = process_identity::current();
        let i = hdr.registry.iter().position(|e| e.compare_exchange(0, me, Ordering::AcqRel, Ordering::Relaxed).is_ok())?;
        hdr.registry_epoch.fetch_add(1, Ordering::AcqRel);
        Some(i)
    }

    /// Watch each process a registry entry names, once per process however
    /// many handles name it (see [`exit_watch`](crate::exit_watch)), so its
    /// exit owes a sweep. The watch follows the pid, so a registrant found
    /// gone once it is set, its pid handed to another process, owes the
    /// sweep at once. A process the operating system gives no exit event for
    /// is left to the sweeps of attaching and of [`reclaim`](Self::reclaim).
    fn watch_registered(&self) {
        let hdr = self.header();
        let epoch = hdr.registry_epoch.load(Ordering::Acquire);
        let me = process_identity::current();
        for entry in &hdr.registry {
            let holder = entry.load(Ordering::Acquire);
            if holder != 0 && holder != me {
                crate::exit_watch::watch_holder(holder);
                if process_identity::holder_gone(holder) {
                    self.owed.store(true, Ordering::Release);
                }
            }
        }
        self.seen_epoch.store(epoch, Ordering::Release);
    }

    /// What a push or a set that found nothing free does before it reports
    /// `Full`: watch the processes registered since this handle last looked,
    /// and sweep once when an exit owes it. Whether the sweep freed anything,
    /// so the caller looks again.
    fn after_miss(&self) -> bool {
        if self.header().registry_epoch.load(Ordering::Acquire) != self.seen_epoch.load(Ordering::Relaxed) {
            self.watch_registered();
        }
        let seen = self.seen_exits.load(Ordering::Relaxed);
        let exits = crate::exit_watch::exits();
        let told = exits != seen && self.seen_exits.compare_exchange(seen, exits, Ordering::AcqRel, Ordering::Relaxed).is_ok();
        let owed = self.owed.load(Ordering::Acquire) && self.owed.swap(false, Ordering::AcqRel);
        if !told && !owed {
            return false;
        }
        self.reclaim() > 0
    }

    /// Whether [`after_miss`](Self::after_miss) would find nothing to do: no
    /// process registered since this handle last looked, and no exit owing a
    /// sweep. A push that found nothing free is refused as `Full` at once then,
    /// calling nothing.
    #[inline(always)]
    fn miss_owes_nothing(&self) -> bool {
        self.header().registry_epoch.load(Ordering::Acquire) == self.seen_epoch.load(Ordering::Relaxed)
            && crate::exit_watch::exits() == self.seen_exits.load(Ordering::Relaxed)
            && !self.owed.load(Ordering::Acquire)
    }

    /// Sweep now: free each node and frame processes that are gone left, then
    /// the registry entries they held, so an entry handed to another process
    /// never names a gone marker's node. How many nodes and frames it freed.
    pub(crate) fn reclaim(&self) -> usize {
        let hdr = self.header();
        let gone: [u64; REGISTRY] = std::array::from_fn(|i| {
            let holder = hdr.registry[i].load(Ordering::Acquire);
            if holder_gone(holder) { holder } else { 0 }
        });
        let freed = self.sweep(&gone);
        for (entry, &holder) in hdr.registry.iter().zip(&gone) {
            if holder != 0 && entry.compare_exchange(holder, 0, Ordering::AcqRel, Ordering::Relaxed).is_ok() {
                hdr.registry_epoch.fetch_add(1, Ordering::AcqRel);
            }
        }
        freed
    }

    fn supported() -> Result<(), LinkedListError> {
        if atomic_pair::supported() {
            return Ok(());
        }
        Err(LinkedListError::IoError(io::Error::new(
            io::ErrorKind::Unsupported,
            "this processor has no lock-free 16-byte compare-and-swap",
        )))
    }

    fn shape_for(capacity: usize, value_size: usize, value_align: usize, tag: u64) -> Result<Shape, LinkedListError> {
        Shape::new(capacity, value_size, value_align, tag).map_err(|why| LinkedListError::Region(RegionError::InvalidArgument(why)))
    }

    /// Obtain the list at `path`: an empty one is laid out when the file does
    /// not exist, and an existing one is attached with its nodes in place.
    pub(crate) fn create(path: &Path, capacity: usize, value_size: usize, value_align: usize, tag: u64) -> Result<Self, LinkedListError> {
        let shape = Self::shape_for(capacity, value_size, value_align, tag)?;
        Self::supported()?;
        let (file, mut mmap) = crate::mmf_attach::create_or_attach(
            path,
            shape.total,
            // SAFETY: the builder hands `init` a zeroed, page-aligned mapping of
            // `shape.total` bytes.
            |p| unsafe { shape.init(p) },
            // SAFETY: an attacher hands the reader a mapping of at least a header.
            |p| unsafe { u64::from((*(p as *const ListHeader)).magic) },
            u64::from(LIST_MAGIC),
        )
        .map_err(|e| crate::mmf_attach::attach_error(e, LinkedListError::LayoutMismatch))?;
        shape.check(mmap.as_ptr()).map_err(|e| crate::mmf_attach::attach_error(e, LinkedListError::LayoutMismatch))?;
        crate::mmf_warm::warm_mmap(&mut mmap);
        Ok(Self::from_parts(file, mmap, shape, true))
    }

    /// Truncate the file at `path` and lay an empty list out there,
    /// invalidating every handle live peers hold.
    pub(crate) fn reset(path: &Path, capacity: usize, value_size: usize, value_align: usize, tag: u64) -> Result<Self, LinkedListError> {
        let shape = Self::shape_for(capacity, value_size, value_align, tag)?;
        Self::supported()?;
        // SAFETY: the reset hands `init` a zeroed, page-aligned mapping of
        // `shape.total` bytes.
        let (file, mut mmap) = crate::mmf_attach::reset(path, shape.total, |p| unsafe { shape.init(p) })?;
        crate::mmf_warm::warm_mmap(&mut mmap);
        Ok(Self::from_parts(file, mmap, shape, false))
    }

    /// Attach to the list at `path`, which must exist with this shape.
    pub(crate) fn open(path: &Path, capacity: usize, value_size: usize, value_align: usize, tag: u64) -> Result<Self, LinkedListError> {
        let shape = Self::shape_for(capacity, value_size, value_align, tag)?;
        Self::supported()?;
        let file = crate::region_file::open_existing(path)?;
        let len = file.metadata()?.len();
        let header = size_of::<ListHeader>();
        if len < shape.total as u64 {
            // A file of another shape is named by the field it differs in,
            // whenever it holds a whole header to read the field from.
            if len >= header as u64 {
                // SAFETY: the file is at least a header long, checked above.
                let head = unsafe { MmapOptions::new().len(header).map(&file)? };
                shape.check(head.as_ptr()).map_err(|e| crate::mmf_attach::attach_error(e, LinkedListError::LayoutMismatch))?;
            }
            return Err(LinkedListError::LayoutMismatch(Mismatch::new("length", len, shape.total as u64)));
        }
        // SAFETY: the file is at least `shape.total` bytes, checked above.
        let mmap = unsafe { MmapOptions::new().len(shape.total).map_mut(&file)? };
        shape.check(mmap.as_ptr()).map_err(|e| crate::mmf_attach::attach_error(e, LinkedListError::LayoutMismatch))?;
        Ok(Self::from_parts(file, mmap, shape, true))
    }

    /// Nodes the list can hold, the sentinel's slot among them.
    pub(crate) fn capacity(&self) -> usize {
        self.shape.capacity
    }

    /// Bytes one node takes, its links and owner word included.
    pub(crate) fn node_size(&self) -> usize {
        self.shape.node_stride
    }

    pub(crate) fn flush(&self) -> io::Result<()> {
        self.mmap.flush()
    }

    pub(crate) fn flush_async(&self) -> io::Result<()> {
        self.mmap.flush_async()
    }

    #[inline]
    fn header(&self) -> &ListHeader {
        // SAFETY: the mapping starts with a header.
        unsafe { &*(self.base as *const ListHeader) }
    }

    #[inline]
    fn node_at(&self, s: u32) -> *mut u8 {
        debug_assert!((s as usize) < self.shape.capacity);
        // SAFETY: the nodes region holds `capacity` nodes of `node_stride`.
        unsafe { self.base.add(self.shape.nodes_at + s as usize * self.shape.node_stride) }
    }

    #[inline]
    fn next(&self, s: u32) -> &AtomicPair {
        // SAFETY: a node starts with its 16-byte-aligned `next`, accessed only
        // as a pair, and the list refused to open where pairs are unsupported.
        unsafe { AtomicPair::at(self.node_at(s).add(NEXT_AT)) }
    }

    #[inline]
    fn prev(&self, s: u32) -> &AtomicPair {
        // SAFETY: as for `next`, at PREV_AT.
        unsafe { AtomicPair::at(self.node_at(s).add(PREV_AT)) }
    }

    #[inline]
    fn owner(&self, s: u32) -> &AtomicU64 {
        // SAFETY: the owner word lies OWNER_AT bytes into the node, aligned.
        unsafe { &*(self.node_at(s).add(OWNER_AT) as *const AtomicU64) }
    }

    /// Where node `s` holds its own value.
    #[inline]
    fn home(&self, s: u32) -> *mut u8 {
        // SAFETY: the value lies `value_at` bytes into the node.
        unsafe { self.node_at(s).add(self.shape.value_at) }
    }

    /// The nodes' place, for one operation to hold.
    #[inline(always)]
    fn nodes(&self) -> Nodes<'_> {
        Nodes {
            first: self.node_at(0),
            stride: self.shape.node_stride,
            value_at: self.shape.value_at,
            _list: std::marker::PhantomData,
        }
    }

    /// The nodes' place by `layout`, which is this list's: a typed list's
    /// constant, so each address it gives folds to the base and a multiple
    /// of the slot.
    #[inline(always)]
    pub(crate) fn nodes_by(&self, layout: NodeLayout) -> Nodes<'_> {
        debug_assert!(
            layout.nodes_at == self.shape.nodes_at
                && layout.stride == self.shape.node_stride
                && layout.value_at == self.shape.value_at,
            "a layout other than the list's own"
        );
        Nodes {
            // SAFETY: the nodes lie `nodes_at` bytes into the mapping.
            first: unsafe { self.base.add(layout.nodes_at) },
            stride: layout.stride,
            value_at: layout.value_at,
            _list: std::marker::PhantomData,
        }
    }

    #[inline]
    fn frame_at(&self, f: u32) -> *mut u8 {
        debug_assert!((f as usize) < self.shape.frame_count);
        // SAFETY: the frames region holds `frame_count` frames of `frame_stride`.
        unsafe { self.base.add(self.shape.frames_at + f as usize * self.shape.frame_stride) }
    }

    /// A frame's word: its generation in the low half, odd while the frame is
    /// free or its writer fills it, and the process that took it in the high
    /// half, zero while it is free.
    #[inline]
    fn frame_word(&self, f: u32) -> &AtomicU64 {
        // SAFETY: a frame starts with its aligned word.
        unsafe { &*(self.frame_at(f) as *const AtomicU64) }
    }

    /// A frame's taker: the holder word, pid and start stamp, of the process
    /// that took it last, written before the frame's word names that pid.
    #[inline]
    fn frame_taker(&self, f: u32) -> &AtomicU64 {
        // SAFETY: the taker's word lies FRAME_TAKER_AT bytes into the frame,
        // aligned.
        unsafe { &*(self.frame_at(f).add(FRAME_TAKER_AT) as *const AtomicU64) }
    }

    /// Whether the process that took frame `f`, whose word read `word`, is
    /// gone. A taker word that names another pid than `word` belongs to a
    /// later take, so the frame has moved on and is not.
    fn frame_taker_gone(&self, f: u32, word: u64) -> bool {
        let pid = (word >> 32) as u32;
        let taker = self.frame_taker(f).load(Ordering::Acquire);
        pid != 0 && process_identity::holder_pid(taker) == pid && holder_gone(taker)
    }

    #[inline]
    fn frame_value(&self, f: u32) -> *mut u8 {
        // SAFETY: the value lies `frame_value_at` bytes into the frame.
        unsafe { self.frame_at(f).add(self.shape.frame_value_at) }
    }

    /// The free-stack link of a free frame, in its value's first word.
    #[inline]
    fn frame_link(&self, f: u32) -> &AtomicU64 {
        // SAFETY: as for `free_link`.
        unsafe { &*(self.frame_value(f) as *const AtomicU64) }
    }

    /// The node a handle names, when its slot is one a node can have.
    #[inline]
    fn slot_in_range(&self, h: u64) -> Option<u32> {
        let s = slot_of(h);
        (s != 0 && (s as usize) < self.shape.capacity).then_some(s)
    }

    // ---- free nodes and frames -------------------------------------------

    /// A node for this process to fill: its slot and generation, from the
    /// free stack or the bump counter, or from a sweep an exit owed. None
    /// when none is left.
    #[inline]
    fn take_node(&self, n: Nodes<'_>) -> Option<(u32, u32)> {
        match self.take_node_once(n) {
            Some(s) => Some((s, Self::claim(n, s))),
            None => self.take_node_after_miss(n),
        }
    }

    /// One node off the free stack, or off the bump counter when the stack
    /// is empty and the counter not spent.
    #[inline(always)]
    fn take_node_once(&self, n: Nodes<'_>) -> Option<u32> {
        let hdr = self.header();
        match Self::pop(&hdr.node_free, |s| n.free_link(s), self.shape.capacity) {
            Popped::Item(s) => Some(s),
            Popped::Empty => Self::bump(&hdr.node_free, &hdr.node_bump, &self.node_limit),
            Popped::Exhausted => None,
        }
    }

    /// [`take_node`](Self::take_node) once the stack and the bump counter came
    /// up empty: each sweep an exit owed, and a second look after it.
    #[cold]
    #[inline(never)]
    fn take_node_after_miss(&self, n: Nodes<'_>) -> Option<(u32, u32)> {
        while self.after_miss() {
            if let Some(s) = self.take_node_once(n) {
                return Some((s, Self::claim(n, s)));
            }
        }
        None
    }

    /// Mark node `s`, just taken, as this process's: its owner word, then a
    /// `next` naming the sentinel under its generation with the free flag
    /// cleared, so a sweep sees a taken node whose taker can die before it
    /// is linked. Its generation.
    #[inline(always)]
    fn claim(n: Nodes<'_>, s: u32) -> u32 {
        Self::claim_as(n, s, process_identity::current())
    }

    /// [`claim`](Self::claim) with this process's holder word in hand.
    #[inline(always)]
    fn claim_as(n: Nodes<'_>, s: u32, holder: u64) -> u32 {
        let next = n.next(s);
        let g = own(next.load_hi());
        n.owner(s).store(holder, Ordering::Relaxed);
        // The owner before the claim: a sweep that reads the node taken reads
        // its taker, never the one before.
        fence(Ordering::Release);
        // A half at a time, the high half last: every swap aimed at the
        // node's earlier lives expects an earlier generation in the high half.
        next.store_lo(SENTINEL);
        next.store_hi(u64::from(g));
        // The claim before the value's stores: a reader copying the node's
        // last value checks its `next` after.
        fence(Ordering::Release);
        g
    }

    /// Free node `s`, which this caller swapped out of the list or found
    /// abandoned, `w` its `next` as last read: the swap steps its
    /// generation, and the node and its frame go back to their stacks.
    /// False when another writer freed it first.
    fn free_node(&self, s: u32, w: u128) -> bool {
        self.free_node_in(self.nodes(), s, w)
    }

    /// [`free_node`](Self::free_node) with the nodes' place in hand. The swap
    /// takes the high half alone: once a node is marked or taken nothing else
    /// swaps its word, and a free node's low half is never read.
    #[inline]
    fn free_node_in(&self, n: Nodes<'_>, s: u32, w: u128) -> bool {
        let h = hi(w);
        if n.next(s).compare_exchange_hi(h, u64::from(own(h).wrapping_add(1)) | FREE).is_err() {
            return false;
        }
        note!(self, ListFreed, s, link(s, own(h)), h, 0);
        if vloc(h) != HOME {
            self.give_frame((vloc(h) - 1) as u32);
        }
        Self::push(&self.header().node_free, s, |s| n.free_link(s));
        true
    }

    /// `h`, an unmarked high half, as this handle marks it: naming the handle
    /// when the value is at home and the handle holds a registry entry.
    #[inline]
    fn mark_of(&self, h: u64) -> u64 {
        if vloc(h) == HOME { h | self.home_mark } else { h | MARK }
    }

    /// Free node `s`, which this caller marked and has seen out of the list,
    /// `w` its frozen `next`: by stores when the mark names this handle,
    /// which nothing else frees, else as [`free_node_in`](Self::free_node_in).
    #[inline]
    fn free_marked_in(&self, n: Nodes<'_>, s: u32, w: u128) {
        let h = hi(w);
        if h & NAMED == 0 {
            self.free_node_in(n, s, w);
            return;
        }
        n.next(s).store_hi(u64::from(own(h).wrapping_add(1)) | FREE);
        note!(self, ListFreed, s, link(s, own(h)), h, 0);
        Self::push(&self.header().node_free, s, |s| n.free_link(s));
    }

    /// Free node `s`, which this caller swapped out of the list for another
    /// writer, `w` its frozen `next`: unless its mark names a handle whose
    /// process lives, which frees it itself.
    fn free_helped(&self, n: Nodes<'_>, s: u32, w: u128) {
        if !self.marker_lives(hi(w)) {
            self.free_node_in(n, s, w);
        }
    }

    /// Whether `h` is a named mark whose registry entry names a process that
    /// is not gone.
    fn marker_lives(&self, h: u64) -> bool {
        if h & NAMED == 0 {
            return false;
        }
        let holder = self.header().registry[marker_entry(h)].load(Ordering::Acquire);
        holder != 0 && !holder_gone(holder)
    }

    /// A frame for this process to fill, odd while it is filled; None when
    /// none is left.
    fn take_frame(&self) -> Option<u32> {
        let hdr = self.header();
        loop {
            let f = match Self::pop(&hdr.frame_free, |f| self.frame_link(f), self.shape.frame_count) {
                Popped::Item(f) => Some(f),
                Popped::Empty => Self::bump(&hdr.frame_free, &hdr.frame_bump, &self.frame_limit),
                Popped::Exhausted => None,
            };
            if let Some(f) = f {
                let word = self.frame_word(f);
                let g = (word.load(Ordering::Acquire) as u32) | 1;
                // The taker before the word that names its pid, so a sweep that
                // reads the word reads its start beside it.
                let me = process_identity::current();
                self.frame_taker(f).store(me, Ordering::Relaxed);
                word.store(u64::from(g) | (u64::from(process_identity::holder_pid(me)) << 32), Ordering::Release);
                fence(Ordering::Release);
                return Some(f);
            }
            if !self.after_miss() {
                return None;
            }
        }
    }

    /// Frame `f`, filled, as one a reader may copy: its generation even.
    fn frame_filled(&self, f: u32) {
        let word = self.frame_word(f);
        let w = word.load(Ordering::Relaxed);
        word.store(w.wrapping_add(1), Ordering::Release);
    }

    /// Free frame `f`: its generation steps to the next odd number, so a copy
    /// begun before refuses it and none begun while it is free accepts it,
    /// and it goes back to the stack, whose link lands in the value's first
    /// word after that step.
    #[cold]
    fn give_frame(&self, f: u32) {
        let word = self.frame_word(f);
        let g = (word.load(Ordering::Acquire) as u32).wrapping_add(1) | 1;
        word.store(u64::from(g), Ordering::Relaxed);
        fence(Ordering::Release);
        Self::push(&self.header().frame_free, f, |f| self.frame_link(f));
    }

    /// Pop the Treiber stack `head`, whose items link through `link_of`.
    fn pop<'a>(head: &AtomicU64, link_of: impl Fn(u32) -> &'a AtomicU64, count: usize) -> Popped {
        let mut top = head.load(Ordering::Acquire);
        loop {
            // One past the top item, so an empty stack's zero and the
            // exhausted mark both step down to a number past any count.
            let s = u64::from(top as u32).wrapping_sub(1);
            if s >= count as u64 {
                return if top as u32 == EMPTY { Popped::Empty } else { Popped::Exhausted };
            }
            let s = s as u32;
            let below = link_of(s).load(Ordering::Acquire) & 0xFFFF_FFFF;
            let next = (((top >> 32).wrapping_add(1)) << 32) | below;
            match head.compare_exchange_weak(top, next, Ordering::AcqRel, Ordering::Acquire) {
                Ok(_) => return Popped::Item(s),
                Err(now) => top = now,
            }
        }
    }

    /// An index off the bump counter `counter`, below `limit`. Finding none
    /// left leaves the exhausted mark in the free-stack head `head`, which the
    /// taker found empty.
    #[inline(always)]
    fn bump(head: &AtomicU64, counter: &AtomicU64, limit: &crate::bump::Limit) -> Option<u32> {
        let Some(i) = crate::bump::take64(counter, limit) else {
            Self::mark_exhausted(head);
            return None;
        };
        Some(i as u32)
    }

    /// Leave the exhausted mark in the free-stack head `head` while it is
    /// empty, stepping its tag as a push or pop does. A head that an item
    /// was freed onto meanwhile is left as it is, and the bump's next
    /// refusal marks it once that item is taken.
    #[cold]
    #[inline(never)]
    fn mark_exhausted(head: &AtomicU64) {
        let mut top = head.load(Ordering::Acquire);
        while top as u32 == EMPTY {
            let next = (((top >> 32).wrapping_add(1)) << 32) | u64::from(EXHAUSTED);
            match head.compare_exchange_weak(top, next, Ordering::AcqRel, Ordering::Acquire) {
                Ok(_) => return,
                Err(now) => top = now,
            }
        }
    }

    fn push<'a>(head: &AtomicU64, s: u32, link_of: impl Fn(u32) -> &'a AtomicU64) {
        let mut top = head.load(Ordering::Acquire);
        loop {
            link_of(s).store(top & 0xFFFF_FFFF, Ordering::Relaxed);
            let next = (((top >> 32).wrapping_add(1)) << 32) | u64::from(s + 1);
            match head.compare_exchange_weak(top, next, Ordering::AcqRel, Ordering::Acquire) {
                Ok(_) => return,
                Err(now) => top = now,
            }
        }
    }

    // ---- values ----------------------------------------------------------

    /// The value node `s` holds, `h` its `next` high half as read, as `read`
    /// copies it from the value's first byte; None when its frame moved
    /// during the copy. A value another writer is replacing reads as some
    /// bytes, which the caller's check of `next` discards.
    fn copy_value<V>(&self, s: u32, h: u64, read: &mut impl FnMut(*const u8) -> V) -> Option<V> {
        let v = vloc(h);
        if v == HOME {
            return Some(read(self.home(s)));
        }
        let f = (v - 1) as u32;
        let word = self.frame_word(f);
        let g = word.load(Ordering::Acquire) as u32;
        let value = read(self.frame_value(f));
        fence(Ordering::Acquire);
        (g & 1 == 0 && word.load(Ordering::Relaxed) as u32 == g).then_some(value)
    }

    // ---- pushes ----------------------------------------------------------

    /// Take a node and have `write` fill its value: its slot and generation,
    /// None when no node is left.
    #[inline]
    fn fill(&self, n: Nodes<'_>, write: impl FnOnce(*mut u8)) -> Option<(u32, u32)> {
        let (s, g) = self.take_node(n)?;
        write(n.home(s));
        Some((s, g))
    }

    /// Add a value at the front, written by `write` into a place nobody else
    /// writes, `value_size` bytes rounded up to a whole word, aligned to the
    /// value's alignment and to eight: its handle, which is never zero, or
    /// None when the list is full.
    pub(crate) fn push_front(&self, write: impl FnOnce(*mut u8)) -> Option<NonZeroU64> {
        let nodes = self.nodes();
        let (n, g) = self.fill(nodes, write)?;
        let me = link(n, g);
        let hdr = self.header();
        // The node's own words a half at a time, as its claim wrote them; the
        // claim left its generation in `next`'s high half.
        let prev = nodes.prev(n);
        prev.store_lo(SENTINEL);
        prev.store_hi(u64::from(g));
        loop {
            let first = hdr.head_next.load(Ordering::Acquire);
            nodes.next(n).store_lo(first);
            if hdr.head_next.compare_exchange(first, me, Ordering::AcqRel, Ordering::Acquire).is_ok() {
                note!(self, ListLinked, n, me, first, 0);
                if first == SENTINEL {
                    // The list was empty, so the node is its last as well.
                    self.move_last_hint(SENTINEL, me);
                } else {
                    nodes.prev(slot_of(first)).store(pair(me, u64::from(gen_of(first))));
                }
                return NonZeroU64::new(me);
            }
        }
    }

    /// Add a value at the back, written by `write` as for
    /// [`push_front`](Self::push_front): its handle, or None when the list is
    /// full. The node's `next` already names the sentinel from its claim.
    ///
    /// One attempt runs here holding nothing across a call: a node off the
    /// free stack or the bump counter, then one swap where the sentinel's
    /// hint points. A pid not yet cached, or a take that finds nothing,
    /// hands the whole push to [`push_back_general`](Self::push_back_general);
    /// a swap that does not land hands the node to
    /// [`push_back_after_walk`](Self::push_back_after_walk).
    #[inline]
    pub(crate) fn push_back(&self, write: impl FnOnce(*mut u8)) -> Option<NonZeroU64> {
        self.push_back_by(self.nodes(), write)
    }

    /// [`push_back`](Self::push_back) with the nodes' place in hand.
    #[inline]
    pub(crate) fn push_back_by(&self, nodes: Nodes<'_>, write: impl FnOnce(*mut u8)) -> Option<NonZeroU64> {
        let holder = process_identity::cached();
        if holder == 0 {
            return self.push_back_general(write);
        }
        let hdr = self.header();
        let n = match Self::take_at_once(hdr, nodes, self.node_limit) {
            Ok(n) => n,
            Err(Missed::Moved) => return self.push_back_general(write),
            Err(Missed::Empty) if self.miss_owes_nothing() => return None,
            Err(Missed::Empty) => return self.push_back_after_miss(write),
        };
        let g = Self::claim_as(nodes, n, holder);
        write(nodes.home(n));
        self.link_back(nodes, n, g)
    }

    /// [`push_back`](Self::push_back) once the free stack and the bump
    /// counter came up empty: None, the list full, unless a sweep an exit
    /// owed frees a node, which the general path then takes.
    #[cold]
    #[inline(never)]
    fn push_back_after_miss(&self, write: impl FnOnce(*mut u8)) -> Option<NonZeroU64> {
        if !self.after_miss() {
            return None;
        }
        self.push_back_general(write)
    }

    /// [`push_back`](Self::push_back) by the general path: a node from
    /// wherever one is left, a sweep an exit owed among them.
    #[cold]
    #[inline(never)]
    fn push_back_general(&self, write: impl FnOnce(*mut u8)) -> Option<NonZeroU64> {
        let nodes = self.nodes();
        let (n, g) = self.fill(nodes, write)?;
        self.link_back(nodes, n, g)
    }

    /// One take from the free stack, or the bump counter when the stack is
    /// empty and the counter not spent: the slot, or what kept it from one.
    /// It calls out only to leave the exhausted mark when the counter
    /// refuses.
    #[inline(always)]
    fn take_at_once(hdr: &ListHeader, n: Nodes<'_>, limit: crate::bump::Limit) -> Result<u32, Missed> {
        let top = hdr.node_free.load(Ordering::Acquire);
        // One past the top item, so an empty stack's zero and the exhausted
        // mark both step down to a number past any capacity. A capacity is
        // below 2^30, so the limit's low half is all of it.
        let s = (top as u32).wrapping_sub(1);
        if s < limit.value() as u32 {
            let below = n.free_link(s).load(Ordering::Acquire) & 0xFFFF_FFFF;
            let next = (((top >> 32).wrapping_add(1)) << 32) | below;
            return if hdr.node_free.compare_exchange(top, next, Ordering::AcqRel, Ordering::Acquire).is_ok() {
                Ok(s)
            } else {
                Err(Missed::Moved)
            };
        }
        if top as u32 != EMPTY {
            return Err(Missed::Empty);
        }
        Self::bump(&hdr.node_free, &hdr.node_bump, &limit).ok_or(Missed::Empty)
    }

    /// Link node `n`, of generation `g`, filled and claimed, at the back: its
    /// handle.
    #[inline(always)]
    fn link_back(&self, nodes: Nodes<'_>, n: u32, g: u32) -> Option<NonZeroU64> {
        let me = link(n, g);
        let hdr = self.header();
        // The node's `prev`, a half at a time as its `next` was claimed.
        let prev = nodes.prev(n);
        prev.store_hi(u64::from(g));
        // One swap at the place the sentinel's hint names: the front of an
        // empty list, or a node live under the hint's generation and unmarked,
        // wherever its value lies, which the swap finds last or does not land.
        let hint = hdr.head_prev.load(Ordering::Acquire);
        prev.store_lo(hint);
        let linked = if hint == SENTINEL {
            hdr.head_next.compare_exchange(SENTINEL, me, Ordering::AcqRel, Ordering::Acquire).is_ok()
        } else {
            let last = nodes.next(slot_of(hint));
            let h = last.load_hi();
            h & !VLOC_MASK == u64::from(gen_of(hint)) && last.compare_exchange(pair(SENTINEL, h), pair(me, h)).is_ok()
        };
        if !linked {
            return self.push_back_after_walk(me);
        }
        note!(self, ListLinked, n, me, SENTINEL, 1);
        hdr.head_prev.store(me, Ordering::Release);
        NonZeroU64::new(me)
    }

    /// [`push_back`](Self::push_back) once the sentinel's hint led to no
    /// swap: the node of handle `me` linked after the last node a walk finds,
    /// or at the front of an empty list, its `prev` set to match. It takes
    /// the nodes' place afresh, so the caller holds nothing for it.
    #[cold]
    #[inline(never)]
    fn push_back_after_walk(&self, me: u64) -> Option<NonZeroU64> {
        let n = self.nodes();
        let s = slot_of(me);
        let hdr = self.header();
        loop {
            let linked = match self.find_last() {
                None => {
                    n.prev(s).store_lo(SENTINEL);
                    hdr.head_next.compare_exchange(SENTINEL, me, Ordering::AcqRel, Ordering::Acquire).is_ok()
                }
                Some((l, lw)) => {
                    n.prev(s).store_lo(l);
                    n.next(slot_of(l)).compare_exchange(lw, pair(me, hi(lw))).is_ok()
                }
            };
            if linked {
                note!(self, ListLinked, s, me, SENTINEL, 1);
                hdr.head_prev.store(me, Ordering::Release);
                return NonZeroU64::new(me);
            }
        }
    }

    // ---- removals --------------------------------------------------------

    /// Remove the first node: its value as `read` copies it from the value's
    /// first byte, or None when the list is empty. `read` may run more than
    /// once, and only its last copy is the value.
    ///
    /// One attempt runs here holding nothing across a call, on a first node
    /// of the usual shape: live, unmarked, its value at home. Any other
    /// shape, and a mark that does not land, hands the whole pop to
    /// [`pop_front_general`](Self::pop_front_general).
    #[inline]
    pub(crate) fn pop_front<V>(&self, read: impl FnMut(*const u8) -> V) -> Option<V> {
        self.pop_front_by(self.nodes(), read)
    }

    /// [`pop_front`](Self::pop_front) with the nodes' place in hand.
    #[inline]
    pub(crate) fn pop_front_by<V>(&self, nodes: Nodes<'_>, mut read: impl FnMut(*const u8) -> V) -> Option<V> {
        let hdr = self.header();
        let first = hdr.head_next.load(Ordering::Acquire);
        if first == SENTINEL {
            return None;
        }
        let s = slot_of(first);
        let link = nodes.next(s);
        // A live node holding its value at home has a high half of its
        // generation alone.
        let h = u64::from(gen_of(first));
        if link.load_hi() != h {
            return self.pop_front_general(read);
        }
        let value = read(nodes.home(s));
        let mh = h | self.home_mark;
        if link.compare_exchange_hi(h, mh).is_err() {
            return self.pop_front_general(read);
        }
        let succ = link.load_lo();
        let marked = pair(succ, mh);
        note!(self, ListMarked, s, first, h, 0);
        if hdr.head_next.compare_exchange(first, succ, Ordering::AcqRel, Ordering::Acquire).is_err() {
            return self.pop_front_unlink(first, marked, value);
        }
        self.free_marked_in(nodes, s, marked);
        if succ == SENTINEL {
            self.move_last_hint(first, SENTINEL);
        } else {
            // The low half alone, naming the sentinel: on a node reused
            // meanwhile it sends a search to the front, where every search
            // may start.
            nodes.prev(slot_of(succ)).store_lo(SENTINEL);
        }
        Some(value)
    }

    /// [`pop_front`](Self::pop_front)'s ending when the front moved after its
    /// mark: node `first`, `marked` its frozen `next`, swapped out where it
    /// now lies, and `value` its value.
    #[cold]
    #[inline(never)]
    fn pop_front_unlink<V>(&self, first: u64, marked: u128, value: V) -> Option<V> {
        self.unlink(first, marked);
        Some(value)
    }

    /// [`pop_front`](Self::pop_front) by the general path, for a first node
    /// of any shape.
    #[cold]
    #[inline(never)]
    fn pop_front_general<V>(&self, mut read: impl FnMut(*const u8) -> V) -> Option<V> {
        let nodes = self.nodes();
        let hdr = self.header();
        loop {
            let first = hdr.head_next.load(Ordering::Acquire);
            if first == SENTINEL {
                return None;
            }
            let s = slot_of(first);
            let link = nodes.next(s);
            let h = link.load_hi();
            // A live node holding its value at home has a high half of its
            // generation alone.
            let value = if h == u64::from(gen_of(first)) {
                read(nodes.home(s))
            } else {
                if own(h) != gen_of(first) || h & FREE != 0 {
                    continue;
                }
                if h & MARK != 0 {
                    self.help_out_front(nodes, first);
                    continue;
                }
                let Some(value) = self.copy_value(s, h, &mut read) else { continue };
                value
            };
            // The mark swaps the high half alone; every swap of the whole word
            // expects it unmarked, so from here the low half holds still.
            let mh = self.mark_of(h);
            if link.compare_exchange_hi(h, mh).is_err() {
                continue;
            }
            let succ = link.load_lo();
            let marked = pair(succ, mh);
            note!(self, ListMarked, s, first, h, 0);
            if hdr.head_next.compare_exchange(first, succ, Ordering::AcqRel, Ordering::Acquire).is_ok() {
                self.free_marked_in(nodes, s, marked);
                if succ == SENTINEL {
                    self.move_last_hint(first, SENTINEL);
                } else {
                    // The low half alone, naming the sentinel: on a node reused
                    // meanwhile it sends a search to the front, where every
                    // search may start.
                    nodes.prev(slot_of(succ)).store_lo(SENTINEL);
                }
            } else {
                self.unlink(first, marked);
            }
            return Some(value);
        }
    }

    /// The first node `first`, which another writer removed and has not yet
    /// swapped out of the front: swapped out for that writer, from its frozen
    /// `next` read whole, and freed unless that writer frees it itself.
    #[cold]
    #[inline(never)]
    fn help_out_front(&self, n: Nodes<'_>, first: u64) {
        let s = slot_of(first);
        let w = n.next(s).load();
        if own(hi(w)) == gen_of(first)
            && hi(w) & FREE == 0
            && self.header().head_next.compare_exchange(first, lo(w), Ordering::AcqRel, Ordering::Acquire).is_ok()
        {
            self.free_helped(n, s, w);
            self.after_unlink(SENTINEL, first, lo(w));
        }
    }

    /// Remove the last node: its value as `read` copies it, as for
    /// [`pop_front`](Self::pop_front), or None when the list is empty.
    pub(crate) fn pop_back<V>(&self, mut read: impl FnMut(*const u8) -> V) -> Option<V> {
        loop {
            let (l, w) = self.find_last()?;
            let s = slot_of(l);
            let h = hi(w);
            let Some(value) = self.copy_value(s, h, &mut read) else { continue };
            let marked = pair(lo(w), self.mark_of(h));
            if self.next(s).compare_exchange(w, marked).is_err() {
                continue;
            }
            note!(self, ListMarked, s, l, h, 1);
            self.unlink(l, marked);
            return Some(value);
        }
    }

    /// Remove the node `h` names: its value as `read` copies it, as for
    /// [`pop_front`](Self::pop_front), or None when the handle names no node
    /// in the list.
    pub(crate) fn remove<V>(&self, h: u64, mut read: impl FnMut(*const u8) -> V) -> Option<V> {
        let s = self.slot_in_range(h)?;
        let link = self.next(s);
        loop {
            let wh = link.load_hi();
            if own(wh) != gen_of(h) || wh & (MARK | FREE) != 0 {
                return None;
            }
            let Some(value) = self.copy_value(s, wh, &mut read) else { continue };
            // As in `pop_front`: the mark swaps the high half alone, after which
            // the low half holds still.
            let mh = self.mark_of(wh);
            if link.compare_exchange_hi(wh, mh).is_ok() {
                let marked = pair(link.load_lo(), mh);
                note!(self, ListMarked, s, h, wh, 2);
                self.unlink(h, marked);
                return Some(value);
            }
        }
    }

    /// Swap the node `x` this caller marked, `mw` its frozen `next`, out of
    /// the list and free it. When another writer swapped it out first, that
    /// writer freed it, unless the mark names this handle.
    fn unlink(&self, x: u64, mw: u128) {
        let succ = lo(mw);
        loop {
            let swapped = match self.find_pred(x) {
                None => {
                    if hi(mw) & NAMED != 0 {
                        self.free_marked_in(self.nodes(), slot_of(x), mw);
                    }
                    return;
                }
                Some(Pred::Sentinel) => {
                    self.header().head_next.compare_exchange(x, succ, Ordering::AcqRel, Ordering::Acquire).is_ok()
                }
                Some(Pred::Node(p, pw)) => self.next(slot_of(p)).compare_exchange(pw, pair(succ, hi(pw))).is_ok(),
            };
            if swapped {
                self.free_marked_in(self.nodes(), slot_of(x), mw);
                let p = self.prev_named(x);
                self.after_unlink(p, x, succ);
                return;
            }
        }
    }

    /// The predecessor `x` was swapped out from, as its `prev` names it, for
    /// the hints that follow; the sentinel when the hint is gone.
    fn prev_named(&self, x: u64) -> u64 {
        let ph = self.prev(slot_of(x)).load();
        if own(hi(ph)) == gen_of(x) { lo(ph) } else { SENTINEL }
    }

    /// The hints after `x` left from between `p` and `succ`: `succ`'s `prev`
    /// names `p`, and when `succ` is the sentinel its `prev` moves from `x`
    /// to `p`. Whether the sentinel's hint moved.
    fn after_unlink(&self, p: u64, x: u64, succ: u64) -> bool {
        if succ == SENTINEL {
            return self.move_last_hint(x, p);
        }
        self.set_prev_hint(succ, p);
        false
    }

    /// Point the sentinel's `prev` at `to` if it still names `from`; a push
    /// or a removal that moved it since keeps theirs.
    fn move_last_hint(&self, from: u64, to: u64) -> bool {
        self.header().head_prev.compare_exchange(from, to, Ordering::AcqRel, Ordering::Relaxed).is_ok()
    }

    /// Point node `x`'s `prev` at `p`. A plain store: a node reused since
    /// gets a hint naming its last generation, which every reader of a hint
    /// checks.
    fn set_prev_hint(&self, x: u64, p: u64) {
        self.prev(slot_of(x)).store(pair(p, u64::from(gen_of(x))));
    }

    // ---- walks -----------------------------------------------------------

    /// Walk on from `pred`, whose `next` named `cur`, until `cur` is `target`
    /// or the sentinel, swapping out each marked node passed and freeing it
    /// unless its marker frees it itself:
    /// the predecessor where it stopped and the link it stopped at. None
    /// when a node under the walk changed, and the walk starts again from
    /// the sentinel.
    fn search(&self, mut pred: Pred, mut cur: u64, target: u64) -> Option<(Pred, u64)> {
        loop {
            if cur == target || cur == SENTINEL {
                return Some((pred, cur));
            }
            let s = slot_of(cur);
            let w = self.next(s).load();
            let h = hi(w);
            if own(h) != gen_of(cur) || h & FREE != 0 {
                return None;
            }
            if h & MARK == 0 {
                pred = Pred::Node(cur, w);
                cur = lo(w);
                continue;
            }
            let succ = lo(w);
            pred = match pred {
                Pred::Sentinel => {
                    if self.header().head_next.compare_exchange(cur, succ, Ordering::AcqRel, Ordering::Acquire).is_err() {
                        return None;
                    }
                    Pred::Sentinel
                }
                Pred::Node(p, pw) => {
                    let swapped = pair(succ, hi(pw));
                    if self.next(slot_of(p)).compare_exchange(pw, swapped).is_err() {
                        return None;
                    }
                    Pred::Node(p, swapped)
                }
            };
            note!(self, ListHelped, s, cur, h, 0);
            self.free_helped(self.nodes(), s, w);
            self.after_unlink(pred.link(), cur, succ);
            cur = succ;
        }
    }

    /// The last node and its `next`, which names the sentinel; None when the
    /// list is empty. Starts from the sentinel's `prev` when it names a live
    /// node.
    fn find_last(&self) -> Option<(u64, u128)> {
        let hint = self.header().head_prev.load(Ordering::Acquire);
        if hint != SENTINEL {
            let w = self.next(slot_of(hint)).load();
            if own(hi(w)) == gen_of(hint)
                && hi(w) & (MARK | FREE) == 0
                && let Some((pred, _)) = self.search(Pred::Node(hint, w), lo(w), SENTINEL)
            {
                return Self::last_of(pred);
            }
        }
        loop {
            let first = self.header().head_next.load(Ordering::Acquire);
            if let Some((pred, _)) = self.search(Pred::Sentinel, first, SENTINEL) {
                return Self::last_of(pred);
            }
        }
    }

    fn last_of(pred: Pred) -> Option<(u64, u128)> {
        match pred {
            Pred::Sentinel => None,
            Pred::Node(l, w) => Some((l, w)),
        }
    }

    /// The node whose `next` names `x`; None when no node does, `x` having
    /// left the list. Starts from `x`'s `prev` when it names a live node.
    fn find_pred(&self, x: u64) -> Option<Pred> {
        let ph = self.prev(slot_of(x)).load();
        if own(hi(ph)) == gen_of(x) {
            let p = lo(ph);
            let start = if p == SENTINEL {
                Some((Pred::Sentinel, self.header().head_next.load(Ordering::Acquire)))
            } else {
                let pw = self.next(slot_of(p)).load();
                (own(hi(pw)) == gen_of(p) && hi(pw) & (MARK | FREE) == 0).then_some((Pred::Node(p, pw), lo(pw)))
            };
            if let Some((pred, cur)) = start
                && let Some((pred, at)) = self.search(pred, cur, x)
                && at == x
            {
                return Some(pred);
            }
        }
        loop {
            let first = self.header().head_next.load(Ordering::Acquire);
            if let Some((pred, at)) = self.search(Pred::Sentinel, first, x) {
                return (at == x).then_some(pred);
            }
        }
    }

    /// The first unmarked node from `cur` on, or the sentinel.
    fn first_from(&self, mut cur: u64) -> Option<u64> {
        let mut steps = 0usize;
        loop {
            if cur == SENTINEL {
                return Some(SENTINEL);
            }
            let w = self.next(slot_of(cur)).load();
            let h = hi(w);
            if own(h) != gen_of(cur) || h & FREE != 0 {
                return None;
            }
            if h & MARK == 0 {
                return Some(cur);
            }
            cur = lo(w);
            steps += 1;
            if steps > self.shape.capacity {
                return Some(SENTINEL);
            }
        }
    }

    // ---- reads -----------------------------------------------------------

    /// The first node's handle, or the sentinel's when the list is empty.
    pub(crate) fn first(&self) -> u64 {
        loop {
            if let Some(at) = self.first_from(self.header().head_next.load(Ordering::Acquire)) {
                return at;
            }
        }
    }

    /// The last node's handle, or the sentinel's when the list is empty.
    pub(crate) fn last(&self) -> u64 {
        self.find_last().map_or(SENTINEL, |(l, _)| l)
    }

    /// The handle after `h`'s node, the sentinel's after the last; the
    /// sentinel's handle gives the first node. None when `h` names no node.
    pub(crate) fn next_of(&self, h: u64) -> Option<u64> {
        if h == SENTINEL {
            return Some(self.first());
        }
        let s = self.slot_in_range(h)?;
        loop {
            let w = self.next(s).load();
            if own(hi(w)) != gen_of(h) || hi(w) & FREE != 0 {
                return None;
            }
            if let Some(at) = self.first_from(lo(w)) {
                return Some(at);
            }
        }
    }

    /// The handle before `h`'s node, the sentinel's before the first; the
    /// sentinel's handle gives the last node. None when `h` names no node
    /// in the list.
    pub(crate) fn prev_of(&self, h: u64) -> Option<u64> {
        if h == SENTINEL {
            return Some(self.last());
        }
        let s = self.slot_in_range(h)?;
        let w = self.next(s).load();
        if own(hi(w)) != gen_of(h) || hi(w) & (MARK | FREE) != 0 {
            return None;
        }
        self.find_pred(h).map(Pred::link)
    }

    /// The value `h`'s node holds, as `read` copies it, as for
    /// [`pop_front`](Self::pop_front); None when `h` names no node in the
    /// list.
    pub(crate) fn get<V>(&self, h: u64, mut read: impl FnMut(*const u8) -> V) -> Option<V> {
        let s = self.slot_in_range(h)?;
        let mut w = self.next(s).load();
        loop {
            let wh = hi(w);
            if own(wh) != gen_of(h) || wh & (MARK | FREE) != 0 {
                return None;
            }
            let copied = self.copy_value(s, wh, &mut read);
            fence(Ordering::Acquire);
            let again = self.next(s).load();
            // The value read is the node's while its generation and its
            // value's place held; a removal begun meanwhile leaves both.
            if let Some(value) = copied
                && unmarked(hi(again)) == wh
            {
                return Some(value);
            }
            w = again;
        }
    }

    /// Replace the value `h`'s node holds with one `write` fills in, as for
    /// [`push_front`](Self::push_front). `InvalidHandle` when `h` names no
    /// node in the list, `Full` when no frame is free.
    pub(crate) fn set(&self, h: u64, write: impl FnOnce(*mut u8)) -> Result<(), LinkedListError> {
        let s = self.slot_in_range(h).ok_or(LinkedListError::InvalidHandle)?;
        let f = self.take_frame().ok_or(LinkedListError::Region(RegionError::Full))?;
        write(self.frame_value(f));
        self.frame_filled(f);
        let mut w = self.next(s).load();
        loop {
            let wh = hi(w);
            if own(wh) != gen_of(h) || wh & (MARK | FREE) != 0 {
                self.give_frame(f);
                return Err(LinkedListError::InvalidHandle);
            }
            let placed = (wh & !VLOC_MASK) | (u64::from(f + 1) << VLOC_SHIFT);
            match self.next(s).compare_exchange(w, pair(lo(w), placed)) {
                Ok(_) => {
                    if vloc(wh) != HOME {
                        self.give_frame((vloc(wh) - 1) as u32);
                    }
                    return Ok(());
                }
                Err(now) => w = now,
            }
        }
    }

    /// Nodes in the list, counted by one walk from the front.
    pub(crate) fn len(&self) -> usize {
        'walk: loop {
            let mut n = 0usize;
            let mut cur = self.header().head_next.load(Ordering::Acquire);
            let mut steps = 0usize;
            while cur != SENTINEL && steps <= self.shape.capacity {
                let w = self.next(slot_of(cur)).load();
                let h = hi(w);
                if own(h) != gen_of(cur) || h & FREE != 0 {
                    continue 'walk;
                }
                if h & MARK == 0 {
                    n += 1;
                }
                cur = lo(w);
                steps += 1;
            }
            return n;
        }
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.first() == SENTINEL
    }

    /// Hand `visit` each node in the list front to back, its handle beside
    /// its value as `copy` reads it from the value's first byte: one walk,
    /// begun again from the front when a node is freed under it, `visit`
    /// receiving `None` at each beginning. `nodes` is the nodes' place, which
    /// a typed list builds from its constant layout.
    pub(crate) fn walk_by<V>(&self, nodes: Nodes<'_>, mut copy: impl FnMut(*const u8) -> V, mut visit: impl FnMut(Option<(u64, V)>)) {
        // Nodes one pass may step through: every node once, and the
        // sentinel's slot, so a pass over a list that loops ends.
        let budget = self.shape.capacity + 1;
        'walk: loop {
            visit(None);
            let mut cur = self.header().head_next.load(Ordering::Acquire);
            let mut left = budget;
            while cur != SENTINEL && left != 0 {
                left -= 1;
                let s = slot_of(cur);
                // The value and the link's low half first, then the high half
                // that says both were the node's: a node's generation only
                // rises, so a high half naming it brackets the reads before.
                // A value outside the node waits on the frame below.
                let v = copy(nodes.home(s));
                let link = nodes.next(s);
                let next = link.load_lo();
                fence(Ordering::Acquire);
                let h = link.load_hi();
                // A live node holding its value at home has a high half of
                // its generation alone; every other shape is `walk_past`'s,
                // which hands `next` back so nothing holds it across the call.
                if h == u64::from(gen_of(cur)) {
                    visit(Some((cur, v)));
                    cur = next;
                } else {
                    match self.walk_past(cur, h, next, v, &mut copy) {
                        Passed::Value(v, after) => {
                            visit(Some((cur, v)));
                            cur = after;
                        }
                        Passed::Marked(after) => cur = after,
                        Passed::Freed => continue 'walk,
                    }
                }
            }
            return;
        }
    }

    /// What a walk does at node `at` when `h`, its `next` high half as read,
    /// is not its generation alone, `next` its low half and `v` its value
    /// as copied from home: the value to visit, a frame's when it lies in
    /// one, a pass over a marked node, each with `next` to step to, or a
    /// fresh walk when the node was freed.
    #[cold]
    #[inline(never)]
    fn walk_past<V>(&self, at: u64, h: u64, next: u64, v: V, copy: &mut impl FnMut(*const u8) -> V) -> Passed<V> {
        if own(h) != gen_of(at) || h & FREE != 0 {
            return Passed::Freed;
        }
        if h & MARK != 0 {
            return Passed::Marked(next);
        }
        if vloc(h) == HOME {
            return Passed::Value(v, next);
        }
        match self.copy_placed(slot_of(at), at, h, copy) {
            Some(v) => Passed::Value(v, next),
            None => Passed::Freed,
        }
    }

    /// The value node `s`, of link `at`, holds in a frame, `h` its `next` high
    /// half as first read; None when the node was freed meanwhile.
    fn copy_placed<V>(&self, s: u32, at: u64, mut h: u64, copy: &mut impl FnMut(*const u8) -> V) -> Option<V> {
        loop {
            let f = (vloc(h) - 1) as u32;
            let word = self.frame_word(f);
            let g = word.load(Ordering::Acquire) as u32;
            let v = copy(self.frame_value(f));
            fence(Ordering::Acquire);
            let again = hi(self.next(s).load());
            if own(again) != gen_of(at) || again & FREE != 0 {
                return None;
            }
            if g & 1 == 0 && word.load(Ordering::Relaxed) as u32 == g && unmarked(again) == unmarked(h) {
                return Some(v);
            }
            if vloc(again) == HOME {
                return None;
            }
            h = again;
        }
    }

    // ---- sweep -----------------------------------------------------------

    /// Free what dead processes left: each node a dead process took and
    /// never linked, each marked node the list no longer reaches whose mark
    /// names no handle, each node whose mark names an entry `gone` holds,
    /// swapped out first where the list still reaches it, and each frame a
    /// dead process took that no node names. `gone` holds, for each registry
    /// entry, the holder word found gone, else zero. How many nodes and
    /// frames it freed.
    fn sweep(&self, gone: &[u64; REGISTRY]) -> usize {
        let used = (self.header().node_bump.load(Ordering::Acquire) as usize).min(self.shape.capacity);
        // Candidates are read before the walk: a candidate an unmarked node's
        // dead taker left links nothing during it.
        let mut nodes = Vec::new();
        for s in 1..used as u32 {
            let w = self.next(s).load();
            let h = hi(w);
            if h & FREE != 0 {
                continue;
            }
            let candidate = if h & MARK != 0 {
                h & NAMED == 0 || gone[marker_entry(h)] != 0
            } else {
                holder_gone(self.owner(s).load(Ordering::Acquire))
            };
            if candidate {
                nodes.push((s, w));
            }
        }
        let frames_used = (self.header().frame_bump.load(Ordering::Acquire) as usize).min(self.shape.frame_count);
        let mut frames = Vec::new();
        for f in 0..frames_used as u32 {
            let word = self.frame_word(f).load(Ordering::Acquire);
            if self.frame_taker_gone(f, word) {
                frames.push((f, word));
            }
        }
        if nodes.is_empty() && frames.is_empty() {
            return 0;
        }
        let reached = self.reached();
        let mut named = vec![false; frames_used];
        for s in 1..used as u32 {
            let h = hi(self.next(s).load());
            if h & FREE == 0 && vloc(h) != HOME && ((vloc(h) - 1) as usize) < frames_used {
                named[(vloc(h) - 1) as usize] = true;
            }
        }
        let mut freed = 0;
        for (s, w) in nodes {
            let h = hi(w);
            let swept = if h & NAMED == 0 {
                !reached[s as usize] && self.free_node(s, w)
            } else if self.header().registry[marker_entry(h)].load(Ordering::Acquire) != gone[marker_entry(h)] {
                false
            } else if reached[s as usize] {
                self.finish_gone(link(s, own(h)), w)
            } else {
                self.free_node(s, w)
            };
            if swept {
                note!(self, ListSwept, s, link(s, own(h)), h, 0);
                freed += 1;
            }
        }
        for (f, word) in frames {
            let g = (word as u32).wrapping_add(1) | 1;
            if !named[f as usize]
                && self.frame_word(f).compare_exchange(word, u64::from(g), Ordering::AcqRel, Ordering::Relaxed).is_ok()
            {
                fence(Ordering::Release);
                Self::push(&self.header().frame_free, f, |f| self.frame_link(f));
                freed += 1;
            }
        }
        freed
    }

    /// Finish the removal a gone marker left: swap the marked node `x`, `mw`
    /// its frozen `next`, out of the list where the list still reaches it,
    /// then free it by CAS. Whether this caller freed it.
    fn finish_gone(&self, x: u64, mw: u128) -> bool {
        let succ = lo(mw);
        loop {
            let swapped = match self.find_pred(x) {
                None => break,
                Some(Pred::Sentinel) => {
                    self.header().head_next.compare_exchange(x, succ, Ordering::AcqRel, Ordering::Acquire).is_ok()
                }
                Some(Pred::Node(p, pw)) => self.next(slot_of(p)).compare_exchange(pw, pair(succ, hi(pw))).is_ok(),
            };
            if swapped {
                let p = self.prev_named(x);
                self.after_unlink(p, x, succ);
                break;
            }
        }
        self.free_node(slot_of(x), mw)
    }

    /// Which nodes one whole walk from the front reaches, begun again when a
    /// node is freed under it.
    fn reached(&self) -> Vec<bool> {
        'walk: loop {
            let mut reached = vec![false; self.shape.capacity];
            let mut cur = self.header().head_next.load(Ordering::Acquire);
            let mut steps = 0usize;
            while cur != SENTINEL && steps <= self.shape.capacity {
                let s = slot_of(cur);
                if s as usize >= self.shape.capacity {
                    break;
                }
                let w = self.next(s).load();
                if own(hi(w)) != gen_of(cur) || hi(w) & FREE != 0 {
                    continue 'walk;
                }
                reached[s as usize] = true;
                cur = lo(w);
                steps += 1;
            }
            return reached;
        }
    }

    /// The address the test-build trace knows this list by.
    #[cfg(test)]
    pub(crate) fn trace_name(&self) -> usize {
        self.base as usize
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::set_table::{put_key, take_key};
    use std::collections::HashSet;

    /// A pid no live process has, as a writer that died would leave.
    const GONE: u32 = u32::MAX - 7;

    fn tmp(name: &str) -> crate::test_paths::TmpFile {
        crate::test_paths::TmpFile::new(format!("subetha-list-core-{name}-{}.bin", std::process::id()))
    }

    fn list(path: &Path, capacity: usize) -> ListCore {
        ListCore::create(path, capacity, 8, 8, 0).expect("a list")
    }

    /// A push's result as the list types report it: the handle, or `Full`.
    fn reported(h: Option<NonZeroU64>) -> Result<u64, LinkedListError> {
        h.map(NonZeroU64::get).ok_or(LinkedListError::Region(RegionError::Full))
    }

    fn push_back(c: &ListCore, v: u64) -> Result<u64, LinkedListError> {
        // SAFETY: the list hands an eight-byte value place nobody else writes.
        reported(c.push_back(|p| unsafe { put_key(p, &v.to_le_bytes()) }))
    }

    fn push_front(c: &ListCore, v: u64) -> Result<u64, LinkedListError> {
        // SAFETY: as in `push_back`.
        reported(c.push_front(|p| unsafe { put_key(p, &v.to_le_bytes()) }))
    }

    /// Copy an eight-byte value out of a value place.
    fn read(p: *const u8) -> u64 {
        let mut out = [0u8; 8];
        // SAFETY: the list hands an eight-byte value place.
        unsafe { take_key(p, &mut out) };
        u64::from_le_bytes(out)
    }

    fn values(c: &ListCore) -> Vec<u64> {
        let mut out = Vec::new();
        c.walk_by(c.nodes(), read, |seen| match seen {
            None => out.clear(),
            Some((_, v)) => out.push(v),
        });
        out
    }

    /// Every node below the bump counter is on the free stack once or in the
    /// list once, unmarked; every frame below its counter is on its stack
    /// once or named by one node: what a list at rest shows. A counter reads
    /// past its pool once racing takers have passed the end, so each is
    /// clamped to its pool.
    fn assert_every_node_accounted(c: &ListCore) {
        let hdr = c.header();
        let used = (hdr.node_bump.load(Ordering::Acquire) as usize).min(c.shape.capacity);
        let mut seen = vec![false; c.shape.capacity];
        let mut top = hdr.node_free.load(Ordering::Acquire) as u32;
        while top != EMPTY && top != EXHAUSTED {
            let s = top - 1;
            assert!(!seen[s as usize], "node {s} is on the free stack twice");
            seen[s as usize] = true;
            assert_ne!(hi(c.next(s).load()) & FREE, 0, "node {s} is on the free stack but not free");
            top = c.nodes().free_link(s).load(Ordering::Acquire) as u32;
        }
        let frames_used = (hdr.frame_bump.load(Ordering::Acquire) as usize).min(c.shape.frame_count);
        let mut frame_seen = vec![false; c.shape.frame_count];
        let mut cur = hdr.head_next.load(Ordering::Acquire);
        let mut last = SENTINEL;
        while cur != SENTINEL {
            let s = slot_of(cur);
            assert!(!seen[s as usize], "node {s} is free or listed twice");
            seen[s as usize] = true;
            let w = c.next(s).load();
            assert_eq!(own(hi(w)), gen_of(cur), "the link to node {s} names its generation");
            assert_eq!(hi(w) & (MARK | FREE), 0, "node {s} in a list at rest is neither marked nor free");
            if vloc(hi(w)) != HOME {
                let f = (vloc(hi(w)) - 1) as usize;
                assert!(!frame_seen[f], "frame {f} is named twice");
                frame_seen[f] = true;
            }
            last = cur;
            cur = lo(w);
        }
        assert_eq!(c.last(), last, "the last node found is the walk's last");
        let lost: Vec<usize> = (1..used).filter(|&s| !seen[s]).collect();
        assert!(lost.is_empty(), "nodes neither free nor listed: {lost:?}");
        let mut top = hdr.frame_free.load(Ordering::Acquire) as u32;
        while top != EMPTY && top != EXHAUSTED {
            let f = top - 1;
            assert!(!frame_seen[f as usize], "frame {f} is free and named, or on the stack twice");
            frame_seen[f as usize] = true;
            top = c.frame_link(f).load(Ordering::Acquire) as u32;
        }
        let lost: Vec<usize> = (0..frames_used).filter(|&f| !frame_seen[f]).collect();
        assert!(lost.is_empty(), "frames neither free nor named: {lost:?}");
    }

    /// A list holds no more nodes than its links can number frames for: the
    /// frames are the nodes and FRAME_SLACK more, each named plus one in the
    /// 29 bits below NAMED.
    #[test]
    fn a_list_too_large_to_name_its_frames_is_refused() {
        let largest = (1 << (63 - VLOC_SHIFT)) - FRAME_SLACK - 1;
        assert!(Shape::new(largest, 8, 8, 0).is_ok(), "the largest list lays out");
        assert!(matches!(Shape::new(largest + 1, 8, 8, 0), Err(why) if why.contains("536870848")));
        assert!(Shape::new(usize::MAX, 8, 8, 0).is_err());
    }

    #[test]
    fn a_fresh_list_lays_out_and_attaches_with_its_nodes_in_place() {
        let p = tmp("attach");
        let c = list(&p, 8);
        push_back(&c, 1).unwrap();
        push_front(&c, 0).unwrap();
        let again = ListCore::create(&p, 8, 8, 8, 0).expect("an attach");
        assert_eq!(values(&again), vec![0, 1]);
        assert_eq!(again.len(), 2);
        assert!(matches!(ListCore::open(&p, 9, 8, 8, 0), Err(LinkedListError::LayoutMismatch(_))));
        assert_every_node_accounted(&c);
    }

    /// A node a dead process took and never linked stays out of use while no
    /// exit owes a sweep, so a push into the full list is refused as `Full` without
    /// sweeping; `reclaim` frees it, as attaching does.
    #[test]
    fn a_node_a_dead_process_took_comes_back_by_reclaim_and_by_attach() {
        let p = tmp("dead-push");
        let c = list(&p, 4);
        let (s, g) = c.take_node(c.nodes()).unwrap();
        c.next(s).store(pair(SENTINEL, u64::from(g)));
        c.owner(s).store(u64::from(GONE), Ordering::Relaxed);
        push_back(&c, 1).unwrap();
        push_back(&c, 2).unwrap();
        assert!(matches!(push_back(&c, 3), Err(LinkedListError::Region(RegionError::Full))), "no exit owes a sweep");
        assert_eq!(c.reclaim(), 1, "reclaim freed the dead process's node");
        let h = push_back(&c, 3).unwrap();
        assert_eq!(slot_of(h), s, "the push took the node the sweep freed");
        assert_eq!(values(&c), vec![1, 2, 3]);
        assert_eq!(c.pop_front(read), Some(1));
        let (s, g) = c.take_node(c.nodes()).unwrap();
        c.next(s).store(pair(SENTINEL, u64::from(g)));
        c.owner(s).store(u64::from(GONE), Ordering::Relaxed);
        drop(ListCore::open(&p, 4, 8, 8, 0).expect("an attach"));
        assert_eq!(hi(c.next(s).load()) & FREE, FREE, "attaching swept the dead process's node");
        assert_every_node_accounted(&c);
    }

    /// A process that registered on the list and is killed while it holds a
    /// node owes a sweep through its exit event, which the next push that
    /// finds nothing free runs.
    #[test]
    fn a_registered_process_killed_holding_a_node_owes_a_sweep() {
        let p = tmp("killed");
        let c = list(&p, 3);
        let exe = std::env::current_exe().unwrap();
        let mut child = std::process::Command::new(exe)
            .args(["--exact", "exit_watch::tests::sleeper_never_runs_on_its_own", "--ignored", "--nocapture"])
            .env("SUBETHA_EXIT_WATCH_SLEEP", "1")
            .spawn()
            .unwrap();
        let child_holder = process_identity::holder_for(child.id());
        let hdr = c.header();
        let entry =
            hdr.registry.iter().position(|e| e.compare_exchange(0, child_holder, Ordering::AcqRel, Ordering::Relaxed).is_ok()).unwrap();
        hdr.registry_epoch.fetch_add(1, Ordering::AcqRel);
        let (s, g) = c.take_node(c.nodes()).unwrap();
        c.next(s).store(pair(SENTINEL, u64::from(g)));
        c.owner(s).store(child_holder, Ordering::Relaxed);
        push_back(&c, 1).unwrap();
        assert!(matches!(push_back(&c, 2), Err(LinkedListError::Region(RegionError::Full))), "the child lives");
        assert_eq!(crate::exit_watch::told(child_holder), Some(false), "the refused push watched the newly registered child");
        child.kill().unwrap();
        child.wait().unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while crate::exit_watch::told(child_holder) != Some(true) {
            assert!(std::time::Instant::now() < deadline, "the child's exit was told");
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        let h = push_back(&c, 2).expect("the owed sweep freed the child's node");
        assert_eq!(slot_of(h), s);
        assert_eq!(hdr.registry[entry].load(Ordering::Acquire), 0, "the dead child's entry was freed");
        assert_eq!(values(&c), vec![1, 2]);
        assert_every_node_accounted(&c);
    }

    /// A node marked and swapped out by a writer that died before freeing it
    /// comes back by the sweep; a live writer's node in flight stays.
    #[test]
    fn a_marked_node_left_unfreed_is_swept_and_a_live_takers_node_stays() {
        let p = tmp("dead-unlink");
        let c = list(&p, 4);
        let h = push_back(&c, 1).unwrap();
        let s = slot_of(h);
        let w = c.next(s).load();
        let marked = pair(lo(w), hi(w) | MARK);
        c.next(s).compare_exchange(w, marked).expect("the mark");
        c.header().head_next.compare_exchange(h, lo(w), Ordering::AcqRel, Ordering::Acquire).expect("the swap out");
        let (live, g) = c.take_node(c.nodes()).unwrap();
        c.next(live).store(pair(SENTINEL, u64::from(g)));
        push_back(&c, 2).unwrap();
        assert_eq!(c.reclaim(), 1, "the marked node no walk reaches is freed");
        push_back(&c, 3).unwrap();
        assert_eq!(values(&c), vec![2, 3]);
        assert_eq!(c.reclaim(), 0, "this process's own node in flight is not swept");
        c.free_node(live, c.next(live).load());
        assert_every_node_accounted(&c);
    }

    /// A registry entry naming a process that is gone, as a killed handle
    /// leaves it: its index.
    fn register_gone(c: &ListCore) -> usize {
        let hdr = c.header();
        let e = hdr
            .registry
            .iter()
            .position(|r| r.compare_exchange(0, u64::from(GONE), Ordering::AcqRel, Ordering::Relaxed).is_ok())
            .expect("a free registry entry");
        hdr.registry_epoch.fetch_add(1, Ordering::AcqRel);
        e
    }

    /// A node a registered handle marked, its process killed before it swapped
    /// the node out, is swapped out and freed by the sweep, and the gone entry
    /// is freed after it.
    #[test]
    fn a_node_a_gone_handle_marked_in_the_list_is_swept_then_its_entry_freed() {
        let p = tmp("gone-named-linked");
        let c = list(&p, 4);
        let e = register_gone(&c);
        let h = push_back(&c, 1).unwrap();
        push_back(&c, 2).unwrap();
        let s = slot_of(h);
        let wh = c.next(s).load_hi();
        c.next(s).compare_exchange_hi(wh, wh | MARK | NAMED | ((e as u64) << VLOC_SHIFT)).expect("the gone handle's mark");
        assert_eq!(c.reclaim(), 1, "the sweep swapped out and freed the gone handle's node");
        assert_eq!(c.header().registry[e].load(Ordering::Acquire), 0, "the entry is freed after its node");
        assert_eq!(values(&c), vec![2]);
        assert_every_node_accounted(&c);
    }

    /// A node a registered handle marked and swapped out, its process killed
    /// before the free, is freed by the sweep.
    #[test]
    fn a_node_a_gone_handle_swapped_out_is_swept() {
        let p = tmp("gone-named-out");
        let c = list(&p, 4);
        let e = register_gone(&c);
        let h = push_back(&c, 1).unwrap();
        push_back(&c, 2).unwrap();
        let s = slot_of(h);
        let w = c.next(s).load();
        c.next(s).compare_exchange_hi(hi(w), hi(w) | MARK | NAMED | ((e as u64) << VLOC_SHIFT)).expect("the mark");
        c.header().head_next.compare_exchange(h, lo(w), Ordering::AcqRel, Ordering::Acquire).expect("the swap out");
        assert_eq!(c.reclaim(), 1, "the sweep freed the node no walk reaches");
        assert_eq!(c.header().registry[e].load(Ordering::Acquire), 0, "the entry is freed after its node");
        push_back(&c, 3).unwrap();
        assert_eq!(values(&c), vec![2, 3]);
        assert_every_node_accounted(&c);
    }

    /// A node a live handle marked is left by a sweep and by another handle's
    /// pop that swaps it out, and its marker frees it.
    #[test]
    fn a_live_handles_marked_node_is_freed_by_that_handle_alone() {
        let p = tmp("live-named");
        let c = list(&p, 4);
        let other = list(&p, 4);
        let h = push_back(&c, 1).unwrap();
        push_back(&c, 2).unwrap();
        let s = slot_of(h);
        let w = c.next(s).load();
        let mh = c.mark_of(hi(w));
        assert_ne!(mh & NAMED, 0, "a handle holding a registry entry names itself");
        c.next(s).compare_exchange_hi(hi(w), mh).expect("the mark");
        assert_eq!(other.reclaim(), 0, "a sweep leaves a live handle's node in the list");
        assert_eq!(other.pop_front(read), Some(2), "the other handle swaps the marked node out and pops the next");
        assert_eq!(hi(c.next(s).load()) & FREE, 0, "the helper left the free to the marker");
        assert_eq!(other.reclaim(), 0, "a sweep leaves a live handle's node out of the list");
        c.unlink(h, pair(lo(w), mh));
        assert_ne!(hi(c.next(s).load()) & FREE, 0, "the marker freed it");
        assert_every_node_accounted(&c);
    }

    /// Sweeps running all the while beside threads that push, pop, remove and
    /// set leave every node and frame accounted for, and no value comes out
    /// twice.
    #[test]
    fn sweeps_beside_writers_leave_every_node_accounted() {
        let p = tmp("sweeping");
        let c = list(&p, 256);
        let stop = AtomicBool::new(false);
        let taken: Vec<Vec<u64>> = std::thread::scope(|s| {
            let sweeper = s.spawn(|| {
                let mut sweeps = 0u64;
                while !stop.load(Ordering::Acquire) {
                    c.reclaim();
                    sweeps += 1;
                }
                sweeps
            });
            let workers: Vec<_> = (0..6u64)
                .map(|t| {
                    let c = &c;
                    s.spawn(move || {
                        let mut got = Vec::new();
                        let mut mine = Vec::new();
                        for i in 0..5_000u64 {
                            let v = (t << 32) | i;
                            match if i % 2 == 0 { push_back(c, v) } else { push_front(c, v) } {
                                Ok(h) => mine.push(h),
                                Err(LinkedListError::Region(RegionError::Full)) => got.extend(c.pop_front(read)),
                                Err(e) => panic!("push: {e}"),
                            }
                            match i % 4 {
                                0 => got.extend(c.pop_back(read)),
                                1 => got.extend(c.pop_front(read)),
                                2 => {
                                    if let Some(h) = mine.pop() {
                                        got.extend(c.remove(h, read));
                                    }
                                }
                                _ => {
                                    if let Some(&h) = mine.last() {
                                        let w = (t << 32) | (1 << 31) | i;
                                        // SAFETY: an eight-byte value place.
                                        match c.set(h, |p| unsafe { put_key(p, &w.to_le_bytes()) }) {
                                            Ok(()) | Err(LinkedListError::InvalidHandle) => {}
                                            Err(e) => panic!("set: {e}"),
                                        }
                                    }
                                }
                            }
                        }
                        got
                    })
                })
                .collect();
            let got: Vec<Vec<u64>> = workers.into_iter().map(|w| w.join().expect("a writer")).collect();
            stop.store(true, Ordering::Release);
            assert!(sweeper.join().expect("the sweeper") > 0, "the sweeper ran");
            got
        });
        let mut seen = HashSet::new();
        for v in taken.into_iter().flatten().chain(values(&c)) {
            assert!(seen.insert(v), "value {v:#x} came out twice");
        }
        assert_every_node_accounted(&c);
    }

    /// A frame a dead setter took and never placed comes back by the sweep.
    #[test]
    fn a_frame_a_dead_setter_took_is_swept() {
        let p = tmp("dead-set");
        let c = list(&p, 4);
        let h = push_back(&c, 1).unwrap();
        let f = c.take_frame().unwrap();
        let word = c.frame_word(f).load(Ordering::Relaxed);
        c.frame_taker(f).store(u64::from(GONE), Ordering::Relaxed);
        c.frame_word(f).store((word & 0xFFFF_FFFF) | (u64::from(GONE) << 32), Ordering::Relaxed);
        // SAFETY: an eight-byte value place.
        c.set(h, |p| unsafe { put_key(p, &5u64.to_le_bytes()) }).unwrap();
        assert_eq!(c.reclaim(), 1, "the sweep freed the dead setter's frame");
        assert_eq!(c.get(h, read), Some(5), "the frame a node names stays");
        assert_every_node_accounted(&c);
    }

    /// A copy from a free frame is refused. Two sets can bring a node's value
    /// back to the frame the first one freed, so a reader whose copy fell
    /// between them checks a `next` that names that frame again, and only the
    /// frame's generation can refuse the free frame's bytes, its first word
    /// the free stack's link.
    #[test]
    fn a_copy_from_a_free_frame_is_refused() {
        let p = tmp("free-frame");
        let c = list(&p, 4);
        let h = push_back(&c, 1).unwrap();
        let s = slot_of(h);
        // SAFETY: an eight-byte value place.
        let set = |v: u64| c.set(h, |p| unsafe { put_key(p, &v.to_le_bytes()) }).unwrap();
        set(2);
        let held = hi(c.next(s).load());
        assert_ne!(vloc(held), HOME, "the set placed its value in a frame");
        set(3);
        assert_ne!(vloc(hi(c.next(s).load())), vloc(held), "the second set moved the value to another frame");
        assert_eq!(c.copy_value(s, held, &mut read), None, "the frame the second set freed");
        set(4);
        assert_eq!(vloc(hi(c.next(s).load())), vloc(held), "the third set took the freed frame back");
        assert_eq!(c.copy_value(s, held, &mut read), Some(4), "the frame filled again");
        assert_eq!(c.get(h, read), Some(4));
        assert_every_node_accounted(&c);
    }

    /// What a process that held this process's pid before it left, recorded
    /// with its own start: a node it took, a frame it took and its registry
    /// entry. The pid is alive, the start is not, so the sweep frees the node
    /// and the frame, reclaim frees the entry, and watching the entry owes a
    /// sweep at once.
    #[test]
    fn what_a_process_left_under_a_reused_pid_is_swept() {
        let p = tmp("reused-pid");
        let c = list(&p, 4);
        let me = process_identity::current();
        let stamp = process_identity::holder_stamp(me);
        assert_ne!(stamp, 0, "this process reads its own start");
        let earlier = process_identity::holder(process_identity::holder_pid(me), stamp.wrapping_add(1).max(1));
        let (s, g) = c.take_node(c.nodes()).unwrap();
        c.next(s).store(pair(SENTINEL, u64::from(g)));
        c.owner(s).store(earlier, Ordering::Relaxed);
        let f = c.take_frame().unwrap();
        c.frame_taker(f).store(earlier, Ordering::Relaxed);
        let hdr = c.header();
        let entry = hdr.registry.iter().position(|e| e.compare_exchange(0, earlier, Ordering::AcqRel, Ordering::Relaxed).is_ok()).unwrap();
        hdr.registry_epoch.fetch_add(1, Ordering::AcqRel);
        c.owed.store(false, Ordering::Release);
        c.watch_registered();
        assert!(c.owed.load(Ordering::Acquire), "an entry whose start is not its pid's owes a sweep at once");
        assert_eq!(c.reclaim(), 2, "the node and the frame are freed");
        assert_eq!(hdr.registry[entry].load(Ordering::Acquire), 0, "the entry is freed");
        assert_eq!(hi(c.next(s).load()) & FREE, FREE);
        assert_every_node_accounted(&c);
    }

    /// Threads pushing, popping, removing by handle and setting at once leave
    /// every node and frame accounted for, and no value comes out twice.
    #[test]
    fn writers_doing_everything_at_once_leave_every_node_accounted() {
        let p = tmp("everything");
        let c = list(&p, 256);
        crate::set_trace::name(&[c.trace_name()]);
        let taken: Vec<Vec<u64>> = std::thread::scope(|s| {
            let workers: Vec<_> = (0..8u64)
                .map(|t| {
                    let c = &c;
                    s.spawn(move || {
                        let mut got = Vec::new();
                        let mut mine = Vec::new();
                        for i in 0..5_000u64 {
                            let v = (t << 32) | i;
                            match if i % 2 == 0 { push_back(c, v) } else { push_front(c, v) } {
                                Ok(h) => mine.push(h),
                                Err(LinkedListError::Region(RegionError::Full)) => got.extend(c.pop_front(read)),
                                Err(e) => panic!("push: {e}"),
                            }
                            match i % 4 {
                                0 => got.extend(c.pop_back(read)),
                                1 => got.extend(c.pop_front(read)),
                                2 => {
                                    if let Some(h) = mine.pop() {
                                        got.extend(c.remove(h, read));
                                    }
                                }
                                _ => {
                                    if let Some(&h) = mine.last() {
                                        let w = (t << 32) | (1 << 31) | i;
                                        // SAFETY: an eight-byte value place.
                                        match c.set(h, |p| unsafe { put_key(p, &w.to_le_bytes()) }) {
                                            Ok(()) | Err(LinkedListError::InvalidHandle) => {}
                                            Err(e) => panic!("set: {e}"),
                                        }
                                    }
                                }
                            }
                        }
                        got
                    })
                })
                .collect();
            workers.into_iter().map(|w| w.join().expect("a writer")).collect()
        });
        let mut seen = HashSet::new();
        for v in taken.into_iter().flatten().chain(values(&c)) {
            assert!(seen.insert(v), "value {v:#x} came out twice");
        }
        assert_every_node_accounted(&c);
        while c.pop_front(read).is_some() {}
        assert!(c.is_empty());
        assert_eq!(c.len(), 0);
        assert_every_node_accounted(&c);
    }

    /// The push the bump counter refuses leaves the exhausted mark in the
    /// free-stack head, and a push after it is refused from the head without
    /// adding to the counter. A node freed onto the mark takes it as its
    /// link, so taking that node leaves the mark again.
    #[test]
    fn an_exhausted_list_refuses_from_its_free_stack_head() {
        let p = tmp("exhausted");
        let c = list(&p, 4);
        let hdr = c.header();
        for v in 1..4u64 {
            assert_eq!(hdr.node_free.load(Ordering::Relaxed) as u32, EMPTY, "the mark is set with a node left");
            push_back(&c, v).expect("a node left");
        }
        assert_eq!(hdr.node_free.load(Ordering::Relaxed) as u32, EMPTY, "taking the last node leaves the head unmarked");
        assert!(matches!(push_back(&c, 4), Err(LinkedListError::Region(RegionError::Full))));
        assert_eq!(hdr.node_free.load(Ordering::Relaxed) as u32, EXHAUSTED, "the refused push left the mark");
        assert_eq!(hdr.node_bump.load(Ordering::Relaxed), 4, "the refused take gave its add back");
        assert!(matches!(push_back(&c, 4), Err(LinkedListError::Region(RegionError::Full))));
        assert_eq!(hdr.node_bump.load(Ordering::Relaxed), 4, "a refusal from the mark adds nothing");
        assert_eq!(c.pop_front(read), Some(1));
        push_back(&c, 5).expect("the node the pop freed");
        assert_eq!(hdr.node_free.load(Ordering::Relaxed) as u32, EXHAUSTED, "taking the freed node leaves the mark");
        assert!(matches!(push_back(&c, 6), Err(LinkedListError::Region(RegionError::Full))));
        assert_eq!(values(&c), vec![2, 3, 5]);
        assert_every_node_accounted(&c);
    }

    /// A take that finds the free stack empty and unmarked with the counter
    /// spent, as taking the last node leaves it, gives its add back and
    /// leaves the mark.
    #[test]
    fn a_spent_counter_under_an_unmarked_head_marks_it_on_the_next_take() {
        let p = tmp("unmarked");
        let c = list(&p, 4);
        let hdr = c.header();
        hdr.node_bump.store(4, Ordering::Relaxed);
        assert!(matches!(push_back(&c, 1), Err(LinkedListError::Region(RegionError::Full))));
        assert_eq!(hdr.node_free.load(Ordering::Relaxed) as u32, EXHAUSTED);
        assert_eq!(hdr.node_bump.load(Ordering::Relaxed), 4, "the take gives its add back");
    }

    /// The frames' free stack takes the mark as the nodes' does: from the
    /// take the counter refuses, and back from a frame freed onto it.
    #[test]
    fn an_exhausted_frame_pool_refuses_from_its_free_stack_head() {
        let p = tmp("frames-exhausted");
        let c = list(&p, 4);
        let hdr = c.header();
        let frames: Vec<u32> = (0..c.shape.frame_count).map(|_| c.take_frame().expect("a frame left")).collect();
        assert_eq!(hdr.frame_free.load(Ordering::Relaxed) as u32, EMPTY, "taking the last frame leaves the head unmarked");
        assert_eq!(c.take_frame(), None);
        assert_eq!(hdr.frame_free.load(Ordering::Relaxed) as u32, EXHAUSTED, "the refused take left the mark");
        assert_eq!(hdr.frame_bump.load(Ordering::Relaxed), c.shape.frame_count as u64, "the refused take gave its add back");
        assert_eq!(c.take_frame(), None);
        assert_eq!(hdr.frame_bump.load(Ordering::Relaxed), c.shape.frame_count as u64, "a refusal from the mark adds nothing");
        c.give_frame(frames[0]);
        assert_eq!(c.take_frame(), Some(frames[0]));
        assert_eq!(hdr.frame_free.load(Ordering::Relaxed) as u32, EXHAUSTED, "taking the freed frame leaves the mark");
    }

    /// Threads pushing past the end at once are handed every node once, and
    /// leave the mark in the free-stack head and the counter at the capacity.
    #[test]
    fn racing_pushes_take_every_node_once_and_leave_the_mark() {
        let p = tmp("racing-pushes");
        let c = list(&p, 256);
        let handles: Vec<Vec<u64>> = std::thread::scope(|s| {
            let pushers: Vec<_> = (0..4u64)
                .map(|t| {
                    let c = &c;
                    s.spawn(move || {
                        let mut mine = Vec::new();
                        for i in 0..256u64 {
                            match push_back(c, (t << 32) | i) {
                                Ok(h) => mine.push(h),
                                Err(LinkedListError::Region(RegionError::Full)) => {}
                                Err(e) => panic!("push: {e}"),
                            }
                        }
                        mine
                    })
                })
                .collect();
            pushers.into_iter().map(|w| w.join().expect("a pusher")).collect()
        });
        let mut slots: Vec<u32> = handles.into_iter().flatten().map(slot_of).collect();
        slots.sort_unstable();
        assert_eq!(slots, (1..256u32).collect::<Vec<_>>(), "every node is handed out once");
        let hdr = c.header();
        assert_eq!(hdr.node_free.load(Ordering::Relaxed) as u32, EXHAUSTED);
        assert_eq!(hdr.node_bump.load(Ordering::Relaxed), 256);
        assert_every_node_accounted(&c);
    }
}
