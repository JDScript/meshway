//! I/O injection only. Production opening, resolution and socket protection have
//! exactly one implementation, in meshway-underlay.
use crate::Error;
use futures_util::future::BoxFuture;
use meshway_control::ExecutionSnapshot;
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

pub trait SocketFactory: Send + Sync + 'static {
    fn connect<'a>(
        &'a self,
        snapshot: &'a ExecutionSnapshot,
        key: &'a str,
        endpoint: usize,
    ) -> BoxFuture<'a, Result<AcceptedSocket, Error>>;
    fn listen<'a>(
        &'a self,
        snapshot: &'a ExecutionSnapshot,
        key: &'a str,
    ) -> BoxFuture<'a, Result<Box<dyn ListenerSocket>, Error>>;

    fn datagram_listener<'a>(
        &'a self,
        _snapshot: &'a ExecutionSnapshot,
        _key: &'a str,
    ) -> BoxFuture<'a, Result<Box<dyn Datagram>, Error>> {
        Box::pin(async { Err(Error::Configuration) })
    }

    fn datagram_candidate<'a>(
        &'a self,
        _snapshot: &'a ExecutionSnapshot,
        _key: &'a str,
        _endpoint: usize,
    ) -> BoxFuture<'a, Result<Vec<DatagramSocket>, Error>> {
        Box::pin(async { Err(Error::Configuration) })
    }
}

#[cfg(target_os = "linux")]
impl SocketFactory for meshway_underlay::SocketFactory {
    fn datagram_listener<'a>(
        &'a self,
        snapshot: &'a ExecutionSnapshot,
        key: &'a str,
    ) -> BoxFuture<'a, Result<Box<dyn Datagram>, Error>> {
        Box::pin(async move {
            let endpoint = self
                .datagram_listener(snapshot, key)
                .await
                .map_err(|_| Error::Underlay)?;
            Ok(Box::new(endpoint) as Box<dyn Datagram>)
        })
    }

    fn datagram_candidate<'a>(
        &'a self,
        snapshot: &'a ExecutionSnapshot,
        key: &'a str,
        endpoint: usize,
    ) -> BoxFuture<'a, Result<Vec<DatagramSocket>, Error>> {
        Box::pin(async move {
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
    fn connect<'a>(
        &'a self,
        snapshot: &'a ExecutionSnapshot,
        key: &'a str,
        endpoint: usize,
    ) -> BoxFuture<'a, Result<AcceptedSocket, Error>> {
        Box::pin(async move {
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
        snapshot: &'a ExecutionSnapshot,
        key: &'a str,
    ) -> BoxFuture<'a, Result<Box<dyn ListenerSocket>, Error>> {
        Box::pin(async move {
            let handle = self.listener(snapshot, key).map_err(|_| Error::Underlay)?;
            let listener = self.listen(&handle).await.map_err(|_| Error::Underlay)?;
            Ok(Box::new(listener) as Box<dyn ListenerSocket>)
        })
    }
}

#[cfg(target_os = "linux")]
impl ListenerSocket for meshway_underlay::ProtectedListener {
    fn local_addr(&self) -> Result<SocketAddr, Error> {
        Ok(self.local_addr()?)
    }
    fn accept(&self) -> BoxFuture<'_, Result<AcceptedSocket, Error>> {
        Box::pin(async move {
            let (stream, peer) = self.accept().await.map_err(|_| Error::Underlay)?;
            Ok(AcceptedSocket {
                local: stream.local_addr()?,
                peer,
                stream: Box::new(stream),
            })
        })
    }
}
