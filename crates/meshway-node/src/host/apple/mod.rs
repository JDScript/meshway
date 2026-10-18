//! NetworkExtension attachment. NE owns settings; Node owns all packet policy.
mod resources;
mod sockets;
#[cfg(test)]
mod tests;
use super::*;
use meshway_underlay::{NetworkChange, NetworkChangeClass};
use std::{
    io::{self, Read},
    net::IpAddr,
    os::fd::OwnedFd,
    sync::Mutex,
};

pub(super) const JOURNAL: &str = "apple-capture.json";

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TunnelSettings {
    pub ipv4: Option<std::net::Ipv4Addr>,
    pub ipv6: Option<std::net::Ipv6Addr>,
    pub routes: Vec<ipnet::IpNet>,
    pub mtu: u16,
}

#[derive(serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct CaptureRecord {
    version: u8,
    settings: TunnelSettings,
}

/// Acknowledgement means NE completed the settings change, not that a packet
/// traversed the Mesh. Calls are serial and must finish before returning.
pub trait TunnelSettingsSink: Send + Sync + 'static {
    fn apply(&self, settings: Option<TunnelSettings>) -> SendFuture<'_, io::Result<()>>;
}

pub struct Apple {
    fd: OwnedFd,
    name: String,
    sink: Arc<dyn TunnelSettingsSink>,
    paths: watch::Sender<NetworkChange>,
    interface: Arc<Mutex<Option<String>>>,
    addresses: Mutex<Vec<IpAddr>>,
}
impl Apple {
    pub fn new(fd: OwnedFd, name: String, sink: Arc<dyn TunnelSettingsSink>) -> io::Result<Self> {
        if name.len() > 15
            || !name.strip_prefix("utun").is_some_and(|suffix| {
                !suffix.is_empty() && suffix.bytes().all(|byte| byte.is_ascii_digit())
            })
        {
            return Err(io::ErrorKind::InvalidInput.into());
        }
        Ok(Self {
            fd,
            name,
            sink,
            paths: watch::channel(NetworkChange::default()).0,
            interface: Arc::new(Mutex::new(None)),
            addresses: Mutex::new(Vec::new()),
        })
    }
    /// NWPathMonitor reports its selected physical interface. Sleep/wake is a
    /// refresh; a changed interface or route is a path event, never authority.
    pub fn path_changed(&self, interface: Option<String>, path: bool) -> io::Result<()> {
        if interface.as_ref().is_some_and(|name| {
            name.is_empty() || name.len() > 15 || name.starts_with("utun") || name == "lo0"
        }) {
            return Err(io::ErrorKind::InvalidInput.into());
        }
        let mut current = self.interface.lock().map_err(|_| io::ErrorKind::Other)?;
        let mut path = path || *current != interface;
        if let Ok(interfaces) = interfaces() {
            let mut addresses: Vec<_> = interfaces
                .into_iter()
                .filter(|entry| Some(&entry.name) == interface.as_ref())
                .map(|entry| entry.address)
                .collect();
            addresses.sort();
            addresses.dedup();
            let mut previous = self.addresses.lock().map_err(|_| io::ErrorKind::Other)?;
            path |= *previous != addresses;
            *previous = addresses;
        } else {
            path = true;
        }
        *current = interface;
        self.paths.send_modify(|change| {
            change.epoch = change.epoch.saturating_add(1);
            if path {
                change.path = change.path.saturating_add(1);
            }
            change.class = Some(if path {
                NetworkChangeClass::Path
            } else {
                NetworkChangeClass::Refresh
            });
            change.at = Some(std::time::SystemTime::now());
        });
        Ok(())
    }
}

pub(super) fn interfaces() -> io::Result<Vec<InterfaceAddress>> {
    use nix::{ifaddrs::getifaddrs, net::if_::InterfaceFlags};
    Ok(getifaddrs()?
        .filter(|entry| {
            entry.flags.contains(InterfaceFlags::IFF_UP)
                && !entry.flags.contains(InterfaceFlags::IFF_LOOPBACK)
        })
        .filter_map(|entry| {
            let socket = entry.address?;
            let address = socket
                .as_sockaddr_in()
                .map(|value| IpAddr::V4(value.ip()))
                .or_else(|| socket.as_sockaddr_in6().map(|value| IpAddr::V6(value.ip())))?;
            Some(InterfaceAddress {
                name: entry.interface_name,
                address,
            })
        })
        .collect())
}
impl Platform for Apple {
    fn capabilities(&self) -> Capabilities {
        Capabilities {
            packets: true,
            path_observation: true,
            exit_families: Vec::new(),
        }
    }
    fn interfaces(&self) -> io::Result<Vec<InterfaceAddress>> {
        interfaces()
    }
    fn open<'a>(
        &'a self,
        directory: Arc<meshway_private_storage::Directory>,
        _: &'a str,
        options: meshway_underlay::Options,
        _: meshway_dns::SystemDnsSettings,
    ) -> SendFuture<'a, std::result::Result<Box<dyn Host>, Problem>> {
        Box::pin(async move {
            options.validate().map_err(|_| Problem::Configuration)?;
            if options.bootstrap_nameservers.is_some() {
                return Err(Problem::Configuration);
            }
            let device = meshway_tun::Utun::from_owned_fd(
                self.fd.try_clone().map_err(|_| Problem::Tun)?,
                options.mtu,
            )
            .map_err(|_| Problem::Tun)?;
            let mut settings = None;
            if let Some(file) = directory
                .file(JOURNAL, false)
                .map_err(|_| Problem::Storage)?
            {
                let mut bytes = Vec::new();
                file.take(65537)
                    .read_to_end(&mut bytes)
                    .map_err(|_| Problem::Storage)?;
                if bytes.len() > 65536 {
                    return Err(Problem::Storage);
                }
                let record: CaptureRecord =
                    serde_json::from_slice(&bytes).map_err(|_| Problem::Storage)?;
                if record.version != 1 {
                    return Err(Problem::Storage);
                }
                let restored = record.settings;
                if restored.routes.len() > 128 || restored.mtu < 1280 {
                    return Err(Problem::Storage);
                }
                self.sink
                    .apply(Some(restored.clone()))
                    .await
                    .map_err(|_| Problem::Underlay)?;
                settings = Some(restored);
            }
            let sockets = Arc::new(sockets::Factory {
                resources: Arc::new(resources::Resources::new(options.max_sockets)),
                interface: self.interface.clone(),
                hosts: options.bootstrap_hosts,
                timeout: options.connect_timeout,
            });
            let watched_routes = settings
                .as_ref()
                .map(|settings| settings.routes.clone())
                .unwrap_or_default();
            Ok(Box::new(Attachment {
                device,
                name: self.name.clone(),
                sink: self.sink.clone(),
                paths: self.paths.clone(),
                directory,
                settings,
                watched_routes,
                check_at: std::time::Instant::now() + std::time::Duration::from_secs(1),
                sockets,
                facts: Facts::default(),
            }) as Box<dyn Host>)
        })
    }
    fn spawn(&self, worker: Worker) -> tokio::task::JoinHandle<std::result::Result<(), Problem>> {
        tokio::task::spawn_blocking(move || tokio::runtime::Handle::current().block_on(worker()))
    }
}

#[derive(Default)]
struct Facts {
    overlap: watch::Sender<Option<meshway_underlay::PoolOverlap>>,
    repairs: watch::Sender<u64>,
}
struct Attachment {
    device: meshway_tun::Utun,
    name: String,
    sink: Arc<dyn TunnelSettingsSink>,
    paths: watch::Sender<NetworkChange>,
    directory: Arc<meshway_private_storage::Directory>,
    settings: Option<TunnelSettings>,
    sockets: Arc<sockets::Factory>,
    facts: Facts,
    watched_routes: Vec<ipnet::IpNet>,
    check_at: std::time::Instant,
}
impl Drop for Attachment {
    fn drop(&mut self) {
        let _ = self.sockets.resources.apply(None);
    }
}
impl Attachment {
    fn observe_overlap(&self) -> io::Result<bool> {
        let found = interfaces()?
            .into_iter()
            .find(|interface| {
                interface.name != self.name
                    && self
                        .watched_routes
                        .iter()
                        .filter(|route| route.prefix_len() != 0)
                        .any(|route| route.contains(&interface.address))
            })
            .map(|interface| meshway_underlay::PoolOverlap {
                interface: interface.name,
                address: interface.address,
            });
        let overlap = found.is_some();
        if overlap {
            self.sockets.resources.apply(None)?;
        }
        self.facts.overlap.send_if_modified(|current| {
            if *current == found {
                false
            } else {
                *current = found;
                true
            }
        });
        Ok(overlap)
    }
}

impl Host for Attachment {
    fn install(&mut self, desired: Desired) -> Local<'_, Result<()>> {
        Box::pin(async move {
            use meshway_tun::PacketDevice;
            let mut routes = self
                .settings
                .as_ref()
                .map(|settings| settings.routes.clone())
                .unwrap_or_default();
            for prefix in desired
                .local()
                .mesh_address_pools
                .ipv4
                .iter()
                .chain(&desired.local().mesh_address_pools.ipv6)
            {
                let route = prefix
                    .parse::<ipnet::IpNet>()
                    .map_err(|_| io::ErrorKind::InvalidData)?;
                if !routes.contains(&route) {
                    routes.push(route);
                }
            }
            let external = desired.capture().is_some_and(|capture| {
                !capture.managed_ipv4.is_empty()
                    || !capture.managed_ipv6.is_empty()
                    || !capture.sink_ipv6.is_empty()
            });
            if external {
                for prefix in ["0.0.0.0/0", "::/0"] {
                    let route = prefix.parse::<ipnet::IpNet>().expect("constant CIDR");
                    if !routes.contains(&route) {
                        routes.push(route);
                    }
                }
            }
            if routes.len() > 128 {
                return Err(Error::Unsupported("capture prefix capacity"));
            }
            let settings = TunnelSettings {
                ipv4: desired
                    .local()
                    .addresses
                    .ipv4
                    .or_else(|| self.settings.as_ref().and_then(|settings| settings.ipv4)),
                ipv6: desired
                    .local()
                    .addresses
                    .ipv6
                    .or_else(|| self.settings.as_ref().and_then(|settings| settings.ipv6)),
                routes,
                mtu: self.device.mtu() as u16,
            };
            self.watched_routes = settings.routes.clone();
            if self.observe_overlap()? {
                return Err(io::ErrorKind::AddrInUse.into());
            }
            if self.settings.as_ref() != Some(&settings) {
                // Store only the negative capture boundary before asking NE;
                // this record never authorizes packets or extends a lease.
                let bytes = serde_json::to_vec(&CaptureRecord {
                    version: 1,
                    settings: settings.clone(),
                })
                .map_err(io::Error::other)?;
                self.directory
                    .replace(JOURNAL, &bytes)
                    .map_err(|error| io::Error::other(error.to_string()))?;
                self.sink.apply(Some(settings.clone())).await?;
                self.settings = Some(settings);
            }
            if external {
                self.sockets.resources.apply(None)?;
                return Err(Error::Unsupported("Apple exit capture is not enabled"));
            }
            self.sockets.resources.apply(if desired.local().enabled {
                Some(desired)
            } else {
                None
            })?;
            Ok(())
        })
    }
    fn suspend(&mut self) -> Local<'_, Result<()>> {
        Box::pin(async {
            self.sockets.resources.apply(None)?;
            Ok(())
        })
    }
    fn down(&mut self) -> Local<'_, Result<()>> {
        Box::pin(async {
            self.sockets.resources.apply(None)?;
            self.sink.apply(None).await?;
            self.directory
                .remove(JOURNAL)
                .map_err(|error| io::Error::other(error.to_string()))?;
            self.settings = None;
            self.watched_routes.clear();
            Ok(())
        })
    }
    fn shutdown(mut self: Box<Self>, keep: bool) -> Local<'static, Result<()>> {
        Box::pin(async move {
            if keep {
                self.suspend().await
            } else {
                self.down().await
            }
        })
    }
    fn tun(&self) -> Option<&dyn meshway_tun::PacketDevice> {
        Some(&self.device)
    }
    fn sockets(&self) -> Arc<dyn meshway_connectivity::SocketFactory> {
        self.sockets.clone()
    }
    fn resolver_exchange(&self) -> Arc<dyn meshway_dns::ResolverExchange> {
        Arc::new(DnsUnavailable)
    }
    fn client_builder(&self) -> Result<reqwest::ClientBuilder> {
        meshway_control_client::http_client_builder()
            .map(|builder| builder.no_proxy().pool_max_idle_per_host(0))
            .map_err(|error| io::Error::other(error.to_string()).into())
    }
    fn network_changes(&self) -> watch::Receiver<NetworkChange> {
        self.paths.subscribe()
    }
    fn request_interrupt(&self) -> Option<Arc<dyn meshway_control_client::RequestInterrupt>> {
        Some(Arc::new(PathInterrupt(self.paths.subscribe())))
    }
    fn pool_overlap(&self) -> watch::Receiver<Option<meshway_underlay::PoolOverlap>> {
        self.facts.overlap.subscribe()
    }
    fn repairs(&self) -> watch::Receiver<u64> {
        self.facts.repairs.subscribe()
    }
    fn owned_interfaces(&self) -> Vec<String> {
        vec![self.name.clone()]
    }
    fn ownership_ended(&self) -> Option<meshway_underlay::OwnershipEnd> {
        None
    }
    fn ended(&self) -> Local<'_, ()> {
        Box::pin(std::future::pending())
    }
    fn recovery_at(&self) -> Option<std::time::Instant> {
        None
    }
    fn recover(&mut self) -> Local<'_, Result<()>> {
        Box::pin(async { Err(Error::Unsupported("NE ownership recovery")) })
    }
    fn repair_at(&self) -> Option<std::time::Instant> {
        Some(self.check_at)
    }
    fn repair(&mut self) -> Local<'_, Result<()>> {
        Box::pin(async {
            self.check_at = std::time::Instant::now() + std::time::Duration::from_secs(1);
            self.observe_overlap()?;
            Ok(())
        })
    }
    fn clamp_mss(&mut self, _: meshway_tun::PathLimits) -> Result<()> {
        Err(Error::Unsupported(
            "MSS clamp; using tunnel MTU and dataplane PTB",
        ))
    }
    fn prepare_dns(
        &mut self,
    ) -> Local<'_, std::result::Result<Box<dyn SystemDns>, meshway_dns::SystemDnsFailure>> {
        Box::pin(async {
            Err(meshway_dns::SystemDnsFailure::new(
                meshway_dns::SystemDnsError::Unsupported,
                "apple_dns_service_address_pending",
            ))
        })
    }
    fn restore_dns(&mut self) -> Local<'_, std::result::Result<(), meshway_dns::SystemDnsError>> {
        Box::pin(async { Ok(()) })
    }
    fn remove_dns_anchor(&mut self) -> Local<'_, Result<()>> {
        Box::pin(async { Ok(()) })
    }
    fn probe_exit<'a>(
        &'a mut self,
        instance: meshway_model::InstanceId,
        netmap: meshway_model::NetmapRef,
        generation: meshway_model::Digest,
        families: &'a [meshway_model::exit::ExitFamily],
    ) -> Local<'a, meshway_model::exit::ExitCapability> {
        Box::pin(async move {
            use meshway_model::exit::*;
            ExitCapability {
                instance_id: instance,
                netmap,
                role_generation: generation,
                families: families
                    .iter()
                    .map(|family| ExitFamilyCapability {
                        family: *family,
                        status: ExitStatus::Unavailable {
                            reason: ExitUnavailableReason::UnsupportedFamily,
                        },
                        installed: false,
                    })
                    .collect(),
                observed_at: chrono::Utc::now(),
            }
        })
    }
    fn install_netmap_exit_role<'a>(
        &'a mut self,
        _: &'a meshway_configuration::netmap::ExitRole,
    ) -> Local<'a, Result<()>> {
        Box::pin(async { Err(Error::Unsupported("Apple exit role")) })
    }
    fn start_exit(
        &self,
        _: meshway_configuration::netmap::ExitRole,
    ) -> std::result::Result<Option<ExitAttachment>, Problem> {
        Ok(None)
    }
    fn diagnostics(&self) -> Arc<dyn LocalDiagnostics> {
        Arc::new(Diagnostics)
    }
}
struct DnsUnavailable;
impl meshway_dns::ResolverExchange for DnsUnavailable {
    fn exchange<'a>(
        &'a self,
        _: &'a meshway_dns::LocalResolverRoute,
        _: bytes::Bytes,
    ) -> meshway_dns::DnsFuture<'a, std::result::Result<bytes::Bytes, meshway_dns::GatewayError>>
    {
        Box::pin(async { Err(meshway_dns::GatewayError) })
    }
}
struct Diagnostics;
struct PathInterrupt(watch::Receiver<NetworkChange>);
impl meshway_control_client::RequestInterrupt for PathInterrupt {
    fn interrupted(&self) -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send>> {
        let mut changes = self.0.clone();
        let path = changes.borrow_and_update().path;
        Box::pin(async move {
            while changes.changed().await.is_ok() {
                if changes.borrow_and_update().path != path {
                    return;
                }
            }
        })
    }
}
impl LocalDiagnostics for Diagnostics {
    fn direct_icmp(&self) -> io::Result<Box<dyn Icmp>> {
        Err(io::ErrorKind::Unsupported.into())
    }
    fn resolve<'a>(&'a self, name: &'a str) -> SendFuture<'a, io::Result<Vec<IpAddr>>> {
        Box::pin(async move {
            Ok(tokio::net::lookup_host((name, 0))
                .await?
                .map(|address| address.ip())
                .collect())
        })
    }
}
