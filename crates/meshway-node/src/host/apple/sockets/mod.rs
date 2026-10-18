//! Apple physical sockets; traversal checks are shared with Linux Underlay.
mod tcp;
mod udp;
use super::resources::{Resource, Resources};
use futures_util::future::BoxFuture;
use meshway_configuration::execution::DialEndpoint;
use meshway_connectivity::{
    AcceptedSocket, DatagramSocket, Error, ListenerSocket, SocketFactory, TraversalSocket,
};
use meshway_engine::Desired;
use meshway_model::state::ResourceRef;
use meshway_transport::{
    raw::Datagram,
    spec::{DialParameters, Endpoint},
};
use std::{
    collections::BTreeMap,
    io,
    net::{IpAddr, SocketAddr},
    sync::{Arc, Mutex},
    time::Duration,
};

#[derive(Clone)]
pub(super) struct Factory {
    pub resources: Arc<Resources>,
    pub interface: Arc<Mutex<Option<String>>>,
    pub hosts: BTreeMap<String, Vec<IpAddr>>,
    pub timeout: Duration,
}
impl Factory {
    pub fn protect(
        &self,
        socket: &socket2::SockRef<'_>,
        ipv4: bool,
        declared: &str,
    ) -> io::Result<()> {
        let current = self.interface.lock().map_err(|_| io::ErrorKind::Other)?;
        let name = if declared.is_empty() {
            current
                .as_deref()
                .ok_or(io::ErrorKind::NetworkUnreachable)?
        } else {
            declared
        };
        if name.starts_with("utun") || name == "lo0" {
            return Err(io::ErrorKind::PermissionDenied.into());
        }
        let index = std::num::NonZeroU32::new(nix::net::if_::if_nametoindex(name)?)
            .ok_or(io::ErrorKind::NetworkUnreachable)?;
        let current = if ipv4 {
            socket.device_index_v4()?
        } else {
            socket.device_index_v6()?
        };
        if current != Some(index) {
            if ipv4 {
                socket.bind_device_by_index_v4(Some(index))?;
            } else {
                socket.bind_device_by_index_v6(Some(index))?;
            }
            let bound = if ipv4 {
                socket.device_index_v4()?
            } else {
                socket.device_index_v6()?
            };
            if bound != Some(index) {
                return Err(io::ErrorKind::PermissionDenied.into());
            }
        }
        Ok(())
    }
    pub async fn resolve(
        &self,
        resource: &Resource,
        endpoint: &Endpoint,
    ) -> io::Result<Vec<SocketAddr>> {
        resource.check()?;
        let port = endpoint.port.get();
        let addresses = if let Ok(ip) = endpoint.host.parse::<IpAddr>() {
            vec![SocketAddr::new(ip, port)]
        } else if let Some(hosts) = self.hosts.get(&endpoint.host) {
            hosts.iter().map(|ip| SocketAddr::new(*ip, port)).collect()
        } else {
            tokio::select! { biased;
                _ = resource.stop.cancelled() => return Err(io::ErrorKind::NotConnected.into()),
                result = tokio::time::timeout(self.timeout, tokio::net::lookup_host((endpoint.host.as_str(), port))) =>
                    result.map_err(|_| io::ErrorKind::TimedOut)??.take(16).collect(),
            }
        };
        resource.check()?;
        if addresses.is_empty() {
            return Err(io::ErrorKind::NotFound.into());
        }
        Ok(addresses)
    }
    fn resource(
        &self,
        desired: &Desired,
        key: &str,
        listener: bool,
    ) -> Result<Arc<Resource>, Error> {
        let purpose = if listener {
            if !desired.local().listeners.contains_key(key) {
                return Err(Error::Configuration);
            }
            ResourceRef::Listener { key: key.into() }
        } else {
            let candidate = desired.dialers().get(key).ok_or(Error::Configuration)?;
            if !desired
                .netmap()
                .projection
                .inventory
                .contains_key(&candidate.peer)
            {
                return Err(Error::Authority);
            }
            ResourceRef::Dial { key: key.into() }
        };
        self.resources
            .acquire(desired, purpose)
            .map_err(|_| Error::Underlay)
    }
}

// Arc lets traversal contexts retain only the physical factory, never Node.
impl SocketFactory for Factory {
    fn connect_endpoint<'a>(
        &'a self,
        desired: &'a Desired,
        key: &'a str,
        endpoint: &'a DialEndpoint,
    ) -> BoxFuture<'a, Result<AcceptedSocket, Error>> {
        Box::pin(async move { tcp::connect(self, desired, key, endpoint).await })
    }
    fn listen<'a>(
        &'a self,
        desired: &'a Desired,
        key: &'a str,
    ) -> BoxFuture<'a, Result<Box<dyn ListenerSocket>, Error>> {
        Box::pin(async move { tcp::listen(self, desired, key) })
    }
    fn datagram_endpoint<'a>(
        &'a self,
        desired: &'a Desired,
        key: &'a str,
        endpoint: &'a DialEndpoint,
    ) -> BoxFuture<'a, Result<Vec<DatagramSocket>, Error>> {
        Box::pin(async move {
            let resource = self.resource(desired, key, false)?;
            let DialParameters::NativeUdp { options } = &desired.dialers()[key].parameters else {
                return Err(Error::Configuration);
            };
            let addresses = self.resolve(&resource, &endpoint.endpoint()).await?;
            let mut sockets = Vec::new();
            for ipv4 in [true, false] {
                let targets: Vec<_> = addresses
                    .iter()
                    .filter(|address| address.is_ipv4() == ipv4)
                    .copied()
                    .collect();
                if targets.is_empty() {
                    continue;
                }
                let resource = self.resource(desired, key, false)?;
                let endpoint = udp::Endpoint::dial(self, resource, options, ipv4, targets.clone())?;
                sockets.push(DatagramSocket {
                    endpoint: Box::new(endpoint),
                    targets,
                });
            }
            Ok(sockets)
        })
    }
    fn datagram_listener<'a>(
        &'a self,
        desired: &'a Desired,
        key: &'a str,
    ) -> BoxFuture<'a, Result<Box<dyn Datagram>, Error>> {
        Box::pin(async move {
            Ok(Box::new(udp::Endpoint::listen(self, desired, key)?) as Box<dyn Datagram>)
        })
    }
    fn traversal_listener<'a>(
        &'a self,
        desired: &'a Desired,
        key: &'a str,
        reference: meshway_control::connectivity::SocketReference,
    ) -> BoxFuture<'a, Result<TraversalSocket, Error>> {
        Box::pin(async move {
            udp::Endpoint::listen(self, desired, key)?.traversal(
                Arc::new(self.clone()),
                desired,
                reference,
            )
        })
    }
    fn bind_datagram_candidate<'a>(
        &'a self,
        desired: &'a Desired,
        key: &'a str,
        ipv4: bool,
        reference: meshway_control::connectivity::SocketReference,
    ) -> BoxFuture<'a, Result<TraversalSocket, Error>> {
        Box::pin(async move {
            let resource = self.resource(desired, key, false)?;
            let DialParameters::NativeUdp { options } = &desired.dialers()[key].parameters else {
                return Err(Error::Configuration);
            };
            udp::Endpoint::dial(self, resource, options, ipv4, Vec::new())?.traversal(
                Arc::new(self.clone()),
                desired,
                reference,
            )
        })
    }
    fn route_source<'a>(&'a self, remote: SocketAddr) -> BoxFuture<'a, Option<IpAddr>> {
        Box::pin(async move {
            let socket = socket2::Socket::new(
                socket2::Domain::for_address(remote),
                socket2::Type::DGRAM,
                Some(socket2::Protocol::UDP),
            )
            .ok()?;
            self.protect(&socket2::SockRef::from(&socket), remote.is_ipv4(), "")
                .ok()?;
            socket.connect(&remote.into()).ok()?;
            socket
                .local_addr()
                .ok()?
                .as_socket()
                .map(|address| address.ip())
        })
    }
    fn address_assigned<'a>(&'a self, address: IpAddr) -> BoxFuture<'a, Option<bool>> {
        Box::pin(async move {
            super::interfaces()
                .ok()
                .map(|addresses| addresses.iter().any(|entry| entry.address == address))
        })
    }
    fn observe_remotes(&self, _: Vec<IpAddr>) { /* NWPathMonitor owns path observations. */
    }
}
