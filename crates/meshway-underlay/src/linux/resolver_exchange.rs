//! Linux physical descriptors for the shared sealed-route DNS exchange.
use super::{
    datagram::ProtectedDatagram,
    sockets::{SocketFactory, connect_tcp},
};
use meshway_dns::{Bytes, DnsFuture, GatewayError, LocalResolverRoute, ResolverExchange, upstream};
use meshway_transport::raw::Datagram;
use std::{
    io,
    net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr},
};

impl ResolverExchange for SocketFactory {
    fn exchange<'a>(
        &'a self,
        route: &'a LocalResolverRoute,
        query: Bytes,
    ) -> DnsFuture<'a, Result<Bytes, GatewayError>> {
        upstream::exchange(self, route, query)
    }
}
impl upstream::Sockets for SocketFactory {
    fn udp<'a>(
        &'a self,
        route: &'a LocalResolverRoute,
    ) -> DnsFuture<'a, io::Result<Box<dyn Datagram>>> {
        Box::pin(async move {
            route.ensure_current().map_err(io::Error::other)?;
            let scope = self.shared.installed_data().map_err(io::Error::other)?;
            let endpoint = route.endpoint();
            let local = SocketAddr::new(
                if endpoint.is_ipv4() {
                    IpAddr::V4(Ipv4Addr::UNSPECIFIED)
                } else {
                    IpAddr::V6(Ipv6Addr::UNSPECIFIED)
                },
                0,
            );
            Ok(Box::new(
                ProtectedDatagram::bind(scope, local, &Default::default(), Some(vec![endpoint]))
                    .map_err(io::Error::other)?,
            ) as Box<dyn Datagram>)
        })
    }
    fn tcp<'a>(
        &'a self,
        route: &'a LocalResolverRoute,
    ) -> DnsFuture<'a, io::Result<upstream::Connection>> {
        Box::pin(async move {
            route.ensure_current().map_err(io::Error::other)?;
            let scope = self.shared.installed_data().map_err(io::Error::other)?;
            let timeout = route
                .deadline()
                .saturating_duration_since(std::time::Instant::now())
                .min(self.timeout);
            let stream = connect_tcp(
                scope,
                route.endpoint(),
                None,
                &Default::default(),
                &Default::default(),
                timeout,
            )
            .await
            .map_err(io::Error::other)?;
            Ok(upstream::Connection {
                peer: stream.peer_addr()?,
                stream: Box::new(stream),
            })
        })
    }
}
