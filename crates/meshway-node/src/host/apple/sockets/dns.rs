//! Protected descriptors only; DNS wire/validation is shared in meshway-dns.
use super::*;
use meshway_dns::{Bytes, DnsFuture, GatewayError, LocalResolverRoute, ResolverExchange, upstream};

impl ResolverExchange for Factory {
    fn exchange<'a>(
        &'a self,
        route: &'a LocalResolverRoute,
        query: Bytes,
    ) -> DnsFuture<'a, Result<Bytes, GatewayError>> {
        upstream::exchange(self, route, query)
    }
}
impl upstream::Sockets for Factory {
    fn udp<'a>(
        &'a self,
        route: &'a LocalResolverRoute,
    ) -> DnsFuture<'a, io::Result<Box<dyn Datagram>>> {
        Box::pin(async move {
            let resource = self.resources.resolver(route)?;
            let endpoint = route.endpoint();
            let local = if endpoint.is_ipv4() {
                IpAddr::from(std::net::Ipv4Addr::UNSPECIFIED)
            } else {
                IpAddr::from(std::net::Ipv6Addr::UNSPECIFIED)
            };
            Ok(Box::new(udp::Endpoint::bind(
                self,
                resource,
                SocketAddr::new(local, 0),
                "",
                &Default::default(),
                Some(vec![endpoint]),
            )?) as Box<dyn Datagram>)
        })
    }
    fn tcp<'a>(
        &'a self,
        route: &'a LocalResolverRoute,
    ) -> DnsFuture<'a, io::Result<upstream::Connection>> {
        Box::pin(async move {
            let resource = self.resources.resolver(route)?;
            let endpoint = route.endpoint();
            let socket = if endpoint.is_ipv4() {
                tokio::net::TcpSocket::new_v4()?
            } else {
                tokio::net::TcpSocket::new_v6()?
            };
            self.protect(&socket2::SockRef::from(&socket), endpoint.is_ipv4(), "")?;
            let timeout = self.timeout.min(
                route
                    .deadline()
                    .saturating_duration_since(std::time::Instant::now()),
            );
            let stream = tokio::select! { biased;
                _ = resource.stop.cancelled() => return Err(io::ErrorKind::NotConnected.into()),
                result = tokio::time::timeout(timeout, socket.connect(endpoint)) => result.map_err(|_| io::ErrorKind::TimedOut)??,
            };
            let stream = tcp::adopt(stream, resource, &Default::default(), &Default::default())?;
            Ok(upstream::Connection {
                stream: stream.stream,
                peer: stream.peer,
            })
        })
    }
}
