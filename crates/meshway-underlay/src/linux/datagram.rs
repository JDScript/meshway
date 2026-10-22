//! The single protected UDP primitive, shared by Mesh endpoints and bootstrap
//! DNS. Only content-scoped factory methods expose Mesh sockets publicly.
use super::{
    protection::{ProtectionScope, io_error},
    sockets::Owned,
};
use crate::{Error, Result};
use meshway_transport::raw::{Datagram, DatagramRead, DatagramSend};
use meshway_transport::spec::SocketOptions;
use meshway_transport::udp::UdpTransport;
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
    buffers: meshway_transport::udp::Buffers,
    targets: Option<Vec<SocketAddr>>,
    pub(super) traversal: Option<std::sync::Arc<super::traversal::Registry>>,
    /// Sources of roaming peers, beyond `targets`.
    roamed: std::sync::Mutex<Roamed>,
}

/// Challenged sources, and authenticated channel sources, one restricted
/// endpoint may hold at once (each kind on its own).
const MAX_ROAMED: usize = 4;

struct Grant {
    address: SocketAddr,
    lifetime: std::sync::Arc<dyn meshway_transport::SendLifetime>,
}

/// The live send grants of a restricted endpoint's roaming peers: sources
/// being challenged, and sources channels authenticated and now follow.
#[derive(Default)]
struct Roamed {
    challenges: Vec<Grant>,
    sources: Vec<Grant>,
}

impl Roamed {
    fn allows(&self, peer: SocketAddr) -> bool {
        self.challenges
            .iter()
            .chain(&self.sources)
            .any(|grant| grant.address == peer && grant.lifetime.is_current())
    }

    /// Add one grant of its kind: ended grants leave first, and a live one
    /// for the same address is replaced, never added beside it.
    fn add(
        grants: &mut Vec<Grant>,
        address: SocketAddr,
        lifetime: std::sync::Arc<dyn meshway_transport::SendLifetime>,
    ) -> Result<()> {
        grants.retain(|grant| grant.lifetime.is_current() && grant.address != address);
        if grants.len() >= MAX_ROAMED {
            return Err(Error::Capacity);
        }
        grants.push(Grant { address, lifetime });
        Ok(())
    }
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
    pub(super) fn scope(&self) -> ProtectionScope {
        self.owned.scope.clone()
    }
    pub(super) fn bind(
        scope: ProtectionScope,
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
        socket.bind(&local.into()).map_err(|source| Error::Bind {
            address: local,
            source,
        })?;
        scope.prepare_checked(&SockRef::from(&socket))?;
        let endpoint = UdpTransport::adopt_batch(UdpSocket::from_std(socket.into())?, options)?;
        scope.prepare_checked(&SockRef::from(&endpoint))?;
        let local = endpoint.local_addr()?;
        let buffers = endpoint.buffers();
        Ok(Self {
            owned: Owned::new(endpoint, scope, capacity)?,
            local,
            buffers,
            targets,
            traversal: None,
            roamed: std::sync::Mutex::default(),
        })
    }

    /// An unrestricted endpoint (a listener) sends anywhere; a dial or
    /// traversal endpoint only to its targets, its live traversal recipients
    /// and the live sources of roaming peers.
    fn allows(&self, peer: SocketAddr) -> bool {
        let Some(targets) = &self.targets else {
            return true;
        };
        targets.contains(&peer)
            || self
                .traversal
                .as_ref()
                .is_some_and(|registry| registry.allows(peer))
            || self.roamed.lock().is_ok_and(|roamed| roamed.allows(peer))
    }

    fn grant(
        &self,
        peer: SocketAddr,
        lifetime: std::sync::Arc<dyn meshway_transport::SendLifetime>,
        kind: impl FnOnce(&mut Roamed) -> &mut Vec<Grant>,
    ) -> io::Result<()> {
        self.owned.scope.check().map_err(io_error)?;
        if self.targets.is_none() {
            return Ok(());
        }
        if peer.is_ipv4() != self.local.is_ipv4()
            || meshway_control::connectivity::udp_recipient(&peer.to_string()).is_err()
            || !lifetime.is_current()
        {
            return Err(io_error(Error::Target));
        }
        let mut roamed = self.roamed.lock().map_err(|_| io_error(Error::Worker))?;
        Roamed::add(kind(&mut roamed), peer, lifetime).map_err(io_error)
    }

    pub fn local_addr(&self) -> io::Result<SocketAddr> {
        self.owned.scope.check().map_err(io_error)?;
        Ok(self.local)
    }

    /// Socket buffer sizes the kernel granted when this endpoint was bound.
    pub fn buffers(&self) -> meshway_transport::udp::Buffers {
        self.buffers
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
            if !self.allows(packet.peer) {
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
        if !self.allows(peer) {
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

    /// The route this descriptor's datagrams to `peer` take: a transient
    /// socket with the same protection and local address, connected (which
    /// sends nothing) only to read the route MTU, including a learned PMTU.
    fn buffers(&self) -> Option<meshway_transport::udp::Buffers> {
        Some(self.buffers)
    }
    /// At most [`MAX_ROAMED`] live challenge grants, each for its own
    /// lifetime and of this endpoint's address family; a restricted endpoint
    /// gains no other recipient.
    fn authorize_challenge(
        &self,
        peer: SocketAddr,
        lifetime: std::sync::Arc<dyn meshway_transport::SendLifetime>,
    ) -> io::Result<()> {
        self.grant(peer, lifetime, |roamed| &mut roamed.challenges)
    }
    /// At most [`MAX_ROAMED`] live channel sources, apart from challenges.
    fn authorize_source(
        &self,
        peer: SocketAddr,
        lifetime: std::sync::Arc<dyn meshway_transport::SendLifetime>,
    ) -> io::Result<()> {
        self.grant(peer, lifetime, |roamed| &mut roamed.sources)
    }
    fn path_mtu(&self, peer: SocketAddr) -> Option<u16> {
        if peer.is_ipv4() != self.local.is_ipv4() {
            return None;
        }
        let probe = || -> Result<u16> {
            let socket = Socket::new(Domain::for_address(peer), Type::DGRAM, Some(Protocol::UDP))?;
            if peer.is_ipv6() {
                socket.set_only_v6(true)?;
            }
            self.owned.scope.prepare_checked(&SockRef::from(&socket))?;
            if !self.local.ip().is_unspecified() {
                socket.bind(&SocketAddr::new(self.local.ip(), 0).into())?;
            }
            socket.connect(&peer.into())?;
            self.owned.scope.prepare_checked(&SockRef::from(&socket))?;
            Ok(meshway_transport::udp::connected_path_mtu(
                &socket,
                peer.is_ipv4(),
            )?)
        };
        probe().ok()
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
mod grant_tests {
    use super::*;
    use std::sync::{
        Arc,
        atomic::{AtomicBool, Ordering::SeqCst},
    };

    #[derive(Debug, Default)]
    struct Revocable(AtomicBool);
    impl meshway_transport::SendLifetime for Revocable {
        fn is_current(&self) -> bool {
            !self.0.load(SeqCst)
        }
        fn ended(
            self: Arc<Self>,
        ) -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send>> {
            Box::pin(std::future::pending())
        }
    }

    /// F6a B3: a channel that follows its peer six times holds one grant,
    /// for its current source, each move revoking the last; repeating an
    /// address replaces its grant, and live challenges never take a
    /// channel source's room (nor the reverse).
    #[test]
    fn a_channel_follows_any_number_of_moves_with_one_grant() {
        let mut roamed = Roamed::default();
        for _ in 0..MAX_ROAMED {
            let address = format!("192.0.2.{}:9", roamed.challenges.len() + 1);
            let lifetime = Arc::new(Revocable::default());
            Roamed::add(&mut roamed.challenges, address.parse().unwrap(), lifetime).unwrap();
        }
        let mut previous: Option<Arc<Revocable>> = None;
        for step in 1..=6 {
            let address: SocketAddr = format!("198.51.100.{step}:9").parse().unwrap();
            let lifetime = Arc::new(Revocable::default());
            Roamed::add(&mut roamed.sources, address, lifetime.clone()).unwrap();
            if let Some(old) = previous.replace(lifetime) {
                old.0.store(true, SeqCst);
            }
            assert!(roamed.allows(address));
            if step > 1 {
                let earlier: SocketAddr = format!("198.51.100.{}:9", step - 1).parse().unwrap();
                assert!(
                    !roamed.allows(earlier),
                    "the move revoked step {step}'s predecessor"
                );
            }
        }
        let current: SocketAddr = "198.51.100.6:9".parse().unwrap();
        for _ in 0..8 {
            Roamed::add(&mut roamed.sources, current, Arc::new(Revocable::default())).unwrap();
        }
        assert_eq!(
            roamed
                .sources
                .iter()
                .filter(|grant| grant.address == current)
                .count(),
            1
        );
        assert_eq!(roamed.challenges.len(), MAX_ROAMED);
        assert!(matches!(
            Roamed::add(
                &mut roamed.challenges,
                "192.0.2.99:9".parse().unwrap(),
                Arc::new(Revocable::default())
            ),
            Err(Error::Capacity)
        ));
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
            network: Default::default(),
            tasks: Tasks::new(16),
            sockets: Arc::new(Semaphore::new(8)),
            data_sockets: Arc::new(Semaphore::new(8)),
            attempts: Arc::new(Semaphore::new(8)),
            resolutions: Arc::new(Semaphore::new(8)),
            execution: RwLock::new(None),
            changes: tokio::sync::watch::channel(None).0,
            cleanups: tokio::sync::watch::channel(None).0,
            ending: Default::default(),
            exit: Default::default(),
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
