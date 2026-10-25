//! Platform-owned I/O injection: Linux Underlay and Apple Host each own their
//! physical descriptors; traversal recipient authorization is shared in Underlay.
use crate::Error;
use futures_util::future::BoxFuture;
use meshway_transport::raw::{Datagram, Stream};
use std::net::SocketAddr;

pub struct AcceptedSocket {
    pub stream: Box<dyn Stream>,
    pub local: SocketAddr,
    pub peer: SocketAddr,
}

pub trait ListenerSocket: Send + Sync {
    fn local_addr(&self) -> Result<SocketAddr, Error>;
    fn accept(&self) -> BoxFuture<'_, Result<AcceptedSocket, Error>>;
}

pub struct DatagramSocket {
    pub endpoint: Box<dyn Datagram>,
    /// Addresses resolved from one published candidate, all in this socket's
    /// address family. They are not authenticated peer identities.
    pub targets: Vec<SocketAddr>,
}

pub struct TraversalSocket {
    pub endpoint: Box<dyn Datagram>,
    pub authorization: std::sync::Arc<dyn DatagramAuthorization>,
}
pub trait DatagramAuthorization: Send + Sync + 'static {
    /// New content: the descriptor now carries `reference`. Earlier discovery
    /// and attempt targets end; selected channels keep only live lifetimes.
    fn rebind(
        &self,
        _reference: &meshway_control::connectivity::SocketReference,
    ) -> Result<(), Error> {
        Err(Error::Underlay)
    }
    fn authorize_stun<'a>(
        &'a self,
        context: &'a meshway_control::connectivity::DiscoveryContext,
        server: &'a str,
    ) -> BoxFuture<'a, Result<Vec<SocketAddr>, Error>>;
    fn authorize_attempt(
        &self,
        scope: &meshway_control::connectivity::AttemptScope,
    ) -> Result<(), Error>;
    /// A source a signed message of this attempt arrived from, other than
    /// the offer's recipient: the peer's NAT mapped it to another port.
    fn authorize_learned(
        &self,
        scope: &meshway_control::connectivity::AttemptScope,
        address: SocketAddr,
    ) -> Result<(), Error>;
    /// `address` is the offer's recipient or a source learned for `scope`.
    fn authorize_channel(
        &self,
        scope: &meshway_control::connectivity::AttemptScope,
        lifetime: std::sync::Arc<dyn meshway_transport::SendLifetime>,
        address: SocketAddr,
    ) -> Result<(), Error>;
}

pub trait SocketFactory: Send + Sync + 'static {
    fn traversal_listener<'a>(
        &'a self,
        _desired: &'a meshway_engine::Desired,
        _key: &'a str,
        _reference: meshway_control::connectivity::SocketReference,
    ) -> BoxFuture<'a, Result<TraversalSocket, Error>> {
        Box::pin(async { Err(Error::Configuration) })
    }
    fn bind_datagram_candidate<'a>(
        &'a self,
        _desired: &'a meshway_engine::Desired,
        _key: &'a str,
        _ipv4: bool,
        _reference: meshway_control::connectivity::SocketReference,
    ) -> BoxFuture<'a, Result<TraversalSocket, Error>> {
        Box::pin(async { Err(Error::Configuration) })
    }
    /// Dial one configured or peer-fact endpoint of the candidate `key`. The
    /// endpoint is selection input, never authority; the implementation still
    /// protects the socket for the candidate's resource.
    fn connect_endpoint<'a>(
        &'a self,
        desired: &'a meshway_engine::Desired,
        key: &'a str,
        endpoint: &'a meshway_configuration::execution::DialEndpoint,
    ) -> BoxFuture<'a, Result<AcceptedSocket, Error>>;
    fn listen<'a>(
        &'a self,
        desired: &'a meshway_engine::Desired,
        key: &'a str,
    ) -> BoxFuture<'a, Result<Box<dyn ListenerSocket>, Error>>;

    /// Datagram sockets for one endpoint of candidate `key`; see
    /// [`Self::connect_endpoint`].
    fn datagram_endpoint<'a>(
        &'a self,
        _desired: &'a meshway_engine::Desired,
        _key: &'a str,
        _endpoint: &'a meshway_configuration::execution::DialEndpoint,
    ) -> BoxFuture<'a, Result<Vec<DatagramSocket>, Error>> {
        Box::pin(async { Err(Error::Configuration) })
    }

    fn datagram_listener<'a>(
        &'a self,
        _desired: &'a meshway_engine::Desired,
        _key: &'a str,
    ) -> BoxFuture<'a, Result<Box<dyn Datagram>, Error>> {
        Box::pin(async { Err(Error::Configuration) })
    }

    /// The source address this Node's protected sockets take toward `remote`
    /// now; `None` without a route, or when the factory cannot tell.
    fn route_source<'a>(&'a self, _remote: SocketAddr) -> BoxFuture<'a, Option<std::net::IpAddr>> {
        Box::pin(async { None })
    }

    /// Whether `address` is assigned to this host now; `None` when the
    /// factory cannot tell (then nothing is retired for it).
    fn address_assigned<'a>(&'a self, _address: std::net::IpAddr) -> BoxFuture<'a, Option<bool>> {
        Box::pin(async { None })
    }

    /// The remotes this Node sends to now, in priority order: the host's
    /// network change classification follows the route toward each.
    fn observe_remotes(&self, _remotes: Vec<std::net::IpAddr>) {}
}

/// A listener's bind failure keeps its cause; every other protected-socket
/// failure stays opaque.
#[cfg(target_os = "linux")]
fn listener_error(key: &str, error: meshway_underlay::Error) -> Error {
    match error {
        meshway_underlay::Error::Bind { address, source } => Error::Bind {
            key: key.to_owned(),
            address,
            source,
        },
        _ => Error::Underlay,
    }
}

/// The Host's installed content, which must be exactly `desired`.
#[cfg(target_os = "linux")]
fn installed(
    factory: &meshway_underlay::SocketFactory,
    desired: &meshway_engine::Desired,
) -> Result<meshway_engine::Desired, Error> {
    let installed = factory.installed().map_err(|_| Error::Underlay)?;
    if installed.reference() != desired.reference() {
        return Err(Error::Authority);
    }
    Ok(installed)
}

#[cfg(target_os = "linux")]
impl SocketFactory for meshway_underlay::SocketFactory {
    fn traversal_listener<'a>(
        &'a self,
        desired: &'a meshway_engine::Desired,
        key: &'a str,
        reference: meshway_control::connectivity::SocketReference,
    ) -> BoxFuture<'a, Result<TraversalSocket, Error>> {
        Box::pin(async move {
            let snapshot = &installed(self, desired)?;
            let socket = self
                .traversal_listener(snapshot, key, reference)
                .await
                .map_err(|error| listener_error(key, error))?;
            Ok(TraversalSocket {
                endpoint: Box::new(socket.endpoint),
                authorization: std::sync::Arc::new(socket.authorization),
            })
        })
    }
    fn bind_datagram_candidate<'a>(
        &'a self,
        desired: &'a meshway_engine::Desired,
        key: &'a str,
        ipv4: bool,
        reference: meshway_control::connectivity::SocketReference,
    ) -> BoxFuture<'a, Result<TraversalSocket, Error>> {
        Box::pin(async move {
            let snapshot = &installed(self, desired)?;
            let socket = self
                .bind_datagram_candidate(snapshot, key, ipv4, reference)
                .await
                .map_err(|_| Error::Underlay)?;
            Ok(TraversalSocket {
                endpoint: Box::new(socket.endpoint),
                authorization: std::sync::Arc::new(socket.authorization),
            })
        })
    }
    fn datagram_listener<'a>(
        &'a self,
        desired: &'a meshway_engine::Desired,
        key: &'a str,
    ) -> BoxFuture<'a, Result<Box<dyn Datagram>, Error>> {
        Box::pin(async move {
            let snapshot = &installed(self, desired)?;
            let endpoint = self
                .datagram_listener(snapshot, key)
                .await
                .map_err(|error| listener_error(key, error))?;
            Ok(Box::new(endpoint) as Box<dyn Datagram>)
        })
    }

    fn datagram_endpoint<'a>(
        &'a self,
        desired: &'a meshway_engine::Desired,
        key: &'a str,
        endpoint: &'a meshway_configuration::execution::DialEndpoint,
    ) -> BoxFuture<'a, Result<Vec<DatagramSocket>, Error>> {
        Box::pin(async move {
            let snapshot = &installed(self, desired)?;
            Ok(self
                .datagram_candidate(snapshot, key, endpoint)
                .await
                .map_err(|_| Error::Underlay)?
                .into_iter()
                .map(|candidate| DatagramSocket {
                    endpoint: Box::new(candidate.endpoint),
                    targets: candidate.targets,
                })
                .collect())
        })
    }
    fn route_source<'a>(&'a self, remote: SocketAddr) -> BoxFuture<'a, Option<std::net::IpAddr>> {
        Box::pin(self.route_source(remote))
    }
    fn address_assigned<'a>(&'a self, address: std::net::IpAddr) -> BoxFuture<'a, Option<bool>> {
        Box::pin(self.address_assigned(address))
    }
    fn observe_remotes(&self, remotes: Vec<std::net::IpAddr>) {
        meshway_underlay::SocketFactory::observe_remotes(self, remotes);
    }
    fn connect_endpoint<'a>(
        &'a self,
        desired: &'a meshway_engine::Desired,
        key: &'a str,
        endpoint: &'a meshway_configuration::execution::DialEndpoint,
    ) -> BoxFuture<'a, Result<AcceptedSocket, Error>> {
        Box::pin(async move {
            let snapshot = &installed(self, desired)?;
            let handle = self
                .candidate(snapshot, key, endpoint)
                .map_err(|_| Error::Underlay)?;
            let stream = self.connect(&handle).await.map_err(|_| Error::Underlay)?;
            Ok(AcceptedSocket {
                local: stream.local_addr()?,
                peer: stream.peer_addr()?,
                stream: Box::new(stream),
            })
        })
    }
    fn listen<'a>(
        &'a self,
        desired: &'a meshway_engine::Desired,
        key: &'a str,
    ) -> BoxFuture<'a, Result<Box<dyn ListenerSocket>, Error>> {
        Box::pin(async move {
            let snapshot = &installed(self, desired)?;
            let handle = self.listener(snapshot, key).map_err(|_| Error::Underlay)?;
            let listener = self
                .listen(&handle)
                .await
                .map_err(|error| listener_error(key, error))?;
            Ok(Box::new(listener) as Box<dyn ListenerSocket>)
        })
    }
}

impl DatagramAuthorization for meshway_underlay::DatagramAuthorization {
    fn rebind(
        &self,
        reference: &meshway_control::connectivity::SocketReference,
    ) -> Result<(), Error> {
        self.rebind(reference).map_err(|_| Error::Underlay)
    }
    fn authorize_stun<'a>(
        &'a self,
        context: &'a meshway_control::connectivity::DiscoveryContext,
        server: &'a str,
    ) -> BoxFuture<'a, Result<Vec<SocketAddr>, Error>> {
        Box::pin(async move {
            self.authorize_stun(context, server)
                .await
                .map_err(|_| Error::Underlay)
        })
    }
    fn authorize_attempt(
        &self,
        scope: &meshway_control::connectivity::AttemptScope,
    ) -> Result<(), Error> {
        self.authorize_attempt(scope).map_err(|_| Error::Underlay)
    }
    fn authorize_learned(
        &self,
        scope: &meshway_control::connectivity::AttemptScope,
        address: SocketAddr,
    ) -> Result<(), Error> {
        self.authorize_learned(scope, address)
            .map_err(|_| Error::Underlay)
    }
    fn authorize_channel(
        &self,
        scope: &meshway_control::connectivity::AttemptScope,
        lifetime: std::sync::Arc<dyn meshway_transport::SendLifetime>,
        address: SocketAddr,
    ) -> Result<(), Error> {
        self.authorize_channel(scope, lifetime, address)
            .map_err(|_| Error::Underlay)
    }
}

#[cfg(target_os = "linux")]
impl ListenerSocket for meshway_underlay::ProtectedListener {
    fn local_addr(&self) -> Result<SocketAddr, Error> {
        Ok(self.local_addr()?)
    }
    fn accept(&self) -> BoxFuture<'_, Result<AcceptedSocket, Error>> {
        Box::pin(async move {
            // A host error (descriptors, memory) is retried by the listener's
            // owner; anything else means this protected listener ended.
            let (stream, peer) = self.accept().await.map_err(|error| match error {
                meshway_underlay::Error::Io(error) => Error::Io(error),
                meshway_underlay::Error::Capacity => Error::Capacity,
                error => {
                    tracing::warn!(%error, "Protected listener ended");
                    Error::Underlay
                }
            })?;
            Ok(AcceptedSocket {
                local: stream.local_addr()?,
                peer,
                stream: Box::new(stream),
            })
        })
    }
}
