//! Who a process is beyond its pid: the pid and when the process started.
//! A pid names a process only until it exits and the system hands the
//! number to another, so a holder recorded by pid alone cannot be told from
//! a newcomer that drew the same number. A holder recorded with its start
//! is gone when its pid names no process, or names one that started at
//! another time.
//!
//! A start is kept as a 32-bit stamp folded from the start time the system
//! reports: the creation time `GetProcessTimes` returns on Windows, the
//! `starttime` field of `/proc/<pid>/stat` on Linux, the `ki_start` a
//! `KERN_PROC_PID` sysctl reports less the boot time `kern.boottime`
//! reports on FreeBSD, and the start time
//! `proc_pidinfo` returns for `PROC_PIDTBSDINFO` on macOS. A stamp is never
//! 0: a recorded stamp of 0 means the start is unknown, and that holder is
//! judged by its pid alone, as is a process whose start this process may
//! not read.
//!
//! A holder word packs the two: the pid in the low 32 bits and the stamp in
//! the high 32.

use std::sync::atomic::{AtomicU64, Ordering};

/// What the system says of a pid.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Probe {
    /// No process has the pid.
    Gone,
    /// A process has the pid, started at this stamp.
    Started(u32),
    /// A process has the pid, and its start cannot be read.
    Unreadable,
}

/// What the system says of a pid's start before it is folded to a stamp,
/// in the units [`start_of`] reads it in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Start {
    /// No process has the pid.
    Gone,
    /// A process has the pid, started at this reading.
    At(u64),
    /// A process has the pid, and its start cannot be read.
    Unreadable,
}

/// The holder word for `pid` started at `stamp`.
#[inline(always)]
pub(crate) const fn holder(pid: u32, stamp: u32) -> u64 {
    ((stamp as u64) << 32) | pid as u64
}

/// The pid a holder word records.
#[inline(always)]
pub(crate) const fn holder_pid(word: u64) -> u32 {
    word as u32
}

/// The start stamp a holder word records, 0 when unknown.
#[inline(always)]
pub(crate) const fn holder_stamp(word: u64) -> u32 {
    (word >> 32) as u32
}

/// Whether the holder `pid`, started at `stamp`, is gone: the pid names no
/// process, or names one whose start reads as another stamp. A stamp of 0,
/// or a start this process may not read, leaves the pid alone to decide.
pub(crate) fn gone(pid: u32, stamp: u32) -> bool {
    match probe(pid) {
        Probe::Gone => true,
        Probe::Started(now) => stamp != 0 && now != stamp,
        Probe::Unreadable => false,
    }
}

/// What the system says of `pid`, its start folded to a stamp.
pub(crate) fn probe(pid: u32) -> Probe {
    match start_of(pid) {
        Start::Gone => Probe::Gone,
        Start::At(start) => Probe::Started(fold(start)),
        Start::Unreadable => Probe::Unreadable,
    }
}

/// This process's holder word once [`current`] has read it, zero until
/// then. It is cached only once the pid is, so the fork handler that clears
/// the cached pid is in place to clear it too.
static CURRENT: AtomicU64 = AtomicU64::new(0);

/// This process's holder word if [`current`] has cached it, else 0: one
/// load, for a path that stamps every operation and sends a 0 to
/// [`current`]. A forked child reads 0 until its first [`current`].
#[inline(always)]
pub(crate) fn cached() -> u64 {
    CURRENT.load(Ordering::Relaxed)
}

/// Forget the cached word: the fork handler's call in a forked child.
#[cfg(unix)]
#[inline(always)]
pub(crate) fn clear_cached() {
    CURRENT.store(0, Ordering::Relaxed);
}

/// This process's holder word: its pid, and its start stamp or 0 when it
/// cannot read its own start. Read from the system once per process: a
/// forked child, whose pid is not the one cached, reads its own.
#[inline]
pub(crate) fn current() -> u64 {
    let pid = crate::peer_directory::current_pid();
    let cached = CURRENT.load(Ordering::Relaxed);
    if cached != 0 && holder_pid(cached) == pid {
        return cached;
    }
    current_uncached(pid)
}

/// The holder word for `pid`: this process's own, read once, or another
/// process's, its start read from the system and 0 when it cannot be read.
pub(crate) fn holder_for(pid: u32) -> u64 {
    if pid == crate::peer_directory::current_pid() {
        return current();
    }
    match probe(pid) {
        Probe::Started(stamp) => holder(pid, stamp),
        Probe::Gone | Probe::Unreadable => holder(pid, 0),
    }
}

/// Whether the holder a holder word records is gone, as [`gone`].
#[inline]
pub(crate) fn holder_gone(word: u64) -> bool {
    gone(holder_pid(word), holder_stamp(word))
}

/// [`current`] when nothing is cached for `pid`.
#[cold]
#[inline(never)]
fn current_uncached(pid: u32) -> u64 {
    let stamp = match probe(pid) {
        Probe::Started(stamp) => stamp,
        Probe::Gone | Probe::Unreadable => 0,
    };
    let word = holder(pid, stamp);
    if crate::peer_directory::cached_pid() == pid {
        CURRENT.store(word, Ordering::Relaxed);
    }
    word
}

/// A start time folded to the stamp a holder records, which is never 0.
#[inline]
fn fold(start: u64) -> u32 {
    ((start ^ (start >> 32)) as u32).max(1)
}

/// Whether a process has `pid`: one this process may not signal still
/// counts, and a value that is no positive pid names none.
#[cfg(unix)]
fn exists(pid: u32) -> bool {
    let pid = match libc::pid_t::try_from(pid) {
        Ok(p) if p > 0 => p,
        Ok(_not_positive) => return false,
        Err(_past_pid_t) => return false,
    };
    // SAFETY: signal 0 checks for the process and delivers nothing.
    let rc = unsafe { libc::kill(pid, 0) };
    rc == 0 || std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
}

/// When `pid` started: its creation time, in 100 ns units of system time.
#[cfg(windows)]
pub(crate) fn start_of(pid: u32) -> Start {
    use windows_sys::Win32::Foundation::{CloseHandle, ERROR_ACCESS_DENIED, FILETIME, STILL_ACTIVE};
    use windows_sys::Win32::System::Threading::{
        GetExitCodeProcess, GetProcessTimes, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION,
    };
    if pid == 0 {
        return Start::Gone;
    }
    // SAFETY: the handle is checked before use and closed once read.
    unsafe {
        let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if handle.is_null() {
            // Access denied still proves the process exists; a missing pid
            // fails with ERROR_INVALID_PARAMETER.
            let denied = std::io::Error::last_os_error().raw_os_error() == Some(ERROR_ACCESS_DENIED as i32);
            return if denied { Start::Unreadable } else { Start::Gone };
        }
        let mut code: u32 = 0;
        let running = GetExitCodeProcess(handle, &mut code) != 0 && code == STILL_ACTIVE as u32;
        let zero = FILETIME { dwLowDateTime: 0, dwHighDateTime: 0 };
        let (mut created, mut exited, mut kernel, mut user) = (zero, zero, zero, zero);
        let timed = GetProcessTimes(handle, &mut created, &mut exited, &mut kernel, &mut user) != 0;
        CloseHandle(handle);
        if !running {
            return Start::Gone;
        }
        if !timed {
            return Start::Unreadable;
        }
        Start::At((u64::from(created.dwHighDateTime) << 32) | u64::from(created.dwLowDateTime))
    }
}

/// When `pid` started: the `starttime` field of its `/proc/<pid>/stat`,
/// clock ticks since boot.
#[cfg(target_os = "linux")]
pub(crate) fn start_of(pid: u32) -> Start {
    if !exists(pid) {
        return Start::Gone;
    }
    let stat = match std::fs::read_to_string(format!("/proc/{pid}/stat")) {
        Ok(stat) => stat,
        // Hidden by the mount (hidepid), or exited since the check: the pid
        // alone decides.
        Err(_unreadable) => return Start::Unreadable,
    };
    match linux_start(&stat) {
        Some(start) => Start::At(start),
        None => Start::Unreadable,
    }
}

/// The `starttime` field, the 22nd, of a `/proc/<pid>/stat` line. The
/// command name, the 2nd, sits in parentheses and may hold spaces and
/// parentheses itself, so the fields are counted from the last `)`: the
/// 3rd is the first after it and the 22nd the twentieth.
#[cfg(any(target_os = "linux", test))]
fn linux_start(stat: &str) -> Option<u64> {
    let close = stat.rfind(')')?;
    let field = stat.get(close + 1..)?.split_ascii_whitespace().nth(19)?;
    field.bytes().try_fold(0u64, |start, digit| {
        if digit.is_ascii_digit() { start.checked_mul(10)?.checked_add(u64::from(digit - b'0')) } else { None }
    })
}

/// When `pid` started: the `ki_start` a `KERN_PROC_PID` sysctl returns less
/// the boot time `kern.boottime` returns, microseconds since boot. The
/// system reports `ki_start` as the boot time plus the uptime at the fork,
/// and moves the boot time when the wall clock is stepped, so the
/// difference is the reading a step leaves alone.
#[cfg(target_os = "freebsd")]
pub(crate) fn start_of(pid: u32) -> Start {
    let id = match libc::pid_t::try_from(pid) {
        Ok(p) if p > 0 => p,
        Ok(_not_positive) => return Start::Gone,
        Err(_past_pid_t) => return Start::Gone,
    };
    let mut mib = [libc::CTL_KERN, libc::KERN_PROC, libc::KERN_PROC_PID, id];
    let mut info = std::mem::MaybeUninit::<libc::kinfo_proc>::zeroed();
    let mut len = std::mem::size_of::<libc::kinfo_proc>();
    // SAFETY: the name holds four entries and the output buffer `len`
    // bytes, the size of one kinfo_proc.
    let rc = unsafe {
        libc::sysctl(mib.as_mut_ptr(), mib.len() as libc::c_uint, info.as_mut_ptr().cast(), &mut len, std::ptr::null(), 0)
    };
    if rc != 0 || len < std::mem::size_of::<libc::kinfo_proc>() {
        return if exists(pid) { Start::Unreadable } else { Start::Gone };
    }
    // SAFETY: the sysctl filled one whole kinfo_proc.
    let info = unsafe { info.assume_init() };
    let start = (info.ki_start.tv_sec as u64).wrapping_mul(1_000_000).wrapping_add(info.ki_start.tv_usec as u64);
    match freebsd_boot_time() {
        Some(boot) => Start::At(start.wrapping_sub(boot)),
        None => Start::Unreadable,
    }
}

/// The boot time `kern.boottime` returns, in microseconds of wall time.
#[cfg(target_os = "freebsd")]
fn freebsd_boot_time() -> Option<u64> {
    let mut mib = [libc::CTL_KERN, libc::KERN_BOOTTIME];
    let mut boot = libc::timeval { tv_sec: 0, tv_usec: 0 };
    let mut len = std::mem::size_of::<libc::timeval>();
    // SAFETY: the name holds two entries and the output buffer `len` bytes,
    // the size of one timeval.
    let rc = unsafe {
        libc::sysctl(mib.as_mut_ptr(), mib.len() as libc::c_uint, (&raw mut boot).cast(), &mut len, std::ptr::null(), 0)
    };
    (rc == 0 && len == std::mem::size_of::<libc::timeval>())
        .then(|| (boot.tv_sec as u64).wrapping_mul(1_000_000).wrapping_add(boot.tv_usec as u64))
}

/// When `pid` started: the start time `proc_pidinfo` returns for
/// `PROC_PIDTBSDINFO`, microseconds of wall time.
#[cfg(target_os = "macos")]
pub(crate) fn start_of(pid: u32) -> Start {
    let id = match libc::c_int::try_from(pid) {
        Ok(p) if p > 0 => p,
        Ok(_not_positive) => return Start::Gone,
        Err(_past_c_int) => return Start::Gone,
    };
    let mut info = std::mem::MaybeUninit::<libc::proc_bsdinfo>::zeroed();
    let size = std::mem::size_of::<libc::proc_bsdinfo>() as libc::c_int;
    // SAFETY: the buffer holds one proc_bsdinfo, `size` bytes.
    let filled = unsafe { libc::proc_pidinfo(id, libc::PROC_PIDTBSDINFO, 0, info.as_mut_ptr().cast(), size) };
    if filled != size {
        return if exists(pid) { Start::Unreadable } else { Start::Gone };
    }
    // SAFETY: proc_pidinfo filled one whole proc_bsdinfo.
    let info = unsafe { info.assume_init() };
    Start::At(info.pbi_start_tvsec.wrapping_mul(1_000_000).wrapping_add(info.pbi_start_tvusec))
}

/// What the system says of `pid` where its start cannot be read: whether
/// a process has it.
#[cfg(all(unix, not(any(target_os = "linux", target_os = "freebsd", target_os = "macos"))))]
pub(crate) fn start_of(pid: u32) -> Start {
    if exists(pid) { Start::Unreadable } else { Start::Gone }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A child process running this test binary's sleeper, killed and
    /// reaped when dropped.
    struct Sleeper(std::process::Child);

    impl Sleeper {
        fn start() -> Self {
            let child = std::process::Command::new(std::env::current_exe().unwrap())
                .args(["--exact", "exit_watch::tests::sleeper_never_runs_on_its_own", "--ignored", "--nocapture"])
                .env("SUBETHA_EXIT_WATCH_SLEEP", "1")
                .spawn()
                .unwrap();
            Self(child)
        }

        fn pid(&self) -> u32 {
            self.0.id()
        }

        /// Kill and reap the child, returning its pid.
        fn end(mut self) -> u32 {
            let pid = self.pid();
            self.0.kill().unwrap();
            self.0.wait().unwrap();
            pid
        }
    }

    impl Drop for Sleeper {
        fn drop(&mut self) {
            if self.0.try_wait().unwrap().is_none() {
                self.0.kill().unwrap();
                self.0.wait().unwrap();
            }
        }
    }

    #[test]
    fn this_process_reads_its_own_start() {
        let me = current();
        assert_eq!(holder_pid(me), std::process::id());
        assert_ne!(holder_stamp(me), 0, "this process reads its own start");
        assert_eq!(probe(std::process::id()), Probe::Started(holder_stamp(me)));
        assert!(!gone(holder_pid(me), holder_stamp(me)));
        assert_eq!(current(), me, "the second read returns the cached word");
    }

    /// A live process recorded with its own start is not gone; recorded
    /// with any other start it is, which is how a pid the system handed to
    /// a newcomer reads.
    #[test]
    fn a_live_pid_with_another_start_reads_as_gone() {
        let child = Sleeper::start();
        let stamp = match probe(child.pid()) {
            Probe::Started(stamp) => stamp,
            other => panic!("a child of this process reads as started, not {other:?}"),
        };
        let other = stamp.wrapping_add(1).max(1);
        assert!(!gone(child.pid(), stamp));
        assert!(gone(child.pid(), other), "another start names another process");
        assert!(!gone(child.pid(), 0), "an unknown start leaves the pid alone to decide");
    }

    #[test]
    fn an_exited_process_reads_as_gone_whatever_its_stamp() {
        let child = Sleeper::start();
        let stamp = match probe(child.pid()) {
            Probe::Started(stamp) => stamp,
            other => panic!("a child of this process reads as started, not {other:?}"),
        };
        let pid = child.end();
        assert!(gone(pid, stamp));
        assert!(gone(pid, 0));
    }

    #[test]
    fn pid_zero_and_pids_past_the_system_range_read_as_gone() {
        assert_eq!(probe(0), Probe::Gone);
        assert!(gone(0, 0));
        assert!(gone(u32::MAX, 0));
    }

    #[test]
    fn a_stamp_is_never_zero_and_a_word_round_trips() {
        assert_eq!(fold(0), 1);
        assert_eq!(fold((1 << 32) | 1), 1);
        assert_eq!(fold(0x1234_5678), 0x1234_5678);
        let word = holder(4242, 0xDEAD_BEEF);
        assert_eq!(holder_pid(word), 4242);
        assert_eq!(holder_stamp(word), 0xDEAD_BEEF);
    }

    /// The command name sits in parentheses and may hold spaces and
    /// parentheses of its own; the start is the 22nd field after it.
    #[test]
    fn the_linux_start_field_is_counted_from_the_last_parenthesis() {
        let fields: Vec<String> = (3..=52).map(|n| n.to_string()).collect();
        let line = format!("4242 (a (tricky) name) {}", fields.join(" "));
        assert_eq!(linux_start(&line), Some(22));
        assert_eq!(linux_start("4242 (short) S 1 2"), None);
        assert_eq!(linux_start("no parenthesis at all"), None);
    }
}
