//! A report of a fault that ends the test process. On Windows the tests'
//! device helpers install an unhandled-exception filter, once per process,
//! that writes the exception's code and address, the module the address
//! is in and its offset there, the name of the faulting thread, which
//! libtest gives as the running test's name, and a backtrace, straight to
//! the standard error handle, which libtest does not capture. Elsewhere
//! installing it does nothing.

#[cfg(windows)]
mod windows {
    use std::ffi::c_void;
    use std::io::Write;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

    #[repr(C)]
    struct ExceptionRecord {
        code: u32,
        flags: u32,
        record: *const ExceptionRecord,
        address: *const c_void,
        parameters: u32,
        information: [usize; 15],
    }

    #[repr(C)]
    struct ExceptionPointers {
        record: *const ExceptionRecord,
        context: *const c_void,
    }

    type Filter = Option<unsafe extern "system" fn(*const ExceptionPointers) -> i32>;

    unsafe extern "system" {
        fn SetUnhandledExceptionFilter(filter: Filter) -> Filter;
        fn GetModuleHandleExW(flags: u32, name: *const u16, module: *mut *mut c_void) -> i32;
        fn GetModuleFileNameW(module: *mut c_void, name: *mut u16, size: u32) -> u32;
    }

    /// GetModuleHandleExW's flags: the name is an address in the module,
    /// and the module's reference count is left as it is.
    const FROM_ADDRESS: u32 = 0x4;
    const UNCHANGED_REFCOUNT: u32 = 0x2;
    /// What a filter returns to let the next one, or the system, handle the
    /// exception.
    const CONTINUE_SEARCH: i32 = 0;

    /// Whether a caller has installed the filter; the first to set it does.
    static INSTALLED: AtomicBool = AtomicBool::new(false);
    /// The filter installed before this one, as an address, 0 for none.
    static PREVIOUS: AtomicUsize = AtomicUsize::new(0);

    pub(crate) fn install() {
        if INSTALLED.swap(true, Ordering::AcqRel) {
            return;
        }
        // SAFETY: `report` has the signature the system calls a filter with,
        // and lives as long as the process.
        let previous = unsafe { SetUnhandledExceptionFilter(Some(report)) };
        if let Some(previous) = previous {
            PREVIOUS.store(previous as usize, Ordering::Release);
        }
    }

    /// The module holding `address`, as its file's path, and the address's
    /// offset in it; `None` for an address in no module.
    fn module_of(address: *const c_void) -> Option<(String, usize)> {
        let mut module = std::ptr::null_mut();
        // SAFETY: with FROM_ADDRESS the name is read as an address, which
        // needs only to be a value; `module` is a valid out pointer.
        let found = unsafe { GetModuleHandleExW(FROM_ADDRESS | UNCHANGED_REFCOUNT, address.cast(), &mut module) };
        if found == 0 {
            return None;
        }
        let mut name = [0u16; 1024];
        // SAFETY: the buffer holds as many units as the size given.
        let len = unsafe { GetModuleFileNameW(module, name.as_mut_ptr(), name.len() as u32) } as usize;
        Some((String::from_utf16_lossy(&name[..len.min(name.len())]), (address as usize).wrapping_sub(module as usize)))
    }

    unsafe extern "system" fn report(pointers: *const ExceptionPointers) -> i32 {
        // SAFETY: the system passes the pointers of the exception being
        // dispatched, valid for the call.
        let record = unsafe { &*(*pointers).record };
        let place = match module_of(record.address) {
            Some((module, offset)) => format!("{module}+{offset:#x}"),
            None => "no module".to_string(),
        };
        let thread = std::thread::current();
        let count = (record.parameters as usize).min(record.information.len());
        let line = format!(
            "FAULT {:#010x} at {:?} in {place}, thread {:?}, parameters {:x?}\n{}\n",
            record.code,
            record.address,
            thread.name(),
            &record.information[..count],
            std::backtrace::Backtrace::force_capture()
        );
        // The process is ending; with no stream to report on, end it now.
        if std::io::stderr().write_all(line.as_bytes()).is_err() {
            std::process::abort();
        }
        match PREVIOUS.load(Ordering::Acquire) {
            0 => CONTINUE_SEARCH,
            previous => {
                // SAFETY: the address is the filter the system held before
                // this one, stored by `install` from a function pointer.
                let previous: unsafe extern "system" fn(*const ExceptionPointers) -> i32 = unsafe { std::mem::transmute(previous) };
                // SAFETY: the same pointers this filter was given.
                unsafe { previous(pointers) }
            }
        }
    }
}

/// Installs the fault report, once per process.
pub(crate) fn install() {
    #[cfg(windows)]
    windows::install();
}
