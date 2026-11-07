//! The error records the module raises, and how a cmdlet finds the files
//! it reads and goes on past one that fails.

use std::any::Any;
use std::fmt::Display;
use std::path::Path;

use pwrs::prelude::*;

/// An argument the call cannot take.
pub(crate) fn invalid(id: &'static str, message: impl Into<String>) -> PsError {
    PsError::new(ErrorCategory::InvalidArgument, id, message.into())
}

/// Something the caller named that does not exist.
pub(crate) fn not_found(id: &'static str, message: impl Into<String>) -> PsError {
    PsError::new(ErrorCategory::ObjectNotFound, id, message.into())
}

/// A call the object's state does not allow.
pub(crate) fn invalid_operation(id: &'static str, message: impl Into<String>) -> PsError {
    PsError::new(ErrorCategory::InvalidOperation, id, message.into())
}

/// An error the GPU, its driver or wgpu raised when `what` failed.
pub(crate) fn device_error(id: &'static str, what: &str, e: impl Display) -> PsError {
    PsError::new(ErrorCategory::DeviceError, id, format!("{what}: {e}"))
}

/// The message a panic carried, as `panic!` and a failed assertion give
/// one. The module's own threads report it with the error they raise,
/// since nothing prints a panic once PWRS has loaded the module.
pub(crate) fn panic_text(payload: Box<dyn Any + Send>) -> String {
    if let Some(text) = payload.downcast_ref::<&str>() {
        (*text).to_string()
    } else if let Some(text) = payload.downcast_ref::<String>() {
        text.clone()
    } else {
        "a panic that carried no message".to_string()
    }
}

/// A path PowerShell could not resolve, raised as `id` with PowerShell's
/// own reason, such as "Cannot find path '...' because it does not
/// exist."; a stopped pipeline stays the error it is.
pub(crate) fn unresolved(id: &'static str, e: PsError) -> PsError {
    if e.category == ErrorCategory::OperationStopped {
        return e;
    }
    let reason = e.message.split_once(": ").filter(|(kind, _)| kind.ends_with("Exception")).map_or(e.message.as_str(), |(_, rest)| rest);
    PsError::new(ErrorCategory::ObjectNotFound, id, reason)
}

/// The files a cmdlet's -Path or -LiteralPath names. A -Path wildcard
/// names every file it matches; a -LiteralPath names one file as written,
/// wildcard characters included, and may be a path a provider qualifies,
/// as the PSPath Get-ChildItem pipes is. A path that names no file is
/// raised as `id` with PowerShell's reason, the same for both.
pub(crate) fn input_files(ps: &Pipeline<'_>, path: &str, literal: bool, id: &'static str) -> PsResult<Vec<String>> {
    let files = ps.resolve_path(path, literal).map_err(|e| unresolved(id, e))?;
    if files.is_empty() {
        return Err(not_found(id, format!("{path} names no file")));
    }
    if literal {
        for file in &files {
            match Path::new(file).try_exists() {
                Ok(true) => {}
                Ok(false) => return Err(not_found(id, format!("Cannot find path '{file}' because it does not exist."))),
                Err(e) => return Err(not_found(id, format!("{file} cannot be read: {e}"))),
            }
        }
    }
    Ok(files)
}

/// Runs `each` on every file in turn. A file `each` fails on is written to
/// the error stream and the files after it still run; a stopped pipeline,
/// or an error that ends the call, ends the loop.
pub(crate) fn for_each_file(ps: &Pipeline<'_>, files: Vec<String>, mut each: impl FnMut(String) -> PsResult<()>) -> PsResult<()> {
    for file in files {
        if ps.stopping() {
            break;
        }
        if let Err(e) = each(file) {
            if e.terminating || e.category == ErrorCategory::OperationStopped {
                return Err(e);
            }
            ps.write_error(&e)?;
        }
    }
    Ok(())
}
