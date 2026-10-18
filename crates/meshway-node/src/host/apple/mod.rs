//! NetworkExtension attachment. NE owns settings; Node owns all packet policy.
mod capture;
mod dns;
mod network_settings;
mod resources;
mod sockets;
pub use dns::Settings as DnsSettings;
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
    /// Resource boundary, not policy. Kept on suspend, replaced by a new Desired.
    #[serde(default)]
    pub egress_routes: Vec<ipnet::IpNet>,
    pub mtu: u16,
    #[serde(default)]
    pub dns: Option<DnsSettings>,
}
impl TunnelSettings {
    pub fn included_routes(&self) -> Vec<ipnet::IpNet> {
        let mut routes = self.routes.clone();
        routes.extend(&self.egress_routes);
        if let Some(dns) = &self.dns {
            let route = ipnet::IpNet::V4(ipnet::Ipv4Net::new(dns.server, 32).expect("host prefix"));
            if !routes.contains(&route) {
                routes.push(route);
            }
        }
        routes
    }
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
        dns_settings: meshway_dns::SystemDnsSettings,
    ) -> SendFuture<'a, std::result::Result<Box<dyn Host>, Problem>> {
        Box::pin(async move {
            options.validate().map_err(|_| Problem::Configuration)?;
            dns_settings
                .validate()
                .map_err(|_| Problem::Configuration)?;
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
                file.take(capture::JOURNAL_BYTES + 1)
                    .read_to_end(&mut bytes)
                    .map_err(|_| Problem::Storage)?;
                if bytes.len() as u64 > capture::JOURNAL_BYTES {
                    return Err(Problem::Storage);
                }
                let record: CaptureRecord =
                    serde_json::from_slice(&bytes).map_err(|_| Problem::Storage)?;
                if record.version != 1 {
                    return Err(Problem::Storage);
                }
                let mut restored = record.settings;
                if restored.routes.len() > 128
                    || restored.egress_routes.len() > capture::MAX_RETAINED_ROUTES
                    || restored.mtu < 1280
                    || restored.dns.as_ref().is_some_and(|dns| !dns.valid())
                {
                    return Err(Problem::Storage);
                }
                if let Some(dns) = &restored.dns {
                    let pools: Vec<_> = restored
                        .routes
                        .iter()
                        .copied()
                        .filter(|route| route.prefix_len() != 0)
                        .collect();
                    if let Err(failure) = dns::conflict(dns.server, &pools) {
                        tracing::warn!(%failure, "DNS capture conflicts at restart; restoring Mesh capture only until DNS can install");
                        restored.dns = None;
                    }
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
                resolver: Arc::new(Default::default()),
            });
            let watched_routes = settings
                .as_ref()
                .map(|settings| settings.routes.clone())
                .unwrap_or_default();
            Ok(Box::new(Attachment {
                device,
                name: self.name.clone(),
                paths: self.paths.clone(),
                settings: Arc::new(tokio::sync::Mutex::new(network_settings::Owner {
                    directory,
                    sink: self.sink.clone(),
                    current: settings,
                    pools: Vec::new(),
                    failed: false,
                })),
                dns: Arc::new(dns::Attachment::new(
                    dns_settings.anchor_address,
                    options.mtu,
                )),
                watched_routes,
                check_at: std::time::Instant::now() + std::time::Duration::from_secs(1),
                sockets: sockets.clone(),
                facts: Facts::default(),
                http: Arc::new(sockets::resolver::HttpResolver {
                    queries: Arc::new(Default::default()),
                    stop: tokio_util::sync::CancellationToken::new(),
                    hosts: sockets.hosts.clone(),
                }),
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
    paths: watch::Sender<NetworkChange>,
    settings: Arc<tokio::sync::Mutex<network_settings::Owner>>,
    dns: Arc<dns::Attachment>,
    sockets: Arc<sockets::Factory>,
    facts: Facts,
    http: Arc<sockets::resolver::HttpResolver>,
    watched_routes: Vec<ipnet::IpNet>,
    check_at: std::time::Instant,
}
impl Drop for Attachment {
    fn drop(&mut self) {
        self.dns.close();
        self.http.stop.cancel();
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
            let mut owner = self.settings.lock().await;
            let mut routes = owner
                .current
                .as_ref()
                .map(|settings| settings.routes.clone())
                .unwrap_or_default();
            owner.pools.clear();
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
                owner.pools.push(route);
                if !routes.contains(&route) {
                    routes.push(route);
                }
            }
            if routes.len() > 128 {
                return Err(Error::Unsupported("capture prefix capacity"));
            }
            // NE also needs routes for static IPv6 sinks, which Linux rejects
            // in nft without a route. If their CIDR expansion exceeds the NE
            // budget, first install a negative boundary; never leave new exit
            // destinations on the physical network because capture is too big.
            let (egress_routes, capture_error) = match capture::routes(desired.capture()) {
                Ok(routes) => (routes, None),
                Err(error) => (capture::negative(), Some(error)),
            };
            let settings = TunnelSettings {
                ipv4: desired.local().addresses.ipv4,
                ipv6: desired.local().addresses.ipv6,
                routes,
                egress_routes,
                mtu: self.device.mtu() as u16,
                dns: owner
                    .current
                    .as_ref()
                    .and_then(|settings| settings.dns.clone()),
            };
            self.watched_routes = settings.routes.clone();
            if self.observe_overlap()? {
                return Err(io::ErrorKind::AddrInUse.into());
            }
            owner.apply(Some(settings)).await?;
            if let Some(error) = capture_error {
                self.sockets.resources.apply(None)?;
                return Err(error);
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
            self.dns.close();
            self.settings.lock().await.apply(None).await?;
            self.watched_routes.clear();
            Ok(())
        })
    }
    fn shutdown(mut self: Box<Self>, keep: bool) -> Local<'static, Result<()>> {
        Box::pin(async move {
            let result = if keep {
                self.suspend().await
            } else {
                self.down().await
            };
            self.http.stop.cancel();
            tokio::join!(
                self.sockets.resolver.shutdown(),
                self.http.queries.shutdown()
            );
            result
        })
    }
    fn tun(&self) -> Option<&dyn meshway_tun::PacketDevice> {
        Some(&self.device)
    }
    fn dns_packets(&self) -> Option<Arc<meshway_dns::packet::Port>> {
        self.dns.port()
    }
    fn sockets(&self) -> Arc<dyn meshway_connectivity::SocketFactory> {
        self.sockets.clone()
    }
    fn resolver_exchange(&self) -> Arc<dyn meshway_dns::ResolverExchange> {
        self.sockets.clone()
    }
    fn client_builder(&self) -> Result<reqwest::ClientBuilder> {
        meshway_control_client::http_client_builder()
            .map(|builder| {
                builder
                    .no_proxy()
                    .pool_max_idle_per_host(0)
                    .dns_resolver(self.http.clone())
            })
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
        Box::pin(self.dns.prepare(self.settings.clone()))
    }
    fn restore_dns(&mut self) -> Local<'_, std::result::Result<(), meshway_dns::SystemDnsError>> {
        Box::pin(async {
            self.dns.close();
            let mut owner = self.settings.lock().await;
            if let Some(mut settings) = owner.current.clone() {
                settings.dns = None;
                owner
                    .apply(Some(settings))
                    .await
                    .map_err(|_| meshway_dns::SystemDnsError::Backend)?;
            }
            Ok(())
        })
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
