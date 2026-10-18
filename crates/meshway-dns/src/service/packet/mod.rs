//! DNS-only virtual IPv4 endpoint. smoltcp supplies TCP reliability/framing;
//! DNS selection and answer validity stay in the existing Dispatcher/Reply.
mod device;
#[cfg(test)]
mod tests;
use super::*;
use smoltcp::{
    iface::{Config, Interface, SocketHandle, SocketSet},
    socket::{tcp, udp},
    wire::{HardwareAddress, IpCidr, IpEndpoint, IpProtocol, Ipv4Packet, TcpPacket, UdpPacket},
};
use std::{collections::VecDeque, net::Ipv4Addr};
use tokio::sync::{Mutex, mpsc};

/// The packet worker is the sole ingress/egress consumer of this port.
pub struct Port {
    address: Ipv4Addr,
    mtu: usize,
    input: mpsc::Sender<Vec<u8>>,
    output: Mutex<mpsc::Receiver<Output>>,
    stop: CancellationToken,
}
pub struct Transport {
    address: Ipv4Addr,
    mtu: usize,
    input: mpsc::Receiver<Vec<u8>>,
    output: mpsc::Sender<Output>,
    stop: CancellationToken,
}
impl Drop for Transport {
    fn drop(&mut self) {
        self.stop.cancel();
    }
}

pub fn channel(address: Ipv4Addr, mtu: usize) -> io::Result<(Arc<Port>, Transport)> {
    if address.is_unspecified()
        || address.is_multicast()
        || address.is_loopback()
        || !(1280..=65535).contains(&mtu)
    {
        return Err(io::ErrorKind::InvalidInput.into());
    }
    let (input, received) = mpsc::channel(device::BATCH);
    let (output, replies) = mpsc::channel(device::BATCH);
    let stop = CancellationToken::new();
    Ok((
        Arc::new(Port {
            address,
            mtu,
            input,
            output: Mutex::new(replies),
            stop: stop.clone(),
        }),
        Transport {
            address,
            mtu,
            input: received,
            output,
            stop,
        },
    ))
}
impl Port {
    /// Reserved service address is consumed even for non-DNS traffic, which is
    /// dropped by the transport; it never falls through into Mesh/exit routing.
    pub fn accepts(&self, bytes: &[u8]) -> bool {
        Ipv4Packet::new_checked(bytes)
            .is_ok_and(|packet| packet.version() == 4 && packet.dst_addr() == self.address)
    }
    pub fn submit(&self, bytes: Vec<u8>) {
        if bytes.len() <= self.mtu && self.accepts(&bytes) && !self.stop.is_cancelled() {
            let _ = self.input.try_send(bytes);
        }
    }
    pub async fn recv(&self) -> Option<Output> {
        self.output.lock().await.recv().await
    }
    pub fn stopped(&self) -> bool {
        self.stop.is_cancelled()
    }
    pub fn close(&self) {
        self.stop.cancel();
    }
}

/// One frozen output. Host checks it on every packet-device write poll; queued
/// positive answers cannot survive a DNS scope change or service retirement.
pub struct Output {
    bytes: Vec<u8>,
    reply: Option<Arc<Reply>>,
    connection: Option<Arc<Connection>>,
    stop: CancellationToken,
}
impl Output {
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }
    pub fn ensure_current(&self) -> io::Result<()> {
        if self.stop.is_cancelled()
            || self
                .connection
                .as_ref()
                .is_some_and(|connection| connection.stop.is_cancelled())
        {
            return Err(io::ErrorKind::NotConnected.into());
        }
        if let Some(reply) = &self.reply {
            reply.ensure_current()?;
        }
        Ok(())
    }
    pub async fn ended(&self) {
        tokio::select! {
            _ = self.stop.cancelled() => {},
            _ = async { if let Some(reply) = &self.reply { reply.ended().await } else { std::future::pending().await } } => {},
            _ = async { if let Some(connection) = &self.connection { connection.stop.cancelled().await } else { std::future::pending().await } } => {},
        }
    }
}
struct Connection {
    stop: CancellationToken,
}
struct Tcp {
    handle: SocketHandle,
    connection: Arc<Connection>,
    buffered: BytesMut,
    codec: TcpCodec,
    querying: bool,
    reply: Option<Arc<Reply>>,
}
struct UdpReply {
    peer: IpEndpoint,
    bytes: Bytes,
    reply: Arc<Reply>,
}
enum Completed {
    Udp(IpEndpoint, Reply),
    Tcp(usize, Arc<Connection>, Reply),
}
struct Stack {
    interface: Interface,
    device: device::Queue,
    sockets: SocketSet<'static>,
    udp: SocketHandle,
    udp_replies: VecDeque<UdpReply>,
    tcp: Vec<Tcp>,
    started: Instant,
}
impl Stack {
    fn new(transport: &Transport, limits: ServiceLimits) -> io::Result<Self> {
        let mut device = device::Queue {
            input: VecDeque::new(),
            output: VecDeque::new(),
            mtu: transport.mtu,
        };
        let mut config = Config::new(HardwareAddress::Ip);
        let mut random = [0; 8];
        getrandom::fill(&mut random).map_err(|_| io::Error::other("OS randomness unavailable"))?;
        config.random_seed = u64::from_ne_bytes(random);
        let mut interface = Interface::new(config, &mut device, smoltcp::time::Instant::ZERO);
        interface.update_ip_addrs(|addresses| {
            addresses
                .push(IpCidr::new(transport.address.into(), 0))
                .expect("one DNS endpoint");
        });
        let mut sockets = SocketSet::new(Vec::new());
        let buffer = || {
            udp::PacketBuffer::new(
                vec![udp::PacketMetadata::EMPTY; device::BATCH],
                vec![0; device::BATCH * transport.mtu],
            )
        };
        let mut udp = udp::Socket::new(buffer(), buffer());
        udp.bind((transport.address, 53))
            .map_err(io::Error::other)?;
        let udp = sockets.add(udp);
        let mut tcp = Vec::new();
        for _ in 0..limits.max_tcp_connections {
            let buffer =
                || tcp::SocketBuffer::new(vec![0; usize::from(limits.max_tcp_frame_size) + 2]);
            let mut socket = tcp::Socket::new(buffer(), buffer());
            socket.set_timeout(Some(smoltcp::time::Duration::from_millis(
                limits.tcp_idle_timeout.as_millis() as u64,
            )));
            socket.set_nagle_enabled(false);
            socket
                .listen((transport.address, 53))
                .map_err(io::Error::other)?;
            tcp.push(Tcp {
                handle: sockets.add(socket),
                connection: Arc::new(Connection {
                    stop: transport.stop.child_token(),
                }),
                buffered: BytesMut::new(),
                codec: TcpCodec::new(limits.max_tcp_frame_size).map_err(io::Error::other)?,
                querying: false,
                reply: None,
            });
        }
        Ok(Self {
            interface,
            device,
            sockets,
            udp,
            udp_replies: VecDeque::new(),
            tcp,
            started: Instant::now(),
        })
    }
    fn poll(&mut self, transport: &Transport) {
        let now = smoltcp::time::Instant::from_millis(self.started.elapsed().as_millis() as i64);
        if self
            .udp_replies
            .iter()
            .any(|reply| reply.reply.ensure_current().is_err())
        {
            let socket = self.sockets.get_mut::<udp::Socket>(self.udp);
            socket.close();
            let _ = socket.bind((transport.address, 53));
            self.udp_replies.clear();
        }
        for tcp in &mut self.tcp {
            let socket = self.sockets.get_mut::<tcp::Socket>(tcp.handle);
            if tcp
                .reply
                .as_ref()
                .is_some_and(|reply| reply.ensure_current().is_err())
            {
                socket.abort();
            }
            if socket.state() == tcp::State::Closed {
                tcp.connection.stop.cancel();
                tcp.connection = Arc::new(Connection {
                    stop: transport.stop.child_token(),
                });
                tcp.buffered.clear();
                tcp.querying = false;
                tcp.reply = None;
                let _ = socket.listen((transport.address, 53));
            }
        }
        self.interface
            .poll(now, &mut self.device, &mut self.sockets);
        while let Some(bytes) = self.device.output.pop_front() {
            let Ok(ip) = Ipv4Packet::new_checked(bytes.as_slice()) else {
                continue;
            };
            let mut reply = None;
            let mut connection = None;
            match ip.next_header() {
                IpProtocol::Udp => {
                    let Ok(udp) = UdpPacket::new_checked(ip.payload()) else {
                        continue;
                    };
                    let peer = IpEndpoint::new(ip.dst_addr().into(), udp.dst_port());
                    let Some(index) = self.udp_replies.iter().position(|reply| {
                        reply.peer == peer && reply.bytes.as_ref() == udp.payload()
                    }) else {
                        continue;
                    };
                    reply = self.udp_replies.remove(index).map(|reply| reply.reply);
                }
                IpProtocol::Tcp => {
                    let Ok(packet) = TcpPacket::new_checked(ip.payload()) else {
                        continue;
                    };
                    let peer = IpEndpoint::new(ip.dst_addr().into(), packet.dst_port());
                    let mut matching = self.tcp.iter().filter(|tcp| {
                        self.sockets
                            .get::<tcp::Socket>(tcp.handle)
                            .remote_endpoint()
                            == Some(peer)
                    });
                    if let Some(tcp) = matching.next() {
                        // Never attach another connection's answer lifetime
                        // to an ambiguous tuple, even if a peer reuses a port.
                        if matching.next().is_some() {
                            continue;
                        }
                        connection = Some(tcp.connection.clone());
                        if !packet.payload().is_empty() {
                            reply = tcp.reply.clone();
                            if reply.is_none() {
                                continue;
                            }
                        }
                    } else if !packet.payload().is_empty() {
                        continue;
                    }
                }
                _ => continue,
            }
            let output = Output {
                bytes,
                reply,
                connection,
                stop: transport.stop.clone(),
            };
            if output.ensure_current().is_ok() {
                let _ = transport.output.try_send(output);
            }
        }
    }
    fn receive(&mut self, bytes: Vec<u8>) {
        if self.device.input.len() >= device::BATCH {
            return;
        }
        let Ok(ip) = Ipv4Packet::new_checked(bytes.as_slice()) else {
            return;
        };
        let dns = match ip.next_header() {
            IpProtocol::Tcp => {
                TcpPacket::new_checked(ip.payload()).is_ok_and(|tcp| tcp.dst_port() == 53)
            }
            IpProtocol::Udp => {
                UdpPacket::new_checked(ip.payload()).is_ok_and(|udp| udp.dst_port() == 53)
            }
            _ => false,
        };
        if dns {
            self.device.input.push_back(bytes);
        }
    }
    fn queries(
        &mut self,
        service: &Service,
        transport: &Transport,
        tasks: &mut JoinSet<Completed>,
    ) {
        while tasks.len() < service.limits.max_queries {
            let socket = self.sockets.get_mut::<udp::Socket>(self.udp);
            let Ok((bytes, metadata)) = socket.recv() else {
                break;
            };
            let Ok(query) = decode_query(bytes) else {
                continue;
            };
            let peer = metadata.endpoint;
            let dispatcher = service.dispatcher.clone();
            let stop = transport.stop.clone();
            let deadline = Instant::now() + service.limits.query_timeout;
            tasks.spawn(async move {
                Completed::Udp(
                    peer,
                    dispatch(dispatcher.as_ref(), query, deadline, stop).await,
                )
            });
        }
        for (index, tcp) in self.tcp.iter_mut().enumerate() {
            let socket = self.sockets.get_mut::<tcp::Socket>(tcp.handle);
            if tcp.reply.is_some() && socket.send_queue() == 0 {
                tcp.reply = None;
            }
            if tcp.querying || tcp.reply.is_some() || tasks.len() >= service.limits.max_queries {
                continue;
            }
            let room = (usize::from(service.limits.max_tcp_frame_size) + 2)
                .saturating_sub(tcp.buffered.len());
            if room != 0 && socket.can_recv() {
                let count = room.min(socket.recv_queue());
                let start = tcp.buffered.len();
                tcp.buffered.resize(start + count, 0);
                let read = socket.recv_slice(&mut tcp.buffered[start..]).unwrap_or(0);
                tcp.buffered.truncate(start + read);
            }
            match tcp.codec.decode(&mut tcp.buffered) {
                Ok(Some(query)) => {
                    tcp.querying = true;
                    let connection = tcp.connection.clone();
                    let stop = connection.stop.clone();
                    let dispatcher = service.dispatcher.clone();
                    let deadline = Instant::now() + service.limits.query_timeout;
                    tasks.spawn(async move {
                        Completed::Tcp(
                            index,
                            connection,
                            dispatch(dispatcher.as_ref(), query, deadline, stop).await,
                        )
                    });
                }
                Err(_) => socket.abort(),
                Ok(None) if !socket.may_recv() && socket.state() == tcp::State::CloseWait => {
                    socket.close()
                }
                Ok(None) => {}
            }
        }
    }
    fn completed(&mut self, completed: Completed, limits: ServiceLimits) {
        match completed {
            Completed::Udp(peer, reply) => {
                let Ok(bytes) = reply.encode(ResponseTransport::Udp, limits.udp_payload_limit)
                else {
                    return;
                };
                if self.udp_replies.len() >= device::BATCH {
                    return;
                }
                if self
                    .sockets
                    .get_mut::<udp::Socket>(self.udp)
                    .send_slice(&bytes, peer)
                    .is_ok()
                {
                    self.udp_replies.push_back(UdpReply {
                        peer,
                        bytes,
                        reply: Arc::new(reply),
                    });
                }
            }
            Completed::Tcp(index, connection, reply) => {
                let tcp = &mut self.tcp[index];
                if !Arc::ptr_eq(&tcp.connection, &connection) {
                    return;
                }
                tcp.querying = false;
                let socket = self.sockets.get_mut::<tcp::Socket>(tcp.handle);
                let Ok(bytes) = reply.encode(ResponseTransport::Tcp, limits.udp_payload_limit)
                else {
                    socket.abort();
                    return;
                };
                let mut frame = BytesMut::new();
                if tcp.codec.encode(bytes, &mut frame).is_err()
                    || !socket.can_send()
                    || socket.send_slice(&frame).ok() != Some(frame.len())
                {
                    socket.abort();
                    return;
                }
                tcp.reply = Some(Arc::new(reply));
            }
        }
    }
}
impl Service {
    pub async fn run_packets(
        self,
        mut transport: Transport,
        shutdown: CancellationToken,
    ) -> io::Result<()> {
        let mut stack = Stack::new(&transport, self.limits)?;
        let mut tasks = JoinSet::new();
        let mut tick = tokio::time::interval(Duration::from_millis(10));
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        let result = loop {
            stack.poll(&transport);
            stack.queries(&self, &transport, &mut tasks);
            tokio::select! { biased;
                _ = shutdown.cancelled() => break Ok(()),
                _ = transport.stop.cancelled() => break Ok(()),
                completed = tasks.join_next(), if !tasks.is_empty() => match completed {
                    Some(Ok(completed)) => stack.completed(completed, self.limits),
                    Some(Err(_)) => break Err(io::Error::other("DNS packet query worker failed")),
                    None => {},
                },
                packet = transport.input.recv() => match packet { Some(packet) => stack.receive(packet), None => break Ok(()) },
                _ = tick.tick() => {},
            }
        };
        transport.stop.cancel();
        tasks.abort_all();
        while tasks.join_next().await.is_some() {}
        self.dispatcher.stop().await;
        result
    }
}
