//! The same owning capability backs managed VLESS and core native carriers.
use meshway_agent_connections::sockets::{self, Protection};
use meshway_agent_underlay::{BootstrapResolver, Purpose, SocketFactory, SystemDnsSnapshot};
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
        self.factory.check_active()
    }
    fn cancelled(&self) -> SocketFuture<'_, ()> {
        Box::pin(async move {
            self.factory.cancelled().await;
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
            .map(|listener| sockets::OuterListener::Prepared(Box::new(listener)))
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
                .map(|stream| sockets::OuterTcp::Prepared(Box::new(stream)))
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
