//! The Node's component set behind the single [`meshway_engine::Engine`].
//! Each apply diffs the actual installed resources against the desired
//! content. Withdrawal terminates every flow, circuit and exit projection,
//! closes adjacencies and keeps only the negative capture and DNS boundaries.
use super::{Peers, Report, dns::Attachment, packets::Packets};
use meshway_authority::{DataPlaneIdentity, DataPlaneKeys};
use meshway_configuration::netmap::DnsPlan;
use meshway_connectivity::Manager;
use meshway_dataplane::secure::SecureNode;
use meshway_engine::{Desired, Error};
use meshway_ipc::Problem;
use meshway_protocol::{InstanceRequest, execution::ApplyFailure};
use meshway_underlay::Host;
use std::sync::Arc;

pub(super) struct Installed {
    pub desired: Desired,
    /// Staged migration only: the Host and DNS snapshot of `desired`.
    pub identity: DataPlaneIdentity,
    /// Absent for a disabled Node, which owns no packet plane.
    pub secure: Option<SecureNode>,
    pub manager: Option<Manager>,
    pub exit: Option<super::exit::Runtime>,
}

pub(super) struct Plane {
    pub host: Host,
    pub dns: Attachment,
    pub peers: Peers,
    pub packets: Packets,
    pub installed: Option<Installed>,
    /// Working report fields owned by the applies; the worker publishes them.
    pub report: Report,
    /// Why the last failed apply failed.
    pub failure: Option<(Problem, ApplyFailure)>,
    traversal: Arc<dyn meshway_connectivity::traversal::TraversalGateway>,
    keys: Option<DataPlaneKeys>,
    instance: Option<InstanceRequest>,
    generation: meshway_model::Sequence,
    /// Staged migration only: snapshots for not-yet-migrated components.
    staged: meshway_authority::StagedIssuer,
    lease: Option<meshway_model::NetmapLease>,
}

impl Plane {
    pub fn new(
        host: Host,
        dns: Attachment,
        peers: Peers,
        traversal: Arc<dyn meshway_connectivity::traversal::TraversalGateway>,
    ) -> Result<Self, Problem> {
        Ok(Self {
            host,
            dns,
            peers,
            packets: Packets::default(),
            installed: None,
            report: Report::stopped(),
            failure: None,
            traversal,
            keys: None,
            instance: None,
            generation: meshway_model::Sequence::MIN,
            staged: meshway_authority::StagedIssuer::new().map_err(|_| Problem::Clock)?,
            lease: None,
        })
    }

    pub fn set_keys(&mut self, keys: Option<DataPlaneKeys>) {
        self.keys = keys;
    }

    /// Staged: the transitional snapshot follows the Engine-accepted lease.
    pub fn stage(&mut self, lease: meshway_model::NetmapLease) {
        self.lease = Some(lease);
        let current = self
            .staged
            .current()
            .map(|identity| identity.snapshot().reference());
        if current == Some(lease.netmap)
            && let Some(state) = &self.installed
            && let Some(keys) = &self.keys
        {
            let content = state.desired.content().clone();
            let _ = self
                .staged
                .issue(&content, lease, self.instance.clone(), keys);
        }
    }

    /// State/receipt binding only; never part of desired content.
    pub async fn set_instance(&mut self, instance: Option<InstanceRequest>) {
        if self.instance == instance {
            return;
        }
        self.instance = instance;
        let Some(state) = &mut self.installed else {
            return;
        };
        let (Some(lease), Some(keys)) = (self.lease, &self.keys) else {
            return;
        };
        let content = state.desired.content().clone();
        let Ok(identity) = self
            .staged
            .issue(&content, lease, self.instance.clone(), keys)
        else {
            return;
        };
        let _ = self.host.install(identity.snapshot().clone()).await;
        if let Some(manager) = &state.manager {
            let _ = manager.set_instance(self.instance.clone());
        }
        if self.dns.guard().is_some() {
            let _ = self.dns.install(identity.snapshot().clone()).await;
        }
        state.identity = identity;
    }

    pub fn secure(&self) -> Option<&SecureNode> {
        self.installed.as_ref()?.secure.as_ref()
    }

    /// The applied content and the State instance, for local diagnostics.
    pub fn diagnostic_scope(&self) -> Option<(&Desired, Option<meshway_model::InstanceId>)> {
        let state = self
            .installed
            .as_ref()
            .filter(|state| state.secure.is_some())?;
        Some((
            &state.desired,
            self.instance.as_ref().map(|instance| instance.instance_id),
        ))
    }

    fn fail(&mut self, problem: Problem, failure: ApplyFailure) -> Error {
        self.failure = Some((problem, failure));
        Error::Apply
    }

    fn identity_for(&mut self, desired: &Desired) -> Result<DataPlaneIdentity, Error> {
        let issued = match (self.lease, &self.keys) {
            (Some(lease), Some(keys)) if lease.netmap == desired.reference() => self
                .staged
                .issue(desired.content(), lease, self.instance.clone(), keys)
                .ok(),
            _ => None,
        };
        issued.ok_or_else(|| {
            tracing::warn!("Netmap not bindable to packet keys");
            self.fail(Problem::Apply, ApplyFailure::Validation)
        })
    }

    async fn close(&mut self) -> Result<(), Problem> {
        let mut failed = false;
        if let Some(state) = &mut self.installed {
            if let Some(exit) = state.exit.take() {
                let result = match &mut state.secure {
                    Some(secure) => exit.stop(secure).await,
                    None => Err(Problem::Underlay),
                };
                if let Err(problem) = result {
                    tracing::warn!(?problem, "Exit retirement failed");
                    failed = true;
                }
            }
            if let Some(mut manager) = state.manager.take() {
                self.peers.detach();
                let _ = manager.shutdown().await;
            }
            state.secure = None;
        }
        self.installed = None;
        self.packets = Packets::default();
        if failed {
            Err(Problem::Underlay)
        } else {
            Ok(())
        }
    }
}

impl meshway_engine::Components for Plane {
    /// Flows, circuits, queued outputs and the exit forwarding owner.
    async fn apply_packets(&mut self, desired: Option<&Desired>) -> Result<(), Error> {
        let Some(desired) = desired else {
            self.dns.suspend();
            let closing = self.close().await;
            self.staged.withdraw();
            return closing.map_err(|problem| self.fail(problem, ApplyFailure::Underlay));
        };
        self.packets.clear_incoming();
        if !desired.local().enabled {
            return Ok(());
        }
        let Some(keys) = self.keys.clone() else {
            return Err(self.fail(Problem::Apply, ApplyFailure::Validation));
        };
        let Some(state) = self.installed.as_mut() else {
            return Err(self.fail(Problem::Apply, ApplyFailure::Validation));
        };
        let carriers = match state.manager.as_ref().map(Manager::carriers) {
            Some(Ok(carriers)) => carriers,
            _ => return Err(self.fail(Problem::Transport, ApplyFailure::Listeners)),
        };
        let secure = match state.secure.take() {
            Some(mut secure) => match secure.apply(desired) {
                Ok(applied) => {
                    tracing::debug!(
                        ?applied,
                        netmap_serial = desired.reference().serial.get(),
                        "SecureNode applied desired content"
                    );
                    secure.set_carriers(carriers).map(|_| secure).ok()
                }
                Err(_) => SecureNode::new(desired, keys, Default::default(), carriers).ok(),
            },
            None => SecureNode::new(desired, keys, Default::default(), carriers).ok(),
        };
        let Some(secure) = secure else {
            return Err(self.fail(Problem::Apply, ApplyFailure::Dataplane));
        };
        let role = match desired.exit_role() {
            Some(plan)
                if self
                    .installed
                    .as_ref()
                    .and_then(|state| state.exit.as_ref())
                    .is_none_or(|exit| exit.plan != *plan) =>
            {
                match self.host.exit_role() {
                    Some(role) => Some((role, plan.clone())),
                    None => return Err(self.fail(Problem::Underlay, ApplyFailure::Underlay)),
                }
            }
            _ => None,
        };
        let state = self.installed.as_mut().expect("checked above");
        state.secure = Some(secure);
        if let Some((role, plan)) = role {
            match super::exit::Runtime::start(role, plan) {
                Ok(runtime) => state.exit = Some(runtime),
                Err(problem) => return Err(self.fail(problem, ApplyFailure::Underlay)),
            }
        }
        let state = self.installed.as_mut().expect("checked above");
        if let (Some(exit), Some(secure)) = (&mut state.exit, &mut state.secure) {
            if let Err(problem) = exit.synchronize(secure).await {
                return Err(self.fail(problem, ApplyFailure::Underlay));
            }
            if let Some(capability) = &mut self.report.exit_capability {
                capability.installed_families = vec![meshway_model::exit::ExitFamily::Ipv4];
            }
        }
        Ok(())
    }

    /// Authenticated adjacencies: unchanged resources and channels survive.
    async fn apply_adjacencies(&mut self, desired: Option<&Desired>) -> Result<(), Error> {
        let Some(desired) = desired else {
            if let Some(state) = &mut self.installed
                && let Some(mut manager) = state.manager.take()
            {
                self.peers.detach();
                let _ = manager.shutdown().await;
            }
            return Ok(());
        };
        if !desired.local().enabled {
            if let Some(state) = &mut self.installed
                && let Some(mut manager) = state.manager.take()
            {
                self.peers.detach();
                let _ = manager.shutdown().await;
            }
            return Ok(());
        }
        let previous = self
            .installed
            .as_mut()
            .and_then(|state| state.manager.take());
        let manager = match previous {
            Some(mut manager) => {
                if manager.apply(desired).await.is_err() {
                    let _ = manager.shutdown().await;
                    self.peers.detach();
                    return Err(self.fail(Problem::Transport, ApplyFailure::Listeners));
                }
                manager
            }
            None => {
                let Ok(generation) = self.generation.checked_next() else {
                    return Err(self.fail(Problem::Worker, ApplyFailure::Listeners));
                };
                self.generation = generation;
                let Some(keys) = self.keys.clone() else {
                    return Err(self.fail(Problem::Apply, ApplyFailure::Validation));
                };
                match Manager::start_with_traversal(
                    desired,
                    keys,
                    Arc::new(self.host.sockets()),
                    Default::default(),
                    self.traversal.clone(),
                    generation,
                )
                .await
                {
                    Ok(manager) => {
                        let _ = manager.set_instance(self.instance.clone());
                        manager
                    }
                    Err(_) => return Err(self.fail(Problem::Transport, ApplyFailure::Listeners)),
                }
            }
        };
        self.peers.attach(manager.facts_sink(), desired.reference());
        if let Some(state) = &mut self.installed {
            state.manager = Some(manager);
        }
        Ok(())
    }

    /// Capture, TUN, retained descriptors, negative DNS and the exit role.
    async fn apply_host(&mut self, desired: Option<&Desired>) -> Result<(), Error> {
        let Some(desired) = desired else {
            return match self.host.suspend().await {
                Ok(()) => Ok(()),
                Err(_) => Err(self.fail(Problem::Underlay, ApplyFailure::Underlay)),
            };
        };
        let identity = self.identity_for(desired)?;
        let continuing = self.installed.as_ref().is_some_and(|state| {
            identity
                .snapshot()
                .netmap_continuation_from(state.identity.snapshot())
                .is_ok()
                || state.identity.snapshot().same_netmap(identity.snapshot())
        });
        if !continuing && let Err(problem) = self.close().await {
            return Err(self.fail(problem, ApplyFailure::Underlay));
        }
        if let Err(error) = self.host.install(identity.snapshot().clone()).await {
            tracing::warn!(%error, "Node capture installation failed");
            return Err(match error {
                meshway_underlay::Error::Tun(_) => self.fail(Problem::Tun, ApplyFailure::Tun),
                _ => self.fail(Problem::Underlay, ApplyFailure::Underlay),
            });
        }
        let netmap = desired.netmap().clone();
        let exit_dns = !matches!(netmap.dns.egress, DnsPlan::NoExit);
        let needs_dns =
            super::requirements::dns_required(&netmap.projection.configuration.dns) || exit_dns;
        self.report.exit_dns_unavailable = false;
        // Required negative DNS first; query authority follows the packet
        // plane in apply_dns.
        let negative = if needs_dns {
            self.dns
                .plan(&mut self.host, &netmap)
                .await
                .map_err(|error| {
                    tracing::warn!(%error, "Required DNS installation failed");
                })
        } else {
            self.dns.down(&mut self.host).await.map_err(|_| ())
        };
        if negative.is_err() {
            return Err(self.fail(Problem::Dns, ApplyFailure::Dns));
        }
        if !netmap.self_.enabled {
            if let Err(problem) = self.close().await {
                return Err(self.fail(problem, ApplyFailure::Underlay));
            }
            self.report.exit_capability = None;
            self.installed = Some(Installed {
                desired: desired.clone(),
                identity,
                secure: None,
                manager: None,
                exit: None,
            });
            return Ok(());
        }
        // Exit prerequisites, observed on behalf of the live instance only.
        self.report.exit_capability = match (&netmap.exit_role, self.instance.clone()) {
            (Some(role), Some(instance)) => Some(
                self.host
                    .probe_exit(instance.instance_id, desired.reference(), role.generation)
                    .await,
            ),
            _ => None,
        };
        let installed_plan = self
            .installed
            .as_ref()
            .and_then(|state| state.exit.as_ref())
            .map(|exit| exit.plan.clone());
        let role_changed = installed_plan.as_ref() != netmap.exit_role.as_ref();
        if role_changed
            && let Some(state) = self.installed.as_mut()
            && let Some(old) = state.exit.take()
        {
            let stopped = match &mut state.secure {
                Some(secure) => old.stop(secure).await,
                None => Err(Problem::Underlay),
            };
            if let Err(problem) = stopped {
                return Err(self.fail(problem, ApplyFailure::Underlay));
            }
        }
        if let Some(role) = &netmap.exit_role
            && role_changed
        {
            if self
                .report
                .exit_capability
                .as_ref()
                .is_some_and(|c| c.status != meshway_model::exit::ExitStatus::Ready)
            {
                return Err(self.fail(Problem::Underlay, ApplyFailure::Underlay));
            }
            if let Err(error) = self.host.install_netmap_exit_role(role).await {
                tracing::warn!(%error, "Exit role installation failed");
                if let Some(capability) = &mut self.report.exit_capability {
                    capability.status = meshway_model::exit::ExitStatus::Unavailable {
                        reason: meshway_model::exit::ExitUnavailableReason::OwnershipConflict,
                    };
                }
                return Err(self.fail(Problem::Underlay, ApplyFailure::Underlay));
            }
        }
        if self.host.tun().is_none() {
            return Err(self.fail(Problem::Tun, ApplyFailure::Tun));
        }
        match &mut self.installed {
            Some(state) => {
                state.desired = desired.clone();
                state.identity = identity;
            }
            None => {
                self.installed = Some(Installed {
                    desired: desired.clone(),
                    identity,
                    secure: None,
                    manager: None,
                    exit: None,
                })
            }
        }
        Ok(())
    }

    /// Query authority over the packet plane, after the negative capture.
    async fn apply_dns(&mut self, desired: Option<&Desired>) -> Result<(), Error> {
        let Some(desired) = desired else {
            self.dns.suspend();
            return Ok(());
        };
        let identity = self.identity_for(desired)?;
        let netmap = desired.netmap();
        let needs_dns = super::requirements::dns_required(&netmap.projection.configuration.dns)
            || !matches!(netmap.dns.egress, DnsPlan::NoExit);
        if needs_dns
            && netmap.self_.enabled
            && let Err(error) = self.dns.install(identity.snapshot().clone()).await
        {
            tracing::warn!(%error, "DNS scope installation failed");
            return Err(self.fail(Problem::Dns, ApplyFailure::Dns));
        }
        if identity.snapshot().ensure_current().is_err() {
            return Err(self.fail(Problem::Apply, ApplyFailure::Validation));
        }
        Ok(())
    }
}
