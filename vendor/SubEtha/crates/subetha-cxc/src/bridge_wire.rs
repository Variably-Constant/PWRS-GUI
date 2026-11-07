//! The bridges' wire records: what the TCP, TCP+TLS and QUIC bridges
//! write for each ring item, and the reader that turns the bytes they
//! receive back into items.
//!
//! The wire is a grid of 64-byte records. A plain item or an inline frame
//! is one record: its bytes, zero to 63, then its kind in the last byte.
//! An offset frame is a header record (the payload's length as a
//! little-endian `u32` in its first four bytes, the offset kind in its
//! last) followed by the payload in whole records, the last one zero past
//! the payload's end. A stream's 8-byte item count counts items, a frame
//! as one. The receiving bridge sends an offset frame's payload into its
//! own ring as an offset frame, so the payload lands in that ring's own
//! payload region.

#![cfg(any(feature = "tcp-bridge", feature = "quic-bridge"))]

use crate::adaptive_ring::{AdaptiveRing, KIND_OFFSET_FRAME, PLAIN_ITEM_BYTES};
use crate::frame_ring::LayoutHint;
use crate::shared_ring::RingError;
use crate::spsc_ring::SPSC_PAYLOAD_BYTES;

/// Bytes per record.
pub(crate) const RECORD: usize = SPSC_PAYLOAD_BYTES;

/// Bytes an offset frame's payload of `len` bytes takes on the wire.
fn padded(len: usize) -> usize {
    len.div_ceil(RECORD) * RECORD
}

/// Lay out an offset frame's records in `out`: its header record, then
/// `payload` in whole records. A payload longer than the header's `u32`
/// length is refused with [`RingError::PayloadTooLarge`].
pub(crate) fn frame_records(payload: &[u8], out: &mut Vec<u8>) -> Result<(), RingError> {
    if payload.len() > u32::MAX as usize {
        return Err(RingError::PayloadTooLarge);
    }
    out.clear();
    out.resize(RECORD + padded(payload.len()), 0);
    out[..4].copy_from_slice(&(payload.len() as u32).to_le_bytes());
    out[PLAIN_ITEM_BYTES] = KIND_OFFSET_FRAME;
    out[RECORD..RECORD + payload.len()].copy_from_slice(payload);
    Ok(())
}

/// One item read back off the wire.
pub(crate) enum Incoming<'a> {
    /// A plain item or an inline frame: one record.
    Record(&'a [u8; RECORD]),
    /// An offset frame's header, with its payload's length; the payload
    /// comes next. A caller that cannot take a frame that long stops here,
    /// before the payload is read.
    FrameHeader(usize),
    /// An offset frame's payload.
    Frame(&'a [u8]),
}

/// Turns the bytes a bridge receives, in whatever pieces they arrive, back
/// into items.
#[derive(Default)]
pub(crate) struct Reader {
    /// Bytes of an item not yet whole, or of the item last delivered.
    partial: Vec<u8>,
    /// The length of the offset frame whose payload is being read.
    frame: Option<usize>,
    /// `partial` holds the item last delivered, cleared before the next
    /// one is read.
    delivered: bool,
}

impl Reader {
    /// The next whole item in what earlier calls held back and `data`,
    /// taking from `data` the bytes it uses. `None` once `data` runs out
    /// first, its bytes held for the next call.
    pub(crate) fn next<'r, 'd: 'r>(&'r mut self, data: &mut &'d [u8]) -> Option<Incoming<'r>> {
        if self.delivered {
            self.partial.clear();
            self.delivered = false;
        }
        let want = self.frame.map_or(RECORD, padded);
        let item: &'r [u8] = if self.partial.is_empty() && data.len() >= want {
            let (item, rest) = data.split_at(want);
            *data = rest;
            item
        } else {
            let take = (want - self.partial.len()).min(data.len());
            self.partial.extend_from_slice(&data[..take]);
            *data = &data[take..];
            if self.partial.len() < want {
                return None;
            }
            self.delivered = true;
            &self.partial
        };
        Some(match self.frame.take() {
            Some(len) => Incoming::Frame(&item[..len]),
            None if item[PLAIN_ITEM_BYTES] == KIND_OFFSET_FRAME => {
                let len = u32::from_le_bytes([item[0], item[1], item[2], item[3]]) as usize;
                self.frame = Some(len);
                Incoming::FrameHeader(len)
            }
            None => Incoming::Record(item.try_into().expect("one record")),
        })
    }
}

/// Refuse with [`RingError::PayloadTooLarge`] an offset frame longer than
/// `ring`'s payload region's blocks, which its send would refuse, before
/// its payload is read.
pub(crate) fn frame_fits(ring: &AdaptiveRing, len: usize) -> Result<(), RingError> {
    if len > ring.frame_capacity() {
        return Err(RingError::PayloadTooLarge);
    }
    Ok(())
}

/// Push a record into `ring` as the item its kind names, waiting while
/// the ring is full. Any other refusal is the answer.
pub(crate) async fn push_record(ring: &AdaptiveRing, record: &[u8; RECORD]) -> Result<(), RingError> {
    loop {
        match ring.send_record(0, record) {
            Err(RingError::Full) => tokio::task::yield_now().await,
            done => return done,
        }
    }
}

/// Push an offset frame's payload into `ring` as an offset frame, waiting
/// while the ring or its payload region is full. Any other refusal is the
/// answer.
pub(crate) async fn push_frame(ring: &AdaptiveRing, payload: &[u8]) -> Result<(), RingError> {
    loop {
        match ring.send_frame_as(0, payload, LayoutHint::ForceOffset) {
            Ok(_) => return Ok(()),
            Err(RingError::Full) => tokio::task::yield_now().await,
            Err(e) => return Err(e),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::frame_ring::FrameClass;

    /// Every item `data` holds when the reader is fed `piece` bytes at a
    /// time, as (kind, bytes): 0 for a record, 2 for a frame.
    fn read_in_pieces(data: &[u8], piece: usize) -> Vec<(u8, Vec<u8>)> {
        let mut reader = Reader::default();
        let mut items = Vec::new();
        for chunk in data.chunks(piece) {
            let mut rest = chunk;
            while let Some(item) = reader.next(&mut rest) {
                match item {
                    Incoming::Record(record) => items.push((0, record.to_vec())),
                    Incoming::FrameHeader(_) => {}
                    Incoming::Frame(payload) => items.push((KIND_OFFSET_FRAME, payload.to_vec())),
                }
            }
            assert!(rest.is_empty(), "every byte is taken or held");
        }
        items
    }

    #[test]
    fn the_reader_gives_back_records_and_frames_however_the_bytes_are_split() {
        let mut wire = Vec::new();
        let mut record = [0u8; RECORD];
        record[..3].copy_from_slice(b"one");
        wire.extend_from_slice(&record);
        let payload: Vec<u8> = (0..200u32).map(|i| (i % 251) as u8).collect();
        let mut framed = Vec::new();
        frame_records(&payload, &mut framed).unwrap();
        assert_eq!(framed.len(), RECORD + 256, "a header and the payload in whole records");
        wire.extend_from_slice(&framed);
        frame_records(&[], &mut framed).unwrap();
        wire.extend_from_slice(&framed);
        record[..3].copy_from_slice(b"two");
        wire.extend_from_slice(&record);

        for piece in [1, 7, 63, 64, 65, 100, wire.len()] {
            let items = read_in_pieces(&wire, piece);
            assert_eq!(items.len(), 4, "piece {piece}");
            assert_eq!(&items[0].1[..3], b"one");
            assert_eq!(items[1], (KIND_OFFSET_FRAME, payload.clone()), "piece {piece}");
            assert_eq!(items[2], (KIND_OFFSET_FRAME, Vec::new()), "an empty frame, piece {piece}");
            assert_eq!(&items[3].1[..3], b"two");
        }
    }

    #[test]
    fn a_frame_past_the_receiving_rings_blocks_is_refused_before_its_payload() {
        let ring = AdaptiveRing::create_anon(1, 1, 64).unwrap();
        let capacity = ring.frame_capacity();
        frame_fits(&ring, capacity).unwrap();
        assert!(matches!(frame_fits(&ring, capacity + 1), Err(RingError::PayloadTooLarge)));
    }

    /// The value of a future that finishes without waiting.
    fn at_once<T>(future: impl std::future::Future<Output = T>) -> T {
        let mut future = std::pin::pin!(future);
        let mut context = std::task::Context::from_waker(std::task::Waker::noop());
        match future.as_mut().poll(&mut context) {
            std::task::Poll::Ready(value) => value,
            std::task::Poll::Pending => panic!("the push waited on a ring with room"),
        }
    }

    #[test]
    fn a_pushed_frame_lands_in_the_receiving_rings_own_region() {
        let ring = AdaptiveRing::create_anon(1, 1, 64).unwrap();
        at_once(push_frame(&ring, b"carried")).unwrap();
        let mut out = Vec::new();
        assert_eq!(ring.recv_frame(0, &mut out).unwrap(), FrameClass::Offset);
        assert_eq!(out, b"carried");
    }
}
