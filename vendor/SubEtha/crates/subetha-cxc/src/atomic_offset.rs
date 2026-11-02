//! Atomic offsets: references into a mapping held as integers relative to
//! a base the reader supplies, so they mean the same in every process
//! whatever address it mapped the region at, and pack two or four to an
//! atomic word when they change together.
//!
//! - [`AtomicOffset`]: one 64-bit offset with a bounded reserve.
//! - [`Tagged32`]: a 32-bit offset beside a 32-bit tag (a version or an
//!   owner token) in one 64-bit word.
//! - [`Pair32`]: two 32-bit indexes that change together.
//! - [`Quad32`]: four 32-bit indexes, or two beside a 64-bit version, in one
//!   16-byte word, swapped as one.
//! - [`AtomicRel`]: a pointer stored relative to its own address, valid
//!   while pointer and pointee move together.
//!
//! A pointer is made from an offset only at the point of use, from the base
//! (or the word's own address) the caller hands in, so it carries that
//! mapping's provenance.

use std::marker::PhantomData;
use std::sync::atomic::{AtomicIsize, AtomicU64, Ordering};

use crate::atomic_pair::AtomicPair;

/// One 64-bit offset.
#[repr(transparent)]
pub struct AtomicOffset(AtomicU64);

impl AtomicOffset {
    pub const fn new(off: u64) -> Self {
        Self(AtomicU64::new(off))
    }

    pub fn load(&self, order: Ordering) -> u64 {
        self.0.load(order)
    }

    pub fn store(&self, off: u64, order: Ordering) {
        self.0.store(off, order);
    }

    /// Replace `current` with `new`: `Err` with the offset found.
    pub fn compare_exchange(&self, current: u64, new: u64) -> Result<u64, u64> {
        self.0.compare_exchange(current, new, Ordering::AcqRel, Ordering::Acquire)
    }

    /// Take `n` units below `limit`: the start of the units taken, or
    /// `None` when they do not fit. Refuses on one read when the offset is
    /// already past room; two takers racing for the last room can carry it
    /// past `limit`, and the one that does is refused, so readers clamp.
    pub fn reserve(&self, n: u64, limit: u64) -> Option<u64> {
        let seen = self.0.load(Ordering::Relaxed);
        if seen.checked_add(n)? > limit {
            return None;
        }
        let start = self.0.fetch_add(n, Ordering::AcqRel);
        (start.checked_add(n)? <= limit).then_some(start)
    }

    /// The `T` at `off` bytes past `base`.
    ///
    /// # Safety
    /// `base + off` addresses a live, aligned `T` inside the allocation
    /// `base` points into, which outlives `'a`.
    pub unsafe fn resolve<'a, T>(base: *const u8, off: u64) -> &'a T {
        // SAFETY: the caller vouches for the address and its lifetime; the
        // pointer is derived from `base`, so it carries its provenance.
        unsafe { &*(base.add(off as usize) as *const T) }
    }
}

/// A 32-bit offset (low half) beside a 32-bit tag (high half).
#[repr(transparent)]
pub struct Tagged32(AtomicU64);

#[inline]
const fn pack2(lo: u32, hi: u32) -> u64 {
    (lo as u64) | ((hi as u64) << 32)
}

#[inline]
const fn unpack2(word: u64) -> (u32, u32) {
    (word as u32, (word >> 32) as u32)
}

impl Tagged32 {
    pub const fn new(off: u32, tag: u32) -> Self {
        Self(AtomicU64::new(pack2(off, tag)))
    }

    /// `(offset, tag)`.
    pub fn load(&self) -> (u32, u32) {
        unpack2(self.0.load(Ordering::Acquire))
    }

    pub fn store(&self, off: u32, tag: u32) {
        self.0.store(pack2(off, tag), Ordering::Release);
    }

    /// Replace `current` with `new`: `Err` with what was found.
    pub fn compare_exchange(&self, current: (u32, u32), new: (u32, u32)) -> Result<(), (u32, u32)> {
        self.0
            .compare_exchange(pack2(current.0, current.1), pack2(new.0, new.1), Ordering::AcqRel, Ordering::Acquire)
            .map(|_| ())
            .map_err(unpack2)
    }

    /// Install `off` with the tag one past `current`'s, as a counted stack
    /// head does, so an offset that leaves and comes back between a load and
    /// this swap fails it.
    pub fn swap_counted(&self, current: (u32, u32), off: u32) -> Result<(u32, u32), (u32, u32)> {
        let next = (off, current.1.wrapping_add(1));
        self.compare_exchange(current, next).map(|()| next)
    }
}

/// Two 32-bit indexes that change together.
#[repr(transparent)]
pub struct Pair32(AtomicU64);

impl Pair32 {
    pub const fn new(a: u32, b: u32) -> Self {
        Self(AtomicU64::new(pack2(a, b)))
    }

    pub fn load(&self) -> (u32, u32) {
        unpack2(self.0.load(Ordering::Acquire))
    }

    pub fn store(&self, a: u32, b: u32) {
        self.0.store(pack2(a, b), Ordering::Release);
    }

    pub fn compare_exchange(&self, current: (u32, u32), new: (u32, u32)) -> Result<(), (u32, u32)> {
        self.0
            .compare_exchange(pack2(current.0, current.1), pack2(new.0, new.1), Ordering::AcqRel, Ordering::Acquire)
            .map(|_| ())
            .map_err(unpack2)
    }
}

/// Four 32-bit indexes, or two beside a 64-bit version, in one aligned
/// 16-byte word in shared memory.
#[repr(transparent)]
pub struct Quad32(AtomicPair);

#[inline]
fn quad_word(v: [u32; 4]) -> u128 {
    u128::from(v[0]) | (u128::from(v[1]) << 32) | (u128::from(v[2]) << 64) | (u128::from(v[3]) << 96)
}

#[inline]
fn quad_parts(word: u128) -> [u32; 4] {
    [word as u32, (word >> 32) as u32, (word >> 64) as u32, (word >> 96) as u32]
}

#[inline]
fn versioned_word((a, b, v): (u32, u32, u64)) -> u128 {
    u128::from(a) | (u128::from(b) << 32) | (u128::from(v) << 64)
}

#[inline]
fn versioned_parts(w: u128) -> (u32, u32, u64) {
    (w as u32, (w >> 32) as u32, (w >> 64) as u64)
}

impl Quad32 {
    /// Whether this processor can hold a `Quad32` at all.
    pub fn supported() -> bool {
        crate::atomic_pair::supported()
    }

    /// The word at `at`.
    ///
    /// # Safety
    /// `at` is 16-byte aligned, addresses 16 bytes readable and writable for
    /// `'a` that are accessed only as a `Quad32`, and [`supported`]
    /// answered true.
    ///
    /// [`supported`]: Self::supported
    pub unsafe fn at<'a>(at: *mut u8) -> &'a Self {
        // SAFETY: `Quad32` is a transparent `AtomicPair`, and the caller
        // vouches for what `AtomicPair::at` requires.
        unsafe { &*(AtomicPair::at(at) as *const AtomicPair as *const Self) }
    }

    pub fn load(&self) -> [u32; 4] {
        quad_parts(self.0.load())
    }

    pub fn store(&self, v: [u32; 4]) {
        self.0.store(quad_word(v));
    }

    pub fn compare_exchange(&self, current: [u32; 4], new: [u32; 4]) -> Result<(), [u32; 4]> {
        self.0.compare_exchange(quad_word(current), quad_word(new)).map(|_| ()).map_err(quad_parts)
    }

    /// `(a, b, version)`.
    pub fn load_versioned(&self) -> (u32, u32, u64) {
        versioned_parts(self.0.load())
    }

    /// Replace `(a, b, version)` with `(a2, b2, version + 1)`.
    pub fn swap_versioned(&self, current: (u32, u32, u64), a: u32, b: u32) -> Result<(u32, u32, u64), (u32, u32, u64)> {
        let next = (a, b, current.2.wrapping_add(1));
        self.0
            .compare_exchange(versioned_word(current), versioned_word(next))
            .map(|_| next)
            .map_err(versioned_parts)
    }
}

/// A pointer stored as its distance from the word's own address, which
/// stays right while the word and its pointee move together (one chunk, one
/// mapping).
pub struct AtomicRel<T> {
    off: AtomicIsize,
    _to: PhantomData<*const T>,
}

/// The stored distance that means null: no pointee is that far below.
const REL_NULL: isize = isize::MIN;

// SAFETY: the word is one atomic; what it points at is the caller's to
// share or not.
unsafe impl<T> Send for AtomicRel<T> {}
unsafe impl<T> Sync for AtomicRel<T> {}

impl<T> AtomicRel<T> {
    pub const fn null() -> Self {
        Self { off: AtomicIsize::new(REL_NULL), _to: PhantomData }
    }

    fn here(&self) -> *const u8 {
        self as *const Self as *const u8
    }

    fn distance(&self, p: *const T) -> isize {
        if p.is_null() { REL_NULL } else { (p as *const u8 as isize).wrapping_sub(self.here() as isize) }
    }

    fn pointer(&self, off: isize) -> *const T {
        if off == REL_NULL {
            std::ptr::null()
        } else {
            self.here().wrapping_offset(off) as *const T
        }
    }

    /// The pointee, null when none; derived from the word's own address.
    pub fn load(&self, order: Ordering) -> *const T {
        self.pointer(self.off.load(order))
    }

    pub fn store(&self, p: *const T, order: Ordering) {
        self.off.store(self.distance(p), order);
    }

    pub fn compare_exchange(&self, current: *const T, new: *const T) -> Result<*const T, *const T> {
        self.off
            .compare_exchange(self.distance(current), self.distance(new), Ordering::AcqRel, Ordering::Acquire)
            .map(|off| self.pointer(off))
            .map_err(|off| self.pointer(off))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    #[test]
    fn a_reserve_never_hands_out_room_past_its_limit() {
        let next = Arc::new(AtomicOffset::new(0));
        let taken: Vec<Vec<u64>> = (0..8)
            .map(|_| {
                let next = Arc::clone(&next);
                std::thread::spawn(move || {
                    let mut mine = Vec::new();
                    while let Some(start) = next.reserve(3, 1000) {
                        mine.push(start);
                    }
                    mine
                })
            })
            .map(|t| t.join().expect("a reserving thread"))
            .collect();
        let mut starts: Vec<u64> = taken.into_iter().flatten().collect();
        starts.sort_unstable();
        assert!(starts.iter().all(|s| s + 3 <= 1000), "every range is inside the limit");
        assert!(starts.windows(2).all(|w| w[0] + 3 <= w[1]), "no two ranges overlap");
        assert_eq!(starts.len(), 333, "every whole range below the limit was handed out");
    }

    #[test]
    fn a_counted_swap_fails_once_the_offset_left_and_came_back() {
        let head = Tagged32::new(5, 0);
        let seen = head.load();
        let moved = head.swap_counted(seen, 9).expect("a swap from what was seen");
        head.swap_counted(moved, 5).expect("the offset comes back");
        assert_eq!(head.load(), (5, 2));
        assert_eq!(head.swap_counted(seen, 7), Err((5, 2)), "the stale tag refuses the swap");
    }

    #[test]
    fn pairs_and_quads_change_whole() {
        let pair = Pair32::new(1, 2);
        assert_eq!(pair.compare_exchange((1, 2), (3, 4)), Ok(()));
        assert_eq!(pair.compare_exchange((1, 2), (5, 6)), Err((3, 4)));
        if !Quad32::supported() {
            return;
        }
        let mut buf = [0u128; 1];
        // SAFETY: a u128 is 16-byte aligned and used only as this word.
        let quad = unsafe { Quad32::at(buf.as_mut_ptr() as *mut u8) };
        quad.store([1, 2, 3, 4]);
        assert_eq!(quad.load(), [1, 2, 3, 4]);
        assert_eq!(quad.compare_exchange([1, 2, 3, 4], [5, 6, 7, 8]), Ok(()));
        assert_eq!(quad.compare_exchange([1, 2, 3, 4], [0, 0, 0, 0]), Err([5, 6, 7, 8]));
        quad.store([1, 2, 0, 0]);
        let v = quad.load_versioned();
        assert_eq!(quad.swap_versioned(v, 3, 4), Ok((3, 4, 1)));
        assert_eq!(quad.swap_versioned(v, 9, 9), Err((3, 4, 1)));
    }

    #[test]
    fn a_relative_pointer_follows_a_moved_block() {
        #[repr(C)]
        struct Block {
            link: AtomicRel<u64>,
            value: u64,
        }
        let mut a = Box::new(Block { link: AtomicRel::null(), value: 42 });
        assert!(a.link.load(Ordering::Acquire).is_null());
        let at = &raw const a.value;
        a.link.store(at, Ordering::Release);
        // SAFETY: the pointee is the block's own field.
        assert_eq!(unsafe { *a.link.load(Ordering::Acquire) }, 42);
        let mut b = Box::new(Block { link: AtomicRel::null(), value: 0 });
        b.value = a.value;
        b.link = AtomicRel { off: AtomicIsize::new(a.link.off.load(Ordering::Relaxed)), _to: PhantomData };
        a.value = 7;
        // SAFETY: the copy's link names the copy's own field.
        assert_eq!(unsafe { *b.link.load(Ordering::Acquire) }, 42, "the copied link names the copy");
    }

    const ROUNDS: u32 = 20_000;

    fn mapping(path: &std::path::Path) -> memmap2::MmapMut {
        let file = std::fs::OpenOptions::new().read(true).write(true).open(path).expect("the shared file");
        // SAFETY: a file this test made, mapped while it lives.
        unsafe { memmap2::MmapMut::map_mut(&file) }.expect("a mapping")
    }

    /// Add `ROUNDS` to the Tagged32 at byte 0 and to each index of the Quad32
    /// at byte 16, each by retried swaps.
    fn count_in(path: &std::path::Path) {
        let mut map = mapping(path);
        let base = map.as_mut_ptr();
        // SAFETY: byte 0 of a page-aligned mapping, used only as this word.
        let tagged = unsafe { &*(base as *const Tagged32) };
        // SAFETY: byte 16 is 16-byte aligned and used only as this word.
        let quad = unsafe { Quad32::at(base.add(16)) };
        for _ in 0..ROUNDS {
            let mut seen = tagged.load();
            while let Err(found) = tagged.compare_exchange(seen, (seen.0 + 1, seen.1)) {
                seen = found;
            }
            let mut q = quad.load();
            while let Err(found) = quad.compare_exchange(q, q.map(|x| x + 1)) {
                q = found;
            }
        }
    }

    /// Run only as the child of `tagged_and_quad_words_count_across_processes`.
    #[test]
    #[ignore = "run only as the child of tagged_and_quad_words_count_across_processes"]
    fn child_counts_in_the_mapped_words() {
        let path = match std::env::var("SUBETHA_ATOMIC_OFFSET_CHILD") {
            Ok(path) => std::path::PathBuf::from(path),
            Err(std::env::VarError::NotPresent) => return,
            Err(e) => panic!("SUBETHA_ATOMIC_OFFSET_CHILD: {e}"),
        };
        count_in(&path);
    }

    #[test]
    fn tagged_and_quad_words_count_across_processes() {
        if !Quad32::supported() {
            return;
        }
        let dir = std::env::temp_dir().join(format!("subetha_atomic_offset_{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("a folder");
        let path = dir.join("words");
        std::fs::write(&path, [0u8; 4096]).expect("the shared file");
        let exe = std::env::current_exe().expect("the test binary");
        let mut child = std::process::Command::new(exe)
            .args(["--exact", "atomic_offset::tests::child_counts_in_the_mapped_words", "--ignored"])
            .env("SUBETHA_ATOMIC_OFFSET_CHILD", &path)
            .spawn()
            .expect("a child");
        count_in(&path);
        assert!(child.wait().expect("the child").success());
        let mut map = mapping(&path);
        let base = map.as_mut_ptr();
        // SAFETY: as in `count_in`.
        let tagged = unsafe { &*(base as *const Tagged32) };
        // SAFETY: as in `count_in`.
        let quad = unsafe { Quad32::at(base.add(16)) };
        assert_eq!(tagged.load().0, 2 * ROUNDS, "no increment lost across the two processes");
        assert_eq!(quad.load(), [2 * ROUNDS; 4]);
        drop(map);
        if let Err(e) = std::fs::remove_dir_all(&dir) {
            eprintln!("could not remove {}: {e}", dir.display());
        }
    }
}
