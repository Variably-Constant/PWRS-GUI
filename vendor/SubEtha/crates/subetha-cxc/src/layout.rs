//! What a file holds against what a call asked for: the one shape every
//! structure's layout-mismatch error carries. An [`std::io::Error`] that
//! refuses a file or named region of another shape carries one too, which
//! [`Mismatch::carried_by`](crate::layout::Mismatch::carried_by) reads.

/// One field of a file's layout that differs from what the call asked
/// for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Mismatch {
    /// What differs, as a reader names it: `length`, `magic`,
    /// `capacity`, `value size`.
    pub field: &'static str,
    /// What the file holds.
    pub found: u64,
    /// What the call asked for.
    pub wanted: u64,
}

impl Mismatch {
    /// The field `field` holds `found` where the call asked for `wanted`.
    pub const fn new(field: &'static str, found: u64, wanted: u64) -> Self {
        Self { field, found, wanted }
    }

    /// A field a header may leave at zero to record nothing: at zero it
    /// reads as matching what the call asked for, and otherwise as what it
    /// holds.
    pub const fn recorded(field: &'static str, found: u64, wanted: u64) -> (&'static str, u64, u64) {
        (field, if found == 0 { wanted } else { found }, wanted)
    }

    /// The first of `fields`, each a name, what the file holds and what
    /// the call asked for, whose two values differ.
    pub fn first(fields: &[(&'static str, u64, u64)]) -> Result<(), Self> {
        match fields.iter().find(|(_, found, wanted)| found != wanted) {
            Some(&(field, found, wanted)) => Err(Self { field, found, wanted }),
            None => Ok(()),
        }
    }

    /// The mismatch `e` carries when it refuses a file or named region of
    /// another shape; `None` for any other error.
    pub fn carried_by(e: &std::io::Error) -> Option<Self> {
        e.get_ref()?.downcast_ref::<Self>().copied()
    }
}

impl std::error::Error for Mismatch {}

/// The refusal a constructor returning [`std::io::Result`] gives a file
/// of another shape: [`std::io::ErrorKind::InvalidData`] carrying the
/// mismatch.
impl From<Mismatch> for std::io::Error {
    fn from(m: Mismatch) -> Self {
        std::io::Error::new(std::io::ErrorKind::InvalidData, m)
    }
}

impl std::fmt::Display for Mismatch {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let Self { field, found, wanted } = *self;
        match field {
            "length" => write!(f, "the file is {found} bytes where {wanted} were asked for"),
            "magic" => write!(f, "the file's magic is {found:#x} where {wanted:#x} was asked for"),
            _ => write!(f, "the file's {field} is {found} where {wanted} was asked for"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_first_field_that_differs_is_named() {
        assert_eq!(Mismatch::first(&[("magic", 7, 7), ("capacity", 16, 16)]), Ok(()));
        assert_eq!(
            Mismatch::first(&[("magic", 7, 7), ("capacity", 16, 32), ("value size", 4, 8)]),
            Err(Mismatch::new("capacity", 16, 32)),
        );
    }

    #[test]
    fn each_field_reads_in_its_own_units() {
        assert_eq!(
            Mismatch::new("capacity", 16, 32).to_string(),
            "the file's capacity is 16 where 32 was asked for"
        );
        assert_eq!(
            Mismatch::new("length", 10, 4160).to_string(),
            "the file is 10 bytes where 4160 were asked for"
        );
        assert_eq!(
            Mismatch::new("magic", 0x5744, 0x4d41).to_string(),
            "the file's magic is 0x5744 where 0x4d41 was asked for"
        );
    }

    #[test]
    fn an_io_error_carries_the_mismatch_it_refuses_with() {
        let refused = std::io::Error::from(Mismatch::new("length", 10, 4160));
        assert_eq!(refused.kind(), std::io::ErrorKind::InvalidData);
        assert_eq!(Mismatch::carried_by(&refused), Some(Mismatch::new("length", 10, 4160)));
        assert_eq!(refused.to_string(), "the file is 10 bytes where 4160 were asked for");
        let other = std::io::Error::new(std::io::ErrorKind::InvalidData, "a malformed datagram");
        assert_eq!(Mismatch::carried_by(&other), None);
        assert_eq!(Mismatch::carried_by(&std::io::Error::from(std::io::ErrorKind::NotFound)), None);
    }
}
