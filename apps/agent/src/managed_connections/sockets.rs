//! Outer TCP socket boundary, shared by TLS/TCP and TLS/WebSocket. Platform
//! protection must happen before connect/listen, never as a global IP bypass.
use meshway_dataplane::sockets::{
    NativeListener, NativeSocketFactory, NativeSocketPurpose, NativeTcp, NativeUdp, SocketFuture,
};
use std::{io, net::SocketAddr};
use std::{
    pin::Pin,
    task::{Context, Poll},
};
use tokio::net::{TcpListener, TcpSocket, TcpStream};

pub(crate) enum Purpose<'a> {
    Listener {
        listener_id: &'a str,
    },
    Dial {
        candidate_id: &'a str,
        peer_node_id: &'a str,
    },
}

/// Controller-driven platform adapters can set SO_MARK or an equivalent
/// per-socket exclusion here. No process-wide or destination-wide exemptions.
pub(crate) trait Protection: Send + Sync {
    fn check_active(&self) -> io::Result<()> {
        Ok(())
    }
    fn cancelled(&self) -> SocketFuture<'_, ()> {
        Box::pin(std::future::pending())
    }
    fn prepare(&self, address: SocketAddr, purpose: Purpose<'_>) -> io::Result<TcpSocket>;
    fn accepted(&self, socket: &TcpStream, listener_id: &str) -> io::Result<()>;

    fn resolve<'a>(&'a self, host: &'a str, port: u16) -> SocketFuture<'a, Vec<SocketAddr>> {
        Box::pin(async move {
            tokio::net::lookup_host((host, port))
                .await
                .map(|a| a.collect())
        })
    }
    fn listen(&self, address: SocketAddr, id: &str) -> io::Result<OuterListener> {
        let socket = self.prepare(address, Purpose::Listener { listener_id: id })?;
        socket.set_reuseaddr(true)?;
        socket.bind(address)?;
        Ok(OuterListener::System(socket.listen(128)?))
    }
    fn dial<'a>(
        &'a self,
        address: SocketAddr,
        id: &'a str,
        peer: &'a str,
    ) -> SocketFuture<'a, OuterTcp> {
        Box::pin(async move {
            let socket = self.prepare(
                address,
                Purpose::Dial {
                    candidate_id: id,
                    peer_node_id: peer,
                },
            )?;
            Ok(OuterTcp::System(socket.connect(address).await?))
        })
    }
    // Implementors must explicitly select a native factory; a protected adapter
    // cannot accidentally inherit an unmarked native fallback.
    fn native(&self) -> &dyn NativeSocketFactory;
}

/// Current Mesh-prefix-only execution uses the system underlay. Full IP capture
/// must install a platform protection adapter before activating these sockets.
pub(crate) struct SystemUnderlay;
impl Protection for SystemUnderlay {
    fn native(&self) -> &dyn NativeSocketFactory {
        &meshway_dataplane::sockets::SystemSocketFactory
    }
    fn prepare(&self, address: SocketAddr, purpose: Purpose<'_>) -> io::Result<TcpSocket> {
        // Preserve IDs at this boundary for future platform ownership/logical
        // binding. They are not destination exemptions or business permissions.
        match purpose {
            Purpose::Listener { listener_id } => {
                let _ = listener_id;
            }
            Purpose::Dial {
                candidate_id,
                peer_node_id,
            } => {
                let _ = (candidate_id, peer_node_id);
            }
        }
        if address.is_ipv4() {
            TcpSocket::new_v4()
        } else {
            TcpSocket::new_v6()
        }
    }
    fn accepted(&self, _: &TcpStream, _: &str) -> io::Result<()> {
        Ok(())
    }
}

pub(crate) fn listen(
    protection: &dyn Protection,
    address: SocketAddr,
    id: &str,
) -> io::Result<OuterListener> {
    protection.listen(address, id)
}

pub(crate) async fn dial(
    protection: &dyn Protection,
    address: SocketAddr,
    id: &str,
    peer: &str,
) -> io::Result<OuterTcp> {
    protection.dial(address, id, peer).await
}

pub(crate) enum OuterListener {
    System(TcpListener),
    Protected(crate::underlay::ProtectedListener),
}
pub(crate) enum OuterTcp {
    System(TcpStream),
    Protected(crate::underlay::ProtectedTcp),
}
impl OuterListener {
    pub(crate) async fn accept(&self) -> io::Result<(OuterTcp, SocketAddr)> {
        match self {
            Self::System(s) => s.accept().await.map(|(s, a)| (OuterTcp::System(s), a)),
            Self::Protected(s) => s.accept().await.map(|(s, a)| (OuterTcp::Protected(s), a)),
        }
    }
}
impl OuterTcp {
    pub(crate) fn accepted(&self, protection: &dyn Protection, id: &str) -> io::Result<()> {
        match self {
            Self::System(s) => protection.accepted(s, id),
            Self::Protected(_) => Ok(()),
        }
    }
    pub(crate) fn peer_addr(&self) -> io::Result<SocketAddr> {
        match self {
            Self::System(s) => s.peer_addr(),
            Self::Protected(s) => s.peer_addr(),
        }
    }
    pub(crate) fn set_nodelay(&self, value: bool) -> io::Result<()> {
        match self {
            Self::System(s) => s.set_nodelay(value),
            Self::Protected(s) => s.set_nodelay(value),
        }
    }
}
impl tokio::io::AsyncRead for OuterTcp {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        b: &mut tokio::io::ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        match self.get_mut() {
            Self::System(s) => Pin::new(s).poll_read(cx, b),
            Self::Protected(s) => Pin::new(s).poll_read(cx, b),
        }
    }
}
impl tokio::io::AsyncWrite for OuterTcp {
    fn poll_write(self: Pin<&mut Self>, cx: &mut Context<'_>, b: &[u8]) -> Poll<io::Result<usize>> {
        match self.get_mut() {
            Self::System(s) => Pin::new(s).poll_write(cx, b),
            Self::Protected(s) => Pin::new(s).poll_write(cx, b),
        }
    }
    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        match self.get_mut() {
            Self::System(s) => Pin::new(s).poll_flush(cx),
            Self::Protected(s) => Pin::new(s).poll_flush(cx),
        }
    }
    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        match self.get_mut() {
            Self::System(s) => Pin::new(s).poll_shutdown(cx),
            Self::Protected(s) => Pin::new(s).poll_shutdown(cx),
        }
    }
}

pub(super) struct NativeAdapter(pub std::sync::Arc<dyn Protection>);
impl NativeSocketFactory for NativeAdapter {
    fn check_active(&self) -> io::Result<()> {
        self.0.check_active()
    }
    fn udp(
        &self,
        bind: SocketAddr,
        buffer: usize,
        purpose: NativeSocketPurpose<'_>,
    ) -> io::Result<Box<dyn NativeUdp>> {
        self.0.native().udp(bind, buffer, purpose)
    }
    fn listen(
        &self,
        bind: SocketAddr,
        purpose: NativeSocketPurpose<'_>,
    ) -> io::Result<Box<dyn NativeListener>> {
        self.0.native().listen(bind, purpose)
    }
    fn dial<'a>(
        &'a self,
        endpoint: SocketAddr,
        purpose: NativeSocketPurpose<'a>,
    ) -> SocketFuture<'a, Box<dyn NativeTcp>> {
        self.0.native().dial(endpoint, purpose)
    }
}
