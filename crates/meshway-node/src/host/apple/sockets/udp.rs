use super::super::resources::Owned;
use super::*;
use meshway_transport::{
    SendLifetime,
    raw::{DatagramRead, DatagramSend},
    spec::{Listener, NativeUdpDialer, SocketOptions},
    udp::UdpTransport,
};
use std::task::{Context, Poll};

type Grants = Vec<(SocketAddr, Arc<dyn SendLifetime>)>;
pub(super) struct Endpoint {
    factory: Factory,
    interface: String,
    socket: Owned<UdpTransport>,
    local: SocketAddr,
    buffers: meshway_transport::raw::Buffers,
    targets: Option<Vec<SocketAddr>>,
    traversal: Option<Arc<meshway_underlay::traversal::Registry>>,
    grants: Mutex<[Grants; 2]>,
}
impl Endpoint {
    fn bind(
        factory: &Factory,
        resource: Arc<Resource>,
        local: SocketAddr,
        interface: &str,
        options: &SocketOptions,
        targets: Option<Vec<SocketAddr>>,
    ) -> io::Result<Self> {
        resource.check()?;
        let socket = socket2::Socket::new(
            socket2::Domain::for_address(local),
            socket2::Type::DGRAM,
            Some(socket2::Protocol::UDP),
        )?;
        if local.is_ipv6() {
            socket.set_only_v6(true)?;
        }
        socket.set_nonblocking(true)?;
        factory.protect(&socket2::SockRef::from(&socket), local.is_ipv4(), interface)?;
        socket.bind(&local.into())?;
        let socket =
            UdpTransport::adopt_batch(tokio::net::UdpSocket::from_std(socket.into())?, options)?;
        let local = socket.local_addr()?;
        let buffers = socket.buffers();
        Ok(Self {
            factory: factory.clone(),
            interface: interface.into(),
            socket: Owned::new(socket, resource)?,
            local,
            buffers,
            targets,
            traversal: None,
            grants: Mutex::new(Default::default()),
        })
    }
    pub fn dial(
        factory: &Factory,
        resource: Arc<Resource>,
        options: &NativeUdpDialer,
        ipv4: bool,
        targets: Vec<SocketAddr>,
    ) -> Result<Self, Error> {
        if !options.enabled {
            return Err(Error::Configuration);
        }
        let local = if options.local_address.is_empty() {
            if ipv4 {
                IpAddr::from(std::net::Ipv4Addr::UNSPECIFIED)
            } else {
                IpAddr::from(std::net::Ipv6Addr::UNSPECIFIED)
            }
        } else {
            options
                .local_address
                .parse()
                .map_err(|_| Error::Configuration)?
        };
        if local.is_ipv4() != ipv4 {
            return Err(Error::Configuration);
        }
        Ok(Self::bind(
            factory,
            resource,
            SocketAddr::new(local, 0),
            &options.bind_interface,
            &options.socket,
            Some(targets),
        )?)
    }
    pub fn listen(factory: &Factory, desired: &Desired, key: &str) -> Result<Self, Error> {
        let resource = factory.resource(desired, key, true)?;
        let Listener::NativeUdp(options) = desired.local().listeners[key].declaration() else {
            return Err(Error::Configuration);
        };
        if !options.enabled {
            return Err(Error::Configuration);
        }
        let address = SocketAddr::new(options.listen, options.listen_port.get());
        Self::bind(
            factory,
            resource,
            address,
            &options.bind_interface,
            &options.socket,
            None,
        )
        .map_err(|source| Error::Bind {
            key: key.into(),
            address,
            source,
        })
    }
    pub fn traversal(
        mut self,
        factory: Arc<Factory>,
        desired: &Desired,
        reference: meshway_control::connectivity::SocketReference,
    ) -> Result<TraversalSocket, Error> {
        if !meshway_underlay::traversal::references(desired, &reference)
            .map_err(|_| Error::Authority)?
            || reference.resource != self.socket.resource.purpose
            || reference.socket_id.len() != 64
            || !reference
                .socket_id
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit())
        {
            return Err(Error::Authority);
        }
        let context = Arc::new(TraversalContext {
            factory,
            resource: self.socket.resource.clone(),
        });
        let (authorization, registry) =
            meshway_underlay::DatagramAuthorization::new(context, reference, self.local);
        self.traversal = Some(registry);
        Ok(TraversalSocket {
            endpoint: Box::new(self),
            authorization: Arc::new(authorization),
        })
    }
    fn allows(&self, peer: SocketAddr) -> bool {
        self.targets
            .as_ref()
            .is_none_or(|targets| targets.contains(&peer))
            || self
                .traversal
                .as_ref()
                .is_some_and(|registry| registry.allows(peer))
            || self.grants.lock().is_ok_and(|grants| {
                grants
                    .iter()
                    .flatten()
                    .any(|(address, lifetime)| *address == peer && lifetime.is_current())
            })
    }
    fn grant(
        &self,
        peer: SocketAddr,
        lifetime: Arc<dyn SendLifetime>,
        kind: usize,
    ) -> io::Result<()> {
        self.socket.resource.check()?;
        if self.targets.is_none() {
            return Ok(());
        }
        if peer.is_ipv4() != self.local.is_ipv4()
            || meshway_control::connectivity::udp_recipient(&peer.to_string()).is_err()
            || !lifetime.is_current()
        {
            return Err(io::ErrorKind::PermissionDenied.into());
        }
        let mut grants = self.grants.lock().map_err(|_| io::ErrorKind::Other)?;
        let grants = &mut grants[kind];
        grants.retain(|(address, lifetime)| *address != peer && lifetime.is_current());
        if grants.len() >= 4 {
            return Err(io::ErrorKind::WouldBlock.into());
        }
        grants.push((peer, lifetime));
        Ok(())
    }
}
impl Datagram for Endpoint {
    fn local_addr(&self) -> io::Result<SocketAddr> {
        self.socket.resource.check()?;
        Ok(self.local)
    }
    fn buffers(&self) -> Option<meshway_transport::raw::Buffers> {
        Some(self.buffers)
    }
    fn poll_send_batch(
        &self,
        cx: &mut Context<'_>,
        packets: &[DatagramSend<'_>],
    ) -> Poll<io::Result<usize>> {
        if packets.iter().any(|packet| !self.allows(packet.peer)) {
            return Poll::Ready(Err(io::ErrorKind::PermissionDenied.into()));
        }
        self.socket.poll(cx, true, |socket, cx| {
            self.factory.protect(
                &socket2::SockRef::from(&*socket),
                self.local.is_ipv4(),
                &self.interface,
            )?;
            socket.poll_send_batch(cx, packets)
        })
    }
    fn poll_recv_batch(
        &self,
        cx: &mut Context<'_>,
        buffers: &mut [Vec<u8>],
    ) -> Poll<io::Result<Vec<DatagramRead>>> {
        self.socket.poll(cx, false, |socket, cx| {
            self.factory.protect(
                &socket2::SockRef::from(&*socket),
                self.local.is_ipv4(),
                &self.interface,
            )?;
            socket.poll_recv_batch(cx, buffers)
        })
    }
    fn poll_send_to(
        &self,
        cx: &mut Context<'_>,
        bytes: &[u8],
        peer: SocketAddr,
    ) -> Poll<io::Result<usize>> {
        if !self.allows(peer) {
            return Poll::Ready(Err(io::ErrorKind::PermissionDenied.into()));
        }
        self.socket.poll(cx, true, |socket, cx| {
            self.factory.protect(
                &socket2::SockRef::from(&*socket),
                self.local.is_ipv4(),
                &self.interface,
            )?;
            socket.poll_send_to(cx, bytes, peer)
        })
    }
    fn poll_recv_from(
        &self,
        cx: &mut Context<'_>,
        buffer: &mut [u8],
    ) -> Poll<io::Result<DatagramRead>> {
        self.socket.poll(cx, false, |socket, cx| {
            self.factory.protect(
                &socket2::SockRef::from(&*socket),
                self.local.is_ipv4(),
                &self.interface,
            )?;
            socket.poll_recv_from(cx, buffer)
        })
    }
    fn authorize_challenge(
        &self,
        peer: SocketAddr,
        lifetime: Arc<dyn SendLifetime>,
    ) -> io::Result<()> {
        self.grant(peer, lifetime, 0)
    }
    fn authorize_source(
        &self,
        peer: SocketAddr,
        lifetime: Arc<dyn SendLifetime>,
    ) -> io::Result<()> {
        self.grant(peer, lifetime, 1)
    }
}
impl Drop for Endpoint {
    fn drop(&mut self) {
        if let Some(registry) = &self.traversal {
            registry.closed.cancel();
        }
    }
}

struct TraversalContext {
    factory: Arc<Factory>,
    resource: Arc<Resource>,
}
impl meshway_underlay::traversal::Context for TraversalContext {
    fn execution(&self) -> meshway_underlay::Result<Desired> {
        Ok(self.factory.resources.execution(&self.resource)?)
    }
    fn resolve<'a>(
        &'a self,
        host: &'a str,
        port: u16,
    ) -> BoxFuture<'a, meshway_underlay::Result<Vec<SocketAddr>>> {
        Box::pin(async move {
            let endpoint = meshway_transport::spec::Endpoint {
                host: host.into(),
                port: meshway_transport::spec::Port::new(port)
                    .map_err(|_| meshway_underlay::Error::Target)?,
            };
            Ok(self.factory.resolve(&self.resource, &endpoint).await?)
        })
    }
}
