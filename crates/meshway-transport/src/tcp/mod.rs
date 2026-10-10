//! Native TCP adoption. Opening, routing/protecting, and authorizing sockets
//! belongs to the host underlay/connectivity layers.
use std::{
    io,
    net::SocketAddr,
    pin::Pin,
    task::{Context, Poll},
};

use crate::spec::{SocketOptions, TcpOptions};
use crate::{Limits, Session};
use socket2::SockRef;
#[cfg(any(target_os = "linux", target_os = "macos"))]
use socket2::TcpKeepalive;
use tokio::{
    io::{AsyncRead, AsyncWrite, ReadBuf},
    net::TcpStream,
};

#[cfg(test)]
mod tests;

/// Raw Tokio stream, used unchanged by direct callers and framed sessions.
#[derive(Debug)]
pub struct TcpTransport {
    stream: TcpStream,
}

impl TcpTransport {
    /// Takes ownership of an already-connected socket and applies the actual
    /// declared options. A zero buffer size retains the OS setting. Invalid or
    /// unsupported settings fail adoption; there is no TLS or implicit dial.
    pub fn adopt(stream: TcpStream, socket: &SocketOptions, tcp: &TcpOptions) -> io::Result<Self> {
        stream.peer_addr()?;
        let reference = SockRef::from(&stream);
        crate::raw::apply_socket_options(&reference, socket)?;
        stream.set_nodelay(tcp.no_delay)?;
        reference.set_keepalive(tcp.keep_alive.enabled)?;
        if tcp.keep_alive.enabled {
            let keepalive = &tcp.keep_alive;
            // Native TCP keepalive options have whole-second resolution. Do
            // not silently truncate a declared subsecond duration to zero.
            for duration in [keepalive.idle.as_std(), keepalive.interval.as_std()] {
                if duration.subsec_nanos() != 0
                    || duration.as_secs() == 0
                    || duration.as_secs() > i32::MAX as u64
                {
                    return Err(invalid("TCP keepalive durations need positive i32 seconds"));
                }
            }
            if keepalive.count == 0 || keepalive.count > i32::MAX as u32 {
                return Err(invalid("TCP keepalive count needs a positive i32"));
            }
            #[cfg(any(target_os = "linux", target_os = "macos"))]
            reference.set_tcp_keepalive(
                &TcpKeepalive::new()
                    .with_time(keepalive.idle.as_std())
                    .with_interval(keepalive.interval.as_std())
                    .with_retries(keepalive.count),
            )?;
            #[cfg(not(any(target_os = "linux", target_os = "macos")))]
            return Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "declared TCP keepalive is implemented on Linux/macOS only",
            ));
        }
        Ok(Self { stream })
    }

    pub fn local_addr(&self) -> io::Result<SocketAddr> {
        self.stream.local_addr()
    }

    pub fn peer_addr(&self) -> io::Result<SocketAddr> {
        self.stream.peer_addr()
    }

    pub fn into_inner(self) -> TcpStream {
        self.stream
    }

    pub fn into_session(self, limits: Limits) -> io::Result<Session> {
        let peer = self.peer_addr()?;
        Ok(Session::stream(self, peer, limits))
    }
}

fn invalid(message: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, message)
}

impl AsyncRead for TcpTransport {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buffer: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        Pin::new(&mut self.get_mut().stream).poll_read(cx, buffer)
    }
}

impl AsyncWrite for TcpTransport {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buffer: &[u8],
    ) -> Poll<io::Result<usize>> {
        Pin::new(&mut self.get_mut().stream).poll_write(cx, buffer)
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.get_mut().stream).poll_flush(cx)
    }

    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.get_mut().stream).poll_shutdown(cx)
    }

    fn is_write_vectored(&self) -> bool {
        self.stream.is_write_vectored()
    }

    fn poll_write_vectored(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buffers: &[io::IoSlice<'_>],
    ) -> Poll<io::Result<usize>> {
        Pin::new(&mut self.get_mut().stream).poll_write_vectored(cx, buffers)
    }
}
