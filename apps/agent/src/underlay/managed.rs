//! The same owning capability backs managed VLESS and core native carriers.
use super::{
    BootstrapResolver, ProtectedListener, ProtectedTcp, ProtectedUdp, Purpose, SocketFactory,
    SystemDnsSnapshot,
};
use crate::managed_connections::sockets::{self, Protection};
use meshway_dataplane::sockets::{
    NativeListener, NativeSocketFactory, NativeSocketPurpose, NativeTcp, NativeUdp, SocketFuture,
};
use std::{io, net::SocketAddr};
use tokio::net::{TcpSocket, TcpStream};

pub(crate) struct ManagedProtection {
    factory: SocketFactory,
    resolver: BootstrapResolver,
}
impl ManagedProtection {
    #[allow(dead_code)] // Installed by the routing owner, never a local parameter.
    pub(crate) fn new(factory: SocketFactory, upstreams: SystemDnsSnapshot) -> Self {
        let resolver = BootstrapResolver::new(factory.clone(), upstreams);
        Self { factory, resolver }
    }
}
impl Protection for ManagedProtection {
    fn check_active(&self) -> io::Result<()> {
        let state = self.factory.0.state.lock().map_err(|_| super::closed())?;
        if state.active {
            Ok(())
        } else {
            Err(super::closed())
        }
    }
    fn cancelled(&self) -> SocketFuture<'_, ()> {
        Box::pin(async move {
            self.factory.0.cancelled.cancelled().await;
            Ok(())
        })
    }
    fn native(&self) -> &dyn NativeSocketFactory {
        self
    }
    fn resolve<'a>(&'a self, host: &'a str, port: u16) -> SocketFuture<'a, Vec<SocketAddr>> {
        Box::pin(async move {
            self.resolver
                .resolve(Purpose::Carrier, host)
                .await
                .map(|a| a.into_iter().map(|ip| SocketAddr::new(ip, port)).collect())
        })
    }
    fn prepare(&self, _: SocketAddr, _: sockets::Purpose<'_>) -> io::Result<TcpSocket> {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "protected socket requires owning seam",
        ))
    }
    fn accepted(&self, _: &TcpStream, _: &str) -> io::Result<()> {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "protected accept requires owning listener",
        ))
    }
    fn listen(&self, bind: SocketAddr, _: &str) -> io::Result<sockets::OuterListener> {
        self.factory
            .listener(Purpose::Carrier, bind)
            .map(sockets::OuterListener::Protected)
    }
    fn dial<'a>(
        &'a self,
        address: SocketAddr,
        _: &'a str,
        _: &'a str,
    ) -> SocketFuture<'a, sockets::OuterTcp> {
        Box::pin(async move {
            self.factory
                .tcp(Purpose::Carrier, address)
                .await
                .map(sockets::OuterTcp::Protected)
        })
    }
}
impl NativeSocketFactory for ManagedProtection {
    fn check_active(&self) -> io::Result<()> {
        Protection::check_active(self)
    }
    fn udp(
        &self,
        bind: SocketAddr,
        buffer: usize,
        _: NativeSocketPurpose<'_>,
    ) -> io::Result<Box<dyn NativeUdp>> {
        Ok(Box::new(self.factory.udp_buffered(
            Purpose::Carrier,
            bind,
            buffer,
        )?))
    }
    fn listen(
        &self,
        bind: SocketAddr,
        _: NativeSocketPurpose<'_>,
    ) -> io::Result<Box<dyn NativeListener>> {
        Ok(Box::new(self.factory.listener(Purpose::Carrier, bind)?))
    }
    fn dial<'a>(
        &'a self,
        endpoint: SocketAddr,
        _: NativeSocketPurpose<'a>,
    ) -> SocketFuture<'a, Box<dyn NativeTcp>> {
        Box::pin(async move {
            // Candidate names were resolved by Protection::resolve before the
            // coherent managed Runtime config was constructed. No ambient DNS.
            Ok(Box::new(self.factory.tcp(Purpose::Carrier, endpoint).await?) as Box<dyn NativeTcp>)
        })
    }
}
impl NativeTcp for ProtectedTcp {
    fn peer_addr(&self) -> io::Result<SocketAddr> {
        self.peer_addr()
    }
    fn set_nodelay(&self, value: bool) -> io::Result<()> {
        self.set_nodelay(value)
    }
}
impl NativeListener for ProtectedListener {
    fn local_addr(&self) -> io::Result<SocketAddr> {
        self.local_addr()
    }
    fn accept(&self) -> SocketFuture<'_, (Box<dyn NativeTcp>, SocketAddr)> {
        Box::pin(async move {
            let (s, a) = self.accept().await?;
            Ok((Box::new(s) as Box<dyn NativeTcp>, a))
        })
    }
}
impl NativeUdp for ProtectedUdp {
    fn local_addr(&self) -> io::Result<SocketAddr> {
        self.local_addr()
    }
    fn writable(&self) -> SocketFuture<'_, ()> {
        Box::pin(self.writable())
    }
    fn try_send_to(&self, bytes: &[u8], remote: SocketAddr) -> io::Result<usize> {
        self.try_send_to(bytes, remote)
    }
    fn recv_from<'a>(&'a self, bytes: &'a mut [u8]) -> SocketFuture<'a, (usize, SocketAddr)> {
        Box::pin(self.recv_from(bytes))
    }
    fn send_to<'a>(&'a self, bytes: &'a [u8], remote: SocketAddr) -> SocketFuture<'a, usize> {
        Box::pin(self.send_to(bytes, remote))
    }
}
