//! A value of up to `N` bytes that carries its own length: what a
//! structure generic over a copyable value holds when its callers have
//! only bytes to give. The C ABI, Python and PowerShell hold their
//! channels', queues' and slot families' items as one, and every surface
//! encodes it the same way, so what one writes another reads.

use subetha_core::{Marshal, MarshalError};

/// Up to `N` bytes and how many of them are the value.
///
/// The length travels with the bytes because a slot is a fixed size:
/// without it a value ending in zero bytes would come back
/// indistinguishable from a shorter one padded out.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ByteValue<const N: usize> {
    len: u32,
    bytes: [u8; N],
}

impl<const N: usize> Default for ByteValue<N> {
    fn default() -> Self {
        Self::empty()
    }
}

/// A value longer than the [`ByteValue`] asked to hold it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ValueTooLong {
    /// The most the value may be, in bytes.
    pub max: usize,
    /// How long the value was.
    pub got: usize,
}

impl std::fmt::Display for ValueTooLong {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "a value here is at most {} bytes, got {}", self.max, self.got)
    }
}

impl std::error::Error for ValueTooLong {}

impl<const N: usize> ByteValue<N> {
    /// The value of no bytes.
    pub fn empty() -> Self {
        Self { len: 0, bytes: [0; N] }
    }

    /// `value` with its length, or [`ValueTooLong`] past `N` bytes.
    pub fn from_bytes(value: &[u8]) -> Result<Self, ValueTooLong> {
        if value.len() > N {
            return Err(ValueTooLong { max: N, got: value.len() });
        }
        let mut held = Self::empty();
        held.len = value.len() as u32;
        held.bytes[..value.len()].copy_from_slice(value);
        Ok(held)
    }

    /// The value's bytes, without the padding after them.
    pub fn as_bytes(&self) -> &[u8] {
        let len = (self.len as usize).min(N);
        &self.bytes[..len]
    }
}

/// A value travels between processes as its length and then its bytes,
/// little-endian, which is the same in every address space.
///
/// # Safety
///
/// `marshal` writes exactly `4 + N` bytes and `unmarshal` reads exactly
/// that many. There are no pointers, handles or descriptors in it: a
/// length and a run of bytes mean the same thing wherever they are read.
/// A length larger than `N` cannot name real bytes, so it is refused as
/// an invalid encoding rather than trusted.
unsafe impl<const N: usize> Marshal for ByteValue<N> {
    const PAYLOAD_BYTES: usize = 4 + N;

    fn marshal(&self, dst: &mut [u8]) {
        dst[..4].copy_from_slice(&self.len.to_le_bytes());
        dst[4..4 + N].copy_from_slice(&self.bytes);
    }

    fn unmarshal(src: &[u8]) -> Result<Self, MarshalError> {
        if src.len() < 4 + N {
            return Err(MarshalError::ShortBuffer { expected: 4 + N, got: src.len() });
        }
        let len = u32::from_le_bytes([src[0], src[1], src[2], src[3]]);
        if len as usize > N {
            return Err(MarshalError::InvalidEncoding);
        }
        let mut held = Self::empty();
        held.len = len;
        held.bytes.copy_from_slice(&src[4..4 + N]);
        Ok(held)
    }
}

/// The most a value can be in the families whose slot is 56 bytes, four
/// of which record the length.
pub const SLOT_VALUE_BYTES: usize = 52;

/// What those families hold: a channel's, a work queue's and a keyed
/// map's items.
pub type SlotValue = ByteValue<SLOT_VALUE_BYTES>;

/// The most a lease's value can be: the lease's region gives 48 bytes to
/// the value, four of which record how long it is.
pub const LEASE_VALUE_BYTES: usize = 44;

/// What an owner lease and the versioned families hold.
pub type LeaseValue = ByteValue<LEASE_VALUE_BYTES>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_value_travels_as_its_length_then_its_bytes() {
        let value = SlotValue::from_bytes(b"ab").expect("two bytes fit");
        let mut wire = [0xFFu8; 4 + SLOT_VALUE_BYTES];
        value.marshal(&mut wire);
        assert_eq!(&wire[..6], &[2, 0, 0, 0, b'a', b'b']);
        assert!(wire[6..].iter().all(|&b| b == 0), "the bytes past the value are zero");
        assert_eq!(SlotValue::unmarshal(&wire).expect("read back").as_bytes(), b"ab");
    }

    #[test]
    fn a_value_ending_in_zero_bytes_keeps_them() {
        let value = SlotValue::from_bytes(&[7, 0, 0]).expect("three bytes fit");
        let mut wire = [0u8; 4 + SLOT_VALUE_BYTES];
        value.marshal(&mut wire);
        assert_eq!(SlotValue::unmarshal(&wire).expect("read back").as_bytes(), &[7, 0, 0]);
    }

    #[test]
    fn a_value_past_its_room_is_refused_and_a_length_past_it_is_not_trusted() {
        assert_eq!(
            LeaseValue::from_bytes(&[0; LEASE_VALUE_BYTES + 1]),
            Err(ValueTooLong { max: LEASE_VALUE_BYTES, got: LEASE_VALUE_BYTES + 1 })
        );
        let mut wire = [0u8; 4 + SLOT_VALUE_BYTES];
        wire[..4].copy_from_slice(&(SLOT_VALUE_BYTES as u32 + 1).to_le_bytes());
        assert!(matches!(SlotValue::unmarshal(&wire), Err(MarshalError::InvalidEncoding)));
    }
}
