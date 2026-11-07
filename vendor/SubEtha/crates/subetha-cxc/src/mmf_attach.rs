//! Race-free construction for the file-backed MMF primitives.
//!
//! [`create_or_attach`] attaches to the region published at a path. A
//! caller that finds none builds its own under a staging name and
//! publishes it by linking that name to the real one, so the region's
//! name only ever appears over complete bytes. Callers that arrive
//! together each build; the first link wins, and every other caller
//! discards its staging region and attaches to the published one. No
//! caller waits on another. [`reset`] truncates and reinitializes.
//!
//! The region's own name is never reclaimed, since a removal takes the
//! name and not the file, leaving a peer mapping an orphan.

use std::fs::File;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use memmap2::{MmapMut, MmapOptions};

/// The one wait left in construction: how long a link that publishes a
/// region is retried while a removal of its name is in flight, before the
/// refusal is reported. Bounded so a denial that stands surfaces as an
/// error rather than an unbounded spin.
pub(crate) const INIT_WAIT: Duration = Duration::from_secs(5);

/// Map the region at `path`: the one published there, or one this caller
/// builds and publishes when none is.
///
/// `total` is the region size. `init` lays out a zeroed mapping of this
/// caller's own staging region, at most once per call, and does not run
/// when a published region is found first. Callers that arrive together
/// may each run it; the first to publish wins and the others discard what
/// they built. `magic` reads a mapping's magic number: zero for a region
/// not laid out, `wanted` once the region is this layout, and any other
/// value for a region of another layout, such as one an earlier SubEtha
/// built, which is refused as a [`Mismatch`](crate::layout::Mismatch) of
/// its magic.
///
/// Errors if the region is smaller than `total`, holds another layout's
/// magic, or if `path` is held by something this module did not publish.
pub(crate) fn create_or_attach<I, M>(
    path: &Path,
    total: usize,
    init: I,
    magic: M,
    wanted: u64,
) -> io::Result<(File, MmapMut)>
where
    I: FnOnce(*mut u8),
    M: Fn(*const u8) -> u64,
{
    let ready = Magic { read: magic, wanted };
    if let Some(pair) = try_attach(path, total, &ready)? {
        return Ok(pair);
    }
    match publish(path, total, init) {
        Ok(pair) => Ok(pair),
        // A peer published first, or the name holds a region built in
        // place under its own name.
        Err(e) if e.kind() == io::ErrorKind::AlreadyExists => match try_attach(path, total, &ready)? {
            Some(pair) => Ok(pair),
            None => Err(unreadable(path)),
        },
        Err(e) => Err(e),
    }
}

/// The error for a region name held by something this module did not
/// publish: `InvalidData`, naming the file and the fix. Removing it by
/// hand is the whole of the recovery, since those bytes may belong to a
/// region another build is filling in right now.
fn unreadable(path: &Path) -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidData,
        format!(
            "the region at {} is not readable: it is empty or unfinished, which a creator that \
             died while building it in place leaves behind. Remove the file to let it be built \
             again.",
            path.display()
        ),
    )
}

/// A region's magic reader and the magic this layout writes.
struct Magic<M> {
    read: M,
    wanted: u64,
}

impl<M: Fn(*const u8) -> u64> Magic<M> {
    /// Whether the mapping at `ptr` is this layout's region, built: false
    /// while its magic is still zero, and another layout's magic refused.
    fn ready(&self, ptr: *const u8) -> io::Result<bool> {
        match (self.read)(ptr) {
            0 => Ok(false),
            found if found == self.wanted => Ok(true),
            found => Err(another_layout(found, self.wanted)),
        }
    }
}

/// The refusal of a region whose magic is `found` where this layout
/// writes `wanted`: `InvalidData` carrying the magic's mismatch.
fn another_layout(found: u64, wanted: u64) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, crate::layout::Mismatch::new("magic", found, wanted))
}

/// Whether a refused create is one that raced the removal of the name it
/// asked for. NTFS answers such a create with access denied, in 58 to 66
/// of 1,600 contended rounds measured on Windows 11, although the name is
/// gone by the time the removal returns. A denial that stands is told from
/// one in flight only by how long it lasts.
fn removal_in_flight(e: &io::Error) -> bool {
    e.kind() == io::ErrorKind::PermissionDenied
}

/// Build the region under a staging name and publish it as `path`.
fn publish<I>(path: &Path, total: usize, init: I) -> io::Result<(File, MmapMut)>
where
    I: FnOnce(*mut u8),
{
    publish_with(path, crate::region_file::create_new, |file| build(file, total, init))
}

/// Create a file under a staging name with `create`, lay it out with
/// `fill`, and publish it as `path` by linking the staging name there.
/// `create` is an exclusive create: it fails with `AlreadyExists` at a
/// name something already holds.
///
/// The link fails with `AlreadyExists` rather than replacing, so whoever
/// links first wins and no one overwrites a file a peer is using; a
/// refused link discards the staging file and reports `AlreadyExists`.
/// Both names address one file afterward, so dropping the staging name
/// leaves the file, and whatever `fill` made of it, in place.
pub(crate) fn publish_with<T>(
    path: &Path,
    create: impl Fn(&Path) -> io::Result<File>,
    fill: impl FnOnce(&File) -> io::Result<T>,
) -> io::Result<(File, T)> {
    let (staging, file) = create_staging(path, &create)?;
    let filled = match fill(&file) {
        Ok(t) => t,
        Err(e) => return Err(discard_staging(&staging, e)),
    };
    #[cfg(test)]
    crate::test_races::pause_point();
    if let Err(e) = link(&staging, path) {
        return Err(discard_staging(&staging, e));
    }
    crate::region_file::remove(&staging)?;
    Ok((file, filled))
}

/// Link the finished region at `staging` into place as `path`. A link
/// refused the way a create racing the removal of the same name is refused
/// is tried again, since the name is free once the removal lands; one
/// refused for `INIT_WAIT` without a break is reported as the denial it is.
fn link(staging: &Path, path: &Path) -> io::Result<()> {
    let mut refused_since = None;
    loop {
        match link_once(staging, path) {
            Ok(()) => return Ok(()),
            Err(e) if removal_in_flight(&e) => {
                let since = *refused_since.get_or_insert_with(Instant::now);
                if since.elapsed() >= INIT_WAIT {
                    return Err(e);
                }
                std::thread::yield_now();
            }
            Err(e) => return Err(e),
        }
    }
}

/// One link attempt. A test arms it to be refused the way a create racing
/// the removal of the same name is refused.
fn link_once(staging: &Path, path: &Path) -> io::Result<()> {
    #[cfg(test)]
    if let Some(refusal) = crate::test_races::take_create_refusal() {
        return Err(refusal);
    }
    std::fs::hard_link(staging, path)
}

/// How many staging names to try before giving up on finding a free one.
const STAGING_ATTEMPTS: u32 = 64;

/// Take a staging file under a name nothing else holds, made by the
/// exclusive create `create`. A taken name is one an earlier run left
/// behind, so it is retried rather than reported.
fn create_staging(
    path: &Path,
    create: &impl Fn(&Path) -> io::Result<File>,
) -> io::Result<(PathBuf, File)> {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    for _ in 0..STAGING_ATTEMPTS {
        let staging = sibling(
            path,
            &format!(".staging.{}.{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)),
        )?;
        match create(&staging) {
            Ok(file) => return Ok((staging, file)),
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e),
        }
    }
    Err(io::Error::other(format!(
        "no free staging name for {} in {STAGING_ATTEMPTS} tries",
        path.display()
    )))
}

/// A name beside `path` carrying `suffix`. The same directory, because
/// publishing links the finished region into place and a link cannot
/// cross a file system.
fn sibling(path: &Path, suffix: &str) -> io::Result<PathBuf> {
    let Some(stem) = path.file_name() else {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("{} does not name a region file", path.display()),
        ));
    };
    let mut name = stem.to_os_string();
    name.push(suffix);
    Ok(path.with_file_name(name))
}

/// Drop a staging region after a failure, reporting `cause`. One that
/// cannot be removed is named in the error too, since it sits in the
/// region's own directory.
fn discard_staging(staging: &Path, cause: io::Error) -> io::Error {
    match crate::region_file::remove(staging) {
        Ok(()) => cause,
        Err(removal) => io::Error::new(
            cause.kind(),
            format!(
                "{cause}; the staging region at {} was also left behind: {removal}",
                staging.display()
            ),
        ),
    }
}

/// Size, map and initialize a region: zeroed first, because `init`
/// writes only the fields it names and a reader reads all of them.
fn build<I>(file: &File, total: usize, init: I) -> io::Result<MmapMut>
where
    I: FnOnce(*mut u8),
{
    file.set_len(total as u64)?;
    let mut mmap = unsafe { MmapOptions::new().len(total).map_mut(file)? };
    unsafe {
        std::ptr::write_bytes(mmap.as_mut_ptr(), 0, total);
        init(mmap.as_mut_ptr());
    }
    Ok(mmap)
}

/// Convert an attach error for a caller with a layout-mismatch error of
/// its own: a refusal carrying a [`Mismatch`](crate::layout::Mismatch)
/// becomes `mismatch` with it, anything else converts as the I/O error it
/// is.
pub(crate) fn attach_error<E: From<io::Error>>(
    e: io::Error,
    mismatch: impl FnOnce(crate::layout::Mismatch) -> E,
) -> E {
    match crate::layout::Mismatch::carried_by(&e) {
        Some(found) => mismatch(found),
        None => E::from(e),
    }
}

/// The refusal of a region `held` bytes long where `wanted` were asked
/// for: `InvalidData` carrying the length mismatch.
pub(crate) fn too_short(held: u64, wanted: usize) -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidData,
        crate::layout::Mismatch::new("length", held, wanted as u64),
    )
}

/// One attach attempt. `Ok(None)` when no region is published at `path`:
/// the name is free, or holds a file that is empty or whose magic is zero,
/// which only a build in place under the region's own name leaves, since
/// a published name is linked over complete bytes.
///
/// A file shorter than `total` but not empty is a region that exists at
/// another size, refused as a length mismatch.
fn try_attach<M>(path: &Path, total: usize, ready: &Magic<M>) -> io::Result<Option<(File, MmapMut)>>
where
    M: Fn(*const u8) -> u64,
{
    let file = match crate::region_file::open_existing(path) {
        Ok(f) => f,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e),
    };
    let on_disk = file.metadata()?.len();
    if on_disk == 0 {
        return Ok(None);
    }
    if (on_disk as usize) < total {
        return Err(too_short(on_disk, total));
    }
    let mmap = unsafe { MmapOptions::new().len(total).map_mut(&file)? };
    if !ready.ready(mmap.as_ptr())? {
        return Ok(None);
    }
    Ok(Some((file, mmap)))
}

/// Truncate and reinitialize the region at `path`, discarding any state a live
/// peer holds. For a caller that knows it owns the path.
///
/// On Windows this errors while any process still maps the region
/// (ERROR_USER_MAPPED_FILE): the OS refuses to truncate a mapped file, so a
/// reset succeeds only once every handle is gone.
pub(crate) fn reset<I>(path: &Path, total: usize, init: I) -> io::Result<(File, MmapMut)>
where
    I: FnOnce(*mut u8),
{
    let file = crate::region_file::create_truncated(path)?;
    file.set_len(total as u64)?;
    let mut mmap = unsafe { MmapOptions::new().len(total).map_mut(&file)? };
    unsafe {
        std::ptr::write_bytes(mmap.as_mut_ptr(), 0, total);
        init(mmap.as_mut_ptr());
    }
    Ok((file, mmap))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::sync::{Arc, Barrier};

    const MAGIC: u64 = 0x4D4D_4641_5454_4143;

    /// A fresh path for one test: whatever an earlier run left there is
    /// removed, and a removal refused for any reason but absence fails the
    /// test rather than gating it on a stale region.
    fn tmp(name: &str) -> std::path::PathBuf {
        let mut p = std::env::temp_dir();
        p.push(format!("subetha-mmf-attach-{name}-{}.bin", std::process::id()));
        match std::fs::remove_file(&p) {
            Ok(()) => {}
            Err(e) if e.kind() == io::ErrorKind::NotFound => {}
            Err(e) => panic!("stale file {} not removed: {e}", p.display()),
        }
        p
    }

    unsafe fn write_magic(ptr: *mut u8) {
        unsafe { std::ptr::write_unaligned(ptr as *mut u64, MAGIC) };
    }

    fn magic_of(ptr: *const u8) -> u64 {
        unsafe { std::ptr::read_unaligned(ptr as *const u64) }
    }

    fn has_magic(ptr: *const u8) -> bool {
        magic_of(ptr) == MAGIC
    }

    /// Racing callers all attach to one region: each may build its own, the
    /// first link publishes, and every caller maps the published region and
    /// none of the discarded ones.
    #[test]
    fn racing_callers_all_attach_to_the_one_region_published() {
        let p = tmp("race");
        let inits = Arc::new(AtomicU32::new(0));
        let gate = Arc::new(Barrier::new(8));
        let mut hs = Vec::new();
        for caller in 1..=8u64 {
            let (p, inits, gate) = (p.clone(), Arc::clone(&inits), Arc::clone(&gate));
            hs.push(std::thread::spawn(move || {
                gate.wait();
                let (_f, m) = create_or_attach(
                    &p,
                    64,
                    |ptr| {
                        inits.fetch_add(1, Ordering::Relaxed);
                        unsafe {
                            std::ptr::write_unaligned(ptr.add(8) as *mut u64, caller);
                            write_magic(ptr);
                        }
                    },
                    magic_of,
                    MAGIC,
                )
                .expect("attach");
                assert!(has_magic(m.as_ptr()), "attached to an uninitialized region");
                unsafe { std::ptr::read_unaligned(m.as_ptr().add(8) as *const u64) }
            }));
        }
        let builders: Vec<u64> = hs.into_iter().map(|h| h.join().unwrap()).collect();
        assert!((1..=8).contains(&builders[0]), "the published region names no caller: {builders:?}");
        assert!(builders.iter().all(|&b| b == builders[0]), "callers mapped different regions: {builders:?}");
        let built = inits.load(Ordering::Relaxed);
        assert!((1..=8).contains(&built), "{built} builds for 8 callers");
        let left = strays(&p);
        assert!(left.is_empty(), "left beside the region: {left:?}");
        std::fs::remove_file(&p).expect("every thread's mapping is dropped and the file removable");
    }

    /// A second call attaches to the live region rather than zeroing it.
    #[test]
    fn attach_does_not_clear_existing_state() {
        let p = tmp("attach");

        let (_f, mut first) =
            create_or_attach(&p, 64, |ptr| unsafe { write_magic(ptr) }, magic_of, MAGIC).unwrap();
        // State a live peer owns, past the magic.
        unsafe { std::ptr::write_unaligned(first.as_mut_ptr().add(8) as *mut u64, 0xDEAD_BEEF) };

        let (_f2, second) =
            create_or_attach(&p, 64, |_| panic!("must not re-initialize"), magic_of, MAGIC).unwrap();
        let seen = unsafe { std::ptr::read_unaligned(second.as_ptr().add(8) as *const u64) };
        assert_eq!(seen, 0xDEAD_BEEF, "attaching cleared state the first caller owned");
        drop(second);
        drop(first);
        drop(_f2);
        drop(_f);
        std::fs::remove_file(&p).expect("the region file is unmapped and removable");
    }

    /// Attachers that reach one published region together all attach, and
    /// none of them builds a region of its own.
    #[test]
    fn attachers_reaching_one_region_together_all_attach() {
        let p = tmp("together");
        let (file, mapping) =
            create_or_attach(&p, 64, |ptr| unsafe { write_magic(ptr) }, magic_of, MAGIC).unwrap();
        let path = Arc::new(p.clone());
        let refused = Arc::new(AtomicU32::new(0));
        let mut hs = Vec::new();
        for _ in 0..8 {
            let (path, refused) = (Arc::clone(&path), Arc::clone(&refused));
            hs.push(std::thread::spawn(move || {
                for _ in 0..200 {
                    match create_or_attach(&path, 64, |_| panic!("the region exists"), magic_of, MAGIC) {
                        Ok((_f, m)) => assert!(has_magic(m.as_ptr()), "attached to an uninitialized region"),
                        Err(e) => {
                            refused.fetch_add(1, Ordering::Relaxed);
                            eprintln!("attach refused: {e}");
                        }
                    }
                }
            }));
        }
        for h in hs {
            h.join().unwrap();
        }
        assert_eq!(refused.load(Ordering::Relaxed), 0, "attaches refused, of 1600");
        let left = strays(&p);
        assert!(left.is_empty(), "left beside the region: {left:?}");
        drop(mapping);
        drop(file);
        std::fs::remove_file(&p).expect("every thread's mapping is dropped and the file removable");
    }

    /// An attacher of a published region builds nothing: a refusal armed
    /// against its next link is still armed once it has attached.
    #[test]
    fn an_attacher_of_a_published_region_builds_nothing() {
        let p = tmp("untouched");
        let (file, mapping) =
            create_or_attach(&p, 64, |ptr| unsafe { write_magic(ptr) }, magic_of, MAGIC).unwrap();
        crate::test_races::refuse_next_create();
        let (f, m) = create_or_attach(&p, 64, |_| panic!("the region exists"), magic_of, MAGIC)
            .expect("a published region is attached as it stands");
        assert!(has_magic(m.as_ptr()), "attached to an uninitialized region");
        assert!(
            crate::test_races::take_create_refusal().is_some(),
            "a link was tried on the way to a published region"
        );
        let left = strays(&p);
        assert!(left.is_empty(), "left beside the region: {left:?}");
        drop(m);
        drop(f);
        drop(mapping);
        drop(file);
        std::fs::remove_file(&p).expect("every mapping is dropped and the file removable");
    }

    /// A caller stopped with its region built and not yet linked keeps no
    /// one waiting: a peer that arrives meanwhile builds and publishes its
    /// own at once. Released, the stopped caller finds the name taken,
    /// discards what it built and attaches to the peer's region.
    #[test]
    fn a_caller_stopped_before_its_link_keeps_no_one_waiting_and_adopts_the_published_region() {
        let p = tmp("stopped_builder");
        let path = p.clone();
        let (pause, caller) = crate::test_races::stopped(move || {
            create_or_attach(
                &path,
                64,
                |ptr| unsafe {
                    std::ptr::write_unaligned(ptr.add(8) as *mut u64, 1);
                    write_magic(ptr);
                },
                magic_of,
                MAGIC,
            )
        });
        let started = Instant::now();
        let (file, mapping) = create_or_attach(
            &p,
            64,
            |ptr| unsafe {
                std::ptr::write_unaligned(ptr.add(8) as *mut u64, 2);
                write_magic(ptr);
            },
            magic_of,
            MAGIC,
        )
        .expect("the peer publishes while the first caller is stopped");
        assert!(
            started.elapsed() < INIT_WAIT / 2,
            "the peer waited {:?} on a caller stopped mid-build",
            started.elapsed()
        );
        pause.release();
        let attached = caller.join().expect("the stopped caller completes");
        let (f, m) = attached.expect("the stopped caller attaches to the published region");
        let builder = unsafe { std::ptr::read_unaligned(m.as_ptr().add(8) as *const u64) };
        assert_eq!(builder, 2, "the stopped caller mapped its own region, not the published one");
        let left = strays(&p);
        assert!(left.is_empty(), "left beside the region: {left:?}");
        drop(m);
        drop(f);
        drop(mapping);
        drop(file);
        std::fs::remove_file(&p).expect("every mapping is dropped and the file removable");
    }

    /// A region that exists at a smaller size is refused at once, as a size
    /// mismatch, rather than waited on as a creator still initializing.
    #[test]
    fn a_smaller_published_region_is_a_size_mismatch_not_a_wait() {
        let p = tmp("smaller");
        let (file, mapping) =
            create_or_attach(&p, 64, |ptr| unsafe { write_magic(ptr) }, magic_of, MAGIC).unwrap();
        let started = Instant::now();
        let err = create_or_attach(&p, 128, |_| panic!("must not re-initialize"), magic_of, MAGIC)
            .expect_err("a smaller region must not attach at a larger size");
        assert_eq!(
            crate::layout::Mismatch::carried_by(&err),
            Some(crate::layout::Mismatch::new("length", 64, 128)),
            "not a size mismatch: {err}"
        );
        assert!(
            started.elapsed() < INIT_WAIT / 2,
            "refused only after waiting {:?} for a creator that finished long ago",
            started.elapsed()
        );
        assert_eq!(err.to_string(), "the file is 64 bytes where 128 were asked for");
        drop(mapping);
        drop(file);
        std::fs::remove_file(&p).expect("the region file is unmapped and removable");
    }

    /// A region another layout built, such as one an earlier SubEtha left
    /// whose magic has since changed, is refused at once as a mismatch of
    /// its magic, rather than waited on as a creator still initializing.
    #[test]
    fn a_region_of_another_layout_is_a_magic_mismatch_not_a_wait() {
        let p = tmp("another-layout");
        let earlier = MAGIC ^ 0xFF;
        let mut bytes = vec![0u8; 64];
        bytes[..8].copy_from_slice(&earlier.to_ne_bytes());
        std::fs::write(&p, &bytes).expect("a region of another layout at the path");
        let started = Instant::now();
        let err = create_or_attach(&p, 64, |_| panic!("must not rebuild another layout's region"), magic_of, MAGIC)
            .expect_err("another layout's region must not attach");
        assert_eq!(
            crate::layout::Mismatch::carried_by(&err),
            Some(crate::layout::Mismatch::new("magic", earlier, MAGIC)),
            "not a magic mismatch: {err}"
        );
        assert!(
            started.elapsed() < INIT_WAIT / 2,
            "refused only after waiting {:?} for a creator that finished long ago",
            started.elapsed()
        );
        std::fs::remove_file(&p).expect("the region file is removable");
    }

    /// A region name held by something this module did not publish is
    /// reported, naming the file and the fix. Nothing recovers it: those
    /// bytes may belong to a region another build is filling in.
    #[test]
    fn an_empty_file_under_the_region_name_is_named_in_the_error() {
        let p = tmp("empty");
        File::create(&p).expect("an empty file at the region path");
        let started = Instant::now();
        let err = create_or_attach(&p, 64, |ptr| unsafe { write_magic(ptr) }, magic_of, MAGIC)
            .expect_err("an empty file has no region to attach to");
        assert!(
            started.elapsed() < INIT_WAIT / 2,
            "refused only after {:?}, as if waiting on a builder",
            started.elapsed()
        );
        assert_eq!(err.kind(), io::ErrorKind::InvalidData, "{err}");
        assert_eq!(crate::layout::Mismatch::carried_by(&err), None);
        let text = err.to_string();
        assert!(text.contains(&p.display().to_string()), "the error names the file: {text}");
        assert!(text.contains("Remove the file"), "the error carries the fix: {text}");
        std::fs::remove_file(&p).expect("the empty file is removable");
    }

    /// A builder that died with its region built and not linked keeps no
    /// one waiting: what it left is a staging file under its own name, and
    /// the next caller publishes a region at once.
    #[test]
    fn a_builder_that_died_before_its_link_keeps_no_one_waiting() {
        let p = tmp("dead_builder");
        let left_behind = sibling(&p, ".staging.4294967295.0").expect("the region path names a file");
        std::fs::write(&left_behind, [0u8; 64]).expect("the dead builder's staging file");
        assert!(!p.exists(), "it died before publishing anything");

        let started = Instant::now();
        let (f, m) = create_or_attach(&p, 64, |ptr| unsafe { write_magic(ptr) }, magic_of, MAGIC)
            .expect("the region is published beside the dead builder's staging file");
        assert!(has_magic(m.as_ptr()), "and it is initialized");
        assert!(
            started.elapsed() < INIT_WAIT / 2,
            "published only after {:?}, as if waiting on the dead builder",
            started.elapsed()
        );

        drop(m);
        drop(f);
        std::fs::remove_file(&left_behind).expect("the dead builder's staging file is removable");
        std::fs::remove_file(&p).expect("the region file is unmapped and removable");
    }

    /// Publishing leaves nothing beside the region: no staging file, which
    /// would be a full-size region nobody reclaims.
    #[test]
    fn publishing_leaves_no_staging_file() {
        let p = tmp("no_litter");
        let (f, m) =
            create_or_attach(&p, 64, |ptr| unsafe { write_magic(ptr) }, magic_of, MAGIC).unwrap();
        assert!(p.exists(), "the region is published under its own name");
        let left = strays(&p);
        assert!(left.is_empty(), "left beside the region: {left:?}");

        drop(m);
        drop(f);
        std::fs::remove_file(&p).expect("the region file is unmapped and removable");
    }

    /// The names in the region's directory that extend its own: a staging
    /// file left beside it.
    fn strays(p: &std::path::Path) -> Vec<String> {
        let dir = p.parent().expect("the region has a directory");
        let stem = p.file_name().expect("the region has a name").to_string_lossy().to_string();
        let mut strays = Vec::new();
        for entry in std::fs::read_dir(dir).expect("the directory lists") {
            let name = entry.expect("the entry reads").file_name().to_string_lossy().to_string();
            if name.starts_with(&stem) && name != stem {
                strays.push(name);
            }
        }
        strays
    }

    /// A publish lands only at a path nobody holds: at a taken one it is
    /// refused as `AlreadyExists`, the region there keeps its state, and the
    /// refused publish leaves no staging file beside it.
    #[test]
    fn a_publish_refuses_a_taken_path_and_leaves_its_region() {
        let p = tmp("fresh_taken");
        let (f, mut m) =
            publish(&p, 64, |ptr| unsafe { write_magic(ptr) }).expect("the path is free");
        unsafe { std::ptr::write_unaligned(m.as_mut_ptr().add(8) as *mut u64, 0xDEAD_BEEF) };

        let err = publish(&p, 64, |ptr| unsafe { write_magic(ptr) })
            .expect_err("a taken path is not laid out again");
        assert_eq!(err.kind(), io::ErrorKind::AlreadyExists, "{err}");
        let seen = unsafe { std::ptr::read_unaligned(m.as_ptr().add(8) as *const u64) };
        assert_eq!(seen, 0xDEAD_BEEF, "the refused call wrote into the region at the taken path");
        let left = strays(&p);
        assert!(left.is_empty(), "left beside the region: {left:?}");

        drop(m);
        drop(f);
        std::fs::remove_file(&p).expect("the region file is unmapped and removable");
    }

    /// A link refused the way a removal in flight refuses it is tried again
    /// rather than reported, and the region is published.
    #[test]
    fn a_link_refused_by_a_removal_in_flight_is_retried_and_the_region_published() {
        let p = tmp("fresh_refused");
        crate::test_races::refuse_next_create();
        let (f, m) = publish(&p, 64, |ptr| unsafe { write_magic(ptr) })
            .expect("the refused link is tried again and the region published");
        assert!(has_magic(m.as_ptr()), "and it is initialized");
        assert!(crate::test_races::take_create_refusal().is_none(), "the armed refusal was met");
        let left = strays(&p);
        assert!(left.is_empty(), "left beside the region: {left:?}");

        drop(m);
        drop(f);
        std::fs::remove_file(&p).expect("the region file is unmapped and removable");
    }


    /// reset deliberately discards it, which is the case truncation was for.
    #[test]
    fn reset_clears_the_region() {
        let p = tmp("reset");

        let (_f, mut m) =
            create_or_attach(&p, 64, |ptr| unsafe { write_magic(ptr) }, magic_of, MAGIC).unwrap();
        unsafe { std::ptr::write_unaligned(m.as_mut_ptr().add(8) as *mut u64, 0xDEAD_BEEF) };
        drop(m);

        let (_f2, fresh) = reset(&p, 64, |ptr| unsafe { write_magic(ptr) }).unwrap();
        let seen = unsafe { std::ptr::read_unaligned(fresh.as_ptr().add(8) as *const u64) };
        assert_eq!(seen, 0, "reset left prior state behind");
        drop(fresh);
        drop(_f2);
        drop(_f);
        std::fs::remove_file(&p).expect("the region file is unmapped and removable");
    }
}
