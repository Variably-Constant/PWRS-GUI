//! Methods that return tasks: the GPU work a task waits for runs on a
//! thread of its own, which settles the task when the work ends.

use std::panic::{AssertUnwindSafe, catch_unwind};

use pwrs::prelude::*;

use crate::error::{device_error, panic_text};

/// What a GPU wait asks between polls whether to end early: a stopped
/// pipeline, or a task whose caller canceled it.
pub(crate) trait Interrupt {
    fn interrupted(&self) -> bool;
}

impl Interrupt for Pipeline<'_> {
    fn interrupted(&self) -> bool {
        self.stopping()
    }
}

impl<T> Interrupt for PsTask<T> {
    fn interrupted(&self) -> bool {
        self.is_cancelled()
    }
}

/// The error id of a wait an [`Interrupt`] ended.
pub(crate) const STOPPED: &str = "WgpuStopped";

/// The error a wait an [`Interrupt`] ended returns.
pub(crate) fn stopped() -> PsError {
    PsError::new(
        ErrorCategory::OperationStopped,
        STOPPED,
        "the wait was stopped while the GPU was still working; the work goes on and finishes on its own",
    )
}

/// Runs `f`, and returns a panic in it as the error `WgpuPanic` with the
/// panic's message.
fn caught<R>(f: impl FnOnce() -> PsResult<R>) -> PsResult<R> {
    match catch_unwind(AssertUnwindSafe(f)) {
        Ok(result) => result,
        Err(payload) => Err(device_error("WgpuPanic", "the task's work panicked", panic_text(payload))),
    }
}

/// Runs `work` on a thread of its own, then `convert` on that thread with
/// it attached to the host, and settles `task`: completed with the value,
/// canceled when the caller's token ended a wait, faulted with any other
/// error, a panic in either included.
pub(crate) fn spawn<T, R>(
    task: PsTask<T>,
    work: impl FnOnce(&dyn Interrupt) -> PsResult<R> + Send + 'static,
    convert: impl FnOnce(R) -> PsResult<T> + Send + 'static,
) -> PsResult<()>
where
    T: IntoPs + 'static,
    R: 'static,
{
    let started = std::thread::Builder::new().name("Wgpu task".to_string()).spawn(move || match caught(|| work(&task)) {
        Ok(value) => {
            let _attached = pwrs::attach_current_thread();
            task.finish(caught(|| convert(value)));
        }
        Err(e) if e.error_id == STOPPED => task.cancel(),
        Err(e) => task.fail(e),
    });
    match started {
        Ok(_detached) => Ok(()),
        Err(e) => Err(device_error("WgpuTaskThread", "the task's thread cannot start", e)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A panic in a task's work is raised as `WgpuPanic` with its message,
    /// formatted or not, or with a note that it carried none; work that
    /// returns, a stopped wait's error included, passes through as it is.
    #[test]
    fn a_panic_in_a_tasks_work_is_raised_with_its_message() {
        let literal = caught(|| -> PsResult<()> { panic!("a literal message") });
        let formatted = caught(|| -> PsResult<()> { panic!("index {} is past {}", 7, 3) });
        let silent = caught(|| -> PsResult<()> { std::panic::panic_any(42u32) });
        for (got, message) in [(literal, "a literal message"), (formatted, "index 7 is past 3"), (silent, "a panic that carried no message")] {
            match got {
                Ok(()) => panic!("work that panicked returned a value"),
                Err(e) => {
                    assert_eq!(e.error_id, "WgpuPanic");
                    assert_eq!(e.message, format!("the task's work panicked: {message}"));
                }
            }
        }
        match caught(|| Ok(5)) {
            Ok(value) => assert_eq!(value, 5),
            Err(e) => panic!("work that returned 5 failed: {e:?}"),
        }
        match caught(|| -> PsResult<()> { Err(stopped()) }) {
            Ok(()) => panic!("a stopped wait returned a value"),
            Err(e) => assert_eq!(e.error_id, STOPPED),
        }
    }
}
