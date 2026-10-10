use meshway_configuration::execution::{DialCandidate, DialEndpoint};
use meshway_engine::Desired;
use meshway_model::NodeId;
use meshway_model::TransportProtocol;
use meshway_transport_spec::{
    DialParameters, Endpoint, Listener, PreparedListener, SocketOptions, TcpOptions,
};
use socket2::SockRef;
use std::{
    fmt, io,
    net::{IpAddr, SocketAddr},
    pin::Pin,
    sync::{Arc, Mutex},
    task::{Context, Poll},
    time::Duration,
};
use tokio::{
    io::{AsyncRead, AsyncWrite, ReadBuf},
    net::{TcpListener, TcpSocket, TcpStream},
    sync::{OnceCell, OwnedSemaphorePermit},
};
use tokio_util::sync::CancellationToken;

use super::{
    datagram::{DatagramCandidate, ProtectedDatagram},
    dns::Resolver,
    protection::{ProtectionScope, Shared, io_error},
};
use crate::{Error, Options, Result, target::View};

#[derive(Clone)]
pub struct SocketFactory {
    pub(super) shared: Arc<Shared>,
    resolver: Arc<OnceCell<Resolver>>,
    pub(super) options: Arc<Options>,
    pub(super) timeout: Duration,
}

#[derive(Clone)]
pub struct CandidateHandle {
    scope: ProtectionScope,
    local: NodeId,
    candidate: DialCandidate,
    endpoint: Endpoint,
}

impl CandidateHandle {
    pub fn local(&self) -> NodeId {
        self.local
    }
    pub fn peer(&self) -> NodeId {
        self.candidate.peer
    }
    pub fn listener_key(&self) -> &str {
        &self.candidate.listener_key
    }
    pub fn protocol(&self) -> TransportProtocol {
        self.candidate.parameters.transport()
    }
    pub fn parameters(&self) -> &DialParameters {
        &self.candidate.parameters
    }
    pub fn endpoint(&self) -> &Endpoint {
        &self.endpoint
    }
}

impl fmt::Debug for CandidateHandle {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CandidateHandle")
            .field("protocol", &self.protocol())
            .finish_non_exhaustive()
    }
}

#[derive(Clone)]
pub struct ListenerHandle {
    scope: ProtectionScope,
    local: NodeId,
    key: String,
    declaration: PreparedListener,
}

impl ListenerHandle {
    pub fn local(&self) -> NodeId {
        self.local
    }
    pub fn key(&self) -> &str {
        &self.key
    }
    pub fn declaration(&self) -> &PreparedListener {
        &self.declaration
    }
}

impl fmt::Debug for ListenerHandle {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ListenerHandle").finish_non_exhaustive()
    }
}

impl SocketFactory {
    pub(super) fn new(shared: Arc<Shared>, options: Options) -> Self {
        Self {
            shared,
            resolver: Arc::new(OnceCell::new()),
            timeout: options.connect_timeout,
            options: Arc::new(options),
        }
    }

    // Candidate-only hosts do not need a Control proxy. Resolver configuration
    // is loaded once on first use; every candidate still gets its own data scope.
    pub(super) async fn resolver(&self) -> Result<&Resolver> {
        self.shared.check()?;
        self.resolver
            .get_or_try_init(|| Resolver::open(self.shared.clone(), &self.options))
            .await
    }

    /// The Host's installed content, for consumers that address sockets by
    /// resource only.
    pub fn installed(&self) -> Result<Desired> {
        self.shared.installed()?.ok_or(Error::Expired)
    }

    pub async fn traversal_listener(
        &self,
        desired: &Desired,
        key: &str,
        reference: meshway_control::connectivity::SocketReference,
    ) -> Result<super::TraversalDatagram> {
        self.validate_socket_reference(
            desired,
            &reference,
            &meshway_model::state::ResourceRef::Listener {
                key: key.to_owned(),
            },
        )?;
        let endpoint = self.datagram_listener(desired, key).await?;
        super::DatagramAuthorization::attach(endpoint, self.resolver().await?.clone(), reference)
    }

    pub async fn bind_datagram_candidate(
        &self,
        desired: &Desired,
        key: &str,
        ipv4: bool,
        reference: meshway_control::connectivity::SocketReference,
    ) -> Result<super::TraversalDatagram> {
        self.validate_socket_reference(
            desired,
            &reference,
            &meshway_model::state::ResourceRef::Dial {
                key: key.to_owned(),
            },
        )?;
        let view = View::of(desired);
        let candidate = view.dialers().get(key).ok_or(Error::Target)?;
        if view.peer(candidate.peer).is_none() {
            return Err(Error::Target);
        }
        let DialParameters::NativeUdp { options } = &candidate.parameters else {
            return Err(Error::Target);
        };
        let local = udp_local(options)?;
        if local.is_some_and(|ip| ip.is_ipv4() != ipv4) {
            return Err(Error::Target);
        }
        let local = local.unwrap_or_else(|| {
            if ipv4 {
                std::net::Ipv4Addr::UNSPECIFIED.into()
            } else {
                std::net::Ipv6Addr::UNSPECIFIED.into()
            }
        });
        let scope = self
            .shared
            .data(desired)?
            .for_resource(reference.resource.clone())
            .physical()?;
        let endpoint = ProtectedDatagram::bind(
            scope,
            SocketAddr::new(local, 0),
            &options.socket,
            Some(Vec::new()),
        )?;
        super::DatagramAuthorization::attach(endpoint, self.resolver().await?.clone(), reference)
    }

    fn validate_socket_reference(
        &self,
        desired: &Desired,
        reference: &meshway_control::connectivity::SocketReference,
        resource: &meshway_model::state::ResourceRef,
    ) -> Result<()> {
        self.shared.data(desired)?;
        if !super::traversal::references(desired, reference)?
            || &reference.resource != resource
            || reference.socket_id.len() != 64
            || !reference.socket_id.bytes().all(|b| b.is_ascii_hexdigit())
        {
            return Err(Error::Target);
        }
        Ok(())
    }

    /// One dial target of candidate `key`: a configured endpoint or a peer
    /// fact. The endpoint is selection input only; the socket is protected
    /// for the candidate's resource like any other.
    pub fn candidate(
        &self,
        desired: &Desired,
        key: &str,
        endpoint: &DialEndpoint,
    ) -> Result<CandidateHandle> {
        let scope =
            self.shared
                .data(desired)?
                .for_resource(meshway_model::state::ResourceRef::Dial {
                    key: key.to_owned(),
                });
        let view = View::of(desired);
        let candidate = view.dialers().get(key).ok_or(Error::Target)?.clone();
        validate_dial(&candidate.parameters)?;
        if view.peer(candidate.peer).is_none() {
            return Err(Error::Target);
        }
        let endpoint = endpoint.endpoint();
        Ok(CandidateHandle {
            scope,
            local: view.node_id(),
            candidate,
            endpoint,
        })
    }

    pub fn listener(&self, desired: &Desired, key: &str) -> Result<ListenerHandle> {
        let scope =
            self.shared
                .data(desired)?
                .for_resource(meshway_model::state::ResourceRef::Listener {
                    key: key.to_owned(),
                });
        let view = View::of(desired);
        let declaration = view.listeners().get(key).ok_or(Error::Target)?.clone();
        validate_listener(declaration.declaration())?;
        Ok(ListenerHandle {
            scope,
            local: view.node_id(),
            key: key.to_owned(),
            declaration,
        })
    }

    pub async fn connect(&self, candidate: &CandidateHandle) -> Result<ProtectedTcp> {
        let _attempt = self
            .shared
            .attempts
            .clone()
            .try_acquire_owned()
            .map_err(|_| Error::Capacity)?;
        if !Arc::ptr_eq(&self.shared, &candidate.scope.shared) {
            return Err(Error::Target);
        }
        candidate.scope.check()?;
        let resolver = self.resolver().await?.scoped(candidate.scope.clone())?;
        let (socket, tcp, local, timeout) = dial_options(&candidate.candidate.parameters)?;
        let addresses = tokio::select! { biased;
            _ = candidate.scope.ended() => return Err(Error::Expired),
            result = resolver.resolve(&candidate.endpoint.host, candidate.endpoint.port.get()) => result?,
        };
        let timeout = timeout.min(self.timeout);
        let operation = async {
            let mut last = None;
            for address in addresses {
                if local.is_some_and(|ip| ip.is_ipv4() != address.is_ipv4()) {
                    continue;
                }
                match connect_tcp(
                    candidate.scope.clone().physical()?,
                    address,
                    local.map(|ip| SocketAddr::new(ip, 0)),
                    socket,
                    tcp,
                    timeout,
                )
                .await
                {
                    Ok(stream) => return Ok(stream),
                    Err(error) => {
                        candidate.scope.check()?;
                        // Protection/capacity failures are terminal, never a retry without protection.
                        if !matches!(error, Error::Io(_)) {
                            return Err(error);
                        }
                        last = Some(error);
                    }
                }
            }
            Err(last.unwrap_or(Error::Resolver))
        };
        tokio::time::timeout(timeout, operation)
            .await
            .map_err(|_| Error::Io(io::ErrorKind::TimedOut.into()))?
    }

    pub async fn datagram_listener(
        &self,
        desired: &Desired,
        key: &str,
    ) -> Result<ProtectedDatagram> {
        let handle = self.listener(desired, key)?;
        let Listener::NativeUdp(value) = handle.declaration.declaration() else {
            return Err(Error::Target);
        };
        ProtectedDatagram::bind(
            handle.scope.clone().physical()?,
            SocketAddr::new(value.listen, value.listen_port.get()),
            &value.socket,
            None,
        )
    }

    pub async fn datagram_candidate(
        &self,
        desired: &Desired,
        key: &str,
        endpoint: &DialEndpoint,
    ) -> Result<Vec<DatagramCandidate>> {
        let handle = self.candidate(desired, key, endpoint)?;
        let DialParameters::NativeUdp { options } = &handle.candidate.parameters else {
            return Err(Error::Target);
        };
        let local = udp_local(options)?;
        let _attempt = self
            .shared
            .attempts
            .clone()
            .try_acquire_owned()
            .map_err(|_| Error::Capacity)?;
        let resolver = self.resolver().await?.scoped(handle.scope.clone())?;
        let addresses = tokio::select! { biased;
            _ = handle.scope.ended() => return Err(Error::Expired),
            result = tokio::time::timeout(options.connect_timeout.as_std().min(self.timeout),
                resolver.resolve(&handle.endpoint.host, handle.endpoint.port.get())) =>
                result.map_err(|_| Error::Resolver)??,
        };
        let mut endpoints = Vec::new();
        for ipv4 in [true, false] {
            if local.is_some_and(|local| local.is_ipv4() != ipv4) {
                continue;
            }
            let mut targets: Vec<_> = addresses
                .iter()
                .copied()
                .filter(|a| a.is_ipv4() == ipv4)
                .collect();
            targets.dedup();
            if targets.is_empty() {
                continue;
            }
            let ip = local.unwrap_or_else(|| {
                if ipv4 {
                    std::net::Ipv4Addr::UNSPECIFIED.into()
                } else {
                    std::net::Ipv6Addr::UNSPECIFIED.into()
                }
            });
            endpoints.push(DatagramCandidate {
                endpoint: ProtectedDatagram::bind(
                    handle.scope.clone().physical()?,
                    SocketAddr::new(ip, 0),
                    &options.socket,
                    Some(targets.clone()),
                )?,
                targets,
            });
        }
        handle.scope.check()?;
        if endpoints.is_empty() {
            return Err(Error::Resolver);
        }
        Ok(endpoints)
    }

    pub async fn listen(&self, handle: &ListenerHandle) -> Result<ProtectedListener> {
        if !Arc::ptr_eq(&self.shared, &handle.scope.shared) {
            return Err(Error::Target);
        }
        let (address, options, tcp) = listener_options(handle.declaration.declaration())?;
        let capacity = handle.scope.capacity()?;
        let socket = tcp_socket(address)?;
        handle.scope.prepare_checked(&SockRef::from(&socket))?;
        // Replacing our publication closes established channels, which may leave
        // TIME_WAIT tuples on this listening port. REUSEADDR permits that restart;
        // REUSEPORT stays off, so a live foreign listener still prevents binding.
        socket.set_reuseaddr(true)?;
        meshway_transport::raw::apply_socket_options(&SockRef::from(&socket), options)?;
        socket.bind(address)?;
        handle.scope.prepare_checked(&SockRef::from(&socket))?;
        let listener = socket.listen(128)?;
        handle.scope.prepare_checked(&SockRef::from(&listener))?;
        let owned = Owned::new(listener, handle.scope.clone().physical()?, capacity)?;
        Ok(ProtectedListener {
            owned,
            options: options.clone(),
            tcp: tcp.clone(),
        })
    }
}

fn udp_local(options: &meshway_transport_spec::NativeUdpDialer) -> Result<Option<IpAddr>> {
    if !options.enabled || !options.bind_interface.is_empty() {
        return Err(Error::Target);
    }
    if options.local_address.is_empty() {
        Ok(None)
    } else {
        options
            .local_address
            .parse()
            .map(Some)
            .map_err(|_| Error::Target)
    }
}

pub(super) fn validate_dial(parameters: &DialParameters) -> Result<()> {
    match parameters {
        DialParameters::NativeUdp { options } => udp_local(options).map(|_| ()),
        parameters => dial_options(parameters).map(|_| ()),
    }
}

pub(super) fn validate_listener(listener: &Listener) -> Result<()> {
    match listener {
        Listener::NativeUdp(value) if value.enabled && value.bind_interface.is_empty() => Ok(()),
        listener => listener_options(listener).map(|_| ()),
    }
}

pub(super) fn dial_options(
    parameters: &DialParameters,
) -> Result<(&SocketOptions, &TcpOptions, Option<IpAddr>, Duration)> {
    let (enabled, interface, local, socket, tcp, timeout) = match parameters {
        DialParameters::NativeTcp { options: v } => (
            v.enabled,
            &v.bind_interface,
            &v.local_address,
            &v.socket,
            &v.tcp,
            v.connect_timeout.as_std(),
        ),
        DialParameters::Vless { options: v, .. } => (
            v.enabled,
            &v.bind_interface,
            &v.local_address,
            &v.socket,
            &v.tcp,
            v.connect_timeout.as_std(),
        ),
        DialParameters::Trojan { options: v, .. } => (
            v.enabled,
            &v.bind_interface,
            &v.local_address,
            &v.socket,
            &v.tcp,
            v.connect_timeout.as_std(),
        ),
        DialParameters::NativeUdp { .. } => return Err(Error::Target),
    };
    if !enabled || !interface.is_empty() {
        return Err(Error::Target);
    }
    let local = if local.is_empty() {
        None
    } else {
        Some(local.parse().map_err(|_| Error::Target)?)
    };
    Ok((socket, tcp, local, timeout))
}

pub(super) fn listener_options(
    listener: &Listener,
) -> Result<(SocketAddr, &SocketOptions, &TcpOptions)> {
    let (enabled, interface, ip, port, socket, tcp) = match listener {
        Listener::NativeTcp(v) => (
            v.enabled,
            &v.bind_interface,
            v.listen,
            v.listen_port,
            &v.socket,
            &v.tcp,
        ),
        Listener::Vless(v) => (
            v.enabled,
            &v.bind_interface,
            v.listen,
            v.listen_port,
            &v.socket,
            &v.tcp,
        ),
        Listener::Trojan(v) => (
            v.enabled,
            &v.bind_interface,
            v.listen,
            v.listen_port,
            &v.socket,
            &v.tcp,
        ),
        Listener::NativeUdp(_) => return Err(Error::Target),
    };
    if !enabled || !interface.is_empty() {
        return Err(Error::Target);
    }
    Ok((SocketAddr::new(ip, port.get()), socket, tcp))
}

pub(super) fn listener_enabled(listener: &Listener) -> bool {
    match listener {
        Listener::NativeUdp(value) => value.enabled,
        Listener::NativeTcp(value) => value.enabled,
        Listener::Vless(value) => value.enabled,
        Listener::Trojan(value) => value.enabled,
    }
}

fn tcp_socket(address: SocketAddr) -> io::Result<TcpSocket> {
    if address.is_ipv4() {
        TcpSocket::new_v4()
    } else {
        TcpSocket::new_v6()
    }
}

pub(super) async fn connect_tcp(
    scope: ProtectionScope,
    address: SocketAddr,
    bind: Option<SocketAddr>,
    options: &SocketOptions,
    tcp: &TcpOptions,
    timeout: Duration,
) -> Result<ProtectedTcp> {
    let capacity = scope.capacity()?;
    let socket = tcp_socket(address)?;
    scope.prepare_checked(&SockRef::from(&socket))?;
    meshway_transport::raw::apply_socket_options(&SockRef::from(&socket), options)?;
    if let Some(bind) = bind {
        scope.prepare_checked(&SockRef::from(&socket))?;
        socket.bind(bind)?;
    }
    scope.prepare_checked(&SockRef::from(&socket))?;
    let stream = tokio::select! { biased;
        _ = scope.ended() => return Err(Error::Expired),
        result = tokio::time::timeout(timeout, socket.connect(address)) => result.map_err(|_| Error::Io(io::ErrorKind::TimedOut.into()))??,
    };
    scope.check()?;
    adopt(stream, scope, capacity, options, tcp)
}

fn adopt(
    stream: TcpStream,
    scope: ProtectionScope,
    capacity: OwnedSemaphorePermit,
    options: &SocketOptions,
    tcp: &TcpOptions,
) -> Result<ProtectedTcp> {
    scope.prepare_checked(&SockRef::from(&stream))?;
    let stream = meshway_transport_tcp::TcpTransport::adopt(stream, options, tcp)?.into_inner();
    let local = stream.local_addr()?;
    let peer = stream.peer_addr()?;
    Ok(ProtectedTcp {
        owned: Owned::new(stream, scope, capacity)?,
        local,
        peer,
    })
}

pub(super) struct Wakers {
    pub read: futures_util::task::AtomicWaker,
    pub write: futures_util::task::AtomicWaker,
}

impl Wakers {
    fn wake(&self) {
        self.read.wake();
        self.write.wake();
    }
}

struct Clear<T> {
    socket: Arc<Mutex<Option<T>>>,
    wakers: Arc<Wakers>,
    capacity: Arc<Mutex<Option<OwnedSemaphorePermit>>>,
}
impl<T> Drop for Clear<T> {
    fn drop(&mut self) {
        if let Ok(mut socket) = self.socket.lock() {
            socket.take();
        }
        if let Ok(mut capacity) = self.capacity.lock() {
            capacity.take();
        }
        self.wakers.wake();
    }
}

pub(super) struct Owned<T> {
    pub(super) socket: Arc<Mutex<Option<T>>>,
    pub(super) scope: ProtectionScope,
    pub(super) wakers: Arc<Wakers>,
    closed: CancellationToken,
    capacity: Arc<Mutex<Option<OwnedSemaphorePermit>>>,
}

impl<T: Send + 'static> Owned<T> {
    pub(super) fn new(
        socket: T,
        scope: ProtectionScope,
        capacity: OwnedSemaphorePermit,
    ) -> Result<Self> {
        let socket = Arc::new(Mutex::new(Some(socket)));
        let capacity = Arc::new(Mutex::new(Some(capacity)));
        let wakers = Arc::new(Wakers {
            read: futures_util::task::AtomicWaker::new(),
            write: futures_util::task::AtomicWaker::new(),
        });
        let clear = Clear {
            socket: socket.clone(),
            wakers: wakers.clone(),
            capacity: capacity.clone(),
        };
        let closed = CancellationToken::new();
        let done = closed.clone();
        let lifetime = scope.clone();
        scope.shared.tasks.spawn(async move {
            let _clear = clear;
            tokio::select! { biased;
                _ = lifetime.ended() => {},
                _ = done.cancelled() => {},
            }
        })?;
        Ok(Self {
            socket,
            scope,
            wakers,
            closed,
            capacity,
        })
    }
}

impl<T> Drop for Owned<T> {
    fn drop(&mut self) {
        self.closed.cancel();
        if let Ok(mut socket) = self.socket.lock() {
            socket.take();
        }
        if let Ok(mut capacity) = self.capacity.lock() {
            capacity.take();
        }
        self.wakers.wake();
    }
}

pub struct ProtectedTcp {
    owned: Owned<TcpStream>,
    local: SocketAddr,
    peer: SocketAddr,
}

impl fmt::Debug for ProtectedTcp {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ProtectedTcp").finish_non_exhaustive()
    }
}

impl ProtectedTcp {
    pub fn local_addr(&self) -> io::Result<SocketAddr> {
        self.owned.scope.check().map_err(io_error)?;
        Ok(self.local)
    }
    pub fn peer_addr(&self) -> io::Result<SocketAddr> {
        self.owned.scope.check().map_err(io_error)?;
        Ok(self.peer)
    }

    fn with<T>(
        &self,
        call: impl FnOnce(&mut TcpStream) -> Poll<io::Result<T>>,
    ) -> Poll<io::Result<T>> {
        let mut socket = match self.owned.socket.lock() {
            Ok(socket) => socket,
            Err(_) => return Poll::Ready(Err(io_error(Error::Worker))),
        };
        let Some(socket) = socket.as_mut() else {
            return Poll::Ready(Err(io_error(Error::Expired)));
        };
        if let Err(error) = self.owned.scope.prepare(&SockRef::from(&*socket)) {
            return Poll::Ready(Err(error));
        }
        call(socket)
    }
}

impl AsyncRead for ProtectedTcp {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buffer: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        self.owned.wakers.read.register(cx.waker());
        self.with(|stream| Pin::new(stream).poll_read(cx, buffer))
    }
}

impl AsyncWrite for ProtectedTcp {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buffer: &[u8],
    ) -> Poll<io::Result<usize>> {
        self.owned.wakers.write.register(cx.waker());
        self.with(|stream| Pin::new(stream).poll_write(cx, buffer))
    }
    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        self.owned.wakers.write.register(cx.waker());
        self.with(|stream| Pin::new(stream).poll_flush(cx))
    }
    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        self.owned.wakers.write.register(cx.waker());
        self.with(|stream| Pin::new(stream).poll_shutdown(cx))
    }
}

pub struct ProtectedListener {
    owned: Owned<TcpListener>,
    options: SocketOptions,
    tcp: TcpOptions,
}

impl ProtectedListener {
    pub fn local_addr(&self) -> io::Result<SocketAddr> {
        self.owned.scope.check().map_err(io_error)?;
        self.owned
            .socket
            .lock()
            .map_err(|_| io_error(Error::Worker))?
            .as_ref()
            .ok_or_else(|| io_error(Error::Expired))?
            .local_addr()
    }

    pub async fn accept(&self) -> Result<(ProtectedTcp, SocketAddr)> {
        let accepted = std::future::poll_fn(|cx| {
            self.owned.wakers.read.register(cx.waker());
            let socket = self
                .owned
                .socket
                .lock()
                .map_err(|_| io_error(Error::Worker))?;
            let listener = socket.as_ref().ok_or_else(|| io_error(Error::Expired))?;
            self.owned.scope.prepare(&SockRef::from(listener))?;
            listener.poll_accept(cx)
        });
        let (stream, peer) = tokio::select! { biased;
            _ = self.owned.scope.ended() => return Err(Error::Expired),
            result = accepted => result?,
        };
        let mark = SockRef::from(&stream).mark()?;
        let owned = self.owned.scope.shared.mark;
        if mark & owned.mask != owned.value {
            return Err(Error::Conflict);
        }
        let capacity = self.owned.scope.capacity()?;
        Ok((
            adopt(
                stream,
                self.owned.scope.clone(),
                capacity,
                &self.options,
                &self.tcp,
            )?,
            peer,
        ))
    }
}
