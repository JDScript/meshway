//! The single protected UDP primitive, shared by Mesh endpoints and bootstrap
//! DNS. Only snapshot-minted factory methods expose Mesh sockets publicly.
use super::{
    protection::{Scope, io_error},
    sockets::Owned,
};
use crate::{Error, Result};
use meshway_transport::raw::{Datagram, DatagramRead, DatagramSend};
use meshway_transport_spec::SocketOptions;
use meshway_transport_udp::UdpTransport;
use socket2::{Domain, Protocol, SockRef, Socket, Type};
use std::{
    fmt, io,
    net::SocketAddr,
    task::{Context, Poll},
};
use tokio::net::UdpSocket;

/// One unconnected, protected descriptor. It exposes no raw socket/FD escape.
pub struct ProtectedDatagram {
    owned: Owned<UdpTransport>,
    local: SocketAddr,
    targets: Option<Vec<SocketAddr>>,
    pub(super) traversal: Option<std::sync::Arc<super::traversal::Registry>>,
}

impl fmt::Debug for ProtectedDatagram {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ProtectedDatagram").finish_non_exhaustive()
    }
}

/// A bound address-family endpoint and only the resolved addresses of the exact
/// published dial candidate. Resolution neither authenticates nor selects one.
pub struct DatagramCandidate {
    pub endpoint: ProtectedDatagram,
    pub targets: Vec<SocketAddr>,
}

impl ProtectedDatagram {
    pub(super) fn scope(&self) -> Scope {
        self.owned.scope.clone()
    }
    pub(super) fn bind(
        scope: Scope,
        local: SocketAddr,
        options: &SocketOptions,
        targets: Option<Vec<SocketAddr>>,
    ) -> Result<Self> {
        let capacity = scope.capacity()?;
        let socket = Socket::new(Domain::for_address(local), Type::DGRAM, Some(Protocol::UDP))?;
        socket.set_nonblocking(true)?;
        if local.is_ipv6() {
            // Separate address-family endpoints; never accept mapped-v4 sources
            // under a different source-address representation.
            socket.set_only_v6(true)?;
        }
        scope.prepare_checked(&SockRef::from(&socket))?;
        meshway_transport::raw::apply_socket_options(&SockRef::from(&socket), options)?;
        scope.prepare_checked(&SockRef::from(&socket))?;
        socket.bind(&local.into())?;
        scope.prepare_checked(&SockRef::from(&socket))?;
        let endpoint = UdpTransport::adopt_batch(UdpSocket::from_std(socket.into())?, options)?;
        scope.prepare_checked(&SockRef::from(&endpoint))?;
        let local = endpoint.local_addr()?;
        Ok(Self {
            owned: Owned::new(endpoint, scope, capacity)?,
            local,
            targets,
            traversal: None,
        })
    }

    pub fn local_addr(&self) -> io::Result<SocketAddr> {
        self.owned.scope.check().map_err(io_error)?;
        Ok(self.local)
    }

    fn with<T>(
        &self,
        call: impl FnOnce(&UdpTransport) -> Poll<io::Result<T>>,
    ) -> Poll<io::Result<T>> {
        let socket = self
            .owned
            .socket
            .lock()
            .map_err(|_| io_error(Error::Worker))?;
        let endpoint = socket.as_ref().ok_or_else(|| io_error(Error::Expired))?;
        self.owned.scope.prepare(&SockRef::from(endpoint))?;
        call(endpoint)
    }
}

impl Datagram for ProtectedDatagram {
    fn poll_send_batch(
        &self,
        cx: &mut Context<'_>,
        packets: &[DatagramSend<'_>],
    ) -> Poll<io::Result<usize>> {
        for packet in packets {
            if self
                .targets
                .as_ref()
                .is_some_and(|targets| !targets.contains(&packet.peer))
                && !self
                    .traversal
                    .as_ref()
                    .is_some_and(|registry| registry.allows(packet.peer))
            {
                return Poll::Ready(Err(io_error(Error::Target)));
            }
        }
        self.owned.wakers.write.register(cx.waker());
        // UdpTransport performs at most one data syscall per batch poll,
        // returning Pending before retrying an unsupported facility.
        self.with(|endpoint| endpoint.poll_send_batch(cx, packets))
    }

    fn poll_recv_batch(
        &self,
        cx: &mut Context<'_>,
        buffers: &mut [Vec<u8>],
    ) -> Poll<io::Result<Vec<DatagramRead>>> {
        self.owned.wakers.read.register(cx.waker());
        self.with(|endpoint| endpoint.poll_recv_batch(cx, buffers))
    }
    fn local_addr(&self) -> io::Result<SocketAddr> {
        self.local_addr()
    }

    fn poll_send_to(
        &self,
        cx: &mut Context<'_>,
        bytes: &[u8],
        peer: SocketAddr,
    ) -> Poll<io::Result<usize>> {
        if self
            .targets
            .as_ref()
            .is_some_and(|targets| !targets.contains(&peer))
            && !self
                .traversal
                .as_ref()
                .is_some_and(|registry| registry.allows(peer))
        {
            return Poll::Ready(Err(io_error(Error::Target)));
        }
        self.owned.wakers.write.register(cx.waker());
        self.with(|endpoint| endpoint.poll_send_to(cx, bytes, peer))
    }

    fn poll_recv_from(
        &self,
        cx: &mut Context<'_>,
        bytes: &mut [u8],
    ) -> Poll<io::Result<DatagramRead>> {
        self.owned.wakers.read.register(cx.waker());
        self.with(|endpoint| endpoint.poll_recv_from(cx, bytes))
    }
}

impl Drop for ProtectedDatagram {
    fn drop(&mut self) {
        if let Some(registry) = &self.traversal {
            registry.closed.cancel();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::linux::{
        protection::{Mark, Shared},
        tasks::Tasks,
    };
    use std::sync::{Arc, RwLock};
    use tokio::sync::Semaphore;
    use tokio_util::sync::CancellationToken;

    fn shared() -> Arc<Shared> {
        assert_eq!(
            std::env::var("MESHWAY_UNDERLAY_ISOLATED").as_deref(),
            Ok("1")
        );
        Arc::new(Shared {
            resources: super::super::protection::Resources::new(128),
            mark: Mark {
                value: 0x100,
                mask: 0x100,
            },
            selectors: Vec::new(),
            conflict: CancellationToken::new(),
            network_changes: tokio::sync::watch::channel(0).0,
            tasks: Tasks::new(16),
            sockets: Arc::new(Semaphore::new(8)),
            data_sockets: Arc::new(Semaphore::new(8)),
            attempts: Arc::new(Semaphore::new(8)),
            resolutions: Arc::new(Semaphore::new(8)),
            execution: RwLock::new(None),
        })
    }

    #[tokio::test]
    #[ignore = "requires an isolated Linux namespace and NET_ADMIN"]
    async fn actual_udp_mark_truncation_and_unpolled_owner_release() {
        for local in ["127.0.0.1:0", "[::1]:0"] {
            let shared = shared();
            let peer = UdpSocket::bind(local).await.unwrap();
            let remote = peer.local_addr().unwrap();
            let endpoint = ProtectedDatagram::bind(
                shared.bootstrap(),
                local.parse().unwrap(),
                &Default::default(),
                Some(vec![remote]),
            )
            .unwrap();
            let address = endpoint.local_addr().unwrap();
            let mark = || {
                let socket = endpoint.owned.socket.lock().unwrap();
                SockRef::from(socket.as_ref().unwrap()).mark().unwrap()
            };
            assert_eq!(mark(), 0x100);
            {
                let socket = endpoint.owned.socket.lock().unwrap();
                SockRef::from(socket.as_ref().unwrap())
                    .set_mark(0x20)
                    .unwrap();
            }
            meshway_transport::raw::send_to(&endpoint, b"protected", remote)
                .await
                .unwrap();
            assert_eq!(mark(), 0x120);
            let mut buffer = [0; 32];
            let (len, source) = peer.recv_from(&mut buffer).await.unwrap();
            assert_eq!(source, address);
            assert_eq!(&buffer[..len], b"protected");
            {
                let socket = endpoint.owned.socket.lock().unwrap();
                SockRef::from(socket.as_ref().unwrap())
                    .set_mark(0x40)
                    .unwrap();
            }
            peer.send_to(&[9; 128], address).await.unwrap();
            let read = meshway_transport::raw::recv_from(&endpoint, &mut buffer)
                .await
                .unwrap();
            assert!(read.truncated);
            assert_eq!(read.original_len, Some(128));
            assert_eq!(read.source, remote);
            assert_eq!(mark(), 0x140);
            {
                let socket = endpoint.owned.socket.lock().unwrap();
                SockRef::from(socket.as_ref().unwrap())
                    .set_mark(0x80)
                    .unwrap();
            }
            let packets = [
                DatagramSend {
                    payload: b"one",
                    peer: remote,
                },
                DatagramSend {
                    payload: b"two",
                    peer: remote,
                },
            ];
            let mut sent = 0;
            while sent < packets.len() {
                sent += std::future::poll_fn(|cx| endpoint.poll_send_batch(cx, &packets[sent..]))
                    .await
                    .unwrap();
            }
            assert_eq!(mark(), 0x180);
            for expected in [b"one", b"two"] {
                let (n, source) = peer.recv_from(&mut buffer).await.unwrap();
                assert_eq!(source, address);
                assert_eq!(&buffer[..n], expected);
                peer.send_to(expected, address).await.unwrap();
            }
            let mut buffers = vec![vec![0; 32]; 2];
            let mut received = 0;
            while received < 2 {
                let reads = std::future::poll_fn(|cx| endpoint.poll_recv_batch(cx, &mut buffers))
                    .await
                    .unwrap();
                for (read, buffer) in reads.iter().zip(&buffers) {
                    assert_eq!(read.source, remote);
                    assert_eq!(&buffer[..read.copied_len], [b"one", b"two"][received]);
                    received += 1;
                }
            }
            let mut waiting = Box::pin(meshway_transport::raw::recv_from(&endpoint, &mut buffer));
            assert!(futures_util::poll!(waiting.as_mut()).is_pending());
            shared.conflict.cancel();
            // The task owns closure even though the borrowing read future lives
            // on without another poll. No FD alias escapes ProtectedDatagram.
            shared.tasks.shutdown().await.unwrap();
            assert!(endpoint.owned.socket.lock().unwrap().is_none());
            assert!(
                std::future::poll_fn(|cx| endpoint.poll_send_batch(cx, &packets))
                    .await
                    .is_err()
            );
            assert_eq!(shared.sockets.available_permits(), 8);
            assert!(
                meshway_transport::raw::send_to(&endpoint, b"late", remote)
                    .await
                    .is_err()
            );
            drop(waiting);
        }
    }
}
