//! Every public error type in `subetha-pointers` implements
//! `std::error::Error`, is `Send + Sync + 'static` so `?` carries it into
//! `Box<dyn Error>` or `anyhow`, and formats to a message.

use std::error::Error;

use subetha_pointers::adaptive_cheri_pointer::CapabilityError;
use subetha_pointers::adaptive_rasp_batch::RaspError;

/// Formats `e`, failing on an empty message, which a reporter would print
/// as nothing.
fn message<E: Error + Send + Sync + 'static>(e: E) -> String {
    let text = e.to_string();
    assert!(!text.is_empty(), "{e:?} formats to an empty message");
    text
}

#[test]
fn every_error_type_formats() {
    for e in [
        CapabilityError::OutOfBounds,
        CapabilityError::PermissionDenied,
        CapabilityError::Sealed,
        CapabilityError::AddressOverflow,
        CapabilityError::Misaligned,
    ] {
        message(e);
    }
    for e in [
        RaspError::OutOfBounds,
        RaspError::PermissionDenied,
        RaspError::Sealed,
        RaspError::AddressOverflow,
        RaspError::LayoutTooWide,
        RaspError::FeatureNotSupported,
    ] {
        message(e);
    }
}
