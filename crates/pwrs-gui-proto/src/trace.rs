//! Lines on standard error that follow a renderer's start and each
//! window's life, written only when the environment variable
//! `PWRS_GUI_TRACE` is set, to find where a renderer stops. Each line
//! gives the milliseconds since the process's first line and the writing
//! thread's name.

use std::sync::OnceLock;
use std::time::Instant;

/// When tracing started, or `None` when it is off; set once.
static STARTED: OnceLock<Option<Instant>> = OnceLock::new();

/// When tracing started, or `None` when it is off: set by the first line
/// from `PWRS_GUI_TRACE` in the process's environment, unless [`decide`]
/// set it first.
fn started() -> Option<Instant> {
    *STARTED.get_or_init(|| std::env::var_os("PWRS_GUI_TRACE").map(|_| Instant::now()))
}

/// Sets whether this process traces, from `PWRS_GUI_TRACE` as an
/// environment other than the process's own has it, such as a PowerShell
/// session's on Linux, FreeBSD and macOS, where `$env:` stays out of the
/// process's environment. Once set, by this or by a first line, it stays.
pub fn decide(on: bool) {
    STARTED.get_or_init(|| on.then(Instant::now));
}

/// Whether tracing is on.
pub fn enabled() -> bool {
    started().is_some()
}

/// Writes the line `text` makes, when tracing is on; `text` runs only
/// then.
pub fn line(text: impl FnOnce() -> String) {
    if let Some(start) = started() {
        let thread = std::thread::current();
        eprintln!("[pwrs-gui {:9.1} ms {}] {}", start.elapsed().as_secs_f64() * 1000.0, thread.name().unwrap_or("unnamed"), text());
    }
}
