//! Daemon assembly of the one DNS query owner and the one system installer.
//! Every resolver route is decided by the installed netmap itself.
mod actor;
pub(super) mod mesh;
pub(super) use actor::{Attachment, Health};
use meshway_configuration::netmap::{DnsPlan, NodeNetmap};
use meshway_dns::{
    CancellationToken, OwnerDispatcher, ResolverExchange, Scope, Service, SystemDnsBackend,
    SystemDnsError, SystemDnsFailure, SystemDnsPlan, SystemDnsSettings,
};
use meshway_engine::Desired;
use meshway_model::exit::{ExitDns, ExitDnsStatus, ExitDnsUnavailableReason};
use std::sync::Arc;
use tokio::{
    net::{TcpListener, UdpSocket},
    sync::watch,
};

/// System-upstream provenance expires after two seconds without a real owner
/// observation. Include this timer even when there is no packet work.
pub(super) const OBSERVE_INTERVAL: std::time::Duration = std::time::Duration::from_millis(250);

pub(super) struct Runtime {
    system: Box<dyn crate::host::SystemDns>,
    settings: SystemDnsSettings,
    scopes: watch::Sender<Option<Scope>>,
    dispatcher: Arc<OwnerDispatcher>,
    stop: CancellationToken,
    service: tokio::task::JoinHandle<std::io::Result<()>>,
    installed_plan: Option<SystemDnsPlan>,
    /// The one producer of the exit resolver fact; the plane reports it.
    exit_dns: watch::Sender<Option<ExitDns>>,
    /// Why the last observation or recovery failed, until one succeeds.
    failure: Option<SystemDnsFailure>,
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
        mut system: Box<dyn crate::host::SystemDns>,
        settings: SystemDnsSettings,
        exchange: Arc<dyn ResolverExchange>,
        exit_dns: watch::Sender<Option<ExitDns>>,
        reports: tokio::sync::mpsc::Sender<meshway_protocol::DnsAssociationReport>,
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
            .map_err(|_| SystemDnsError::Settings)?
            .with_association_reports(reports),
        );
        let stop = CancellationToken::new();
        let service = serve(system.as_mut(), &settings, &dispatcher, &stop).await?;
        Ok(Self {
            system,
            settings,
            scopes,
            dispatcher,
            stop,
            service,
            installed_plan: None,
            exit_dns,
            failure: None,
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
        // The same content with other trusted associations changes nothing
        // DNS installs or answers with: its published scope and in-flight
        // queries (some waiting for exactly these associations) continue.
        let continuing = self.scopes.borrow().as_ref().is_some_and(|scope| {
            let held = scope.desired.netmap();
            held.network_id == netmap.network_id
                && held.node_id == netmap.node_id
                && held.serial == netmap.serial
                && held.dns == netmap.dns
                && held.projection.configuration.dns == netmap.projection.configuration.dns
        });
        if !continuing {
            self.suspend();
        }
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
        let current = self.scopes.borrow().clone();
        if let Some(scope) = current
            && scope.desired.same_content(&desired)
        {
            // Only trusted associations changed: the same scope, its
            // tickets and system installation stay; answers see them now.
            if !scope.desired.same(&desired) {
                let mut next = scope;
                next.desired = desired;
                meshway_dns::publish_scope(&self.scopes, Some(next));
            }
            return self.observe().await;
        }
        let netmap = desired.netmap().as_ref().clone();
        // Required negative capture first; a later failure keeps it with Scope=None.
        self.hold(&netmap).await?;
        self.check(&netmap)?;
        let (system, served) = self.resolvers(&desired);
        let scope = Scope::netmap(desired, system).serving(served.clone().ok().flatten());
        self.dispatcher
            .validate_scope(&scope)
            .map_err(|_| SystemDnsError::ResolverPlan)?;
        if self.service.is_finished() {
            return Err(SystemDnsError::Backend);
        }
        let reference = scope.desired.reference();
        meshway_dns::publish_scope(&self.scopes, Some(scope));
        self.report_exit_dns(reference, &netmap, &served);
        if self.dispatcher.reconcile().await.is_err() {
            self.suspend();
            return Err(SystemDnsError::ResolverPlan);
        }
        self.observe().await
    }
    pub fn suspend(&self) {
        meshway_dns::publish_scope(&self.scopes, None);
        self.exit_dns.send_if_modified(|fact| fact.take().is_some());
    }
    /// This Node's own OS upstreams (none for a Mesh exit client, which
    /// resolves through its exit) and, for an exit resolver, what it serves
    /// its sources with or why it cannot. The two are independent: served
    /// unavailability never fails this Node's own DNS installation.
    #[allow(clippy::type_complexity)]
    fn resolvers(
        &self,
        desired: &Desired,
    ) -> (
        Option<meshway_dns::SystemResolverSnapshot>,
        Result<Option<meshway_dns::SystemResolverSnapshot>, ExitDnsUnavailableReason>,
    ) {
        let system = match desired.netmap().dns.egress {
            DnsPlan::ViaExit { .. } | DnsPlan::Unavailable { .. } => None,
            DnsPlan::NoExit | DnsPlan::ServeExitResolver { .. } => self.system.system_resolvers(),
        };
        let served = match desired.netmap().dns.egress {
            DnsPlan::ServeExitResolver { .. } => {
                meshway_dns::exit_resolvers(desired, self.system.served_resolvers())
            }
            _ => Ok(None),
        };
        (system, served)
    }
    /// Publish the exit resolver fact of the content just published: absent
    /// unless the netmap serves sources.
    fn report_exit_dns(
        &self,
        reference: meshway_model::NetmapRef,
        netmap: &NodeNetmap,
        served: &Result<Option<meshway_dns::SystemResolverSnapshot>, ExitDnsUnavailableReason>,
    ) {
        let fact =
            matches!(netmap.dns.egress, DnsPlan::ServeExitResolver { .. }).then(|| ExitDns {
                netmap: reference,
                status: match served {
                    Ok(_) => ExitDnsStatus::Serving,
                    Err(reason) => ExitDnsStatus::Unavailable { reason: *reason },
                },
            });
        self.exit_dns.send_if_modified(|current| {
            if *current == fact {
                return false;
            }
            if let Some(ExitDns {
                status: ExitDnsStatus::Unavailable { reason },
                ..
            }) = &fact
            {
                tracing::warn!(?reason, "Exit resolver cannot serve its sources");
            }
            *current = fact;
            true
        });
    }
    /// Parent selects this regularly with the kernel/lease watcher. A failed
    /// observation withdraws query authority without removing forced suffixes.
    /// An exit resolver's served snapshot follows the observed OS inventory:
    /// a changed snapshot republishes the same content's scope, which ends
    /// this Node's in-flight queries of the old scope.
    pub async fn observe(&mut self) -> Result<(), SystemDnsError> {
        let result = self.observed().await;
        self.failure = result.as_ref().err().cloned();
        result.map_err(|failure| failure.error)
    }
    /// Why the last observation or recovery failed, until one succeeds.
    pub fn failure(&self) -> Option<&SystemDnsFailure> {
        self.failure.as_ref()
    }
    fn system_failure(&self, error: SystemDnsError) -> SystemDnsFailure {
        self.system
            .failure()
            .filter(|failure| failure.error == error)
            .cloned()
            .unwrap_or_else(|| SystemDnsFailure::new(error, "system_owner"))
    }
    fn service_failure() -> SystemDnsFailure {
        SystemDnsFailure::new(SystemDnsError::Backend, "query_service")
            .expected("the DNS query service serving its bind")
            .actual("the service task ended")
    }
    async fn observed(&mut self) -> Result<(), SystemDnsFailure> {
        let result = if self.service.is_finished() {
            Err(Self::service_failure())
        } else {
            match self.system.observe().await {
                Ok(()) => Ok(()),
                Err(error) => Err(self.system_failure(error)),
            }
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
            let (system, served) = self.resolvers(&scope.desired);
            let next =
                Scope::netmap(scope.desired.clone(), system).serving(served.clone().ok().flatten());
            if !scope.equivalent(&next) {
                if self.dispatcher.validate_scope(&next).is_err() {
                    self.suspend();
                    return Err(SystemDnsFailure::new(
                        SystemDnsError::ResolverPlan,
                        "served_scope",
                    )
                    .expected("a valid scope for the re-observed served resolvers")
                    .actual("the dispatcher rejected it"));
                }
                meshway_dns::publish_scope(&self.scopes, Some(next));
            }
            self.report_exit_dns(scope.desired.reference(), scope.desired.netmap(), &served);
        }
        Ok(())
    }
    /// After a failed observation: re-prove the system installation against
    /// the backend as it is now, reinstall the held negative capture (lost
    /// settings are installed afresh) and restart a finished query service.
    /// Query authority stays withdrawn until the parent installs content again.
    pub async fn recover(&mut self) -> Result<(), SystemDnsError> {
        let result = self.recovered().await;
        self.failure = result.as_ref().err().cloned();
        result.map_err(|failure| failure.error)
    }
    async fn recovered(&mut self) -> Result<(), SystemDnsFailure> {
        self.suspend();
        if self.service.is_finished() {
            self.service = serve(
                self.system.as_mut(),
                &self.settings,
                &self.dispatcher,
                &self.stop,
            )
            .await
            .map_err(|error| {
                SystemDnsFailure::new(error, "query_service")
                    .expected(format!(
                        "the DNS query service bound to {}",
                        self.settings.bind
                    ))
                    .actual("bind failed")
            })?;
        }
        if let Err(error) = self.system.recover().await {
            return Err(self.system_failure(error));
        }
        if let Some(plan) = &self.installed_plan
            && let Err(error) = self.system.install(plan).await
        {
            return Err(self.system_failure(error));
        }
        self.observed().await
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
    _system: &mut dyn crate::host::SystemDns,
    settings: &SystemDnsSettings,
    dispatcher: &Arc<OwnerDispatcher>,
    stop: &CancellationToken,
) -> Result<tokio::task::JoinHandle<std::io::Result<()>>, SystemDnsError> {
    #[cfg(target_os = "macos")]
    if let Some(transport) = _system.packet_transport()? {
        let service = Service::new(dispatcher.clone(), Default::default())
            .map_err(|_| SystemDnsError::Settings)?;
        return Ok(tokio::spawn(service.run_packets(transport, stop.clone())));
    }
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
