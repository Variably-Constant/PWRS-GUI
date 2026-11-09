//! `RawLruCache` - the shared cache at key and value sizes chosen at run
//! time.
//!
//! [`SharedLRUCache`](crate::shared_lru_cache::SharedLRUCache) is generic
//! over its key and value types, which a caller binding through C cannot
//! name. This is the same cache, the CLOCK cache both are faces of, taking
//! those sizes as arguments instead.
//!
//! Keys and values cross as byte slices of exactly the declared size. A
//! slice of any other length is refused rather than padded or truncated,
//! because a key silently changed is a key that names a different entry.
//! Two keys are the same key when their bytes are.
//!
//! Any number of processes put, touch, remove and evict at once, and none
//! waits on another, live or dead. A put and a touch mark an entry
//! referenced, and eviction's hand passes a referenced entry once, clearing
//! the mark, so the entry evicted is close to the least recently used one
//! rather than exactly it.

use std::path::Path;

use crate::clock_cache::{CacheCore, CacheError};
use crate::layout::Mismatch;

/// Why an operation on the cache could not be carried out.
#[derive(Debug)]
pub enum RawLruError {
    /// A key or value slice was not the size this cache was built for.
    WrongSize { expected: usize, found: usize },
    /// The capacity asked for cannot hold anything.
    ZeroCapacity,
    /// The file holds a cache of another shape.
    LayoutMismatch(Mismatch),
    /// The operating system refused to open, size or map the file, or the
    /// processor has no lock-free 16-byte compare-and-swap
    /// (`ErrorKind::Unsupported`); the error carries the code and message.
    IoError(std::io::Error),
    /// No frame could be freed for a new entry: every frame is being filled
    /// by a writer still alive.
    Full,
}

impl std::fmt::Display for RawLruError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::WrongSize { expected, found } => {
                write!(f, "the slice is {found} bytes where the cache holds {expected}")
            }
            Self::ZeroCapacity => write!(f, "a capacity of zero cannot hold anything"),
            Self::LayoutMismatch(m) => write!(f, "a cache of another shape: {m}"),
            Self::IoError(e) => write!(f, "io: {e}"),
            Self::Full => write!(f, "every frame is being filled by a live writer"),
        }
    }
}

impl std::error::Error for RawLruError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::IoError(e) => Some(e),
            Self::WrongSize { .. } | Self::ZeroCapacity | Self::LayoutMismatch(_) | Self::Full => None,
        }
    }
}

impl From<CacheError> for RawLruError {
    fn from(e: CacheError) -> Self {
        match e {
            CacheError::Mismatch(m) => Self::LayoutMismatch(m),
            CacheError::Io(e) => Self::IoError(e),
            CacheError::Full => Self::Full,
        }
    }
}

pub struct RawLruCache {
    core: CacheCore,
}

impl RawLruCache {
    /// Create the cache under `base_path`, or attach to one already there
    /// with the same shape.
    pub fn create(base_path: impl AsRef<Path>, capacity: u32, key_size: usize, value_size: usize) -> Result<Self, RawLruError> {
        Self::check_shape(capacity, key_size, value_size)?;
        let core = CacheCore::create(base_path.as_ref(), capacity as usize, key_size, value_size)?;
        Ok(Self { core })
    }

    /// Attach to a cache another process created under `base_path`.
    pub fn open(base_path: impl AsRef<Path>, capacity: u32, key_size: usize, value_size: usize) -> Result<Self, RawLruError> {
        Self::check_shape(capacity, key_size, value_size)?;
        let core = CacheCore::open(base_path.as_ref(), capacity as usize, key_size, value_size)?;
        Ok(Self { core })
    }

    fn check_shape(capacity: u32, key_size: usize, value_size: usize) -> Result<(), RawLruError> {
        if capacity == 0 {
            return Err(RawLruError::ZeroCapacity);
        }
        if key_size == 0 || value_size == 0 {
            return Err(RawLruError::WrongSize { expected: 1, found: 0 });
        }
        Ok(())
    }

    pub fn capacity(&self) -> u32 {
        self.core.capacity() as u32
    }

    pub fn key_size(&self) -> usize {
        self.core.key_size()
    }

    pub fn value_size(&self) -> usize {
        self.core.value_size()
    }

    /// Entries the cache holds.
    pub fn len(&self) -> usize {
        self.core.len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    fn sized(&self, what: &[u8], expected: usize) -> Result<(), RawLruError> {
        if what.len() == expected {
            Ok(())
        } else {
            Err(RawLruError::WrongSize { expected, found: what.len() })
        }
    }

    /// The value for `key` into `value_out`, leaving it unreferenced as it
    /// was. `false` when the cache does not hold it.
    pub fn get(&self, key: &[u8], value_out: &mut [u8]) -> Result<bool, RawLruError> {
        self.sized(key, self.key_size())?;
        self.sized(value_out, self.value_size())?;
        Ok(self.core.get(key, value_out))
    }

    /// Whether `key` is in the cache, without referencing it.
    pub fn contains_key(&self, key: &[u8]) -> Result<bool, RawLruError> {
        self.sized(key, self.key_size())?;
        Ok(self.core.contains(key))
    }

    /// Mark `key` referenced, so eviction passes it once. `false` when it is
    /// absent.
    pub fn touch(&self, key: &[u8]) -> Result<bool, RawLruError> {
        self.sized(key, self.key_size())?;
        Ok(self.core.touch(key, None))
    }

    /// The value for `key`, marking it referenced.
    pub fn get_and_touch(&self, key: &[u8], value_out: &mut [u8]) -> Result<bool, RawLruError> {
        self.sized(key, self.key_size())?;
        self.sized(value_out, self.value_size())?;
        Ok(self.core.touch(key, Some(value_out)))
    }

    /// Store `value` under `key`, referenced, evicting first where the cache
    /// is full and the key is new.
    ///
    /// `true` when the key was already present and its value replaced.
    pub fn put(&self, key: &[u8], value: &[u8]) -> Result<bool, RawLruError> {
        self.sized(key, self.key_size())?;
        self.sized(value, self.value_size())?;
        Ok(self.core.put(key, value, None)?)
    }

    /// Drop `key`, writing its value into `value_out`. `false` when the
    /// cache did not hold it.
    pub fn remove(&self, key: &[u8], value_out: &mut [u8]) -> Result<bool, RawLruError> {
        self.sized(key, self.key_size())?;
        self.sized(value_out, self.value_size())?;
        Ok(self.core.remove(key, value_out))
    }

    /// Evict the entry eviction's hand reaches first unreferenced, writing
    /// its key and value out. `false` when the cache is empty.
    pub fn evict_oldest(&self, key_out: &mut [u8], value_out: &mut [u8]) -> Result<bool, RawLruError> {
        self.sized(key_out, self.key_size())?;
        self.sized(value_out, self.value_size())?;
        Ok(self.core.evict(key_out, value_out))
    }

    /// Push the cache's file to disk and wait for it.
    pub fn flush(&self) -> Result<(), RawLruError> {
        self.core.flush().map_err(RawLruError::IoError)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn scratch(name: &str) -> PathBuf {
        let mut p = std::env::temp_dir();
        p.push(format!("subetha_raw_lru_{name}_{}", std::process::id()));
        p
    }

    /// Clear the cache file. An absent file is the expected case on the first
    /// run; anything else is reported rather than swallowed, since a file
    /// that cannot be cleared makes the next assertion read a previous run's
    /// entries.
    fn cleanup(base: &Path) {
        let p = CacheCore::path(base);
        match std::fs::remove_file(&p) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => panic!("could not clear {}: {e}", p.display()),
        }
    }

    #[test]
    fn put_and_get_at_runtime_sizes() {
        let base = scratch("basic");
        cleanup(&base);
        let c = RawLruCache::create(&base, 4, 8, 16).expect("created");
        assert_eq!(c.key_size(), 8);
        assert_eq!(c.value_size(), 16);

        let key = [1u8; 8];
        let value = [7u8; 16];
        assert!(!c.put(&key, &value).expect("put"), "the key was not there before");
        assert_eq!(c.len(), 1);

        let mut out = [0u8; 16];
        assert!(c.get(&key, &mut out).expect("get"));
        assert_eq!(out, value);

        // Putting the same key again replaces rather than adds.
        let replacement = [9u8; 16];
        assert!(c.put(&key, &replacement).expect("put"), "the key was there");
        assert_eq!(c.len(), 1);
        assert!(c.get(&key, &mut out).expect("get"));
        assert_eq!(out, replacement);

        drop(c);
        cleanup(&base);
    }

    #[test]
    fn a_slice_of_the_wrong_size_is_refused_rather_than_padded() {
        let base = scratch("sizes");
        cleanup(&base);
        let c = RawLruCache::create(&base, 4, 8, 16).expect("created");
        let short = [1u8; 4];
        let value = [0u8; 16];
        assert!(matches!(c.put(&short, &value), Err(RawLruError::WrongSize { expected: 8, found: 4 })));
        let mut small_out = [0u8; 8];
        assert!(matches!(c.get(&[1u8; 8], &mut small_out), Err(RawLruError::WrongSize { expected: 16, found: 8 })));
        drop(c);
        cleanup(&base);
    }

    #[test]
    fn eviction_passes_a_touched_entry_and_takes_an_untouched_one() {
        let base = scratch("evict");
        cleanup(&base);
        let c = RawLruCache::create(&base, 3, 8, 8).expect("created");
        for i in 0u64..3 {
            c.put(&i.to_le_bytes(), &(i * 10).to_le_bytes()).expect("put");
        }
        // Every entry is referenced as it lands, so the hand clears each mark
        // once around and then takes the first it reaches again.
        c.put(&3u64.to_le_bytes(), &30u64.to_le_bytes()).expect("put");
        assert_eq!(c.len(), 3);
        let mut out = [0u8; 8];
        assert!(!c.get(&0u64.to_le_bytes(), &mut out).expect("get"), "taken first in the hand's order");

        // Touch 1 so the hand passes it; 2, untouched since its mark was
        // cleared, goes instead.
        assert!(c.touch(&1u64.to_le_bytes()).expect("touch"));
        c.put(&4u64.to_le_bytes(), &40u64.to_le_bytes()).expect("put");
        assert!(c.get(&1u64.to_le_bytes(), &mut out).expect("get"), "touched, so kept");
        assert!(!c.get(&2u64.to_le_bytes(), &mut out).expect("get"), "evicted");
        assert!(c.get(&3u64.to_le_bytes(), &mut out).expect("get"));
        assert!(c.get(&4u64.to_le_bytes(), &mut out).expect("get"));

        drop(c);
        cleanup(&base);
    }

    #[test]
    fn remove_hands_back_the_value_and_forgets_the_key() {
        let base = scratch("remove");
        cleanup(&base);
        let c = RawLruCache::create(&base, 4, 8, 8).expect("created");
        c.put(&1u64.to_le_bytes(), &42u64.to_le_bytes()).expect("put");
        let mut out = [0u8; 8];
        assert!(c.remove(&1u64.to_le_bytes(), &mut out).expect("remove"));
        assert_eq!(u64::from_le_bytes(out), 42);
        assert!(!c.remove(&1u64.to_le_bytes(), &mut out).expect("remove"), "already gone");
        assert_eq!(c.len(), 0);
        drop(c);
        cleanup(&base);
    }

    #[test]
    fn evicting_an_empty_cache_says_so_rather_than_failing() {
        let base = scratch("empty");
        cleanup(&base);
        let c = RawLruCache::create(&base, 2, 8, 8).expect("created");
        let mut k = [0u8; 8];
        let mut v = [0u8; 8];
        assert!(!c.evict_oldest(&mut k, &mut v).expect("evict"));
        drop(c);
        cleanup(&base);
    }

    #[test]
    fn a_capacity_of_zero_is_refused() {
        let base = scratch("zerocap");
        cleanup(&base);
        assert!(matches!(RawLruCache::create(&base, 0, 8, 8), Err(RawLruError::ZeroCapacity)));
        cleanup(&base);
    }

    #[test]
    fn a_cache_of_another_shape_is_refused_by_its_field() {
        let base = scratch("shape");
        cleanup(&base);
        drop(RawLruCache::create(&base, 4, 8, 8).expect("created"));
        match RawLruCache::open(&base, 4, 8, 16) {
            Err(RawLruError::LayoutMismatch(m)) => assert_eq!(m.field, "value size"),
            other => panic!("a cache of another value size was attached: {:?}", other.err()),
        }
        cleanup(&base);
    }

    #[test]
    fn a_second_handle_on_the_same_file_sees_the_same_entries() {
        let base = scratch("shared");
        cleanup(&base);
        let a = RawLruCache::create(&base, 4, 8, 8).expect("created");
        a.put(&5u64.to_le_bytes(), &55u64.to_le_bytes()).expect("put");

        let b = RawLruCache::open(&base, 4, 8, 8).expect("opened");
        let mut out = [0u8; 8];
        assert!(b.get(&5u64.to_le_bytes(), &mut out).expect("get"));
        assert_eq!(u64::from_le_bytes(out), 55);

        // And a write through the second is seen by the first.
        b.put(&6u64.to_le_bytes(), &66u64.to_le_bytes()).expect("put");
        assert!(a.get(&6u64.to_le_bytes(), &mut out).expect("get"));
        assert_eq!(u64::from_le_bytes(out), 66);

        drop((a, b));
        cleanup(&base);
    }
}
