//! Object-safe raw interfaces; polling never allocates a boxed packet future.
use std::{
    future::poll_fn,
    io,
    net::SocketAddr,
    task::{Context, Poll},
};

use tokio::io::{AsyncRead, AsyncWrite};

pub use crate::output::CheckedOutput;

/// Common native buffer setup for providers adopting host-owned sockets.
/// Zero retains the OS setting; this neither opens nor protects a socket.
pub fn apply_socket_options(
    socket: &socket2::SockRef<'_>,
    options: &crate::spec::SocketOptions,
) -> io::Result<()> {
    if options.receive_buffer > i32::MAX as u32 || options.send_buffer > i32::MAX as u32 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "socket buffer sizes must fit a positive i32",
        ));
    }
    if options.receive_buffer != 0 {
        socket.set_recv_buffer_size(options.receive_buffer as usize)?;
    }
    if options.send_buffer != 0 {
        socket.set_send_buffer_size(options.send_buffer as usize)?;
    }
    Ok(())
}

/// Any ordinary Tokio byte stream can be a carrier, including an adapter over
/// a host-opened socket. This trait adds no handshake or socket protection.
/// Buffering providers place [`CheckedOutput`] below their protocol stack to
/// enforce Session's operation scope on actual output, including read-driven
/// flushes. Provider output must remain inside its synchronous I/O polls.
pub trait Stream: AsyncRead + AsyncWrite + Send + Unpin + 'static {}
impl<T: AsyncRead + AsyncWrite + Send + Unpin + 'static> Stream for T {}

/// Metadata for one consumed datagram, including an empty or truncated one.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DatagramRead {
    pub source: SocketAddr,
    /// Bytes initialized in the supplied receive buffer.
    pub copied_len: usize,
    /// Original message length, when the operating system reports it. A
    /// truncated macOS datagram has `None`, rather than an invented length.
    pub original_len: Option<usize>,
    pub truncated: bool,
}

/// Socket buffer sizes the kernel actually granted, read back after the
/// request. Linux reports twice the granted bytes (skb bookkeeping) and
/// limits an ordinary request to `net.core.rmem_max`／`wmem_max`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Buffers {
    pub receive: usize,
    pub send: usize,
    /// The requests in bytes (configured, or the provider default).
    pub receive_requested: usize,
    pub send_requested: usize,
}

impl Buffers {
    /// The kernel granted less than requested in either direction: the
    /// ordinary request was limited by `net.core.rmem_max`／`wmem_max`.
    pub fn limited(&self) -> bool {
        !satisfies(self.receive, self.receive_requested)
            || !satisfies(self.send, self.send_requested)
    }
}

/// Whether a socket buffer the kernel reports as `readback` bytes holds a
/// request of `requested` bytes. Linux reports twice what it granted (skb
/// bookkeeping), so a whole grant reads back as twice the request and a
/// request limited to anything less reads back below that; other systems
/// report the grant itself. The one judgement behind the buffer log,
/// `meshway status` and the default raise.
pub fn satisfies(readback: usize, requested: usize) -> bool {
    const REPORTED_PER_GRANTED: usize = if cfg!(target_os = "linux") { 2 } else { 1 };
    readback >= requested.saturating_mul(REPORTED_PER_GRANTED)
}

#[derive(Clone, Copy)]
pub struct DatagramSend<'a> {
    pub payload: &'a [u8],
    pub peer: SocketAddr,
}

/// An endpoint, not an authenticated connection or a pseudo-accept listener.
///
/// Implementors preserve datagram boundaries and report truncation. Polls must
/// be nonblocking and cancellation safe: `Pending` retains no caller buffer.
/// Use at most one concurrent receiver and one concurrent sender per endpoint
/// (a session supplies exactly these). Socket-level size errors remain I/O
/// errors, even when a framing limit is larger than the underlay permits.
pub trait Datagram: Send + Sync + 'static {
    /// At most one data syscall per poll, including fallback paths. Success is
    /// a complete prefix count; Pending retains no buffers and sends no prefix.
    fn poll_send_batch(
        &self,
        cx: &mut Context<'_>,
        packets: &[DatagramSend<'_>],
    ) -> Poll<io::Result<usize>> {
        let Some(packet) = packets.first() else {
            return Poll::Ready(Ok(0));
        };
        self.poll_send_to(cx, packet.payload, packet.peer)
            .map(|result| {
                result.and_then(|size| {
                    if size == packet.payload.len() {
                        Ok(1)
                    } else {
                        Err(io::Error::new(
                            io::ErrorKind::WriteZero,
                            "partial datagram send",
                        ))
                    }
                })
            })
    }

    /// Fill a prefix of initialized buffers; metadata preserves message
    /// boundaries and truncation. At most one data syscall per poll.
    fn poll_recv_batch(
        &self,
        cx: &mut Context<'_>,
        buffers: &mut [Vec<u8>],
    ) -> Poll<io::Result<Vec<DatagramRead>>> {
        let Some(buffer) = buffers.first_mut() else {
            return Poll::Ready(Ok(Vec::new()));
        };
        self.poll_recv_from(cx, buffer)
            .map(|result| result.map(|read| vec![read]))
    }
    fn local_addr(&self) -> io::Result<SocketAddr>;
    fn poll_send_to(
        &self,
        cx: &mut Context<'_>,
        payload: &[u8],
        peer: SocketAddr,
    ) -> Poll<io::Result<usize>>;
    fn poll_recv_from(
        &self,
        cx: &mut Context<'_>,
        buffer: &mut [u8],
    ) -> Poll<io::Result<DatagramRead>>;
    /// The outer path MTU (IP bytes) toward `peer` as this endpoint's own
    /// route currently knows it, including a learned smaller PMTU; `None`
    /// when the endpoint cannot tell. Sends no packet.
    fn path_mtu(&self, _peer: SocketAddr) -> Option<u16> {
        None
    }
    /// The socket buffer sizes the kernel granted this endpoint; `None` when
    /// the endpoint has no operating-system socket of its own to report.
    fn buffers(&self) -> Option<Buffers> {
        None
    }
    /// Let this endpoint also send to `peer` while `lifetime` is current.
    /// Only the endpoint's authentication owner calls this, after verifying
    /// a signed message of one of its channels from `peer` (a roaming peer's
    /// new source). An endpoint that restricts its recipients bounds these
    /// grants; one that does not restrict them needs none.
    fn authorize_source(
        &self,
        _peer: SocketAddr,
        _lifetime: std::sync::Arc<dyn crate::SendLifetime>,
    ) -> io::Result<()> {
        Ok(())
    }
}

impl<D: Datagram + ?Sized> Datagram for Box<D> {
    fn poll_send_batch(
        &self,
        cx: &mut Context<'_>,
        packets: &[DatagramSend<'_>],
    ) -> Poll<io::Result<usize>> {
        (**self).poll_send_batch(cx, packets)
    }
    fn poll_recv_batch(
        &self,
        cx: &mut Context<'_>,
        buffers: &mut [Vec<u8>],
    ) -> Poll<io::Result<Vec<DatagramRead>>> {
        (**self).poll_recv_batch(cx, buffers)
    }
    fn local_addr(&self) -> io::Result<SocketAddr> {
        (**self).local_addr()
    }

    fn poll_send_to(
        &self,
        cx: &mut Context<'_>,
        payload: &[u8],
        peer: SocketAddr,
    ) -> Poll<io::Result<usize>> {
        (**self).poll_send_to(cx, payload, peer)
    }

    fn poll_recv_from(
        &self,
        cx: &mut Context<'_>,
        buffer: &mut [u8],
    ) -> Poll<io::Result<DatagramRead>> {
        (**self).poll_recv_from(cx, buffer)
    }
    fn path_mtu(&self, peer: SocketAddr) -> Option<u16> {
        (**self).path_mtu(peer)
    }
    fn buffers(&self) -> Option<Buffers> {
        (**self).buffers()
    }
    fn authorize_source(
        &self,
        peer: SocketAddr,
        lifetime: std::sync::Arc<dyn crate::SendLifetime>,
    ) -> io::Result<()> {
        (**self).authorize_source(peer, lifetime)
    }
}

/// A shared endpoint: one owner polls I/O, another may read path facts.
impl<D: Datagram + ?Sized> Datagram for std::sync::Arc<D> {
    fn poll_send_batch(
        &self,
        cx: &mut Context<'_>,
        packets: &[DatagramSend<'_>],
    ) -> Poll<io::Result<usize>> {
        (**self).poll_send_batch(cx, packets)
    }
    fn poll_recv_batch(
        &self,
        cx: &mut Context<'_>,
        buffers: &mut [Vec<u8>],
    ) -> Poll<io::Result<Vec<DatagramRead>>> {
        (**self).poll_recv_batch(cx, buffers)
    }
    fn local_addr(&self) -> io::Result<SocketAddr> {
        (**self).local_addr()
    }
    fn poll_send_to(
        &self,
        cx: &mut Context<'_>,
        payload: &[u8],
        peer: SocketAddr,
    ) -> Poll<io::Result<usize>> {
        (**self).poll_send_to(cx, payload, peer)
    }
    fn poll_recv_from(
        &self,
        cx: &mut Context<'_>,
        buffer: &mut [u8],
    ) -> Poll<io::Result<DatagramRead>> {
        (**self).poll_recv_from(cx, buffer)
    }
    fn path_mtu(&self, peer: SocketAddr) -> Option<u16> {
        (**self).path_mtu(peer)
    }
    fn buffers(&self) -> Option<Buffers> {
        (**self).buffers()
    }
    fn authorize_source(
        &self,
        peer: SocketAddr,
        lifetime: std::sync::Arc<dyn crate::SendLifetime>,
    ) -> io::Result<()> {
        (**self).authorize_source(peer, lifetime)
    }
}

/// Raw send, shared by direct users and the framed owner.
pub async fn send_to<D: Datagram + ?Sized>(
    endpoint: &D,
    payload: &[u8],
    peer: SocketAddr,
) -> io::Result<usize> {
    poll_fn(|cx| endpoint.poll_send_to(cx, payload, peer)).await
}

/// Raw receive, without silently converting a truncated datagram into success.
pub async fn recv_from<D: Datagram + ?Sized>(
    endpoint: &D,
    buffer: &mut [u8],
) -> io::Result<DatagramRead> {
    poll_fn(|cx| endpoint.poll_recv_from(cx, buffer)).await
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Linux reads back twice the grant: a 4 MiB request limited to 3 MiB
    /// reads back 6 MiB, above the request yet limited.
    #[test]
    fn a_readback_holds_a_request_only_at_its_reported_size() {
        let mib = 1024 * 1024;
        let buffers = |receive| Buffers {
            receive,
            send: 4 * mib,
            receive_requested: 4 * mib,
            send_requested: 2 * mib,
        };
        if cfg!(target_os = "linux") {
            assert!(buffers(6 * mib).limited());
            assert!(!buffers(8 * mib).limited());
            assert!(!satisfies(4 * mib, 4 * mib));
        } else {
            assert!(!buffers(4 * mib).limited());
            assert!(buffers(3 * mib).limited());
        }
        assert!(buffers(416 * 1024).limited());
    }
}
