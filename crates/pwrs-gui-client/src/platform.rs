//! The operating system's part: whether this session can show a window,
//! the job object that ends a helper with the process that started it on
//! Windows, and the wait that reports when a helper has exited.

use crate::{GuiError, Inner, StartOptions};
use std::path::{Path, PathBuf};
use std::sync::Weak;

/// Fails when a session started as `options` says has no display a window
/// could open on. On macOS the helper reports a missing window server when
/// it starts.
pub(crate) fn check_display(options: &StartOptions) -> Result<(), GuiError> {
    imp::check_display(options).map_err(|why| GuiError::NoDisplay(format!("no window can open: {why}")))
}

/// What a session's death says about a helper that ended: how it ended,
/// and the last lines of its log with the log's path.
pub(crate) fn helper_ended(how: &str, log: &Path) -> String {
    let tail = match std::fs::read_to_string(log) {
        Ok(text) if text.trim().is_empty() => format!("its log {} is empty", log.display()),
        Ok(text) => {
            let lines: Vec<&str> = text.lines().collect();
            let from = lines.len().saturating_sub(LOG_LINES);
            format!("the last lines of its log {}:\n{}", log.display(), lines[from..].join("\n"))
        }
        Err(e) => format!("its log {} cannot be read: {e}", log.display()),
    };
    format!("the helper ended ({how}); {tail}")
}

/// Lines of a dead helper's log an error includes; the whole log stays at
/// the path the error names.
const LOG_LINES: usize = 20;

#[cfg(windows)]
mod imp {
    use crate::StartOptions;
    use windows_sys::Win32::System::StationsAndDesktops::{GetProcessWindowStation, GetUserObjectInformationW, UOI_FLAGS, USEROBJECTFLAGS};

    /// `WSF_VISIBLE`: the window station has visible display surfaces.
    const WSF_VISIBLE: u32 = 1;

    pub(super) fn check_display(_options: &StartOptions) -> Result<(), String> {
        let station = unsafe { GetProcessWindowStation() };
        if station.is_null() {
            return Err(format!("the process has no window station: {}", std::io::Error::last_os_error()));
        }
        let mut flags = USEROBJECTFLAGS { fInherit: 0, fReserved: 0, dwFlags: 0 };
        let mut needed = 0u32;
        let size = u32::try_from(size_of::<USEROBJECTFLAGS>()).expect("USEROBJECTFLAGS is a few bytes");
        let ok = unsafe { GetUserObjectInformationW(station, UOI_FLAGS, (&mut flags as *mut USEROBJECTFLAGS).cast(), size, &mut needed) };
        if ok == 0 {
            return Err(format!("the window station's flags cannot be read: {}", std::io::Error::last_os_error()));
        }
        if flags.dwFlags & WSF_VISIBLE == 0 {
            return Err(String::from("this session's window station is not visible, as in a service"));
        }
        Ok(())
    }
}

#[cfg(all(unix, not(target_vendor = "apple")))]
mod imp {
    use crate::StartOptions;

    pub(super) fn check_display(options: &StartOptions) -> Result<(), String> {
        if options.var("DISPLAY").is_some() || options.var("WAYLAND_DISPLAY").is_some() {
            Ok(())
        } else {
            Err(String::from("neither DISPLAY nor WAYLAND_DISPLAY is set"))
        }
    }
}

#[cfg(target_vendor = "apple")]
mod imp {
    use crate::StartOptions;

    pub(super) fn check_display(_options: &StartOptions) -> Result<(), String> {
        Ok(())
    }
}

#[cfg(windows)]
pub(crate) use windows_parts::{ExitWatch, Job, allow_front, kill_on_close};

#[cfg(windows)]
mod windows_parts {
    use super::*;
    use std::cell::Cell;
    use std::ffi::c_void;
    use std::io;
    use std::os::windows::io::AsRawHandle;
    use std::process::Child;
    use std::ptr::null_mut;
    use windows_sys::Win32::Foundation::{CloseHandle, DUPLICATE_SAME_ACCESS, DuplicateHandle, ERROR_IO_PENDING, HANDLE, INVALID_HANDLE_VALUE};
    use windows_sys::Win32::System::JobObjects::{
        AssignProcessToJobObject, CreateJobObjectW, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE, JOB_OBJECT_LIMIT_SILENT_BREAKAWAY_OK, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
        JobObjectExtendedLimitInformation, SetInformationJobObject,
    };
    use windows_sys::Win32::System::Threading::{GetCurrentProcess, GetExitCodeProcess, INFINITE, RegisterWaitForSingleObject, UnregisterWaitEx, WT_EXECUTEONLYONCE};

    /// Lets process `pid` take the front, so a helper started earlier may
    /// bring a new window forward as a newly started one may. Windows takes
    /// the grant only from a process that may take the front itself, as pwsh
    /// may while a person's command runs in its console, and returns
    /// whether it did.
    pub(crate) fn allow_front(pid: u32) -> bool {
        use windows_sys::Win32::UI::WindowsAndMessaging::AllowSetForegroundWindow;
        unsafe { AllowSetForegroundWindow(pid) != 0 }
    }

    /// A job object that ends every process in it when its last handle
    /// closes, so a helper ends with the process that started it.
    pub(crate) struct Job(HANDLE);

    // SAFETY: a job handle is a kernel handle, usable from any thread.
    unsafe impl Send for Job {}
    unsafe impl Sync for Job {}

    impl Drop for Job {
        fn drop(&mut self) {
            if unsafe { CloseHandle(self.0) } == 0 {
                eprintln!("PWRS-GUI: the helper's job object could not be closed: {}", io::Error::last_os_error());
            }
        }
    }

    /// Puts `child` in a new job that ends it when the job's handle closes.
    /// With `children_outside`, the processes `child` starts stay out of the
    /// job: the web helper's WebView2 browser process is shared by every
    /// process using the same data folder, so WebView2 ends it, not the job
    /// of the helper that happened to start it.
    pub(crate) fn kill_on_close(child: &Child, children_outside: bool) -> io::Result<Job> {
        let handle = unsafe { CreateJobObjectW(std::ptr::null(), std::ptr::null()) };
        if handle.is_null() {
            return Err(io::Error::last_os_error());
        }
        let job = Job(handle);
        let mut info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = unsafe { std::mem::zeroed() };
        info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE | if children_outside { JOB_OBJECT_LIMIT_SILENT_BREAKAWAY_OK } else { 0 };
        let size = u32::try_from(size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>()).expect("the limit block is a few hundred bytes");
        if unsafe { SetInformationJobObject(job.0, JobObjectExtendedLimitInformation, (&info as *const JOBOBJECT_EXTENDED_LIMIT_INFORMATION).cast(), size) } == 0 {
            return Err(io::Error::last_os_error());
        }
        if unsafe { AssignProcessToJobObject(job.0, child.as_raw_handle()) } == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(job)
    }

    thread_local! {
        /// Set while this thread runs an exit callback, so a session whose
        /// last reference drops inside it unregisters without waiting on
        /// the callback it is in.
        static IN_EXIT_CALLBACK: Cell<bool> = const { Cell::new(false) };
    }

    /// What an exit callback reads: its own handle on the helper process,
    /// the session to tell, and the helper's log.
    struct ExitContext {
        process: HANDLE,
        session: Weak<Inner>,
        log: PathBuf,
    }

    impl Drop for ExitContext {
        fn drop(&mut self) {
            if unsafe { CloseHandle(self.process) } == 0 {
                eprintln!("PWRS-GUI: the helper's process handle could not be closed: {}", io::Error::last_os_error());
            }
        }
    }

    /// The thread pool's wait on a helper's exit, which tells the session
    /// when it fires.
    pub(crate) struct ExitWatch {
        wait: HANDLE,
        context: *mut ExitContext,
    }

    // SAFETY: the wait handle and the context pointer are used only to
    // unregister and free, once, in Drop.
    unsafe impl Send for ExitWatch {}
    unsafe impl Sync for ExitWatch {}

    unsafe extern "system" fn on_exit(context: *mut c_void, _timed_out: bool) {
        IN_EXIT_CALLBACK.with(|f| f.set(true));
        let (process, session, log) = {
            // SAFETY: the context lives until the wait is unregistered, which
            // waits for this callback or happens inside it below.
            let c = unsafe { &*(context as *const ExitContext) };
            (c.process, c.session.clone(), c.log.clone())
        };
        if let Some(inner) = session.upgrade() {
            let mut code = 0u32;
            let how = if unsafe { GetExitCodeProcess(process, &mut code) } != 0 {
                format!("exit code {code:#x}")
            } else {
                format!("its exit code cannot be read: {}", io::Error::last_os_error())
            };
            inner.die(helper_ended(&how, &log));
            drop(inner);
        }
        IN_EXIT_CALLBACK.with(|f| f.set(false));
    }

    impl ExitWatch {
        pub(crate) fn arm(child: &Child, session: Weak<Inner>, log: PathBuf) -> io::Result<ExitWatch> {
            let me = unsafe { GetCurrentProcess() };
            let mut process: HANDLE = null_mut();
            if unsafe { DuplicateHandle(me, child.as_raw_handle(), me, &mut process, 0, 0, DUPLICATE_SAME_ACCESS) } == 0 {
                return Err(io::Error::last_os_error());
            }
            let context = Box::into_raw(Box::new(ExitContext { process, session, log }));
            let mut wait: HANDLE = null_mut();
            if unsafe { RegisterWaitForSingleObject(&mut wait, process, Some(on_exit), context.cast(), INFINITE, WT_EXECUTEONLYONCE) } == 0 {
                let e = io::Error::last_os_error();
                // SAFETY: the registration failed, so nothing else refers
                // to it.
                drop(unsafe { Box::from_raw(context) });
                return Err(e);
            }
            Ok(ExitWatch { wait, context })
        }
    }

    impl Drop for ExitWatch {
        fn drop(&mut self) {
            let in_callback = IN_EXIT_CALLBACK.with(|f| f.get());
            let completion = if in_callback { null_mut() } else { INVALID_HANDLE_VALUE };
            if unsafe { UnregisterWaitEx(self.wait, completion) } == 0 {
                let e = io::Error::last_os_error();
                let pending = in_callback && e.raw_os_error() == Some(ERROR_IO_PENDING as i32);
                if !pending {
                    eprintln!("PWRS-GUI: the wait on the helper's exit could not be unregistered: {e}");
                }
            }
            // SAFETY: outside the callback the unregistration waited for any
            // callback to finish; inside it, the callback has finished reading
            // the context.
            drop(unsafe { Box::from_raw(self.context) });
        }
    }
}

#[cfg(unix)]
pub(crate) use unix_parts::ExitWatch;

#[cfg(unix)]
mod unix_parts {
    use super::*;
    use std::io;
    use std::process::Child;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};

    /// One thread per helper, parked on its exit: it waits without
    /// reaping, marks the helper exited, reaps it, and tells the session.
    /// Waiting before reaping means the process id cannot be reused while a
    /// kill might still target it.
    pub(crate) struct ExitWatch {
        pid: libc::pid_t,
        exited: Arc<AtomicBool>,
    }

    impl ExitWatch {
        pub(crate) fn arm(child: Child, session: Weak<Inner>, log: PathBuf) -> io::Result<ExitWatch> {
            let pid = libc::pid_t::try_from(child.id()).map_err(|e| io::Error::other(format!("process id {} does not fit pid_t: {e}", child.id())))?;
            drop(child);
            let exited = Arc::new(AtomicBool::new(false));
            let flag = Arc::clone(&exited);
            std::thread::Builder::new().name(String::from("pwrs-gui-exit")).spawn(move || {
                let how = wait_for(pid, &flag);
                if let Some(inner) = session.upgrade() {
                    inner.die(helper_ended(&how, &log));
                }
            })?;
            Ok(ExitWatch { pid, exited })
        }

        /// Ends the helper unless it has already exited.
        pub(crate) fn kill(&self) {
            if !self.exited.load(Ordering::SeqCst) && unsafe { libc::kill(self.pid, libc::SIGKILL) } != 0 {
                let e = io::Error::last_os_error();
                if e.raw_os_error() != Some(libc::ESRCH) {
                    eprintln!("PWRS-GUI: the helper {} could not be ended: {e}", self.pid);
                }
            }
        }
    }

    /// Waits for `pid` to exit, marks `exited`, reaps it, and says how it
    /// ended.
    fn wait_for(pid: libc::pid_t, exited: &AtomicBool) -> String {
        // A child's process id is positive, so its value is unchanged in
        // id_t, which is unsigned on Linux and 64 bits wide on FreeBSD.
        let id = pid as libc::id_t;
        loop {
            let mut info: libc::siginfo_t = unsafe { std::mem::zeroed() };
            if unsafe { libc::waitid(libc::P_PID, id, &mut info, libc::WEXITED | libc::WNOWAIT) } == 0 {
                break;
            }
            let e = io::Error::last_os_error();
            if e.raw_os_error() != Some(libc::EINTR) {
                exited.store(true, Ordering::SeqCst);
                return format!("its exit cannot be waited on: {e}");
            }
        }
        exited.store(true, Ordering::SeqCst);
        let mut status = 0;
        loop {
            if unsafe { libc::waitpid(pid, &mut status, 0) } == pid {
                break;
            }
            let e = io::Error::last_os_error();
            if e.raw_os_error() != Some(libc::EINTR) {
                return format!("it could not be reaped: {e}");
            }
        }
        if libc::WIFEXITED(status) {
            format!("exit code {}", libc::WEXITSTATUS(status))
        } else if libc::WIFSIGNALED(status) {
            format!("signal {}", libc::WTERMSIG(status))
        } else {
            format!("wait status {status:#x}")
        }
    }
}
