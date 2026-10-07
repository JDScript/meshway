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

#[cfg(target_os = "linux")]
mod batch;
#[cfg(test)]
mod tests;

/// Successfully enabled features. mmsg is established on its first real call;
/// `None` means unprobed, not unsupported. No probe emits a network packet.
#[derive(Clone, Copy, Debug, Default)]
pub struct Capabilities {
    pub udp_segment: bool,
    pub udp_gro: bool,
    pub sendmmsg: Option<bool>,
    pub recvmmsg: Option<bool>,
    pub gso_sends: u64,
    pub gro_receives: u64,
    pub mmsg_sends: u64,
    pub mmsg_receives: u64,
}

/// A raw, unconnected UDP endpoint. It can serve many explicit peers; no
/// authenticated accepts, automatic retransmission, or global ordering.
#[derive(Debug)]
pub struct UdpTransport {
    socket: UdpSocket,
    #[cfg(target_os = "linux")]
    batch: Option<batch::State>,
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
        Ok(Self {
            socket,
            #[cfg(target_os = "linux")]
            batch: None,
        })
    }

    /// Opt into Linux GSO/GRO and mmsg, preserving the single-message API.
    /// Unspecified Linux batch buffers request 7 MiB to absorb GSO bursts;
    /// explicit sizes win, and the kernel may clamp the ordinary SO_*BUF request.
    pub fn adopt_batch(socket: UdpSocket, options: &SocketOptions) -> io::Result<Self> {
        let endpoint = Self::adopt(socket, options)?;
        #[cfg(target_os = "linux")]
        {
            let mut endpoint = endpoint;
            const DEFAULT_BUFFER: usize = 7 * 1024 * 1024;
            let reference = SockRef::from(&endpoint.socket);
            if options.receive_buffer == 0 && reference.recv_buffer_size()? < DEFAULT_BUFFER {
                reference.set_recv_buffer_size(DEFAULT_BUFFER)?;
            }
            if options.send_buffer == 0 && reference.send_buffer_size()? < DEFAULT_BUFFER {
                reference.set_send_buffer_size(DEFAULT_BUFFER)?;
            }
            endpoint.batch = Some(batch::State::new(&endpoint.socket)?);
            Ok(endpoint)
        }
        #[cfg(not(target_os = "linux"))]
        {
            Ok(endpoint)
        }
    }

    pub fn capabilities(&self) -> Capabilities {
        #[cfg(target_os = "linux")]
        if let Some(state) = &self.batch {
            return state.capabilities();
        }
        Capabilities::default()
    }

    /// Read-only observation handle; it cannot retain or use the descriptor.
    pub fn capability_report(&self) -> impl Fn() -> Capabilities + Send + Sync + 'static {
        #[cfg(target_os = "linux")]
        let shared = self.batch.as_ref().map(|state| state.caps.clone());
        #[cfg(not(target_os = "linux"))]
        let shared: Option<std::sync::Arc<std::sync::Mutex<Capabilities>>> = None;
        move || {
            shared.as_ref().map_or_else(Capabilities::default, |caps| {
                *caps.lock().expect("UDP capability lock")
            })
        }
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
    fn poll_send_batch(
        &self,
        cx: &mut Context<'_>,
        packets: &[raw::DatagramSend<'_>],
    ) -> Poll<io::Result<usize>> {
        #[cfg(target_os = "linux")]
        if let Some(state) = &self.batch {
            return state.poll_send(&self.socket, cx, packets);
        }
        let Some(packet) = packets.first() else {
            return Poll::Ready(Ok(0));
        };
        self.poll_send_to(cx, packet.payload, packet.peer)
            .map(|result| {
                result.and_then(|n| {
                    if n == packet.payload.len() {
                        Ok(1)
                    } else {
                        Err(io::Error::new(
                            io::ErrorKind::WriteZero,
                            "partial UDP write",
                        ))
                    }
                })
            })
    }

    fn poll_recv_batch(
        &self,
        cx: &mut Context<'_>,
        buffers: &mut [Vec<u8>],
    ) -> Poll<io::Result<Vec<DatagramRead>>> {
        #[cfg(target_os = "linux")]
        if let Some(state) = &self.batch {
            return state.poll_recv(&self.socket, cx, buffers);
        }
        let Some(buffer) = buffers.first_mut() else {
            return Poll::Ready(Ok(Vec::new()));
        };
        self.poll_recv_from(cx, buffer)
            .map(|result| result.map(|read| vec![read]))
    }
    fn local_addr(&self) -> io::Result<SocketAddr> {
        self.local_addr()
    }

    fn poll_send_to(
        &self,
        cx: &mut Context<'_>,
        payload: &[u8],
        peer: SocketAddr,
    ) -> Poll<io::Result<usize>> {
        static PROFILE: meshway_logging::profile::Metric =
            meshway_logging::profile::Metric::new("udp.send.poll");
        let _profile = PROFILE.enter();
        self.socket.poll_send_to(cx, payload, peer)
    }

    fn poll_recv_from(
        &self,
        cx: &mut Context<'_>,
        buffer: &mut [u8],
    ) -> Poll<io::Result<DatagramRead>> {
        #[cfg(target_os = "linux")]
        if let Some(state) = &self.batch {
            return state.poll_recv_one(&self.socket, cx, buffer);
        }
        static PROFILE: meshway_logging::profile::Metric =
            meshway_logging::profile::Metric::new("udp.recv.poll");
        let _profile = PROFILE.enter();
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
