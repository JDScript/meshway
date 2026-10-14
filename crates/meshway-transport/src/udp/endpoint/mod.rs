use std::{
    io::{self, IoSliceMut},
    net::SocketAddr,
    os::fd::{AsFd, BorrowedFd},
    task::{Context, Poll, ready},
};

use crate::raw::Buffers;
use crate::spec::SocketOptions;
use crate::{
    Limits, Session,
    raw::{self, Datagram, DatagramRead},
};
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
    /// GSO batches the path MTU rejected; GSO stayed enabled for them.
    pub gso_size_rejections: u64,
}

/// Default Linux batch-endpoint receive buffer request: above the 3 MB
/// bandwidth-delay product of a 400 Mbit/s channel at 60 ms RTT, so a window
/// arriving at once or a briefly descheduled receiver (a 2 vCPU host busy
/// with crypto) does not overflow it; larger only adds queueing delay when
/// the receiver is CPU-bound.
pub const DEFAULT_RECEIVE_BUFFER: usize = 4 * 1024 * 1024;
/// Default Linux batch-endpoint send buffer request. A send buffer only
/// holds datagrams the host has not transmitted yet; a larger one adds
/// queueing delay, not throughput.
pub const DEFAULT_SEND_BUFFER: usize = 2 * 1024 * 1024;

fn read_buffers(
    socket: &UdpSocket,
    receive_requested: usize,
    send_requested: usize,
) -> io::Result<Buffers> {
    let reference = SockRef::from(socket);
    Ok(Buffers {
        receive: reference.recv_buffer_size()?,
        send: reference.send_buffer_size()?,
        receive_requested,
        send_requested,
    })
}

/// A raw, unconnected UDP endpoint. It can serve many explicit peers; no
/// authenticated accepts, automatic retransmission, or global ordering.
#[derive(Debug)]
pub struct UdpTransport {
    socket: UdpSocket,
    buffers: Buffers,
    #[cfg(target_os = "linux")]
    batch: Option<batch::State>,
}

/// Request `bytes` for one direction: first the privileged request that is
/// not limited by the sysctl maximum, then, without the privilege, the
/// ordinary one the kernel limits to `net.core.rmem_max`／`wmem_max`.
#[cfg(target_os = "linux")]
fn request_buffer(socket: &UdpSocket, receive: bool, bytes: usize) -> io::Result<()> {
    use nix::sys::socket::{setsockopt, sockopt};
    let forced = if receive {
        setsockopt(socket, sockopt::RcvBufForce, &bytes)
    } else {
        setsockopt(socket, sockopt::SndBufForce, &bytes)
    };
    match forced {
        Ok(()) => Ok(()),
        Err(nix::errno::Errno::EPERM) => {
            let reference = SockRef::from(socket);
            if receive {
                reference.set_recv_buffer_size(bytes)
            } else {
                reference.set_send_buffer_size(bytes)
            }
        }
        Err(error) => Err(error.into()),
    }
}

// The host's protection wrapper borrows the actual adopted descriptor before
// each I/O poll. This does not duplicate or transfer socket ownership.
impl AsFd for UdpTransport {
    fn as_fd(&self) -> BorrowedFd<'_> {
        self.socket.as_fd()
    }
}

/// Every outer datagram carries DF (IPv6: never fragmented locally). A packet
/// larger than the route's path MTU fails its send with `EMSGSIZE` instead of
/// leaving as fragments a middlebox may drop; the kernel's route learns a
/// smaller PMTU from ICMP and [`connected_path_mtu`] reads it.
#[cfg(target_os = "linux")]
fn forbid_fragmentation(socket: &UdpSocket, ipv4: bool) -> io::Result<()> {
    use rustix::net::sockopt;
    if ipv4 {
        sockopt::set_ip_mtu_discover(socket, sockopt::Ipv4PathMtuDiscovery::DO)?;
    } else {
        sockopt::set_ipv6_mtu_discover(socket, sockopt::Ipv6PathMtuDiscovery::DO)?;
    }
    Ok(())
}

/// UDP payload a 1500-byte path carries in one IP packet toward `peer`.
#[cfg(target_os = "linux")]
pub(super) fn whole_on_ethernet(peer: SocketAddr) -> usize {
    match peer {
        SocketAddr::V6(peer) if peer.ip().to_ipv4_mapped().is_none() => 1500 - 48,
        _ => 1500 - 28,
    }
}

/// One datagram DF refused (larger than the route MTU) that no 1500-byte
/// path carries whole either (an exit DNS message of up to 4 KiB, say): DF
/// could only lose it, so for this send alone the kernel may fragment it
/// (`IP_PMTUDISC_WANT`). A Mesh data frame of a 1500-byte path's size never
/// takes this path: it fails instead of leaving as fragments.
#[cfg(target_os = "linux")]
pub(super) fn send_fragmentable(
    socket: &UdpSocket,
    payload: &[u8],
    peer: SocketAddr,
) -> io::Result<usize> {
    use rustix::net::sockopt;
    let ipv4 = socket.local_addr()?.is_ipv4();
    let discovery = |strict: bool| {
        if ipv4 {
            sockopt::set_ip_mtu_discover(
                socket,
                if strict {
                    sockopt::Ipv4PathMtuDiscovery::DO
                } else {
                    sockopt::Ipv4PathMtuDiscovery::WANT
                },
            )
        } else {
            sockopt::set_ipv6_mtu_discover(
                socket,
                if strict {
                    sockopt::Ipv6PathMtuDiscovery::DO
                } else {
                    sockopt::Ipv6PathMtuDiscovery::WANT
                },
            )
        }
    };
    discovery(false)?;
    // A raw nonblocking send: the caller's try_io owns readiness.
    let sent = SockRef::from(socket).send_to(payload, &peer.into());
    // DF again before any other send, whatever this one did.
    discovery(true)?;
    sent
}

/// The route MTU of a connected UDP socket (`IP_MTU`/`IPV6_MTU`): the kernel
/// route toward its peer, including a PMTU learned from ICMP. The host opens
/// and protects that socket; connecting a UDP socket sends nothing.
#[cfg(target_os = "linux")]
pub fn connected_path_mtu(socket: &impl AsFd, ipv4: bool) -> io::Result<u16> {
    let mtu = if ipv4 {
        rustix::net::sockopt::ip_mtu(socket)?
    } else {
        rustix::net::sockopt::ipv6_mtu(socket)?
    };
    u16::try_from(mtu).map_err(|_| io::Error::from(io::ErrorKind::InvalidData))
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
        let local = socket.local_addr()?;
        let reference = SockRef::from(&socket);
        raw::apply_socket_options(&reference, options)?;
        #[cfg(target_os = "linux")]
        forbid_fragmentation(&socket, local.is_ipv4())?;
        #[cfg(not(target_os = "linux"))]
        let _ = local;
        let buffers = read_buffers(
            &socket,
            options.receive_buffer as usize,
            options.send_buffer as usize,
        )?;
        Ok(Self {
            socket,
            buffers,
            #[cfg(target_os = "linux")]
            batch: None,
        })
    }

    /// Opt into Linux GSO/GRO and mmsg, preserving the single-message API.
    /// Unspecified Linux batch buffers request [`DEFAULT_RECEIVE_BUFFER`] and
    /// [`DEFAULT_SEND_BUFFER`] and never lower a larger existing buffer;
    /// explicit sizes are requested as configured. Each request is made
    /// privileged first (`SO_RCVBUFFORCE`／`SO_SNDBUFFORCE`, `CAP_NET_ADMIN`),
    /// so a Node daemon is not limited by `net.core.rmem_max`／`wmem_max`;
    /// without the privilege the kernel limits it. [`Self::buffers`] reports
    /// what was granted.
    pub fn adopt_batch(socket: UdpSocket, options: &SocketOptions) -> io::Result<Self> {
        let endpoint = Self::adopt(socket, options)?;
        #[cfg(target_os = "linux")]
        {
            let mut endpoint = endpoint;
            let reference = SockRef::from(&endpoint.socket);
            let receive = match options.receive_buffer {
                0 => DEFAULT_RECEIVE_BUFFER,
                bytes => bytes as usize,
            };
            let send = match options.send_buffer {
                0 => DEFAULT_SEND_BUFFER,
                bytes => bytes as usize,
            };
            if options.receive_buffer != 0
                || !crate::raw::satisfies(reference.recv_buffer_size()?, receive)
            {
                request_buffer(&endpoint.socket, true, receive)?;
            }
            if options.send_buffer != 0
                || !crate::raw::satisfies(reference.send_buffer_size()?, send)
            {
                request_buffer(&endpoint.socket, false, send)?;
            }
            endpoint.buffers = read_buffers(&endpoint.socket, receive, send)?;
            endpoint.batch = Some(batch::State::new(&endpoint.socket)?);
            Ok(endpoint)
        }
        #[cfg(not(target_os = "linux"))]
        {
            Ok(endpoint)
        }
    }

    /// Buffer sizes read back once the endpoint was adopted.
    pub fn buffers(&self) -> Buffers {
        self.buffers
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
        static PROFILE: meshway_logging::profile::Metric =
            meshway_logging::profile::Metric::new("udp.batch.send.poll");
        let _profile = PROFILE.enter();
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
    fn buffers(&self) -> Option<Buffers> {
        Some(self.buffers)
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
        let result = ready!(self.socket.poll_send_to(cx, payload, peer));
        #[cfg(target_os = "linux")]
        if result
            .as_ref()
            .is_err_and(|error| error.raw_os_error() == Some(nix::errno::Errno::EMSGSIZE as i32))
            && payload.len() > whole_on_ethernet(peer)
        {
            loop {
                ready!(self.socket.poll_send_ready(cx))?;
                match self.socket.try_io(Interest::WRITABLE, || {
                    send_fragmentable(&self.socket, payload, peer)
                }) {
                    Err(error) if error.kind() == io::ErrorKind::WouldBlock => continue,
                    result => return Poll::Ready(result),
                }
            }
        }
        Poll::Ready(result)
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
