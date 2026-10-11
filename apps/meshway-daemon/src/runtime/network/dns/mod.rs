//! Daemon assembly of the one DNS query owner and the one system installer.
//! Every resolver route is decided by the installed netmap itself.
mod actor;
pub(super) mod mesh;
pub(super) use actor::Attachment;
use meshway_configuration::netmap::{DnsPlan, NodeNetmap};
use meshway_dns::{
    CancellationToken, OwnerDispatcher, ResolverExchange, Scope, Service, SystemDnsBackend,
    SystemDnsError, SystemDnsOwner, SystemDnsPlan, SystemDnsSettings,
};
use meshway_engine::Desired;
use std::sync::Arc;
use tokio::{
    net::{TcpListener, UdpSocket},
    sync::watch,
};

/// System-upstream provenance expires after two seconds without a real owner
/// observation. Include this timer even when there is no packet work.
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
        exchange: Arc<dyn ResolverExchange>,
    ) -> Result<Self, SystemDnsError> {
        settings.validate()?;
        let (scopes, received) = watch::channel(None);
        let dispatcher = Arc::new(
            OwnerDispatcher::new(
                received,
                exchange,
                meshway_dns::Limits {
                    max_pending: 128,
                    query_timeout: std::time::Duration::from_secs(5),
                },
                1232,
            )
            .map_err(|_| SystemDnsError::Settings)?,
        );
        let stop = CancellationToken::new();
        let service = serve(&settings, &dispatcher, &stop).await?;
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

    /// Netmap DNS settings this installer can serve.
    pub fn check(&self, netmap: &NodeNetmap) -> Result<(), SystemDnsError> {
        let settings = &netmap.projection.configuration.dns;
        let plan = SystemDnsPlan::new(settings, &netmap.dns.egress)?;
        let via_exit = matches!(netmap.dns.egress, DnsPlan::ViaExit { .. });
        if settings.servers.values().any(|server| {
            (!via_exit || server.use_with_exit)
                && (server.server.is_loopback()
                    || matches!(server.server, std::net::IpAddr::V6(ip) if ip.is_unicast_link_local()))
        }) {
            // A local stub can recurse through the very split route being
            // installed; scoped IPv6 also lacks a published interface context.
            return Err(SystemDnsError::ResolverPlan);
        }
        if matches!(self.settings.backend, SystemDnsBackend::Resolved)
            && plan.needs_system_fallback(settings)
        {
            // Resolved's original link-specific routing must not be flattened.
            return Err(SystemDnsError::ResolverPlan);
        }
        Ok(())
    }

    /// Install the required negative system capture, including for
    /// unsupported plans. It never publishes query authority.
    pub async fn hold(&mut self, netmap: &NodeNetmap) -> Result<(), SystemDnsError> {
        self.suspend();
        if matches!(&self.settings.backend, SystemDnsBackend::OwnedResolvConf { path } if path != std::path::Path::new("/etc/resolv.conf"))
        {
            return Err(SystemDnsError::Unsupported);
        }
        let plan =
            SystemDnsPlan::required(&netmap.projection.configuration.dns, &netmap.dns.egress)?;
        if self.installed_plan.as_ref() == Some(&plan) {
            return self.observe().await;
        }
        self.system.install(&plan).await?;
        self.installed_plan = Some(plan);
        Ok(())
    }

    /// Publish query authority for the desired content. The same content
    /// keeps the query owner (`Scope::equivalent`).
    pub async fn install(&mut self, desired: Desired) -> Result<(), SystemDnsError> {
        let continuing = self
            .scopes
            .borrow()
            .as_ref()
            .is_some_and(|scope| scope.desired.reference() == desired.reference());
        if continuing {
            return self.observe().await;
        }
        let netmap = desired.netmap().as_ref().clone();
        // Required negative capture first; a later failure keeps it with Scope=None.
        self.hold(&netmap).await?;
        self.check(&netmap)?;
        let scope = Scope::netmap(desired, self.resolvers(&netmap.dns.egress));
        self.dispatcher
            .validate_scope(&scope)
            .map_err(|_| SystemDnsError::ResolverPlan)?;
        if self.service.is_finished() {
            return Err(SystemDnsError::Backend);
        }
        meshway_dns::publish_scope(&self.scopes, Some(scope));
        if self.dispatcher.reconcile().await.is_err() {
            self.suspend();
            return Err(SystemDnsError::ResolverPlan);
        }
        self.observe().await
    }
    pub fn suspend(&self) {
        meshway_dns::publish_scope(&self.scopes, None);
    }
    /// The OS resolvers a plan may use: none for a Mesh exit client, the
    /// served snapshot for an exit resolver, else this Node's own fallback.
    fn resolvers(&self, plan: &DnsPlan) -> Option<meshway_dns::SystemResolverSnapshot> {
        match plan {
            DnsPlan::ViaExit { .. } | DnsPlan::Unavailable { .. } => None,
            DnsPlan::ServeExitResolver { .. } => self.system.served_resolvers(),
            DnsPlan::NoExit => self.system.system_resolvers(),
        }
    }
    /// Parent selects this regularly with the kernel/lease watcher. A failed
    /// observation withdraws query authority without removing forced suffixes.
    /// An exit resolver's served snapshot follows the observed OS inventory:
    /// a changed snapshot republishes the same content's scope.
    pub async fn observe(&mut self) -> Result<(), SystemDnsError> {
        let result = if self.service.is_finished() {
            Err(SystemDnsError::Backend)
        } else {
            self.system.observe().await
        };
        if result.is_err() {
            self.suspend();
            return result;
        }
        let current = self.scopes.borrow().clone();
        if let Some(scope) = current
            && matches!(
                scope.desired.netmap().dns.egress,
                DnsPlan::ServeExitResolver { .. }
            )
        {
            let next = Scope::netmap(
                scope.desired.clone(),
                self.resolvers(&scope.desired.netmap().dns.egress),
            );
            if !scope.equivalent(&next) {
                if self.dispatcher.validate_scope(&next).is_err() {
                    self.suspend();
                    return Err(SystemDnsError::ResolverPlan);
                }
                meshway_dns::publish_scope(&self.scopes, Some(next));
            }
        }
        Ok(())
    }
    /// After a failed observation: re-prove the system installation against
    /// the backend as it is now, reinstall the held negative capture (lost
    /// settings are installed afresh) and restart a finished query service.
    /// Query authority stays withdrawn until the parent installs content again.
    pub async fn recover(&mut self) -> Result<(), SystemDnsError> {
        self.suspend();
        if self.service.is_finished() {
            self.service = serve(&self.settings, &self.dispatcher, &self.stop).await?;
        }
        self.system.recover().await?;
        if let Some(plan) = &self.installed_plan {
            self.system.install(plan).await?;
        }
        self.observe().await
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

/// Bind the real service sockets before any system-DNS mutation.
async fn serve(
    settings: &SystemDnsSettings,
    dispatcher: &Arc<OwnerDispatcher>,
    stop: &CancellationToken,
) -> Result<tokio::task::JoinHandle<std::io::Result<()>>, SystemDnsError> {
    let udp = UdpSocket::bind(settings.bind)
        .await
        .map_err(|_| SystemDnsError::Backend)?;
    let tcp = TcpListener::bind(settings.bind)
        .await
        .map_err(|_| SystemDnsError::Backend)?;
    let service = Service::new(dispatcher.clone(), Default::default())
        .map_err(|_| SystemDnsError::Settings)?;
    Ok(tokio::spawn(service.run(udp, tcp, stop.clone())))
}
