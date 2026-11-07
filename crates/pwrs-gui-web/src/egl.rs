//! Whether EGL starts on this machine, checked before WebKitGTK starts.
//!
//! WebKitGTK's web process opens a surfaceless EGL display as it starts
//! and aborts when that fails, as some GPU drivers make it fail ("Could
//! not create surfaceless EGL display: EGL_BAD_ALLOC" on an AMD RX 9060 XT,
//! "Could not create default EGL display: EGL_BAD_PARAMETER" where Mesa
//! does not match the driver), and then no page draws. The helper opens a
//! display the same way in a child process first, where a driver that
//! crashes takes only the child down. When that fails, WebKitGTK runs on
//! Mesa's software renderer and WebKit's shared-memory renderer, which the
//! environment set here selects for this process and for the processes
//! WebKitGTK starts.

use std::ffi::{CStr, c_void};

/// Mesa's surfaceless EGL platform, the one WebKitGTK's web process opens.
const PLATFORM_SURFACELESS_MESA: u32 = 0x31DD;

/// The environment that puts WebKitGTK on the software renderers.
const SOFTWARE: [(&str, &str); 3] = [("LIBGL_ALWAYS_SOFTWARE", "1"), ("GALLIUM_DRIVER", "llvmpipe"), ("WEBKIT_DISABLE_DMABUF_RENDERER", "1")];

/// How long the probe may take, in milliseconds, before it counts as
/// failed: a driver that hangs as EGL starts hangs WebKitGTK the same way.
const DEADLINE_MS: libc::c_int = 10_000;

/// The probe's exit codes: libEGL missing, an entry point missing, and the
/// bases to which the step that failed adds EGL's error, counted from
/// `EGL_SUCCESS` (0x3000).
const NO_LIBRARY: u8 = 1;
const NO_ENTRY: u8 = 2;
const DISPLAY_FAILED: u8 = 32;
const INITIALIZE_FAILED: u8 = 64;

/// EGL's errors, from `EGL_SUCCESS` (0x3000) to `EGL_CONTEXT_LOST`
/// (0x300E).
const ERRORS: [&str; 15] = [
    "EGL_SUCCESS",
    "EGL_NOT_INITIALIZED",
    "EGL_BAD_ACCESS",
    "EGL_BAD_ALLOC",
    "EGL_BAD_ATTRIBUTE",
    "EGL_BAD_CONFIG",
    "EGL_BAD_CONTEXT",
    "EGL_BAD_CURRENT_SURFACE",
    "EGL_BAD_DISPLAY",
    "EGL_BAD_MATCH",
    "EGL_BAD_NATIVE_PIXMAP",
    "EGL_BAD_NATIVE_WINDOW",
    "EGL_BAD_PARAMETER",
    "EGL_BAD_SURFACE",
    "EGL_CONTEXT_LOST",
];

/// The index past [`ERRORS`] that stands for an error EGL does not name.
const UNNAMED: u8 = 15;

type GetPlatformDisplay = unsafe extern "C" fn(u32, *mut c_void, *const isize) -> *mut c_void;
type Initialize = unsafe extern "C" fn(*mut c_void, *mut i32, *mut i32) -> u32;
type GetError = unsafe extern "C" fn() -> i32;

/// Probes EGL and, when it does not start, sets the environment that puts
/// WebKitGTK on the software renderers, then probes again; returns a line
/// for the session's log when it set it. A process told to render in
/// software already (`LIBGL_ALWAYS_SOFTWARE=1`) is left as it is.
///
/// # Safety
///
/// The process must have one thread: this forks, and it sets environment
/// variables, which another thread could be reading.
pub unsafe fn choose_renderer() -> Option<String> {
    if std::env::var_os("LIBGL_ALWAYS_SOFTWARE").is_some_and(|v| v == "1") {
        return None;
    }
    let why = match probe() {
        Ok(()) => return None,
        Err(why) => why,
    };
    for (name, value) in SOFTWARE {
        // SAFETY: the caller promises one thread, so nothing reads the
        // environment while it changes.
        unsafe { std::env::set_var(name, value) };
    }
    let now = match probe() {
        Ok(()) => String::from("WebKitGTK draws with Mesa's software renderer"),
        Err(again) => format!("WebKitGTK is set to draw with Mesa's software renderer, though EGL does not start with it either ({again}), so pages may not draw"),
    };
    Some(format!("EGL did not start on this machine's GPU ({why}); {now}"))
}

/// Opens a surfaceless EGL display in a child process: Ok when it
/// initializes, or what went wrong. The child's end of a pipe closes when
/// it exits, however it exits, and the parent waits for that with a
/// deadline, so a driver that hangs costs the deadline and no more.
fn probe() -> Result<(), String> {
    let mut ends = [0; 2];
    // SAFETY: `ends` has room for the two descriptors pipe fills.
    if unsafe { libc::pipe(ends.as_mut_ptr()) } != 0 {
        return Err(format!("the probe cannot make its pipe: {}", std::io::Error::last_os_error()));
    }
    let [read, write] = ends;
    // SAFETY: the process has one thread (choose_renderer's contract), so
    // the child may run any code before it leaves through _exit.
    match unsafe { libc::fork() } {
        -1 => {
            let error = std::io::Error::last_os_error();
            // SAFETY: both descriptors are this process's own.
            unsafe {
                libc::close(read);
                libc::close(write);
            }
            Err(format!("the probe cannot start: {error}"))
        }
        0 => {
            // SAFETY: in the child, which closes the pipe's read end it
            // does not use and leaves through _exit, skipping the parent's
            // exit handlers; its write end closes as it exits.
            unsafe {
                libc::close(read);
                libc::_exit(i32::from(open_display()))
            }
        }
        child => {
            // SAFETY: the write end belongs to the child.
            unsafe { libc::close(write) };
            let outcome = wait(child, read);
            // SAFETY: the read end is this process's own.
            unsafe { libc::close(read) };
            outcome
        }
    }
}

/// Waits for the child `child` to end, by its pipe's read end `read`
/// closing, and returns what its exit says. A child still running at the
/// deadline is ended.
fn wait(child: libc::pid_t, read: libc::c_int) -> Result<(), String> {
    let mut watch = libc::pollfd { fd: read, events: libc::POLLIN, revents: 0 };
    let ready = loop {
        // SAFETY: one pollfd, alive through the call.
        let ready = unsafe { libc::poll(&mut watch, 1, DEADLINE_MS) };
        if ready >= 0 || std::io::Error::last_os_error().kind() != std::io::ErrorKind::Interrupted {
            break ready;
        }
    };
    let mut status = 0;
    if ready <= 0 {
        let why = if ready == 0 {
            format!("EGL did not start within {} s", DEADLINE_MS / 1000)
        } else {
            format!("the probe's pipe cannot be polled: {}", std::io::Error::last_os_error())
        };
        // SAFETY: `child` is this process's child and not yet waited on.
        unsafe {
            libc::kill(child, libc::SIGKILL);
            libc::waitpid(child, &mut status, 0);
        }
        return Err(why);
    }
    // SAFETY: `child` is this process's child and not yet waited on.
    if unsafe { libc::waitpid(child, &mut status, 0) } != child {
        return Err(format!("the probe cannot be waited on: {}", std::io::Error::last_os_error()));
    }
    if libc::WIFEXITED(status) {
        match libc::WEXITSTATUS(status) {
            0 => Ok(()),
            code => Err(describe(code)),
        }
    } else if libc::WIFSIGNALED(status) {
        Err(format!("the driver ended the probe with signal {}", libc::WTERMSIG(status)))
    } else {
        Err(format!("the probe ended with status {status}"))
    }
}

/// What a probe's exit code, 0 to 255, says went wrong.
fn describe(code: libc::c_int) -> String {
    let display = libc::c_int::from(DISPLAY_FAILED);
    let initialize = libc::c_int::from(INITIALIZE_FAILED);
    match code {
        c if c == libc::c_int::from(NO_LIBRARY) => String::from("libEGL.so.1 cannot be loaded"),
        c if c == libc::c_int::from(NO_ENTRY) => String::from("libEGL.so.1 lacks eglGetPlatformDisplay, eglInitialize or eglGetError"),
        c if (display..initialize).contains(&c) => format!("the surfaceless EGL display cannot be made: {}", error_name(c - display)),
        c if c >= initialize => format!("the surfaceless EGL display cannot be initialized: {}", error_name(c - initialize)),
        c => format!("the probe exited with {c}"),
    }
}

/// The name of EGL's error at `index` from `EGL_SUCCESS`.
fn error_name(index: libc::c_int) -> &'static str {
    match index {
        0..=14 => ERRORS[index as usize],
        _ => "an error EGL does not name",
    }
}

/// EGL's error, counted from `EGL_SUCCESS`, as one byte of an exit code:
/// [`UNNAMED`] for one EGL does not name.
fn error_index(error: i32) -> u8 {
    match error {
        0x3000..=0x300E => (error - 0x3000) as u8,
        _ => UNNAMED,
    }
}

/// An entry point of the loaded libEGL, or none.
fn symbol(library: *mut c_void, name: &CStr) -> Option<*mut c_void> {
    // SAFETY: `library` is a handle dlopen returned and `name` ends in NUL.
    let address = unsafe { libc::dlsym(library, name.as_ptr()) };
    (!address.is_null()).then_some(address)
}

/// Opens and initializes a surfaceless EGL display, in the probe's child,
/// and returns its exit code: 0 when EGL starts.
fn open_display() -> u8 {
    // SAFETY: dlopen with a NUL-terminated name.
    let library = unsafe { libc::dlopen(c"libEGL.so.1".as_ptr(), libc::RTLD_NOW | libc::RTLD_LOCAL) };
    if library.is_null() {
        return NO_LIBRARY;
    }
    let (Some(get_display), Some(initialize), Some(get_error)) =
        (symbol(library, c"eglGetPlatformDisplay"), symbol(library, c"eglInitialize"), symbol(library, c"eglGetError"))
    else {
        return NO_ENTRY;
    };
    // SAFETY: the addresses are EGL 1.5's entry points of these signatures.
    let (get_display, initialize, get_error) = unsafe {
        (
            std::mem::transmute::<*mut c_void, GetPlatformDisplay>(get_display),
            std::mem::transmute::<*mut c_void, Initialize>(initialize),
            std::mem::transmute::<*mut c_void, GetError>(get_error),
        )
    };
    // SAFETY: the surfaceless platform takes no native display and no
    // attributes.
    let display = unsafe { get_display(PLATFORM_SURFACELESS_MESA, std::ptr::null_mut(), std::ptr::null()) };
    if display.is_null() {
        // SAFETY: EGL's error for the call just made on this thread.
        return DISPLAY_FAILED + error_index(unsafe { get_error() });
    }
    let (mut major, mut minor) = (0, 0);
    // SAFETY: a display eglGetPlatformDisplay returned, and two integers.
    if unsafe { initialize(display, &mut major, &mut minor) } == 0 {
        // SAFETY: EGL's error for the call just made on this thread.
        return INITIALIZE_FAILED + error_index(unsafe { get_error() });
    }
    0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_exit_code_names_the_step_and_egl_s_error() {
        assert_eq!(describe(i32::from(DISPLAY_FAILED + 3)), "the surfaceless EGL display cannot be made: EGL_BAD_ALLOC");
        assert_eq!(describe(i32::from(INITIALIZE_FAILED + 12)), "the surfaceless EGL display cannot be initialized: EGL_BAD_PARAMETER");
        assert_eq!(describe(i32::from(INITIALIZE_FAILED + UNNAMED)), "the surfaceless EGL display cannot be initialized: an error EGL does not name");
        assert_eq!(describe(i32::from(NO_LIBRARY)), "libEGL.so.1 cannot be loaded");
        assert_eq!(describe(5), "the probe exited with 5");
    }

    #[test]
    fn an_error_outside_egl_s_range_is_unnamed() {
        assert_eq!(error_index(0x3003), 3);
        assert_eq!(error_index(0x2fff), UNNAMED);
        assert_eq!(error_index(0x3100), UNNAMED);
    }
}
