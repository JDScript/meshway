use super::Capabilities;
use meshway_transport::raw::{DatagramRead, DatagramSend};
use nix::sys::socket::{
    self, ControlMessage, ControlMessageOwned, MsgFlags, MultiHeaders, SockaddrStorage, sockopt,
};
use std::{
    collections::VecDeque,
    io::{self, IoSlice, IoSliceMut},
    net::SocketAddr,
    os::fd::AsRawFd,
    sync::{Arc, Mutex},
    task::{Context, Poll, ready},
};
use tokio::{io::Interest, net::UdpSocket};

const COUNT: usize = 32;
const SIZE: usize = 65536;

struct Received {
    bytes: Vec<u8>,
    read: DatagramRead,
}

struct Rx {
    scratch: Vec<Vec<u8>>,
    pending: VecDeque<Received>,
}

pub(super) struct State {
    pub(super) caps: Arc<Mutex<Capabilities>>,
    rx: Mutex<Rx>,
}

impl std::fmt::Debug for State {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("UdpBatchState")
            .field("capabilities", &self.capabilities())
            .finish_non_exhaustive()
    }
}

fn unsupported(error: &io::Error) -> bool {
    matches!(error.raw_os_error(), Some(22 | 92 | 95 | 38))
}

fn invalid() -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, "invalid UDP batch metadata")
}

impl State {
    pub fn new(socket: &UdpSocket) -> io::Result<Self> {
        let probe = |result: nix::Result<()>| match result {
            Ok(()) => Ok(true),
            Err(error) => {
                let error = io::Error::from(error);
                if unsupported(&error) {
                    Ok(false)
                } else {
                    Err(error)
                }
            }
        };
        let segment = probe(socket::setsockopt(socket, sockopt::UdpGsoSegment, &0))?;
        let gro = probe(socket::setsockopt(socket, sockopt::UdpGroSegment, &true))?;
        Ok(Self {
            caps: Arc::new(Mutex::new(Capabilities {
                udp_segment: segment,
                udp_gro: gro,
                ..Capabilities::default()
            })),
            rx: Mutex::new(Rx {
                scratch: (0..COUNT).map(|_| vec![0; SIZE]).collect(),
                pending: VecDeque::new(),
            }),
        })
    }

    pub fn capabilities(&self) -> Capabilities {
        *self.caps.lock().expect("UDP capability lock")
    }

    pub fn poll_send(
        &self,
        socket: &UdpSocket,
        cx: &mut Context<'_>,
        packets: &[DatagramSend<'_>],
    ) -> Poll<io::Result<usize>> {
        if packets.is_empty() {
            return Poll::Ready(Ok(0));
        }
        ready!(socket.poll_send_ready(cx))?;
        let caps = self.capabilities();
        let packets = &packets[..packets.len().min(COUNT)];
        let size = packets[0].payload.len();
        let mut gso_count = 0;
        let mut bytes = 0;
        if caps.udp_segment && size > 0 && size <= u16::MAX as usize {
            for packet in packets {
                if packet.peer != packets[0].peer
                    || packet.payload.is_empty()
                    || packet.payload.len() > size
                    || bytes + packet.payload.len() > 65535
                {
                    break;
                }
                bytes += packet.payload.len();
                gso_count += 1;
                if packet.payload.len() < size {
                    break;
                }
            }
        }
        let gso = gso_count > 1;
        let mmsg = !gso && caps.sendmmsg != Some(false);
        let result = socket.try_io(Interest::WRITABLE, || {
            if gso {
                let slices: Vec<_> = packets[..gso_count]
                    .iter()
                    .map(|p| IoSlice::new(p.payload))
                    .collect();
                let peer = SockaddrStorage::from(packets[0].peer);
                let segment = size as u16;
                let written = socket::sendmsg(
                    socket.as_raw_fd(),
                    &slices,
                    &[ControlMessage::UdpGsoSegments(&segment)],
                    MsgFlags::MSG_DONTWAIT,
                    Some(&peer),
                )?;
                if written != bytes {
                    return Err(io::Error::new(
                        io::ErrorKind::WriteZero,
                        "partial UDP GSO write",
                    ));
                }
                Ok(gso_count)
            } else if mmsg {
                let slices: Vec<_> = packets.iter().map(|p| [IoSlice::new(p.payload)]).collect();
                let peers: Vec<_> = packets
                    .iter()
                    .map(|p| Some(SockaddrStorage::from(p.peer)))
                    .collect();
                let mut headers = MultiHeaders::preallocate(packets.len(), None);
                let results = socket::sendmmsg(
                    socket.as_raw_fd(),
                    &mut headers,
                    &slices,
                    &peers,
                    [],
                    MsgFlags::MSG_DONTWAIT,
                )?;
                let mut count = 0;
                for (result, packet) in results.zip(packets) {
                    if result.bytes != packet.payload.len() {
                        return Err(io::Error::new(
                            io::ErrorKind::WriteZero,
                            "partial UDP mmsg write",
                        ));
                    }
                    count += 1;
                }
                if count == 0 {
                    return Err(io::ErrorKind::WriteZero.into());
                }
                Ok(count)
            } else {
                let peer = SockaddrStorage::from(packets[0].peer);
                let n = socket::sendmsg(
                    socket.as_raw_fd(),
                    &[IoSlice::new(packets[0].payload)],
                    &[],
                    MsgFlags::MSG_DONTWAIT,
                    Some(&peer),
                )?;
                if n != size {
                    return Err(io::ErrorKind::WriteZero.into());
                }
                Ok(1)
            }
        });
        match result {
            Err(error) if unsupported(&error) && (gso || mmsg) => {
                let mut caps = self.caps.lock().expect("UDP capability lock");
                if gso {
                    caps.udp_segment = false;
                } else {
                    caps.sendmmsg = Some(false);
                }
                cx.waker().wake_by_ref();
                Poll::Pending // next poll rechecks the host scope before fallback
            }
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                cx.waker().wake_by_ref();
                Poll::Pending
            }
            result => {
                if result.is_ok() {
                    let mut caps = self.caps.lock().expect("UDP capability lock");
                    if mmsg {
                        caps.sendmmsg = Some(true);
                        caps.mmsg_sends += 1;
                    }
                    if gso {
                        caps.gso_sends += 1;
                    }
                }
                Poll::Ready(result)
            }
        }
    }

    fn fill(&self, socket: &UdpSocket, cx: &mut Context<'_>, rx: &mut Rx) -> Poll<io::Result<()>> {
        if !rx.pending.is_empty() {
            return Poll::Ready(Ok(()));
        }
        ready!(socket.poll_recv_ready(cx))?;
        let mmsg = self.capabilities().recvmmsg != Some(false);
        let result = socket.try_io(Interest::READABLE, || {
            let count = if mmsg { COUNT } else { 1 };
            let mut slices: Vec<_> = rx.scratch[..count]
                .iter_mut()
                .map(|b| [IoSliceMut::new(b)])
                .collect();
            let mut headers =
                MultiHeaders::<SockaddrStorage>::preallocate(count, Some(nix::cmsg_space!(i32)));
            let mut messages = Vec::new();
            if mmsg {
                for result in socket::recvmmsg(
                    socket.as_raw_fd(),
                    &mut headers,
                    &mut slices,
                    MsgFlags::MSG_DONTWAIT | MsgFlags::MSG_TRUNC,
                    None,
                )? {
                    messages.push(metadata(result)?);
                }
            } else {
                let mut ancillary = nix::cmsg_space!(i32);
                let result = socket::recvmsg(
                    socket.as_raw_fd(),
                    &mut slices[0],
                    Some(&mut ancillary),
                    MsgFlags::MSG_DONTWAIT | MsgFlags::MSG_TRUNC,
                )?;
                messages.push(metadata(result)?);
            }
            Ok(messages)
        });
        let messages = match result {
            Err(error) if mmsg && unsupported(&error) => {
                self.caps.lock().expect("UDP capability lock").recvmmsg = Some(false);
                cx.waker().wake_by_ref();
                return Poll::Pending;
            }
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                cx.waker().wake_by_ref();
                return Poll::Pending;
            }
            result => result?,
        };
        if mmsg {
            let mut caps = self.caps.lock().expect("UDP capability lock");
            caps.recvmmsg = Some(true);
            caps.mmsg_receives += 1;
        }
        for ((read, segment), buffer) in messages.into_iter().zip(&rx.scratch) {
            if let Some(segment) = segment {
                self.caps.lock().expect("UDP capability lock").gro_receives += 1;
                if read.truncated || read.copied_len == 0 || read.copied_len.div_ceil(segment) > 64
                {
                    return Poll::Ready(Err(invalid()));
                }
                for bytes in buffer[..read.copied_len].chunks(segment) {
                    rx.pending.push_back(Received {
                        bytes: bytes.to_vec(),
                        read: DatagramRead {
                            copied_len: bytes.len(),
                            original_len: Some(bytes.len()),
                            ..read
                        },
                    });
                }
            } else {
                rx.pending.push_back(Received {
                    bytes: buffer[..read.copied_len].to_vec(),
                    read,
                });
            }
        }
        if rx.pending.is_empty() {
            return Poll::Ready(Err(invalid()));
        }
        Poll::Ready(Ok(()))
    }

    pub fn poll_recv_one(
        &self,
        socket: &UdpSocket,
        cx: &mut Context<'_>,
        buffer: &mut [u8],
    ) -> Poll<io::Result<DatagramRead>> {
        let mut rx = self.rx.lock().expect("UDP receive lock");
        ready!(self.fill(socket, cx, &mut rx))?;
        Poll::Ready(Ok(copy(
            rx.pending.pop_front().expect("filled receive queue"),
            buffer,
        )))
    }

    pub fn poll_recv(
        &self,
        socket: &UdpSocket,
        cx: &mut Context<'_>,
        buffers: &mut [Vec<u8>],
    ) -> Poll<io::Result<Vec<DatagramRead>>> {
        if buffers.is_empty() {
            return Poll::Ready(Ok(Vec::new()));
        }
        let mut rx = self.rx.lock().expect("UDP receive lock");
        ready!(self.fill(socket, cx, &mut rx))?;
        let mut reads = Vec::new();
        for buffer in buffers.iter_mut().take(COUNT) {
            let Some(packet) = rx.pending.pop_front() else {
                break;
            };
            reads.push(copy(packet, buffer));
        }
        Poll::Ready(Ok(reads))
    }
}

fn copy(packet: Received, buffer: &mut [u8]) -> DatagramRead {
    let n = buffer.len().min(packet.bytes.len());
    buffer[..n].copy_from_slice(&packet.bytes[..n]);
    DatagramRead {
        copied_len: n,
        truncated: packet.read.truncated || n < packet.bytes.len(),
        ..packet.read
    }
}

fn metadata(
    message: socket::RecvMsg<'_, '_, SockaddrStorage>,
) -> io::Result<(DatagramRead, Option<usize>)> {
    if message.flags.contains(MsgFlags::MSG_CTRUNC) {
        return Err(invalid());
    }
    let address = message.address.ok_or_else(invalid)?;
    let source: SocketAddr = if let Some(v4) = address.as_sockaddr_in() {
        (*v4).into()
    } else if let Some(v6) = address.as_sockaddr_in6() {
        (*v6).into()
    } else {
        return Err(invalid());
    };
    let mut segment = None;
    for control in message.cmsgs()? {
        if let ControlMessageOwned::UdpGroSegments(size) = control {
            let size = usize::try_from(size)
                .ok()
                .filter(|s| *s > 0 && *s <= 65535)
                .ok_or_else(invalid)?;
            if segment.replace(size).is_some() {
                return Err(invalid());
            }
        }
    }
    Ok((
        DatagramRead {
            source,
            copied_len: message.bytes.min(SIZE),
            original_len: Some(message.bytes),
            truncated: message.flags.contains(MsgFlags::MSG_TRUNC),
        },
        segment,
    ))
}
