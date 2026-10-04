//! Object-safe raw interfaces; polling never allocates a boxed packet future.
use std::{
    future::poll_fn,
    io,
    net::SocketAddr,
    task::{Context, Poll},
};

use tokio::io::{AsyncRead, AsyncWrite};

/// Common native buffer setup for providers adopting host-owned sockets.
/// Zero retains the OS setting; this neither opens nor protects a socket.
pub fn apply_socket_options(
    socket: &socket2::SockRef<'_>,
    options: &meshway_transport_spec::SocketOptions,
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

/// An endpoint, not an authenticated connection or a pseudo-accept listener.
///
/// Implementors preserve datagram boundaries and report truncation. Polls must
/// be nonblocking and cancellation safe: `Pending` retains no caller buffer.
/// Use at most one concurrent receiver and one concurrent sender per endpoint
/// (a session supplies exactly these). Socket-level size errors remain I/O
/// errors, even when a framing limit is larger than the underlay permits.
pub trait Datagram: Send + Sync + 'static {
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
}

impl<D: Datagram + ?Sized> Datagram for Box<D> {
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
