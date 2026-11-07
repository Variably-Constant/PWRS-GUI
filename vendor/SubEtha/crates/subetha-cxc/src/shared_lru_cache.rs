//! `SharedLRUCache<K, V>` - a cache any number of processes read and write
//! at once, holding up to its capacity and evicting by CLOCK.
//!
//! This is the typed face of the cache
//! [`RawLruCache`](crate::raw_lru_cache::RawLruCache) is the byte face of,
//! and both attach to the same file. `K` and `V` are stored as their bytes,
//! as [`SharedHashMap`](crate::shared_hash_map::SharedHashMap) stores its
//! keys, so each must be plain data with no padding and no pointer, and every
//! process attached to one cache must agree on them; two keys are the same
//! key when their bytes are.
//!
//! # Concurrency
//!
//! Every operation runs in any number of processes at once, and none waits
//! on another, live or dead. A snapshot taken beside writers lists entries
//! as they stood when the walk reached them.
//!
//! # Recency
//!
//! A put and a touch mark an entry referenced, and a get leaves the mark as
//! it was. Eviction's hand passes a referenced entry once, clearing its mark,
//! and evicts the first unreferenced entry it reaches, so the entry evicted is
//! close to the least recently used one rather than exactly it. A read that
//! should keep its entry cached is a [`get_and_touch`](SharedLRUCache::get_and_touch);
//! a plain [`get`](SharedLRUCache::get) costs no write.
//!
//! # Files
//!
//! One file, `<base>.cache.bin`.

use std::marker::PhantomData;
use std::mem::{size_of, MaybeUninit};
use std::path::Path;

use crate::clock_cache::{CacheCore, CacheError};

#[derive(Debug)]
pub enum LRUError {
    /// The file holds a cache of another shape.
    LayoutMismatch(crate::layout::Mismatch),
    /// The operating system refused to open, size or map the file, or the
    /// processor has no lock-free 16-byte compare-and-swap
    /// (`ErrorKind::Unsupported`); the error carries the code and message.
    IoError(std::io::Error),
    /// No frame could be freed for a new entry: every frame is being filled
    /// by a writer still alive.
    Full,
}

impl std::fmt::Display for LRUError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::LayoutMismatch(m) => write!(f, "a cache of another shape: {m}"),
            Self::IoError(e) => write!(f, "io: {e}"),
            Self::Full => write!(f, "every frame is being filled by a live writer"),
        }
    }
}

impl std::error::Error for LRUError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::IoError(e) => Some(e),
            Self::LayoutMismatch(_) | Self::Full => None,
        }
    }
}

impl From<std::io::Error> for LRUError {
    fn from(e: std::io::Error) -> Self {
        Self::IoError(e)
    }
}

impl From<CacheError> for LRUError {
    fn from(e: CacheError) -> Self {
        match e {
            CacheError::Mismatch(m) => Self::LayoutMismatch(m),
            CacheError::Io(e) => Self::IoError(e),
            CacheError::Full => Self::Full,
        }
    }
}

/// The bytes of `value`, which the type's contract says are all data.
fn bytes_of<T: Copy>(value: &T) -> &[u8] {
    // SAFETY: a `T` is `size_of::<T>()` readable bytes with no padding, by
    // the contract on `SharedLRUCache`.
    unsafe { std::slice::from_raw_parts(value as *const T as *const u8, size_of::<T>()) }
}

/// The `T` whose bytes `bytes` are.
///
/// # Safety
/// `bytes` are the bytes of a `T`.
unsafe fn read_as<T: Copy>(bytes: &[u8]) -> T {
    debug_assert_eq!(bytes.len(), size_of::<T>());
    // SAFETY: the caller's contract.
    unsafe { std::ptr::read_unaligned(bytes.as_ptr() as *const T) }
}

/// Room for one `T` the cache copies in: zeroed, so its bytes can be lent as
/// a slice, and read as a `T` only once the cache says it wrote one.
struct Room<T>(MaybeUninit<T>);

impl<T: Copy> Room<T> {
    fn new() -> Self {
        Self(MaybeUninit::zeroed())
    }

    fn bytes(&mut self) -> &mut [u8] {
        // SAFETY: the room was zeroed, so every byte is initialized, and the
        // slice covers the room exactly.
        unsafe { std::slice::from_raw_parts_mut(self.0.as_mut_ptr() as *mut u8, size_of::<T>()) }
    }

    /// The `T` in the room, when `wrote` says the cache copied one in.
    fn filled(self, wrote: bool) -> Option<T> {
        // SAFETY: the cache copies in whole stored values, and a stored value
        // is the bytes of a `T` by the contract on `SharedLRUCache`.
        wrote.then(|| unsafe { self.0.assume_init() })
    }
}

pub struct SharedLRUCache<
    K: Copy + Eq + Default + 'static,
    V: Copy + Default + 'static,
> {
    core: CacheCore,
    _phantom: PhantomData<(K, V)>,
    header_sidecar: subetha_core::HandshakeHeader,
    ring_sidecar: Box<subetha_core::ObservationRing>,
}

impl<
    K: Copy + Eq + Default + Send + Sync + 'static,
    V: Copy + Default + Send + Sync + 'static,
> subetha_sidecar::AdaptiveInstance for SharedLRUCache<K, V> {
    fn header(&self) -> &subetha_core::HandshakeHeader { &self.header_sidecar }
    fn ring(&self) -> &subetha_core::ObservationRing { &self.ring_sidecar }
    fn make_policy(&self) -> Box<dyn subetha_sidecar::Policy> {
        Box::new(subetha_sidecar::NoMigrationPolicy)
    }
}

impl<
    K: Copy + Eq + Default + 'static,
    V: Copy + Default + 'static,
> SharedLRUCache<K, V> {
    const KEY: usize = {
        assert!(size_of::<K>() > 0, "a cache key carries at least one byte");
        size_of::<K>()
    };

    fn with_core(core: CacheCore) -> Self {
        Self {
            core,
            _phantom: PhantomData,
            header_sidecar: subetha_core::HandshakeHeader::new(),
            ring_sidecar: Box::new(subetha_core::ObservationRing::new()),
        }
    }

    /// Obtain the cache under `base_path` holding up to `capacity` entries:
    /// an empty one when none exists, or the one there, entries in place,
    /// when it has this shape.
    pub fn create(
        base_path: impl AsRef<Path>, capacity: u32,
    ) -> Result<Self, LRUError> {
        assert!(capacity >= 1);
        let core = CacheCore::create(base_path.as_ref(), capacity as usize, Self::KEY, size_of::<V>())?;
        Ok(Self::with_core(core))
    }

    /// Attach to the cache another process created under `base_path`.
    pub fn open(
        base_path: impl AsRef<Path>, capacity: u32,
    ) -> Result<Self, LRUError> {
        let core = CacheCore::open(base_path.as_ref(), capacity as usize, Self::KEY, size_of::<V>())?;
        Ok(Self::with_core(core))
    }

    #[inline]
    pub fn capacity(&self) -> u32 { self.core.capacity() as u32 }

    /// Current number of entries.
    pub fn len(&self) -> usize {
        self.core.len()
    }

    pub fn is_empty(&self) -> bool { self.len() == 0 }

    /// The value for `key`, leaving its mark as it was.
    pub fn get(&self, key: &K) -> Option<V> {
        let mut value = Room::<V>::new();
        let hit = self.core.get(bytes_of(key), value.bytes());
        self.ring_sidecar.push_op(
            crate::sidecar_ops::lru_cache::OP_GET,
            if hit { 0 } else { 2 },
        );
        value.filled(hit)
    }

    /// Whether the cache holds `key`, leaving its mark as it was.
    pub fn contains_key(&self, key: &K) -> bool {
        self.core.contains(bytes_of(key))
    }

    /// Mark `key` referenced, so eviction passes it once. `false` when the
    /// cache does not hold it.
    pub fn touch(&self, key: &K) -> bool {
        let hit = self.core.touch(bytes_of(key), None);
        self.ring_sidecar.push_op(crate::sidecar_ops::lru_cache::OP_TOUCH, if hit { 0 } else { 2 });
        hit
    }

    /// The value for `key`, marking it referenced.
    pub fn get_and_touch(&self, key: &K) -> Option<V> {
        let mut value = Room::<V>::new();
        let hit = self.core.touch(bytes_of(key), Some(value.bytes()));
        self.ring_sidecar.push_op(
            crate::sidecar_ops::lru_cache::OP_GET,
            if hit { 0 } else { 2 },
        );
        if hit {
            self.ring_sidecar.push_op(crate::sidecar_ops::lru_cache::OP_TOUCH, 0);
        }
        value.filled(hit)
    }

    /// Store `value` under `key`, marked referenced. A new key in a full
    /// cache evicts first. The previous value when `key` was present.
    pub fn put(&self, key: K, value: V) -> Result<Option<V>, LRUError> {
        let mut old = Room::<V>::new();
        let r = self.core.put(bytes_of(&key), bytes_of(&value), Some(old.bytes()));
        self.ring_sidecar.push_op(
            crate::sidecar_ops::lru_cache::OP_PUT,
            if r.is_err() { 1 } else { 0 },
        );
        Ok(old.filled(r?))
    }

    /// Drop `key`, handing back its value when the cache held it.
    pub fn remove(&self, key: &K) -> Option<V> {
        let mut value = Room::<V>::new();
        let hit = self.core.remove(bytes_of(key), value.bytes());
        self.ring_sidecar.push_op(
            crate::sidecar_ops::lru_cache::OP_REMOVE,
            if hit { 0 } else { 2 },
        );
        value.filled(hit)
    }

    /// Evict the entry eviction's hand reaches first unreferenced, handing
    /// back its key and value; `None` when the cache is empty.
    pub fn evict_oldest(&self) -> Option<(K, V)> {
        let mut key = Room::<K>::new();
        let mut value = Room::<V>::new();
        let hit = self.core.evict(key.bytes(), value.bytes());
        self.ring_sidecar.push_op(
            crate::sidecar_ops::lru_cache::OP_EVICT,
            if hit { 0 } else { 2 },
        );
        key.filled(hit).zip(value.filled(hit))
    }

    /// Every entry, last to be evicted first: the reverse of
    /// [`snapshot_lru_first`](Self::snapshot_lru_first).
    pub fn snapshot_mru_first(&self) -> Vec<(K, V)> {
        let mut entries = self.snapshot_lru_first();
        entries.reverse();
        entries
    }

    /// Every entry in the order eviction would take them: from the hand on,
    /// the unreferenced entries, then the referenced ones.
    pub fn snapshot_lru_first(&self) -> Vec<(K, V)> {
        let mut entries = Vec::with_capacity(self.len());
        self.core.for_each_in_eviction_order(|k, v| {
            // SAFETY: the cache hands out whole stored keys and values, the
            // bytes of a `K` and a `V` by the contract on `SharedLRUCache`.
            entries.push(unsafe { (read_as::<K>(k), read_as::<V>(v)) });
        });
        entries
    }

    /// Push the cache's file to disk and wait for it.
    pub fn flush(&self) -> Result<(), LRUError> {
        Ok(self.core.flush()?)
    }

    /// Start pushing the cache's file to disk without waiting.
    pub fn flush_async(&self) -> Result<(), LRUError> {
        Ok(self.core.flush_async()?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn tmp_base(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!("subetha-lru-{name}-{}", std::process::id()))
    }

    /// Remove the cache file. An absent file is the expected case before a
    /// test runs; any other failure is reported, since a file left behind
    /// hands the next run this run's entries.
    fn cleanup(base: &Path) {
        let p = CacheCore::path(base);
        match std::fs::remove_file(&p) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => panic!("could not clear {}: {e}", p.display()),
        }
    }

    fn fresh(name: &str, capacity: u32) -> (SharedLRUCache<u32, u32>, PathBuf) {
        let base = tmp_base(name);
        cleanup(&base);
        (SharedLRUCache::create(&base, capacity).expect("created"), base)
    }

    /// Unmap the cache, then remove its file.
    fn done<K: Copy + Eq + Default + 'static, V: Copy + Default + 'static>(c: SharedLRUCache<K, V>, base: PathBuf) {
        drop(c);
        cleanup(&base);
    }

    #[test]
    fn create_initial_state_is_empty() {
        let (c, base) = fresh("init", 16);
        assert_eq!(c.capacity(), 16);
        assert_eq!(c.len(), 0);
        assert!(c.is_empty());
        assert_eq!(c.get(&1), None);
        done(c, base);
    }

    #[test]
    fn put_get_round_trip() {
        let (c, base) = fresh("rt", 16);
        c.put(1, 100).unwrap();
        c.put(2, 200).unwrap();
        c.put(3, 300).unwrap();
        assert_eq!(c.get(&1), Some(100));
        assert_eq!(c.get(&2), Some(200));
        assert_eq!(c.get(&3), Some(300));
        assert_eq!(c.get(&999), None);
        assert!(c.contains_key(&2));
        assert!(!c.contains_key(&999));
        assert_eq!(c.len(), 3);
        done(c, base);
    }

    #[test]
    fn a_put_of_a_present_key_replaces_it_and_marks_it_referenced() {
        let (c, base) = fresh("update", 16);
        for (k, v) in [(1, 100), (2, 200), (3, 300)] {
            c.put(k, v).unwrap();
        }
        // The hand clears every mark once around, then takes the first entry.
        assert_eq!(c.evict_oldest(), Some((1, 100)));
        assert_eq!(c.put(2, 222).unwrap(), Some(200));
        assert_eq!(c.get(&2), Some(222));
        assert_eq!(c.len(), 2);
        // 3 is unmarked and 2 marked again, so eviction would take 3 first.
        assert_eq!(c.snapshot_lru_first(), vec![(3, 300), (2, 222)]);
        done(c, base);
    }

    #[test]
    fn a_get_leaves_the_mark_and_a_touch_sets_it() {
        let (c, base) = fresh("marks", 16);
        for k in 1..=4u32 {
            c.put(k, k * 10).unwrap();
        }
        assert_eq!(c.evict_oldest(), Some((1, 10)));
        // 2, 3 and 4 are unmarked now; a get leaves 2 so, a touch marks 3.
        assert_eq!(c.get(&2), Some(20));
        assert!(c.touch(&3));
        assert_eq!(c.evict_oldest(), Some((2, 20)), "a get does not keep an entry");
        assert_eq!(c.evict_oldest(), Some((4, 40)), "a touch does, once");
        assert_eq!(c.evict_oldest(), Some((3, 30)));
        assert_eq!(c.evict_oldest(), None);
        done(c, base);
    }

    #[test]
    fn touch_nonexistent_returns_false() {
        let (c, base) = fresh("touch-none", 8);
        c.put(1, 100).unwrap();
        assert!(!c.touch(&999));
        done(c, base);
    }

    #[test]
    fn get_and_touch_reads_the_value_and_marks_it() {
        let (c, base) = fresh("get-touch", 16);
        for k in 1..=4u32 {
            c.put(k, k * 10).unwrap();
        }
        assert_eq!(c.evict_oldest(), Some((1, 10)));
        assert_eq!(c.get_and_touch(&2), Some(20));
        assert_eq!(c.get_and_touch(&9), None);
        assert_eq!(c.evict_oldest(), Some((3, 30)), "the touched entry is passed over");
        done(c, base);
    }

    #[test]
    fn eviction_at_capacity_takes_the_first_unmarked_entry() {
        let (c, base) = fresh("evict", 3);
        c.put(1, 10).unwrap();
        c.put(2, 20).unwrap();
        c.put(3, 30).unwrap();
        // Every entry is marked as it lands, so the hand clears each mark once
        // around and takes the first it reaches again.
        let prev = c.put(4, 40).unwrap();
        assert_eq!(prev, None);
        assert_eq!(c.len(), 3);
        assert_eq!(c.get(&1), None, "key 1 should have been evicted");
        assert_eq!(c.get(&2), Some(20));
        assert_eq!(c.get(&3), Some(30));
        assert_eq!(c.get(&4), Some(40));
        done(c, base);
    }

    #[test]
    fn a_touch_keeps_an_entry_through_the_next_eviction() {
        let (c, base) = fresh("touch-evict", 3);
        c.put(1, 10).unwrap();
        c.put(2, 20).unwrap();
        c.put(3, 30).unwrap();
        c.put(4, 40).unwrap();
        assert_eq!(c.get(&1), None);
        // 2 and 3 are unmarked since the hand passed them; touch 2.
        assert!(c.touch(&2));
        c.put(5, 50).unwrap();
        assert_eq!(c.get(&2), Some(20), "touched key 2 should survive");
        assert_eq!(c.get(&3), None, "untouched key 3 should be evicted");
        assert_eq!(c.get(&4), Some(40));
        assert_eq!(c.get(&5), Some(50));
        done(c, base);
    }

    #[test]
    fn remove_hands_back_the_value_and_drops_the_entry() {
        let (c, base) = fresh("rm", 8);
        c.put(1, 10).unwrap();
        c.put(2, 20).unwrap();
        c.put(3, 30).unwrap();
        let v = c.remove(&2).unwrap();
        assert_eq!(v, 20);
        assert_eq!(c.len(), 2);
        assert_eq!(c.get(&2), None);
        assert_eq!(c.remove(&2), None, "already gone");
        // Other keys still present.
        assert_eq!(c.get(&1), Some(10));
        assert_eq!(c.get(&3), Some(30));
        let snap = c.snapshot_mru_first();
        let keys: Vec<u32> = snap.iter().map(|(k, _)| *k).collect();
        assert!(!keys.contains(&2));
        done(c, base);
    }

    #[test]
    fn evict_oldest_takes_what_the_hand_picks() {
        let (c, base) = fresh("evict-oldest", 8);
        c.put(1, 10).unwrap();
        c.put(2, 20).unwrap();
        c.put(3, 30).unwrap();
        // All three are marked; the hand clears them and takes the first it
        // reaches again.
        let evicted = c.evict_oldest().unwrap();
        assert_eq!(evicted, (1, 10));
        assert_eq!(c.len(), 2);
        assert_eq!(c.get(&1), None);
        done(c, base);
    }

    #[test]
    fn evict_oldest_on_empty_returns_none() {
        let (c, base) = fresh("evict-empty", 4);
        assert_eq!(c.evict_oldest(), None);
        done(c, base);
    }

    #[test]
    fn snapshot_mru_and_lru_first_are_reverses() {
        let (c, base) = fresh("snap", 8);
        for (k, v) in [(1, 10), (2, 20), (3, 30)] {
            c.put(k, v).unwrap();
        }
        let mru = c.snapshot_mru_first();
        let mut lru = c.snapshot_lru_first();
        assert_eq!(lru.len(), 3);
        lru.reverse();
        assert_eq!(mru, lru);
        done(c, base);
    }

    #[test]
    fn cross_handle_visibility() {
        let (writer, base) = fresh("cross-handle", 8);
        let reader: SharedLRUCache<u32, u32> = SharedLRUCache::open(&base, 8).unwrap();
        writer.put(42, 4242).unwrap();
        writer.put(7, 77).unwrap();
        assert_eq!(reader.get(&42), Some(4242));
        assert_eq!(reader.get(&7), Some(77));
        assert_eq!(writer.evict_oldest(), Some((42, 4242)));
        assert_eq!(reader.get(&42), None);
        // A write through the reader reaches the writer.
        assert!(reader.touch(&7));
        reader.put(8, 88).unwrap();
        assert_eq!(writer.get(&8), Some(88));
        drop(reader);
        done(writer, base);
    }

    #[test]
    fn struct_key_value_round_trip() {
        #[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Default, Hash)]
        #[repr(C)]
        struct UserKey { realm: u32, user: u32 }
        #[derive(Clone, Copy, Debug, PartialEq, Default)]
        #[repr(C)]
        struct Session { token: u64, expires_us: u64 }
        let base = tmp_base("struct");
        cleanup(&base);
        let c: SharedLRUCache<UserKey, Session> = SharedLRUCache::create(&base, 8).unwrap();
        let k = UserKey { realm: 1, user: 42 };
        let v = Session { token: 0xDEAD_BEEF, expires_us: 9_999_999_999 };
        c.put(k, v).unwrap();
        assert_eq!(c.get(&k), Some(v));
        assert_eq!(c.get(&UserKey { realm: 2, user: 42 }), None);
        done(c, base);
    }

    #[test]
    fn disk_persistence_survives_reopen() {
        let (c, base) = fresh("disk", 8);
        c.put(1, 100).unwrap();
        c.put(2, 200).unwrap();
        c.put(3, 300).unwrap();
        c.flush().unwrap();
        c.flush_async().unwrap();
        drop(c);
        let c2: SharedLRUCache<u32, u32> = SharedLRUCache::open(&base, 8).unwrap();
        assert_eq!(c2.len(), 3);
        assert_eq!(c2.get(&1), Some(100));
        assert_eq!(c2.get(&2), Some(200));
        assert_eq!(c2.get(&3), Some(300));
        done(c2, base);
    }

    #[test]
    fn a_cache_of_another_shape_is_refused_by_its_field() {
        let (c, base) = fresh("shape", 8);
        drop(c);
        match SharedLRUCache::<u32, u64>::open(&base, 8) {
            Err(LRUError::LayoutMismatch(m)) => assert_eq!(m.field, "value size"),
            other => panic!("a cache of another value size was attached: {:?}", other.err()),
        }
        cleanup(&base);
    }

    #[test]
    fn many_evictions_keep_the_newest_entries() {
        let (c, base) = fresh("many-evict", 10);
        for k in 0..50u32 {
            c.put(k, k * 10).unwrap();
        }
        // Each entry lands marked and none is touched, so the hand takes
        // entries in the order they landed.
        assert_eq!(c.len(), 10);
        for k in 0..40u32 {
            assert_eq!(c.get(&k), None, "key {k} should have been evicted");
        }
        for k in 40..50u32 {
            assert_eq!(c.get(&k), Some(k * 10), "key {k} should be present");
        }
        done(c, base);
    }

    #[test]
    fn touched_keys_survive_subsequent_evictions() {
        let (c, base) = fresh("touch-survival", 5);
        for k in 0..5u32 {
            c.put(k, k * 10).unwrap();
        }
        // The first new key clears every mark once around and takes key 0.
        c.put(100, 1000).unwrap();
        assert_eq!(c.get(&0), None);
        // Touch 1 and 2; the next two new keys pass them and take 3 and 4.
        assert!(c.touch(&1));
        assert!(c.touch(&2));
        c.put(101, 1010).unwrap();
        c.put(102, 1020).unwrap();
        assert_eq!(c.get(&1), Some(10));
        assert_eq!(c.get(&2), Some(20));
        assert_eq!(c.get(&3), None);
        assert_eq!(c.get(&4), None);
        assert_eq!(c.get(&100), Some(1000));
        assert_eq!(c.get(&101), Some(1010));
        assert_eq!(c.get(&102), Some(1020));
        done(c, base);
    }

    #[test]
    fn writers_in_many_threads_each_land_their_entries() {
        let (c, base) = fresh("threads", 4096);
        std::thread::scope(|s| {
            for t in 0..8u32 {
                let c = &c;
                s.spawn(move || {
                    for i in 0..32u32 {
                        let k = t * 1000 + i;
                        assert_eq!(c.put(k, k + 1).expect("put"), None, "each key is put once");
                    }
                });
            }
        });
        assert_eq!(c.len(), 8 * 32);
        for t in 0..8u32 {
            for i in 0..32u32 {
                let k = t * 1000 + i;
                assert_eq!(c.get(&k), Some(k + 1), "key {k}");
            }
        }
        done(c, base);
    }
}
