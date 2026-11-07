//! `RawCell`: the one-value cell of
//! [`SharedCell`](crate::shared_cell::SharedCell) at a value size fixed at
//! run time instead of by a type parameter, for a caller reaching the cell
//! from another language.
//!
//! It reads and writes the same file format through the same protocol, and
//! records the caller's size in the header where the typed cell records its
//! type's, so the two open each other's regions when the sizes agree and
//! refuse each other when they do not. A `RawCell` of four bytes and a
//! `SharedCell<u32>` are the same region.
//!
//! Any number of writers and readers, none waiting on another: a write
//! fills a buffer no reader is copying and publishes it with one
//! compare-and-swap of the cell's head, and a read retries only when a
//! publish landed while it copied. The cell's module documentation has the
//! whole protocol.

use std::fs::File;
use std::path::Path;

use memmap2::{MmapMut, MmapOptions};

use crate::shared_cell::{CellCore, CellHeader, SharedCellError, CELL_FILE_SIZE, CELL_MAGIC, PAYLOAD_BYTES};

pub struct RawCell {
    core: CellCore,
    _file: File,
    mmap: MmapMut,
    value_size: usize,
}

unsafe impl Send for RawCell {}
unsafe impl Sync for RawCell {}

impl std::fmt::Debug for RawCell {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RawCell").field("value_size", &self.value_size).finish()
    }
}

impl RawCell {
    /// A value size the region can hold: at least one byte, and no more
    /// than one cell carries.
    fn check_size(value_size: usize) -> Result<(), SharedCellError> {
        if value_size == 0 || value_size > PAYLOAD_BYTES {
            return Err(SharedCellError::PayloadTooLarge);
        }
        Ok(())
    }

    /// Obtain the cell at `path` for a `value_size`-byte value,
    /// initializing an empty one if the path does not yet exist and
    /// attaching to it if it does. Attaching leaves the value and
    /// generation in place; a region built for another size is a
    /// `LayoutMismatch`.
    pub fn create(path: impl AsRef<Path>, value_size: usize) -> Result<Self, SharedCellError> {
        Self::check_size(value_size)?;
        let (file, mmap) = crate::mmf_attach::create_or_attach(
            path.as_ref(),
            CELL_FILE_SIZE,
            |ptr| unsafe { crate::shared_cell::init_region(ptr, value_size) },
            |ptr| unsafe { u64::from((*(ptr as *const CellHeader)).magic) },
            u64::from(CELL_MAGIC),
        )
        .map_err(|e| crate::mmf_attach::attach_error(e, SharedCellError::LayoutMismatch))?;
        Self::from_region(file, mmap, value_size)
    }

    /// Truncate the cell at `path` and initialize an empty one,
    /// discarding whatever value a live peer holds.
    pub fn reset(path: impl AsRef<Path>, value_size: usize) -> Result<Self, SharedCellError> {
        Self::check_size(value_size)?;
        let (file, mmap) = crate::mmf_attach::reset(path.as_ref(), CELL_FILE_SIZE, |ptr| unsafe {
            crate::shared_cell::init_region(ptr, value_size)
        })?;
        Self::from_region(file, mmap, value_size)
    }

    /// Attach to the cell at `path`; the file must exist.
    pub fn open(path: impl AsRef<Path>, value_size: usize) -> Result<Self, SharedCellError> {
        Self::check_size(value_size)?;
        let file = crate::region_file::open_existing(path.as_ref())?;
        crate::shared_cell::long_enough(&file)?;
        let mmap = unsafe { MmapOptions::new().len(CELL_FILE_SIZE).map_mut(&file)? };
        Self::from_region(file, mmap, value_size)
    }

    /// Wrap an initialized region, refusing one built for another size.
    fn from_region(file: File, mut mmap: MmapMut, value_size: usize) -> Result<Self, SharedCellError> {
        let header = unsafe { &*(mmap.as_ptr() as *const CellHeader) };
        crate::shared_cell::header_matches(header, value_size)?;
        // SAFETY: the mapping is a laid-out cell of CELL_FILE_SIZE bytes for
        // values of `value_size` bytes, held as long as the core.
        let core = unsafe { CellCore::new(mmap.as_mut_ptr(), value_size) };
        Ok(Self { core, _file: file, mmap, value_size })
    }

    /// Bytes the value takes.
    pub fn value_size(&self) -> usize {
        self.value_size
    }

    /// The generation: how many writes have landed.
    pub fn version(&self) -> u64 {
        self.core.generation()
    }

    /// Make `value`, exactly the value size, current after any write that
    /// lands first, and answer its generation.
    pub fn set(&self, value: &[u8]) -> Result<u64, SharedCellError> {
        if value.len() != self.value_size {
            return Err(SharedCellError::PayloadTooLarge);
        }
        // SAFETY: `value` holds the value size, the core's value size.
        Ok(unsafe { self.core.set(value.as_ptr()) })
    }

    /// Make `value`, exactly the value size, current if the generation is
    /// still `seen`: the new generation, or the generation a write that
    /// landed first left, in which case `value` was never visible.
    pub fn compare_and_set(&self, seen: u64, value: &[u8]) -> Result<Result<u64, u64>, SharedCellError> {
        if value.len() != self.value_size {
            return Err(SharedCellError::PayloadTooLarge);
        }
        // SAFETY: `value` holds the value size, the core's value size.
        Ok(unsafe { self.core.compare_and_set(seen, value.as_ptr()) })
    }

    /// Copy the value into `out`, at least the value size, and answer the
    /// generation that made it current.
    pub fn load(&self, out: &mut [u8]) -> Result<u64, SharedCellError> {
        if out.len() < self.value_size {
            return Err(SharedCellError::PayloadTooLarge);
        }
        // SAFETY: `out` holds at least the value size.
        Ok(unsafe { self.core.load(out.as_mut_ptr()) }.0)
    }

    /// Copy the value into `out`, at least the value size.
    pub fn get(&self, out: &mut [u8]) -> Result<(), SharedCellError> {
        self.load(out).map(|_| ())
    }

    pub fn flush(&self) -> Result<(), SharedCellError> {
        self.mmap.flush()?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shared_cell::SharedCell;

    /// A file for one test, removed when the test ends; declared before the
    /// primitive that maps it so it drops after it.
    fn tmp(name: &str) -> crate::test_paths::TmpFile {
        crate::test_paths::TmpFile::new(format!("subetha-raw-cell-{name}-{}.bin", std::process::id()))
    }

    #[test]
    fn a_value_round_trips_and_the_generation_counts_the_writes() {
        let path = tmp("round-trip");
        let cell = RawCell::create(&path, 8).unwrap();
        assert_eq!(cell.value_size(), 8);
        assert_eq!(cell.version(), 0);
        let mut out = [0u8; 8];
        cell.get(&mut out).unwrap();
        assert_eq!(out, [0u8; 8], "a fresh cell reads as zeros");
        assert_eq!(cell.set(&[7u8; 8]).unwrap(), 1);
        assert_eq!(cell.version(), 1, "one write, counted once");
        cell.get(&mut out).unwrap();
        assert_eq!(out, [7u8; 8]);
        cell.set(&[9u8; 8]).unwrap();
        assert_eq!(cell.load(&mut out).unwrap(), 2);
        assert_eq!(out, [9u8; 8]);
        assert!(matches!(cell.set(&[0u8; 7]).unwrap_err(), SharedCellError::PayloadTooLarge));
        assert!(matches!(cell.get(&mut [0u8; 7]).unwrap_err(), SharedCellError::PayloadTooLarge));
    }

    #[test]
    fn a_typed_cell_and_a_raw_one_of_the_same_size_share_a_region() {
        let path = tmp("interop");
        let typed: SharedCell<[u8; 4]> = SharedCell::create(&path).unwrap();
        typed.set(*b"abcd");
        let raw = RawCell::open(&path, 4).unwrap();
        let mut out = [0u8; 4];
        raw.get(&mut out).unwrap();
        assert_eq!(&out, b"abcd");
        raw.set(b"wxyz").unwrap();
        assert_eq!(typed.get(), *b"wxyz");
        assert_eq!(typed.version(), raw.version());
        // A size the region was not built for is refused, in both
        // directions.
        let four_where_eight = crate::layout::Mismatch::new("value size", 4, 8);
        assert!(matches!(
            RawCell::open(&path, 8).unwrap_err(),
            SharedCellError::LayoutMismatch(m) if m == four_where_eight,
        ));
        match SharedCell::<[u8; 8]>::open(&path) {
            Ok(_) => panic!("a typed cell of another size must be refused this region"),
            Err(e) => assert!(
                matches!(e, SharedCellError::LayoutMismatch(m) if m == four_where_eight),
                "{e:?}",
            ),
        }
    }

    /// A compare-and-set lands on the generation it saw and loses to one a
    /// write made since, through either handle.
    #[test]
    fn compare_and_set_lands_only_on_the_generation_it_saw() {
        let path = tmp("cas");
        let a = RawCell::create(&path, 4).unwrap();
        let b = RawCell::open(&path, 4).unwrap();
        let mut out = [0u8; 4];
        let seen = a.load(&mut out).unwrap();
        match b.compare_and_set(seen, b"won!").unwrap() {
            Ok(generation) => assert_eq!(generation, seen + 1),
            Err(now) => panic!("a write on the current generation lost to {now}"),
        }
        match a.compare_and_set(seen, b"lost").unwrap() {
            Ok(generation) => panic!("a write on a stale generation landed as {generation}"),
            Err(now) => assert_eq!(now, seen + 1),
        }
        assert_eq!(a.load(&mut out).unwrap(), seen + 1);
        assert_eq!(&out, b"won!");
    }

    #[test]
    fn a_shorter_value_leaves_no_tail_of_a_longer_one() {
        let path = tmp("tail");
        let cell = RawCell::create(&path, 52).unwrap();
        cell.set(&[0xAA; 52]).unwrap();
        let mut out = [0u8; 52];
        cell.get(&mut out).unwrap();
        assert_eq!(out, [0xAA; 52]);
        let mut short = [0u8; 52];
        cell.set(&{
            let mut v = [0u8; 52];
            v[..4].copy_from_slice(b"abcd");
            v
        })
        .unwrap();
        cell.get(&mut short).unwrap();
        assert_eq!(&short[..4], b"abcd");
        assert_eq!(&short[4..], &[0u8; 48], "the tail is zero, not the old value");
    }

    #[test]
    fn a_size_no_cell_can_hold_is_refused_before_any_file_is_touched() {
        let path = tmp("too-big");
        assert!(matches!(RawCell::create(&path, 0).unwrap_err(), SharedCellError::PayloadTooLarge));
        assert!(matches!(RawCell::create(&path, PAYLOAD_BYTES + 1).unwrap_err(), SharedCellError::PayloadTooLarge));
        assert!(!path.exists());
    }
}
