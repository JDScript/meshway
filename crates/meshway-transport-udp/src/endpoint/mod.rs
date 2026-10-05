use std::{
    io::{self, IoSliceMut},
    net::SocketAddr,
    os::fd::{AsFd, BorrowedFd},
    task::{Context, Poll, ready},
};

use meshway_transport::{
    Limits, Session,
    raw::{self, Datagram, DatagramRead},
};
use meshway_transport_spec::SocketOptions;
use rustix::net::{RecvAncillaryBuffer, RecvFlags, ReturnFlags, recvmsg};
use socket2::SockRef;
use tokio::{io::Interest, net::UdpSocket};

#[cfg(test)]
mod tests;

/// A raw, unconnected UDP endpoint. It can serve many explicit peers; no
/// authenticated accepts, automatic retransmission, or global ordering.
#[derive(Debug)]
pub struct UdpTransport {
    socket: UdpSocket,
}

// The host's protection wrapper borrows the actual adopted descriptor before
// each I/O poll. This does not duplicate or transfer socket ownership.
impl AsFd for UdpTransport {
    fn as_fd(&self) -> BorrowedFd<'_> {
        self.socket.as_fd()
    }
}

impl UdpTransport {
    /// Adopts an already-bound, unconnected Tokio socket. Connected UDP is
    /// rejected because its kernel peer filtering would hide endpoint source
    /// semantics. Zero buffer sizes retain the OS values.
    pub fn adopt(socket: UdpSocket, options: &SocketOptions) -> io::Result<Self> {
        if socket.peer_addr().is_ok() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "UDP endpoint requires an unconnected socket",
            ));
        }
        socket.local_addr()?;
        let reference = SockRef::from(&socket);
        raw::apply_socket_options(&reference, options)?;
        Ok(Self { socket })
    }

    pub fn local_addr(&self) -> io::Result<SocketAddr> {
        self.socket.local_addr()
    }

    pub fn into_inner(self) -> UdpSocket {
        self.socket
    }

    pub fn into_session(self, limits: Limits) -> Session {
        Session::datagram(self, limits)
    }

    pub async fn send_to(&self, payload: &[u8], peer: SocketAddr) -> io::Result<usize> {
        raw::send_to(self, payload, peer).await
    }

    pub async fn recv_from(&self, buffer: &mut [u8]) -> io::Result<DatagramRead> {
        raw::recv_from(self, buffer).await
    }

    fn try_recv(&self, buffer: &mut [u8]) -> io::Result<DatagramRead> {
        let mut slices = [IoSliceMut::new(buffer)];
        let mut ancillary = RecvAncillaryBuffer::new(&mut []);
        // Linux can return the original length with MSG_TRUNC as an input
        // flag. macOS reliably exposes truncation in msg_flags, but not the
        // original length; don't infer it from the copied byte count.
        #[cfg(target_os = "linux")]
        let flags = RecvFlags::DONTWAIT | RecvFlags::TRUNC;
        #[cfg(target_os = "macos")]
        let flags = RecvFlags::DONTWAIT;
        let message = recvmsg(&self.socket, &mut slices, &mut ancillary, flags)?;
        let source = message
            .address
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "missing UDP source"))?
            .try_into()
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "non-IP UDP source"))?;
        let truncated = message.flags.contains(ReturnFlags::TRUNC);
        let copied_len = message.bytes.min(buffer.len());
        #[cfg(target_os = "linux")]
        let original_len = Some(message.bytes);
        #[cfg(target_os = "macos")]
        let original_len = (!truncated).then_some(message.bytes);
        Ok(DatagramRead {
            source,
            copied_len,
            original_len,
            truncated,
        })
    }
}

impl Datagram for UdpTransport {
    fn local_addr(&self) -> io::Result<SocketAddr> {
        self.local_addr()
    }

    fn poll_send_to(
        &self,
        cx: &mut Context<'_>,
        payload: &[u8],
        peer: SocketAddr,
    ) -> Poll<io::Result<usize>> {
        self.socket.poll_send_to(cx, payload, peer)
    }

    fn poll_recv_from(
        &self,
        cx: &mut Context<'_>,
        buffer: &mut [u8],
    ) -> Poll<io::Result<DatagramRead>> {
        loop {
            ready!(self.socket.poll_recv_ready(cx))?;
            // try_io clears Tokio's readiness on EWOULDBLOCK. Calling a raw
            // recvmsg without this integration can spin on stale readiness.
            match self
                .socket
                .try_io(Interest::READABLE, || self.try_recv(buffer))
            {
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => continue,
                result => return Poll::Ready(result),
            }
        }
    }
}
