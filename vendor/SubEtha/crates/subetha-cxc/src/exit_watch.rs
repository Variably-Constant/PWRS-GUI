//! Tell a flag when another process exits, from the operating system's own
//! exit event, so nothing polls for it.
//!
//! - Windows: a wait on the process handle in the system thread pool, which
//!   runs the callback once the process is gone; no thread of SubEtha's.
//! - Linux and Android: a pidfd for the process in one epoll set, which one
//!   thread per process waits on, started by the first watch.
//! - FreeBSD, macOS and the other BSDs: an `EVFILT_PROC` `NOTE_EXIT` filter
//!   in one kqueue, which one thread per process waits on, started by the
//!   first watch.
//!
//! [`watch_holder`](crate::exit_watch::watch_holder) watches a holder word's
//! process this way once per process, however many structures name it, and
//! counts the exits it is told of in [`exits`](crate::exit_watch::exits): a
//! reader that finds the count moved looks for the
//! holders that are gone. Elsewhere, or where the operating system refuses
//! the watch, it answers false and the caller relies on its own fallback.

use std::ptr;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicPtr, AtomicU64, Ordering};

use crate::atomic_pair::AtomicPair;
use crate::process_identity;

/// What a watch tells once its process exits.
pub(crate) trait Exited: Send + Sync {
    /// The watched process has exited.
    fn exited(&self);
}

impl Exited for AtomicBool {
    fn exited(&self) {
        self.store(true, Ordering::Release);
    }
}

/// Exits of watched holders this process has been told of.
static EXITS: AtomicU64 = AtomicU64::new(0);

/// How many exits of holders [`watch_holder`] watched this process has been
/// told of.
#[inline(always)]
pub(crate) fn exits() -> u64 {
    EXITS.load(Ordering::Acquire)
}

/// Slots in one chunk of [`HOLDERS`].
const HOLDERS_PER_CHUNK: usize = 64;

/// The holder words this process watches, one exit watch each. A slot pairs
/// a holder word in its low half with whether its exit was told in its high
/// half: zero while free, the holder and 0 while its watch waits, the holder
/// and 1 once its exit was told. A told slot is taken again only once its
/// holder reads gone, so no holder is watched twice. Chunks are added as
/// every slot fills and are never freed.
struct Holders {
    slots: [AtomicPair; HOLDERS_PER_CHUNK],
    next: AtomicPtr<Holders>,
}

static HOLDERS: Holders = Holders::new();

impl Holders {
    const fn new() -> Self {
        Self { slots: [const { AtomicPair::new(0) }; HOLDERS_PER_CHUNK], next: AtomicPtr::new(ptr::null_mut()) }
    }

    /// Every chunk, the first one first.
    fn chunks() -> impl Iterator<Item = &'static Holders> {
        std::iter::successors(Some(&HOLDERS), |chunk| {
            let next = chunk.next.load(Ordering::Acquire);
            // SAFETY: a chunk is leaked once linked and never freed.
            (!next.is_null()).then(|| unsafe { &*next })
        })
    }

    /// Every slot, in chunk order.
    fn slots() -> impl Iterator<Item = &'static AtomicPair> {
        Self::chunks().flat_map(|chunk| chunk.slots.iter())
    }

    /// A slot set to `holder`, its exit not told: a free one, else a told
    /// one whose holder is gone, else one in a chunk added for it.
    fn take(holder: u64) -> &'static AtomicPair {
        loop {
            for slot in Self::slots() {
                if slot.load_lo() == 0 && slot.compare_exchange(0, u128::from(holder)).is_ok() {
                    return slot;
                }
            }
            for slot in Self::slots() {
                let held = slot.load();
                if held >> 64 != 0 && process_identity::holder_gone(held as u64) && slot.compare_exchange(held, u128::from(holder)).is_ok() {
                    return slot;
                }
            }
            let last = Self::chunks().last().unwrap_or(&HOLDERS);
            let fresh = Box::into_raw(Box::new(Holders::new()));
            if last.next.compare_exchange(ptr::null_mut(), fresh, Ordering::AcqRel, Ordering::Acquire).is_err() {
                // SAFETY: the chunk made above, which no list holds.
                drop(unsafe { Box::from_raw(fresh) });
            }
        }
    }
}

/// The watch of one holder: its slot marked told and [`EXITS`] stepped once
/// its process exits.
struct HolderExit {
    slot: &'static AtomicPair,
}

impl Exited for HolderExit {
    fn exited(&self) {
        self.slot.store_hi(1);
        EXITS.fetch_add(1, Ordering::AcqRel);
    }
}

/// What this process knows of the exit of the process `holder` names: None
/// when it does not watch it, false while its watch waits, true once its
/// exit was told.
pub(crate) fn told(holder: u64) -> Option<bool> {
    Holders::slots().find_map(|slot| {
        let held = slot.load();
        (held as u64 == holder).then_some(held >> 64 != 0)
    })
}

/// Watch the process `holder` names, its pid and start stamp, for its exit,
/// which steps [`exits`]: one watch per holder however many callers ask, and
/// none again once its exit was told, save that two threads asking at once
/// can each set one. The watch follows the pid, so a caller that finds the
/// holder gone once this returns has seen an exit no count will report.
/// False when this platform or this process's rights give no exit event for
/// it.
pub(crate) fn watch_holder(holder: u64) -> bool {
    if told(holder).is_some() {
        return true;
    }
    let slot = Holders::take(holder);
    if Holders::slots().take_while(|s| !ptr::eq(*s, slot)).any(|s| s.load_lo() == holder) {
        slot.store(0);
        return true;
    }
    if imp::watch(process_identity::holder_pid(holder), Arc::new(HolderExit { slot })) {
        return true;
    }
    slot.store(0);
    false
}

#[cfg(windows)]
mod imp {
    use super::*;
    use std::ffi::c_void;
    use std::sync::atomic::AtomicIsize;
    use windows_sys::Win32::Foundation::{CloseHandle, GetLastError, ERROR_INVALID_PARAMETER, HANDLE};
    use windows_sys::Win32::System::Threading::{
        OpenProcess, RegisterWaitForSingleObject, UnregisterWait, INFINITE, PROCESS_SYNCHRONIZE, WT_EXECUTEONLYONCE,
    };

    /// One watch: what to tell, the process handle, and the wait the
    /// registration returned, zero until it has returned.
    struct Watch {
        notice: Arc<dyn Exited>,
        process: HANDLE,
        wait: AtomicIsize,
    }

    unsafe extern "system" fn exited(context: *mut c_void, _timed_out: bool) {
        // SAFETY: the context is the watch `watch` leaked, handed to this
        // callback alone, which runs once.
        let watch = unsafe { Box::from_raw(context as *mut Watch) };
        watch.notice.exited();
        // The registration may not have returned its wait yet.
        let mut wait = watch.wait.load(Ordering::Acquire);
        while wait == 0 {
            std::hint::spin_loop();
            wait = watch.wait.load(Ordering::Acquire);
        }
        // SAFETY: the wait and the handle are this watch's; a callback may
        // cancel its own wait without blocking.
        unsafe {
            UnregisterWait(wait as HANDLE);
            CloseHandle(watch.process);
        }
    }

    pub(super) fn watch(pid: u32, notice: Arc<dyn Exited>) -> bool {
        // SAFETY: plain kernel32 calls; the handle goes to the watch.
        let process = unsafe { OpenProcess(PROCESS_SYNCHRONIZE, 0, pid) };
        if process.is_null() {
            // No such process: it has exited.
            // SAFETY: read right after the failed call.
            if unsafe { GetLastError() } == ERROR_INVALID_PARAMETER {
                notice.exited();
                return true;
            }
            return false;
        }
        let watch = Box::into_raw(Box::new(Watch { notice, process, wait: AtomicIsize::new(0) }));
        let mut wait: HANDLE = std::ptr::null_mut();
        // SAFETY: the callback owns the leaked watch once it runs, and runs
        // once; the wait handle reaches it through the watch.
        let registered =
            unsafe { RegisterWaitForSingleObject(&mut wait, process, Some(exited), watch as *const c_void, INFINITE, WT_EXECUTEONLYONCE) };
        if registered == 0 {
            // SAFETY: the registration failed, so no callback holds the watch.
            let watch = unsafe { Box::from_raw(watch) };
            // SAFETY: the handle opened above.
            unsafe { CloseHandle(watch.process) };
            return false;
        }
        // SAFETY: the watch lives until its callback frees it, which waits
        // for this store.
        unsafe { (*watch).wait.store(wait as isize, Ordering::Release) };
        true
    }
}

#[cfg(any(target_os = "linux", target_os = "android"))]
mod imp {
    use super::*;
    use std::sync::OnceLock;

    /// One watch: what to tell and the pidfd that reports the exit.
    struct Watch {
        notice: Arc<dyn Exited>,
        fd: libc::c_int,
    }

    /// The epoll set every watch of this process joins, and whether its
    /// waiting thread started; -1 when none could be made.
    fn epoll() -> libc::c_int {
        static EPOLL: OnceLock<libc::c_int> = OnceLock::new();
        *EPOLL.get_or_init(|| {
            // SAFETY: a plain syscall.
            let fd = unsafe { libc::epoll_create1(libc::EPOLL_CLOEXEC) };
            if fd < 0 {
                return -1;
            }
            let started = std::thread::Builder::new().name("subetha-exit-watch".into()).spawn(move || wait_forever(fd));
            if started.is_err() {
                // SAFETY: the set made above, which nothing else holds.
                unsafe { libc::close(fd) };
                return -1;
            }
            fd
        })
    }

    fn wait_forever(epoll: libc::c_int) {
        let mut events = [libc::epoll_event { events: 0, u64: 0 }; 16];
        loop {
            // SAFETY: the buffer holds `events.len()` events.
            let n = unsafe { libc::epoll_wait(epoll, events.as_mut_ptr(), events.len() as libc::c_int, -1) };
            for event in events.iter().take(n.max(0) as usize) {
                // SAFETY: each event carries the watch `watch` leaked into the
                // set, which leaves the set below and is freed once.
                let watch = unsafe { Box::from_raw(event.u64 as *mut Watch) };
                watch.notice.exited();
                // SAFETY: the pidfd is this watch's; closing it leaves the set.
                unsafe { libc::close(watch.fd) };
            }
        }
    }

    pub(super) fn watch(pid: u32, notice: Arc<dyn Exited>) -> bool {
        // No process has a pid past pid_t's range, so it reads as gone, as a
        // pid the kernel finds no process for does below.
        if pid > libc::pid_t::MAX as u32 {
            notice.exited();
            return true;
        }
        // SAFETY: a plain syscall.
        let fd = unsafe { libc::syscall(libc::SYS_pidfd_open, pid as libc::pid_t, 0) } as libc::c_int;
        if fd < 0 {
            if std::io::Error::last_os_error().raw_os_error() == Some(libc::ESRCH) {
                notice.exited();
                return true;
            }
            return false;
        }
        let set = epoll();
        if set < 0 {
            // SAFETY: the pidfd opened above.
            unsafe { libc::close(fd) };
            return false;
        }
        let watch = Box::into_raw(Box::new(Watch { notice, fd }));
        let mut event = libc::epoll_event { events: libc::EPOLLIN as u32 | libc::EPOLLONESHOT as u32, u64: watch as u64 };
        // SAFETY: the set and the pidfd are open; the waiting thread owns the
        // leaked watch once the event fires.
        if unsafe { libc::epoll_ctl(set, libc::EPOLL_CTL_ADD, fd, &mut event) } != 0 {
            // SAFETY: the add failed, so no event holds the watch.
            let watch = unsafe { Box::from_raw(watch) };
            // SAFETY: the pidfd opened above.
            unsafe { libc::close(watch.fd) };
            return false;
        }
        true
    }
}

#[cfg(any(target_os = "freebsd", target_os = "macos", target_os = "ios", target_os = "netbsd", target_os = "openbsd", target_os = "dragonfly"))]
mod imp {
    use super::*;
    use std::sync::OnceLock;

    /// The kqueue every watch of this process joins; -1 when none could be
    /// made.
    fn kqueue() -> libc::c_int {
        static KQ: OnceLock<libc::c_int> = OnceLock::new();
        *KQ.get_or_init(|| {
            // SAFETY: a plain syscall.
            let kq = unsafe { libc::kqueue() };
            if kq < 0 {
                return -1;
            }
            let started = std::thread::Builder::new().name("subetha-exit-watch".into()).spawn(move || wait_forever(kq));
            if started.is_err() {
                // SAFETY: the queue made above, which nothing else holds.
                unsafe { libc::close(kq) };
                return -1;
            }
            kq
        })
    }

    fn wait_forever(kq: libc::c_int) {
        // SAFETY: an all-zero kevent is a valid value to overwrite.
        let mut events: [libc::kevent; 16] = unsafe { std::mem::zeroed() };
        loop {
            // SAFETY: the buffer holds `events.len()` events; no timeout.
            let n = unsafe { libc::kevent(kq, std::ptr::null(), 0, events.as_mut_ptr(), events.len() as libc::c_int, std::ptr::null()) };
            for event in events.iter().take(n.max(0) as usize) {
                // SAFETY: each event carries the boxed notice `watch` leaked
                // into the queue, which a one-shot filter reports once.
                let notice = unsafe { Box::from_raw(event.udata as *mut Arc<dyn Exited>) };
                notice.exited();
            }
        }
    }

    pub(super) fn watch(pid: u32, notice: Arc<dyn Exited>) -> bool {
        let kq = kqueue();
        if kq < 0 {
            return false;
        }
        let udata = Box::into_raw(Box::new(notice));
        // SAFETY: an all-zero kevent is a valid value to fill in.
        let mut change: libc::kevent = unsafe { std::mem::zeroed() };
        change.ident = pid as _;
        change.filter = libc::EVFILT_PROC;
        change.flags = libc::EV_ADD | libc::EV_ONESHOT;
        change.fflags = libc::NOTE_EXIT;
        change.udata = udata as _;
        // SAFETY: one change, no events asked back.
        let rc = unsafe { libc::kevent(kq, &change, 1, std::ptr::null_mut(), 0, std::ptr::null()) };
        if rc != 0 {
            let gone = std::io::Error::last_os_error().raw_os_error() == Some(libc::ESRCH);
            // SAFETY: the filter was not added, so the queue holds no reference.
            let notice = unsafe { Box::from_raw(udata) };
            if gone {
                notice.exited();
                return true;
            }
            return false;
        }
        true
    }
}

#[cfg(not(any(
    windows,
    target_os = "linux",
    target_os = "android",
    target_os = "freebsd",
    target_os = "macos",
    target_os = "ios",
    target_os = "netbsd",
    target_os = "openbsd",
    target_os = "dragonfly"
)))]
mod imp {
    use super::*;

    pub(super) fn watch(_pid: u32, _notice: Arc<dyn Exited>) -> bool {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    /// A watched child leaves the flag clear while it lives and sets it once
    /// it is killed.
    #[test]
    fn a_killed_child_sets_its_flag() {
        let exe = std::env::current_exe().unwrap();
        // The child is this test binary running only the sleeper below.
        let mut child = std::process::Command::new(exe)
            .args(["--exact", "exit_watch::tests::sleeper_never_runs_on_its_own", "--ignored", "--nocapture"])
            .env("SUBETHA_EXIT_WATCH_SLEEP", "1")
            .spawn()
            .unwrap();
        let flag = Arc::new(AtomicBool::new(false));
        assert!(imp::watch(child.id(), Arc::clone(&flag) as Arc<dyn Exited>), "this platform gives an exit event");
        std::thread::sleep(Duration::from_millis(100));
        assert!(!flag.load(Ordering::Acquire), "a live child leaves the flag clear");
        child.kill().unwrap();
        child.wait().unwrap();
        let deadline = Instant::now() + Duration::from_secs(10);
        while !flag.load(Ordering::Acquire) {
            assert!(Instant::now() < deadline, "the exit set the flag");
            std::thread::sleep(Duration::from_millis(1));
        }
    }

    /// A holder asked for twice is watched once, and its exit frees its slot
    /// and steps the count of exits.
    #[test]
    fn a_holder_is_watched_once_and_its_exit_counted() {
        let exe = std::env::current_exe().unwrap();
        let mut child = std::process::Command::new(exe)
            .args(["--exact", "exit_watch::tests::sleeper_never_runs_on_its_own", "--ignored", "--nocapture"])
            .env("SUBETHA_EXIT_WATCH_SLEEP", "1")
            .spawn()
            .unwrap();
        let holder = process_identity::holder_for(child.id());
        let before = exits();
        assert!(watch_holder(holder), "this platform gives an exit event");
        assert!(watch_holder(holder));
        let slots = || Holders::slots().filter(|slot| slot.load_lo() == holder).count();
        assert_eq!(slots(), 1, "one slot for the holder");
        assert_eq!(told(holder), Some(false), "a live holder's watch waits");
        child.kill().unwrap();
        child.wait().unwrap();
        let deadline = Instant::now() + Duration::from_secs(10);
        while told(holder) != Some(true) || exits() == before {
            assert!(Instant::now() < deadline, "the exit was told and stepped the count");
            std::thread::sleep(Duration::from_millis(1));
        }
        assert!(watch_holder(holder));
        assert_eq!(slots(), 1, "a holder whose exit was told is not watched again");
    }

    /// The child a test kills: it sleeps when asked to.
    #[test]
    #[ignore = "run only as a child process that a test kills"]
    fn sleeper_never_runs_on_its_own() {
        if std::env::var_os("SUBETHA_EXIT_WATCH_SLEEP").is_some() {
            std::thread::sleep(Duration::from_secs(600));
        }
    }
}
