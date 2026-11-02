//! `TcpBridge`: cross-host substrate primitive over plain TCP.
//!
//! Where `QuicBridge` provides encrypted, multiplexed, congestion-
//! controlled transport, `TcpBridge` is the trusted-network primitive
//! for callers that want maximum throughput without QUIC's
//! per-packet TLS overhead. Cross-platform via
//! `tokio::net::TcpStream`.
//!
//! # Data path: burst-batched egress, chunked ingress
//!
//! A per-slot socket write (one `write_all` await per 64-byte item)
//! serializes the whole bridge on syscall + reactor latency -
//! microseconds per item against a wire that moves the same bytes
//! in nanoseconds. The client therefore burst-drains the ring:
//! every already-available slot (up to [`EGRESS_BATCH_SLOTS`]) is
//! copied into one contiguous buffer and shipped with a single
//! write. A lone item still ships immediately - batching never
//! waits for items that have not arrived - so latency-sensitive
//! request/response traffic is not penalized. `TCP_NODELAY` is set
//! on both ends for the same reason; under a saturating stream the
//! kernel still fills segments to MSS from the batched writes.
//!
//! The server mirrors this with chunked reads: `read()` returns
//! whatever bytes the wire has (never blocking for a full batch),
//! complete 64-byte slots are pushed into the consumer ring, and a
//! partial slot carries over to the next read.
//!
//! Items cross as 64-byte records: a plain item or an inline frame is
//! one record carrying its kind in its last byte, and an offset frame
//! goes as its payload (a header record holding its length, then the
//! payload in whole records), which the server sends into its own ring as
//! an offset frame in that ring's own payload region. A receiving ring
//! that is full makes the server wait; any other refusal ends it with
//! that refusal.

#![cfg(feature = "tcp-bridge")]

use std::net::SocketAddr;
use std::sync::Arc;

use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

use crate::adaptive_ring::{AdaptiveRing, Took};
use crate::bridge_wire::{self, Incoming, Reader, RECORD};
use crate::shared_ring::RingError;

/// Slots per batched egress write (16 KiB of payload per syscall at
/// the 64-byte slot size).
pub const EGRESS_BATCH_SLOTS: usize = 256;

/// Ingress socket-read buffer in bytes.
const INGRESS_BUF_BYTES: usize = 64 * 1024;

/// The bridge's error for a ring's refusal, saying which ring and what.
fn refused(what: &str, e: RingError) -> TcpBridgeError {
    std::io::Error::other(format!("{what}: {e}")).into()
}

// ===================================================================
// Stream-generic data path: the burst-batched egress + chunked
// ingress loops touch only the AsyncRead/AsyncWrite trait surface, so
// they run identically over a plain `TcpStream` and over a
// `tokio_rustls::TlsStream<TcpStream>`. The TCP and TCP+TLS bridges
// therefore share one wire protocol - the only difference between
// them is the record layer the bytes pass through, which is the
// fairest possible TCP-vs-TLS comparison.
// ===================================================================

/// Ship `n_items` ring slots over any async stream, framed by an
/// 8-byte big-endian item count. Burst-batches every already-available
/// slot (up to [`EGRESS_BATCH_SLOTS`]) into one `write_all`; a lone
/// item ships immediately.
pub(crate) async fn ship_stream<S: AsyncRead + AsyncWrite + Unpin>(
    stream: &mut S,
    producer_ring: &AdaptiveRing,
    n_items: u64,
) -> Result<(), TcpBridgeError> {
    stream.write_all(&n_items.to_be_bytes()).await?;

    let mut batch = vec![0u8; EGRESS_BATCH_SLOTS * RECORD];
    let mut frame = Vec::new();
    let mut framed = Vec::new();
    let mut shipped: u64 = 0;
    while shipped < n_items {
        let budget = EGRESS_BATCH_SLOTS.min((n_items - shipped) as usize);
        let mut filled = 0usize;
        let mut frame_taken = false;
        while filled < budget {
            let dst = <&mut [u8; RECORD]>::try_from(&mut batch[filled * RECORD..(filled + 1) * RECORD])
                .expect("one record");
            match producer_ring.take_record(0, dst, &mut frame) {
                Ok(Took::Record) => filled += 1,
                Ok(Took::Frame) => {
                    frame_taken = true;
                    break;
                }
                // Nothing queued right now: ship what is filled.
                Err(RingError::Empty) => break,
                // The ring itself refused the pop; the bridge cannot
                // continue past it and says which error ended it.
                Err(e) => return Err(refused("the producer ring refused a pop", e)),
            }
        }
        if filled > 0 {
            stream.write_all(&batch[..filled * RECORD]).await?;
            shipped += filled as u64;
        }
        if frame_taken {
            bridge_wire::frame_records(&frame, &mut framed)
                .map_err(|e| refused("a frame too long for the wire", e))?;
            stream.write_all(&framed).await?;
            shipped += 1;
        } else if filled == 0 {
            tokio::task::yield_now().await;
        }
    }
    // Flush before the half-close so any plaintext still buffered in the
    // record layer is encrypted and written to the socket (a no-op for a
    // plain TcpStream; forces the final TLS record out).
    stream.flush().await?;
    stream.shutdown().await?;
    // Barrier: wait for the receiver to finish before returning. The
    // receiver closes its side once it has drained all `total` bytes,
    // which surfaces here as EOF (or a reset, equally final). Without
    // this, the client process can exit and RST the socket while the
    // receiver - slower for TLS, which must decrypt - is still draining
    // the last records, surfacing as a spurious connection-reset error.
    // Any other read error is the socket failing before the receiver
    // finished, and is the caller's to hear.
    let mut sink = [0u8; 64];
    loop {
        match stream.read(&mut sink).await {
            Ok(0) => break,
            Ok(_) => continue,
            Err(e)
                if matches!(
                    e.kind(),
                    std::io::ErrorKind::ConnectionReset
                        | std::io::ErrorKind::ConnectionAborted
                        | std::io::ErrorKind::BrokenPipe
                        | std::io::ErrorKind::UnexpectedEof
                ) =>
            {
                break;
            }
            Err(e) => return Err(e.into()),
        }
    }
    Ok(())
}

/// Drain a framed payload stream off any async-readable stream into
/// the consumer ring. Reads the 8-byte item-count header, then turns
/// each read into whole items (a partial one carries to the next read).
/// Returns the framed item count.
pub(crate) async fn drain_stream<R: AsyncRead + Unpin>(
    stream: &mut R,
    consumer_ring: &AdaptiveRing,
) -> Result<u64, TcpBridgeError> {
    let mut header = [0u8; 8];
    stream.read_exact(&mut header).await?;
    let total: u64 = u64::from_be_bytes(header);

    let mut buf = vec![0u8; INGRESS_BUF_BYTES];
    let mut reader = Reader::default();
    let mut received: u64 = 0;
    while received < total {
        let n = stream.read(&mut buf).await?;
        if n == 0 {
            return Err(TcpBridgeError::Closed);
        }
        let mut data: &[u8] = &buf[..n];
        while received < total {
            match reader.next(&mut data) {
                None => break,
                Some(Incoming::Record(record)) => {
                    bridge_wire::push_record(consumer_ring, record)
                        .await
                        .map_err(|e| refused("the consumer ring refused an item", e))?;
                    received += 1;
                }
                Some(Incoming::FrameHeader(len)) => bridge_wire::frame_fits(consumer_ring, len)
                    .map_err(|e| refused(&format!("the consumer ring cannot take a {len}-byte frame"), e))?,
                Some(Incoming::Frame(payload)) => {
                    bridge_wire::push_frame(consumer_ring, payload)
                        .await
                        .map_err(|e| refused("the consumer ring refused a frame", e))?;
                    received += 1;
                }
            }
        }
    }
    Ok(total)
}

/// Errors the TCP bridge halves can return.
#[derive(Debug)]
pub enum TcpBridgeError {
    Io(std::io::Error),
    Closed,
}

impl std::fmt::Display for TcpBridgeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(e) => write!(f, "io: {e}"),
            Self::Closed => write!(f, "connection closed"),
        }
    }
}

impl std::error::Error for TcpBridgeError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(e) => Some(e),
            Self::Closed => None,
        }
    }
}

impl From<std::io::Error> for TcpBridgeError {
    fn from(e: std::io::Error) -> Self { Self::Io(e) }
}

/// Client half: pulls bytes from a local producer ring and ships
/// them across a TCP connection.
pub struct TcpBridgeClient {
    producer_ring: Arc<AdaptiveRing>,
    server_addr: SocketAddr,
}

impl TcpBridgeClient {
    /// Takes the substrate's default ring type (AdaptiveRing); the
    /// caller registers producer/consumer ids on it first. The
    /// bridge uses producer_id 0.
    pub fn new(producer_ring: Arc<AdaptiveRing>, server_addr: SocketAddr) -> Self {
        Self { producer_ring, server_addr }
    }

    /// Connect + ship `n_items` slots across the TCP connection.
    /// Burst-batched egress: every already-available slot (up to
    /// [`EGRESS_BATCH_SLOTS`]) goes out in one socket write; a lone
    /// item ships immediately.
    pub async fn run(&self, n_items: u64) -> Result<(), TcpBridgeError> {
        let stream = TcpStream::connect(self.server_addr).await?;
        // Nagle off: the bridge ferries latency-sensitive slots;
        // batched writes keep segments MSS-filled under load.
        stream.set_nodelay(true)?;
        #[cfg(target_os = "linux")]
        crate::net_tune::tune_tcp_socket(std::os::fd::AsRawFd::as_raw_fd(&stream));
        let mut stream = stream;
        ship_stream(&mut stream, &self.producer_ring, n_items).await
    }
}

/// Server half: accepts one incoming connection + pushes received
/// bytes into a local consumer ring.
pub struct TcpBridgeServer {
    consumer_ring: Arc<AdaptiveRing>,
    listener: TcpListener,
}

impl TcpBridgeServer {
    pub async fn bind(
        consumer_ring: Arc<AdaptiveRing>,
        addr: SocketAddr,
    ) -> Result<Self, TcpBridgeError> {
        let listener = TcpListener::bind(addr).await?;
        Ok(Self { consumer_ring, listener })
    }

    pub fn local_addr(&self) -> Result<SocketAddr, std::io::Error> {
        self.listener.local_addr()
    }

    /// Accept one connection, read its framed payload stream, push
    /// into the consumer ring. Returns the count of items received.
    ///
    /// Chunked ingress: each socket `read` takes whatever bytes the
    /// wire has - it never waits for a full batch, so a lone item
    /// flows through with no added latency - and complete slots are
    /// pushed as they assemble; a partial slot carries to the next
    /// read.
    pub async fn accept_one(&self) -> Result<u64, TcpBridgeError> {
        let (stream, _) = self.listener.accept().await?;
        stream.set_nodelay(true)?;
        #[cfg(target_os = "linux")]
        crate::net_tune::tune_tcp_socket(std::os::fd::AsRawFd::as_raw_fd(&stream));
        let mut stream = stream;
        drain_stream(&mut stream, &self.consumer_ring).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adaptive_ring::{RingItem, PLAIN_ITEM_BYTES};
    use crate::frame_ring::{FrameClass, LayoutHint};

    #[tokio::test]
    async fn plain_items_inline_frames_and_offset_frames_cross_with_their_kinds() {
        let from = Arc::new(AdaptiveRing::create_anon(1, 1, 64).unwrap());
        let to = Arc::new(AdaptiveRing::create_anon(1, 1, 64).unwrap());
        let big: Vec<u8> = (0..5000u32).map(|i| (i % 251) as u8).collect();
        from.try_send(0, &[7u8; PLAIN_ITEM_BYTES]).unwrap();
        from.send_frame_as(0, b"inline", LayoutHint::Auto).unwrap();
        from.send_frame_as(0, &big, LayoutHint::Auto).unwrap();
        from.send_frame_as(0, b"short but offset", LayoutHint::ForceOffset).unwrap();
        from.try_send(0, b"last").unwrap();

        let server = TcpBridgeServer::bind(Arc::clone(&to), "127.0.0.1:0".parse().unwrap()).await.unwrap();
        let addr = server.local_addr().unwrap();
        let accepting = tokio::spawn(async move { server.accept_one().await });
        TcpBridgeClient::new(Arc::clone(&from), addr).run(5).await.unwrap();
        assert_eq!(accepting.await.unwrap().unwrap(), 5);

        let mut out = Vec::new();
        assert_eq!(to.recv_any(0, &mut out).unwrap(), RingItem::Plain);
        assert_eq!(out, [7u8; PLAIN_ITEM_BYTES]);
        assert_eq!(to.recv_any(0, &mut out).unwrap(), RingItem::Frame(FrameClass::Inline));
        assert_eq!(out, b"inline");
        assert_eq!(to.recv_any(0, &mut out).unwrap(), RingItem::Frame(FrameClass::Offset));
        assert_eq!(out, big, "an offset frame's payload crosses into the receiving ring's region");
        assert_eq!(to.recv_any(0, &mut out).unwrap(), RingItem::Frame(FrameClass::Offset));
        assert_eq!(out, b"short but offset");
        assert_eq!(to.recv_any(0, &mut out).unwrap(), RingItem::Plain);
        assert_eq!(&out[..4], b"last");
        assert!(matches!(to.recv_any(0, &mut out), Err(RingError::Empty)));
    }
}
