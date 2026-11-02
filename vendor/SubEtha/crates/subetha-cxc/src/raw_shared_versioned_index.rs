//! The versioned index with its key and value sizes given at run time,
//! for callers that cannot name them in the type system.
//!
//! [`SharedVersionedIndex`](crate::shared_versioned_index::SharedVersionedIndex)
//! fixes both as generic parameters, which suits Rust callers and shuts
//! out every binding that reaches this crate through C. This form is the
//! same index, one lock-free skip list and the same version chains, over
//! keys and values held as bytes. Keys order as unsigned bytes, so a
//! caller that wants numeric order stores its integers big-endian. The
//! typed index orders by `K: Ord`, which puts the same keys in a different
//! order, so the two carry different magics and neither opens the other's
//! file.
//!
//! The index is built over an epoch table the caller already holds, so a
//! binding that hands out that table's pins and tickets as tokens of its
//! own stamps the index from the same table. A ticket crosses as the slot
//! and epoch [`SharedEpochs::claim_ticket`] answers and a pin as its
//! epoch, rather than as guards, and every call holds a pin of its own
//! while it walks, so whatever values it is given, what it reads stays in
//! place.

use std::cmp::Ordering;
use std::ops::Bound;
use std::path::Path;
use std::sync::Arc;

use crate::shared_epochs::{Epoch, SharedEpochs};
use crate::shared_versioned_index::{Index, IndexError, Shape, DEFAULT_CHUNK_BITS};

/// "VINDEXR1": a raw index's main file is never opened as a typed one.
pub const RAW_INDEX_MAGIC: u64 = 0x5649_4E44_4558_5231;

/// What [`RawSharedVersionedIndex::range_at`] answers: each row's key and
/// value bytes, and the last key examined when the walk stopped short of
/// the range's end.
pub type RawRangeRows = (Vec<(Vec<u8>, Vec<u8>)>, Option<Vec<u8>>);

/// Keys and values as bytes of sizes fixed at run time, keys ordered as
/// unsigned bytes.
pub(crate) struct Bytes {
    key: usize,
    value: usize,
}

impl Shape for Bytes {
    const MAGIC: u64 = RAW_INDEX_MAGIC;

    fn key_size(&self) -> usize {
        self.key
    }

    fn key_align(&self) -> usize {
        1
    }

    fn value_size(&self) -> usize {
        self.value
    }

    fn value_align(&self) -> usize {
        1
    }

    unsafe fn cmp(&self, a: *const u8, b: *const u8) -> Ordering {
        // SAFETY: the caller passes two keys of `self.key` bytes.
        unsafe { std::slice::from_raw_parts(a, self.key).cmp(std::slice::from_raw_parts(b, self.key)) }
    }
}

#[derive(Debug)]
pub struct RawSharedVersionedIndex {
    core: Index<Bytes>,
}

impl RawSharedVersionedIndex {
    /// Obtain the index at `path` over the epoch table `epochs`, for keys of
    /// `key_size` bytes and values of `value_size`, making it when the file
    /// does not exist and attaching to it when it does.
    pub fn create(path: impl AsRef<Path>, epochs: Arc<SharedEpochs>, key_size: usize, value_size: usize) -> Result<Self, IndexError> {
        Self::create_with_chunks(path, epochs, key_size, value_size, DEFAULT_CHUNK_BITS)
    }

    /// [`create`](Self::create) with chunk files of `1 << chunk_bits`
    /// slots, from 4 to 20 bits. An index is attached at the chunk size it
    /// was made with.
    pub fn create_with_chunks(
        path: impl AsRef<Path>,
        epochs: Arc<SharedEpochs>,
        key_size: usize,
        value_size: usize,
        chunk_bits: u32,
    ) -> Result<Self, IndexError> {
        Ok(Self { core: Index::create(path.as_ref(), epochs, Bytes { key: key_size, value: value_size }, chunk_bits)? })
    }

    /// Attach to an existing index over the epoch table `epochs`, made for
    /// keys of `key_size` bytes and values of `value_size`.
    pub fn open(path: impl AsRef<Path>, epochs: Arc<SharedEpochs>, key_size: usize, value_size: usize) -> Result<Self, IndexError> {
        Ok(Self { core: Index::open(path.as_ref(), epochs, Bytes { key: key_size, value: value_size })? })
    }

    /// Remove every file of the index at `path`, typed or raw: its main
    /// file and each chunk file beside it. A missing file is counted, not
    /// an error; a refusal is counted and the first one is named. A handle
    /// still open keeps its mappings until it drops. The epoch table is a
    /// file of its own and is removed on its own.
    pub fn unlink(path: impl AsRef<Path>) -> crate::adaptive_ring::UnlinkReport {
        crate::shared_versioned_index::unlink_files(path.as_ref())
    }

    /// The epoch table the index stamps from.
    #[inline]
    pub fn epochs(&self) -> &Arc<SharedEpochs> {
        self.core.epochs()
    }

    /// Bytes a key takes.
    #[inline]
    pub fn key_size(&self) -> usize {
        self.core.shape().key
    }

    /// Bytes a value takes.
    #[inline]
    pub fn value_size(&self) -> usize {
        self.core.shape().value
    }

    fn key(&self, key: &[u8]) -> Result<*const u8, IndexError> {
        if key.len() == self.key_size() {
            Ok(key.as_ptr())
        } else {
            Err(IndexError::InvalidConfig("a key is not the size the index was made with"))
        }
    }

    fn value(&self, value: &[u8]) -> Result<*const u8, IndexError> {
        if value.len() == self.value_size() {
            Ok(value.as_ptr())
        } else {
            Err(IndexError::InvalidConfig("a value is not the size the index was made with"))
        }
    }

    fn bound(&self, bound: Bound<&[u8]>) -> Result<Bound<*const u8>, IndexError> {
        Ok(match bound {
            Bound::Unbounded => Bound::Unbounded,
            Bound::Included(k) => Bound::Included(self.key(k)?),
            Bound::Excluded(k) => Bound::Excluded(self.key(k)?),
        })
    }

    /// Make `value` current for `key` in the statement holding `ticket`,
    /// the slot and epoch [`SharedEpochs::claim_ticket`] answered, whose
    /// snapshot is the epoch `pin`; `None` deletes. A key whose newest
    /// version was born after the pin by another epoch refuses the write
    /// with [`IndexError::Conflict`], and the statement voids what it wrote
    /// and runs again. A second write by the same ticket replaces its own
    /// version. A ticket the epoch table does not hold at that slot is
    /// refused.
    pub fn write(&self, key: &[u8], value: Option<&[u8]>, ticket: (usize, Epoch), pin: Epoch) -> Result<(), IndexError> {
        let key = self.key(key)?;
        let value = value.map(|v| self.value(v)).transpose()?;
        let (slot, epoch) = ticket;
        let tickets = self.core.epochs().tickets();
        // A ticket slot holds its epoch biased by one, as `claim_ticket`
        // publishes it.
        if slot >= tickets.capacity() || epoch.checked_add(1).is_none_or(|held_as| tickets.payload(slot) != Some(held_as)) {
            return Err(IndexError::InvalidConfig("a ticket the epoch table does not hold"));
        }
        let held = self.core.epochs().pin()?;
        // SAFETY: the key and the value are the sizes the index was made
        // with.
        let written = unsafe { self.core.write_at(key, value, epoch, slot, pin) };
        drop(held);
        written
    }

    /// Take back what `epoch` wrote to `key`: its version goes and the one
    /// it superseded is current again. `false` when the key's newest
    /// version is not `epoch`'s.
    pub fn void(&self, key: &[u8], epoch: Epoch) -> Result<bool, IndexError> {
        let key = self.key(key)?;
        // SAFETY: the key is the size the index was made with.
        unsafe { self.core.void(key, epoch) }
    }

    /// Take back everything `epoch` wrote, key by key, from the write list
    /// of the ticket holding it: the epoch of a dead ticket, before
    /// [`SharedEpochs::free_dead_ticket`]. Returns how many keys it took a
    /// version back from.
    pub fn void_epoch(&self, epoch: Epoch) -> Result<usize, IndexError> {
        self.core.void_epoch(epoch)
    }

    /// The value of `key` a statement reading at the epoch `pin` sees, or
    /// one the epoch `own` of its own ticket wrote, copied into the front of
    /// `out`: `true` when there is one. `out` holds at least a value.
    pub fn get_at(&self, key: &[u8], pin: Epoch, own: Option<Epoch>, out: &mut [u8]) -> Result<bool, IndexError> {
        let key = self.key(key)?;
        let size = self.value_size();
        if out.len() < size {
            return Err(IndexError::InvalidConfig("the buffer is shorter than a value"));
        }
        let held = self.core.epochs().pin()?;
        // SAFETY: the key is the size the index was made with, and the value
        // it answers stays in place while `held` is.
        let found = unsafe {
            match self.core.get_at(key, pin, own)? {
                Some(v) => {
                    std::ptr::copy_nonoverlapping(v, out.as_mut_ptr(), size);
                    true
                }
                None => false,
            }
        };
        drop(held);
        Ok(found)
    }

    /// The entries a statement reading at the epoch `pin` sees in the
    /// range, those the epoch `own` of its own ticket wrote included, in key
    /// order. `limit` counts versions examined; when it stops the walk the
    /// last key examined is returned to resume from with
    /// [`Bound::Excluded`], and `None` means the range is exhausted. A key's
    /// versions are never split across two calls, every call examines at
    /// least one key, and bounds that cross read as no rows.
    pub fn range_at(
        &self,
        low: Bound<&[u8]>,
        high: Bound<&[u8]>,
        limit: usize,
        pin: Epoch,
        own: Option<Epoch>,
    ) -> Result<RawRangeRows, IndexError> {
        let (low, high) = (self.bound(low)?, self.bound(high)?);
        let (key_size, value_size) = (self.key_size(), self.value_size());
        let held = self.core.epochs().pin()?;
        let mut rows = Vec::new();
        // SAFETY: the bounds are keys of the size the index was made with,
        // and every key and value the walk visits, and the key it answers,
        // stay in place while `held` is.
        let cursor = unsafe {
            self.core
                .range_walk(low, high, limit, pin, own, |k, v| {
                    rows.push((std::slice::from_raw_parts(k, key_size).to_vec(), std::slice::from_raw_parts(v, value_size).to_vec()));
                })?
                .map(|k| std::slice::from_raw_parts(k, key_size).to_vec())
        };
        drop(held);
        Ok((rows, cursor))
    }

    /// Whether any key in the range gained a version after `since` by an
    /// epoch other than `own`, a deletion included: what a statement
    /// validates for each range it scanned before it publishes.
    pub fn changed_since(&self, low: Bound<&[u8]>, high: Bound<&[u8]>, since: Epoch, own: Option<Epoch>) -> Result<bool, IndexError> {
        let (low, high) = (self.bound(low)?, self.bound(high)?);
        let held = self.core.epochs().pin()?;
        // SAFETY: the bounds are keys of the size the index was made with.
        let changed = unsafe { self.core.changed_since(low, high, since, own) };
        drop(held);
        changed
    }

    /// Drop every version superseded at or below the epoch table's reclaim
    /// horizon, remove every key whose newest version is a deletion at or
    /// below it, and reuse what earlier sweeps retired once the horizon has
    /// passed it. Returns how many versions holding a value went.
    pub fn sweep(&self) -> Result<usize, IndexError> {
        self.core.sweep()
    }

    /// Versions held, current and superseded, deletions included.
    pub fn version_count(&self) -> Result<usize, IndexError> {
        self.core.version_count()
    }

    /// Keys whose newest version holds a value.
    pub fn live_count(&self) -> Result<usize, IndexError> {
        self.core.live_count()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicUsize, Ordering as AtomicOrdering};

    fn tmp(name: &str) -> (PathBuf, PathBuf) {
        let mut p = std::env::temp_dir();
        p.push(format!("subetha-rvindex-{name}-{}.bin", std::process::id()));
        let mut e = std::env::temp_dir();
        e.push(format!("subetha-rvindex-{name}-{}.epochs", std::process::id()));
        remove_family(&p);
        remove_family(&e);
        (p, e)
    }

    fn remove_family(main: &Path) {
        let dir = main.parent().expect("the path has a directory");
        let stem = main.file_name().expect("the path names a file").to_string_lossy().to_string();
        for entry in std::fs::read_dir(dir).expect("the temp directory lists") {
            let entry = entry.expect("the entry reads");
            let name = entry.file_name().to_string_lossy().to_string();
            if name == stem || name.starts_with(&format!("{stem}.")) {
                match std::fs::remove_file(entry.path()) {
                    Ok(()) => {}
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                    Err(e) => panic!("{} not removed: {e}", entry.path().display()),
                }
            }
        }
    }

    fn index(name: &str, key_size: usize, value_size: usize) -> (RawSharedVersionedIndex, PathBuf, PathBuf) {
        let (p, e) = tmp(name);
        let epochs = Arc::new(SharedEpochs::create(&e, 32).expect("the epoch table is made"));
        let idx = RawSharedVersionedIndex::create_with_chunks(&p, epochs, key_size, value_size, 6).expect("the index is made");
        (idx, p, e)
    }

    fn done(idx: RawSharedVersionedIndex, p: PathBuf, e: PathBuf) {
        drop(idx);
        remove_family(&p);
        remove_family(&e);
    }

    /// One statement: a pin and a ticket taken before it, then `f` with the
    /// ticket's slot and epoch and the pin's epoch, then publish.
    fn statement(idx: &RawSharedVersionedIndex, f: impl FnOnce((usize, Epoch), Epoch)) -> Epoch {
        let epochs = idx.epochs();
        let (pin_slot, pin) = epochs.claim_pin().expect("a pin");
        let ticket = epochs.claim_ticket().expect("a ticket");
        f(ticket, pin);
        epochs.publish_ticket(ticket.0);
        epochs.release_pin(pin_slot);
        ticket.1
    }

    fn get(idx: &RawSharedVersionedIndex, key: &[u8], pin: Epoch, own: Option<Epoch>) -> Option<Vec<u8>> {
        let mut out = vec![0u8; idx.value_size()];
        idx.get_at(key, pin, own, &mut out).expect("read").then_some(out)
    }

    /// An index grown past its first chunks leaves chunk files beside its
    /// main file; unlink takes them all and nothing else named like it.
    #[test]
    fn unlink_removes_the_main_file_and_every_chunk_file() {
        let (idx, p, e) = index("unlink", 4, 8);
        statement(&idx, |t, pin| {
            for k in 0..500u32 {
                idx.write(&k.to_be_bytes(), Some(&[7u8; 8]), t, pin).expect("write");
            }
        });
        let neighbour = p.with_file_name(format!("{}.keysx", p.file_name().expect("a name").to_string_lossy()));
        std::fs::write(&neighbour, b"not a chunk").expect("a neighbouring file");
        drop(idx);
        let report = RawSharedVersionedIndex::unlink(&p);
        assert_eq!(report.failed, 0, "{report:?}");
        assert!(report.removed > 1, "the main file and its chunk files: {report:?}");
        let stem = p.file_name().expect("a name").to_string_lossy().to_string();
        let left: Vec<String> = std::fs::read_dir(p.parent().expect("a directory"))
            .expect("the temp directory lists")
            .map(|entry| entry.expect("the entry reads").file_name().to_string_lossy().to_string())
            .filter(|name| name == &stem || name.starts_with(&format!("{stem}.")))
            .collect();
        assert_eq!(left, vec![neighbour.file_name().expect("a name").to_string_lossy().to_string()]);
        std::fs::remove_file(&neighbour).expect("the neighbouring file is removed");
        remove_family(&e);
    }

    /// Keys order as unsigned bytes, so big-endian integers range in numeric
    /// order, and a value of another width than the key round-trips.
    #[test]
    fn keys_order_as_unsigned_bytes_and_values_round_trip() {
        let (idx, p, e) = index("order", 4, 12);
        statement(&idx, |t, pin| {
            for k in [300u32, 2, 70_000, 1] {
                let value: Vec<u8> = (0..12).map(|i| (k as u8).wrapping_add(i)).collect();
                idx.write(&k.to_be_bytes(), Some(&value), t, pin).expect("write");
            }
        });
        let (_, pin) = idx.epochs().claim_pin().expect("a pin");
        let (rows, cursor) = idx.range_at(Bound::Unbounded, Bound::Unbounded, usize::MAX, pin, None).expect("range");
        assert_eq!(cursor, None);
        let keys: Vec<u32> = rows.iter().map(|(k, _)| u32::from_be_bytes(k[..].try_into().expect("four bytes"))).collect();
        assert_eq!(keys, vec![1, 2, 300, 70_000]);
        let value = get(&idx, &300u32.to_be_bytes(), pin, None).expect("the key is there");
        assert_eq!(value, (0..12).map(|i| 44u8.wrapping_add(i)).collect::<Vec<u8>>());
        done(idx, p, e);
    }

    /// The contract the typed index keeps, through the byte form: a pin
    /// keeps its version across an update, a statement reads its own write,
    /// a version born after the pin is a conflict naming its epoch, `void`
    /// takes a version back, and a resumed range covers every key.
    #[test]
    fn the_byte_form_keeps_the_index_contract() {
        let (idx, p, e) = index("contract", 8, 8);
        let k = |n: u64| n.to_be_bytes();
        statement(&idx, |t, pin| {
            for n in 0..10u64 {
                idx.write(&k(n), Some(&k(n)), t, pin).expect("write");
            }
        });
        let (stale_slot, stale) = idx.epochs().claim_pin().expect("a pin");
        let won = statement(&idx, |t, pin| idx.write(&k(1), Some(&k(11)), t, pin).expect("write"));
        assert_eq!(get(&idx, &k(1), stale, None), Some(k(1).to_vec()), "the pin keeps its version");
        let late = idx.epochs().claim_ticket().expect("a ticket");
        assert!(matches!(idx.write(&k(1), Some(&k(12)), late, stale), Err(IndexError::Conflict { epoch }) if epoch == won));
        idx.write(&k(2), Some(&k(22)), late, stale).expect("write");
        assert_eq!(get(&idx, &k(2), stale, Some(late.1)), Some(k(22).to_vec()), "its own write");
        assert_eq!(get(&idx, &k(2), stale, None), Some(k(2).to_vec()), "not another reader's");
        assert!(idx.void(&k(2), late.1).expect("void"));
        assert_eq!(get(&idx, &k(2), stale, Some(late.1)), Some(k(2).to_vec()), "voided");
        idx.epochs().publish_ticket(late.0);
        let mut seen = Vec::new();
        let mut low: Option<Vec<u8>> = None;
        loop {
            let lb = match &low {
                Some(c) => Bound::Excluded(&c[..]),
                None => Bound::Unbounded,
            };
            let (rows, cursor) = idx.range_at(lb, Bound::Unbounded, 3, stale, None).expect("range");
            seen.extend(rows.into_iter().map(|(key, _)| u64::from_be_bytes(key[..].try_into().expect("eight bytes"))));
            match cursor {
                Some(c) => low = Some(c),
                None => break,
            }
        }
        assert_eq!(seen, (0..10u64).collect::<Vec<_>>());
        idx.epochs().release_pin(stale_slot);
        done(idx, p, e);
    }

    /// A key or value of another size, a ticket the table does not hold and
    /// a short buffer are each refused without touching the index.
    #[test]
    fn wrong_sizes_and_tickets_are_refused() {
        let (idx, p, e) = index("refused", 8, 4);
        let (_, pin) = idx.epochs().claim_pin().expect("a pin");
        let ticket = idx.epochs().claim_ticket().expect("a ticket");
        assert!(matches!(idx.write(&[0u8; 7], Some(&[0u8; 4]), ticket, pin), Err(IndexError::InvalidConfig(_))));
        assert!(matches!(idx.write(&[0u8; 8], Some(&[0u8; 5]), ticket, pin), Err(IndexError::InvalidConfig(_))));
        assert!(matches!(idx.write(&[0u8; 8], Some(&[0u8; 4]), (ticket.0, ticket.1 + 1), pin), Err(IndexError::InvalidConfig(_))));
        assert!(matches!(idx.write(&[0u8; 8], Some(&[0u8; 4]), (99, ticket.1), pin), Err(IndexError::InvalidConfig(_))));
        assert!(matches!(idx.get_at(&[0u8; 8], pin, None, &mut [0u8; 3]), Err(IndexError::InvalidConfig(_))));
        idx.epochs().publish_ticket(ticket.0);
        assert!(matches!(idx.write(&[0u8; 8], Some(&[0u8; 4]), ticket, pin), Err(IndexError::InvalidConfig(_))), "a published ticket");
        assert_eq!(idx.version_count().expect("count"), 0);
        done(idx, p, e);
    }

    /// A typed file and a raw one refuse each other, since they order the
    /// same keys differently.
    #[test]
    fn a_raw_index_does_not_open_a_typed_one() {
        let (p, e) = tmp("magic");
        let typed = crate::shared_versioned_index::SharedVersionedIndex::<u64, u64>::create_with_chunks(&p, &e, 32, 6)
            .expect("the typed index is made");
        drop(typed);
        let epochs = Arc::new(SharedEpochs::open(&e, 32).expect("the epoch table opens"));
        assert!(matches!(RawSharedVersionedIndex::open(&p, epochs, 8, 8), Err(IndexError::LayoutMismatch(_))));
        remove_family(&p);
        remove_family(&e);
    }

    /// Eight threads racing on one live key through the byte form admit
    /// exactly one.
    #[test]
    fn racing_writers_through_the_byte_form_admit_exactly_one() {
        let (idx, p, e) = index("race", 8, 8);
        statement(&idx, |t, pin| idx.write(&1u64.to_be_bytes(), Some(&0u64.to_be_bytes()), t, pin).expect("write"));
        let (_, pin) = idx.epochs().claim_pin().expect("a pin");
        let won = AtomicUsize::new(0);
        std::thread::scope(|s| {
            for t in 0..8u64 {
                let (idx, won) = (&idx, &won);
                s.spawn(move || {
                    let ticket = idx.epochs().claim_ticket().expect("a ticket");
                    match idx.write(&1u64.to_be_bytes(), Some(&t.to_be_bytes()), ticket, pin) {
                        Ok(()) => {
                            won.fetch_add(1, AtomicOrdering::Relaxed);
                        }
                        Err(IndexError::Conflict { .. }) => {}
                        Err(other) => panic!("thread {t}: {other}"),
                    }
                    idx.epochs().publish_ticket(ticket.0);
                });
            }
        });
        assert_eq!(won.load(AtomicOrdering::Relaxed), 1);
        done(idx, p, e);
    }

    /// An open ticket's epoch is taken back whole by `void_epoch`.
    #[test]
    fn void_epoch_takes_back_an_open_tickets_writes() {
        let (idx, p, e) = index("voidepoch", 8, 8);
        statement(&idx, |t, pin| idx.write(&1u64.to_be_bytes(), Some(&10u64.to_be_bytes()), t, pin).expect("write"));
        let (pin_slot, pin) = idx.epochs().claim_pin().expect("a pin");
        let ticket = idx.epochs().claim_ticket().expect("a ticket");
        for n in 0..20u64 {
            idx.write(&n.to_be_bytes(), Some(&(n + 1000).to_be_bytes()), ticket, pin).expect("write");
        }
        assert_eq!(idx.void_epoch(ticket.1).expect("void"), 20);
        idx.epochs().publish_ticket(ticket.0);
        idx.epochs().release_pin(pin_slot);
        let (_, now) = idx.epochs().claim_pin().expect("a pin");
        assert_eq!(get(&idx, &1u64.to_be_bytes(), now, None), Some(10u64.to_be_bytes().to_vec()));
        assert_eq!(get(&idx, &2u64.to_be_bytes(), now, None), None);
        assert_eq!(idx.live_count().expect("count"), 1);
        done(idx, p, e);
    }
}
