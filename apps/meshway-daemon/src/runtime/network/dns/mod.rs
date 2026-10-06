//! Daemon assembly of the one DNS query owner and the one system installer.
//! Control demand creation/refresh happens outside this module, before install.
mod actor;
pub(super) mod intents;
pub(super) use actor::Attachment;
use meshway_authority::ExecutionSnapshot;
use meshway_configuration::execution::NodeConfiguration;
use meshway_dns::{
    CancellationToken, DiscoveryGateway, OwnerDispatcher, ResolverDemand, ResolverExchange,
    ResolverSource, Scope, Service, SystemDnsBackend, SystemDnsError, SystemDnsOwner,
    SystemDnsPlan, SystemDnsSettings,
};
use meshway_model::RequestId;
use std::{collections::BTreeMap, net::SocketAddr, sync::Arc};
use tokio::{
    net::{TcpListener, UdpSocket},
    sync::watch,
};

/// System-upstream provenance expires after two seconds without a real owner
/// observation. Include this timer even when there is no packet/demand work.
pub(super) const OBSERVE_INTERVAL: std::time::Duration = std::time::Duration::from_millis(250);

pub(super) struct Runtime {
    system: SystemDnsOwner,
    settings: SystemDnsSettings,
    scopes: watch::Sender<Option<Scope>>,
    dispatcher: Arc<OwnerDispatcher>,
    stop: CancellationToken,
    service: tokio::task::JoinHandle<std::io::Result<()>>,
    installed_plan: Option<SystemDnsPlan>,
}
impl Drop for Runtime {
    fn drop(&mut self) {
        self.suspend();
        self.stop.cancel();
    }
}
impl Runtime {
    /// Parent first reserves the namespace, obtains the Kernel's anchor, and
    /// calls SystemDnsOwner::prepare_reserved. This function binds real service
    /// sockets before any system-DNS mutation; Scope=None rejects all queries.
    pub async fn prepare(
        system: SystemDnsOwner,
        settings: SystemDnsSettings,
        discovery: Arc<dyn DiscoveryGateway>,
        exchange: Arc<dyn ResolverExchange>,
    ) -> Result<Self, SystemDnsError> {
        settings.validate()?;
        let udp = UdpSocket::bind(settings.bind)
            .await
            .map_err(|_| SystemDnsError::Backend)?;
        let tcp = TcpListener::bind(settings.bind)
            .await
            .map_err(|_| SystemDnsError::Backend)?;
        let (scopes, received) = watch::channel(None);
        let dispatcher = Arc::new(
            OwnerDispatcher::new(
                received,
                discovery,
                exchange,
                meshway_dns::Limits {
                    max_pending: 128,
                    query_timeout: std::time::Duration::from_secs(5),
                },
                1232,
            )
            .map_err(|_| SystemDnsError::Settings)?,
        );
        let service = Service::new(dispatcher.clone(), Default::default())
            .map_err(|_| SystemDnsError::Settings)?;
        let stop = CancellationToken::new();
        let shutdown = stop.clone();
        let service = tokio::spawn(service.run(udp, tcp, shutdown));
        Ok(Self {
            system,
            settings,
            scopes,
            dispatcher,
            stop,
            service,
            installed_plan: None,
        })
    }

    /// Selection descriptors only, not permissions. Parent maps stable IDs to
    /// exact Ip demands through the unique Control and atomic refresh path.
    pub fn demands(&self, cfg: &NodeConfiguration) -> Result<Vec<ResolverDemand>, SystemDnsError> {
        let plan = SystemDnsPlan::new(&cfg.dns.settings, &cfg.dns.egress)?;
        if cfg.dns.settings.servers.values().any(|server| {
            server.server.is_loopback()
                || matches!(server.server, std::net::IpAddr::V6(ip) if ip.is_unicast_link_local())
        }) {
            // A local stub can recurse through the very split route being
            // installed; scoped IPv6 also lacks a published interface context.
            return Err(SystemDnsError::ResolverPlan);
        }
        if matches!(self.settings.backend, SystemDnsBackend::Resolved)
            && plan.needs_system_fallback(&cfg.dns.settings)
        {
            // Resolved's original link-specific routing must not be flattened.
            return Err(SystemDnsError::ResolverPlan);
        }
        let mut demands: Vec<_> = cfg
            .dns
            .settings
            .servers
            .iter()
            .map(|(key, server)| ResolverDemand {
                source: ResolverSource::Configured(key.clone()),
                endpoint: SocketAddr::new(server.server, server.server_port.get()),
                transport: server.transport,
                profile: cfg.dns.profile.clone(),
            })
            .collect();
        if let Some(system) = self.system.system_resolvers() {
            demands.extend(system.demands(&cfg.dns.profile));
        }
        if demands.len() > 256 {
            return Err(SystemDnsError::Capacity);
        }
        Ok(demands)
    }

    /// Call before Control prewarms demands, including unsupported plans. This
    /// retains required negative DNS capture during preparation/failure and
    /// never publishes execution authority or an applied completion.
    pub async fn hold(&mut self, cfg: &NodeConfiguration) -> Result<(), SystemDnsError> {
        self.suspend();
        if matches!(&self.settings.backend, SystemDnsBackend::OwnedResolvConf { path } if path != std::path::Path::new("/etc/resolv.conf"))
        {
            return Err(SystemDnsError::Unsupported);
        }
        let plan = SystemDnsPlan::required(&cfg.dns.settings, &cfg.dns.egress)?;
        if self.installed_plan.as_ref() == Some(&plan) {
            return self.observe().await;
        }
        self.system.install(&plan).await?;
        self.installed_plan = Some(plan);
        Ok(())
    }

    pub async fn install(
        &mut self,
        snapshot: ExecutionSnapshot,
        demands: &BTreeMap<ResolverSource, RequestId>,
    ) -> Result<(), SystemDnsError> {
        self.suspend();
        let cfg = snapshot
            .configuration()
            .map_err(|_| SystemDnsError::ResolverPlan)?;
        // Install required negative capture before discovering a missing grant.
        // Subsequent failure retains the stub and settings, with Scope=None.
        self.hold(cfg).await?;
        self.demands(cfg)?;
        let system_resolvers = self.system.system_resolvers();
        let mut configured = BTreeMap::new();
        let mut system_demands = BTreeMap::new();
        let mut system_tcp_demands = BTreeMap::new();
        for (source, id) in demands {
            match source {
                ResolverSource::Configured(key) => {
                    configured.insert(key.clone(), *id);
                }
                ResolverSource::System { snapshot, index } => {
                    if system_resolvers
                        .as_ref()
                        .is_none_or(|system| system.id() != snapshot)
                    {
                        return Err(SystemDnsError::ResolverPlan);
                    }
                    system_demands.insert(*index, *id);
                }
                ResolverSource::SystemTcp { snapshot, index } => {
                    if system_resolvers
                        .as_ref()
                        .is_none_or(|system| system.id() != snapshot)
                    {
                        return Err(SystemDnsError::ResolverPlan);
                    }
                    system_tcp_demands.insert(*index, *id);
                }
            }
        }
        let scope = Scope {
            snapshot,
            resolver_demands: configured,
            system_resolvers,
            system_demands,
            system_tcp_demands,
        };
        scope
            .validate_resolvers()
            .map_err(|_| SystemDnsError::ResolverPlan)?;
        if self.service.is_finished() {
            return Err(SystemDnsError::Backend);
        }
        self.scopes.send_replace(Some(scope));
        if self.dispatcher.reconcile().await.is_err() {
            self.suspend();
            return Err(SystemDnsError::ResolverPlan);
        }
        self.observe().await
    }
    pub fn suspend(&self) {
        self.scopes.send_replace(None);
    }
    /// Parent selects this regularly with the kernel/lease watcher. A failed
    /// observation withdraws query authority without removing forced suffixes.
    pub async fn observe(&mut self) -> Result<(), SystemDnsError> {
        let result = if self.service.is_finished() {
            Err(SystemDnsError::Backend)
        } else {
            self.system.observe().await
        };
        if result.is_err() {
            self.suspend();
        }
        result
    }
    /// Explicit down restores DNS before the parent deletes the Kernel anchor.
    pub async fn shutdown(mut self, keep: bool) -> Result<(), SystemDnsError> {
        self.suspend();
        let result = if keep {
            Ok(())
        } else {
            self.system.down().await
        };
        self.stop.cancel();
        let joined = (&mut self.service).await;
        let storage = self.system.shutdown(true).await;
        result?;
        storage?;
        joined
            .map_err(|_| SystemDnsError::Backend)?
            .map_err(|_| SystemDnsError::Backend)
    }
}
